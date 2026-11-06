//! GUI-specific core behavior for the Tauri desktop app.
//!
//! `association`, `scanner`, `dry_run`, `package`, `upload_queue`, and
//! `uploader` already cover everything `astro-sync-cli` needs; this module
//! adds the handful of behaviors that are specific to the GUI shell and
//! were never part of the CLI's scope, so `app/src-tauri`'s commands stay
//! thin wrappers rather than growing their own logic:
//!
//! - [`GuiSettings`]/[`load_settings`]/[`save_settings`] -- the portal URL,
//!   device label, and UI locale, persisted as plain JSON at a path the
//!   Tauri shell chooses (its own app-data directory).
//! - [`load_device_token`]/[`save_device_token`]/[`clear_device_token`] --
//!   the paired device's bearer token. See the "Device token storage" note
//!   below for where this lives (the OS keychain, with a file fallback)
//!   and why.
//! - [`claim_device`] -- exchanges a pairing code shown on the portal's own
//!   account page for a device bearer token
//!   (`POST /v1/devices/claim`, reusing the exact pairing contract
//!   `contracts/schemas/api/device-pairing.schema.json`
//!   documents), behind the same injectable-transport pattern
//!   [`crate::uploader`] already uses.
//! - [`list_sessions`] -- the portal's observation-session page
//!   (`GET /v1/sessions`), for the session-list screen.
//! - [`upload_analysis`] -- uploads a local-analysis report
//!   (`crate::analysis::report`) as a new session-analysis version
//!   (`POST /v1/sessions/{id}/analyses`), for the Analysis screen.
//! - [`summarize_queue`] -- the per-state/byte counts the upload-queue
//!   screen shows, computed from [`crate::upload_queue::QueueEntry`] rows
//!   the GUI already fetched.
//! - [`drive_queue`] -- processes every pending queue entry one at a time.
//!   It is the one loop the GUI's "upload all" action and
//!   `astro-sync-cli`'s `upload` command share, so the two never disagree on
//!   what a refusal does: a file the service refuses for good is set aside
//!   and the run goes on, a file that needs the person (storage full) holds
//!   the run, and a failure that will pass (a network that is down, a
//!   service that fails, too many uploads open in the space) is waited out
//!   -- longer each time, never in a loop ([`Backoff`], [`DriveOptions`]).
//!   It can be paused between files, or during a wait, via
//!   `should_continue`.
//!
//! Kept in its own module, rather than folded into an existing one, so
//! concurrent work elsewhere in this crate (a `canonical_json` addition)
//! and in `astro-sync-cli` stays conflict-free.
//!
//! ## Device token storage: OS keyring, with a file fallback
//!
//! [`load_device_token`]/[`save_device_token`]/[`clear_device_token`] now go
//! through the `keyring` crate first -- macOS Keychain, the Linux Secret
//! Service, or the Windows Credential Manager, whichever `default_credential_builder`
//! resolves to on the running platform -- and only fall back to the old
//! plain file (still restricted to owner read/write, Unix `0600`,
//! immediately after writing) when the keyring itself is unavailable or
//! refuses the call: headless session, locked/absent keychain daemon, an
//! unsupported platform, or the user denying an access prompt. That
//! fallback is automatic and silent to the caller (it shows up only as
//! which of the two stores ends up holding the token), which is why
//! `app/src-tauri`'s `settings_get` Tauri command never needs to know or
//! care which one is in play -- it already only ever surfaced a
//! paired/not-paired boolean, never the token itself.
//!
//! Every credential this app stores uses one fixed keyring *service* name,
//! [`DEVICE_TOKEN_KEYRING_SERVICE`], and an *account* equal to the paired
//! portal's URL -- passed in by the caller as `keyring_account` on every
//! function below -- so a device re-paired against a different portal gets
//! its own credential instead of overwriting the first one's. Using a fixed
//! service name (rather than, say, embedding the app's install path or a
//! per-run random value) matters specifically because this app isn't
//! code-signed with a stable identity yet (the Apple developer account is
//! mid-migration): keeping the service/account stable is the one thing
//! under this crate's control that keeps repeated access looking like "the
//! same credential" to the platform store, rather than compounding the
//! signing problem with an identity that also changes across runs.
//!
//! ### What a developer will see on macOS, unsigned
//!
//! macOS ties a Keychain item's access-control list to the *code signature*
//! of the app that created it. An ad hoc/unsigned build's signature is a
//! hash of the binary itself, so it changes on every `cargo tauri build`.
//! Concretely, that means: the *first* time a freshly built app touches the
//! keychain (pairing, or the one-time migration below), macOS shows its
//! standard "`app` wants to use your confidential information stored in
//! `<portal URL>` in your keychain" prompt with Allow/Always Allow/Deny.
//! Choosing Always Allow is remembered for *that exact build*, so relaunching
//! the same binary won't re-prompt -- but the next `cargo tauri build`
//! produces a new signature and will prompt again, once, on its first
//! keychain access. This is a property of unsigned builds, not a bug in
//! this module; it goes away once the app ships signed with a stable
//! Developer ID (Allow/Always Allow then persists across rebuilds). Denying
//! the prompt is treated exactly like any other keyring refusal: the token
//! falls back to the file store below, and pairing still works.
//!
//! This prompt is interactive UI shown only when the GUI app itself runs;
//! nothing in this crate's test suite or `cargo build`/`cargo test` ever
//! constructs a real platform [`keyring::Entry`] (see the tests at the
//! bottom of this file), so neither can hang waiting on it.
//!
//! ### Migration from the pre-8.8 file
//!
//! An install that paired before this task has its token sitting in the
//! legacy file with no keyring entry yet. The first [`load_device_token`]
//! call after upgrading finds the keyring empty for that account, reads the
//! file, writes the token into the keyring, deletes the file, and logs
//! (to stderr, one line, *never the token itself*) that the migration
//! happened. If the keyring write fails, the file is left in place
//! untouched and the same migration is retried on the next call.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;

use crate::service_wait::{
    HttpAnswer, HttpFailure, PatientHttp, RequestFactory, ServiceWaitError, WaitHooks, WakePolicy,
};
use crate::space::SignInEnd;
use crate::standing::{Standing, standing_of_entitlements};
use crate::upload_queue::{QueueEntry, QueueState, UploadQueue, UploadQueueError};
use crate::uploader::{UploadTarget, UploadTransport, Verdict, upload_entry};

/// Portal URL, device label, and UI locale, as edited from the Settings
/// screen. Every field is optional: a freshly installed app has none of
/// them yet.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GuiSettings {
    pub portal_url: Option<String>,
    pub device_label: Option<String>,
    /// `"en"` / `"fr"` / `"ja"`. `None` means "follow the system locale",
    /// resolved entirely on the TypeScript side.
    pub locale: Option<String>,
}

#[derive(Debug, Error)]
pub enum GuiBridgeError {
    #[error("io error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("invalid settings JSON at {path}: {source}")]
    InvalidSettings {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("the service URL must be an absolute http(s) URL, got {0:?}")]
    InvalidPortalUrl(String),
    #[error(
        "the pairing code must be exactly 8 characters (Crockford base32, excluding I, L, O, U)"
    )]
    InvalidPairingCode,
    #[error("the device label must not be empty")]
    EmptyDeviceLabel,
    #[error("could not build the HTTP client: {0}")]
    ClientBuild(String),
    /// The service could not be reached at all (name not found,
    /// connection refused, TLS failure, no network): immediate.
    #[error("network error talking to the service: {0}")]
    Transport(String),
    /// The service was asleep or starting and the wait ended without an
    /// answer, or a write was sent once and never confirmed, or the person
    /// cancelled: see [`ServiceWaitError`].
    #[error(transparent)]
    Wait(#[from] ServiceWaitError),
    #[error("unreadable response from the service: {0}")]
    InvalidResponse(String),
    #[error("refused by the service ({code}): {message}")]
    Refused { code: String, message: String },
    #[error("upload queue error: {0}")]
    Queue(#[from] UploadQueueError),
    /// The OS keyring (Keychain/Secret Service/Credential Manager) refused
    /// or could not perform this call -- see the module doc's "Device
    /// token storage" note. Callers inside this module treat this as the
    /// signal to fall back to the legacy file; it is only surfaced to
    /// callers outside the module if that file fallback also fails.
    #[error("the system keyring is unavailable: {0}")]
    KeyringUnavailable(String),
    /// The service answered `401` `device_detached` or `space_deleted`: this
    /// computer is no longer attached to a space. Not a refusal of one
    /// request -- the end of the sign-in ([`SignInEnd`]), after which the
    /// token is forgotten and the person is asked to sign in again; nothing
    /// else is lost.
    #[error("{}", .0.plain_message())]
    SignInEnded(SignInEnd),
}

impl GuiBridgeError {
    /// The stable code an interface looks up as `errors.<code>`.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            GuiBridgeError::Io { .. } => "io_error",
            GuiBridgeError::InvalidSettings { .. } => "invalid_settings_file",
            GuiBridgeError::InvalidPortalUrl(_) => "invalid_portal_url",
            GuiBridgeError::InvalidPairingCode => "invalid_pairing_code",
            GuiBridgeError::EmptyDeviceLabel => "empty_device_label",
            GuiBridgeError::ClientBuild(_) => "client_build_error",
            GuiBridgeError::Transport(_) => "network_error",
            // The wait for the sleeping service ended without an answer (gave
            // up, cancelled, or a write never confirmed): each has its own
            // code, so the interface says what happened in plain words.
            GuiBridgeError::Wait(wait) => wait.code(),
            GuiBridgeError::InvalidResponse(_) => "invalid_portal_response",
            // A refusal the contract names has a sentence of its own in the
            // three languages (`crate::refusal`): the ceiling on the size of
            // a request, the ceilings on sessions, the space's subscription,
            // the ceiling on devices. Any other is `portal_refused`.
            GuiBridgeError::Refused { code, .. } => {
                crate::refusal::stable_code(code).unwrap_or("portal_refused")
            }
            GuiBridgeError::Queue(_) => "queue_error",
            // In practice never returned to a caller outside `gui_bridge`:
            // every keyring-backed function there falls back to the legacy
            // file on this error instead of propagating it. Kept as a
            // stable code anyway, for the exhaustive match and as a safety
            // net if a future caller ever surfaces it directly.
            GuiBridgeError::KeyringUnavailable(_) => "keyring_unavailable",
            GuiBridgeError::SignInEnded(end) => end.code(),
        }
    }
}

impl From<HttpFailure> for GuiBridgeError {
    fn from(failure: HttpFailure) -> Self {
        match failure {
            HttpFailure::Unreachable(detail) => GuiBridgeError::Transport(detail),
            HttpFailure::Wait(wait) => GuiBridgeError::Wait(wait),
        }
    }
}

fn validate_portal_url(url: &str) -> Result<(), GuiBridgeError> {
    let parsed =
        reqwest::Url::parse(url).map_err(|_| GuiBridgeError::InvalidPortalUrl(url.to_string()))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(GuiBridgeError::InvalidPortalUrl(url.to_string()));
    }
    Ok(())
}

fn ensure_parent_dir(path: &Path) -> Result<(), GuiBridgeError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| GuiBridgeError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

/// Loads settings from `path`. A missing file is not an error: it is a
/// freshly installed app with every field unset.
pub fn load_settings(path: &Path) -> Result<GuiSettings, GuiBridgeError> {
    if !path.exists() {
        return Ok(GuiSettings::default());
    }
    let raw = fs::read_to_string(path).map_err(|source| GuiBridgeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&raw).map_err(|source| GuiBridgeError::InvalidSettings {
        path: path.to_path_buf(),
        source,
    })
}

/// Persists `settings` at `path` as pretty JSON, creating parent
/// directories as needed. Refuses a non-empty `portalUrl` that is not an
/// absolute `http(s)` URL, so a typo is caught here rather than at the
/// first failed request.
pub fn save_settings(path: &Path, settings: &GuiSettings) -> Result<(), GuiBridgeError> {
    if let Some(url) = &settings.portal_url {
        validate_portal_url(url)?;
    }
    ensure_parent_dir(path)?;
    let json = serde_json::to_string_pretty(settings).expect("gui settings always serialize");
    fs::write(path, json).map_err(|source| GuiBridgeError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// The one keyring *service* every device-token credential this app stores
/// is filed under, on every platform. See the module doc's "Device token
/// storage" note for why this is fixed rather than derived from anything
/// that could vary between runs or rebuilds.
pub const DEVICE_TOKEN_KEYRING_SERVICE: &str = "space.planastro.astro-sync";

/// Loads the paired device's bearer token, or `None` when no device has
/// been paired yet. `keyring_account` identifies the credential within
/// [`DEVICE_TOKEN_KEYRING_SERVICE`] -- callers pass the paired portal's URL
/// (the same value held in [`GuiSettings::portal_url`]).
///
/// Tries the OS keyring first. If it holds nothing for this account, the
/// legacy file at `path` is checked and, if it has a token, migrated into
/// the keyring (written there, then deleted from `path`, logged without
/// the token's value) before being returned. If the keyring itself is
/// unavailable or refuses the call, `path` is read directly instead, with
/// no migration attempted -- see the module doc's "Device token storage"
/// note.
pub fn load_device_token(
    path: &Path,
    keyring_account: &str,
) -> Result<Option<String>, GuiBridgeError> {
    match SystemKeyring::new(keyring_account) {
        Ok(keyring) => load_device_token_via(&keyring, path),
        Err(_) => load_device_token_file(path),
    }
}

/// Persists `token` under `keyring_account` (see [`load_device_token`]) in
/// the OS keyring. Falls back to the legacy 0600 file at `path` only if the
/// keyring is unavailable or refuses the write; on a successful keyring
/// write, any leftover legacy file is removed so the token exists in
/// exactly one place.
pub fn save_device_token(
    path: &Path,
    keyring_account: &str,
    token: &str,
) -> Result<(), GuiBridgeError> {
    match SystemKeyring::new(keyring_account) {
        Ok(keyring) => save_device_token_via(&keyring, path, token),
        Err(_) => save_device_token_file(path, token),
    }
}

/// Removes the stored device token, if any -- the "unpair this device"
/// action. Clears both the keyring entry for `keyring_account` (when given
/// -- a freshly installed, never-paired app has no portal URL to derive an
/// account from) and the legacy file at `path`. Missing in either place is
/// not an error. A keyring delete failure other than "no such entry" is
/// swallowed rather than propagated: unpairing must still succeed locally
/// even if the platform store refuses the delete.
pub fn clear_device_token(
    path: &Path,
    keyring_account: Option<&str>,
) -> Result<(), GuiBridgeError> {
    if let Some(account) = keyring_account
        && let Ok(keyring) = SystemKeyring::new(account)
    {
        return clear_device_token_via(&keyring, path);
    }
    clear_device_token_file(path)
}

/// The fallback/migration orchestration behind [`load_device_token`],
/// generic over [`KeyringBackend`] so it can be driven by [`SystemKeyring`]
/// in production and by a hand-rolled in-memory fake in tests -- see the
/// [`KeyringBackend`] doc for why the fake, not `keyring`'s own `mock`
/// module, is what exercises this particular logic.
fn load_device_token_via<B: KeyringBackend>(
    keyring: &B,
    path: &Path,
) -> Result<Option<String>, GuiBridgeError> {
    match keyring.load() {
        Ok(Some(token)) => Ok(Some(token)),
        Ok(None) => {
            let Some(token) = load_device_token_file(path)? else {
                return Ok(None);
            };
            if keyring.save(&token).is_ok() {
                clear_device_token_file(path)?;
                eprintln!("astro-sync: device token moved from the file to the system keyring.");
            }
            // If the keyring write failed, the file stays put and this
            // same migration is retried on the next call; either way the
            // token we just read from the file is the correct answer now.
            Ok(Some(token))
        }
        Err(GuiBridgeError::KeyringUnavailable(_)) => load_device_token_file(path),
        Err(other) => Err(other),
    }
}

/// The fallback orchestration behind [`save_device_token`]; see
/// [`load_device_token_via`].
fn save_device_token_via<B: KeyringBackend>(
    keyring: &B,
    path: &Path,
    token: &str,
) -> Result<(), GuiBridgeError> {
    if keyring.save(token).is_ok() {
        return clear_device_token_file(path);
    }
    save_device_token_file(path, token)
}

/// The cleanup orchestration behind [`clear_device_token`]; see
/// [`load_device_token_via`].
fn clear_device_token_via<B: KeyringBackend>(
    keyring: &B,
    path: &Path,
) -> Result<(), GuiBridgeError> {
    let _ = keyring.delete();
    clear_device_token_file(path)
}

/// Loads the paired device's bearer token from `path` alone, or `None` when
/// no device has been paired yet (a missing file is not an error). The
/// legacy-file half of the storage scheme described in the module doc;
/// callers outside this module should use [`load_device_token`] instead.
fn load_device_token_file(path: &Path) -> Result<Option<String>, GuiBridgeError> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path).map_err(|source| GuiBridgeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let token = raw.trim().to_string();
    Ok(if token.is_empty() { None } else { Some(token) })
}

#[cfg(unix)]
fn restrict_to_owner(path: &Path) -> Result<(), GuiBridgeError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|source| {
        GuiBridgeError::Io {
            path: path.to_path_buf(),
            source,
        }
    })
}

#[cfg(not(unix))]
fn restrict_to_owner(_path: &Path) -> Result<(), GuiBridgeError> {
    // No portable, dependency-free equivalent of `chmod 0600` on this
    // platform.7 only requires `cargo tauri build` on macOS.
    Ok(())
}

/// Persists `token` at `path` and immediately restricts the file to owner
/// read/write only (see the module doc's "Device token storage" note). The
/// legacy-file half of the storage scheme; callers outside this module
/// should use [`save_device_token`] instead.
fn save_device_token_file(path: &Path, token: &str) -> Result<(), GuiBridgeError> {
    ensure_parent_dir(path)?;
    fs::write(path, token).map_err(|source| GuiBridgeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    restrict_to_owner(path)
}

/// Removes the token file, if any. The legacy-file half of the storage
/// scheme; callers outside this module should use [`clear_device_token`]
/// instead.
fn clear_device_token_file(path: &Path) -> Result<(), GuiBridgeError> {
    if path.exists() {
        fs::remove_file(path).map_err(|source| GuiBridgeError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

/// A place to save/load/delete one device token, behind a trait purely so
/// the fallback/migration orchestration in [`load_device_token_via`]/
/// [`save_device_token_via`]/[`clear_device_token_via`] can be unit-tested
/// deterministically, the same way [`PortalGuiTransport`] and
/// [`UploadTransport`] are elsewhere in this module.
///
/// [`SystemKeyring`] is the only production implementation. It is *not*
/// exercised through this trait in tests -- `keyring`'s own `mock` module
/// (always compiled in; see the module doc's "Device token storage" note)
/// is used directly against [`SystemKeyring`] instead, because that is
/// what genuinely proves its `set_password`/`get_password`/
/// `delete_credential` calls and its `NoEntry` handling are correct.
/// `keyring::mock` cannot, however, stand in for a real *store* here: by
/// its own module doc's design ("provides no persistence"), every fresh
/// `keyring::Entry::new` call it backs gets its own empty, unshared
/// credential, so it cannot simulate the one thing this orchestration
/// logic needs to be tested against -- a save through one call being
/// visible to a *later, separate* load. A small hand-rolled fake (see this
/// file's `tests` module) gives that instead.
trait KeyringBackend {
    fn save(&self, token: &str) -> Result<(), GuiBridgeError>;
    fn load(&self) -> Result<Option<String>, GuiBridgeError>;
    fn delete(&self) -> Result<(), GuiBridgeError>;
}

/// One credential in the OS-native secure store, addressed by
/// [`DEVICE_TOKEN_KEYRING_SERVICE`] and an `account` (the paired portal's
/// URL) fixed at construction. Every method maps [`keyring::Error::NoEntry`]
/// to `Ok(None)`/`Ok(())` -- there being no credential yet is not a failure
/// -- and every other [`keyring::Error`] to
/// [`GuiBridgeError::KeyringUnavailable`], the signal
/// [`load_device_token_via`] and friends use to fall back to the legacy
/// file.
struct SystemKeyring {
    entry: keyring::Entry,
}

impl SystemKeyring {
    fn new(account: &str) -> Result<Self, GuiBridgeError> {
        let entry = keyring::Entry::new(DEVICE_TOKEN_KEYRING_SERVICE, account)
            .map_err(keyring_unavailable)?;
        Ok(Self { entry })
    }
}

impl KeyringBackend for SystemKeyring {
    fn save(&self, token: &str) -> Result<(), GuiBridgeError> {
        self.entry.set_password(token).map_err(keyring_unavailable)
    }

    fn load(&self) -> Result<Option<String>, GuiBridgeError> {
        match self.entry.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(keyring_unavailable(error)),
        }
    }

    fn delete(&self) -> Result<(), GuiBridgeError> {
        match self.entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(keyring_unavailable(error)),
        }
    }
}

fn keyring_unavailable(error: keyring::Error) -> GuiBridgeError {
    GuiBridgeError::KeyringUnavailable(error.to_string())
}

fn is_valid_pairing_code(code: &str) -> bool {
    const ALPHABET: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    code.len() == 8
        && code
            .chars()
            .all(|c| ALPHABET.contains(c.to_ascii_uppercase()))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClaimRequestBody<'a> {
    code: &'a str,
    label: &'a str,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaimResponseBody {
    device_token: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ErrorEnvelope {
    error: ErrorDetail,
}

#[derive(Debug, Clone, Deserialize)]
struct ErrorDetail {
    code: String,
    message: String,
}

/// A session as `GET /v1/sessions` reports it
/// (`contracts/schemas/api/session.schema.json`'s
/// `resource`). Only the fields the session-list screen shows; nothing is
/// re-derived or reinterpreted here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub plan_id: Option<String>,
    /// `"open"` / `"paused"` / `"finished"`.
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub event_count: u64,
    #[serde(default)]
    pub supplement_count: u64,
    #[serde(default)]
    pub pending_supplement_count: u64,
    /// The night's date (`YYYY-MM-DD`): the plan's date, else the evening of
    /// the start at the place's longitude (`session.schema.json`'s
    /// `eveningDate`). The window names a night by it, never by the calendar
    /// day of the computer's time zone. A service of an earlier version does
    /// not send it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evening_date: Option<String>,
    /// How the night turned out, taken from the debriefing of the session's
    /// plan (`session.schema.json`'s `outcome`): `"executed"`, `"partial"`,
    /// `"abandoned"`, or `None` while the session says nothing (not debriefed
    /// yet, or no plan). A service of an earlier version does not send it,
    /// which reads as `None`, as the contract says. Astro Sync reads it to
    /// hold back the treatment of a session that is still to debrief.
    #[serde(default)]
    pub outcome: Option<String>,
}

/// `GET /v1/sessions`'s response envelope (`session.schema.json`'s
/// `list`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionListPage {
    pub sessions: Vec<SessionSummary>,
    #[serde(default)]
    pub next_since: Option<String>,
    #[serde(default)]
    pub has_more: bool,
}

/// `POST /v1/sessions/{id}/analyses`'s answer (`session-analysis.schema.json`,
/// `createResponse`): the night's one analysis, whether it replaced an earlier
/// one, and the night's steps after the sending. The report itself is not read
/// back (no `deny_unknown_fields`): the caller holds the one it sent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UploadedAnalysis {
    pub analysis: UploadedAnalysisResource,
    pub replaced: bool,
    pub steps: UploadedAnalysisSteps,
}

/// The analysis in place after a sending: when it was sent and from which
/// device.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UploadedAnalysisResource {
    pub id: String,
    pub session_id: String,
    pub report_version: u32,
    pub created_at: String,
    #[serde(default)]
    pub origin_device_id: Option<String>,
    #[serde(default)]
    pub device_name: Option<String>,
}

/// The night's steps after a sending: `proposed` written from the report,
/// `kept_edited` corrected by hand and kept.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UploadedAnalysisSteps {
    pub proposed: u64,
    pub kept_edited: u64,
}

/// `GET /v1/capabilities`'s response, exactly the neutral capability map
/// `contracts/schemas/api/capabilities.schema.json` describes --
/// dotted string keys, primitive boolean/integer values only, no product
/// name, tenant, user, device, or bearer field. `deny_unknown_fields`: a
/// response carrying anything other than exactly these ten fields is a
/// contract violation this GUI should surface as an error rather than
/// silently accept.
///
/// The one capability this GUI reads today is
/// [`Capabilities::analysis_logs`] -- it gates both the Analysis screen
/// (uploading a local-analysis report) and the Progression screen
/// (automatic Siril progressions). The contract lists the automatic Siril
/// progression under its own `analysis.progression` capability, distinct from
/// `analysis.logs`, but the two are granted together, so a person never sees
/// one without the other regardless of which single capability this GUI keys
/// its gate on. `analysis.logs` is used for both screens here because it is
/// the capability the Analysis screen's own upload path already checks
/// server-side (`analysis_not_entitled`) -- gating the Progression screen on
/// the same name keeps the GUI's one local gate consistent with the one
/// refusal code it already has to handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Capabilities {
    #[serde(rename = "planner.multiple")]
    pub planner_multiple: bool,
    #[serde(rename = "planner.mosaic")]
    pub planner_mosaic: bool,
    /// The plate-solving aiming capability. Astro Sync never reads it;
    /// it is here because the map is strict (`deny_unknown_fields`).
    #[serde(rename = "pointing.solve")]
    pub pointing_solve: bool,
    #[serde(rename = "sync.plan_limit")]
    pub sync_plan_limit: u64,
    #[serde(rename = "gallery.private")]
    pub gallery_private: bool,
    /// The invitations to the private gallery of a space. Astro Sync never
    /// reads it; it is here because the map is strict (`deny_unknown_fields`).
    #[serde(rename = "portal.invitations")]
    pub portal_invitations: bool,
    #[serde(rename = "capture.variant_limit")]
    pub capture_variant_limit: u64,
    /// How many captures the space may hold at a time. Astro Sync never reads
    /// it (it reacts to the refusal instead); it is here because the map is
    /// strict.
    #[serde(rename = "capture.count_limit")]
    pub capture_count_limit: u64,
    /// Whether a capture of the space may be made public. Never read here
    /// either; the map is strict.
    #[serde(rename = "capture.public")]
    pub capture_public: bool,
    #[serde(rename = "analysis.logs")]
    pub analysis_logs: bool,
    #[serde(rename = "analysis.progression")]
    pub analysis_progression: bool,
    #[serde(rename = "storage.max_file_bytes")]
    pub storage_max_file_bytes: u64,
    #[serde(rename = "storage.quota_bytes")]
    pub storage_quota_bytes: u64,
    #[serde(rename = "history.retention_days")]
    pub history_retention_days: u64,
}

/// A [`Capabilities`] snapshot as cached on disk, timestamped with when it
/// was fetched: "cached locally (file in the existing state directory,
/// timestamped)". The timestamp is kept for operator/support visibility
/// (e.g. "this profile was last confirmed three weeks ago") and future
/// staleness policy; today's policy is the simplest one the plan asks for
/// -- valid offline indefinitely, replaced wholesale the next time a
/// network refresh succeeds, never expired by age alone (a paired device
/// that stays offline for a long time must keep working exactly as before,
/// not silently lose access to screens it was already entitled to).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitiesCache {
    pub capabilities: Capabilities,
    pub fetched_at_utc: String,
    /// What the billing of the space limits (a payment that failed: the space
    /// is read-only for a while), read from `GET /v1/entitlements` at the same
    /// time. Absent in a cache written before it existed, and when nothing
    /// limits the space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standing: Option<Standing>,
}

/// Loads a previously cached [`CapabilitiesCache`] from `path`. A missing
/// file is not an error -- it is simply "never successfully fetched yet"
/// (a freshly paired device, or one paired before this cache existed) --
/// on the same convention as [`load_settings`].
pub fn load_capabilities_cache(path: &Path) -> Result<Option<CapabilitiesCache>, GuiBridgeError> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path).map_err(|source| GuiBridgeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|source| GuiBridgeError::InvalidSettings {
            path: path.to_path_buf(),
            source,
        })
}

/// Persists `cache` at `path` as pretty JSON, creating parent directories
/// as needed -- on the same convention as [`save_settings`]. Not secret
/// (unlike the device token): a capability profile is not sensitive, so
/// this is a plain file, no keyring/owner-only permission involved.
pub fn save_capabilities_cache(
    path: &Path,
    cache: &CapabilitiesCache,
) -> Result<(), GuiBridgeError> {
    ensure_parent_dir(path)?;
    let json = serde_json::to_string_pretty(cache).expect("capabilities cache always serializes");
    fs::write(path, json).map_err(|source| GuiBridgeError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// The portal calls the GUI needs that neither `uploader` nor any existing
/// module covers, behind a trait so tests never touch the network -- the
/// same seam `crate::uploader::UploadTransport` uses.
pub trait PortalGuiTransport {
    /// `POST /v1/devices/claim`: exchanges a pairing code for a device
    /// bearer token. Returns the raw token; the caller decides where (and
    /// whether) to persist it.
    fn claim_device(
        &self,
        base_url: &str,
        code: &str,
        label: &str,
    ) -> Result<String, GuiBridgeError>;

    /// `GET /v1/sessions?since=&limit=`.
    fn list_sessions(
        &self,
        base_url: &str,
        token: &str,
        since: Option<&str>,
        limit: Option<u32>,
    ) -> Result<SessionListPage, GuiBridgeError>;

    /// `POST /v1/sessions/{session_id}/analyses` (contract checkpoint):
    /// uploads a local-analysis report
    /// (`crate::analysis::report::SessionReport`, already rendered to JSON
    /// by the caller -- this trait stays agnostic of that type, exactly
    /// like `crate::uploader::UploadTransport` never depends on
    /// `crate::package`) as a new, immutable version. A `403`
    /// `analysis_not_entitled` refusal (the account's capability profile
    /// does not grant `analysis.logs`) surfaces as
    /// [`GuiBridgeError::Refused`] like any other portal refusal; the
    /// Analysis screen matches on its `code` for a clear, localized
    /// message.
    fn upload_analysis(
        &self,
        base_url: &str,
        token: &str,
        session_id: &str,
        report: &serde_json::Value,
    ) -> Result<UploadedAnalysis, GuiBridgeError>;

    /// `GET /v1/capabilities`: the paired portal's complete, composed
    /// neutral capability map for this device's account (`v1.md`,
    /// "Capability enforcement" -- device-token authenticated, no
    /// product name, tenant, user, or device field in the response
    /// itself). Read by [`refresh_capabilities`] to gate the Progression
    /// and Analysis screens.
    fn fetch_capabilities(
        &self,
        base_url: &str,
        token: &str,
    ) -> Result<Capabilities, GuiBridgeError>;

    /// `GET /v1/entitlements`, read for one thing: whether the payment of the
    /// space's subscription failed, and until when the space is read-only
    /// ([`Standing`]). Read by [`refresh_capabilities`] with the capabilities.
    /// A transport that does not know the route says nothing.
    fn fetch_standing(
        &self,
        _base_url: &str,
        _token: &str,
    ) -> Result<Option<Standing>, GuiBridgeError> {
        Ok(None)
    }
}

/// The real transport: `reqwest` blocking client over `rustls`, waiting
/// patiently for a sleeping service ([`PatientHttp`]).
///
/// Which call may be sent again while the service wakes is decided call by
/// call, on what the contract says about repeating it:
///
/// - `list_sessions`, `fetch_capabilities`, `fetch_standing`: reads -- retried
///   freely.
/// - `claim_device`: consumes a single-use pairing code (a second claim of
///   the same code is a `409`), so a repeat could lose the device token:
///   never retried. The service is woken first by a read that has no effect
///   (the public identity route), then the claim is sent exactly once.
/// - `upload_analysis`: every `POST` replaces the night's analysis and
///   rewrites its proposed steps: never retried, same wake-then-send-once
///   rule.
pub struct HttpPortalGuiTransport {
    http: PatientHttp,
}

impl HttpPortalGuiTransport {
    pub fn new() -> Result<Self, GuiBridgeError> {
        Self::with_options(WakePolicy::default(), WaitHooks::silent())
    }

    /// Like [`HttpPortalGuiTransport::new`], with an explicit waiting policy
    /// (tests shorten it) and the hooks through which a caller shows the
    /// "the service is starting" state and can cancel the wait.
    pub fn with_options(policy: WakePolicy, hooks: WaitHooks) -> Result<Self, GuiBridgeError> {
        let http = PatientHttp::new(policy, hooks).map_err(GuiBridgeError::ClientBuild)?;
        Ok(Self { http })
    }

    fn url(base_url: &str, path: &str) -> String {
        format!("{}{}", base_url.trim_end_matches('/'), path)
    }

    /// Sends a request whose repeat is harmless (a read, or a check that
    /// writes nothing) and reads its JSON answer; a refusal keeps its code.
    /// Used by the routes of `crate::night_online` and `crate::precheck`.
    pub(crate) fn read_json<T: serde::de::DeserializeOwned>(
        &self,
        build: &RequestFactory,
    ) -> Result<T, GuiBridgeError> {
        let answer = self
            .http
            .send_safe(self.http.policy().attempt_timeout, build)?;
        if answer.is_success() {
            return parse_body(&answer);
        }
        Err(refusal(&answer))
    }

    /// Sends a write that must not be repeated (a creation) once, after
    /// waking the service, and reads its JSON answer; a refusal keeps its
    /// code (`POST /v1/targets/personal`).
    pub(crate) fn write_json_once<T: serde::de::DeserializeOwned>(
        &self,
        base_url: &str,
        build: &(dyn Fn(&reqwest::blocking::Client) -> reqwest::blocking::RequestBuilder + Sync),
    ) -> Result<T, GuiBridgeError> {
        self.http.wake(base_url)?;
        let answer = self
            .http
            .send_once(self.http.policy().write_timeout, build)?;
        if answer.is_success() {
            return parse_body(&answer);
        }
        Err(refusal(&answer))
    }

    /// The same for a file (`GET /v1/captures/{id}/original`): the status
    /// and the bytes, with the time a large file takes.
    pub(crate) fn read_bytes(
        &self,
        build: &RequestFactory,
    ) -> Result<(u16, Vec<u8>), GuiBridgeError> {
        let answer = self
            .http
            .send_safe(self.http.policy().chunk_timeout, build)?;
        if answer.is_success() {
            return Ok((answer.status, answer.body));
        }
        Err(refusal(&answer))
    }
}

fn read_error_envelope(answer: &HttpAnswer) -> (String, String) {
    let text = answer.text();
    match serde_json::from_str::<ErrorEnvelope>(&text) {
        Ok(envelope) => (envelope.error.code, envelope.error.message),
        Err(_) => ("invalid_response".to_string(), text),
    }
}

fn parse_body<T: serde::de::DeserializeOwned>(answer: &HttpAnswer) -> Result<T, GuiBridgeError> {
    serde_json::from_slice(&answer.body)
        .map_err(|error| GuiBridgeError::InvalidResponse(error.to_string()))
}

fn refusal(answer: &HttpAnswer) -> GuiBridgeError {
    let (code, message) = read_error_envelope(answer);
    // The service says this computer is no longer attached to its space: not
    // a refusal of this one request.
    if answer.status == 401
        && let Some(end) = SignInEnd::from_code(&code)
    {
        return GuiBridgeError::SignInEnded(end);
    }
    GuiBridgeError::Refused { code, message }
}

impl PortalGuiTransport for HttpPortalGuiTransport {
    fn claim_device(
        &self,
        base_url: &str,
        code: &str,
        label: &str,
    ) -> Result<String, GuiBridgeError> {
        let url = Self::url(base_url, "/api/v1/devices/claim");
        self.http.wake(base_url)?;
        let answer = self
            .http
            .send_once(self.http.policy().write_timeout, &|client| {
                client
                    .post(url.clone())
                    .json(&ClaimRequestBody { code, label })
            })?;
        if answer.is_success() {
            let body: ClaimResponseBody = parse_body(&answer)?;
            return Ok(body.device_token);
        }
        Err(refusal(&answer))
    }

    fn list_sessions(
        &self,
        base_url: &str,
        token: &str,
        since: Option<&str>,
        limit: Option<u32>,
    ) -> Result<SessionListPage, GuiBridgeError> {
        let url = Self::url(base_url, "/api/v1/sessions");
        let token = token.to_owned();
        let mut query: Vec<(&'static str, String)> = Vec::new();
        if let Some(since) = since {
            query.push(("since", since.to_string()));
        }
        if let Some(limit) = limit {
            query.push(("limit", limit.to_string()));
        }
        let build: RequestFactory = Arc::new(move |client| {
            let request = client.get(url.clone()).bearer_auth(&token);
            if query.is_empty() {
                request
            } else {
                request.query(&query)
            }
        });
        let answer = self
            .http
            .send_safe(self.http.policy().attempt_timeout, &build)?;
        if answer.is_success() {
            return parse_body(&answer);
        }
        Err(refusal(&answer))
    }

    fn upload_analysis(
        &self,
        base_url: &str,
        token: &str,
        session_id: &str,
        report: &serde_json::Value,
    ) -> Result<UploadedAnalysis, GuiBridgeError> {
        let url = Self::url(base_url, &format!("/api/v1/sessions/{session_id}/analyses"));
        self.http.wake(base_url)?;
        let answer = self
            .http
            .send_once(self.http.policy().chunk_timeout, &|client| {
                client.post(url.clone()).bearer_auth(token).json(report)
            })?;
        if answer.is_success() {
            return parse_body(&answer);
        }
        Err(refusal(&answer))
    }

    fn fetch_capabilities(
        &self,
        base_url: &str,
        token: &str,
    ) -> Result<Capabilities, GuiBridgeError> {
        let url = Self::url(base_url, "/api/v1/capabilities");
        let token = token.to_owned();
        let build: RequestFactory =
            Arc::new(move |client| client.get(url.clone()).bearer_auth(&token));
        let answer = self
            .http
            .send_safe(self.http.policy().attempt_timeout, &build)?;
        if answer.is_success() {
            return parse_body(&answer);
        }
        Err(refusal(&answer))
    }

    fn fetch_standing(
        &self,
        base_url: &str,
        token: &str,
    ) -> Result<Option<Standing>, GuiBridgeError> {
        let url = Self::url(base_url, "/api/v1/entitlements");
        let token = token.to_owned();
        let build: RequestFactory =
            Arc::new(move |client| client.get(url.clone()).bearer_auth(&token));
        let answer = self
            .http
            .send_safe(self.http.policy().attempt_timeout, &build)?;
        if answer.is_success() {
            let body: serde_json::Value = parse_body(&answer)?;
            return standing_of_entitlements(&body).map_err(GuiBridgeError::InvalidResponse);
        }
        Err(refusal(&answer))
    }
}

/// Validates `code`/`device_label`, then exchanges the pairing code for a
/// device bearer token through `transport`. Validation happens here (not
/// only server-side) so the Settings screen can show an inline error
/// before a request ever leaves the machine.
pub fn claim_device<T: PortalGuiTransport>(
    transport: &T,
    portal_url: &str,
    code: &str,
    device_label: &str,
) -> Result<String, GuiBridgeError> {
    validate_portal_url(portal_url)?;
    let normalized_code = code.trim().to_ascii_uppercase();
    if !is_valid_pairing_code(&normalized_code) {
        return Err(GuiBridgeError::InvalidPairingCode);
    }
    let label = device_label.trim();
    if label.is_empty() {
        return Err(GuiBridgeError::EmptyDeviceLabel);
    }
    transport.claim_device(portal_url, &normalized_code, label)
}

/// Fetches one page of the portal's observation-session list.
pub fn list_sessions<T: PortalGuiTransport>(
    transport: &T,
    portal_url: &str,
    token: &str,
    since: Option<&str>,
    limit: Option<u32>,
) -> Result<SessionListPage, GuiBridgeError> {
    validate_portal_url(portal_url)?;
    transport.list_sessions(portal_url, token, since, limit)
}

/// The report as it leaves the computer (only results leave):
/// the measured rows keep their numbers and lose `sourcePath`, the name of
/// the frame each was read from (folder of the target, link, the file's own
/// name with its date, time and temperature). The local report keeps it.
#[must_use]
pub fn report_for_sending(report: &serde_json::Value) -> serde_json::Value {
    let mut sent = report.clone();
    if let Some(rows) = sent
        .get_mut("quality")
        .and_then(serde_json::Value::as_array_mut)
    {
        for row in rows {
            if let Some(row) = row.as_object_mut() {
                row.remove("sourcePath");
            }
        }
    }
    sent
}

/// Uploads one local-analysis report as a new version of `session_id`
/// (contract checkpoint). `report` is already-rendered JSON (typically
/// `analysis::report::render_report_json`'s own output, parsed back into a
/// `serde_json::Value` -- this module takes no dependency on
/// `analysis::report`'s types, on the same doctrine
/// `crate::uploader::UploadTransport` already follows for `crate::package`).
pub fn upload_analysis<T: PortalGuiTransport>(
    transport: &T,
    portal_url: &str,
    token: &str,
    session_id: &str,
    report: &serde_json::Value,
) -> Result<UploadedAnalysis, GuiBridgeError> {
    validate_portal_url(portal_url)?;
    transport.upload_analysis(portal_url, token, session_id, &report_for_sending(report))
}

/// One night's measured sky background at one of the account's places, as
/// the service keeps it (`POST`/`GET /v1/place-sky-measures`;
/// `contracts/schemas/api/place-sky-measure.schema.json`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlaceSkyMeasure {
    pub place_id: String,
    pub night: String,
    pub sqm: f64,
    pub pose_count: u64,
    pub method_version: String,
    pub observed_at: String,
    #[serde(default)]
    pub session_id: Option<String>,
    pub updated_at: String,
}

/// `POST /v1/place-sky-measures`'s answer: the stored value, and whether it
/// replaced an earlier one for the same place and night.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UploadedSkyMeasure {
    pub measure: PlaceSkyMeasure,
    pub replaced: bool,
}

/// The upload of a night's measured sky background, apart from
/// [`PortalGuiTransport`] so that its fakes stay as they are. Free: the
/// route asks for no capability, unlike the analysis upload.
pub trait SkyMeasureTransport {
    /// `POST /v1/place-sky-measures` with the body
    /// `analysis::sky_night::upload_body` builds.
    fn upload_place_sky_measure(
        &self,
        base_url: &str,
        token: &str,
        body: &serde_json::Value,
    ) -> Result<UploadedSkyMeasure, GuiBridgeError>;
}

impl SkyMeasureTransport for HttpPortalGuiTransport {
    /// Sending the same night twice stores the same value once (the later
    /// replaces the earlier, `api/v1.md`): the request may be repeated while
    /// the service wakes, like a read.
    fn upload_place_sky_measure(
        &self,
        base_url: &str,
        token: &str,
        body: &serde_json::Value,
    ) -> Result<UploadedSkyMeasure, GuiBridgeError> {
        let url = Self::url(base_url, "/api/v1/place-sky-measures");
        let token = token.to_owned();
        let body = body.clone();
        let build: RequestFactory =
            Arc::new(move |client| client.post(url.clone()).bearer_auth(&token).json(&body));
        let answer = self
            .http
            .send_safe(self.http.policy().write_timeout, &build)?;
        if answer.is_success() {
            return parse_body(&answer);
        }
        Err(refusal(&answer))
    }
}

/// Uploads one night's measured sky background (`body`, from
/// `analysis::sky_night::upload_body`) to the paired service.
pub fn upload_place_sky_measure<T: SkyMeasureTransport>(
    transport: &T,
    portal_url: &str,
    token: &str,
    body: &serde_json::Value,
) -> Result<UploadedSkyMeasure, GuiBridgeError> {
    validate_portal_url(portal_url)?;
    transport.upload_place_sky_measure(portal_url, token, body)
}

/// The capability-refresh policy in one call: try the network first, cache a
/// fresh, timestamped result on success; on ANY failure (no network, the
/// portal unreachable, a refused/invalid response), fall back to whatever was
/// last cached at `cache_path`, so a paired device that cannot currently reach
/// its portal keeps whatever capability-gated screens it already had, unchanged
/// (they stay valid offline). Only when there is neither a fresh answer nor any
/// cache at all (a freshly paired device whose very first refresh fails) does
/// this return the network error -- the caller (`app/src-tauri`'s
/// `capabilities` command) treats that the same as "capability unknown".
///
/// A cache write failure after a successful fetch does not fail this call:
/// the freshly fetched profile is still correct to use for the running
/// session, only not persisted for the next offline launch.
pub fn refresh_capabilities<T: PortalGuiTransport>(
    transport: &T,
    cache_path: &Path,
    portal_url: &str,
    token: &str,
) -> Result<CapabilitiesCache, GuiBridgeError> {
    validate_portal_url(portal_url)?;
    match transport.fetch_capabilities(portal_url, token) {
        Ok(capabilities) => {
            // The standing is read beside the capabilities. A service that
            // answers (even by a refusal: a private server has no
            // subscription to speak of) is believed; one that cannot be
            // reached leaves the standing the cache already had.
            let standing = match transport.fetch_standing(portal_url, token) {
                Ok(standing) => standing,
                Err(GuiBridgeError::Refused { .. }) => None,
                Err(_) => load_capabilities_cache(cache_path)
                    .ok()
                    .flatten()
                    .and_then(|cached| cached.standing),
            };
            let cache = CapabilitiesCache {
                capabilities,
                fetched_at_utc: crate::time_util::now_iso8601_utc(),
                standing,
            };
            let _ = save_capabilities_cache(cache_path, &cache);
            Ok(cache)
        }
        // The service says this computer has left its space (detached, or
        // the space deleted): a cached profile of that space must not be
        // served as if it were still current. (A token that was only
        // replaced, or revoked on a private server, leaves the cache
        // alone, as any refusal always did.)
        Err(ended @ GuiBridgeError::SignInEnded(end)) if end.forgets_the_space() => Err(ended),
        Err(network_error) => match load_capabilities_cache(cache_path) {
            Ok(Some(cached)) => Ok(cached),
            // No cache yet, or the cached file is unreadable/corrupt: in
            // either case there is nothing usable to fall back to, so the
            // original network failure is what the caller should see.
            Ok(None) | Err(_) => Err(network_error),
        },
    }
}

/// How the capability refresh at startup should wait, given what is cached:
/// `(policy, announce)`.
///
/// With a cached profile the refresh is a courtesy the application must not
/// wait for -- a device that cannot reach its portal keeps the profile it
/// had, and a sleeping service must not hold the window up for a minute --
/// so it makes one short attempt and says nothing (the attempt still
/// reaches the service, which starts waking for whatever the person does
/// next). With no cached profile there is nothing else to show, so it waits
/// as patiently as any other call and announces the wait.
#[must_use]
pub fn capabilities_refresh_plan(
    cache_path: &Path,
    patient: &WakePolicy,
    background: &WakePolicy,
) -> (WakePolicy, bool) {
    match load_capabilities_cache(cache_path) {
        Ok(Some(_)) => (background.clone(), false),
        Ok(None) | Err(_) => (patient.clone(), true),
    }
}

/// Per-state and per-byte counts for the upload-queue screen, computed
/// straight from the rows [`UploadQueue::list`] already returned -- no
/// second query, no logic duplicated on the TypeScript side.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct QueueSummary {
    pub total: usize,
    pub queued: usize,
    pub uploading: usize,
    pub completed: usize,
    pub failed: usize,
    /// Files the service refuses for good (`QueueState::Refused`).
    pub refused: usize,
    /// Files waiting for the person to act (`QueueState::Held`).
    pub held: usize,
    pub total_bytes: u64,
    /// Bytes durably accepted so far: full size for `completed`, the
    /// recorded server offset (never more than the file's size) for
    /// `uploading` and `held`, nothing yet for `queued`/`failed`/`refused`.
    pub sent_bytes: u64,
}

#[must_use]
pub fn summarize_queue(entries: &[QueueEntry]) -> QueueSummary {
    let mut summary = QueueSummary::default();
    for entry in entries {
        summary.total += 1;
        summary.total_bytes += entry.size;
        match entry.state {
            QueueState::Queued => summary.queued += 1,
            QueueState::Uploading => {
                summary.uploading += 1;
                summary.sent_bytes += entry.offset.min(entry.size);
            }
            QueueState::Completed => {
                summary.completed += 1;
                summary.sent_bytes += entry.size;
            }
            QueueState::Failed => summary.failed += 1,
            QueueState::Refused => summary.refused += 1,
            QueueState::Held => {
                summary.held += 1;
                summary.sent_bytes += entry.offset.min(entry.size);
            }
        }
    }
    summary
}

/// One step of [`drive_queue`]'s progress, meant to be turned directly into
/// a Tauri event payload. The wire shape is what the front end
/// (`app/src/lib/bridge.ts`, `QueueDriveEvent`) reads: `kind` is the variant
/// name as written here (`"Started"`, `"Completed"`, ...) and every field is
/// camelCase -- `rename_all_fields`, not `rename_all`, which would rename the
/// variants instead and leave `capture_id` in snake_case.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all_fields = "camelCase")]
pub enum QueueDriveEvent {
    Started {
        sha256: String,
        source_path: PathBuf,
        bytes: u64,
    },
    Progress {
        sha256: String,
        sent: u64,
        total: u64,
    },
    Completed {
        sha256: String,
        capture_id: String,
        deduplicated: bool,
        /// What the service left of the capture metadata Astro Sync
        /// proposed (the owner's own values), empty most of the time
        /// (then not written).
        #[serde(skip_serializing_if = "Vec::is_empty")]
        skipped: Vec<crate::metadata::SkippedField>,
        /// The image replaced the one its capture had (the phone's photo of
        /// the session): not written when it did not.
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        replaced: bool,
    },
    /// This file failed for good (or the run cannot go on: the service ended
    /// the sign-in, the local queue database failed). When the file itself is
    /// at fault the queue entry is already marked `failed` (by
    /// [`upload_entry`]) and the run continues with the next entry.
    Failed {
        sha256: String,
        /// A stable code the interface translates (`errors.<code>`), so a
        /// person never has to read the technical `message`.
        code: String,
        message: String,
    },
    /// The service refuses this file for good (an SVG or HTML document, a
    /// file over the size the space allows, a refusal this Astro Sync does
    /// not know): the entry is already marked `refused` with `code`, it will
    /// not be sent again, the file on the computer is untouched, and the run
    /// continues with the next entry.
    Refused {
        sha256: String,
        code: String,
        message: String,
    },
    /// The person has to act before anything more can be sent (the space's
    /// storage is full, its subscription has ended): the entry is `held`
    /// with its session and offset, and the run stops right after with
    /// [`StopReason::Held`]. Everything else would be refused the same way.
    Held {
        sha256: String,
        code: String,
        message: String,
    },
    /// Not the file's fault, and it will pass (a network that is down, a
    /// service that is starting or failing, too many uploads open in the
    /// space): the entry stays queued and this run tries it again in
    /// `retry_in_seconds`, by itself. `attempt` counts the tries so far; the
    /// wait grows with it ([`Backoff`]). `code` says why, as a failure would.
    Waiting {
        sha256: String,
        code: String,
        message: String,
        retry_in_seconds: u64,
        attempt: u32,
    },
    /// Emitted once, right before `drive_queue` returns early, either
    /// because the run cannot make progress on the same entry any more
    /// ([`StopReason::NoProgress`]), because `should_continue` returned
    /// `false` (a paused run), because the person cancelled a wait, or
    /// because the run needs the person to act ([`StopReason::Held`]). Either
    /// way the queue is left exactly as progress had recorded it, ready to
    /// resume on the next call.
    Stopped {
        sha256: Option<String>,
        reason: StopReason,
    },
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    QueueEmpty,
    Paused,
    NoProgress,
    /// The person cancelled the wait for a sleeping service.
    Cancelled,
    /// A file is held for the person's action ([`QueueDriveEvent::Held`]).
    Held,
}

/// How long a run waits before it tries a file again after a failure that is
/// not the file's fault: the first wait, then longer each time, then the
/// last one for as long as it takes. Waiting longer is the point -- a
/// service that is failing or full is not helped by being asked every few
/// seconds, and a person's laptop should not keep a radio busy for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backoff {
    steps: Vec<Duration>,
}

/// The longest a run honours a `Retry-After` for: past it the run still asks
/// again, at its own pace.
const LONGEST_RETRY_AFTER: Duration = Duration::from_secs(3600);

impl Backoff {
    /// 15 seconds, 30 seconds, 1 minute, 2 minutes, 5 minutes, 10 minutes,
    /// then every 15 minutes.
    #[must_use]
    pub fn standard() -> Self {
        Self::new(
            [15, 30, 60, 120, 300, 600, 900]
                .into_iter()
                .map(Duration::from_secs)
                .collect(),
        )
    }

    /// These waits, then the last one again and again. An empty list is one
    /// second: a run never spins.
    #[must_use]
    pub fn new(mut steps: Vec<Duration>) -> Self {
        if steps.is_empty() {
            steps.push(Duration::from_secs(1));
        }
        Self { steps }
    }

    /// The wait before the `attempt`-th new try (the first is `1`).
    #[must_use]
    pub fn wait(&self, attempt: u32) -> Duration {
        let index = usize::try_from(attempt.saturating_sub(1)).unwrap_or(usize::MAX);
        self.steps[index.min(self.steps.len() - 1)]
    }
}

/// How [`drive_queue`] behaves when a failure is not the file's fault and will
/// pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveOptions {
    pub backoff: Backoff,
    /// How many times a file is tried again before the run stops on it and
    /// leaves it queued for the next run; `None` for as long as the person
    /// does not pause (the window).
    pub max_replays: Option<u32>,
    /// Whether a service that **does not answer** (a network that is down, a
    /// service still starting after the whole wait) is waited for too, or
    /// ends the run at once. A service that answers -- it fails, or says
    /// something is full -- is always waited for. The window waits for both:
    /// a laptop's network comes back. The command line does not: an address
    /// that cannot be reached fails at once, as it always did, and the next
    /// run resumes.
    pub replay_when_unanswered: bool,
}

impl DriveOptions {
    /// The window: tries again for as long as it takes, waiting longer each
    /// time, until the person pauses.
    #[must_use]
    pub fn interactive() -> Self {
        Self {
            backoff: Backoff::standard(),
            max_replays: None,
            replay_when_unanswered: true,
        }
    }

    /// The command line: the same waits for a service that answers, but a
    /// bounded number of tries -- a script must end -- and none at all for a
    /// service that does not answer. The file stays queued for the next run.
    #[must_use]
    pub fn command_line() -> Self {
        Self {
            backoff: Backoff::standard(),
            max_replays: Some(4),
            replay_when_unanswered: false,
        }
    }
}

/// Waits `total`, looking at `should_continue` every tenth of a second so a
/// pause ends the wait at once; `false` when it did.
fn wait_unless_stopped(total: Duration, should_continue: &mut impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + total;
    loop {
        if !should_continue() {
            return false;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return true;
        }
        std::thread::sleep(left.min(Duration::from_millis(100)));
    }
}

/// Processes queue entries one at a time via [`upload_entry`] -- the one loop
/// the window's "upload all" and the command line's `upload` share: the
/// oldest `queued`/`uploading` row first. `should_continue` is polled before
/// each entry and while waiting, which is what lets a "pause" action stop the
/// run between files, or during a wait, without losing any progress already
/// made on the current one. Returns the number of entries this call finished
/// (sent, or set aside for good by the service).
///
/// Starting a run is the person acting, so it first releases every entry
/// that was `held` for them ([`UploadQueue::release_held`]): one whose
/// reason still stands is held again by the service's first answer.
///
/// What happens to a file that does not go through is its
/// [`crate::uploader::Verdict`], and never more than once for a refusal:
///
/// - the service **refuses it for good** ([`QueueDriveEvent::Refused`]): the
///   entry is set aside, the run goes on with the next;
/// - **the person has to act** ([`QueueDriveEvent::Held`]): the run stops
///   ([`StopReason::Held`]);
/// - it is **not its fault and will pass** ([`QueueDriveEvent::Waiting`]): the
///   run waits (`options.backoff`, longer each time) and tries the same file
///   again, without marking it, up to `options.max_replays` times;
/// - the service **ended the sign-in**, or the local database failed
///   ([`QueueDriveEvent::Failed`]): the run stops, the file is as it was.
pub fn drive_queue<T: UploadTransport>(
    transport: &T,
    queue: &UploadQueue,
    token: &str,
    target: &UploadTarget,
    options: &DriveOptions,
    mut should_continue: impl FnMut() -> bool,
    mut on_event: impl FnMut(QueueDriveEvent),
) -> Result<usize, GuiBridgeError> {
    queue.release_held()?;
    let mut processed = 0_usize;
    // The entry the run just finished with. The queue must not hand it back:
    // if it does (it could not record the outcome), the run stops instead of
    // sending it again.
    let mut previous_entry_id: Option<i64> = None;
    loop {
        if !should_continue() {
            on_event(QueueDriveEvent::Stopped {
                sha256: None,
                reason: StopReason::Paused,
            });
            return Ok(processed);
        }
        let Some(mut entry) = queue.next_pending()? else {
            on_event(QueueDriveEvent::Stopped {
                sha256: None,
                reason: StopReason::QueueEmpty,
            });
            return Ok(processed);
        };
        if previous_entry_id == Some(entry.id) {
            on_event(QueueDriveEvent::Stopped {
                sha256: Some(entry.sha256.clone()),
                reason: StopReason::NoProgress,
            });
            return Ok(processed);
        }

        on_event(QueueDriveEvent::Started {
            sha256: entry.sha256.clone(),
            source_path: entry.source_path.clone(),
            bytes: entry.size,
        });
        let mut replays = 0_u32;
        loop {
            let progress_sha256 = entry.sha256.clone();
            let result = upload_entry(transport, queue, token, &entry, target, |sent, total| {
                on_event(QueueDriveEvent::Progress {
                    sha256: progress_sha256.clone(),
                    sent,
                    total,
                });
            });
            let error = match result {
                Ok(outcome) => {
                    on_event(QueueDriveEvent::Completed {
                        sha256: entry.sha256.clone(),
                        capture_id: outcome.capture_id,
                        deduplicated: outcome.deduplicated,
                        skipped: outcome.skipped,
                        replaced: outcome.replaced,
                    });
                    processed += 1;
                    previous_entry_id = Some(entry.id);
                    break;
                }
                Err(error) => error,
            };
            // The events say it in French, in plain words: the window looks
            // the code up in its own dictionary and shows this text only
            // for a code it has no sentence for; the command line prints it.
            let (sha256, code, message) = (
                entry.sha256.clone(),
                error.code().to_string(),
                error.plain_message(),
            );
            match error.verdict() {
                Verdict::Refused => {
                    on_event(QueueDriveEvent::Refused {
                        sha256,
                        code,
                        message,
                    });
                    processed += 1;
                    previous_entry_id = Some(entry.id);
                    break;
                }
                Verdict::Failed => {
                    // Already marked `failed` by `upload_entry`; `next_pending`
                    // will skip it from here on, so the run can continue.
                    on_event(QueueDriveEvent::Failed {
                        sha256,
                        code,
                        message,
                    });
                    processed += 1;
                    previous_entry_id = Some(entry.id);
                    break;
                }
                Verdict::NeedsPerson => {
                    on_event(QueueDriveEvent::Held {
                        sha256: sha256.clone(),
                        code,
                        message,
                    });
                    on_event(QueueDriveEvent::Stopped {
                        sha256: Some(sha256),
                        reason: StopReason::Held,
                    });
                    return Ok(processed);
                }
                Verdict::Cancelled => {
                    // The person cancelled the wait for a sleeping service:
                    // not a failure. The entry stays exactly as progress had
                    // recorded it, ready to resume.
                    on_event(QueueDriveEvent::Stopped {
                        sha256: Some(sha256),
                        reason: StopReason::Cancelled,
                    });
                    return Ok(processed);
                }
                Verdict::SignedOut | Verdict::Local => {
                    // Not the file's fault and not something waiting mends:
                    // leave the entry exactly as progress had recorded it and
                    // stop.
                    on_event(QueueDriveEvent::Failed {
                        sha256,
                        code,
                        message,
                    });
                    return Ok(processed);
                }
                Verdict::RetryLater => {
                    replays += 1;
                    if options.max_replays.is_some_and(|most| replays > most)
                        || (error.is_silence() && !options.replay_when_unanswered)
                    {
                        // This run has waited as long as it was told to (or
                        // was told not to wait for a service that does not
                        // answer): the file stays queued (not marked) for the
                        // next one.
                        on_event(QueueDriveEvent::Failed {
                            sha256: sha256.clone(),
                            code,
                            message,
                        });
                        on_event(QueueDriveEvent::Stopped {
                            sha256: Some(sha256),
                            reason: StopReason::NoProgress,
                        });
                        return Ok(processed);
                    }
                    let wait = options.backoff.wait(replays).max(
                        error
                            .retry_after()
                            .unwrap_or_default()
                            .min(LONGEST_RETRY_AFTER),
                    );
                    on_event(QueueDriveEvent::Waiting {
                        sha256: sha256.clone(),
                        code,
                        message,
                        retry_in_seconds: wait.as_secs() + u64::from(wait.subsec_nanos() > 0),
                        attempt: replays,
                    });
                    if !wait_unless_stopped(wait, &mut should_continue) {
                        on_event(QueueDriveEvent::Stopped {
                            sha256: Some(sha256),
                            reason: StopReason::Paused,
                        });
                        return Ok(processed);
                    }
                    // What the last attempt recorded (a session, an offset) is
                    // where the next one starts from.
                    if let Some(fresh) = queue.find_by_sha256(&entry.sha256)? {
                        entry = fresh;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dry_run::{DryRunEntry, DryRunReport};
    use crate::package::AssetKind;
    use crate::scanner::FileClass;
    use crate::test_support::TempDir;
    use crate::uploader::{
        FinalizeOutcome, FinalizeResponse, PutOutcome, UploadError, UploadSession,
    };
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::time::Duration;

    #[test]
    fn settings_round_trip_and_missing_file_is_default() {
        let dir = TempDir::new("gui-settings");
        let path = dir.path().join("nested/settings.json");

        assert_eq!(load_settings(&path).unwrap(), GuiSettings::default());

        let settings = GuiSettings {
            portal_url: Some("https://portal.example".to_string()),
            device_label: Some("Bureau atelier".to_string()),
            locale: Some("fr".to_string()),
        };
        save_settings(&path, &settings).unwrap();
        assert_eq!(load_settings(&path).unwrap(), settings);
    }

    #[test]
    fn save_settings_refuses_a_non_http_portal_url() {
        let dir = TempDir::new("gui-settings-bad-url");
        let path = dir.path().join("settings.json");
        let settings = GuiSettings {
            portal_url: Some("not-a-url".to_string()),
            ..GuiSettings::default()
        };
        assert!(matches!(
            save_settings(&path, &settings),
            Err(GuiBridgeError::InvalidPortalUrl(_))
        ));
        assert!(!path.exists());
    }

    /// A [`Capabilities`] value with every field fixed except
    /// `analysis.logs`/`analysis.progression`, which move together --
    /// as they do in practice (see [`Capabilities`]'s own doc comment) --
    /// so a test only ever has to say "entitled" or not.
    fn sample_capabilities(analysis_entitled: bool) -> Capabilities {
        Capabilities {
            planner_multiple: true,
            planner_mosaic: true,
            pointing_solve: true,
            sync_plan_limit: 1000,
            gallery_private: true,
            portal_invitations: true,
            capture_variant_limit: 9_007_199_254_740_991,
            capture_count_limit: 9_007_199_254_740_991,
            capture_public: true,
            analysis_logs: analysis_entitled,
            analysis_progression: analysis_entitled,
            storage_max_file_bytes: 104_857_600,
            storage_quota_bytes: 53_687_091_200,
            history_retention_days: if analysis_entitled { 730 } else { 90 },
        }
    }

    #[test]
    fn capabilities_cache_round_trip_and_missing_file_is_none() {
        let dir = TempDir::new("gui-capabilities-cache");
        let path = dir.path().join("nested/capabilities.json");

        assert_eq!(load_capabilities_cache(&path).unwrap(), None);

        let cache = CapabilitiesCache {
            capabilities: sample_capabilities(true),
            fetched_at_utc: "2026-09-27T00:00:00Z".to_string(),
            standing: None,
        };
        save_capabilities_cache(&path, &cache).unwrap();
        assert_eq!(load_capabilities_cache(&path).unwrap(), Some(cache));
    }

    #[test]
    fn capabilities_cache_load_rejects_invalid_json() {
        let dir = TempDir::new("gui-capabilities-cache-bad-json");
        let path = dir.path().join("capabilities.json");
        std::fs::write(&path, "not json").unwrap();
        assert!(matches!(
            load_capabilities_cache(&path),
            Err(GuiBridgeError::InvalidSettings { .. })
        ));
    }

    #[test]
    fn capabilities_map_carries_pointing_solve() {
        let full = r#"{"planner.multiple":true,"planner.mosaic":true,"pointing.solve":true,"sync.plan_limit":1000,"gallery.private":true,"portal.invitations":true,"capture.variant_limit":4,"capture.count_limit":9007199254740991,"capture.public":true,"analysis.logs":false,"analysis.progression":false,"storage.max_file_bytes":104857600,"storage.quota_bytes":53687091200,"history.retention_days":90}"#;
        let parsed: Capabilities = serde_json::from_str(full).unwrap();
        assert!(parsed.pointing_solve);
        let round_trip = serde_json::to_value(parsed).unwrap();
        assert_eq!(round_trip["pointing.solve"], serde_json::json!(true));
        // The map is complete or refused: the eleven keys of before are
        // not enough any more.
        let without = full.replace(r#""pointing.solve":true,"#, "");
        assert!(serde_json::from_str::<Capabilities>(&without).is_err());
    }

    const FOURTEEN_KEYS: &str = r#"{"planner.multiple":true,"planner.mosaic":true,"pointing.solve":true,"sync.plan_limit":1000,"gallery.private":true,"portal.invitations":false,"capture.variant_limit":0,"capture.count_limit":3,"capture.public":true,"analysis.logs":false,"analysis.progression":false,"storage.max_file_bytes":26214400,"storage.quota_bytes":78643200,"history.retention_days":7}"#;

    #[test]
    fn capabilities_map_of_fourteen_keys_parses_and_round_trips() {
        let parsed: Capabilities = serde_json::from_str(FOURTEEN_KEYS).unwrap();
        assert!(!parsed.portal_invitations);
        assert_eq!(parsed.capture_count_limit, 3);
        assert!(parsed.capture_public);
        // The unlimited value of the contract fits the `u64` of the count.
        let unlimited = FOURTEEN_KEYS.replace(
            r#""capture.count_limit":3"#,
            r#""capture.count_limit":9007199254740991"#,
        );
        let parsed_unlimited: Capabilities = serde_json::from_str(&unlimited).unwrap();
        assert_eq!(parsed_unlimited.capture_count_limit, 9_007_199_254_740_991);
        let round_trip = serde_json::to_value(parsed).unwrap();
        let original: serde_json::Value = serde_json::from_str(FOURTEEN_KEYS).unwrap();
        assert_eq!(round_trip, original);
    }

    #[test]
    fn capabilities_map_still_refuses_an_unknown_or_a_missing_key() {
        // An unknown key is refused (`deny_unknown_fields`).
        let unknown = FOURTEEN_KEYS.replace(
            r#""capture.public":true,"#,
            r#""capture.public":true,"capture.unknown":true,"#,
        );
        assert!(serde_json::from_str::<Capabilities>(&unknown).is_err());
        // So is a map without one of the three new keys.
        for key in [
            r#""portal.invitations":false,"#,
            r#""capture.count_limit":3,"#,
            r#""capture.public":true,"#,
        ] {
            let without = FOURTEEN_KEYS.replace(key, "");
            assert!(
                serde_json::from_str::<Capabilities>(&without).is_err(),
                "{key}"
            );
        }
    }

    /// A hand-rolled in-memory [`KeyringBackend`], sharing state across
    /// calls the way a real keyring does and `keyring::mock` deliberately
    /// does not -- see [`KeyringBackend`]'s doc for why this, rather than
    /// `keyring::mock`, is what the fallback/migration tests below use.
    #[derive(Default)]
    struct FakeKeyring {
        stored: RefCell<Option<String>>,
        should_fail: Cell<bool>,
    }

    impl FakeKeyring {
        /// A keyring that refuses every call, the way a locked/absent
        /// platform store (or a denied access prompt) would.
        fn always_unavailable() -> Self {
            let keyring = Self::default();
            keyring.should_fail.set(true);
            keyring
        }

        fn refusal<T>() -> Result<T, GuiBridgeError> {
            Err(GuiBridgeError::KeyringUnavailable(
                "fake keyring refuses (test)".to_string(),
            ))
        }
    }

    impl KeyringBackend for FakeKeyring {
        fn save(&self, token: &str) -> Result<(), GuiBridgeError> {
            if self.should_fail.get() {
                return Self::refusal();
            }
            *self.stored.borrow_mut() = Some(token.to_string());
            Ok(())
        }

        fn load(&self) -> Result<Option<String>, GuiBridgeError> {
            if self.should_fail.get() {
                return Self::refusal();
            }
            Ok(self.stored.borrow().clone())
        }

        fn delete(&self) -> Result<(), GuiBridgeError> {
            if self.should_fail.get() {
                return Self::refusal();
            }
            *self.stored.borrow_mut() = None;
            Ok(())
        }
    }

    #[test]
    fn device_token_round_trip_via_keyring_and_clear() {
        let dir = TempDir::new("gui-device-token");
        let path = dir.path().join("device-token.txt");
        let keyring = FakeKeyring::default();

        assert_eq!(load_device_token_via(&keyring, &path).unwrap(), None);

        save_device_token_via(&keyring, &path, "a-token-value").unwrap();
        assert!(
            !path.exists(),
            "a working keyring must never leave a copy in the legacy file"
        );
        assert_eq!(
            load_device_token_via(&keyring, &path).unwrap(),
            Some("a-token-value".to_string())
        );

        clear_device_token_via(&keyring, &path).unwrap();
        assert_eq!(load_device_token_via(&keyring, &path).unwrap(), None);
        // Clearing an already-absent token is not an error.
        clear_device_token_via(&keyring, &path).unwrap();
    }

    #[test]
    fn device_token_migrates_from_the_legacy_file_then_deletes_it() {
        let dir = TempDir::new("gui-device-token-migrate");
        let path = dir.path().join("device-token.txt");
        // Simulate an install that paired before the keyring was used: a token
        // sitting in the file, nothing in the keyring yet.
        save_device_token_file(&path, "legacy-token").unwrap();
        let keyring = FakeKeyring::default();

        let token = load_device_token_via(&keyring, &path).unwrap();

        assert_eq!(token, Some("legacy-token".to_string()));
        assert!(
            !path.exists(),
            "the legacy file must be deleted once migrated"
        );
        assert_eq!(
            keyring.stored.borrow().as_deref(),
            Some("legacy-token"),
            "the token must actually have been written into the keyring, not just returned"
        );
    }

    #[test]
    fn device_token_falls_back_to_the_file_when_the_keyring_refuses() {
        let dir = TempDir::new("gui-device-token-fallback");
        let path = dir.path().join("device-token.txt");
        let keyring = FakeKeyring::always_unavailable();

        save_device_token_via(&keyring, &path, "fallback-token").unwrap();
        assert!(
            keyring.stored.borrow().is_none(),
            "the refusing keyring must never end up holding the token"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        assert_eq!(
            load_device_token_via(&keyring, &path).unwrap(),
            Some("fallback-token".to_string()),
            "reading back must also fall back to the file rather than losing the token"
        );

        clear_device_token_via(&keyring, &path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn system_keyring_round_trips_and_maps_errors_through_the_keyring_crates_mock() {
        // `keyring`'s own mock backend (always compiled in; see the module
        // doc). Installed as the process-wide default, which is safe here
        // because nothing else in this crate's test suite ever touches a
        // real platform keyring.
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        let system = SystemKeyring::new("system-keyring-mock-test").unwrap();

        assert_eq!(system.load().unwrap(), None);

        system.save("real-keyring-token").unwrap();
        assert_eq!(
            system.load().unwrap(),
            Some("real-keyring-token".to_string())
        );

        system.delete().unwrap();
        assert_eq!(system.load().unwrap(), None);
        // Deleting an already-absent credential is not an error either.
        system.delete().unwrap();

        // Force the next call to fail the way a locked/unavailable real
        // keychain would, and check `SystemKeyring` turns that into
        // `KeyringUnavailable` rather than a panic or a false "no entry".
        let mock: &keyring::mock::MockCredential = system
            .entry
            .get_credential()
            .downcast_ref()
            .expect("the mock credential builder is active for this test");
        mock.set_error(keyring::Error::NoStorageAccess(
            "keyring locked (test)".to_string().into(),
        ));
        assert!(matches!(
            system.load(),
            Err(GuiBridgeError::KeyringUnavailable(_))
        ));
    }

    struct FakeTransport {
        claim_result: Result<String, GuiBridgeError>,
        sessions_result: RefCell<Option<Result<SessionListPage, GuiBridgeError>>>,
        analysis_result: RefCell<Option<Result<UploadedAnalysis, GuiBridgeError>>>,
        capabilities_result: RefCell<Option<Result<Capabilities, GuiBridgeError>>>,
    }

    impl PortalGuiTransport for FakeTransport {
        fn claim_device(
            &self,
            _base_url: &str,
            _code: &str,
            _label: &str,
        ) -> Result<String, GuiBridgeError> {
            match &self.claim_result {
                Ok(token) => Ok(token.clone()),
                Err(GuiBridgeError::Refused { code, message }) => Err(GuiBridgeError::Refused {
                    code: code.clone(),
                    message: message.clone(),
                }),
                Err(_) => unreachable!("test only scripts Refused or Ok"),
            }
        }

        fn list_sessions(
            &self,
            _base_url: &str,
            _token: &str,
            _since: Option<&str>,
            _limit: Option<u32>,
        ) -> Result<SessionListPage, GuiBridgeError> {
            self.sessions_result
                .borrow_mut()
                .take()
                .expect("test scripted exactly one list_sessions call")
        }

        fn upload_analysis(
            &self,
            _base_url: &str,
            _token: &str,
            _session_id: &str,
            _report: &serde_json::Value,
        ) -> Result<UploadedAnalysis, GuiBridgeError> {
            self.analysis_result
                .borrow_mut()
                .take()
                .expect("test scripted exactly one upload_analysis call")
        }

        fn fetch_capabilities(
            &self,
            _base_url: &str,
            _token: &str,
        ) -> Result<Capabilities, GuiBridgeError> {
            self.capabilities_result
                .borrow_mut()
                .take()
                .expect("test scripted exactly one fetch_capabilities call")
        }
    }

    /// Keeps the body of the one analysis it is handed.
    #[derive(Default)]
    struct CapturingTransport {
        sent: RefCell<Option<serde_json::Value>>,
    }

    impl PortalGuiTransport for CapturingTransport {
        fn claim_device(&self, _: &str, _: &str, _: &str) -> Result<String, GuiBridgeError> {
            unreachable!()
        }
        fn list_sessions(
            &self,
            _: &str,
            _: &str,
            _: Option<&str>,
            _: Option<u32>,
        ) -> Result<SessionListPage, GuiBridgeError> {
            unreachable!()
        }
        fn upload_analysis(
            &self,
            _: &str,
            _: &str,
            _: &str,
            report: &serde_json::Value,
        ) -> Result<UploadedAnalysis, GuiBridgeError> {
            *self.sent.borrow_mut() = Some(report.clone());
            Err(GuiBridgeError::InvalidResponse("captured".to_string()))
        }
        fn fetch_capabilities(&self, _: &str, _: &str) -> Result<Capabilities, GuiBridgeError> {
            unreachable!()
        }
    }

    /// The names of the frames stay on the computer.
    #[test]
    fn the_report_that_leaves_carries_no_frame_name() {
        let report = serde_json::json!({
            "reportVersion": 2,
            "quality": [
                {"timestampUtc": "2026-10-07T21:20:00.000Z", "exposureSeconds": 180,
                 "fwhmPixels": 2.6, "sourcePath": "cible 1/lights/pose-1/Light_2026-10-07_21-20-00_-10C.fits"},
                {"timestampUtc": "2026-10-07T21:23:00.000Z", "exposureSeconds": 180,
                 "fwhmPixels": 2.7, "sourcePath": "cible 1/lights/pose-1/Light_2026-10-07_21-23-00_-10C.fits"}
            ],
            "guiding": [],
        });
        let transport = CapturingTransport::default();
        let _ = upload_analysis(
            &transport,
            "https://portal.example",
            "token",
            "s-1",
            &report,
        );
        let sent = transport
            .sent
            .borrow()
            .clone()
            .expect("the report was sent");
        let text = sent.to_string();
        assert!(!text.contains("sourcePath"), "{text}");
        assert!(!text.contains(".fits"), "{text}");
        assert!(!text.contains("lights/"), "{text}");
        // The measures stay; the caller's own report keeps its names.
        assert_eq!(sent["quality"][1]["fwhmPixels"], 2.7);
        assert_eq!(sent["quality"].as_array().unwrap().len(), 2);
        assert!(report.to_string().contains("sourcePath"));
        // A report without rows is left as it is.
        let bare = serde_json::json!({"reportVersion": 2});
        assert_eq!(report_for_sending(&bare), bare);
    }

    #[test]
    fn claim_device_rejects_an_empty_label_before_any_network_call() {
        let transport = FakeTransport {
            claim_result: Ok("unused".to_string()),
            sessions_result: RefCell::new(None),
            analysis_result: RefCell::new(None),
            capabilities_result: RefCell::new(None),
        };
        assert!(matches!(
            claim_device(&transport, "https://portal.example", "ABCDEFGH", "  "),
            Err(GuiBridgeError::EmptyDeviceLabel)
        ));
    }

    #[test]
    fn claim_device_rejects_a_malformed_code_before_any_network_call() {
        let transport = FakeTransport {
            claim_result: Ok("unused".to_string()),
            sessions_result: RefCell::new(None),
            analysis_result: RefCell::new(None),
            capabilities_result: RefCell::new(None),
        };
        assert!(matches!(
            claim_device(&transport, "https://portal.example", "short", "Bureau"),
            Err(GuiBridgeError::InvalidPairingCode)
        ));
    }

    #[test]
    fn claim_device_normalizes_case_and_trims_the_label() {
        let transport = FakeTransport {
            claim_result: Ok("device-token-value".to_string()),
            sessions_result: RefCell::new(None),
            analysis_result: RefCell::new(None),
            capabilities_result: RefCell::new(None),
        };
        let token = claim_device(
            &transport,
            "https://portal.example",
            "abcdefgh",
            "  Bureau atelier  ",
        )
        .unwrap();
        assert_eq!(token, "device-token-value");
    }

    #[test]
    fn claim_device_surfaces_a_portal_refusal() {
        let transport = FakeTransport {
            claim_result: Err(GuiBridgeError::Refused {
                code: "pairing_not_found".to_string(),
                message: "expired".to_string(),
            }),
            sessions_result: RefCell::new(None),
            analysis_result: RefCell::new(None),
            capabilities_result: RefCell::new(None),
        };
        assert!(matches!(
            claim_device(&transport, "https://portal.example", "ABCDEFGH", "Bureau"),
            Err(GuiBridgeError::Refused { code, .. }) if code == "pairing_not_found"
        ));
    }

    #[test]
    fn list_sessions_rejects_an_invalid_portal_url() {
        let transport = FakeTransport {
            claim_result: Ok("unused".to_string()),
            sessions_result: RefCell::new(None),
            analysis_result: RefCell::new(None),
            capabilities_result: RefCell::new(None),
        };
        assert!(matches!(
            list_sessions(&transport, "not-a-url", "token", None, None),
            Err(GuiBridgeError::InvalidPortalUrl(_))
        ));
    }

    #[test]
    fn list_sessions_returns_the_transport_page() {
        let page = SessionListPage {
            sessions: vec![SessionSummary {
                id: "11111111-1111-1111-1111-111111111111".to_string(),
                plan_id: None,
                status: "open".to_string(),
                started_at: "2026-09-20T21:00:00Z".to_string(),
                finished_at: None,
                created_at: "2026-09-20T21:00:00Z".to_string(),
                updated_at: "2026-09-20T21:00:00Z".to_string(),
                event_count: 0,
                supplement_count: 0,
                pending_supplement_count: 0,
                evening_date: Some("2026-09-20".to_string()),
                outcome: None,
            }],
            next_since: Some("2026-09-20T21:00:00Z".to_string()),
            has_more: false,
        };
        let transport = FakeTransport {
            claim_result: Ok("unused".to_string()),
            sessions_result: RefCell::new(Some(Ok(page.clone()))),
            analysis_result: RefCell::new(None),
            capabilities_result: RefCell::new(None),
        };
        let result = list_sessions(
            &transport,
            "https://portal.example",
            "token",
            None,
            Some(20),
        )
        .unwrap();
        assert_eq!(result, page);
    }

    #[test]
    fn a_session_carries_the_outcome_the_service_reads_from_its_debriefing() {
        let body = |outcome: &str| {
            format!(
                r#"{{"id":"s-1","planId":"p-1","status":"finished","startedAt":"2026-10-07T19:00:00Z","finishedAt":"2026-10-08T02:00:00Z","createdAt":"2026-10-07T19:00:00Z","updatedAt":"2026-10-08T02:00:00Z"{outcome}}}"#
            )
        };
        let abandoned: SessionSummary =
            serde_json::from_str(&body(r#","outcome":"abandoned""#)).unwrap();
        assert_eq!(abandoned.outcome.as_deref(), Some("abandoned"));
        let null: SessionSummary = serde_json::from_str(&body(r#","outcome":null"#)).unwrap();
        assert_eq!(null.outcome, None);
        // A service of an earlier version does not send it: the contract
        // reads that as `null`.
        let absent: SessionSummary = serde_json::from_str(&body("")).unwrap();
        assert_eq!(absent.outcome, None);
        // It goes back to the interface under the same name.
        let again = serde_json::to_value(&abandoned).unwrap();
        assert_eq!(again["outcome"], "abandoned");
    }

    #[test]
    fn upload_analysis_rejects_an_invalid_portal_url() {
        let transport = FakeTransport {
            claim_result: Ok("unused".to_string()),
            sessions_result: RefCell::new(None),
            analysis_result: RefCell::new(None),
            capabilities_result: RefCell::new(None),
        };
        assert!(matches!(
            upload_analysis(
                &transport,
                "not-a-url",
                "token",
                "session-1",
                &serde_json::json!({}),
            ),
            Err(GuiBridgeError::InvalidPortalUrl(_))
        ));
    }

    #[test]
    fn upload_analysis_returns_the_analysis_that_replaced_the_previous_one() {
        let transport = FakeTransport {
            claim_result: Ok("unused".to_string()),
            sessions_result: RefCell::new(None),
            analysis_result: RefCell::new(Some(Ok(UploadedAnalysis {
                analysis: UploadedAnalysisResource {
                    id: "11111111-1111-1111-1111-111111111111".to_string(),
                    session_id: "session-1".to_string(),
                    report_version: 2,
                    created_at: "2026-09-27T00:00:00Z".to_string(),
                    origin_device_id: None,
                    device_name: None,
                },
                replaced: true,
                steps: UploadedAnalysisSteps {
                    proposed: 3,
                    kept_edited: 1,
                },
            }))),
            capabilities_result: RefCell::new(None),
        };
        let result = upload_analysis(
            &transport,
            "https://portal.example",
            "token",
            "session-1",
            &serde_json::json!({"reportVersion": 2}),
        )
        .unwrap();
        assert_eq!(result.analysis.report_version, 2);
        assert!(result.replaced);
        assert_eq!(result.steps.kept_edited, 1);
    }

    /// The Analysis screen's own "capability state": the account is not
    /// entitled to upload local-analysis reports, the same
    /// `analysis_not_entitled` code `api/v1.md`'s "Session analyses"
    /// documents surfaces as `GuiBridgeError::Refused`, exactly like any
    /// other portal refusal (`claim_device_surfaces_a_portal_refusal`
    /// above) -- the frontend keys its clear, localized message off `code`.
    #[test]
    fn upload_analysis_surfaces_the_not_entitled_refusal() {
        let transport = FakeTransport {
            claim_result: Ok("unused".to_string()),
            sessions_result: RefCell::new(None),
            analysis_result: RefCell::new(Some(Err(GuiBridgeError::Refused {
                code: "analysis_not_entitled".to_string(),
                message: "This account cannot upload local-analysis reports.".to_string(),
            }))),
            capabilities_result: RefCell::new(None),
        };
        assert!(matches!(
            upload_analysis(
                &transport,
                "https://portal.example",
                "token",
                "session-1",
                &serde_json::json!({}),
            ),
            Err(GuiBridgeError::Refused { code, .. }) if code == "analysis_not_entitled"
        ));
    }

    /// The soft-downgrade read-only lock on an expired-subscription account
    /// refuses a new analysis upload with `entitlement_read_only`, the exact
    /// same stable code every other paid write already surfaces -- passed
    /// through as `GuiBridgeError::Refused` exactly like
    /// `analysis_not_entitled` above, so `commands.rs`'s `upload_analysis` can
    /// key its own clear, localized message off `code`.
    #[test]
    fn upload_analysis_surfaces_the_entitlement_read_only_refusal() {
        let transport = FakeTransport {
            claim_result: Ok("unused".to_string()),
            sessions_result: RefCell::new(None),
            analysis_result: RefCell::new(Some(Err(GuiBridgeError::Refused {
                code: "entitlement_read_only".to_string(),
                message: "This account's subscription has ended: analysis uploads are suspended; existing analyses remain readable and exportable.".to_string(),
            }))),
            capabilities_result: RefCell::new(None),
        };
        assert!(matches!(
            upload_analysis(
                &transport,
                "https://portal.example",
                "token",
                "session-1",
                &serde_json::json!({}),
            ),
            Err(GuiBridgeError::Refused { code, .. }) if code == "entitlement_read_only"
        ));
    }

    #[test]
    fn refresh_capabilities_rejects_an_invalid_portal_url() {
        let dir = TempDir::new("gui-capabilities-refresh-bad-url");
        let path = dir.path().join("capabilities.json");
        let transport = FakeTransport {
            claim_result: Ok("unused".to_string()),
            sessions_result: RefCell::new(None),
            analysis_result: RefCell::new(None),
            capabilities_result: RefCell::new(None),
        };
        assert!(matches!(
            refresh_capabilities(&transport, &path, "not-a-url", "token"),
            Err(GuiBridgeError::InvalidPortalUrl(_))
        ));
    }

    /// 5's "refreshed on opening if the network answers": a successful
    /// fetch wholesale replaces whatever was cached before, timestamp
    /// included -- there is no partial merge, and a stale entry never
    /// survives a fresh, successful answer.
    #[test]
    fn refresh_capabilities_overwrites_a_stale_cache_on_a_successful_fetch() {
        let dir = TempDir::new("gui-capabilities-refresh-fresh");
        let path = dir.path().join("capabilities.json");
        let stale = CapabilitiesCache {
            capabilities: sample_capabilities(false),
            fetched_at_utc: "2026-01-01T00:00:00Z".to_string(),
            standing: None,
        };
        save_capabilities_cache(&path, &stale).unwrap();

        let fresh = sample_capabilities(true);
        let transport = FakeTransport {
            claim_result: Ok("unused".to_string()),
            sessions_result: RefCell::new(None),
            analysis_result: RefCell::new(None),
            capabilities_result: RefCell::new(Some(Ok(fresh))),
        };

        let result =
            refresh_capabilities(&transport, &path, "https://portal.example", "token").unwrap();
        assert_eq!(result.capabilities, fresh);
        assert_ne!(result.fetched_at_utc, stale.fetched_at_utc);

        // The stale entry on disk is fully replaced, not merged -- reading
        // it back agrees exactly with what the call just returned.
        assert_eq!(load_capabilities_cache(&path).unwrap(), Some(result));
    }

    /// 5's "valable hors ligne": when the portal cannot currently be
    /// reached (no network, or any other transport-level failure), a
    /// previously cached profile is returned unchanged rather than the
    /// network error -- a paired device keeps whatever capability-gated
    /// screens it already had.
    #[test]
    fn refresh_capabilities_falls_back_to_the_cache_when_the_network_is_unavailable() {
        let dir = TempDir::new("gui-capabilities-refresh-offline");
        let path = dir.path().join("capabilities.json");
        let cached = CapabilitiesCache {
            capabilities: sample_capabilities(true),
            fetched_at_utc: "2026-09-20T00:00:00Z".to_string(),
            standing: None,
        };
        save_capabilities_cache(&path, &cached).unwrap();

        let transport = FakeTransport {
            claim_result: Ok("unused".to_string()),
            sessions_result: RefCell::new(None),
            analysis_result: RefCell::new(None),
            capabilities_result: RefCell::new(Some(Err(GuiBridgeError::Transport(
                "connection refused".to_string(),
            )))),
        };

        let result =
            refresh_capabilities(&transport, &path, "https://portal.example", "token").unwrap();
        assert_eq!(result, cached);
        // The failed attempt never touches the cache on disk.
        assert_eq!(load_capabilities_cache(&path).unwrap(), Some(cached));
    }

    /// The one case where there is truly nothing to show: a freshly paired
    /// device whose very first capability refresh fails before any cache
    /// ever existed. The Tauri `capabilities` command (`app/src-tauri`)
    /// treats this the same as "capability unknown" -- no Progression/
    /// Analysis screen, same as a standard profile -- but this crate-level
    /// function itself still reports the real failure rather than
    /// inventing a value.
    #[test]
    fn refresh_capabilities_fails_when_there_is_neither_a_fresh_answer_nor_any_cache() {
        let dir = TempDir::new("gui-capabilities-refresh-nothing");
        let path = dir.path().join("capabilities.json");

        let transport = FakeTransport {
            claim_result: Ok("unused".to_string()),
            sessions_result: RefCell::new(None),
            analysis_result: RefCell::new(None),
            capabilities_result: RefCell::new(Some(Err(GuiBridgeError::Transport(
                "connection refused".to_string(),
            )))),
        };

        assert!(matches!(
            refresh_capabilities(&transport, &path, "https://portal.example", "token"),
            Err(GuiBridgeError::Transport(_))
        ));
        assert!(!path.exists());
    }

    /// A transport that answers the capabilities with a profile and the
    /// standing with whatever it is scripted to say.
    struct StandingTransport {
        standing: RefCell<Option<Result<Option<Standing>, GuiBridgeError>>>,
    }

    impl PortalGuiTransport for StandingTransport {
        fn claim_device(&self, _: &str, _: &str, _: &str) -> Result<String, GuiBridgeError> {
            unreachable!()
        }
        fn list_sessions(
            &self,
            _: &str,
            _: &str,
            _: Option<&str>,
            _: Option<u32>,
        ) -> Result<SessionListPage, GuiBridgeError> {
            unreachable!()
        }
        fn upload_analysis(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &serde_json::Value,
        ) -> Result<UploadedAnalysis, GuiBridgeError> {
            unreachable!()
        }
        fn fetch_capabilities(&self, _: &str, _: &str) -> Result<Capabilities, GuiBridgeError> {
            Ok(sample_capabilities(true))
        }
        fn fetch_standing(&self, _: &str, _: &str) -> Result<Option<Standing>, GuiBridgeError> {
            self.standing
                .borrow_mut()
                .take()
                .expect("test scripted exactly one fetch_standing call")
        }
    }

    fn payment_failed() -> Standing {
        Standing::PaymentFailed {
            since: "2026-09-30T12:00:00.000Z".to_string(),
            read_only_until: "2026-12-29T12:00:00.000Z".to_string(),
        }
    }

    /// A payment that failed is kept beside the capabilities, so that a
    /// computer offline for a week still says it; a fresh answer that says
    /// nothing (the payment resumed) clears it at once.
    #[test]
    fn refresh_capabilities_keeps_the_standing_of_a_payment_that_failed() {
        let dir = TempDir::new("gui-capabilities-standing");
        let path = dir.path().join("capabilities.json");

        let held = StandingTransport {
            standing: RefCell::new(Some(Ok(Some(payment_failed())))),
        };
        let result = refresh_capabilities(&held, &path, "https://portal.example", "t").unwrap();
        assert_eq!(result.standing, Some(payment_failed()));
        assert_eq!(
            load_capabilities_cache(&path).unwrap().unwrap().standing,
            Some(payment_failed())
        );

        let resumed = StandingTransport {
            standing: RefCell::new(Some(Ok(None))),
        };
        let result = refresh_capabilities(&resumed, &path, "https://portal.example", "t").unwrap();
        assert_eq!(result.standing, None);
        assert_eq!(
            load_capabilities_cache(&path).unwrap().unwrap().standing,
            None
        );
    }

    /// The standing that cannot be read right now (no network) stays what
    /// the cache said; a service that answers by a refusal (a private server
    /// has nothing to say of subscriptions) is believed.
    #[test]
    fn refresh_capabilities_keeps_the_cached_standing_when_only_the_standing_cannot_be_read() {
        let dir = TempDir::new("gui-capabilities-standing-offline");
        let path = dir.path().join("capabilities.json");
        save_capabilities_cache(
            &path,
            &CapabilitiesCache {
                capabilities: sample_capabilities(true),
                fetched_at_utc: "2026-10-01T00:00:00Z".to_string(),
                standing: Some(payment_failed()),
            },
        )
        .unwrap();

        let unreachable = StandingTransport {
            standing: RefCell::new(Some(Err(GuiBridgeError::Transport("reset".to_string())))),
        };
        let result =
            refresh_capabilities(&unreachable, &path, "https://portal.example", "t").unwrap();
        assert_eq!(result.standing, Some(payment_failed()));

        let refused = StandingTransport {
            standing: RefCell::new(Some(Err(GuiBridgeError::Refused {
                code: "not_found".to_string(),
                message: "no such route".to_string(),
            }))),
        };
        let result = refresh_capabilities(&refused, &path, "https://portal.example", "t").unwrap();
        assert_eq!(result.standing, None);
    }

    /// A cache written before the standing existed still loads.
    #[test]
    fn a_capabilities_cache_without_a_standing_still_loads() {
        let dir = TempDir::new("gui-capabilities-cache-old");
        let path = dir.path().join("capabilities.json");
        let old = serde_json::json!({
            "capabilities": serde_json::to_value(sample_capabilities(true)).unwrap(),
            "fetchedAtUtc": "2026-09-20T00:00:00Z",
        });
        std::fs::write(&path, old.to_string()).unwrap();
        assert_eq!(
            load_capabilities_cache(&path).unwrap().unwrap().standing,
            None
        );
    }

    fn sample_dry_run_report(bytes: u64) -> DryRunReport {
        DryRunReport {
            source_root: PathBuf::from("/tmp/does-not-matter"),
            entries: vec![DryRunEntry {
                source_relative_path: "frame.tif".to_string(),
                class: FileClass::Fits,
                asset_kind: AssetKind::Image,
                package_relative_path: "images/frame.tif".to_string(),
                bytes,
                sha256: "a".repeat(64),
            }],
            total_bytes: bytes,
        }
    }

    /// Every refusal the contract names reaches the page with its own code --
    /// an analysis over the ceiling of a request, a ceiling of sessions, a
    /// space at its ceiling of devices -- and any other is the generic one.
    #[test]
    fn a_refusal_the_contract_names_reaches_the_page_with_its_own_code() {
        for code in crate::refusal::known_codes() {
            let error = GuiBridgeError::Refused {
                code: code.to_string(),
                message: "The service's own English sentence.".to_string(),
            };
            assert_eq!(error.code(), code);
        }
        let unknown = GuiBridgeError::Refused {
            code: "a_code_from_a_newer_service".to_string(),
            message: "The service's own English sentence.".to_string(),
        };
        assert_eq!(unknown.code(), "portal_refused");
    }

    #[test]
    fn summarize_queue_counts_every_state_and_sent_bytes() {
        let dir = TempDir::new("gui-summary-queue");
        let queue = UploadQueue::open(&dir.path().join("queue.sqlite")).unwrap();
        queue
            .enqueue_from_dry_run(&sample_dry_run_report(1000))
            .unwrap();
        let entries = queue.list().unwrap();
        let summary = summarize_queue(&entries);
        assert_eq!(
            summary,
            QueueSummary {
                total: 1,
                queued: 1,
                uploading: 0,
                completed: 0,
                failed: 0,
                refused: 0,
                held: 0,
                total_bytes: 1000,
                sent_bytes: 0,
            }
        );

        queue
            .mark_progress(&"a".repeat(64), "upload-1", 400)
            .unwrap();
        let entries = queue.list().unwrap();
        let summary = summarize_queue(&entries);
        assert_eq!(summary.uploading, 1);
        assert_eq!(summary.sent_bytes, 400);

        queue.mark_completed(&"a".repeat(64), "capture-1").unwrap();
        let entries = queue.list().unwrap();
        let summary = summarize_queue(&entries);
        assert_eq!(summary.completed, 1);
        assert_eq!(summary.sent_bytes, 1000);
    }

    /// A scriptable fake transport, mirroring `crate::uploader`'s own test
    /// support: each method pops its next scripted response.
    #[derive(Default)]
    struct FakeUploadTransport {
        creates: RefCell<VecDeque<Result<UploadSession, UploadError>>>,
        puts: RefCell<VecDeque<Result<PutOutcome, UploadError>>>,
        finalizes: RefCell<VecDeque<Result<FinalizeOutcome, UploadError>>>,
    }

    impl UploadTransport for FakeUploadTransport {
        fn create(
            &self,
            _token: &str,
            _metadata: &crate::uploader::UploadMetadata,
        ) -> Result<UploadSession, UploadError> {
            self.creates
                .borrow_mut()
                .pop_front()
                .expect("scripted create")
        }
        fn status(&self, _token: &str, _upload_id: &str) -> Result<UploadSession, UploadError> {
            unreachable!("test never triggers a status reconciliation")
        }
        fn put_chunk(
            &self,
            _token: &str,
            _upload_id: &str,
            _offset: u64,
            _chunk: &[u8],
        ) -> Result<PutOutcome, UploadError> {
            self.puts.borrow_mut().pop_front().expect("scripted put")
        }
        fn finalize(&self, _token: &str, _upload_id: &str) -> Result<FinalizeOutcome, UploadError> {
            self.finalizes
                .borrow_mut()
                .pop_front()
                .expect("scripted finalize")
        }
    }

    /// The options of a run that never waits for a file again: the shape the
    /// tests of a single outcome want.
    fn no_replays() -> DriveOptions {
        DriveOptions {
            backoff: Backoff::new(vec![Duration::from_millis(5)]),
            max_replays: Some(0),
            replay_when_unanswered: true,
        }
    }

    #[test]
    fn drive_queue_processes_one_entry_and_reports_queue_empty() {
        let dir = TempDir::new("gui-drive-queue-empty");
        let source = TempDir::new("gui-drive-queue-source");
        let file_path = source.path().join("frame.tif");
        fs::write(&file_path, vec![7_u8; 4]).unwrap();
        let sha256 = "b".repeat(64);

        let queue = UploadQueue::open(&dir.path().join("queue.sqlite")).unwrap();
        let report = DryRunReport {
            source_root: source.path().to_path_buf(),
            entries: vec![DryRunEntry {
                source_relative_path: "frame.tif".to_string(),
                class: FileClass::Fits,
                asset_kind: AssetKind::Image,
                package_relative_path: "images/frame.tif".to_string(),
                bytes: 4,
                sha256: sha256.clone(),
            }],
            total_bytes: 4,
        };
        queue.enqueue_from_dry_run(&report).unwrap();

        let transport = FakeUploadTransport {
            creates: RefCell::new(VecDeque::from([Ok(UploadSession {
                upload_id: "upload-1".to_string(),
                offset: 0,
                byte_size: 4,
                chunk_size: 8_388_608,
                completed: false,
                expires_at: None,
                capture_id: None,
            })])),
            puts: RefCell::new(VecDeque::from([Ok(PutOutcome::Advanced(UploadSession {
                upload_id: "upload-1".to_string(),
                offset: 4,
                byte_size: 4,
                chunk_size: 8_388_608,
                completed: false,
                expires_at: None,
                capture_id: None,
            }))])),
            finalizes: RefCell::new(VecDeque::from([Ok(FinalizeOutcome::Completed(
                FinalizeResponse {
                    capture_id: "capture-1".to_string(),
                    deduplicated: false,
                    ..Default::default()
                },
            ))])),
        };

        let mut events = Vec::new();
        let processed = drive_queue(
            &transport,
            &queue,
            "token",
            &UploadTarget::default(),
            &no_replays(),
            || true,
            |event| events.push(event),
        )
        .unwrap();

        assert_eq!(processed, 1);
        assert!(matches!(
            events.last(),
            Some(QueueDriveEvent::Stopped {
                reason: StopReason::QueueEmpty,
                ..
            })
        ));
        assert!(events.iter().any(|event| matches!(
            event,
            QueueDriveEvent::Completed { capture_id, .. } if capture_id == "capture-1"
        )));
    }

    #[test]
    fn drive_queue_stops_immediately_when_should_continue_is_false() {
        let dir = TempDir::new("gui-drive-queue-paused");
        let queue = UploadQueue::open(&dir.path().join("queue.sqlite")).unwrap();
        queue
            .enqueue_from_dry_run(&sample_dry_run_report(10))
            .unwrap();

        let transport = FakeUploadTransport::default();
        let mut events = Vec::new();
        let processed = drive_queue(
            &transport,
            &queue,
            "token",
            &UploadTarget::default(),
            &no_replays(),
            || false,
            |event| events.push(event),
        )
        .unwrap();

        assert_eq!(processed, 0);
        assert!(matches!(
            events.as_slice(),
            [QueueDriveEvent::Stopped {
                reason: StopReason::Paused,
                ..
            }]
        ));
    }

    /// The front end (`app/src/lib/bridge.ts`, `QueueDriveEvent`) reads
    /// `kind` as `"Started" | "Progress" | "Completed" | "Failed" |
    /// "Stopped"` and every field in camelCase. Before this contract was
    /// pinned the enum serialized as `{"kind":"completed","capture_id":..}`,
    /// which no screen ever matched -- the queue never showed live progress.
    #[test]
    fn queue_drive_events_serialize_as_the_front_end_reads_them() {
        let started = serde_json::to_value(QueueDriveEvent::Started {
            sha256: "abc".to_string(),
            source_path: PathBuf::from("/night/frame.tif"),
            bytes: 12,
        })
        .unwrap();
        assert_eq!(
            started,
            serde_json::json!({
                "kind": "Started",
                "sha256": "abc",
                "sourcePath": "/night/frame.tif",
                "bytes": 12,
            })
        );

        let completed = serde_json::to_value(QueueDriveEvent::Completed {
            sha256: "abc".to_string(),
            capture_id: "capture-1".to_string(),
            deduplicated: true,
            skipped: Vec::new(),
            replaced: false,
        })
        .unwrap();
        assert_eq!(
            completed,
            serde_json::json!({
                "kind": "Completed",
                "sha256": "abc",
                "captureId": "capture-1",
                "deduplicated": true,
            })
        );
        // An image that replaced the capture's own says so.
        let replaced = serde_json::to_value(QueueDriveEvent::Completed {
            sha256: "abc".to_string(),
            capture_id: "capture-1".to_string(),
            deduplicated: false,
            skipped: Vec::new(),
            replaced: true,
        })
        .unwrap();
        assert_eq!(replaced["replaced"], true);

        let failed = serde_json::to_value(QueueDriveEvent::Failed {
            sha256: "abc".to_string(),
            code: "network_error".to_string(),
            message: "detail".to_string(),
        })
        .unwrap();
        assert_eq!(
            failed,
            serde_json::json!({
                "kind": "Failed",
                "sha256": "abc",
                "code": "network_error",
                "message": "detail",
            })
        );

        let stopped = serde_json::to_value(QueueDriveEvent::Stopped {
            sha256: None,
            reason: StopReason::Cancelled,
        })
        .unwrap();
        assert_eq!(
            stopped,
            serde_json::json!({"kind": "Stopped", "sha256": null, "reason": "cancelled"})
        );
    }

    fn queue_with_one_file(label: &str) -> (TempDir, TempDir, UploadQueue) {
        let dir = TempDir::new(label);
        let source = TempDir::new(&format!("{label}-source"));
        fs::write(source.path().join("frame.tif"), vec![7_u8; 4]).unwrap();
        let queue = UploadQueue::open(&dir.path().join("queue.sqlite")).unwrap();
        let report = DryRunReport {
            source_root: source.path().to_path_buf(),
            entries: vec![DryRunEntry {
                source_relative_path: "frame.tif".to_string(),
                class: FileClass::Fits,
                asset_kind: AssetKind::Image,
                package_relative_path: "images/frame.tif".to_string(),
                bytes: 4,
                sha256: "c".repeat(64),
            }],
            total_bytes: 4,
        };
        queue.enqueue_from_dry_run(&report).unwrap();
        (dir, source, queue)
    }

    /// A file that could not be sent because the service is unreachable is
    /// reported with the stable code the interface translates ("Impossible
    /// de joindre le portail..."), not with the transport's technical text
    /// alone -- and stays queued, resumable.
    #[test]
    fn drive_queue_reports_a_transient_failure_with_a_stable_code() {
        let (_dir, _source, queue) = queue_with_one_file("gui-drive-queue-failed");
        let transport = FakeUploadTransport {
            creates: RefCell::new(VecDeque::from([Err(UploadError::Transport(
                "error sending request".to_string(),
            ))])),
            ..FakeUploadTransport::default()
        };
        let mut events = Vec::new();
        let processed = drive_queue(
            &transport,
            &queue,
            "token",
            &UploadTarget::default(),
            &no_replays(),
            || true,
            |event| events.push(event),
        )
        .unwrap();

        assert_eq!(processed, 0);
        assert!(events.iter().any(|event| matches!(
            event,
            QueueDriveEvent::Failed { code, .. } if code == "network_error"
        )));
        assert_eq!(queue.list().unwrap()[0].state, QueueState::Queued);
    }

    /// Cancelling the wait for a sleeping service ends the run as a plain
    /// "stopped on your request" -- not a failure -- and leaves the file
    /// exactly as it was.
    #[test]
    fn drive_queue_stops_as_cancelled_when_the_person_cancels_the_wait() {
        let (_dir, _source, queue) = queue_with_one_file("gui-drive-queue-cancelled");
        let transport = FakeUploadTransport {
            creates: RefCell::new(VecDeque::from([Err(UploadError::Wait(
                ServiceWaitError::Cancelled,
            ))])),
            ..FakeUploadTransport::default()
        };
        let mut events = Vec::new();
        let processed = drive_queue(
            &transport,
            &queue,
            "token",
            &UploadTarget::default(),
            &no_replays(),
            || true,
            |event| events.push(event),
        )
        .unwrap();

        assert_eq!(processed, 0);
        assert!(matches!(
            events.last(),
            Some(QueueDriveEvent::Stopped {
                sha256: Some(_),
                reason: StopReason::Cancelled,
            })
        ));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, QueueDriveEvent::Failed { .. })),
            "a cancel is not a failure"
        );
        assert_eq!(queue.list().unwrap()[0].state, QueueState::Queued);
    }

    /// A queue with these files (name, sha256), 4 bytes each, oldest first.
    fn queue_with_files(label: &str, files: &[(&str, &str)]) -> (TempDir, TempDir, UploadQueue) {
        let dir = TempDir::new(label);
        let source = TempDir::new(&format!("{label}-source"));
        let queue = UploadQueue::open(&dir.path().join("queue.sqlite")).unwrap();
        let mut entries = Vec::new();
        for (name, sha256) in files {
            fs::write(source.path().join(name), vec![7_u8; 4]).unwrap();
            entries.push(DryRunEntry {
                source_relative_path: (*name).to_string(),
                class: FileClass::Fits,
                asset_kind: AssetKind::Image,
                package_relative_path: format!("images/{name}"),
                bytes: 4,
                sha256: (*sha256).to_string(),
            });
        }
        queue
            .enqueue_from_dry_run(&DryRunReport {
                source_root: source.path().to_path_buf(),
                total_bytes: 4 * entries.len() as u64,
                entries,
            })
            .unwrap();
        (dir, source, queue)
    }

    fn session_of(upload_id: &str, offset: u64, completed: bool) -> UploadSession {
        UploadSession {
            upload_id: upload_id.to_string(),
            offset,
            byte_size: 4,
            chunk_size: 8_388_608,
            completed,
            expires_at: None,
            capture_id: None,
        }
    }

    fn sent_ok(capture: &str) -> Result<FinalizeOutcome, UploadError> {
        Ok(FinalizeOutcome::Completed(FinalizeResponse {
            capture_id: capture.to_string(),
            deduplicated: false,
            ..Default::default()
        }))
    }

    fn event_kinds(events: &[QueueDriveEvent]) -> Vec<&'static str> {
        events
            .iter()
            .map(|event| match event {
                QueueDriveEvent::Started { .. } => "started",
                QueueDriveEvent::Progress { .. } => "progress",
                QueueDriveEvent::Completed { .. } => "completed",
                QueueDriveEvent::Failed { .. } => "failed",
                QueueDriveEvent::Refused { .. } => "refused",
                QueueDriveEvent::Held { .. } => "held",
                QueueDriveEvent::Waiting { .. } => "waiting",
                QueueDriveEvent::Stopped { .. } => "stopped",
            })
            .collect()
    }

    /// The service refuses one file for good (an SVG document): that file
    /// leaves the queue with the sentence's code, and the run goes on with
    /// the next one -- and never asks about the refused one again.
    #[test]
    fn drive_queue_sets_a_refused_file_aside_and_goes_on_with_the_next() {
        let (_dir, _source, queue) = queue_with_files(
            "gui-drive-refused",
            &[("drawing.png", "aaa"), ("frame.tif", "bbb")],
        );
        let transport = FakeUploadTransport {
            creates: RefCell::new(VecDeque::from([
                Err(UploadError::refusal(
                    415,
                    "capture_content_refused",
                    "SVG document",
                )),
                Ok(session_of("upload-b", 0, false)),
            ])),
            puts: RefCell::new(VecDeque::from([Ok(PutOutcome::Advanced(session_of(
                "upload-b", 4, false,
            )))])),
            finalizes: RefCell::new(VecDeque::from([sent_ok("capture-b")])),
        };
        let mut events = Vec::new();
        let processed = drive_queue(
            &transport,
            &queue,
            "token",
            &UploadTarget::default(),
            &no_replays(),
            || true,
            |event| events.push(event),
        )
        .unwrap();

        assert_eq!(processed, 2, "a refused file is a file finished with");
        assert!(events.iter().any(|event| matches!(
            event,
            QueueDriveEvent::Refused { sha256, code, .. }
                if sha256 == "aaa" && code == "capture_content_refused"
        )));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, QueueDriveEvent::Failed { .. })),
            "refused is not failed"
        );
        let refused = queue.find_by_sha256("aaa").unwrap().unwrap();
        assert_eq!(refused.state, QueueState::Refused);
        assert_eq!(
            refused.last_error_code.as_deref(),
            Some("capture_content_refused")
        );
        assert_eq!(
            queue.find_by_sha256("bbb").unwrap().unwrap().state,
            QueueState::Completed
        );
        // Nothing was scripted for a second create of the refused file.
    }

    /// The space's storage is full: the run stops on the first file, says so
    /// once, and leaves every file in the queue -- the other one is not even
    /// tried, since it would be refused the same way. Started again after
    /// the person acted, the run resumes from the offset already reached.
    #[test]
    fn drive_queue_stops_on_a_full_storage_and_resumes_when_started_again() {
        let (_dir, _source, queue) = queue_with_files(
            "gui-drive-held",
            &[("first.tif", "aaa"), ("second.tif", "bbb")],
        );
        let transport = FakeUploadTransport {
            creates: RefCell::new(VecDeque::from([Ok(session_of("upload-a", 0, false))])),
            puts: RefCell::new(VecDeque::from([Err(UploadError::refusal(
                409,
                "storage_quota_reached",
                "quota",
            ))])),
            finalizes: RefCell::new(VecDeque::new()),
        };
        let mut events = Vec::new();
        let processed = drive_queue(
            &transport,
            &queue,
            "token",
            &UploadTarget::default(),
            &no_replays(),
            || true,
            |event| events.push(event),
        )
        .unwrap();

        assert_eq!(processed, 0);
        assert_eq!(
            event_kinds(&events),
            ["started", "held", "stopped"],
            "one notice, then the run stops: {events:?}"
        );
        assert!(matches!(
            events.last(),
            Some(QueueDriveEvent::Stopped {
                reason: StopReason::Held,
                ..
            })
        ));
        assert_eq!(
            queue.find_by_sha256("aaa").unwrap().unwrap().state,
            QueueState::Held
        );
        assert_eq!(
            queue.find_by_sha256("bbb").unwrap().unwrap().state,
            QueueState::Queued,
            "the other file is untouched"
        );

        // The person made room and starts again: the held file goes on from
        // its session (a status call, then the chunk), the other follows.
        let resumed = FakeResumeTransport::default();
        resumed
            .statuses
            .borrow_mut()
            .push_back(Ok(session_of("upload-a", 0, false)));
        resumed
            .creates
            .borrow_mut()
            .push_back(Ok(session_of("upload-b", 0, false)));
        for capture in ["capture-a", "capture-b"] {
            resumed
                .puts
                .borrow_mut()
                .push_back(Ok(PutOutcome::Advanced(session_of("x", 4, false))));
            resumed.finalizes.borrow_mut().push_back(sent_ok(capture));
        }
        let mut events = Vec::new();
        let processed = drive_queue(
            &resumed,
            &queue,
            "token",
            &UploadTarget::default(),
            &no_replays(),
            || true,
            |event| events.push(event),
        )
        .unwrap();
        assert_eq!(processed, 2);
        assert_eq!(
            queue.find_by_sha256("aaa").unwrap().unwrap().state,
            QueueState::Completed
        );
        assert_eq!(
            queue.find_by_sha256("bbb").unwrap().unwrap().state,
            QueueState::Completed
        );
    }

    /// Like [`FakeUploadTransport`], with a scriptable status call too.
    #[derive(Default)]
    struct FakeResumeTransport {
        creates: RefCell<VecDeque<Result<UploadSession, UploadError>>>,
        statuses: RefCell<VecDeque<Result<UploadSession, UploadError>>>,
        puts: RefCell<VecDeque<Result<PutOutcome, UploadError>>>,
        finalizes: RefCell<VecDeque<Result<FinalizeOutcome, UploadError>>>,
    }

    impl UploadTransport for FakeResumeTransport {
        fn create(
            &self,
            _token: &str,
            _metadata: &crate::uploader::UploadMetadata,
        ) -> Result<UploadSession, UploadError> {
            self.creates
                .borrow_mut()
                .pop_front()
                .expect("scripted create")
        }
        fn status(&self, _token: &str, _upload_id: &str) -> Result<UploadSession, UploadError> {
            self.statuses
                .borrow_mut()
                .pop_front()
                .expect("scripted status")
        }
        fn put_chunk(
            &self,
            _token: &str,
            _upload_id: &str,
            _offset: u64,
            _chunk: &[u8],
        ) -> Result<PutOutcome, UploadError> {
            self.puts.borrow_mut().pop_front().expect("scripted put")
        }
        fn finalize(&self, _token: &str, _upload_id: &str) -> Result<FinalizeOutcome, UploadError> {
            self.finalizes
                .borrow_mut()
                .pop_front()
                .expect("scripted finalize")
        }
    }

    /// Too many uploads are open in the space: the file is not failed, the
    /// run says it will wait, waits (longer each time), tries again by itself
    /// and finishes the file.
    #[test]
    fn drive_queue_waits_and_resumes_by_itself_when_the_space_has_too_many_uploads_open() {
        let (_dir, _source, queue) = queue_with_one_file("gui-drive-sessions-limit");
        let too_many = || {
            Err(UploadError::refusal(
                409,
                "upload_sessions_limit_reached",
                "too many open uploads",
            ))
        };
        let transport = FakeUploadTransport {
            creates: RefCell::new(VecDeque::from([
                too_many(),
                too_many(),
                too_many(),
                Ok(session_of("upload-1", 0, false)),
            ])),
            puts: RefCell::new(VecDeque::from([Ok(PutOutcome::Advanced(session_of(
                "upload-1", 4, false,
            )))])),
            finalizes: RefCell::new(VecDeque::from([sent_ok("capture-1")])),
        };
        let options = DriveOptions {
            backoff: Backoff::new(vec![
                Duration::from_millis(5),
                Duration::from_millis(10),
                Duration::from_millis(20),
            ]),
            max_replays: None,
            replay_when_unanswered: true,
        };
        let mut events = Vec::new();
        let started = Instant::now();
        let processed = drive_queue(
            &transport,
            &queue,
            "token",
            &UploadTarget::default(),
            &options,
            || true,
            |event| events.push(event),
        )
        .unwrap();

        assert_eq!(processed, 1);
        assert!(started.elapsed() >= Duration::from_millis(35), "it waited");
        let waits: Vec<(u32, &str)> = events
            .iter()
            .filter_map(|event| match event {
                QueueDriveEvent::Waiting { attempt, code, .. } => Some((*attempt, code.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(
            waits,
            [
                (1, "upload_sessions_limit_reached"),
                (2, "upload_sessions_limit_reached"),
                (3, "upload_sessions_limit_reached")
            ]
        );
        assert!(
            !events.iter().any(|event| matches!(
                event,
                QueueDriveEvent::Failed { .. } | QueueDriveEvent::Refused { .. }
            )),
            "the file was never marked: {events:?}"
        );
        assert_eq!(
            queue.list().unwrap()[0].state,
            QueueState::Completed,
            "and it went through"
        );
    }

    #[test]
    fn the_wait_grows_with_each_try_and_stays_at_its_last_step() {
        let backoff = Backoff::standard();
        let waits: Vec<u64> = (1..=9)
            .map(|attempt| backoff.wait(attempt).as_secs())
            .collect();
        assert_eq!(waits, [15, 30, 60, 120, 300, 600, 900, 900, 900]);
        for pair in waits.windows(2) {
            assert!(pair[1] >= pair[0], "never shorter: {waits:?}");
        }
        assert_eq!(Backoff::new(Vec::new()).wait(1), Duration::from_secs(1));
    }

    /// The window pauses during a wait: the run ends at once, the file stays
    /// queued and unmarked.
    #[test]
    fn a_pause_during_a_wait_ends_the_run_at_once_and_leaves_the_file_queued() {
        let (_dir, _source, queue) = queue_with_one_file("gui-drive-pause-in-wait");
        let transport = FakeUploadTransport {
            creates: RefCell::new(VecDeque::from([Err(UploadError::Transport(
                "connection reset".to_string(),
            ))])),
            ..FakeUploadTransport::default()
        };
        let options = DriveOptions {
            backoff: Backoff::new(vec![Duration::from_secs(600)]),
            max_replays: None,
            replay_when_unanswered: true,
        };
        let mut events = Vec::new();
        let paused = Cell::new(false);
        let started = Instant::now();
        let processed = drive_queue(
            &transport,
            &queue,
            "token",
            &UploadTarget::default(),
            &options,
            || !paused.get(),
            |event| {
                if matches!(event, QueueDriveEvent::Waiting { .. }) {
                    paused.set(true);
                }
                events.push(event);
            },
        )
        .unwrap();

        assert_eq!(processed, 0);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the wait was cut short"
        );
        assert!(matches!(
            events.last(),
            Some(QueueDriveEvent::Stopped {
                reason: StopReason::Paused,
                ..
            })
        ));
        assert_eq!(queue.list().unwrap()[0].state, QueueState::Queued);
    }

    /// A service that keeps failing is not asked for ever by a run that was
    /// given a limit: it stops, says so, and leaves the file queued.
    #[test]
    fn a_run_with_a_limit_stops_after_that_many_new_tries() {
        let (_dir, _source, queue) = queue_with_one_file("gui-drive-limit");
        let failing = || Err(UploadError::refusal(500, "internal_error", "boom"));
        let transport = FakeUploadTransport {
            creates: RefCell::new(VecDeque::from([failing(), failing(), failing()])),
            ..FakeUploadTransport::default()
        };
        let options = DriveOptions {
            backoff: Backoff::new(vec![Duration::from_millis(1)]),
            max_replays: Some(2),
            replay_when_unanswered: true,
        };
        let mut events = Vec::new();
        let processed = drive_queue(
            &transport,
            &queue,
            "token",
            &UploadTarget::default(),
            &options,
            || true,
            |event| events.push(event),
        )
        .unwrap();

        assert_eq!(processed, 0);
        assert_eq!(
            event_kinds(&events),
            ["started", "waiting", "waiting", "failed", "stopped"],
            "two new tries, then the run stops on it: {events:?}"
        );
        assert!(events.iter().any(|event| matches!(
            event,
            QueueDriveEvent::Failed { code, .. } if code == "service_error"
        )));
        assert!(matches!(
            events.last(),
            Some(QueueDriveEvent::Stopped {
                reason: StopReason::NoProgress,
                ..
            })
        ));
        assert_eq!(queue.list().unwrap()[0].state, QueueState::Queued);
        assert_eq!(
            queue.list().unwrap()[0].last_error_code,
            None,
            "a wait is not a mark"
        );
    }

    /// The command line does not wait for an address that cannot be reached
    /// -- it fails at once, as it always did -- but does wait for a service
    /// that answers that it is busy.
    #[test]
    fn a_run_that_does_not_wait_for_silence_ends_at_once_on_an_unreachable_service() {
        let options = DriveOptions {
            backoff: Backoff::new(vec![Duration::from_millis(1)]),
            max_replays: Some(4),
            replay_when_unanswered: false,
        };

        let (_dir, _source, queue) = queue_with_one_file("gui-drive-silence");
        let transport = FakeUploadTransport {
            creates: RefCell::new(VecDeque::from([Err(UploadError::Transport(
                "connection refused".to_string(),
            ))])),
            ..FakeUploadTransport::default()
        };
        let mut events = Vec::new();
        drive_queue(
            &transport,
            &queue,
            "token",
            &UploadTarget::default(),
            &options,
            || true,
            |event| events.push(event),
        )
        .unwrap();
        assert_eq!(
            event_kinds(&events),
            ["started", "failed", "stopped"],
            "no wait: {events:?}"
        );
        assert_eq!(queue.list().unwrap()[0].state, QueueState::Queued);

        // The same run waits for a service that answers "too many uploads".
        let (_dir, _source, queue) = queue_with_one_file("gui-drive-answered");
        let transport = FakeUploadTransport {
            creates: RefCell::new(VecDeque::from([
                Err(UploadError::refusal(
                    409,
                    "upload_sessions_limit_reached",
                    "too many",
                )),
                Ok(session_of("upload-1", 0, false)),
            ])),
            puts: RefCell::new(VecDeque::from([Ok(PutOutcome::Advanced(session_of(
                "upload-1", 4, false,
            )))])),
            finalizes: RefCell::new(VecDeque::from([sent_ok("capture-1")])),
        };
        let mut events = Vec::new();
        let processed = drive_queue(
            &transport,
            &queue,
            "token",
            &UploadTarget::default(),
            &options,
            || true,
            |event| events.push(event),
        )
        .unwrap();
        assert_eq!(processed, 1);
        assert!(event_kinds(&events).contains(&"waiting"), "{events:?}");
    }

    /// `Retry-After` is honoured when it asks for more than the run would
    /// have waited.
    #[test]
    fn a_retry_after_longer_than_the_step_is_the_wait_announced() {
        let (_dir, _source, queue) = queue_with_one_file("gui-drive-retry-after");
        let transport = FakeUploadTransport {
            creates: RefCell::new(VecDeque::from([Err(UploadError::Refused {
                status: 429,
                code: "rate_limited".to_string(),
                message: "slow down".to_string(),
                retry_after_seconds: Some(120),
            })])),
            ..FakeUploadTransport::default()
        };
        let options = DriveOptions {
            backoff: Backoff::new(vec![Duration::from_secs(15)]),
            max_replays: None,
            replay_when_unanswered: true,
        };
        let mut events = Vec::new();
        let paused = Cell::new(false);
        drive_queue(
            &transport,
            &queue,
            "token",
            &UploadTarget::default(),
            &options,
            || !paused.get(),
            |event| {
                if matches!(event, QueueDriveEvent::Waiting { .. }) {
                    paused.set(true);
                }
                events.push(event);
            },
        )
        .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            QueueDriveEvent::Waiting {
                retry_in_seconds: 120,
                ..
            }
        )));
    }

    #[test]
    fn the_new_drive_events_serialize_as_the_front_end_reads_them() {
        for (event, expected) in [
            (
                QueueDriveEvent::Refused {
                    sha256: "abc".to_string(),
                    code: "capture_content_refused".to_string(),
                    message: "detail".to_string(),
                },
                serde_json::json!({"kind": "Refused", "sha256": "abc", "code": "capture_content_refused", "message": "detail"}),
            ),
            (
                QueueDriveEvent::Held {
                    sha256: "abc".to_string(),
                    code: "storage_quota_reached".to_string(),
                    message: "detail".to_string(),
                },
                serde_json::json!({"kind": "Held", "sha256": "abc", "code": "storage_quota_reached", "message": "detail"}),
            ),
            (
                QueueDriveEvent::Waiting {
                    sha256: "abc".to_string(),
                    code: "upload_sessions_limit_reached".to_string(),
                    message: "detail".to_string(),
                    retry_in_seconds: 30,
                    attempt: 2,
                },
                serde_json::json!({"kind": "Waiting", "sha256": "abc", "code": "upload_sessions_limit_reached", "message": "detail", "retryInSeconds": 30, "attempt": 2}),
            ),
            (
                QueueDriveEvent::Stopped {
                    sha256: Some("abc".to_string()),
                    reason: StopReason::Held,
                },
                serde_json::json!({"kind": "Stopped", "sha256": "abc", "reason": "held"}),
            ),
        ] {
            assert_eq!(serde_json::to_value(event).unwrap(), expected);
        }
    }

    #[test]
    fn a_refused_and_a_held_file_are_counted_apart_in_the_summary() {
        let (_dir, _source, queue) = queue_with_files(
            "gui-summary-states",
            &[("a.tif", "aaa"), ("b.tif", "bbb"), ("c.tif", "ccc")],
        );
        queue.mark_refused("aaa", "file_too_large", "413").unwrap();
        queue.mark_progress("bbb", "upload-b", 3).unwrap();
        queue.hold("bbb", "storage_quota_reached", "409").unwrap();
        let summary = summarize_queue(&queue.list().unwrap());
        assert_eq!((summary.refused, summary.held, summary.queued), (1, 1, 1));
        assert_eq!(summary.failed, 0);
        assert_eq!(
            summary.sent_bytes, 3,
            "what a held file already sent counts"
        );
    }

    /// With a cached capability profile the refresh is a background courtesy:
    /// it must not hold up the application for a service that is asleep, and
    /// must not announce anything. With no cache there is nothing else to
    /// show, so it waits as patiently as any other call and says so.
    #[test]
    fn the_capabilities_refresh_only_waits_when_there_is_no_cached_profile() {
        let dir = TempDir::new("gui-capabilities-plan");
        let path = dir.path().join("capabilities.json");

        let patient = WakePolicy::default();
        let background = WakePolicy::background_refresh();
        let (policy, announce) = capabilities_refresh_plan(&path, &patient, &background);
        assert_eq!(policy, patient);
        assert!(announce);

        save_capabilities_cache(
            &path,
            &CapabilitiesCache {
                capabilities: sample_capabilities(true),
                fetched_at_utc: "2026-09-20T00:00:00Z".to_string(),
                standing: None,
            },
        )
        .unwrap();
        let (policy, announce) = capabilities_refresh_plan(&path, &patient, &background);
        assert_eq!(policy, background);
        assert!(!announce);
        assert!(policy.total_budget < Duration::from_secs(15));
    }
}

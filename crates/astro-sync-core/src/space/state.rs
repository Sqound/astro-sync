//! What stays on the computer between two launches, and the steps of a
//! sign-in that both the desktop application and the command line take.
//!
//! Four small files live in the application's own directory (never in a
//! folder associated with a session):
//!
//! - `settings.json` ([`crate::gui_bridge::GuiSettings`]): the address of the
//!   service and the name of this computer, as for a pairing;
//! - the device token, in the system keyring with the owner-only file as a
//!   fallback ([`crate::gui_bridge::save_device_token`]): **the same store,
//!   the same account (the address of the service) and the same reading as
//!   the token of a pairing**, so every call that sends a file, reads a
//!   session or refreshes the capabilities works unchanged;
//! - the space record ([`SpaceRecord`]): which space this computer is signed
//!   in to, at what web address, under what name -- so the screen shows them
//!   without a network -- or, after the service ended the sign-in, the
//!   sentence-to-be ([`SignInEndedNotice`]) and the address to offer again;
//! - the installation identifier ([`load_or_create_installation_id`]).
//!
//! None of these is the upload queue or the folders associated with
//! sessions: ending or leaving a sign-in never touches them.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{
    DeviceIdentity, HOSTED_SERVICE_ORIGIN, MAX_DEVICE_NAME_CHARS, RegisteredDevice, SignInEnd,
    SignedIn, SpaceClient, SpaceError, SpaceInfo, normalize_device_name,
};
use crate::gui_bridge::{self, GuiBridgeError, GuiSettings};
use crate::service_wait::ServiceWaitError;

/// The files of a sign-in, each at the path the application chose.
#[derive(Debug, Clone)]
pub struct SpaceFiles {
    /// `settings.json` (`gui_bridge::load_settings`).
    pub settings: PathBuf,
    /// The owner-only fallback file of the device token
    /// (`gui_bridge::save_device_token`).
    pub device_token: PathBuf,
    /// The [`SpaceRecord`].
    pub record: PathBuf,
    /// The installation identifier.
    pub installation_id: PathBuf,
}

/// Where the device token is kept, behind a trait so the steps below can be
/// played without touching a real keyring.
pub trait TokenStore {
    fn save(&self, token: &str) -> Result<(), GuiBridgeError>;
    /// Forgets the token. Nothing to forget is not an error.
    fn clear(&self) -> Result<(), GuiBridgeError>;
}

/// The store of a pairing: the system keyring under the address of the
/// service, the owner-only file as a fallback.
#[derive(Debug, Clone)]
pub struct SystemTokenStore {
    token_path: PathBuf,
    account: String,
}

impl SystemTokenStore {
    #[must_use]
    pub fn new(token_path: impl Into<PathBuf>, account: impl Into<String>) -> Self {
        Self {
            token_path: token_path.into(),
            account: account.into(),
        }
    }
}

impl TokenStore for SystemTokenStore {
    fn save(&self, token: &str) -> Result<(), GuiBridgeError> {
        gui_bridge::save_device_token(&self.token_path, &self.account, token)
    }

    fn clear(&self) -> Result<(), GuiBridgeError> {
        gui_bridge::clear_device_token(&self.token_path, Some(&self.account))
    }
}

/// The space this computer is signed in to by e-mail code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceLink {
    /// The service the token belongs to: the link only counts while the
    /// application still points at it.
    pub portal_url: String,
    pub space: SpaceInfo,
    pub device: RegisteredDevice,
}

/// A sign-in the service ended, remembered so the sign-in screen can say so.
/// It holds the reason and the moment, **nothing of the space**: what this
/// computer knew of it -- the token, the space, its address -- is forgotten.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignInEndedNotice {
    pub reason: SignInEnd,
    pub at: String,
}

/// The record file: a link, or the notice of a sign-in that ended, or nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceRecord {
    #[serde(default)]
    pub link: Option<SpaceLink>,
    #[serde(default)]
    pub ended: Option<SignInEndedNotice>,
}

fn io_error(path: &Path, source: std::io::Error) -> SpaceError {
    SpaceError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn ensure_parent(path: &Path) -> Result<(), SpaceError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;
    }
    Ok(())
}

/// Reads the record. A missing file is an empty record, and so is one that
/// cannot be read as a record: it holds nothing that cannot be asked of the
/// service again, and a damaged file must not lock the person out of signing
/// in.
pub fn load_record(path: &Path) -> Result<SpaceRecord, SpaceError> {
    match fs::read_to_string(path) {
        Ok(raw) => Ok(serde_json::from_str(&raw).unwrap_or_default()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(SpaceRecord::default()),
        Err(error) => Err(io_error(path, error)),
    }
}

/// Writes the record; an empty one removes the file.
pub fn save_record(path: &Path, record: &SpaceRecord) -> Result<(), SpaceError> {
    if *record == SpaceRecord::default() {
        return match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error(path, error)),
        };
    }
    ensure_parent(path)?;
    let json = serde_json::to_string_pretty(record).expect("a space record always serialises");
    fs::write(path, json).map_err(|source| io_error(path, source))
}

fn is_valid_installation_id(candidate: &str) -> bool {
    (16..=128).contains(&candidate.len())
        && candidate.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        })
}

/// The identifier of this installation, generated once (random bytes from the
/// operating system) and kept in `path`; a file that holds something else is
/// replaced. It is not a secret -- it names this computer's line in "My
/// devices" -- and it is not derived from anything about the machine.
pub fn load_or_create_installation_id(path: &Path) -> Result<String, SpaceError> {
    if let Ok(raw) = fs::read_to_string(path) {
        let existing = raw.trim();
        if is_valid_installation_id(existing) {
            return Ok(existing.to_string());
        }
    }
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| io_error(path, std::io::Error::other(error.to_string())))?;
    // 64 symbols, a byte reduced to six bits: no bias.
    let random: String = bytes
        .iter()
        .map(|byte| char::from(ALPHABET[usize::from(byte & 63)]))
        .collect();
    let identifier = format!("astrosync-{random}");
    ensure_parent(path)?;
    fs::write(path, &identifier).map_err(|source| io_error(path, source))?;
    Ok(identifier)
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// The name of this machine, as its owner set it: "Camille's MacBook Pro"
/// (macOS `ComputerName`), else the host name without its domain.
#[must_use]
pub fn machine_name() -> Option<String> {
    #[cfg(target_os = "macos")]
    if let Some(name) = command_output("scutil", &["--get", "ComputerName"]) {
        return Some(name);
    }
    let host = std::env::var("COMPUTERNAME")
        .ok()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| command_output("hostname", &[]))
        .or_else(|| {
            std::env::var("HOSTNAME")
                .ok()
                .filter(|name| !name.trim().is_empty())
        })?;
    let short = host.trim().split('.').next().unwrap_or_default().trim();
    (!short.is_empty()).then(|| short.to_string())
}

/// « Astro Sync — MacBook Pro »: how the contract's own example names this
/// application in "My devices", next to the phones (`fixtures/api/
/// sign-in-verify.json`, `verify-code-astro-sync`).
#[must_use]
pub fn device_name_for(machine: Option<&str>) -> String {
    let machine = machine.map(str::trim).filter(|name| !name.is_empty());
    let name = match machine {
        Some(machine) => format!("Astro Sync \u{2014} {machine}"),
        None => "Astro Sync".to_string(),
    };
    name.chars().take(MAX_DEVICE_NAME_CHARS).collect()
}

/// The name this computer signs in under unless the person chose another.
#[must_use]
pub fn default_device_name() -> String {
    device_name_for(machine_name().as_deref())
}

impl DeviceIdentity {
    /// This installation, whose identifier is kept in `installation_id_file`,
    /// under `name`.
    pub fn at(installation_id_file: &Path, name: &str) -> Result<Self, SpaceError> {
        Ok(Self {
            installation_id: load_or_create_installation_id(installation_id_file)?,
            name: normalize_device_name(name)?,
            model: None,
        })
    }

    /// This installation, under `name`, with the identifier kept where
    /// `files` say.
    pub fn for_installation(files: &SpaceFiles, name: &str) -> Result<Self, SpaceError> {
        Self::at(&files.installation_id, name)
    }
}

/// What the interface shows: whether this computer is connected to the
/// service, to which space, and if a sign-in just ended, why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceStatus {
    /// The address of the service in force: the saved one, else the hosted
    /// service's.
    pub service_url: String,
    /// Whether that is the hosted service.
    pub hosted: bool,
    /// A token is kept for that address.
    pub paired: bool,
    /// Signed in by e-mail code: the space and the name of this computer.
    pub space: Option<SpaceInfo>,
    pub device: Option<RegisteredDevice>,
    /// The service ended the last sign-in: what the sign-in screen says.
    pub ended: Option<SignInEndedNotice>,
    /// The name a new sign-in registers this computer under.
    pub default_device_name: String,
}

/// Whether `url` is the hosted service. A private server, whatever its
/// address, is not.
#[must_use]
pub fn is_hosted_service(url: &str) -> bool {
    same_service(url, HOSTED_SERVICE_ORIGIN)
}

fn same_service(left: &str, right: &str) -> bool {
    left.trim()
        .trim_end_matches('/')
        .eq_ignore_ascii_case(right.trim().trim_end_matches('/'))
}

/// The address of the service the application points at: the saved one, else
/// the hosted service's.
pub fn service_url(settings: &GuiSettings) -> String {
    settings
        .portal_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .unwrap_or(HOSTED_SERVICE_ORIGIN)
        .to_string()
}

/// The state of the connection, from the files alone (no network). `paired`
/// is whether a token is kept for the service in force -- the caller reads
/// it from the token store, which it alone knows how to reach.
pub fn space_status(files: &SpaceFiles, paired: bool) -> Result<SpaceStatus, SpaceError> {
    let settings = gui_bridge::load_settings(&files.settings)?;
    let service_url = service_url(&settings);
    let record = load_record(&files.record)?;
    let link = record
        .link
        .filter(|link| paired && same_service(&link.portal_url, &service_url));
    let default_device_name = match settings
        .device_label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
    {
        Some(label) => label.to_string(),
        None => default_device_name(),
    };
    Ok(SpaceStatus {
        hosted: same_service(&service_url, HOSTED_SERVICE_ORIGIN),
        service_url,
        paired,
        space: link.as_ref().map(|link| link.space.clone()),
        device: link.map(|link| link.device),
        ended: if paired { None } else { record.ended },
        default_device_name,
    })
}

/// Keeps what a verified sign-in returned: the token first (nothing else
/// changes if that fails), then the service's address and the name of this
/// computer in the settings -- exactly what a pairing saves -- and the
/// record of the space.
pub fn complete_sign_in(
    files: &SpaceFiles,
    store: &dyn TokenStore,
    portal_url: &str,
    signed_in: &SignedIn,
) -> Result<SpaceStatus, SpaceError> {
    let portal_url = portal_url.trim();
    store.save(signed_in.device_token.expose())?;
    let mut settings = gui_bridge::load_settings(&files.settings)?;
    settings.portal_url = Some(portal_url.to_string());
    settings.device_label = Some(signed_in.device.name.clone());
    gui_bridge::save_settings(&files.settings, &settings)?;
    save_record(
        &files.record,
        &SpaceRecord {
            link: Some(SpaceLink {
                portal_url: portal_url.to_string(),
                space: signed_in.space.clone(),
                device: signed_in.device.clone(),
            }),
            ended: None,
        },
    )?;
    space_status(files, true)
}

/// The service ended the sign-in (`device_detached`, `space_deleted`): the
/// token is forgotten and the reason kept for the sign-in screen. **Nothing
/// else is touched** -- not the folders associated with sessions, not the
/// upload queue: they wait for the next sign-in. Saying it twice is the same
/// as once.
pub fn end_sign_in(
    files: &SpaceFiles,
    store: &dyn TokenStore,
    end: SignInEnd,
) -> Result<SpaceStatus, SpaceError> {
    store.clear()?;
    save_record(
        &files.record,
        &SpaceRecord {
            link: None,
            ended: Some(SignInEndedNotice {
                reason: end,
                at: crate::time_util::now_iso8601_utc(),
            }),
        },
    )?;
    space_status(files, false)
}

/// Whether what the service said ends **this** computer's sign-in by e-mail
/// code. `device_detached` and `space_deleted` always do. `device_
/// authentication_required` -- the token was replaced by a newer sign-in --
/// only does when this computer is signed in to a space (a link is on record):
/// on a private server the same answer is an ordinary revoked device, which
/// is none of this sign-in's business and is left as it always was.
pub fn ends_this_sign_in(files: &SpaceFiles, end: SignInEnd) -> Result<bool, SpaceError> {
    Ok(end.forgets_the_space() || load_record(&files.record)?.link.is_some())
}

/// Asks the service who this token is, and remembers the answer. A token the
/// service ended ([`SignInEnd`]) is forgotten ([`end_sign_in`]) and reported
/// as an error; any other failure (no network, the service asleep) leaves
/// everything as it was, for the caller to fall back on what is on disk.
pub fn refresh_space(
    client: &SpaceClient,
    files: &SpaceFiles,
    store: &dyn TokenStore,
    token: &str,
) -> Result<SpaceStatus, SpaceError> {
    match client.current_space(token) {
        Ok(current) => {
            let settings = gui_bridge::load_settings(&files.settings)?;
            let record = load_record(&files.record)?;
            let previous_device = record.link.as_ref().map(|link| link.device.clone());
            if let Some(device) = current.device.or(previous_device) {
                save_record(
                    &files.record,
                    &SpaceRecord {
                        link: Some(SpaceLink {
                            portal_url: service_url(&settings),
                            space: current.space,
                            device,
                        }),
                        ended: None,
                    },
                )?;
            }
            space_status(files, true)
        }
        Err(error) => {
            if let Some(end) = error.sign_in_end() {
                end_sign_in(files, store, end)?;
            }
            Err(error)
        }
    }
}

/// The routes of the space devices (`PATCH`, `DELETE`
/// `/v1/devices/{id}`) may not be answered by every service: a route that does not exist, or that answers a credential it does not
/// take, is not a failure of the person's action.
fn route_not_answered_yet(error: &SpaceError) -> bool {
    error
        .refusal()
        .is_some_and(|refusal| matches!(refusal.status, 401 | 403 | 404 | 405))
}

/// What renaming this computer did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameOutcome {
    pub status: SpaceStatus,
    /// The service listed the new name (`PATCH /v1/devices/{id}`). `false`
    /// when it does not answer that route yet: the name is then kept on this
    /// computer only.
    pub service_updated: bool,
}

/// Changes the name of this computer: at the service, so "My devices"
/// lists it, and here.
pub fn rename_computer(
    client: &SpaceClient,
    files: &SpaceFiles,
    store: &dyn TokenStore,
    token: &str,
    name: &str,
) -> Result<RenameOutcome, SpaceError> {
    let name = normalize_device_name(name)?;
    let mut settings = gui_bridge::load_settings(&files.settings)?;
    let mut record = load_record(&files.record)?;
    let mut service_updated = false;
    if let Some(link) = record.link.as_mut() {
        match client.rename_device(token, &link.device.id, &name) {
            Ok(device) => {
                link.device = device;
                service_updated = true;
            }
            Err(error) if route_not_answered_yet(&error) => link.device.name = name.clone(),
            Err(error) => {
                if let Some(end) = error.sign_in_end() {
                    end_sign_in(files, store, end)?;
                }
                return Err(error);
            }
        }
        settings.device_label = Some(link.device.name.clone());
    } else {
        settings.device_label = Some(name);
    }
    gui_bridge::save_settings(&files.settings, &settings)?;
    save_record(&files.record, &record)?;
    Ok(RenameOutcome {
        status: space_status(files, true)?,
        service_updated,
    })
}

/// What detaching this computer did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetachOutcome {
    pub status: SpaceStatus,
    /// The service knows this computer left (`DELETE /v1/devices/{id}`
    /// answered, or the service had already detached it). `false` when it
    /// could not be told -- no network, or a service that does not answer
    /// that route yet: the computer is detached here all the same, and the
    /// service keeps listing it until it is removed from "My devices".
    pub service_notified: bool,
}

/// "Detach this computer": tells the service (as far as it can be
/// told), forgets the token and the link. The person's decision is carried
/// out here whatever the network says, except when they cancel the wait for
/// a starting service, which cancels the detachment. The folders associated
/// with sessions and the upload queue stay: what is waiting to be sent goes
/// on waiting, for the next sign-in.
pub fn detach_computer(
    client: &SpaceClient,
    files: &SpaceFiles,
    store: &dyn TokenStore,
    token: &str,
) -> Result<DetachOutcome, SpaceError> {
    let record = load_record(&files.record)?;
    let mut service_notified = false;
    if let Some(link) = &record.link {
        match client.detach_device(token, &link.device.id) {
            Ok(()) => service_notified = true,
            Err(error) if error.sign_in_end().is_some() => service_notified = true,
            Err(SpaceError::Wait(ServiceWaitError::Cancelled)) => {
                return Err(SpaceError::Wait(ServiceWaitError::Cancelled));
            }
            Err(_) => {}
        }
    }
    store.clear()?;
    save_record(&files.record, &SpaceRecord::default())?;
    Ok(DetachOutcome {
        status: space_status(files, false)?,
        service_notified,
    })
}

/// Writes a device token to a file only its owner can read -- the file the
/// command line's `upload --token` reads. Created with owner-only
/// permissions from the start (never a moment with wider ones) and
/// replaced if it exists.
pub fn write_token_file(path: &Path, token: &str) -> Result<(), SpaceError> {
    ensure_parent(path)?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|source| io_error(path, source))?;
    #[cfg(unix)]
    {
        // A file that already existed keeps its mode through `open`.
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|source| io_error(path, source))?;
    }
    file.write_all(token.as_bytes())
        .map_err(|source| io_error(path, source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::space::Secret;
    use crate::test_support::TempDir;
    use std::cell::RefCell;

    #[derive(Default)]
    struct MemoryStore {
        token: RefCell<Option<String>>,
    }

    impl TokenStore for MemoryStore {
        fn save(&self, token: &str) -> Result<(), GuiBridgeError> {
            *self.token.borrow_mut() = Some(token.to_string());
            Ok(())
        }
        fn clear(&self) -> Result<(), GuiBridgeError> {
            *self.token.borrow_mut() = None;
            Ok(())
        }
    }

    fn files_in(dir: &TempDir) -> SpaceFiles {
        SpaceFiles {
            settings: dir.path().join("settings.json"),
            device_token: dir.path().join("device-token"),
            record: dir.path().join("space.json"),
            installation_id: dir.path().join("installation-id"),
        }
    }

    fn signed_in() -> SignedIn {
        SignedIn {
            created: false,
            space: SpaceInfo {
                id: "4ec83832-fbb7-4f8d-91ca-8c7eefd8ab71".to_string(),
                email: "vous@exemple.fr".to_string(),
                web_url: "https://portal.example.test".to_string(),
            },
            device: RegisteredDevice {
                id: "98dc0200-d02c-44a3-899b-129fa99f3e0c".to_string(),
                name: "Astro Sync \u{2014} MacBook Pro".to_string(),
            },
            device_token: Secret::new("QJR3D-0HcwGBdSXJ8IpOIuINTTIPAl2OHFyn1hIeyUQ"),
        }
    }

    #[test]
    fn the_installation_identifier_is_made_once_and_kept() {
        let dir = TempDir::new("space-installation-id");
        let path = dir.path().join("nested/installation-id");
        let first = load_or_create_installation_id(&path).unwrap();
        assert!(is_valid_installation_id(&first), "{first}");
        assert!(first.starts_with("astrosync-"));
        assert_eq!(load_or_create_installation_id(&path).unwrap(), first);
        // Another installation has another one.
        let other = dir.path().join("other-installation-id");
        assert_ne!(load_or_create_installation_id(&other).unwrap(), first);
        // A file that holds something else is replaced by a valid identifier.
        fs::write(&path, "too short").unwrap();
        let replaced = load_or_create_installation_id(&path).unwrap();
        assert!(is_valid_installation_id(&replaced));
        assert_ne!(replaced, first);
    }

    #[test]
    fn the_default_name_of_this_computer_follows_the_contracts_example() {
        assert_eq!(
            device_name_for(Some("MacBook Pro")),
            "Astro Sync \u{2014} MacBook Pro"
        );
        assert_eq!(device_name_for(Some("  ")), "Astro Sync");
        assert_eq!(device_name_for(None), "Astro Sync");
        assert!(device_name_for(Some(&"x".repeat(300))).chars().count() <= MAX_DEVICE_NAME_CHARS);
    }

    #[test]
    fn a_completed_sign_in_is_kept_the_way_a_pairing_is() {
        let dir = TempDir::new("space-complete");
        let files = files_in(&dir);
        let store = MemoryStore::default();

        let status =
            complete_sign_in(&files, &store, "https://portal.example.test/", &signed_in()).unwrap();

        assert_eq!(
            store.token.borrow().as_deref(),
            Some("QJR3D-0HcwGBdSXJ8IpOIuINTTIPAl2OHFyn1hIeyUQ")
        );
        let settings = gui_bridge::load_settings(&files.settings).unwrap();
        assert_eq!(
            settings.portal_url.as_deref(),
            Some("https://portal.example.test/")
        );
        assert_eq!(
            settings.device_label.as_deref(),
            Some("Astro Sync \u{2014} MacBook Pro")
        );
        assert!(status.paired);
        assert_eq!(status.space.unwrap().email, "vous@exemple.fr");
        assert_eq!(
            status.device.unwrap().name,
            "Astro Sync \u{2014} MacBook Pro"
        );
        assert_eq!(status.ended, None);
        assert!(
            !status.hosted,
            "an address of the tests is not the hosted service"
        );
        // No file of the sign-in holds the token: it is in the store alone.
        for file in [&files.settings, &files.record] {
            assert!(
                !fs::read_to_string(file).unwrap().contains("QJR3D"),
                "{}",
                file.display()
            );
        }
    }

    #[test]
    fn a_sign_in_the_service_ended_is_forgotten_and_the_reason_kept() {
        let dir = TempDir::new("space-ended");
        let files = files_in(&dir);
        let store = MemoryStore::default();
        complete_sign_in(&files, &store, "https://portal.example.test", &signed_in()).unwrap();

        let status = end_sign_in(&files, &store, SignInEnd::DeviceDetached).unwrap();

        assert!(store.token.borrow().is_none(), "the token is forgotten");
        assert!(!status.paired);
        assert_eq!(status.space, None);
        let ended = status.ended.expect("the reason is kept");
        assert_eq!(ended.reason, SignInEnd::DeviceDetached);
        // The space is forgotten -- not its address either: the notice holds
        // the reason and the moment, nothing of the space.
        let on_disk = fs::read_to_string(&files.record).unwrap();
        assert!(
            !on_disk.contains("vous@exemple.fr") && !on_disk.contains("4ec83832"),
            "{on_disk}"
        );
        // Saying it twice is the same as once.
        let again = end_sign_in(&files, &store, SignInEnd::DeviceDetached).unwrap();
        assert_eq!(again.ended.unwrap().reason, SignInEnd::DeviceDetached);
        // The address of the service stays for the next sign-in.
        assert_eq!(again.service_url, "https://portal.example.test");
        // Signing in again clears the notice.
        let back =
            complete_sign_in(&files, &store, "https://portal.example.test", &signed_in()).unwrap();
        assert_eq!(back.ended, None);
        assert!(back.paired);
    }

    #[test]
    fn a_link_only_counts_for_the_service_it_was_made_with() {
        let dir = TempDir::new("space-other-service");
        let files = files_in(&dir);
        let store = MemoryStore::default();
        complete_sign_in(&files, &store, "https://portal.example.test", &signed_in()).unwrap();
        // The application is pointed at a private server afterwards.
        gui_bridge::save_settings(
            &files.settings,
            &GuiSettings {
                portal_url: Some("https://private.example.test".to_string()),
                ..GuiSettings::default()
            },
        )
        .unwrap();

        let status = space_status(&files, true).unwrap();

        assert!(status.paired);
        assert_eq!(
            status.space, None,
            "a private pairing has no space of its own"
        );
        assert_eq!(status.service_url, "https://private.example.test");
    }

    #[test]
    fn without_settings_the_service_is_the_hosted_one() {
        let dir = TempDir::new("space-default-service");
        let files = files_in(&dir);
        let status = space_status(&files, false).unwrap();
        assert_eq!(status.service_url, HOSTED_SERVICE_ORIGIN);
        assert!(status.hosted);
        assert!(!status.paired);
        assert!(status.default_device_name.starts_with("Astro Sync"));
    }

    #[test]
    fn a_damaged_record_is_an_empty_one() {
        let dir = TempDir::new("space-damaged-record");
        let files = files_in(&dir);
        fs::write(&files.record, "{ not json").unwrap();
        assert_eq!(load_record(&files.record).unwrap(), SpaceRecord::default());
    }

    #[cfg(unix)]
    #[test]
    fn a_token_file_is_owner_only_from_the_start_and_replaced() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("space-token-file");
        let path = dir.path().join("private/token.txt");
        write_token_file(&path, "first-token").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "first-token");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        // A file that already exists with wider permissions is tightened.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        write_token_file(&path, "second").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

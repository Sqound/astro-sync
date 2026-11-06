//! Signing in to the hosted service by e-mail code.
//!
//! On the hosted service Astro Sync signs in with an e-mail address, then
//! the code of six digits received by e-mail, no password. It announces itself as the application `sync`
//! (`device.application` of `contracts/schemas/api/sign-in.schema.json`):
//! it is a helper that sends captures and logs, not a device under the
//! « one device, one space » rule, so it is never refused, never limited and
//! never announced to another space. On a private server nothing changes: it
//! keeps pairing with the code the server shows (`gui_bridge::claim_device`).
//!
//! The contract is `contracts/schemas/api/v1.md` (sections
//! « How a person signs in: `signIn` », « Hosted sign-in by e-mail code » and
//! « Space error codes »); the exchanges it records under
//! `contracts/fixtures/api/` are replayed against this code by
//! the tests, on a fake service.
//!
//! - [`client`]: the HTTP side. [`SpaceClient::announcement`] reads what the
//!   service says about how a person signs in (an absent announcement is a
//!   private server), [`SpaceClient::request_code`] has a code sent,
//!   [`SpaceClient::verify_code`] exchanges it for a device token, and the
//!   space of the token is read, its device renamed or detached.
//! - [`state`]: what stays on the computer between two launches -- the
//!   installation identifier, the link to the space (its address, the name
//!   of this computer), the reason a sign-in ended -- and the orchestration
//!   both the desktop application and the command line share: complete a
//!   sign-in, end one, refresh, rename, detach.
//!
//! **The token** the service hands out is kept and used *exactly like the
//! one of a pairing*: the system keyring under the service's address, with
//! the owner-only file as a fallback (`gui_bridge::save_device_token`); every
//! call sends it as `Authorization: Bearer <token>`. It never appears in a
//! log, in an error, in a `Debug` output nor on any output: it lives in a
//! [`Secret`], whose `Debug` says `<hidden>` and which has no `Display` and no `Serialize`.
//!
//! **A sign-in that the service ends** (`401` `device_detached` or
//! `space_deleted`, described by the contract for the space devices and
//! of the deletion) is [`SignInEnd`]: every call that carries the token
//! reports it as such, the token is forgotten and the person is brought back
//! to the sign-in screen with a sentence that says why. Nothing else is
//! lost: the folders associated with sessions and the files waiting in the
//! upload queue stay exactly as they were, and the pending uploads are never
//! marked failed by it.

pub mod client;
pub mod state;

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use thiserror::Error;

use crate::gui_bridge::GuiBridgeError;
use crate::service_wait::{HttpFailure, ServiceWaitError};

pub use client::{
    CodeRequested, CurrentSpace, RegisteredDevice, SignInAnnouncement, SignedIn, SpaceClient,
    SpaceCreation, SpaceInfo,
};
pub use state::{
    DetachOutcome, RenameOutcome, SignInEndedNotice, SpaceFiles, SpaceLink, SpaceRecord,
    SpaceStatus, SystemTokenStore, TokenStore, complete_sign_in, default_device_name,
    detach_computer, device_name_for, end_sign_in, ends_this_sign_in, is_hosted_service,
    load_or_create_installation_id, load_record, machine_name, refresh_space, rename_computer,
    save_record, service_url, space_status, write_token_file,
};

/// The hosted service, the default of a person who has not chosen a private
/// server.
pub const HOSTED_SERVICE_ORIGIN: &str = "https://portal.planastro.space";

/// The `device.application` Astro Sync announces itself with.
pub const SYNC_APPLICATION: &str = "sync";

/// The digits of a sign-in code (`sign-in.schema.json` `$defs/code`).
pub const CODE_LENGTH: usize = 6;

/// The longest name of a device (`sign-in.schema.json` `$defs/device.name`).
pub const MAX_DEVICE_NAME_CHARS: usize = 120;

/// Who is signing in: this installation of Astro Sync (`installation_id`, an
/// identifier of its own, generated once and kept next to the application's
/// other files) and the name "My devices" lists it under. The
/// application (`sync`) and the platform are Astro Sync's own and never vary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceIdentity {
    /// 16 to 128 characters of `A-Z a-z 0-9 _ -`
    /// ([`state::load_or_create_installation_id`]).
    pub installation_id: String,
    /// By default the name of the machine ([`state::default_device_name`]).
    pub name: String,
    /// The model of the machine, when known; optional in the contract.
    pub model: Option<String>,
}

/// A secret the service handed out (the device token): kept in memory only as
/// long as needed, never displayed. See the module documentation.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The value, for the one place that must send or store it.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret(<hidden>)")
    }
}

/// Why the service ended a sign-in that was working.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignInEnd {
    /// `401` `device_detached`: this computer was detached from its space
    /// (from the web, from a phone, from "My devices", or by itself).
    DeviceDetached,
    /// `401` `space_deleted`: the space was deleted.
    SpaceDeleted,
    /// `401` `device_authentication_required`: the service no longer knows
    /// this token. A new sign-in of the same installation in the same space
    /// replaces the token, and the old one answers so. The space is still
    /// there and so is this computer's place in it: it only has to sign in
    /// again. (On a private server this is a revoked device; see
    /// [`SignInEnd::forgets_the_space`].)
    ConnectionReplaced,
}

impl SignInEnd {
    /// The contract's error code, and the key the interface looks up as
    /// `errors.<code>`.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::DeviceDetached => "device_detached",
            Self::SpaceDeleted => "space_deleted",
            Self::ConnectionReplaced => "device_authentication_required",
        }
    }

    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "device_detached" => Some(Self::DeviceDetached),
            "space_deleted" => Some(Self::SpaceDeleted),
            "device_authentication_required" => Some(Self::ConnectionReplaced),
            _ => None,
        }
    }

    /// Whether the service says this computer has left the space (detached,
    /// or the space deleted) rather than only that this token is no longer
    /// the one in force. Only the first is certain on every kind of service:
    /// on a private server `device_authentication_required` is an ordinary
    /// revoked device, which is not a sign-in by e-mail code ending.
    #[must_use]
    pub fn forgets_the_space(self) -> bool {
        !matches!(self, Self::ConnectionReplaced)
    }

    /// The same sentence in plain words, for the command line.
    #[must_use]
    pub fn plain_message(self) -> &'static str {
        match self {
            Self::DeviceDetached => {
                "This computer was detached from your Astro Portal space. Sign in again to resume sending: your associated folders and pending uploads are kept."
            }
            Self::SpaceDeleted => {
                "Your Astro Portal space was deleted. Sign in to another space to resume sending: your associated folders and pending uploads are kept."
            }
            Self::ConnectionReplaced => {
                "The service no longer recognises this computer’s connection: a newer sign-in may have replaced it. Sign in again to resume sending; your associated folders and pending uploads are kept."
            }
        }
    }
}

/// A refusal the service answered in the v1 error envelope, with the extra
/// fields that sit beside `error` (`attemptsRemaining`, `retryAt`) and the
/// `Retry-After` header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub status: u16,
    /// The contract's code, as sent (`sign_in_code_incorrect`, ...).
    pub code: String,
    /// The service's own sentence (English): a fallback, never shown when the
    /// interface has a sentence of its own for the code.
    pub message: String,
    /// With `sign_in_code_incorrect`: how many wrong codes are left (4 to 1).
    pub attempts_remaining: Option<u8>,
    /// With `sign_in_locked`, `sign_in_rate_limited`, `service_starting`.
    pub retry_at: Option<String>,
    /// The `Retry-After` header, in seconds: what a countdown counts from,
    /// since the clock of this computer need not agree with the service's.
    pub retry_after_seconds: Option<u64>,
}

impl Refusal {
    /// The code the interface looks up as `errors.<code>`: the contract's own
    /// for every refusal of the sign-in routes, `portal_refused` for any
    /// other.
    #[must_use]
    pub fn static_code(&self) -> &'static str {
        match self.code.as_str() {
            "invalid_request" => "invalid_request",
            "email_invalid" => "email_invalid",
            "sign_in_code_incorrect" => "sign_in_code_incorrect",
            "sign_in_code_expired" => "sign_in_code_expired",
            "sign_in_link_invalid" => "sign_in_link_invalid",
            "sign_in_locked" => "sign_in_locked",
            "sign_in_rate_limited" => "sign_in_rate_limited",
            "email_delivery_failed" => "email_delivery_failed",
            "sign_in_unavailable" => "sign_in_unavailable",
            // `403` on the verification: the code is right, but the space of
            // this address is no longer available (the person was disabled).
            "space_unavailable" => "space_unavailable",
            "service_starting" => "service_starting",
            // `404 not_found` on a sign-in route: a service that does not
            // sign in by e-mail (a private server).
            "not_found" => "sign_in_wrong_service",
            // `409` on the verification, once the address is proven and the
            // code used: the space already has its ceiling of devices.
            // Detaching one, then asking for a new code, is the way forward.
            "device_limit_reached" => "device_limit_reached",
            // `403` on a private server signing in by e-mail code: the address
            // is not its owner's. `409` on the same server before its
            // installation: no address signs in until its owner installs it
            // on the web (`bootstrapPath`).
            "not_owner" => "not_owner",
            "not_initialized" => "not_initialized",
            _ => "portal_refused",
        }
    }
}

/// Every way a step of the hosted sign-in can fail.
#[derive(Debug, Error)]
pub enum SpaceError {
    /// The address of the service is not an absolute `http(s)` address.
    #[error(
        "the address of the service must be a full http(s) address, for example https://astro.example"
    )]
    InvalidServiceUrl(String),
    /// The e-mail address, once normalised, is not an address. Caught before
    /// anything is sent.
    #[error("enter a valid e-mail address")]
    InvalidEmail,
    /// The code does not hold six digits. Caught before anything is sent.
    #[error("the sign-in code has six digits")]
    InvalidCode,
    #[error("the name of this computer must not be empty")]
    EmptyDeviceName,
    /// The service could not be reached at all (name not found, connection
    /// refused, TLS failure, no network): nothing was sent.
    #[error("could not reach the service (address not found, connection refused or no network)")]
    Transport(String),
    /// The wait for a starting service ended without an answer, or a write
    /// was sent once and never confirmed, or the person cancelled.
    #[error(transparent)]
    Wait(#[from] ServiceWaitError),
    /// The answer could not be read; the text is the technical detail, for
    /// logs -- never for a person.
    #[error("the service answered in an unexpected way: update Astro Sync, or try again later")]
    InvalidResponse(String),
    /// Whatever answered is not an Astro portal.
    #[error("this address does not answer like an Astro Portal service")]
    NotAnAstroPortal,
    /// An Astro portal, but of an API version this Astro Sync does not speak.
    #[error("this service needs a newer version of Astro Sync: update the application")]
    IncompatibleApiVersion,
    #[error("{}", refusal_sentence(.0))]
    Refused(Refusal),
    #[error("{}", .0.plain_message())]
    SignInEnded(SignInEnd),
    #[error("{0}")]
    Bridge(#[from] GuiBridgeError),
    #[error("file unreadable or not writable ({path}): {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl SpaceError {
    /// The stable code an interface looks up as `errors.<code>`.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidServiceUrl(_) => "invalid_portal_url",
            Self::InvalidEmail => "email_invalid",
            Self::InvalidCode => "sign_in_code_malformed",
            Self::EmptyDeviceName => "empty_device_label",
            Self::Transport(_) => "network_error",
            Self::Wait(wait) => wait.code(),
            Self::InvalidResponse(_) => "invalid_portal_response",
            Self::NotAnAstroPortal => "not_astro_service",
            Self::IncompatibleApiVersion => "incompatible_api_version",
            Self::Refused(refusal) => refusal.static_code(),
            Self::SignInEnded(end) => end.code(),
            Self::Bridge(error) => error.code(),
            Self::Io { .. } => "io_error",
        }
    }

    /// The refusal, when the service answered one.
    #[must_use]
    pub fn refusal(&self) -> Option<&Refusal> {
        match self {
            Self::Refused(refusal) => Some(refusal),
            _ => None,
        }
    }

    /// Whether the service ended the sign-in.
    #[must_use]
    pub fn sign_in_end(&self) -> Option<SignInEnd> {
        match self {
            Self::SignInEnded(end) => Some(*end),
            Self::Bridge(GuiBridgeError::SignInEnded(end)) => Some(*end),
            _ => None,
        }
    }
}

impl From<HttpFailure> for SpaceError {
    fn from(failure: HttpFailure) -> Self {
        match failure {
            HttpFailure::Unreachable(detail) => Self::Transport(detail),
            HttpFailure::Wait(wait) => Self::Wait(wait),
        }
    }
}

/// "42 seconds", "15 minutes", "24 hours": a wait, in words. A lock
/// of the sign-in lasts 15 minutes, then 1 hour, then 24 hours: none is ever
/// said as a count of minutes it is not.
#[must_use]
pub fn wait_in_words(seconds: u64) -> String {
    if seconds < 90 {
        let seconds = seconds.max(1);
        return format!("{seconds} second{}", if seconds > 1 { "s" } else { "" });
    }
    let minutes = seconds.div_ceil(60);
    if minutes < 60 {
        return format!("{minutes} minutes");
    }
    let hours = seconds.div_ceil(3600);
    if hours < 48 {
        return format!("{hours} hour{}", if hours > 1 { "s" } else { "" });
    }
    format!("{} days", seconds.div_ceil(86_400))
}

/// "2026-09-29 21:35 UTC" from the service's own `retryAt`
/// (`2026-09-29T21:35:00.000Z`), or `None` when it is not that shape. Said in
/// UTC because the command line knows no other zone.
fn utc_text(at: &str) -> Option<String> {
    let (date, time) = at.split_once('T')?;
    let mut parts = date.split('-');
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
    let (hours, rest) = time.split_once(':')?;
    let minutes = rest.get(..2)?;
    let digits = |text: &str, len: usize| {
        text.len() == len && text.bytes().all(|byte| byte.is_ascii_digit())
    };
    (parts.next().is_none()
        && digits(year, 4)
        && digits(month, 2)
        && digits(day, 2)
        && digits(hours, 2)
        && digits(minutes, 2))
    .then(|| format!("{year}-{month}-{day} {hours}:{minutes} UTC"))
}

/// When the person may try again, as the service said it -- `Retry-After`
/// counted from now, and `retryAt` -- never a duration of Astro Sync's own.
fn when_text(refusal: &Refusal) -> String {
    let at = refusal.retry_at.as_deref().and_then(utc_text);
    match (&refusal.retry_after_seconds, at) {
        (Some(seconds), Some(at)) => {
            format!("in {} (from {at})", wait_in_words(*seconds))
        }
        (Some(seconds), None) => format!("in {}", wait_in_words(*seconds)),
        (None, Some(at)) => format!("from {at}"),
        (None, None) => "a little later".to_string(),
    }
}

/// What the command line says for each refusal of the contract: what
/// happened, in plain words, and the way forward.
fn refusal_sentence(refusal: &Refusal) -> String {
    match refusal.code.as_str() {
        "sign_in_code_incorrect" => {
            let left = match refusal.attempts_remaining {
                Some(1) => " You have 1 try left.".to_string(),
                Some(count) => format!(" You have {count} tries left."),
                None => String::new(),
            };
            format!(
                "This code does not match. Check the last message you received: only the last code sent is valid.{left}"
            )
        }
        "sign_in_code_expired" => "This code has expired. A code is good for 10 minutes, once only. Ask for a new one: it arrives in a few seconds.".to_string(),
        "sign_in_link_invalid" => "This link is no longer valid. Ask for a new code.".to_string(),
        "sign_in_locked" => format!(
            "Too many tries. For your security, codes for this address cannot be entered for now. You can ask for a new one {}.",
            when_text(refusal)
        ),
        "sign_in_rate_limited" => format!(
            "Too many codes requested, or too quickly. Try again {}.",
            when_text(refusal)
        ),
        "email_invalid" => "Enter a valid e-mail address.".to_string(),
        "email_delivery_failed" => "The message could not be sent. Nothing is counted: try again right away.".to_string(),
        // With `retryAt` the e-mail budget of the whole service is spent: the
        // sending of codes is suspended, for every address, until then.
        "sign_in_unavailable" if refusal.retry_at.is_some() || refusal.retry_after_seconds.is_some() => format!(
            "Sending codes is paused for the moment. You can try again {}.",
            when_text(refusal)
        ),
        "sign_in_unavailable" => "Signing in by e-mail is not available on this service right now.".to_string(),
        "space_unavailable" => "The space of this address is no longer available. Your code is right, but this space was closed. Use another e-mail address.".to_string(),
        // The address is proven and the code used: the space keeps its
        // ceiling of devices. Detach one, then ask for a new code.
        "device_limit_reached" => crate::refusal::plain_sentence("device_limit_reached")
            .unwrap_or_default()
            .to_string(),
        "service_starting" => "The service is starting. It may take a minute. Try again in a moment.".to_string(),
        // A private server signing in by e-mail code: the words of a portal
        // of one person.
        "not_owner" => "This portal belongs to someone else. Use the e-mail address chosen when it was set up.".to_string(),
        "not_initialized" => "This portal does not have an owner yet. Finish setting it up from a browser.".to_string(),
        "not_found" => "This service does not sign in by e-mail: it is a private server, which you connect with the eight-character code it shows.".to_string(),
        "invalid_request" => "The service did not understand the request.".to_string(),
        // A refusal this Astro Sync does not know (a newer service): the
        // generic sentence, never the raw code or the service's own English.
        _ => "The service refused the request. Try again later; if it keeps happening, update Astro Sync.".to_string(),
    }
}

/// The languages of the message the service sends (`fr`, `en`, `ja`); any
/// other locale is `en`, as the service reads an unknown one.
#[must_use]
pub fn message_language(locale: &str) -> &'static str {
    let base = locale
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    match base.as_str() {
        "fr" => "fr",
        "ja" => "ja",
        _ => "en",
    }
}

/// The characters the service refuses in an address and never removes, so
/// that an address cannot look like another (`v1.md`, « Hosted sign-in by
/// e-mail code »): the control characters U+0000-U+001F and U+007F, and the
/// formatting ones U+200B-U+200F, U+202A-U+202E, U+2060-U+206F and U+FEFF (a
/// zero-width space, a direction mark). White space is removed first, so the
/// tab and the line breaks of U+0000-U+001F never get here.
fn is_control_or_invisible(character: char) -> bool {
    matches!(
        character,
        '\u{0}'..='\u{1F}'
            | '\u{7F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FEFF}'
    )
}

/// The address as the service normalises it -- every white space removed,
/// letters lowercased, the final dot of the domain removed -- or
/// [`SpaceError::InvalidEmail`] when the result is not an address: one `@`, a
/// non-empty local part, a domain holding a dot with something on each side,
/// at most 254 characters, and no control or invisible character (the
/// service refuses such an address and never removes the character; a
/// zero-width space pasted along with an address is the usual cause).
/// Checking here keeps a typo from ever leaving the computer; the service
/// checks again, and puts the domain in punycode and the whole in its
/// composed form, which is why the address to use afterwards is the one it
/// answers ([`CodeRequested::email`]).
pub fn normalize_email(typed: &str) -> Result<String, SpaceError> {
    let email: String = typed
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>()
        .to_lowercase();
    if email.chars().any(is_control_or_invisible) {
        return Err(SpaceError::InvalidEmail);
    }
    if email.chars().count() > 254 || email.matches('@').count() != 1 {
        return Err(SpaceError::InvalidEmail);
    }
    let (local, domain) = email.split_once('@').ok_or(SpaceError::InvalidEmail)?;
    let domain = domain.strip_suffix('.').unwrap_or(domain);
    let dotted = domain
        .rsplit_once('.')
        .is_some_and(|(before, after)| !before.is_empty() && !after.is_empty());
    if local.is_empty() || !dotted {
        return Err(SpaceError::InvalidEmail);
    }
    Ok(format!("{local}@{domain}"))
}

/// The six digits of a code, with the spaces and hyphens typed between them
/// removed, or [`SpaceError::InvalidCode`].
pub fn normalize_code(typed: &str) -> Result<String, SpaceError> {
    let digits: String = typed
        .chars()
        .filter(|character| !character.is_whitespace() && *character != '-')
        .collect();
    if digits.len() == CODE_LENGTH && digits.chars().all(|character| character.is_ascii_digit()) {
        Ok(digits)
    } else {
        Err(SpaceError::InvalidCode)
    }
}

/// A name for a device: trimmed, not empty, at most
/// [`MAX_DEVICE_NAME_CHARS`] characters.
pub fn normalize_device_name(typed: &str) -> Result<String, SpaceError> {
    let name = typed.trim();
    if name.is_empty() {
        return Err(SpaceError::EmptyDeviceName);
    }
    Ok(name.chars().take(MAX_DEVICE_NAME_CHARS).collect())
}

/// The application's own name for the platform it runs on, as the contract
/// spells it (`sign-in.schema.json` `$defs/device.platform`).
#[must_use]
pub fn current_platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_e_mail_address_is_normalised_like_the_service_does() {
        assert_eq!(
            normalize_email("  Jean.Dupont+Astro@Exemple.FR ").unwrap(),
            "jean.dupont+astro@exemple.fr",
            "white space removed, letters lowercased, dots and + kept"
        );
        assert_eq!(
            normalize_email("vous @ exemple.fr").unwrap(),
            "vous@exemple.fr"
        );
        for refused in [
            "",
            "vous-exemple.fr",
            "you@",
            "@exemple.fr",
            "vous@exemple",
            "you@.com",
            "vous@exemple.",
            "a@b@exemple.fr",
        ] {
            assert!(
                matches!(normalize_email(refused), Err(SpaceError::InvalidEmail)),
                "{refused:?} is not an address"
            );
        }
        let long = format!("{}@exemple.fr", "a".repeat(250));
        assert!(matches!(
            normalize_email(&long),
            Err(SpaceError::InvalidEmail)
        ));
    }

    /// `400 email_invalid`: the service refuses an address with a control or
    /// invisible character and never removes it, so that an address cannot
    /// look like another. Astro Sync says so before anything leaves.
    #[test]
    fn an_address_with_an_invisible_or_control_character_is_refused_before_it_is_sent() {
        for (name, character) in [
            ("a bell (the recorded exchange)", '\u{7}'),
            ("a null", '\u{0}'),
            ("a delete", '\u{7F}'),
            ("a zero-width space", '\u{200B}'),
            ("a zero-width joiner", '\u{200D}'),
            ("a left-to-right mark", '\u{200E}'),
            ("a right-to-left override", '\u{202E}'),
            ("a word joiner", '\u{2060}'),
            ("a byte order mark", '\u{FEFF}'),
        ] {
            for typed in [
                format!("vous{character}@exemple.fr"),
                format!("vous@exemple{character}.fr"),
                format!("{character}vous@exemple.fr"),
            ] {
                assert!(
                    matches!(normalize_email(&typed), Err(SpaceError::InvalidEmail)),
                    "{name}: {typed:?}"
                );
            }
        }
        // White space is what the service removes: a tab, a line break, a
        // no-break and an ideographic space are not a refusal.
        assert_eq!(
            normalize_email("vous\t@exemple.fr\n").unwrap(),
            "vous@exemple.fr"
        );
        assert_eq!(
            normalize_email("vous\u{A0}@\u{3000}exemple.fr").unwrap(),
            "vous@exemple.fr"
        );
    }

    #[test]
    fn the_final_dot_of_the_domain_is_removed_like_the_service_does() {
        assert_eq!(
            normalize_email("you@Bücher.example.").unwrap(),
            "you@bücher.example"
        );
        assert!(matches!(
            normalize_email("vous@exemple.fr.."),
            Err(SpaceError::InvalidEmail)
        ));
        assert!(matches!(
            normalize_email("vous@exemple."),
            Err(SpaceError::InvalidEmail)
        ));
    }

    #[test]
    fn a_code_is_six_digits_with_spaces_and_hyphens_ignored() {
        assert_eq!(normalize_code("481207").unwrap(), "481207");
        assert_eq!(normalize_code(" 481 207 ").unwrap(), "481207");
        assert_eq!(normalize_code("481-207").unwrap(), "481207");
        for refused in ["", "48120", "4812071", "48120a", "４８１２０７"] {
            assert!(
                matches!(normalize_code(refused), Err(SpaceError::InvalidCode)),
                "{refused:?} is not a code"
            );
        }
    }

    #[test]
    fn a_device_name_is_trimmed_and_bounded() {
        assert_eq!(
            normalize_device_name("  Workshop desk ").unwrap(),
            "Workshop desk"
        );
        assert!(matches!(
            normalize_device_name("   "),
            Err(SpaceError::EmptyDeviceName)
        ));
        let long = "é".repeat(200);
        assert_eq!(
            normalize_device_name(&long).unwrap().chars().count(),
            MAX_DEVICE_NAME_CHARS
        );
    }

    #[test]
    fn the_message_language_is_one_the_service_knows() {
        assert_eq!(message_language("fr"), "fr");
        assert_eq!(message_language("fr-FR"), "fr");
        assert_eq!(message_language("ja_JP"), "ja");
        assert_eq!(message_language("EN-gb"), "en");
        assert_eq!(message_language("de"), "en");
        assert_eq!(message_language(""), "en");
    }

    #[test]
    fn a_secret_never_shows_itself() {
        let secret = Secret::new("QJR3D-0HcwGBdSXJ8IpOIuINTTIPAl2OHFyn1hIeyUQ");
        let shown = format!("{secret:?} {:?}", Some(&secret));
        assert!(!shown.contains("QJR3D"), "{shown}");
        assert_eq!(
            secret.expose(),
            "QJR3D-0HcwGBdSXJ8IpOIuINTTIPAl2OHFyn1hIeyUQ"
        );
    }

    #[test]
    fn a_sign_in_ended_by_the_service_has_a_code_and_a_sentence() {
        for end in [
            SignInEnd::DeviceDetached,
            SignInEnd::SpaceDeleted,
            SignInEnd::ConnectionReplaced,
        ] {
            assert_eq!(SignInEnd::from_code(end.code()), Some(end));
            assert!(end.plain_message().contains("are kept"));
        }
        assert_eq!(SignInEnd::from_code("authentication_required"), None);
        assert!(SignInEnd::DeviceDetached.forgets_the_space());
        assert!(SignInEnd::SpaceDeleted.forgets_the_space());
        assert!(!SignInEnd::ConnectionReplaced.forgets_the_space());
    }

    #[test]
    fn every_refusal_of_the_contract_has_a_code_and_a_plain_sentence() {
        let refusal = |code: &str, attempts: Option<u8>, seconds: Option<u64>| Refusal {
            status: 400,
            code: code.to_string(),
            message: "The service's own English sentence.".to_string(),
            attempts_remaining: attempts,
            retry_at: Some("2026-09-29T21:35:00.000Z".to_string()),
            retry_after_seconds: seconds,
        };
        for code in [
            "invalid_request",
            "email_invalid",
            "sign_in_code_incorrect",
            "sign_in_code_expired",
            "sign_in_link_invalid",
            "sign_in_locked",
            "sign_in_rate_limited",
            "email_delivery_failed",
            "sign_in_unavailable",
            "space_unavailable",
            "service_starting",
            "device_limit_reached",
            "not_owner",
            "not_initialized",
        ] {
            let error = SpaceError::Refused(refusal(code, Some(3), Some(900)));
            assert_eq!(error.code(), code);
            let text = error.to_string();
            assert!(
                !text.contains("English sentence") && !text.contains(code),
                "{code}: the sentence is plain, not the technical one: {text}"
            );
        }
        assert_eq!(
            SpaceError::Refused(refusal("not_found", None, None)).code(),
            "sign_in_wrong_service"
        );
        assert_eq!(
            SpaceError::Refused(refusal("something_new", None, None)).code(),
            "portal_refused"
        );
        // A refusal nobody knows yet is the generic sentence: never its raw
        // code, never the service's own English.
        let unknown = SpaceError::Refused(refusal("something_new", None, None)).to_string();
        assert!(
            !unknown.contains("something_new") && !unknown.contains("English sentence"),
            "{unknown}"
        );
        // The code is right but the space is gone: nothing to retry.
        let unavailable = SpaceError::Refused(refusal("space_unavailable", None, None)).to_string();
        assert!(
            unavailable.contains("is no longer available") && !unavailable.contains("Try again"),
            "{unavailable}"
        );
        let locked = SpaceError::Refused(refusal("sign_in_locked", None, Some(892))).to_string();
        assert!(locked.contains("in 15 minutes"), "{locked}");
        let one_left =
            SpaceError::Refused(refusal("sign_in_code_incorrect", Some(1), None)).to_string();
        assert!(one_left.contains("You have 1 try left."), "{one_left}");
        let four_left =
            SpaceError::Refused(refusal("sign_in_code_incorrect", Some(4), None)).to_string();
        assert!(four_left.contains("You have 4 tries left."), "{four_left}");
    }

    #[test]
    fn a_wait_is_said_in_seconds_then_in_minutes_then_in_hours() {
        assert_eq!(wait_in_words(42), "42 seconds");
        assert_eq!(wait_in_words(1), "1 second");
        assert_eq!(wait_in_words(0), "1 second");
        assert_eq!(wait_in_words(892), "15 minutes");
        assert_eq!(wait_in_words(1440), "24 minutes");
        assert_eq!(wait_in_words(3540), "59 minutes");
        // The three waits of a lock: 15 minutes, 1 hour, 24 hours.
        assert_eq!(wait_in_words(900), "15 minutes");
        assert_eq!(wait_in_words(3600), "1 hour");
        assert_eq!(wait_in_words(3599), "1 hour");
        assert_eq!(wait_in_words(86_400), "24 hours");
        assert_eq!(wait_in_words(200_000), "3 days");
    }

    /// The lock lasts 15 minutes, 1 hour or 24 hours: what the person reads
    /// is what the service said (`Retry-After`, `retryAt`), never a duration
    /// of Astro Sync's own.
    #[test]
    fn a_lock_says_the_wait_and_the_time_the_service_announced() {
        let locked = |seconds: Option<u64>, at: Option<&str>| {
            SpaceError::Refused(Refusal {
                status: 429,
                code: "sign_in_locked".to_string(),
                message: "The service's own English sentence.".to_string(),
                attempts_remaining: None,
                retry_at: at.map(str::to_string),
                retry_after_seconds: seconds,
            })
            .to_string()
        };
        let quarter = locked(Some(900), Some("2026-09-29T21:35:00.000Z"));
        assert!(quarter.contains("in 15 minutes"), "{quarter}");
        assert!(quarter.contains("2026-09-29 21:35 UTC"), "{quarter}");
        let hour = locked(Some(3600), Some("2026-09-29T22:20:00.000Z"));
        assert!(hour.contains("in 1 hour"), "{hour}");
        assert!(hour.contains("22:20 UTC"), "{hour}");
        let day = locked(Some(86_400), Some("2026-09-30T21:20:00.000Z"));
        assert!(day.contains("in 24 hours"), "{day}");
        assert!(day.contains("2026-09-30 21:20 UTC"), "{day}");
        // Only `retryAt`, or nothing: no invented duration.
        assert!(
            locked(None, Some("2026-09-29T21:35:00.000Z")).contains("from 2026-09-29 21:35 UTC")
        );
        let nothing = locked(None, None);
        assert!(nothing.contains("a little later"), "{nothing}");
        assert!(!nothing.contains("minutes"), "{nothing}");
        // A `retryAt` that is not a date is not repeated.
        assert!(locked(None, Some("soon")).contains("a little later"));
    }

    /// `503 sign_in_unavailable` with `retryAt`: the e-mail budget of the whole
    /// service is spent, the sending of codes is suspended until then. Without
    /// it the settings are incomplete: the message as it was.
    #[test]
    fn a_suspended_sending_of_codes_says_so_and_when_it_ends() {
        let unavailable = |seconds: Option<u64>, at: Option<&str>| {
            SpaceError::Refused(Refusal {
                status: 503,
                code: "sign_in_unavailable".to_string(),
                message: "The service's own English sentence.".to_string(),
                attempts_remaining: None,
                retry_at: at.map(str::to_string),
                retry_after_seconds: seconds,
            })
            .to_string()
        };
        let suspended = unavailable(Some(900), Some("2026-09-29T21:45:00.000Z"));
        assert!(
            suspended.contains("Sending codes is paused for the moment"),
            "{suspended}"
        );
        assert!(suspended.contains("in 15 minutes"), "{suspended}");
        assert!(suspended.contains("21:45 UTC"), "{suspended}");
        assert_eq!(
            unavailable(None, None),
            "Signing in by e-mail is not available on this service right now."
        );
    }

    #[test]
    fn a_device_ceiling_says_to_detach_a_device_and_that_the_code_was_used() {
        let refused = SpaceError::Refused(Refusal {
            status: 409,
            code: "device_limit_reached".to_string(),
            message: "The service's own English sentence.".to_string(),
            attempts_remaining: None,
            retry_at: None,
            retry_after_seconds: None,
        });
        assert_eq!(refused.code(), "device_limit_reached");
        let text = refused.to_string();
        assert!(text.contains("Detach a device"), "{text}");
        assert!(text.contains("“My devices”"), "{text}");
        assert!(text.contains("mobile application or on the web"), "{text}");
        assert!(text.contains("new code"), "{text}");
    }
}

//! Astro Sync reports its errors to the hosted service, anonymously, unless
//! the person turns it off in the settings. **A private server receives
//! nothing**: the report is queued and sent only while the service in force
//! is the hosted one.
//!
//! - **What a report holds**, and nothing else ([`ErrorReport`]): a random
//!   identifier of the report, the instant, the kind (`panic`: the program
//!   stopped on an unexpected failure; `ui`: the window met an uncaught
//!   error), the application's version, the system (`macos`, `windows` or
//!   `linux`), its version when readable, the processor family, a message
//!   (300 characters at most) and a stack (2 000 at most; the functions and
//!   the file names, never a path). The message and the stack are cleaned
//!   ([`scrub`]): a path, a name between quotes, an address, a coordinate, a
//!   secret never reach the queue.
//! - **What it never holds**: a file or folder path, the name of a night, a
//!   place or its coordinates, a frame, an e-mail address, the name of this
//!   computer or of its owner, the account, the space, the service's token.
//!   The service gets the report beside a random number drawn when Astro Sync
//!   is first used ([`diagnostic_id`]), which is *not* the installation
//!   identifier of the sign-in and links to no account.
//! - **The queue** ([`record`]) is a file of 20 reports at most (the oldest
//!   go first); a report like one already queued is not queued twice.
//! - **The sending** ([`flush`]) is one request of 10 reports at most,
//!   made at start and after a report, never in a loop and never waiting for
//!   the service to wake: what is not delivered stays queued for the next
//!   time. A report the service refuses for good is dropped.
//! - **The gates**, all of them needed, checked at the moment of queueing and
//!   again at the moment of sending: the switch is on the screen
//!   ([`ReportingPolicy::switch_on_screen`]), the person did not turn
//!   `report_errors` off ([`crate::local_settings::LocalSettings`]), and the
//!   service in force is the hosted one ([`crate::space::is_hosted_service`]).
//!   When one fails, the queue is emptied.

pub mod scrub;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use thiserror::Error;

use crate::gui_bridge;
use crate::local_settings;
use crate::space;

/// The route of the hosted service the reports go to.
pub const REPORT_PATH: &str = "/api/v1/diagnostics/errors";
/// The application's name in a batch.
pub const APPLICATION: &str = "astro-sync";
/// The reports the queue holds at most.
pub const QUEUE_LIMIT: usize = 20;
/// The reports of one request at most (the service's limit).
pub const BATCH_LIMIT: usize = 10;
/// The longest message and stack, in characters (the service's limits).
pub const MESSAGE_CHARS: usize = 300;
pub const STACK_CHARS: usize = 2000;
/// The most requests of one [`flush`].
const REQUESTS_PER_FLUSH: usize = 2;

/// Serialises the queue's reads and writes inside the process (a panic on
/// one thread while another flushes).
static QUEUE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Error)]
pub enum ErrorReportError {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{0}")]
    Other(String),
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> ErrorReportError + '_ {
    move |source| ErrorReportError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// The kind of an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The program stopped on an unexpected failure.
    Panic,
    /// The window met an uncaught error.
    Ui,
}

/// One report, exactly as it is queued and sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorReport {
    /// 32 hexadecimal characters, drawn at random: a batch sent again is
    /// recognised by it.
    pub id: String,
    /// When it happened, ISO 8601 UTC.
    pub at: String,
    pub kind: ErrorKind,
    pub app_version: String,
    /// `macos`, `windows` or `linux`.
    pub os: String,
    pub os_version: Option<String>,
    /// The processor family (`aarch64`, `x86_64`).
    pub arch: String,
    pub message: String,
    pub stack: Option<String>,
}

/// The system an error happened on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct System {
    pub os: String,
    pub os_version: Option<String>,
    pub arch: String,
}

impl System {
    /// The system this program runs on. The version is read from the files
    /// the system keeps for it (macOS, Linux), without starting a program;
    /// where it is not readable (Windows), none.
    #[must_use]
    pub fn current() -> Self {
        let os = space::current_platform().to_string();
        let os_version = match os.as_str() {
            "macos" => fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist")
                .ok()
                .and_then(|plist| plist_value(&plist, "ProductVersion")),
            "linux" => fs::read_to_string("/etc/os-release")
                .ok()
                .and_then(|release| os_release_value(&release, "VERSION_ID")),
            _ => None,
        }
        .filter(|version| is_version(version));
        Self {
            os,
            os_version,
            arch: std::env::consts::ARCH.to_string(),
        }
    }
}

/// `<key>name</key><string>value</string>` of a property list.
fn plist_value(plist: &str, name: &str) -> Option<String> {
    let after = plist.split(&format!("<key>{name}</key>")).nth(1)?;
    let start = after.find("<string>")? + "<string>".len();
    let end = after[start..].find("</string>")?;
    Some(after[start..start + end].trim().to_string())
}

/// `NAME=value` or `NAME="value"` of an `os-release` file.
fn os_release_value(release: &str, name: &str) -> Option<String> {
    release.lines().find_map(|line| {
        let value = line.strip_prefix(name)?.strip_prefix('=')?;
        Some(value.trim().trim_matches('"').to_string())
    })
}

/// What the service accepts as a version (`1.0.0`, `15.1`, `22.04`).
fn is_version(text: &str) -> bool {
    let mut chars = text.chars();
    text.chars().count() <= 40
        && chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '_' | '-'))
}

/// Where the reporting's files are: the same two settings files the rest of
/// the application reads, the queue, and the random identifier.
#[derive(Debug, Clone)]
pub struct ErrorReportFiles {
    /// `settings.json` (the service address; [`gui_bridge::GuiSettings`]).
    pub settings: PathBuf,
    /// `local-settings.json` (`report_errors`).
    pub local_settings: PathBuf,
    /// The queue of reports waiting to be sent.
    pub queue: PathBuf,
    /// The random identifier sent beside the reports.
    pub diagnostic_id: PathBuf,
}

/// Whether the build may report at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportingPolicy {
    /// The setting is on the screen. While it is not, nothing is queued and
    /// nothing is sent, whatever the settings file says; the application
    /// passes `true` in the change that puts the switch in the settings.
    pub switch_on_screen: bool,
}

/// Whether an error may be queued and sent now: the switch is on the screen,
/// the person did not turn it off, and the service in force is the hosted one.
/// Any file that cannot be read is a no.
#[must_use]
pub fn reporting_allowed(files: &ErrorReportFiles, policy: ReportingPolicy) -> bool {
    if !policy.switch_on_screen {
        return false;
    }
    let Ok(local) = local_settings::load(&files.local_settings) else {
        return false;
    };
    let Ok(settings) = gui_bridge::load_settings(&files.settings) else {
        return false;
    };
    local.report_errors && space::is_hosted_service(&space::service_url(&settings))
}

/// The random identifier sent beside the reports: 32 hexadecimal characters
/// drawn when it is first needed and kept in `path`. It is nobody's
/// identifier: not derived from the machine, the account or the sign-in's
/// installation identifier, and drawn again if the file is removed.
pub fn diagnostic_id(path: &Path) -> Result<String, ErrorReportError> {
    if let Ok(raw) = fs::read_to_string(path) {
        let existing = raw.trim();
        if is_hex_id(existing) {
            return Ok(existing.to_string());
        }
    }
    let identifier = random_hex_id()?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(io(parent))?;
    }
    fs::write(path, &identifier).map_err(io(path))?;
    Ok(identifier)
}

fn is_hex_id(text: &str) -> bool {
    text.len() == 32
        && text
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

fn random_hex_id() -> Result<String, ErrorReportError> {
    let mut bytes = [0_u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|error| ErrorReportError::Other(error.to_string()))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// An error as the program met it, before it is cleaned.
#[derive(Debug, Clone, Copy)]
pub struct RawError<'a> {
    pub kind: ErrorKind,
    pub message: &'a str,
    pub stack: Option<&'a str>,
}

/// The report of `raw`, cleaned and cut. `now` is an ISO 8601 UTC instant.
pub fn build_report(
    raw: RawError<'_>,
    app_version: &str,
    system: &System,
    now: &str,
) -> Result<ErrorReport, ErrorReportError> {
    Ok(ErrorReport {
        id: random_hex_id()?,
        at: now.to_string(),
        kind: raw.kind,
        app_version: if is_version(app_version) {
            app_version.to_string()
        } else {
            "unknown".to_string()
        },
        os: system.os.clone(),
        os_version: system.os_version.clone(),
        arch: system.arch.clone(),
        message: scrub::clean(raw.message, MESSAGE_CHARS)
            .unwrap_or_else(|| "[no message]".to_string()),
        stack: raw
            .stack
            .and_then(|stack| scrub::clean_stack(stack, STACK_CHARS)),
    })
}

/// What two reports of the same failure share, so that the queue holds one.
fn fingerprint(report: &ErrorReport) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{:?}", report.kind));
    hasher.update(&report.message);
    hasher.update(
        report
            .stack
            .as_deref()
            .and_then(|s| s.lines().next())
            .unwrap_or(""),
    );
    format!("{:x}", hasher.finalize())
}

/// Whether a report is clean as it stands -- the net under the cleaning, for
/// a queue file written by another version or edited by hand.
fn is_sendable(report: &ErrorReport) -> bool {
    is_hex_id(&report.id)
        && !scrub::still_unsafe(&report.message)
        && report
            .stack
            .as_deref()
            .is_none_or(|stack| !scrub::still_unsafe(stack))
        && report.os_version.as_deref().is_none_or(is_version)
        && report.message.chars().count() <= MESSAGE_CHARS
        && report
            .stack
            .as_deref()
            .is_none_or(|stack| stack.chars().count() <= STACK_CHARS)
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Queue {
    #[serde(default)]
    reports: Vec<ErrorReport>,
}

/// The queue; a missing, unreadable or invalid file is an empty queue (a
/// report is not worth a failure). Reports that are not clean are left out.
fn load_queue(path: &Path) -> Queue {
    let mut queue: Queue = fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    queue.reports.retain(is_sendable);
    queue
}

fn save_queue(path: &Path, queue: &Queue) -> Result<(), ErrorReportError> {
    if queue.reports.is_empty() {
        return match fs::remove_file(path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(io(path)(error)),
            _ => Ok(()),
        };
    }
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(io(parent))?;
    }
    let json = serde_json::to_string(queue).expect("the queue serializes");
    let side = path.with_extension("json.part");
    fs::write(&side, json).map_err(io(&side))?;
    fs::rename(&side, path).map_err(io(path))
}

/// What [`record`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recorded {
    /// The reporting is not allowed: nothing was kept.
    NotAllowed,
    /// The same failure is already waiting.
    AlreadyQueued,
    /// Queued; how many reports wait now.
    Queued(usize),
}

/// Queues the report of `raw` when the reporting is allowed. Never sends.
pub fn record(
    files: &ErrorReportFiles,
    policy: ReportingPolicy,
    app_version: &str,
    raw: RawError<'_>,
) -> Result<Recorded, ErrorReportError> {
    if !reporting_allowed(files, policy) {
        return Ok(Recorded::NotAllowed);
    }
    let report = build_report(
        raw,
        app_version,
        &System::current(),
        &crate::now_iso8601_utc(),
    )?;
    let _guard = QUEUE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut queue = load_queue(&files.queue);
    let print = fingerprint(&report);
    if queue
        .reports
        .iter()
        .any(|queued| fingerprint(queued) == print)
    {
        return Ok(Recorded::AlreadyQueued);
    }
    queue.reports.push(report);
    while queue.reports.len() > QUEUE_LIMIT {
        queue.reports.remove(0);
    }
    save_queue(&files.queue, &queue)?;
    Ok(Recorded::Queued(queue.reports.len()))
}

/// How a request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// The service kept the reports (or already had them).
    Accepted,
    /// The service refused them for good: they are dropped.
    Refused,
    /// Not now (no network, a service that sleeps or fails, a limit, a token
    /// that is not accepted yet): they stay queued.
    Retry,
}

/// Sends one batch to the service.
pub trait ErrorReportTransport {
    fn deliver(&self, service_url: &str, token: &str, body: &serde_json::Value) -> Delivery;
}

/// The body of a request: the application, the random identifier, the reports.
#[must_use]
pub fn request_body(diagnostic_id: &str, reports: &[ErrorReport]) -> serde_json::Value {
    serde_json::json!({
        "application": APPLICATION,
        "diagnosticId": diagnostic_id,
        "reports": reports,
    })
}

/// What [`flush`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Flushed {
    /// The reporting is not allowed: the queue was emptied.
    NotAllowed,
    /// Nothing waits.
    Nothing,
    /// Reports wait and there is no token to send them with.
    NoToken,
    /// Sent or dropped; how many left the queue, how many still wait.
    Done { left: usize, waiting: usize },
}

/// Sends what waits, at most [`REQUESTS_PER_FLUSH`] requests of
/// [`BATCH_LIMIT`] reports. `token` is the device token of the hosted
/// service, if this computer is signed in.
pub fn flush(
    files: &ErrorReportFiles,
    policy: ReportingPolicy,
    token: Option<&str>,
    transport: &dyn ErrorReportTransport,
) -> Result<Flushed, ErrorReportError> {
    let _guard = QUEUE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !reporting_allowed(files, policy) {
        save_queue(&files.queue, &Queue::default())?;
        return Ok(Flushed::NotAllowed);
    }
    let mut queue = load_queue(&files.queue);
    if queue.reports.is_empty() {
        save_queue(&files.queue, &queue)?;
        return Ok(Flushed::Nothing);
    }
    let Some(token) = token.filter(|token| !token.is_empty()) else {
        return Ok(Flushed::NoToken);
    };
    let settings = gui_bridge::load_settings(&files.settings)
        .map_err(|error| ErrorReportError::Other(error.to_string()))?;
    let service = space::service_url(&settings);
    let id = diagnostic_id(&files.diagnostic_id)?;
    let mut left = 0;
    for _ in 0..REQUESTS_PER_FLUSH {
        if queue.reports.is_empty() {
            break;
        }
        let count = queue.reports.len().min(BATCH_LIMIT);
        let body = request_body(&id, &queue.reports[..count]);
        match transport.deliver(&service, token, &body) {
            Delivery::Accepted | Delivery::Refused => {
                queue.reports.drain(..count);
                left += count;
            }
            Delivery::Retry => break,
        }
    }
    save_queue(&files.queue, &queue)?;
    Ok(Flushed::Done {
        left,
        waiting: queue.reports.len(),
    })
}

/// The service, over HTTPS: one request, ten seconds at most, no redirect, no
/// waiting for a sleeping service.
pub struct HttpErrorReportTransport {
    client: reqwest::blocking::Client,
}

impl HttpErrorReportTransport {
    pub fn new() -> Result<Self, ErrorReportError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            // Sent to the portal only, like the rest of the application's
            // requests to it (see `client_identity`).
            .default_headers(crate::client_identity::headers())
            .build()
            .map_err(|error| ErrorReportError::Other(error.to_string()))?;
        Ok(Self { client })
    }
}

impl ErrorReportTransport for HttpErrorReportTransport {
    fn deliver(&self, service_url: &str, token: &str, body: &serde_json::Value) -> Delivery {
        let url = format!("{}{REPORT_PATH}", service_url.trim_end_matches('/'));
        let answer = self.client.post(url).bearer_auth(token).json(body).send();
        match answer.map(|answer| answer.status().as_u16()) {
            Ok(200..=299) => Delivery::Accepted,
            // A token not accepted (yet), a limit, a request that was too
            // slow, a service that fails: later.
            Ok(401 | 403 | 408 | 429 | 500..) | Err(_) => Delivery::Retry,
            Ok(_) => Delivery::Refused,
        }
    }
}

#[cfg(test)]
mod tests;

//! The resumable-upload HTTP client, against
//! `contracts/schemas/api/uploads.md`.
//!
//! [`UploadTransport`] is the injectable seam: production code talks to a
//! real portal through [`HttpUploadTransport`] (`reqwest`, blocking,
//! `rustls`); tests supply a fake that scripts exactly the responses a
//! scenario needs (a `409` offset mismatch, a `422` hash mismatch, a `413`
//! refusal, a dropped connection mid-transfer). [`upload_entry`] is the
//! orchestration every caller (CLI, GUI) drives one [`QueueEntry`] through:
//! it never assumes a fresh session starts at offset `0` -- it always reads
//! the server's own answer -- and it persists progress to the
//! [`UploadQueue`] after every accepted chunk, so a crash mid-transfer
//! leaves exactly enough on disk to resume.
//!
//! **What a refusal does to a file.** Every failure has a [`Verdict`], and
//! the verdict -- never the wording of the service -- decides what happens,
//! so that no refusal is ever sent again in a loop:
//!
//! - the service **refuses the file for good** (`415`
//!   `capture_content_refused`, `413` `file_too_large`, `413`
//!   `payload_too_large`, the ceilings of a session, and any other `4xx`
//!   this Astro Sync does not know): the file leaves the queue as
//!   [`QueueState::Refused`], with the code its sentence is looked up by;
//! - **the person has to act** (`409` `capture_limit_reached`, `409`
//!   `storage_quota_reached`, `403` `entitlement_read_only`): the file is
//!   [`QueueState::Held`], its session and offset kept, and the run stops --
//!   everything else would be refused the same way;
//! - the failure is **not the file's fault and will pass** (`409`
//!   `upload_sessions_limit_reached`, `429`, any `5xx`, a service that is
//!   still starting, a network that is down): the file stays queued and the
//!   run tries again later, by itself, waiting longer each time
//!   ([`crate::gui_bridge::drive_queue`]).
//!
//! A `409` or a `422` at finalize is read as what it says
//! (`upload_incomplete`, `upload_hash_mismatch`) and nothing else: any other
//! one is a refusal like the rest.

use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

use crate::metadata::{self, Proposals, SkippedField};
use crate::refusal;
use crate::service_wait::{
    HttpAnswer, HttpFailure, PatientHttp, RequestFactory, ServiceWaitError, WaitHooks, WakePolicy,
};
use crate::space::SignInEnd;
use crate::upload_queue::{QueueEntry, ResultKind, UploadQueue, UploadQueueError};

/// The `uploadMetadata` shape `POST /v1/uploads`'s JSON create step sends
/// (`capture.schema.json`'s `uploadMetadata`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub target_ids: Vec<String>,
    pub original_name: String,
    pub sha256: String,
    pub byte_size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    /// The capture this image is a variant of -- never its main image by
    /// itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_id: Option<String>,
    /// `astro-sync` or `stacking` (never `web` from a device).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// `progression` for a progression image, the history of
    /// `capture_id` (never a variant, never counted as one).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// The light frames a progression image stacks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progression_frames: Option<u64>,
    /// The multi-night plan of an image of the plan (`role: "plan"`),
    /// with the nights it combines, its total by filter, its render.
    /// Absent on every other image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub combined_night_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub totals_by_filter: Option<Vec<crate::upload_queue::PlanFilterTotal>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_render: Option<String>,
    /// The producer the proposals below come from
    /// ([`crate::metadata::SOURCE`]); written only when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_source: Option<String>,
    /// What Astro Sync proposes (`acquisition`, `calibration`, `sensor`,
    /// `processing`, `variantGeneratedTitle`, `variantGeneratedDescription`):
    /// never over a value the owner typed, the service says what it left.
    #[serde(flatten)]
    pub proposals: Proposals,
}

/// The prefix of [`crate::upload_queue::EntryTarget::variant_of`] when the
/// capture an image is a variant of is the one another image of the same
/// sending creates (`sha256:<digest of that image>`): resolved at the
/// upload, once that image is sent.
pub const VARIANT_OF_SHA256: &str = "sha256:";

/// Which session/plan/targets an upload should be attributed to. Kept
/// separate from [`QueueEntry`] (which only knows the file, not what it is
/// *of*) so the same queued file could in principle be re-targeted before
/// its first byte is sent.4 always fills this from the CLI's own flags.
#[derive(Debug, Clone, Default)]
pub struct UploadTarget {
    pub session_id: Option<String>,
    pub plan_id: Option<String>,
    pub target_ids: Vec<String>,
}

/// The `uploadSession` envelope shared by create/`PUT`/`GET`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadSession {
    pub upload_id: String,
    pub offset: u64,
    pub byte_size: u64,
    pub chunk_size: u64,
    pub completed: bool,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub capture_id: Option<String>,
}

/// `POST /v1/uploads/{id}/finalize`'s success body. The nested `capture`
/// object is intentionally not modelled here -- nothing in this crate needs
/// more than the `captureId` it already carries directly.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizeResponse {
    pub capture_id: String,
    pub deduplicated: bool,
    /// The proposals the service left (`edited`, `deleted`).
    #[serde(default)]
    pub skipped: Vec<SkippedField>,
    /// The image a variant upload added (or found), and the capture's main
    /// image: what a proposal after the fact names.
    #[serde(default)]
    pub variant: Option<ImageRef>,
    #[serde(default)]
    pub capture: Option<CaptureRef>,
    /// True when this image **replaced** the image of the capture it belongs
    /// to (one capture per session and target, a space without variants):
    /// the Journal's `replaced`. Absent = false.
    #[serde(default)]
    pub replaced: bool,
}

/// The `id` of the `variant` of a finalize answer.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ImageRef {
    pub id: String,
}

/// The two fields of the finalize answer's `capture` this crate reads.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureRef {
    #[serde(default)]
    pub main_image_id: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
}

impl FinalizeResponse {
    /// The id of the image `sha256` sent: the variant added, else the
    /// capture's main image when it has these bytes.
    fn image_id_of(&self, sha256: &str) -> Option<String> {
        self.variant.as_ref().map(|v| v.id.clone()).or_else(|| {
            let capture = self.capture.as_ref()?;
            capture
                .sha256
                .as_deref()
                .is_some_and(|s| s.eq_ignore_ascii_case(sha256))
                .then(|| capture.main_image_id.clone())
                .flatten()
        })
    }
}

#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    error: ErrorDetail,
    #[serde(default)]
    offset: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ErrorDetail {
    code: String,
    message: String,
}

/// `PUT`'s two outcomes: normal advancement, or the offset the server
/// actually holds when the declared one disagreed (`409
/// upload_offset_mismatch`) -- not an error, since the contract expects a
/// client to simply resume from the number given.
#[derive(Debug, Clone)]
pub enum PutOutcome {
    Advanced(UploadSession),
    OffsetMismatch { offset: u64 },
}

/// Finalize's four outcomes: done, not all bytes arrived yet (`409
/// upload_incomplete`), the assembled digest disagreed (`422
/// upload_hash_mismatch`, session reset to offset `0` server-side), or the
/// capture the image is added to left the space after the create (`409
/// upload_capture_gone`: a new create says where it goes now, the bytes
/// received kept).
#[derive(Debug, Clone)]
pub enum FinalizeOutcome {
    Completed(FinalizeResponse),
    Incomplete { offset: u64 },
    HashMismatch,
    CaptureGone,
}

/// What a failure does to the file and to the run that met it. It is decided
/// by the status and the code of the answer -- never by the wording of the
/// service -- and it is the reason no refusal is sent again in a loop. See
/// the module documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The service refuses this file for good: [`QueueState::Refused`], with
    /// the code of the refusal. The run goes on with the next file.
    Refused,
    /// This transfer failed for good and is not retried on its own:
    /// [`QueueState::Failed`]. The run goes on with the next file.
    Failed,
    /// The person has to act before anything more can be sent (storage full,
    /// subscription ended): [`QueueState::Held`], and the run stops.
    NeedsPerson,
    /// Not the file's fault, and it will pass (a network that is down, a
    /// service that is starting or failing, too many uploads open at once):
    /// the file stays queued and the run tries again later, by itself,
    /// waiting longer each time.
    RetryLater,
    /// The person cancelled the wait: nothing failed.
    Cancelled,
    /// The service ended the sign-in: nothing is wrong with the file, which
    /// stays queued for the next sign-in.
    SignedOut,
    /// The local queue database failed: the run stops, the file is as it
    /// was.
    Local,
}

/// The code of a file the queue names but that is no longer where it was: the
/// interface says which file and which folder, and offers « Réessayer ».
pub const SOURCE_FILE_MISSING: &str = "source_file_missing";

/// Every way a resumable-upload call can fail. [`UploadError::verdict`] says
/// what each one does to the file: only a `Refused` or `Failed` verdict marks
/// the queue entry -- see [`UploadError::is_terminal`].
#[derive(Debug, Error)]
pub enum UploadError {
    /// The service could not be reached at all (name not found, connection
    /// refused, TLS failure, no network): immediate, never waited on.
    #[error("network error talking to the service: {0}")]
    Transport(String),
    /// The service was asleep or starting and the wait ended without an
    /// answer (gave up, or the person cancelled): see [`ServiceWaitError`].
    #[error(transparent)]
    Wait(#[from] ServiceWaitError),
    #[error("unreadable response from the service: {0}")]
    InvalidResponse(String),
    #[error("upload session {0:?} was not found (expired or unknown)")]
    SessionNotFound(String),
    /// The service answered with a status and an error the protocol does not
    /// read itself. `status` and `code` decide the [`Verdict`];
    /// `retry_after_seconds` is the `Retry-After` header, when the service
    /// sent one.
    #[error("refused by the service ({status} {code}): {message}")]
    Refused {
        status: u16,
        code: String,
        message: String,
        retry_after_seconds: Option<u64>,
    },
    #[error("could not read the source file {path}: {source}")]
    SourceFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("upload queue error: {0}")]
    Queue(#[from] UploadQueueError),
    #[error("giving up after {attempts} attempt(s): {reason}")]
    GaveUp { attempts: u32, reason: String },
    /// The service answered `401` `device_detached` or `space_deleted`: this
    /// computer is no longer attached to a space. **Not terminal for the
    /// file**: nothing is wrong with it, so it stays in the queue exactly as
    /// it was and goes on when the person signs in again -- a pending upload
    /// is never lost to it.
    #[error("{}", .0.plain_message())]
    SignInEnded(SignInEnd),
}

impl UploadError {
    /// A refusal, as the service sent it.
    #[must_use]
    pub fn refusal(status: u16, code: &str, message: &str) -> Self {
        UploadError::Refused {
            status,
            code: code.to_string(),
            message: message.to_string(),
            retry_after_seconds: None,
        }
    }

    /// What this failure does to the file and to the run. See [`Verdict`].
    #[must_use]
    pub fn verdict(&self) -> Verdict {
        match self {
            UploadError::Transport(_) => Verdict::RetryLater,
            UploadError::Wait(ServiceWaitError::Cancelled) => Verdict::Cancelled,
            UploadError::Wait(_) => Verdict::RetryLater,
            UploadError::InvalidResponse(_)
            | UploadError::SessionNotFound(_)
            | UploadError::SourceFile { .. }
            | UploadError::GaveUp { .. } => Verdict::Failed,
            UploadError::Queue(_) => Verdict::Local,
            UploadError::SignInEnded(_) => Verdict::SignedOut,
            UploadError::Refused { status, code, .. } => refusal_verdict(*status, code),
        }
    }

    /// Whether the service did not answer at all (the network is down, or it
    /// was still starting when the whole wait ended), as opposed to
    /// answering something that will pass.
    #[must_use]
    pub fn is_silence(&self) -> bool {
        matches!(self, UploadError::Transport(_) | UploadError::Wait(_))
    }

    /// Whether this failure ends the file's turn for good (marks the queue
    /// entry `failed` or `refused`) rather than leaving it for a later
    /// attempt (a network that is down, a service that fails, a space that
    /// keeps too many uploads open), holding it for the person (storage
    /// full), or leaving it for the next sign-in.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self.verdict(), Verdict::Refused | Verdict::Failed)
    }

    /// The stable, machine-readable code a front end looks up in its own
    /// dictionaries (`errors.<code>`), so a person never reads a raw
    /// technical message for the failures they can act on. A refusal the
    /// contract names has its own code; any other `4xx` is
    /// `upload_refused`, any other `5xx` `service_error`.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            UploadError::Transport(_) => "network_error",
            UploadError::Wait(wait) => wait.code(),
            UploadError::SignInEnded(end) => end.code(),
            UploadError::Refused { status, code, .. } => {
                refusal::code_for_answer(*status, code).unwrap_or("upload_error")
            }
            UploadError::SourceFile { source, .. }
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                SOURCE_FILE_MISSING
            }
            _ => "upload_error",
        }
    }

    /// How long the service asked to wait (`Retry-After`), when it did.
    #[must_use]
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            UploadError::Refused {
                retry_after_seconds: Some(seconds),
                ..
            } => Some(Duration::from_secs(*seconds)),
            _ => None,
        }
    }

    /// The same failure in plain words, for the command line: what happened and
    /// what can be done, never a status, a code or the service's English.
    #[must_use]
    pub fn plain_message(&self) -> String {
        match self {
            UploadError::Transport(_) => "Could not reach the service (address not found, connection refused or no network).".to_string(),
            UploadError::Wait(wait) => wait.to_string(),
            UploadError::InvalidResponse(_) => "The service answered in an unexpected way: update Astro Sync, or try again later.".to_string(),
            UploadError::SessionNotFound(_) => "The service no longer knows this upload (it expires after 72 hours). Add the file again to send it again.".to_string(),
            UploadError::SourceFile { path, source }
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                format!(
                    "File not found: {} (it was in {}).",
                    path.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    path.parent().map(|dir| dir.display().to_string()).unwrap_or_default(),
                )
            }
            UploadError::SourceFile { path, .. } => format!(
                "The file {} could not be read: check that it is still there.",
                path.display()
            ),
            UploadError::Queue(error) => {
                format!("The local upload queue reported an error: {error}")
            }
            UploadError::GaveUp { attempts, reason } => {
                format!("Sending was given up after {attempts} attempt(s): {reason}.")
            }
            UploadError::SignInEnded(end) => end.plain_message().to_string(),
            UploadError::Refused { .. } => refusal::plain_sentence(self.code())
                .unwrap_or("The service refused the request.")
                .to_string(),
        }
    }
}

/// What a refusal of `status` and `code` does. The codes the contract names
/// come first; any other `4xx` is a refusal for good, any `5xx` a failure of
/// the service that will pass.
fn refusal_verdict(status: u16, code: &str) -> Verdict {
    match (status, code) {
        // Nothing wrong with the file: the space has no room (a full storage,
        // or the number of captures its profile keeps -- the fourth of a free
        // space), or no right to send, until the person acts. The session the
        // service kept is kept.
        // A variant past `capture.variant_limit` is the same: kept in the
        // queue with its message, resumable once the person acted.
        (
            _,
            "capture_limit_reached"
            | "capture_variant_limit_reached"
            | "storage_quota_reached"
            | "entitlement_read_only"
            | "variant_capture_pending",
        ) => Verdict::NeedsPerson,
        // The space already keeps its ceiling of unfinished uploads open;
        // one of them finishing (or expiring) makes room.
        (_, "upload_sessions_limit_reached") => Verdict::RetryLater,
        (408 | 429, _) | (500..=599, _) => Verdict::RetryLater,
        (400..=499, _) => Verdict::Refused,
        _ => Verdict::Failed,
    }
}

/// An answer that is not a success, read once: the status, the code and the
/// sentence of its envelope (`invalid_response` and the raw text when it has
/// none), the offset an upload conflict carries, and `Retry-After`.
struct Answered {
    status: u16,
    code: String,
    message: String,
    offset: Option<u64>,
    retry_after_seconds: Option<u64>,
}

impl Answered {
    fn of(answer: &HttpAnswer) -> Self {
        let text = answer.text();
        let (code, message, offset) = match serde_json::from_str::<ErrorEnvelope>(&text) {
            Ok(envelope) => (envelope.error.code, envelope.error.message, envelope.offset),
            Err(_) => ("invalid_response".to_string(), text, None),
        };
        Self {
            status: answer.status,
            code,
            message,
            offset,
            retry_after_seconds: answer.retry_after_seconds,
        }
    }

    /// The service no longer knows this upload: its 72 hours are over, or it
    /// never existed. (A bare `404` without an envelope is the same; a `404`
    /// of any other code -- `plan_not_found`, `target_not_found` -- is a
    /// refusal of what the upload is attributed to, not a lost upload.)
    fn upload_is_gone(&self) -> bool {
        self.status == 404 && (self.code == "upload_not_found" || self.code == "invalid_response")
    }

    /// The failure this answer is: the end of the sign-in when a `401` says
    /// so, else a refusal whose verdict the status and the code decide.
    fn into_error(self) -> UploadError {
        if self.status == 401 {
            if let Some(end) = SignInEnd::from_code(&self.code) {
                return UploadError::SignInEnded(end);
            }
            // "No credential, or a revoked one": the device was detached.
            if self.code == "authentication_required" {
                return UploadError::SignInEnded(SignInEnd::DeviceDetached);
            }
        }
        UploadError::Refused {
            status: self.status,
            code: self.code,
            message: self.message,
            retry_after_seconds: self.retry_after_seconds,
        }
    }
}

impl From<HttpFailure> for UploadError {
    fn from(failure: HttpFailure) -> Self {
        match failure {
            HttpFailure::Unreachable(detail) => UploadError::Transport(detail),
            HttpFailure::Wait(wait) => UploadError::Wait(wait),
        }
    }
}

/// What a completed transfer produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadOutcome {
    pub capture_id: String,
    /// True when the bytes were already a finalized capture (or an
    /// in-progress session already covered them) -- the re-upload-is-a-
    /// no-op case the contract's idempotency rules guarantee.
    pub deduplicated: bool,
    /// What the service left of the proposals, the owner's values
    /// ([`metadata::skipped_sentence`] says it).
    pub skipped: Vec<SkippedField>,
    /// The code of a refusal of the proposals (`web_session_required`,
    /// `invalid_metadata`...): the image is sent all the same.
    pub proposals_refused: Option<String>,
    /// The image replaced the one the capture already had (the phone's photo
    /// of the session): the queue's row says it
    /// ([`crate::upload_queue::REPLACED_NOTE`]).
    pub replaced: bool,
}

/// One queue entry's [`upload_entry`] result, shaped for machine
/// consumption (CLI `upload --json`, and the Tauri GUI). `UploadOutcome`
/// and [`UploadError`] are `upload_entry`'s native `Result<T, E>` -- neither
/// derives `Serialize` (an error's `Display` string is all a machine caller
/// needs), so this flattens either branch into one stable, serializable
/// shape. Lives here rather than in the CLI so no caller has to reimplement
/// this flattening to get the exact JSON the CLI prints. `Deserialize` is
/// derived too so a test (or the GUI) can read the CLI's `upload --json`
/// array straight back into this type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadEntryResult {
    pub sha256: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deduplicated: Option<bool>,
    /// What went wrong, in plain words (never a status or the
    /// service's own English).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The stable code of the failure (`capture_content_refused`,
    /// `storage_quota_reached`, `network_error`, ...): what a script tests,
    /// where `error` is for a person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// What the service left of the capture metadata proposed with this
    /// file (the owner's own values).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<SkippedField>,
    /// The sentence that says it, in plain words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The image replaced the one its capture had (the phone's photo of the
    /// session); not written when it did not.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub replaced: bool,
}

impl UploadEntryResult {
    /// Builds the result for one entry from `upload_entry`'s own
    /// `Result<UploadOutcome, UploadError>`, identified by its content hash
    /// (queue entries are keyed by `sha256`, not by name).
    #[must_use]
    pub fn from_outcome(
        sha256: impl Into<String>,
        outcome: &Result<UploadOutcome, UploadError>,
    ) -> Self {
        match outcome {
            Ok(result) => {
                let mut sent = Self::sent(sha256, &result.capture_id, result.deduplicated);
                sent.note = metadata::skipped_sentence(&result.skipped);
                sent.skipped = result.skipped.clone();
                sent.replaced = result.replaced;
                sent
            }
            Err(error) => Self::failed(sha256, error.code(), error.plain_message()),
        }
    }

    /// A file that was sent: its capture, and whether the service already had
    /// the bytes.
    #[must_use]
    pub fn sent(
        sha256: impl Into<String>,
        capture_id: impl Into<String>,
        deduplicated: bool,
    ) -> Self {
        Self {
            sha256: sha256.into(),
            ok: true,
            capture_id: Some(capture_id.into()),
            deduplicated: Some(deduplicated),
            error: None,
            code: None,
            skipped: Vec::new(),
            note: None,
            replaced: false,
        }
    }

    /// A file that was not sent, with the stable `code` of why and the
    /// sentence.
    #[must_use]
    pub fn failed(
        sha256: impl Into<String>,
        code: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        Self {
            sha256: sha256.into(),
            ok: false,
            capture_id: None,
            deduplicated: None,
            error: Some(error.into()),
            code: Some(code.into()),
            skipped: Vec::new(),
            note: None,
            replaced: false,
        }
    }
}

/// The four HTTP calls the resumable protocol needs, kept behind a trait so
/// tests can script exact server behavior without a real portal.
pub trait UploadTransport {
    fn create(&self, token: &str, metadata: &UploadMetadata) -> Result<UploadSession, UploadError>;
    fn status(&self, token: &str, upload_id: &str) -> Result<UploadSession, UploadError>;
    fn put_chunk(
        &self,
        token: &str,
        upload_id: &str,
        offset: u64,
        chunk: &[u8],
    ) -> Result<PutOutcome, UploadError>;
    fn finalize(&self, token: &str, upload_id: &str) -> Result<FinalizeOutcome, UploadError>;

    /// Proposes the capture's metadata for a capture that exists
    /// (`PATCH /v1/captures/{id}`
    /// `{ "proposed": { "source": "astro-sync", … } }`), answering what the
    /// service left. A transport that knows no proposals proposes nothing.
    fn propose_for_capture(
        &self,
        _token: &str,
        _capture_id: &str,
        _proposals: &Proposals,
    ) -> Result<Vec<SkippedField>, UploadError> {
        Ok(Vec::new())
    }

    /// Proposes the title and the description of an image that exists
    /// (`PATCH /v1/captures/{id}/variants/{imageId}`
    /// `{ title, description, source }`), answering what the service
    /// left.
    fn propose_for_image(
        &self,
        _token: &str,
        _capture_id: &str,
        _image_id: &str,
        _proposals: &Proposals,
    ) -> Result<Vec<SkippedField>, UploadError> {
        Ok(Vec::new())
    }
}

/// The real transport: `reqwest` blocking client over `rustls`, against
/// `{base_url}/api/v1/uploads...` (the route handlers live under
/// `src/app/api/v1/uploads/`; `api/uploads.md`'s own paths omit the leading
/// `/api` the way the contract documents the surface abstractly).
///
/// Every call here is safe to send more than once, which is what lets the
/// patient client ([`PatientHttp`]) retry them while the hosted service
/// wakes up from sleep -- each for a reason the contract states
/// (`uploads.md`), not a hope:
///
/// - `create`: idempotent on `(tenant, sha256)` -- a second create answers
///   the same session at its current offset ("Idempotency -- two levels").
/// - `status`: a read.
/// - `put_chunk`: the `Upload-Offset` header must equal the server's own
///   durable offset; a replay of a chunk that already landed is a `409` with
///   the real offset, which [`upload_entry`] resumes from -- never a
///   second copy of the bytes.
/// - `finalize`: "Idempotent. Calling finalize again on an already-completed
///   session replays this exact result -- the same `captureId`."
pub struct HttpUploadTransport {
    http: PatientHttp,
    base_url: String,
}

impl HttpUploadTransport {
    pub fn new(base_url: impl Into<String>) -> Result<Self, UploadError> {
        Self::with_options(base_url, WakePolicy::default(), WaitHooks::silent())
    }

    /// Like [`HttpUploadTransport::new`], with an explicit waiting policy
    /// (tests shorten it) and the hooks through which a caller shows the
    /// "the service is starting" state and can cancel the wait.
    pub fn with_options(
        base_url: impl Into<String>,
        policy: WakePolicy,
        hooks: WaitHooks,
    ) -> Result<Self, UploadError> {
        let http = PatientHttp::new(policy, hooks).map_err(UploadError::Transport)?;
        let base_url = base_url.into();
        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

fn parse_session(answer: &HttpAnswer) -> Result<UploadSession, UploadError> {
    serde_json::from_slice::<UploadSession>(&answer.body)
        .map_err(|error| UploadError::InvalidResponse(error.to_string()))
}

impl UploadTransport for HttpUploadTransport {
    fn create(&self, token: &str, metadata: &UploadMetadata) -> Result<UploadSession, UploadError> {
        let url = self.url("/api/v1/uploads");
        let token = token.to_owned();
        let metadata = metadata.clone();
        let build: RequestFactory =
            Arc::new(move |client| client.post(url.clone()).bearer_auth(&token).json(&metadata));
        let answer = self
            .http
            .send_safe(self.http.policy().attempt_timeout, &build)?;
        if answer.is_success() {
            return parse_session(&answer);
        }
        // Every refusal of the creation -- the file is too large, the space
        // has no room, too many uploads are open -- is read by its status
        // and its code (`UploadError::verdict`), never by its wording.
        Err(Answered::of(&answer).into_error())
    }

    fn status(&self, token: &str, upload_id: &str) -> Result<UploadSession, UploadError> {
        let url = self.url(&format!("/api/v1/uploads/{upload_id}"));
        let token = token.to_owned();
        let build: RequestFactory =
            Arc::new(move |client| client.get(url.clone()).bearer_auth(&token));
        let answer = self
            .http
            .send_safe(self.http.policy().attempt_timeout, &build)?;
        if answer.is_success() {
            return parse_session(&answer);
        }
        let refusal = Answered::of(&answer);
        if refusal.upload_is_gone() {
            return Err(UploadError::SessionNotFound(upload_id.to_string()));
        }
        Err(refusal.into_error())
    }

    fn put_chunk(
        &self,
        token: &str,
        upload_id: &str,
        offset: u64,
        chunk: &[u8],
    ) -> Result<PutOutcome, UploadError> {
        let url = self.url(&format!("/api/v1/uploads/{upload_id}"));
        let token = token.to_owned();
        let chunk = chunk.to_vec();
        let build: RequestFactory = Arc::new(move |client| {
            client
                .put(url.clone())
                .bearer_auth(&token)
                .header("Upload-Offset", offset.to_string())
                .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
                .body(chunk.clone())
        });
        let answer = self
            .http
            .send_safe(self.http.policy().chunk_timeout, &build)?;
        if answer.is_success() {
            return parse_session(&answer).map(PutOutcome::Advanced);
        }
        let refusal = Answered::of(&answer);
        if refusal.upload_is_gone() {
            return Err(UploadError::SessionNotFound(upload_id.to_string()));
        }
        // The one `409` a chunk is answered with that is not a refusal: the
        // offset disagreed, and the service says where it really is. Any
        // other `409` -- `storage_quota_reached` in the middle of a
        // transfer, for one -- is what it says.
        if refusal.status == 409
            && refusal.code == "upload_offset_mismatch"
            && let Some(offset) = refusal.offset
        {
            return Ok(PutOutcome::OffsetMismatch { offset });
        }
        Err(refusal.into_error())
    }

    fn finalize(&self, token: &str, upload_id: &str) -> Result<FinalizeOutcome, UploadError> {
        let url = self.url(&format!("/api/v1/uploads/{upload_id}/finalize"));
        let token = token.to_owned();
        let build: RequestFactory =
            Arc::new(move |client| client.post(url.clone()).bearer_auth(&token));
        let answer = self
            .http
            .send_safe(self.http.policy().attempt_timeout, &build)?;
        if answer.is_success() {
            return serde_json::from_slice::<FinalizeResponse>(&answer.body)
                .map(FinalizeOutcome::Completed)
                .map_err(|error| UploadError::InvalidResponse(error.to_string()));
        }
        let refusal = Answered::of(&answer);
        if refusal.upload_is_gone() {
            return Err(UploadError::SessionNotFound(upload_id.to_string()));
        }
        // Only what the contract says a finalize answers is read as more
        // than a refusal: `409 upload_incomplete` (with the real offset) and
        // `422 upload_hash_mismatch`. Any other `409` -- the space's storage
        // or subscription changed while the file was on its way -- or `422`
        // is a refusal like the rest; reading it as "incomplete" would send
        // the same chunk and the same finalize again, forever.
        match (refusal.status, refusal.code.as_str()) {
            (409, "upload_incomplete") => Ok(FinalizeOutcome::Incomplete {
                offset: refusal.offset.unwrap_or(0),
            }),
            (422, "upload_hash_mismatch") => Ok(FinalizeOutcome::HashMismatch),
            (409, "upload_capture_gone") => Ok(FinalizeOutcome::CaptureGone),
            _ => Err(refusal.into_error()),
        }
    }

    /// Idempotent (« the same proposals twice change nothing »), so the
    /// patient client may send it again while the service wakes.
    fn propose_for_capture(
        &self,
        token: &str,
        capture_id: &str,
        proposals: &Proposals,
    ) -> Result<Vec<SkippedField>, UploadError> {
        let url = self.url(&format!("/api/v1/captures/{capture_id}"));
        let token = token.to_owned();
        let body = serde_json::json!({ "proposed": capture_proposed_body(proposals) });
        let build: RequestFactory =
            Arc::new(move |client| client.patch(url.clone()).bearer_auth(&token).json(&body));
        self.proposed_answer(&build)
    }

    fn propose_for_image(
        &self,
        token: &str,
        capture_id: &str,
        image_id: &str,
        proposals: &Proposals,
    ) -> Result<Vec<SkippedField>, UploadError> {
        let url = self.url(&format!(
            "/api/v1/captures/{capture_id}/variants/{image_id}"
        ));
        let token = token.to_owned();
        let body = image_proposed_body(proposals);
        let build: RequestFactory =
            Arc::new(move |client| client.patch(url.clone()).bearer_auth(&token).json(&body));
        self.proposed_answer(&build)
    }
}

impl HttpUploadTransport {
    /// `{ capture, skipped }`: only what was left is read.
    fn proposed_answer(&self, build: &RequestFactory) -> Result<Vec<SkippedField>, UploadError> {
        #[derive(Deserialize)]
        struct Answer {
            #[serde(default)]
            skipped: Vec<SkippedField>,
        }
        let answer = self
            .http
            .send_safe(self.http.policy().attempt_timeout, build)?;
        if answer.is_success() {
            return serde_json::from_slice::<Answer>(&answer.body)
                .map(|a| a.skipped)
                .map_err(|error| UploadError::InvalidResponse(error.to_string()));
        }
        Err(Answered::of(&answer).into_error())
    }
}

/// `PATCH /v1/captures/{id}`'s `proposed`: the capture's part, under the
/// producer's name.
#[must_use]
pub fn capture_proposed_body(proposals: &Proposals) -> serde_json::Value {
    let mut body =
        serde_json::to_value(proposals.capture_part()).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(map) = body.as_object_mut() {
        map.insert("source".into(), metadata::SOURCE.into());
    }
    body
}

/// `PATCH /v1/captures/{id}/variants/{imageId}`: the generated title and
/// description of the image (ten languages), its own steps and its quality, under the
/// producer's name -- never the owner's `title` nor `description`.
#[must_use]
pub fn image_proposed_body(proposals: &Proposals) -> serde_json::Value {
    let mut body = serde_json::Map::new();
    if let Some(processing) = &proposals.variant_processing
        && let Ok(value) = serde_json::to_value(processing)
    {
        body.insert("processing".into(), value);
    }
    if let Some(title) = &proposals.variant_generated_title
        && let Ok(value) = serde_json::to_value(title)
    {
        body.insert("generatedTitle".into(), value);
    }
    if let Some(description) = &proposals.variant_generated_description
        && let Ok(value) = serde_json::to_value(description)
    {
        body.insert("generatedDescription".into(), value);
    }
    if let Some(quality) = &proposals.image_quality
        && let Ok(value) = serde_json::to_value(quality)
    {
        body.insert("imageQuality".into(), value);
    }
    body.insert("source".into(), metadata::SOURCE.into());
    serde_json::Value::Object(body)
}

/// The create step's metadata: the row's own attribution where it has one,
/// else the run's `target`.
pub fn build_metadata(entry: &QueueEntry, target: &UploadTarget) -> UploadMetadata {
    let original_name = entry
        .source_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| entry.sha256.clone());
    let own = &entry.target;
    if entry.kind == ResultKind::Plan && own.variant_of.is_none() {
        return plan_image_metadata(entry, original_name);
    }
    // A second render of a plan is an image of the capture the first one
    // creates, not a capture of its own. The service refuses `role: plan` with
    // a `captureId`, so it leaves as any variant: the capture (`captureId`,
    // resolved at the upload) and the target, no role and no plan fields.
    UploadMetadata {
        session_id: own.session_id.clone().or_else(|| target.session_id.clone()),
        plan_id: target.plan_id.clone(),
        target_ids: if own.target_ids.is_empty() {
            target.target_ids.clone()
        } else {
            own.target_ids.clone()
        },
        original_name,
        sha256: entry.sha256.clone(),
        byte_size: entry.size,
        content_type: None,
        capture_id: own.variant_of.clone(),
        source: own.source.clone(),
        role: (entry.kind == ResultKind::Progression)
            .then(|| crate::precheck::PROGRESSION_ROLE.to_string()),
        progression_frames: own
            .frame_count
            .filter(|_| entry.kind == ResultKind::Progression),
        project_id: None,
        combined_night_ids: None,
        totals_by_filter: None,
        plan_render: None,
        metadata_source: None,
        proposals: Proposals::default(),
    }
    .with_proposals(own.metadata.as_ref(), own.variant_of.is_some())
}

/// The role an image of a multi-night plan is sent with.
pub const PLAN_ROLE: &str = "plan";

/// The create step of an image of a multi-night plan (`uploads.md`, « The
/// image of a multi-night plan »): its plan and what it combines, never a
/// night (`sessionId`, `planId` and `captureId` absent: the run's own
/// attribution is not used).
fn plan_image_metadata(entry: &QueueEntry, original_name: String) -> UploadMetadata {
    let plan = entry.target.plan.clone().unwrap_or_default();
    UploadMetadata {
        session_id: None,
        plan_id: None,
        target_ids: entry.target.target_ids.clone(),
        original_name,
        sha256: entry.sha256.clone(),
        byte_size: entry.size,
        content_type: None,
        capture_id: None,
        source: entry.target.source.clone(),
        role: Some(PLAN_ROLE.to_string()),
        progression_frames: None,
        project_id: Some(plan.project_id),
        combined_night_ids: Some(plan.combined_night_ids),
        totals_by_filter: Some(plan.totals_by_filter),
        plan_render: plan.plan_render,
        metadata_source: None,
        proposals: Proposals::default(),
    }
    // The quality of the combined image, when it was measured and the space
    // may keep it: the image creates the capture of the plan, so it carries
    // every proposal there is.
    .with_proposals(entry.target.metadata.as_ref(), false)
    .with_plan_processing()
}

impl UploadMetadata {
    /// The image of a plan always says what made it. A row queued before the
    /// processing was kept with the row (or by a build that did not) is given
    /// « Astro Sync <version> » and what the row knows of the plan, so the
    /// service never falls back on the `Software` tag of the TIFF («
    /// tifffile.py »).
    fn with_plan_processing(mut self) -> Self {
        if self.proposals.processing.is_some() {
            return self;
        }
        let mut steps = Vec::new();
        if let Some(render) = self.plan_render.as_deref() {
            steps.push(render.to_string());
        }
        if let Some(nights) = self.combined_night_ids.as_ref().filter(|n| !n.is_empty()) {
            steps.push(format!("{} nights", nights.len()));
        }
        self.proposals.processing = Some(metadata::processing_proposal(
            &metadata::software_label(None, false),
            &steps,
        ));
        self.metadata_source
            .get_or_insert_with(|| metadata::SOURCE.to_string());
        self
    }
}

impl UploadMetadata {
    /// The proposals the create step carries: everything for the image that
    /// creates a capture, only the title and the description of an image that
    /// is a variant (the capture's part is proposed after the fact, once the
    /// capture is known: [`follow_up_proposals`]).
    #[must_use]
    pub fn with_proposals(mut self, proposals: Option<&Proposals>, is_variant: bool) -> Self {
        let carried = proposals.map(|p| {
            if is_variant {
                p.image_part()
            } else {
                p.clone()
            }
        });
        if let Some(carried) = carried.filter(|p| !p.is_empty()) {
            self.metadata_source = Some(metadata::SOURCE.to_string());
            self.proposals = carried;
        }
        self
    }
}

/// The capture the image `first_sha256` of the same sending created: a
/// variant waits for it (held, `variant_capture_pending`) and is refused for
/// good when that image was (`variant_capture_refused`).
fn capture_created_by(queue: &UploadQueue, first_sha256: &str) -> Result<String, UploadError> {
    use crate::upload_queue::QueueState;
    match queue.find_by_sha256(first_sha256)? {
        Some(first) if first.state == QueueState::Completed && first.capture_id.is_some() => {
            Ok(first.capture_id.unwrap_or_default())
        }
        Some(first) if matches!(first.state, QueueState::Refused | QueueState::Failed) => {
            Err(UploadError::refusal(
                400,
                "variant_capture_refused",
                "The image whose capture this one is a variant of was not sent.",
            ))
        }
        _ => Err(UploadError::refusal(
            409,
            "variant_capture_pending",
            "The image whose capture this one is a variant of is not sent yet.",
        )),
    }
}

/// Gets an authoritative session for `entry`: reconciles an existing
/// `upload_id` through `status` (never assumes it is still at the offset
/// last written locally), or creates a fresh one. A session whose id has
/// expired (or never existed) is forgotten and recreated once.
fn reconcile_or_create<T: UploadTransport>(
    transport: &T,
    queue: &UploadQueue,
    token: &str,
    entry: &QueueEntry,
    metadata: &UploadMetadata,
) -> Result<UploadSession, UploadError> {
    let session = match &entry.upload_id {
        Some(upload_id) => match transport.status(token, upload_id) {
            Ok(session) => session,
            Err(UploadError::SessionNotFound(_)) => {
                queue.reset_for_restart(&entry.sha256)?;
                transport.create(token, metadata)?
            }
            Err(other) => return Err(other),
        },
        None => transport.create(token, metadata)?,
    };
    queue.mark_progress(&entry.sha256, &session.upload_id, session.offset)?;
    Ok(session)
}

/// Sends every remaining chunk of `entry`'s file, from `session.offset`
/// onward, never larger than `session.chunk_size`, persisting progress to
/// `queue` after each accepted chunk. Mutates `session.offset` in place.
fn send_chunks<T: UploadTransport>(
    transport: &T,
    queue: &UploadQueue,
    token: &str,
    entry: &QueueEntry,
    session: &mut UploadSession,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<(), UploadError> {
    if session.offset >= session.byte_size {
        return Ok(());
    }
    let mut file = File::open(&entry.source_path).map_err(|source| UploadError::SourceFile {
        path: entry.source_path.clone(),
        source,
    })?;
    let chunk_size = session.chunk_size.max(1) as usize;
    let mut buffer = vec![0_u8; chunk_size];
    let mut stuck_attempts = 0_u32;

    while session.offset < session.byte_size {
        let remaining = (session.byte_size - session.offset) as usize;
        let this_len = remaining.min(chunk_size);
        file.seek(SeekFrom::Start(session.offset))
            .map_err(|source| UploadError::SourceFile {
                path: entry.source_path.clone(),
                source,
            })?;
        file.read_exact(&mut buffer[..this_len])
            .map_err(|source| UploadError::SourceFile {
                path: entry.source_path.clone(),
                source,
            })?;

        match transport.put_chunk(
            token,
            &session.upload_id,
            session.offset,
            &buffer[..this_len],
        )? {
            PutOutcome::Advanced(updated) => {
                session.offset = updated.offset;
                stuck_attempts = 0;
                queue.mark_progress(&entry.sha256, &session.upload_id, session.offset)?;
                on_progress(session.offset, session.byte_size);
            }
            PutOutcome::OffsetMismatch { offset } => {
                stuck_attempts += 1;
                if stuck_attempts > 3 {
                    return Err(UploadError::GaveUp {
                        attempts: stuck_attempts,
                        reason: format!(
                            "persistent offset disagreement with the server (offset {offset})"
                        ),
                    });
                }
                session.offset = offset;
                queue.mark_progress(&entry.sha256, &session.upload_id, session.offset)?;
            }
        }
    }
    Ok(())
}

/// Drives one [`QueueEntry`] from wherever it currently stands through to a
/// finalized capture: reconciles or creates its session, sends every
/// remaining chunk (resuming from the server's own reported offset, never
/// assuming one), finalizes, and reacts to a hash mismatch by restarting
/// the transfer from scratch exactly once more before giving up. `entry`
/// already `completed` (the create/status call answers `completed: true`,
/// e.g. because the bytes were already a finalized capture, or the file is
/// being re-uploaded) is a no-op beyond recording the outcome -- the
/// contract's re-upload-is-idempotent guarantee.
///
/// What becomes of the queue entry when this fails is decided by the
/// failure's [`Verdict`], recorded here before the error is returned, so a
/// caller looping over `next_pending` never spins on the same refusal:
///
/// - `Refused`: the entry is marked `refused` with the code of the refusal;
/// - `Failed`: the entry is marked `failed` with a readable reason;
/// - `NeedsPerson`: the entry is `held`, with its session and its offset;
/// - anything else (a network that is down, a service that fails, too many
///   uploads open, a sign-in the service ended, a local database error) leaves
///   the entry exactly as progress had already recorded it, ready to resume.
///
/// The file on the computer is only ever read.
pub fn upload_entry<T: UploadTransport>(
    transport: &T,
    queue: &UploadQueue,
    token: &str,
    entry: &QueueEntry,
    target: &UploadTarget,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<UploadOutcome, UploadError> {
    let result = upload_entry_inner(transport, queue, token, entry, target, &mut on_progress);
    if let Err(error) = &result {
        let reason = error.to_string();
        let _ = match error.verdict() {
            Verdict::Refused => queue.mark_refused(&entry.sha256, error.code(), &reason),
            Verdict::Failed if error.code() == SOURCE_FILE_MISSING => {
                queue.mark_failed_coded(&entry.sha256, SOURCE_FILE_MISSING, &reason)
            }
            Verdict::Failed => queue.mark_failed(&entry.sha256, &reason),
            Verdict::NeedsPerson => queue.hold(&entry.sha256, error.code(), &reason),
            Verdict::RetryLater | Verdict::Cancelled | Verdict::SignedOut | Verdict::Local => {
                Ok(())
            }
        };
    }
    result
}

/// What the proposals after the fact answered.
#[derive(Debug, Default)]
struct Proposed {
    skipped: Vec<SkippedField>,
    refused: Option<String>,
}

/// The proposals the create step did not carry, or the service did not apply,
/// go to the capture that now exists.
///
/// - an image that **creates** a capture carried everything in its create
///   step, and the service applied it at completion (`applied_at_create`):
///   nothing more;
/// - a **variant** carried its title and description; the capture's part
///   (acquisition, calibration, sensor) goes now;
/// - bytes the service **already held** (a deduplicated finalize, a session
///   already complete) applied nothing at create: both parts go now (the
///   image's title and description only when its id is known).
///
/// A refusal of the proposals (a field a device may not write, a value out of
/// bounds, a subscription ended) never fails the upload -- the image is
/// there; its code is returned. A failure that will pass (network, service)
/// is returned as an error before the file is marked completed, so the next
/// run proposes again (the service takes the same proposals twice as one).
fn follow_up_proposals<T: UploadTransport>(
    transport: &T,
    token: &str,
    entry: &QueueEntry,
    capture_id: &str,
    image_id: Option<&str>,
    applied_at_create: bool,
) -> Result<Proposed, UploadError> {
    let mut answer = Proposed::default();
    let Some(proposals) = entry.target.metadata.as_ref().filter(|p| !p.is_empty()) else {
        return Ok(answer);
    };
    let is_variant = entry.target.variant_of.is_some();
    // How the capture was made is its main image's to say: a variant never
    // proposes its own steps over them (the last variant sent would win).
    let capture_part = if is_variant {
        proposals.capture_part().without_processing()
    } else {
        proposals.capture_part()
    };
    if !capture_part.is_empty() && (is_variant || !applied_at_create) {
        absorb(
            transport.propose_for_capture(token, capture_id, &capture_part),
            &mut answer,
        )?;
    }
    if proposals.has_image_part()
        && !applied_at_create
        && let Some(image_id) = image_id
    {
        absorb(
            transport.propose_for_image(token, capture_id, image_id, &proposals.image_part()),
            &mut answer,
        )?;
    }
    Ok(answer)
}

fn absorb(
    result: Result<Vec<SkippedField>, UploadError>,
    answer: &mut Proposed,
) -> Result<(), UploadError> {
    match result {
        Ok(skipped) => {
            // A proposal the space has no right to keep (`capability`) is not
            // told: the image is sent all the same.
            answer.skipped.extend(metadata::visible_skipped(&skipped));
            Ok(())
        }
        Err(error) => match error.verdict() {
            Verdict::Refused | Verdict::Failed | Verdict::NeedsPerson => {
                // The service's own code (`web_session_required`,
                // `invalid_metadata`...) when it gave one.
                let code = match &error {
                    UploadError::Refused { code, .. } => code.clone(),
                    other => other.code().to_string(),
                };
                answer.refused.get_or_insert(code);
                Ok(())
            }
            _ => Err(error),
        },
    }
}

/// The proposals for an image already online, which no upload carries
/// ("already online, identical"): the capture's part, then the image's
/// title and description. One answer per plan; a refusal is a code, never
/// an error.
pub fn propose_for_online_images<T: UploadTransport>(
    transport: &T,
    token: &str,
    plans: &[crate::precheck::ImagePlan],
) -> Result<Vec<OnlineProposal>, UploadError> {
    use crate::precheck::Decision;
    let mut answers = Vec::new();
    for plan in plans {
        let (
            Decision::AlreadyOnline {
                capture_id,
                image_id,
            },
            Some(proposals),
        ) = (&plan.decision, plan.metadata.as_ref())
        else {
            continue;
        };
        let mut proposed = Proposed::default();
        let capture_part = proposals.capture_part();
        if !capture_part.is_empty() {
            absorb(
                transport.propose_for_capture(token, capture_id, &capture_part),
                &mut proposed,
            )?;
        }
        if proposals.has_image_part() {
            absorb(
                transport.propose_for_image(token, capture_id, image_id, &proposals.image_part()),
                &mut proposed,
            )?;
        }
        answers.push(OnlineProposal {
            sha256: plan.image.sha256.clone(),
            capture_id: capture_id.clone(),
            note: metadata::skipped_sentence(&proposed.skipped),
            skipped: proposed.skipped,
            refused: proposed.refused,
        });
    }
    Ok(answers)
}

/// What proposing for an image already online answered.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineProposal {
    pub sha256: String,
    pub capture_id: String,
    pub skipped: Vec<SkippedField>,
    /// The sentence that says what the owner's values left.
    pub note: Option<String>,
    /// The code of a refusal of the proposals, if any.
    pub refused: Option<String>,
}

/// How many times finalize may answer "not all bytes arrived" in a row for
/// one transfer before Astro Sync stops asking: each one is followed by the
/// missing bytes, so a service that keeps saying it is not a service that is
/// waiting for them.
const MAX_INCOMPLETE_ANSWERS: u32 = 5;

fn upload_entry_inner<T: UploadTransport>(
    transport: &T,
    queue: &UploadQueue,
    token: &str,
    entry: &QueueEntry,
    target: &UploadTarget,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<UploadOutcome, UploadError> {
    // A folder renamed since the row was written (the provisional folder of
    // a plan, settled once its plan was created): the file is where the
    // renamed folder leads, and the row says so from now on.
    let relocated;
    let entry = match queue.moved_source(&entry.source_path)? {
        Some(path) => {
            queue.relocate(&entry.sha256, &path)?;
            relocated = QueueEntry {
                source_path: path,
                ..entry.clone()
            };
            &relocated
        }
        None => entry,
    };
    // Whatever row a queue holds, a frame or a raw log never leaves.
    if crate::upload_queue::result_kind_of(&entry.source_path).is_err() {
        return Err(UploadError::refusal(
            400,
            "not_a_result",
            "Astro Sync sends results only: never a frame nor a raw log.",
        ));
    }
    let mut metadata = build_metadata(entry, target);
    if let Some(first) = entry
        .target
        .variant_of
        .as_deref()
        .and_then(|v| v.strip_prefix(VARIANT_OF_SHA256))
    {
        metadata.capture_id = Some(capture_created_by(queue, first)?);
    }
    let mut session = reconcile_or_create(transport, queue, token, entry, &metadata)?;

    if session.completed {
        return already_sent(transport, queue, token, entry, &session, on_progress);
    }

    let mut restarts = 0_u32;
    let mut incomplete_answers = 0_u32;
    let mut recreated = false;
    loop {
        send_chunks(transport, queue, token, entry, &mut session, on_progress)?;

        match transport.finalize(token, &session.upload_id)? {
            FinalizeOutcome::Completed(response) => {
                let image_id = response.image_id_of(&entry.sha256);
                let mut proposed = follow_up_proposals(
                    transport,
                    token,
                    entry,
                    &response.capture_id,
                    image_id.as_deref(),
                    !response.deduplicated,
                )?;
                // What the service left when it applied the create step's
                // proposals at completion comes first.
                let mut skipped = response.skipped.clone();
                skipped.append(&mut proposed.skipped);
                let skipped = metadata::visible_skipped(&skipped);
                queue.mark_completed(&entry.sha256, &response.capture_id)?;
                if response.replaced {
                    queue.mark_replaced(&entry.sha256)?;
                }
                on_progress(entry.size, entry.size);
                return Ok(UploadOutcome {
                    capture_id: response.capture_id,
                    deduplicated: response.deduplicated,
                    skipped,
                    proposals_refused: proposed.refused,
                    replaced: response.replaced,
                });
            }
            FinalizeOutcome::Incomplete { offset } => {
                incomplete_answers += 1;
                if incomplete_answers > MAX_INCOMPLETE_ANSWERS {
                    return Err(UploadError::GaveUp {
                        attempts: incomplete_answers,
                        reason: format!(
                            "the service keeps answering that bytes are missing (offset {offset})"
                        ),
                    });
                }
                queue.mark_progress(&entry.sha256, &session.upload_id, offset)?;
                session.offset = offset;
            }
            FinalizeOutcome::HashMismatch => {
                restarts += 1;
                if restarts > 1 {
                    return Err(UploadError::GaveUp {
                        attempts: restarts,
                        reason: "invalid digest even after a complete new upload".to_string(),
                    });
                }
                queue.reset_for_restart(&entry.sha256)?;
                session.offset = 0;
            }
            FinalizeOutcome::CaptureGone => {
                // The upload was aimed at a capture that left the space after
                // its create (an upload left open while the space was
                // emptied, then the plan sent again). Not a refusal of the
                // image: a new create names the capture it goes to now, and
                // the service keeps the bytes received. Once: a capture that
                // is really gone is refused by that create
                // (`404 capture_not_found`).
                if recreated {
                    return Err(UploadError::refusal(
                        409,
                        "upload_capture_gone",
                        "The capture this image is added to is no longer in the space.",
                    ));
                }
                recreated = true;
                session = transport.create(token, &metadata)?;
                queue.mark_progress(&entry.sha256, &session.upload_id, session.offset)?;
                if session.completed {
                    return already_sent(transport, queue, token, entry, &session, on_progress);
                }
            }
        }
    }
}

/// Bytes the space already held (a create or a status that answers
/// `completed: true`): the create step applied nothing, so the proposals go
/// after the fact, and the row records the capture.
fn already_sent<T: UploadTransport>(
    transport: &T,
    queue: &UploadQueue,
    token: &str,
    entry: &QueueEntry,
    session: &UploadSession,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<UploadOutcome, UploadError> {
    let capture_id = session.capture_id.clone().ok_or_else(|| {
        UploadError::InvalidResponse("completed session without captureId".to_string())
    })?;
    let proposed = follow_up_proposals(transport, token, entry, &capture_id, None, false)?;
    queue.mark_completed(&entry.sha256, &capture_id)?;
    on_progress(entry.size, entry.size);
    Ok(UploadOutcome {
        capture_id,
        deduplicated: true,
        skipped: proposed.skipped,
        proposals_refused: proposed.refused,
        replaced: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dry_run::{DryRunEntry, DryRunReport};
    use crate::package::AssetKind;
    use crate::scanner::FileClass;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::fs;

    /// A scriptable fake transport: each method pops its next scripted
    /// response (or panics if the test did not script enough calls -- a
    /// signal the orchestration called it more times than expected).
    #[derive(Default)]
    struct FakeTransport {
        creates: RefCell<VecDeque<Result<UploadSession, UploadError>>>,
        statuses: RefCell<VecDeque<Result<UploadSession, UploadError>>>,
        puts: RefCell<VecDeque<Result<PutOutcome, UploadError>>>,
        finalizes: RefCell<VecDeque<Result<FinalizeOutcome, UploadError>>>,
        /// What each create step carried.
        created: RefCell<Vec<UploadMetadata>>,
        /// The scripted answers of the proposals after the fact (an empty
        /// script answers `Ok(vec![])`) and what was proposed.
        capture_answers: RefCell<VecDeque<Result<Vec<SkippedField>, UploadError>>>,
        image_answers: RefCell<VecDeque<Result<Vec<SkippedField>, UploadError>>>,
        captures_proposed: RefCell<Vec<(String, Proposals)>>,
        images_proposed: RefCell<Vec<(String, String, Proposals)>>,
    }

    fn err_clone(error: &UploadError) -> UploadError {
        match error {
            UploadError::Transport(message) => UploadError::Transport(message.clone()),
            UploadError::Wait(wait) => UploadError::Wait(wait.clone()),
            UploadError::InvalidResponse(message) => UploadError::InvalidResponse(message.clone()),
            UploadError::SessionNotFound(id) => UploadError::SessionNotFound(id.clone()),
            UploadError::Refused {
                status,
                code,
                message,
                retry_after_seconds,
            } => UploadError::Refused {
                status: *status,
                code: code.clone(),
                message: message.clone(),
                retry_after_seconds: *retry_after_seconds,
            },
            UploadError::SourceFile { path, source } => UploadError::SourceFile {
                path: path.clone(),
                source: std::io::Error::new(source.kind(), source.to_string()),
            },
            UploadError::Queue(_) => UploadError::Transport("queue error clone unsupported".into()),
            UploadError::GaveUp { attempts, reason } => UploadError::GaveUp {
                attempts: *attempts,
                reason: reason.clone(),
            },
            UploadError::SignInEnded(end) => UploadError::SignInEnded(*end),
        }
    }

    impl UploadTransport for FakeTransport {
        fn create(
            &self,
            _token: &str,
            metadata: &UploadMetadata,
        ) -> Result<UploadSession, UploadError> {
            self.created.borrow_mut().push(metadata.clone());
            self.creates
                .borrow_mut()
                .pop_front()
                .expect("unscripted create() call")
        }

        fn status(&self, _token: &str, _upload_id: &str) -> Result<UploadSession, UploadError> {
            self.statuses
                .borrow_mut()
                .pop_front()
                .expect("unscripted status() call")
        }

        fn put_chunk(
            &self,
            _token: &str,
            _upload_id: &str,
            _offset: u64,
            _chunk: &[u8],
        ) -> Result<PutOutcome, UploadError> {
            self.puts
                .borrow_mut()
                .pop_front()
                .expect("unscripted put_chunk() call")
        }

        fn finalize(&self, _token: &str, _upload_id: &str) -> Result<FinalizeOutcome, UploadError> {
            self.finalizes
                .borrow_mut()
                .pop_front()
                .expect("unscripted finalize() call")
        }

        fn propose_for_capture(
            &self,
            _token: &str,
            capture_id: &str,
            proposals: &Proposals,
        ) -> Result<Vec<SkippedField>, UploadError> {
            self.captures_proposed
                .borrow_mut()
                .push((capture_id.to_string(), proposals.clone()));
            self.capture_answers
                .borrow_mut()
                .pop_front()
                .unwrap_or(Ok(Vec::new()))
        }

        fn propose_for_image(
            &self,
            _token: &str,
            capture_id: &str,
            image_id: &str,
            proposals: &Proposals,
        ) -> Result<Vec<SkippedField>, UploadError> {
            self.images_proposed.borrow_mut().push((
                capture_id.to_string(),
                image_id.to_string(),
                proposals.clone(),
            ));
            self.image_answers
                .borrow_mut()
                .pop_front()
                .unwrap_or(Ok(Vec::new()))
        }
    }

    fn session(
        upload_id: &str,
        offset: u64,
        byte_size: u64,
        chunk_size: u64,
        completed: bool,
    ) -> UploadSession {
        UploadSession {
            upload_id: upload_id.to_string(),
            offset,
            byte_size,
            chunk_size,
            completed,
            expires_at: None,
            capture_id: None,
        }
    }

    /// Builds a queue with one entry backed by a real temp file of `bytes`
    /// content. Returns `(queue, entry, source_dir_guard, queue_dir_guard)`
    /// -- both guards must stay alive for the duration of the test (their
    /// `Drop` removes the temp directories).
    fn queue_with_file(
        bytes: &[u8],
        sha256: &str,
    ) -> (
        UploadQueue,
        QueueEntry,
        crate::test_support::TempDir,
        crate::test_support::TempDir,
    ) {
        let source_dir = crate::test_support::TempDir::new("uploader-source");
        fs::write(source_dir.path().join("frame.tif"), bytes).unwrap();
        let report = DryRunReport {
            source_root: source_dir.path().to_path_buf(),
            entries: vec![DryRunEntry {
                source_relative_path: "frame.tif".to_string(),
                class: FileClass::Fits,
                asset_kind: AssetKind::Image,
                package_relative_path: "images/frame.tif".to_string(),
                bytes: bytes.len() as u64,
                sha256: sha256.to_string(),
            }],
            total_bytes: bytes.len() as u64,
        };
        let queue_dir = crate::test_support::TempDir::new("uploader-queue-db");
        let queue = UploadQueue::open(&queue_dir.path().join("queue.sqlite")).unwrap();
        queue.enqueue_from_dry_run(&report).unwrap();
        let entry = queue.find_by_sha256(sha256).unwrap().unwrap();
        (queue, entry, source_dir, queue_dir)
    }

    #[test]
    fn a_fresh_upload_creates_sends_every_chunk_and_finalizes() {
        let (queue, entry, _guard, _queue_guard) = queue_with_file(b"0123456789", "sha-fresh");
        let transport = FakeTransport::default();
        transport
            .creates
            .borrow_mut()
            .push_back(Ok(session("up-1", 0, 10, 4, false)));
        transport
            .puts
            .borrow_mut()
            .push_back(Ok(PutOutcome::Advanced(session("up-1", 4, 10, 4, false))));
        transport
            .puts
            .borrow_mut()
            .push_back(Ok(PutOutcome::Advanced(session("up-1", 8, 10, 4, false))));
        transport
            .puts
            .borrow_mut()
            .push_back(Ok(PutOutcome::Advanced(session("up-1", 10, 10, 4, false))));
        transport
            .finalizes
            .borrow_mut()
            .push_back(Ok(FinalizeOutcome::Completed(FinalizeResponse {
                capture_id: "capture-1".to_string(),
                deduplicated: false,
                ..Default::default()
            })));

        let mut progress_calls = Vec::new();
        let outcome = upload_entry(
            &transport,
            &queue,
            "test-token",
            &entry,
            &UploadTarget::default(),
            |sent, total| progress_calls.push((sent, total)),
        )
        .unwrap();

        assert_eq!(outcome.capture_id, "capture-1");
        assert!(!outcome.deduplicated);
        assert_eq!(progress_calls, vec![(4, 10), (8, 10), (10, 10), (10, 10)]);

        let stored = queue.find_by_sha256("sha-fresh").unwrap().unwrap();
        assert_eq!(stored.state, crate::upload_queue::QueueState::Completed);
        assert_eq!(stored.capture_id.as_deref(), Some("capture-1"));
        assert_eq!(stored.offset, 10);
    }

    /// The plan folder of an image waiting in the queue was renamed after its
    /// plan was created: the file is sent from where the folder is now, and
    /// the row says so.
    #[test]
    fn a_waiting_file_whose_folder_was_renamed_is_sent_from_the_new_place() {
        use crate::upload_queue::{EntryTarget, QueueState, ResultFile, ResultKind};
        let root = crate::test_support::TempDir::new("uploader-renamed");
        let folder = root.path().join("Sh2-103 · local-fea0a6");
        fs::create_dir_all(folder.join("progression")).unwrap();
        let file = folder.join("progression/plan_night-01_120-frames.jpg");
        fs::write(&file, b"0123").unwrap();
        let queue = UploadQueue::open(&root.path().join("queue.sqlite")).unwrap();
        queue
            .enqueue_results(&[ResultFile {
                path: file,
                sha256: "sha-renamed".into(),
                size: 4,
                kind: ResultKind::Progression,
                target: EntryTarget::default(),
            }])
            .unwrap();
        // The entry as the run read it, before the rename.
        let entry = queue.find_by_sha256("sha-renamed").unwrap().unwrap();
        let renamed = root.path().join("Sh2-103 · 80340a");
        fs::rename(&folder, &renamed).unwrap();
        queue.rebase_provisional(&folder, &renamed, &[]).unwrap();

        let transport = FakeTransport::default();
        transport
            .creates
            .borrow_mut()
            .push_back(Ok(session("up-1", 0, 4, 4, false)));
        transport
            .puts
            .borrow_mut()
            .push_back(Ok(PutOutcome::Advanced(session("up-1", 4, 4, 4, false))));
        transport
            .finalizes
            .borrow_mut()
            .push_back(Ok(FinalizeOutcome::Completed(FinalizeResponse {
                capture_id: "capture-1".to_string(),
                ..Default::default()
            })));
        upload_entry(
            &transport,
            &queue,
            "test-token",
            &entry,
            &UploadTarget::default(),
            |_, _| {},
        )
        .unwrap();
        let stored = queue.find_by_sha256("sha-renamed").unwrap().unwrap();
        assert_eq!(stored.state, QueueState::Completed);
        assert_eq!(
            stored.source_path,
            renamed.join("progression/plan_night-01_120-frames.jpg")
        );
    }

    /// A file that is really gone fails with its own code, which the
    /// interface turns into « fichier introuvable : <nom>, il était dans
    /// <dossier> » with « Réessayer »; trying again sends it once it is back.
    #[test]
    fn a_missing_file_says_which_file_and_which_folder_and_can_be_retried() {
        use crate::upload_queue::QueueState;
        let (queue, entry, source_dir, _queue_dir) = queue_with_file(b"0123", "sha-gone");
        let path = source_dir.path().join("frame.tif");
        fs::remove_file(&path).unwrap();
        let transport = FakeTransport::default();
        transport
            .creates
            .borrow_mut()
            .push_back(Ok(session("up-1", 0, 4, 4, false)));
        let error = upload_entry(
            &transport,
            &queue,
            "test-token",
            &entry,
            &UploadTarget::default(),
            |_, _| {},
        )
        .unwrap_err();
        assert_eq!(error.code(), SOURCE_FILE_MISSING);
        assert_eq!(error.verdict(), Verdict::Failed);
        assert_eq!(
            error.plain_message(),
            format!(
                "File not found: frame.tif (it was in {}).",
                source_dir.path().display()
            )
        );
        let stored = queue.find_by_sha256("sha-gone").unwrap().unwrap();
        assert_eq!(stored.state, QueueState::Failed);
        assert_eq!(stored.last_error_code.as_deref(), Some(SOURCE_FILE_MISSING));

        // The file comes back: « Réessayer ».
        fs::write(&path, b"0123").unwrap();
        queue.requeue_failed("sha-gone").unwrap();
        let again = queue.next_pending().unwrap().unwrap();
        assert_eq!(again.state, QueueState::Queued);
        assert_eq!(again.source_path, path);
    }

    #[test]
    fn create_answering_completed_is_a_no_op_reupload() {
        let (queue, entry, _guard, _queue_guard) = queue_with_file(b"already-there", "sha-dedup");
        let transport = FakeTransport::default();
        transport.creates.borrow_mut().push_back(Ok(UploadSession {
            upload_id: "up-1".to_string(),
            offset: 13,
            byte_size: 13,
            chunk_size: 8,
            completed: true,
            expires_at: None,
            capture_id: Some("capture-existing".to_string()),
        }));

        let outcome = upload_entry(
            &transport,
            &queue,
            "test-token",
            &entry,
            &UploadTarget::default(),
            |_, _| {},
        )
        .unwrap();

        assert_eq!(outcome.capture_id, "capture-existing");
        assert!(outcome.deduplicated);
        // No put_chunk/finalize scripted at all -- if the orchestration
        // tried to send a byte, the fake would panic on an unscripted call.
    }

    #[test]
    fn a_put_offset_mismatch_resyncs_and_keeps_going() {
        let (queue, entry, _guard, _queue_guard) = queue_with_file(b"01234567", "sha-mismatch");
        let transport = FakeTransport::default();
        transport
            .creates
            .borrow_mut()
            .push_back(Ok(session("up-1", 0, 8, 4, false)));
        // The client thought it was at 0; the server says 4 (e.g. a retried
        // request the server had already durably applied once).
        transport
            .puts
            .borrow_mut()
            .push_back(Ok(PutOutcome::OffsetMismatch { offset: 4 }));
        transport
            .puts
            .borrow_mut()
            .push_back(Ok(PutOutcome::Advanced(session("up-1", 8, 8, 4, false))));
        transport
            .finalizes
            .borrow_mut()
            .push_back(Ok(FinalizeOutcome::Completed(FinalizeResponse {
                capture_id: "capture-2".to_string(),
                deduplicated: false,
                ..Default::default()
            })));

        let outcome = upload_entry(
            &transport,
            &queue,
            "test-token",
            &entry,
            &UploadTarget::default(),
            |_, _| {},
        )
        .unwrap();
        assert_eq!(outcome.capture_id, "capture-2");
    }

    #[test]
    fn a_hash_mismatch_at_finalize_restarts_from_zero_once() {
        let (queue, entry, _guard, _queue_guard) =
            queue_with_file(b"01234567", "sha-hash-mismatch");
        let transport = FakeTransport::default();
        transport
            .creates
            .borrow_mut()
            .push_back(Ok(session("up-1", 0, 8, 8, false)));
        transport
            .puts
            .borrow_mut()
            .push_back(Ok(PutOutcome::Advanced(session("up-1", 8, 8, 8, false))));
        transport
            .finalizes
            .borrow_mut()
            .push_back(Ok(FinalizeOutcome::HashMismatch));
        // Restart: the queue was reset to offset 0, so the whole file is
        // sent again in one chunk (chunk_size 8 == byte_size 8 here).
        transport
            .puts
            .borrow_mut()
            .push_back(Ok(PutOutcome::Advanced(session("up-1", 8, 8, 8, false))));
        transport
            .finalizes
            .borrow_mut()
            .push_back(Ok(FinalizeOutcome::Completed(FinalizeResponse {
                capture_id: "capture-3".to_string(),
                deduplicated: false,
                ..Default::default()
            })));

        let outcome = upload_entry(
            &transport,
            &queue,
            "test-token",
            &entry,
            &UploadTarget::default(),
            |_, _| {},
        )
        .unwrap();
        assert_eq!(outcome.capture_id, "capture-3");
    }

    #[test]
    fn a_second_consecutive_hash_mismatch_gives_up_and_marks_failed() {
        let (queue, entry, _guard, _queue_guard) =
            queue_with_file(b"01234567", "sha-hash-mismatch-twice");
        let transport = FakeTransport::default();
        transport
            .creates
            .borrow_mut()
            .push_back(Ok(session("up-1", 0, 8, 8, false)));
        transport
            .puts
            .borrow_mut()
            .push_back(Ok(PutOutcome::Advanced(session("up-1", 8, 8, 8, false))));
        transport
            .finalizes
            .borrow_mut()
            .push_back(Ok(FinalizeOutcome::HashMismatch));
        transport
            .puts
            .borrow_mut()
            .push_back(Ok(PutOutcome::Advanced(session("up-1", 8, 8, 8, false))));
        transport
            .finalizes
            .borrow_mut()
            .push_back(Ok(FinalizeOutcome::HashMismatch));

        let error = upload_entry(
            &transport,
            &queue,
            "test-token",
            &entry,
            &UploadTarget::default(),
            |_, _| {},
        )
        .unwrap_err();
        assert!(matches!(error, UploadError::GaveUp { .. }));

        let stored = queue
            .find_by_sha256("sha-hash-mismatch-twice")
            .unwrap()
            .unwrap();
        assert_eq!(stored.state, crate::upload_queue::QueueState::Failed);
        assert!(stored.last_error.is_some());
    }

    #[test]
    fn a_413_file_too_large_at_create_refuses_the_file_with_no_bytes_sent_and_no_loop() {
        let (queue, entry, _guard, _queue_guard) = queue_with_file(b"01234567", "sha-too-large");
        let transport = FakeTransport::default();
        transport
            .creates
            .borrow_mut()
            .push_back(Err(UploadError::refusal(
                413,
                "file_too_large",
                "byteSize exceeds storage.max_file_bytes",
            )));

        let error = upload_entry(
            &transport,
            &queue,
            "test-token",
            &entry,
            &UploadTarget::default(),
            |_, _| {},
        )
        .unwrap_err();
        assert_eq!(error.code(), "file_too_large");
        // No put_chunk/finalize scripted -- a call to either would panic.

        let stored = queue.find_by_sha256("sha-too-large").unwrap().unwrap();
        assert_eq!(stored.state, crate::upload_queue::QueueState::Refused);
        assert_eq!(stored.last_error_code.as_deref(), Some("file_too_large"));
        assert!(
            stored
                .last_error
                .as_deref()
                .unwrap()
                .contains("max_file_bytes")
        );
        // A second attempt must not be picked up automatically.
        assert_eq!(queue.next_pending().unwrap(), None);
    }

    /// The first failure to fix: a `409` at finalize used to be read as
    /// "not all bytes arrived" whatever it said, so a space whose storage
    /// filled while the file was on its way sent the same chunk and the same
    /// finalize again, forever. It is a refusal that needs the person, and
    /// the session is kept.
    #[test]
    fn a_409_storage_quota_at_finalize_holds_the_file_instead_of_asking_again() {
        let (queue, entry, _guard, _queue_guard) = queue_with_file(b"01234567", "sha-quota-final");
        let transport = FakeTransport::default();
        transport
            .creates
            .borrow_mut()
            .push_back(Ok(session("up-1", 0, 8, 8, false)));
        transport
            .puts
            .borrow_mut()
            .push_back(Ok(PutOutcome::Advanced(session("up-1", 8, 8, 8, false))));
        transport
            .finalizes
            .borrow_mut()
            .push_back(Err(UploadError::refusal(
                409,
                "storage_quota_reached",
                "quota",
            )));
        // Nothing else is scripted: a second put or finalize would panic.

        let error = upload_entry(
            &transport,
            &queue,
            "test-token",
            &entry,
            &UploadTarget::default(),
            |_, _| {},
        )
        .unwrap_err();
        assert_eq!(error.verdict(), Verdict::NeedsPerson);

        let stored = queue.find_by_sha256("sha-quota-final").unwrap().unwrap();
        assert_eq!(stored.state, crate::upload_queue::QueueState::Held);
        assert_eq!(
            stored.last_error_code.as_deref(),
            Some("storage_quota_reached")
        );
        assert_eq!(stored.upload_id.as_deref(), Some("up-1"));
        assert_eq!(stored.offset, 8, "the bytes already accepted are kept");
        assert_eq!(queue.next_pending().unwrap(), None);
    }

    #[test]
    fn a_storage_quota_refusal_in_the_middle_of_a_transfer_holds_the_file_at_its_offset() {
        let (queue, entry, _guard, _queue_guard) =
            queue_with_file(b"0123456789ABCDEF", "sha-quota-mid");
        let transport = FakeTransport::default();
        transport
            .creates
            .borrow_mut()
            .push_back(Ok(session("up-1", 0, 16, 4, false)));
        transport
            .puts
            .borrow_mut()
            .push_back(Ok(PutOutcome::Advanced(session("up-1", 4, 16, 4, false))));
        transport
            .puts
            .borrow_mut()
            .push_back(Err(UploadError::refusal(
                409,
                "storage_quota_reached",
                "quota",
            )));

        let error = upload_entry(
            &transport,
            &queue,
            "test-token",
            &entry,
            &UploadTarget::default(),
            |_, _| {},
        )
        .unwrap_err();
        assert_eq!(error.code(), "storage_quota_reached");
        let stored = queue.find_by_sha256("sha-quota-mid").unwrap().unwrap();
        assert_eq!(stored.state, crate::upload_queue::QueueState::Held);
        assert_eq!(stored.offset, 4);
        assert_eq!(stored.upload_id.as_deref(), Some("up-1"));
    }

    #[test]
    fn a_finalize_that_keeps_saying_bytes_are_missing_is_given_up_not_asked_forever() {
        let (queue, entry, _guard, _queue_guard) = queue_with_file(b"01234567", "sha-incomplete");
        let transport = FakeTransport::default();
        transport
            .creates
            .borrow_mut()
            .push_back(Ok(session("up-1", 0, 8, 8, false)));
        // The first put, then one put and one "incomplete" per round.
        for _ in 0..=MAX_INCOMPLETE_ANSWERS {
            transport
                .puts
                .borrow_mut()
                .push_back(Ok(PutOutcome::Advanced(session("up-1", 8, 8, 8, false))));
            transport
                .finalizes
                .borrow_mut()
                .push_back(Ok(FinalizeOutcome::Incomplete { offset: 0 }));
        }

        let error = upload_entry(
            &transport,
            &queue,
            "test-token",
            &entry,
            &UploadTarget::default(),
            |_, _| {},
        )
        .unwrap_err();
        assert!(matches!(error, UploadError::GaveUp { .. }), "{error:?}");
        assert_eq!(
            queue
                .find_by_sha256("sha-incomplete")
                .unwrap()
                .unwrap()
                .state,
            crate::upload_queue::QueueState::Failed
        );
    }

    #[test]
    fn a_retry_later_failure_leaves_the_file_queued_and_never_marks_it() {
        for error in [
            UploadError::refusal(409, "upload_sessions_limit_reached", "too many"),
            UploadError::refusal(500, "internal_error", "boom"),
            UploadError::refusal(503, "service_starting", "starting"),
            UploadError::refusal(429, "rate_limited", "slow down"),
            UploadError::Transport("connection reset".to_string()),
            UploadError::Wait(ServiceWaitError::StartTimeout { waited_secs: 100 }),
        ] {
            let (queue, entry, _guard, _queue_guard) =
                queue_with_file(b"01234567", "sha-retry-later");
            let transport = FakeTransport::default();
            transport
                .creates
                .borrow_mut()
                .push_back(Err(err_clone(&error)));

            let returned = upload_entry(
                &transport,
                &queue,
                "test-token",
                &entry,
                &UploadTarget::default(),
                |_, _| {},
            )
            .unwrap_err();
            assert_eq!(returned.verdict(), Verdict::RetryLater, "{error:?}");
            let stored = queue.find_by_sha256("sha-retry-later").unwrap().unwrap();
            assert_eq!(
                stored.state,
                crate::upload_queue::QueueState::Queued,
                "{error:?}: not failed, not refused"
            );
            assert_eq!(stored.last_error_code, None);
            assert_eq!(
                queue.next_pending().unwrap().unwrap().sha256,
                "sha-retry-later"
            );
        }
    }

    /// A refusal nobody knows yet is never sent again: a `4xx` refuses the
    /// file with the generic code, a `5xx` is the service's failure.
    #[test]
    fn a_refusal_nobody_knows_yet_is_read_by_its_status_never_replayed() {
        let (queue, entry, _guard, _queue_guard) = queue_with_file(b"01234567", "sha-unknown");
        let transport = FakeTransport::default();
        transport
            .creates
            .borrow_mut()
            .push_back(Err(UploadError::refusal(
                422,
                "something_from_a_newer_service",
                "The English sentence of the service.",
            )));
        let error = upload_entry(
            &transport,
            &queue,
            "test-token",
            &entry,
            &UploadTarget::default(),
            |_, _| {},
        )
        .unwrap_err();
        assert_eq!(error.verdict(), Verdict::Refused);
        assert_eq!(error.code(), "upload_refused");
        assert!(
            !error.plain_message().contains("newer_service")
                && !error.plain_message().contains("English"),
            "never the raw code nor the service's own English: {}",
            error.plain_message()
        );
        let stored = queue.find_by_sha256("sha-unknown").unwrap().unwrap();
        assert_eq!(stored.state, crate::upload_queue::QueueState::Refused);
        assert_eq!(stored.last_error_code.as_deref(), Some("upload_refused"));
        assert_eq!(queue.next_pending().unwrap(), None);

        assert_eq!(
            UploadError::refusal(500, "internal_error", "x").code(),
            "service_error"
        );
        assert_eq!(
            UploadError::refusal(503, "service_starting", "x").code(),
            "service_starting"
        );
    }

    #[test]
    fn the_verdict_of_each_refusal_the_contract_names() {
        for (status, code, verdict) in [
            (415, "capture_content_refused", Verdict::Refused),
            (413, "file_too_large", Verdict::Refused),
            (413, "payload_too_large", Verdict::Refused),
            (409, "session_limit_reached", Verdict::Refused),
            (409, "session_event_limit_reached", Verdict::Refused),
            (400, "invalid_request", Verdict::Refused),
            (404, "plan_not_found", Verdict::Refused),
            (409, "capture_limit_reached", Verdict::NeedsPerson),
            (409, "storage_quota_reached", Verdict::NeedsPerson),
            (403, "entitlement_read_only", Verdict::NeedsPerson),
            (409, "upload_sessions_limit_reached", Verdict::RetryLater),
            (429, "anything", Verdict::RetryLater),
            (502, "anything", Verdict::RetryLater),
        ] {
            assert_eq!(
                UploadError::refusal(status, code, "m").verdict(),
                verdict,
                "{status} {code}"
            );
        }
    }

    /// The fourth capture of a free space is not a file refused for good --
    /// nothing is wrong with it -- but one the person makes room for: held
    /// with its session, never dropped from the queue.
    #[test]
    fn the_fourth_capture_of_a_free_space_is_held_not_dropped() {
        let error = UploadError::refusal(409, "capture_limit_reached", "m");
        assert_eq!(error.verdict(), Verdict::NeedsPerson);
        assert!(!error.is_terminal(), "the file is not finished with");
        assert_eq!(error.code(), "capture_limit_reached");
        // Whatever the status the service gives it, it is the same hold,
        // as the full storage is.
        assert_eq!(
            UploadError::refusal(403, "capture_limit_reached", "m").verdict(),
            Verdict::NeedsPerson
        );
        // Its sentence is the catalogue's plain English, not the service's.
        assert!(
            crate::refusal::plain_sentence(error.code())
                .is_some_and(|sentence| sentence.starts_with("Not added"))
        );
    }

    #[test]
    fn a_401_authentication_required_is_the_end_of_the_sign_in_not_a_refused_file() {
        let bare = Answered {
            status: 401,
            code: "authentication_required".to_string(),
            message: "no credential".to_string(),
            offset: None,
            retry_after_seconds: None,
        };
        assert!(matches!(
            bare.into_error(),
            UploadError::SignInEnded(SignInEnd::DeviceDetached)
        ));
        let other = Answered {
            status: 404,
            code: "plan_not_found".to_string(),
            message: "no plan".to_string(),
            offset: None,
            retry_after_seconds: None,
        };
        assert!(
            !other.upload_is_gone(),
            "a 404 of another code is a refusal of the attribution, not a lost upload"
        );
    }

    #[test]
    fn every_failure_has_a_plain_sentence_for_the_command_line() {
        for error in [
            UploadError::Transport("error sending request for url (http://x)".to_string()),
            UploadError::Wait(ServiceWaitError::StartTimeout { waited_secs: 100 }),
            UploadError::InvalidResponse("expected value at line 1".to_string()),
            UploadError::SessionNotFound("up-1".to_string()),
            UploadError::GaveUp {
                attempts: 2,
                reason: "invalid digest".to_string(),
            },
            UploadError::refusal(415, "capture_content_refused", "SVG"),
            UploadError::refusal(409, "storage_quota_reached", "quota"),
            UploadError::refusal(409, "upload_sessions_limit_reached", "many"),
            UploadError::refusal(413, "file_too_large", "large"),
            UploadError::refusal(413, "payload_too_large", "large"),
            UploadError::refusal(409, "session_limit_reached", "sessions"),
            UploadError::refusal(409, "session_event_limit_reached", "events"),
            UploadError::refusal(403, "entitlement_read_only", "read only"),
            UploadError::refusal(422, "brand_new", "The English sentence."),
            UploadError::refusal(500, "internal_error", "The English sentence."),
        ] {
            let text = error.plain_message();
            assert!(
                !text.contains("The English sentence")
                    && !text.contains("http://")
                    && !text.contains("expected value")
                    && !text.contains(" 4")
                    && !text.contains("brand_new"),
                "{error:?} -> {text}"
            );
            assert!(text.chars().count() > 30, "{error:?} -> {text}");
        }
    }

    #[test]
    fn a_dropped_connection_mid_transfer_leaves_the_entry_resumable_not_failed() {
        let (queue, entry, _guard, _queue_guard) =
            queue_with_file(b"0123456789ABCDEF", "sha-dropped");
        let transport = FakeTransport::default();
        transport
            .creates
            .borrow_mut()
            .push_back(Ok(session("up-1", 0, 16, 4, false)));
        transport
            .puts
            .borrow_mut()
            .push_back(Ok(PutOutcome::Advanced(session("up-1", 4, 16, 4, false))));
        // Simulated dropped connection on the second chunk.
        transport
            .puts
            .borrow_mut()
            .push_back(Err(UploadError::Transport("connection reset".to_string())));

        let error = upload_entry(
            &transport,
            &queue,
            "test-token",
            &entry,
            &UploadTarget::default(),
            |_, _| {},
        )
        .unwrap_err();
        assert!(matches!(error, UploadError::Transport(_)));

        let stored = queue.find_by_sha256("sha-dropped").unwrap().unwrap();
        assert_eq!(stored.state, crate::upload_queue::QueueState::Uploading);
        assert_eq!(stored.offset, 4, "the last durably-accepted offset is kept");

        // Resuming: a fresh call reconciles through `status`, not a blind
        // create, then finishes the transfer normally.
        let refreshed = queue.find_by_sha256("sha-dropped").unwrap().unwrap();
        transport
            .statuses
            .borrow_mut()
            .push_back(Ok(session("up-1", 4, 16, 4, false)));
        transport
            .puts
            .borrow_mut()
            .push_back(Ok(PutOutcome::Advanced(session("up-1", 8, 16, 4, false))));
        transport
            .puts
            .borrow_mut()
            .push_back(Ok(PutOutcome::Advanced(session("up-1", 12, 16, 4, false))));
        transport
            .puts
            .borrow_mut()
            .push_back(Ok(PutOutcome::Advanced(session("up-1", 16, 16, 4, false))));
        transport
            .finalizes
            .borrow_mut()
            .push_back(Ok(FinalizeOutcome::Completed(FinalizeResponse {
                capture_id: "capture-resumed".to_string(),
                deduplicated: false,
                ..Default::default()
            })));

        let outcome = upload_entry(
            &transport,
            &queue,
            "test-token",
            &refreshed,
            &UploadTarget::default(),
            |_, _| {},
        )
        .unwrap();
        assert_eq!(outcome.capture_id, "capture-resumed");
    }

    #[test]
    fn an_unknown_upload_id_at_resume_is_recreated_from_scratch() {
        let (queue, _entry, _guard, _queue_guard) =
            queue_with_file(b"0123456789ABCDEF", "sha-expired");
        // Pretend a previous run recorded a session that has since expired.
        queue.mark_progress("sha-expired", "up-expired", 4).unwrap();
        let entry = queue.find_by_sha256("sha-expired").unwrap().unwrap();

        let transport = FakeTransport::default();
        transport
            .statuses
            .borrow_mut()
            .push_back(Err(UploadError::SessionNotFound("up-expired".to_string())));
        transport
            .creates
            .borrow_mut()
            .push_back(Ok(session("up-new", 0, 16, 16, false)));
        transport
            .puts
            .borrow_mut()
            .push_back(Ok(PutOutcome::Advanced(session(
                "up-new", 16, 16, 16, false,
            ))));
        transport
            .finalizes
            .borrow_mut()
            .push_back(Ok(FinalizeOutcome::Completed(FinalizeResponse {
                capture_id: "capture-fresh".to_string(),
                deduplicated: false,
                ..Default::default()
            })));

        let outcome = upload_entry(
            &transport,
            &queue,
            "test-token",
            &entry,
            &UploadTarget::default(),
            |_, _| {},
        )
        .unwrap();
        assert_eq!(outcome.capture_id, "capture-fresh");
    }

    #[test]
    fn only_a_refusal_for_good_or_a_failed_transfer_is_terminal() {
        assert!(!UploadError::Transport("x".into()).is_terminal());
        // A wait that ended without an answer (gave up, cancelled, or a
        // write never confirmed) leaves the file exactly as progress
        // recorded it: resumable, never marked failed.
        for wait in [
            ServiceWaitError::StartTimeout { waited_secs: 100 },
            ServiceWaitError::Cancelled,
            ServiceWaitError::NotConfirmed,
        ] {
            assert!(!UploadError::Wait(wait).is_terminal());
        }
        // The service refuses the file for good.
        assert!(UploadError::refusal(413, "file_too_large", "x").is_terminal());
        assert!(UploadError::refusal(415, "capture_content_refused", "x").is_terminal());
        assert!(UploadError::refusal(400, "x", "y").is_terminal());
        // Waiting for the person, or for the service: the file is not done
        // with, and not marked.
        assert!(!UploadError::refusal(409, "storage_quota_reached", "x").is_terminal());
        assert!(!UploadError::refusal(409, "upload_sessions_limit_reached", "x").is_terminal());
        assert!(!UploadError::refusal(500, "x", "y").is_terminal());
        assert!(!UploadError::SignInEnded(SignInEnd::DeviceDetached).is_terminal());
        assert!(UploadError::SessionNotFound("x".into()).is_terminal());
        assert!(
            UploadError::GaveUp {
                attempts: 1,
                reason: "x".into()
            }
            .is_terminal()
        );
        let _ = err_clone(&UploadError::Transport("x".into()));
    }

    #[test]
    fn codes_are_stable_and_never_leak_technical_text_for_the_waiting_cases() {
        assert_eq!(UploadError::Transport("x".into()).code(), "network_error");
        assert_eq!(
            UploadError::Wait(ServiceWaitError::StartTimeout { waited_secs: 100 }).code(),
            "service_start_timeout"
        );
        assert_eq!(
            UploadError::Wait(ServiceWaitError::Cancelled).code(),
            "cancelled"
        );
        assert_eq!(
            UploadError::Wait(ServiceWaitError::NotConfirmed).code(),
            "service_write_unconfirmed"
        );
        assert_eq!(
            UploadError::GaveUp {
                attempts: 1,
                reason: "x".into()
            }
            .code(),
            "upload_error"
        );
        for code in crate::refusal::known_codes() {
            assert_eq!(UploadError::refusal(409, code, "y").code(), code);
        }
    }

    #[test]
    fn a_result_for_a_script_carries_the_code_and_a_plain_sentence() {
        let refused: Result<UploadOutcome, UploadError> =
            Err(UploadError::refusal(415, "capture_content_refused", "SVG"));
        let result = UploadEntryResult::from_outcome("abc", &refused);
        assert!(!result.ok);
        assert_eq!(result.code.as_deref(), Some("capture_content_refused"));
        assert!(result.error.as_deref().unwrap().contains("SVG or HTML"));
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["code"], "capture_content_refused");
        let sent = UploadEntryResult::sent("abc", "capture-1", false);
        assert!(sent.ok && sent.code.is_none() && sent.error.is_none());
        assert!(serde_json::to_value(&sent).unwrap().get("code").is_none());
    }

    /// A row that is not a result -- whatever put it there -- is refused
    /// before any call to the service.
    #[test]
    fn a_frame_never_leaves_even_from_a_row_the_queue_should_not_hold() {
        let queue = UploadQueue::open(&std::env::temp_dir().join(format!(
            "astro-sync-uploader-frame-{}.sqlite",
            std::process::id()
        )))
        .unwrap();
        let entry = QueueEntry {
            id: 1,
            source_path: PathBuf::from("/night/M31/lights/l-0001.fits"),
            sha256: "f".repeat(64),
            size: 10,
            upload_id: None,
            offset: 0,
            state: crate::upload_queue::QueueState::Queued,
            capture_id: None,
            last_error: None,
            last_error_code: None,
            created_at: String::new(),
            updated_at: String::new(),
            kind: crate::upload_queue::ResultKind::Final,
            target: crate::upload_queue::EntryTarget::default(),
        };
        // No call is scripted: any request would panic.
        let transport = FakeTransport::default();
        let error = upload_entry(
            &transport,
            &queue,
            "token",
            &entry,
            &UploadTarget::default(),
            |_, _| {},
        )
        .unwrap_err();
        assert_eq!(error.code(), "not_a_result");
        assert_eq!(error.verdict(), Verdict::Refused);
    }

    #[test]
    fn a_row_carries_its_night_its_targets_and_the_capture_it_is_a_variant_of() {
        let entry = QueueEntry {
            id: 1,
            source_path: PathBuf::from("/night/M31/stacked/m31.tif"),
            sha256: "a".repeat(64),
            size: 10,
            upload_id: None,
            offset: 0,
            state: crate::upload_queue::QueueState::Queued,
            capture_id: None,
            last_error: None,
            last_error_code: None,
            created_at: String::new(),
            updated_at: String::new(),
            kind: crate::upload_queue::ResultKind::Stacked,
            target: crate::upload_queue::EntryTarget {
                session_id: Some("night-1".into()),
                target_ids: vec!["m31".into()],
                variant_of: Some("capture-1".into()),
                source: Some("stacking".into()),
                metadata: None,
                frame_count: None,
                plan: None,
            },
        };
        let run = UploadTarget {
            session_id: Some("other".into()),
            plan_id: Some("plan-1".into()),
            target_ids: vec!["other".into()],
        };
        let metadata = serde_json::to_value(build_metadata(&entry, &run)).unwrap();
        assert_eq!(metadata["sessionId"], "night-1");
        assert_eq!(metadata["targetIds"], serde_json::json!(["m31"]));
        assert_eq!(metadata["captureId"], "capture-1");
        assert_eq!(metadata["source"], "stacking");
        assert_eq!(metadata["planId"], "plan-1");
        assert_eq!(metadata["originalName"], "m31.tif");
    }

    /// Two renders of one plan are one capture and one variant. The second
    /// leaves without the plan role (the service refuses a `captureId` with
    /// it) and with the capture the first one created.
    #[test]
    fn the_second_render_of_a_plan_is_a_variant_of_the_capture_of_the_first() {
        let plan_row = |variant_of: Option<&str>| QueueEntry {
            id: 1,
            source_path: PathBuf::from("/plan/IC1805/stacked/ic1805.tif"),
            sha256: "b".repeat(64),
            size: 10,
            upload_id: None,
            offset: 0,
            state: crate::upload_queue::QueueState::Queued,
            capture_id: None,
            last_error: None,
            last_error_code: None,
            created_at: String::new(),
            updated_at: String::new(),
            kind: crate::upload_queue::ResultKind::Plan,
            target: crate::upload_queue::EntryTarget {
                session_id: None,
                target_ids: vec!["ic1805".into()],
                variant_of: variant_of.map(str::to_string),
                source: Some("stacking".into()),
                metadata: None,
                frame_count: None,
                plan: Some(crate::upload_queue::PlanUpload {
                    project_id: "project-1".into(),
                    combined_night_ids: vec!["night-1".into()],
                    totals_by_filter: Vec::new(),
                    plan_render: Some("natural".into()),
                }),
            },
        };
        let head = serde_json::to_value(build_metadata(&plan_row(None), &UploadTarget::default()))
            .unwrap();
        assert_eq!(head["role"], "plan");
        assert_eq!(head["projectId"], "project-1");
        assert!(head.get("captureId").is_none_or(|v| v.is_null()));

        let digest = format!("{VARIANT_OF_SHA256}{}", "a".repeat(64));
        let variant = serde_json::to_value(build_metadata(
            &plan_row(Some(&digest)),
            &UploadTarget::default(),
        ))
        .unwrap();
        assert_eq!(variant["captureId"], digest.as_str());
        assert_eq!(variant["targetIds"], serde_json::json!(["ic1805"]));
        for key in ["role", "projectId", "combinedNightIds", "planRender"] {
            assert!(
                variant.get(key).is_none_or(|v| v.is_null()),
                "{key} must not leave with a variant"
            );
        }
    }

    /* ----------------------- The capture metadata --------------------------- */

    fn proposals() -> Proposals {
        Proposals {
            acquisition: vec![crate::metadata::AcquisitionLine {
                filter: Some("Ha".into()),
                frame_count: 12,
                exposure_seconds: 300.0,
                gain: Some(100),
            }],
            calibration: Some("30 darks · 30 flats".into()),
            processing: Some(crate::metadata::ProcessingProposal {
                software: Some("Astro Sync · Siril 1.4.4".into()),
                steps: Some("raw · 12 lights".into()),
            }),
            variant_processing: Some(crate::metadata::ProcessingProposal {
                software: Some("Astro Sync · Siril 1.4.4".into()),
                steps: Some("raw · contrast strong · 12 lights".into()),
            }),
            variant_generated_title: Some(crate::metadata::GeneratedText::from([
                ("en".to_string(), "Natural rendering".to_string()),
                ("fr".to_string(), "Rendu naturel".to_string()),
            ])),
            variant_generated_description: Some(crate::metadata::GeneratedText::from([(
                "en".to_string(),
                "The sensor's colours.".to_string(),
            )])),
            ..Proposals::default()
        }
    }

    /// A queue with one result carrying `metadata`, `variant_of` when it is
    /// a variant of a capture online.
    fn queue_with_proposals(
        sha256: &str,
        variant_of: Option<&str>,
        metadata: Option<Proposals>,
    ) -> (
        UploadQueue,
        QueueEntry,
        crate::test_support::TempDir,
        crate::test_support::TempDir,
    ) {
        let source_dir = crate::test_support::TempDir::new("uploader-proposals-source");
        let path = source_dir.path().join("M31_raw.tif");
        fs::write(&path, b"0123").unwrap();
        let queue_dir = crate::test_support::TempDir::new("uploader-proposals-queue");
        let queue = UploadQueue::open(&queue_dir.path().join("queue.sqlite")).unwrap();
        queue
            .enqueue_results(&[crate::upload_queue::ResultFile {
                path: path.clone(),
                sha256: sha256.to_string(),
                size: 4,
                kind: crate::upload_queue::ResultKind::Stacked,
                target: crate::upload_queue::EntryTarget {
                    session_id: Some("night-1".into()),
                    target_ids: vec!["t-m31".into()],
                    variant_of: variant_of.map(str::to_string),
                    source: Some("stacking".into()),
                    metadata,
                    frame_count: None,
                    plan: None,
                },
            }])
            .unwrap();
        let entry = queue.find_by_sha256(sha256).unwrap().unwrap();
        (queue, entry, source_dir, queue_dir)
    }

    fn script_a_complete_upload(transport: &FakeTransport, finalized: FinalizeResponse) {
        transport
            .creates
            .borrow_mut()
            .push_back(Ok(session("up-1", 0, 4, 4, false)));
        transport
            .puts
            .borrow_mut()
            .push_back(Ok(PutOutcome::Advanced(session("up-1", 4, 4, 4, false))));
        transport
            .finalizes
            .borrow_mut()
            .push_back(Ok(FinalizeOutcome::Completed(finalized)));
    }

    fn run(
        transport: &FakeTransport,
        queue: &UploadQueue,
        entry: &QueueEntry,
    ) -> Result<UploadOutcome, UploadError> {
        upload_entry(
            transport,
            queue,
            "test-token",
            entry,
            &UploadTarget::default(),
            |_, _| {},
        )
    }

    #[test]
    fn an_image_that_creates_a_capture_carries_every_proposal_in_its_create_step() {
        let (queue, entry, _a, _b) = queue_with_proposals("sha-new", None, Some(proposals()));
        let transport = FakeTransport::default();
        script_a_complete_upload(
            &transport,
            FinalizeResponse {
                capture_id: "capture-1".into(),
                skipped: vec![SkippedField {
                    field: "calibration".into(),
                    reason: "edited".into(),
                }],
                ..FinalizeResponse::default()
            },
        );

        let outcome = run(&transport, &queue, &entry).unwrap();

        let created = transport.created.borrow();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].metadata_source.as_deref(), Some("astro-sync"));
        assert_eq!(created[0].proposals, proposals());
        let json = serde_json::to_value(&created[0]).unwrap();
        assert_eq!(json["metadataSource"], "astro-sync");
        assert_eq!(json["acquisition"][0]["frameCount"], 12);
        assert_eq!(json["variantGeneratedTitle"]["en"], "Natural rendering");
        assert_eq!(json["variantGeneratedTitle"]["fr"], "Rendu naturel");
        assert!(
            json.get("variantTitle").is_none(),
            "never the owner's title"
        );
        // The service applied them at completion: nothing is proposed again,
        // and what it left is said.
        assert!(transport.captures_proposed.borrow().is_empty());
        assert!(transport.images_proposed.borrow().is_empty());
        assert_eq!(outcome.skipped.len(), 1);
        assert_eq!(
            crate::metadata::skipped_sentence(&outcome.skipped).as_deref(),
            Some("The values you entered in your Astro Portal space are kept.")
        );
        assert_eq!(outcome.proposals_refused, None);
    }

    #[test]
    fn an_image_without_proposals_writes_no_metadata_field() {
        let (queue, entry, _a, _b) = queue_with_proposals("sha-none", None, None);
        let transport = FakeTransport::default();
        script_a_complete_upload(
            &transport,
            FinalizeResponse {
                capture_id: "capture-1".into(),
                ..FinalizeResponse::default()
            },
        );
        run(&transport, &queue, &entry).unwrap();
        let json = serde_json::to_value(&transport.created.borrow()[0]).unwrap();
        for key in [
            "metadataSource",
            "acquisition",
            "calibration",
            "sensor",
            "processing",
            "variantTitle",
            "variantDescription",
            "variantGeneratedTitle",
            "variantGeneratedDescription",
            "variantProcessing",
            "imageQuality",
        ] {
            assert!(json.get(key).is_none(), "{key} must not be written");
        }
    }

    #[test]
    fn a_variant_carries_its_texts_and_the_capture_gets_its_part_after_the_fact() {
        let (queue, entry, _a, _b) =
            queue_with_proposals("sha-variant", Some("capture-1"), Some(proposals()));
        let transport = FakeTransport::default();
        script_a_complete_upload(
            &transport,
            FinalizeResponse {
                capture_id: "capture-1".into(),
                variant: Some(ImageRef {
                    id: "image-2".into(),
                }),
                ..FinalizeResponse::default()
            },
        );

        run(&transport, &queue, &entry).unwrap();

        // The create step: the image's title and description, nothing of the
        // capture's.
        let created = transport.created.borrow();
        assert_eq!(created[0].capture_id.as_deref(), Some("capture-1"));
        assert_eq!(created[0].proposals, proposals().image_part());
        // After the fact: the capture's part under the capture, once.
        let proposed = transport.captures_proposed.borrow();
        assert_eq!(proposed.len(), 1);
        assert_eq!(proposed[0].0, "capture-1");
        // ... without the steps of the variant's own render: the capture's
        // `processing` stays the main image's.
        assert_eq!(
            proposed[0].1,
            proposals().capture_part().without_processing()
        );
        assert_eq!(proposed[0].1.processing, None);
        assert!(transport.images_proposed.borrow().is_empty());
        // ... while the variant keeps the steps of its own render with it
        // (`variantProcessing`), which the capture shows once it is the main one.
        let json = serde_json::to_value(&created[0]).unwrap();
        assert_eq!(
            json["variantProcessing"]["steps"],
            "raw · contrast strong · 12 lights"
        );
        assert!(json.get("processing").is_none());
    }

    #[test]
    fn bytes_the_service_already_held_get_both_parts_after_the_fact() {
        let (queue, entry, _a, _b) = queue_with_proposals("sha-held", None, Some(proposals()));
        let transport = FakeTransport::default();
        script_a_complete_upload(
            &transport,
            FinalizeResponse {
                capture_id: "capture-7".into(),
                deduplicated: true,
                capture: Some(CaptureRef {
                    main_image_id: Some("image-main".into()),
                    sha256: Some("SHA-HELD".into()),
                }),
                ..FinalizeResponse::default()
            },
        );
        transport
            .capture_answers
            .borrow_mut()
            .push_back(Ok(vec![SkippedField {
                field: "acquisition".into(),
                reason: "deleted".into(),
            }]));

        let outcome = run(&transport, &queue, &entry).unwrap();

        assert!(outcome.deduplicated);
        let captures = transport.captures_proposed.borrow();
        assert_eq!(captures.len(), 1);
        assert_eq!(captures[0].1, proposals().capture_part());
        let images = transport.images_proposed.borrow();
        assert_eq!(images.len(), 1);
        assert_eq!(
            (images[0].0.as_str(), images[0].1.as_str()),
            ("capture-7", "image-main")
        );
        assert_eq!(images[0].2, proposals().image_part());
        assert_eq!(outcome.skipped[0].reason, "deleted");
    }

    fn measured() -> crate::metadata::ImageQuality {
        crate::metadata::ImageQuality {
            fwhm_pixels: 3.8,
            fwhm_arcsec: Some(11.79),
            eccentricity: Some(0.31),
            star_count: 212,
            measured_on: "linear".into(),
            measured_at: "2026-10-07T23:40:00Z".into(),
            tool: crate::metadata::QualityTool {
                name: "Astro Sync".into(),
                version: "0.1.0".into(),
            },
        }
    }

    #[test]
    fn the_quality_of_the_image_goes_with_it_and_a_missing_right_is_not_told() {
        let with_quality = Proposals {
            image_quality: Some(measured()),
            ..proposals()
        };
        // The create step of an image that creates a capture carries it.
        let (queue, entry, _a, _b) =
            queue_with_proposals("sha-q-new", None, Some(with_quality.clone()));
        let transport = FakeTransport::default();
        script_a_complete_upload(
            &transport,
            FinalizeResponse {
                capture_id: "capture-1".into(),
                // The space lost `analysis.logs`: the image is stored, the
                // quality dropped, and nothing alarming is said.
                skipped: vec![SkippedField {
                    field: "imageQuality".into(),
                    reason: "capability".into(),
                }],
                ..FinalizeResponse::default()
            },
        );
        let outcome = run(&transport, &queue, &entry).unwrap();
        let json = serde_json::to_value(&transport.created.borrow()[0]).unwrap();
        assert_eq!(json["imageQuality"]["fwhmPixels"], 3.8);
        assert_eq!(json["imageQuality"]["fwhmArcsec"], 11.79);
        assert_eq!(json["imageQuality"]["tool"]["name"], "Astro Sync");
        assert_eq!(outcome.skipped, []);
        let result = UploadEntryResult::from_outcome("sha-q-new", &Ok(outcome));
        assert!(result.ok && result.note.is_none() && result.skipped.is_empty());

        // An image the service already held: the quality goes by the
        // producers' image route, body alone valid, the same body twice.
        let (queue, entry, _a, _b) =
            queue_with_proposals("sha-q-held", None, Some(with_quality.clone()));
        let transport = FakeTransport::default();
        script_a_complete_upload(
            &transport,
            FinalizeResponse {
                capture_id: "capture-7".into(),
                deduplicated: true,
                capture: Some(CaptureRef {
                    main_image_id: Some("image-main".into()),
                    sha256: Some("SHA-Q-HELD".into()),
                }),
                ..FinalizeResponse::default()
            },
        );
        transport
            .image_answers
            .borrow_mut()
            .push_back(Ok(vec![SkippedField {
                field: "imageQuality".into(),
                reason: "capability".into(),
            }]));
        let outcome = run(&transport, &queue, &entry).unwrap();
        assert_eq!(outcome.skipped, []);
        assert_eq!(outcome.proposals_refused, None);
        let images = transport.images_proposed.borrow();
        assert_eq!(images[0].2.image_quality, Some(measured()));
        let body = image_proposed_body(&images[0].2);
        assert_eq!(body["imageQuality"]["starCount"], 212);
        assert_eq!(body["source"], "astro-sync");
        assert_eq!(body, image_proposed_body(&with_quality.image_part()));
        let only = Proposals {
            image_quality: Some(measured()),
            ..Proposals::default()
        };
        assert!(only.has_image_part() && !only.has_capture_part());
        let alone = image_proposed_body(&only);
        assert_eq!(alone.as_object().unwrap().len(), 2); // imageQuality and source
    }

    #[test]
    fn an_image_that_replaced_the_phones_photo_says_so_on_its_row() {
        let (queue, entry, _a, _b) = queue_with_proposals("sha-replaced", None, None);
        let transport = FakeTransport::default();
        script_a_complete_upload(
            &transport,
            FinalizeResponse {
                capture_id: "capture-1".into(),
                replaced: true,
                ..FinalizeResponse::default()
            },
        );
        let outcome = run(&transport, &queue, &entry).unwrap();
        assert!(outcome.replaced);
        let row = queue.find_by_sha256("sha-replaced").unwrap().unwrap();
        assert_eq!(row.state, crate::upload_queue::QueueState::Completed);
        assert_eq!(
            row.last_error_code.as_deref(),
            Some(crate::upload_queue::REPLACED_NOTE)
        );
        let result = UploadEntryResult::from_outcome("sha-replaced", &Ok(outcome));
        assert!(result.replaced);
        assert_eq!(serde_json::to_value(&result).unwrap()["replaced"], true);

        // An image that was added as a variant (`replaced` absent: false) says
        // nothing, and the field is not written.
        let (queue, entry, _a, _b) = queue_with_proposals("sha-added", None, None);
        let transport = FakeTransport::default();
        script_a_complete_upload(
            &transport,
            FinalizeResponse {
                capture_id: "capture-2".into(),
                ..FinalizeResponse::default()
            },
        );
        let outcome = run(&transport, &queue, &entry).unwrap();
        assert!(!outcome.replaced);
        let row = queue.find_by_sha256("sha-added").unwrap().unwrap();
        assert_eq!(row.last_error_code, None);
        let result = UploadEntryResult::from_outcome("sha-added", &Ok(outcome));
        assert!(
            serde_json::to_value(&result)
                .unwrap()
                .get("replaced")
                .is_none()
        );
        // The answer of the service is read as the contract writes it.
        let answered: FinalizeResponse =
            serde_json::from_str(r#"{"captureId":"c","deduplicated":false,"replaced":true}"#)
                .unwrap();
        assert!(answered.replaced);
    }

    #[test]
    fn a_session_the_service_already_completed_proposes_to_the_capture_it_names() {
        let (queue, entry, _a, _b) = queue_with_proposals("sha-done", None, Some(proposals()));
        let transport = FakeTransport::default();
        transport.creates.borrow_mut().push_back(Ok(UploadSession {
            completed: true,
            capture_id: Some("capture-existing".into()),
            ..session("up-1", 4, 4, 4, true)
        }));

        let outcome = run(&transport, &queue, &entry).unwrap();

        assert!(outcome.deduplicated);
        let captures = transport.captures_proposed.borrow();
        assert_eq!(captures[0].0, "capture-existing");
        // The image's id is not known on this path: only the capture's part.
        assert!(transport.images_proposed.borrow().is_empty());
    }

    #[test]
    fn a_field_a_device_may_not_write_never_fails_the_upload() {
        let (queue, entry, _a, _b) =
            queue_with_proposals("sha-403", Some("capture-1"), Some(proposals()));
        let transport = FakeTransport::default();
        script_a_complete_upload(
            &transport,
            FinalizeResponse {
                capture_id: "capture-1".into(),
                ..FinalizeResponse::default()
            },
        );
        transport
            .capture_answers
            .borrow_mut()
            .push_back(Err(UploadError::refusal(
                403,
                "web_session_required",
                "Only the web session writes this field.",
            )));

        let outcome = run(&transport, &queue, &entry).unwrap();

        // The image is there; the refusal is a code, and the file is done.
        assert_eq!(outcome.capture_id, "capture-1");
        assert_eq!(
            outcome.proposals_refused.as_deref(),
            Some("web_session_required")
        );
        let stored = queue.find_by_sha256("sha-403").unwrap().unwrap();
        assert_eq!(stored.state, crate::upload_queue::QueueState::Completed);
    }

    #[test]
    fn a_network_failure_of_the_proposals_leaves_the_file_to_be_proposed_again() {
        let (queue, entry, _a, _b) =
            queue_with_proposals("sha-net", Some("capture-1"), Some(proposals()));
        let transport = FakeTransport::default();
        script_a_complete_upload(
            &transport,
            FinalizeResponse {
                capture_id: "capture-1".into(),
                ..FinalizeResponse::default()
            },
        );
        transport
            .capture_answers
            .borrow_mut()
            .push_back(Err(UploadError::Transport("connection reset".into())));

        let error = run(&transport, &queue, &entry).unwrap_err();
        assert_eq!(error.verdict(), Verdict::RetryLater);
        let stored = queue.find_by_sha256("sha-net").unwrap().unwrap();
        assert_ne!(stored.state, crate::upload_queue::QueueState::Completed);

        // The next run finds the session complete and proposes again.
        transport.statuses.borrow_mut().push_back(Ok(UploadSession {
            completed: true,
            capture_id: Some("capture-1".into()),
            ..session("up-1", 4, 4, 4, true)
        }));
        let entry = queue.find_by_sha256("sha-net").unwrap().unwrap();
        let outcome = run(&transport, &queue, &entry).unwrap();
        assert_eq!(outcome.capture_id, "capture-1");
        assert_eq!(transport.captures_proposed.borrow().len(), 2);
        let stored = queue.find_by_sha256("sha-net").unwrap().unwrap();
        assert_eq!(stored.state, crate::upload_queue::QueueState::Completed);
    }

    #[test]
    fn the_proposals_of_a_row_survive_the_queue() {
        let (queue, _entry, _a, _b) =
            queue_with_proposals("sha-kept", Some("capture-1"), Some(proposals()));
        let stored = queue.find_by_sha256("sha-kept").unwrap().unwrap();
        assert_eq!(stored.target.metadata, Some(proposals()));
        let (queue, _entry, _a, _b) = queue_with_proposals("sha-none", None, None);
        assert_eq!(
            queue
                .find_by_sha256("sha-none")
                .unwrap()
                .unwrap()
                .target
                .metadata,
            None
        );
    }

    #[test]
    fn the_bodies_of_the_two_patches_name_the_producer_and_nothing_a_device_may_not_write() {
        let capture = capture_proposed_body(&proposals());
        assert_eq!(capture["source"], "astro-sync");
        assert_eq!(capture["calibration"], "30 darks · 30 flats");
        assert!(capture.get("variantGeneratedTitle").is_none());
        assert!(capture.get("title").is_none() && capture.get("notes").is_none());
        let image = image_proposed_body(&proposals());
        assert_eq!(
            image,
            serde_json::json!({
                "processing": {
                    "software": "Astro Sync · Siril 1.4.4",
                    "steps": "raw · contrast strong · 12 lights"
                },
                "generatedTitle": { "en": "Natural rendering", "fr": "Rendu naturel" },
                "generatedDescription": { "en": "The sensor's colours." },
                "source": "astro-sync"
            })
        );
    }

    #[test]
    fn the_images_already_online_are_proposed_to_by_their_ids() {
        use crate::precheck::{Decision, ImageCandidate, ImagePlan};
        let candidate = |sha: &str| ImageCandidate {
            render_rank: None,
            target_id: "t-m31".into(),
            path: PathBuf::from("/night/M31/stacked/M31_raw.tif"),
            sha256: sha.into(),
            size: 4,
            kind: crate::upload_queue::ResultKind::Stacked,
            render: None,
            contrast: crate::stacking::Contrast::None,
            retouch: Default::default(),
            frame_count: None,
            twin_sha256: None,
        };
        let plans = vec![
            ImagePlan {
                image: candidate("a"),
                decision: Decision::AlreadyOnline {
                    capture_id: "capture-3".into(),
                    image_id: "image-3".into(),
                },
                metadata: Some(proposals()),
                from_device: false,
            },
            // Nothing proposed, nothing sent; a new capture is not "online".
            ImagePlan {
                image: candidate("b"),
                decision: Decision::AlreadyOnline {
                    capture_id: "capture-4".into(),
                    image_id: "image-4".into(),
                },
                metadata: None,
                from_device: false,
            },
            ImagePlan {
                image: candidate("c"),
                decision: Decision::NewCapture,
                metadata: Some(proposals()),
                from_device: false,
            },
        ];
        let transport = FakeTransport::default();
        transport
            .image_answers
            .borrow_mut()
            .push_back(Ok(vec![SkippedField {
                field: "title".into(),
                reason: "edited".into(),
            }]));

        let answers = propose_for_online_images(&transport, "test-token", &plans).unwrap();

        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0].capture_id, "capture-3");
        assert_eq!(
            answers[0].note.as_deref(),
            Some("The values you entered in your Astro Portal space are kept.")
        );
        assert_eq!(transport.captures_proposed.borrow().len(), 1);
        assert_eq!(transport.images_proposed.borrow()[0].1, "image-3");
    }

    /// A multi-night plan of three nights, sent as on 03/11/2026: the image
    /// of the plan, then the progression image of each night (120 · 209 ·
    /// 309 frames). Each one names the capture of the plan (`variant_of`,
    /// resolved from the queue), its role and its frames, and no night: a
    /// night of the plan needs no capture of its own.
    fn plan_with_three_progressions() -> (UploadQueue, Vec<QueueEntry>, crate::test_support::TempDir)
    {
        use crate::upload_queue::{EntryTarget, PlanUpload, ResultFile, ResultKind};
        let root = crate::test_support::TempDir::new("uploader-plan-progression");
        let folder = root.path().join("Sh2-103 · 80340a");
        fs::create_dir_all(folder.join("stacked")).unwrap();
        fs::create_dir_all(folder.join("progression")).unwrap();
        let plan_file = folder.join("stacked/Sh2-103_plan_raw.tif");
        fs::write(&plan_file, b"plan").unwrap();
        let mut files = vec![ResultFile {
            path: plan_file,
            sha256: "sha-plan".into(),
            size: 4,
            kind: ResultKind::Plan,
            target: EntryTarget {
                plan: Some(PlanUpload {
                    project_id: "project-1".into(),
                    combined_night_ids: vec!["n1".into(), "n2".into(), "n3".into()],
                    ..PlanUpload::default()
                }),
                ..EntryTarget::default()
            },
        }];
        for (night, frames) in [(1, 120_u64), (2, 209), (3, 309)] {
            let path = folder.join(format!(
                "progression/Sh2-103_plan_night-0{night}_{frames}-frames.jpg"
            ));
            fs::write(&path, b"0123").unwrap();
            files.push(ResultFile {
                path,
                sha256: format!("sha-night-{night}"),
                size: 4,
                kind: ResultKind::Progression,
                target: EntryTarget {
                    variant_of: Some(format!("{VARIANT_OF_SHA256}sha-plan")),
                    frame_count: Some(frames),
                    ..EntryTarget::default()
                },
            });
        }
        let queue = UploadQueue::open(&root.path().join("queue.sqlite")).unwrap();
        queue.enqueue_results(&files).unwrap();
        queue.mark_completed("sha-plan", "capture-plan").unwrap();
        let entries = (1..=3)
            .map(|night| {
                queue
                    .find_by_sha256(&format!("sha-night-{night}"))
                    .unwrap()
                    .unwrap()
            })
            .collect();
        (queue, entries, root)
    }

    fn finalized_on(capture_id: &str) -> Result<FinalizeOutcome, UploadError> {
        Ok(FinalizeOutcome::Completed(FinalizeResponse {
            capture_id: capture_id.to_string(),
            ..Default::default()
        }))
    }

    #[test]
    fn every_progression_image_of_a_plan_names_the_capture_of_the_plan_and_its_frames() {
        let (queue, entries, _root) = plan_with_three_progressions();
        let transport = FakeTransport::default();
        for night in 1..=3 {
            let id = format!("up-{night}");
            transport
                .creates
                .borrow_mut()
                .push_back(Ok(session(&id, 0, 4, 4, false)));
            transport
                .puts
                .borrow_mut()
                .push_back(Ok(PutOutcome::Advanced(session(&id, 4, 4, 4, false))));
            transport
                .finalizes
                .borrow_mut()
                .push_back(finalized_on("capture-plan"));
        }
        for entry in &entries {
            let outcome = upload_entry(
                &transport,
                &queue,
                "test-token",
                entry,
                &UploadTarget::default(),
                |_, _| {},
            )
            .unwrap();
            assert_eq!(outcome.capture_id, "capture-plan");
        }
        let created = transport.created.borrow();
        let bodies: Vec<_> = created
            .iter()
            .map(|m| serde_json::to_value(m).unwrap())
            .collect();
        for (body, frames) in bodies.iter().zip([120, 209, 309]) {
            assert_eq!(body["captureId"], "capture-plan", "{body}");
            assert_eq!(body["role"], "progression");
            assert_eq!(body["progressionFrames"], frames);
            assert!(body.get("sessionId").is_none(), "no night: {body}");
            assert!(body.get("planId").is_none(), "{body}");
            assert!(body.get("projectId").is_none(), "{body}");
        }
    }

    #[test]
    fn a_finalize_whose_capture_is_gone_creates_again_and_is_never_refused() {
        let (queue, entries, _root) = plan_with_three_progressions();
        let transport = FakeTransport::default();
        transport
            .creates
            .borrow_mut()
            .push_back(Ok(session("up-old", 0, 4, 4, false)));
        transport
            .puts
            .borrow_mut()
            .push_back(Ok(PutOutcome::Advanced(session("up-old", 4, 4, 4, false))));
        transport
            .finalizes
            .borrow_mut()
            .push_back(Ok(FinalizeOutcome::CaptureGone));
        // The new create: the same upload, now aimed at the capture of the
        // plan, its bytes kept (nothing more to send).
        transport
            .creates
            .borrow_mut()
            .push_back(Ok(session("up-old", 4, 4, 4, false)));
        transport
            .finalizes
            .borrow_mut()
            .push_back(finalized_on("capture-plan"));

        let outcome = upload_entry(
            &transport,
            &queue,
            "test-token",
            &entries[0],
            &UploadTarget::default(),
            |_, _| {},
        )
        .unwrap();
        assert_eq!(outcome.capture_id, "capture-plan");
        let created = transport.created.borrow();
        assert_eq!(created.len(), 2);
        assert!(
            created
                .iter()
                .all(|m| m.capture_id.as_deref() == Some("capture-plan"))
        );
        let stored = queue.find_by_sha256("sha-night-1").unwrap().unwrap();
        assert_eq!(stored.state, crate::upload_queue::QueueState::Completed);
        assert_eq!(stored.capture_id.as_deref(), Some("capture-plan"));
    }

    #[test]
    fn a_capture_gone_again_after_a_new_create_ends_once_never_in_a_loop() {
        let (queue, entries, _root) = plan_with_three_progressions();
        let transport = FakeTransport::default();
        transport
            .creates
            .borrow_mut()
            .push_back(Ok(session("up-1", 4, 4, 4, false)));
        transport
            .finalizes
            .borrow_mut()
            .push_back(Ok(FinalizeOutcome::CaptureGone));
        transport
            .creates
            .borrow_mut()
            .push_back(Ok(session("up-1", 4, 4, 4, false)));
        transport
            .finalizes
            .borrow_mut()
            .push_back(Ok(FinalizeOutcome::CaptureGone));

        let error = upload_entry(
            &transport,
            &queue,
            "test-token",
            &entries[1],
            &UploadTarget::default(),
            |_, _| {},
        )
        .unwrap_err();
        assert!(matches!(error, UploadError::Refused { status: 409, .. }));
        assert_eq!(transport.created.borrow().len(), 2);
    }
}

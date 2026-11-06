//! The sky annotations of an image, right after it was uploaded (the
//! service never solves).
//!
//! The night wizard solves the images of a night in its "Sky annotations"
//! step; the detection flow and the plan images never did, so a
//! multi-night plan image reached the portal without any annotation. Every
//! upload of a stacked, plan or final image therefore ends, on this
//! computer, with [`annotate_uploaded`]:
//!
//! 1. the service is asked which captures still lack a solution
//!    ([`list_missing`]): the capture of the image that is not in the list
//!    is already solved (the night wizard did it, or the service already held
//!    the bytes) -- **nothing is solved twice** ([`SkyState::Already`]);
//! 2. the image just sent is solved from the file here, when it is the main
//!    image of its capture (the header first, then the solver the person
//!    chose: [`solve_and_send`]), else from the original fetched;
//! 3. what became of it is kept in the queue's database
//!    ([`SkyAnnotation`], sibling table `sky_annotation`): "sent",
//!    "unsolved" with the reason, "no solver" -- the "Uploads"
//!    page and the banner of the result read it, and "Try again" runs the
//!    same function again.
//!
//! The caller runs it **after** the file's upload, off the upload thread, so
//! the next file never waits for a solver. A progression image has no
//! annotations of its own ([`applies_to`]).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Instant;

use crate::gui_bridge::GuiBridgeError;
use crate::night_resume::OriginalTransport;
use crate::sky_solution::{
    SkySolutionTransport, SolutionSource, SolveFailure, SolveOutcome, SolveSetup, list_missing,
    solve_and_send,
};
use crate::upload_queue::{ResultKind, UploadQueue};

/// Where the annotations of one uploaded image stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkyState {
    /// Being solved here.
    Running,
    /// Solved on this computer and sent.
    Sent,
    /// The service already had them: nothing was solved.
    Already,
    /// No annotations: `failure` or `error_code` says why.
    Failed,
}

impl SkyState {
    pub(crate) fn as_sql(self) -> &'static str {
        match self {
            SkyState::Running => "running",
            SkyState::Sent => "sent",
            SkyState::Already => "already",
            SkyState::Failed => "failed",
        }
    }

    pub(crate) fn from_sql(value: &str) -> Self {
        match value {
            "running" => SkyState::Running,
            "sent" => SkyState::Sent,
            "already" => SkyState::Already,
            _ => SkyState::Failed,
        }
    }
}

/// The code a resolution left `running` by an application that stopped is
/// ended with ([`UploadQueue::end_interrupted_sky`]).
pub const INTERRUPTED: &str = "sky_interrupted";

/// The annotations of one uploaded image, kept with its row of the queue
/// (`camelCase` on the wire: the event `astro-sync://sky-event` and
/// `queue_status` carry it as it is).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkyAnnotation {
    pub sha256: String,
    pub capture_id: Option<String>,
    pub state: SkyState,
    /// Why the solver gave nothing (tagged by `code`), when it did not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<SolveFailure>,
    /// Why the service could not be asked or told (`network_error`, ...): the
    /// window looks it up as `errors.<code>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    /// The core's own words for either, French, for a code the window has no
    /// sentence for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// How long the resolution took.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    /// What gave the solution: the header of the image, ASTAP, astrometry.net.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SolutionSource>,
    pub updated_at: String,
}

impl SkyAnnotation {
    fn new(sha256: &str, capture_id: &str, state: SkyState, now: &str) -> Self {
        Self {
            sha256: sha256.to_string(),
            capture_id: Some(capture_id.to_string()),
            state,
            failure: None,
            error_code: None,
            message: None,
            elapsed_ms: None,
            source: None,
            updated_at: now.to_string(),
        }
    }

    /// Whether the person can try again: the annotations are not there.
    #[must_use]
    pub fn retryable(&self) -> bool {
        self.state == SkyState::Failed
    }
}

/// What was just uploaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadedImage {
    pub sha256: String,
    pub capture_id: String,
    /// The file that was sent, where it was on this computer.
    pub path: PathBuf,
    pub kind: ResultKind,
}

/// Whether the annotations are made for a result of this kind: a progression
/// image is the history of a capture, not one of its images.
#[must_use]
pub fn applies_to(kind: ResultKind) -> bool {
    kind != ResultKind::Progression
}

/// Solves the image just sent and sends its solution, once. Every state it
/// passes through is written in `queue` and given to `on_change` (the event
/// of the window); the last one is returned. See the module documentation.
pub fn annotate_uploaded<T: SkySolutionTransport + OriginalTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
    queue: &UploadQueue,
    image: &UploadedImage,
    setup: &SolveSetup<'_>,
    on_change: &mut dyn FnMut(&SkyAnnotation),
) -> SkyAnnotation {
    let mut write = |annotation: SkyAnnotation| {
        // A queue that cannot be written still lets the resolution go on: the
        // window has the event.
        let _ = queue.set_sky(&annotation);
        on_change(&annotation);
        annotation
    };
    let state =
        |kind: SkyState| SkyAnnotation::new(&image.sha256, &image.capture_id, kind, setup.now);
    write(state(SkyState::Running));
    let started = Instant::now();
    let elapsed_ms = || u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let failed_service = |error: &GuiBridgeError| SkyAnnotation {
        error_code: Some(error.code().to_string()),
        message: Some(error.to_string()),
        elapsed_ms: Some(elapsed_ms()),
        ..state(SkyState::Failed)
    };

    let missing = match list_missing(transport, base_url, token) {
        Ok(missing) => missing,
        Err(error) => return write(failed_service(&error)),
    };
    // Not in the list: the service already holds a solution for it.
    let Some(capture) = missing.iter().find(|c| c.id == image.capture_id) else {
        return write(SkyAnnotation {
            elapsed_ms: Some(elapsed_ms()),
            ..state(SkyState::Already)
        });
    };
    // The file just sent is solved where it is, when it is the capture's main
    // image; any other case is solved from the original, fetched.
    let mut here = std::collections::BTreeMap::new();
    if image.path.is_file() {
        here.insert(image.sha256.to_ascii_lowercase(), image.path.clone());
    }
    let source = capture.solve_from(&here);
    match solve_and_send(transport, base_url, token, &source, capture.hints(), setup) {
        Ok(SolveOutcome::Sent { solution, .. }) => write(SkyAnnotation {
            source: Some(solution.source),
            elapsed_ms: Some(elapsed_ms()),
            ..state(SkyState::Sent)
        }),
        Ok(SolveOutcome::Failed {
            failure, message, ..
        }) => write(SkyAnnotation {
            failure: Some(failure),
            message: Some(message),
            elapsed_ms: Some(elapsed_ms()),
            ..state(SkyState::Failed)
        }),
        Err(error) => write(failed_service(&error)),
    }
}

#[cfg(test)]
mod tests;

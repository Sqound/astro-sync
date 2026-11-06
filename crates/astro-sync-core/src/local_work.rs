//! Local work that takes a while -- the walk and the hashes of a night's
//! folder, the detection, the analysis, Siril, the stacking -- runs off the
//! window's thread, says how far it is and can be stopped (otherwise the window can freeze
//! on a real folder).
//! This module holds the two pieces every such work shares: the stop switch
//! ([`CancelFlag`]) and the progress it reports ([`WorkProgress`]). The
//! thread itself is the caller's (the Tauri commands run the work on a
//! blocking thread of their own; the command line runs it in place).

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The stop switch of a local work. Cloned freely: every clone is the same
/// switch. A work checks it between files and between chunks of a file, so
/// it stops within one chunk (64 KiB) of the request.
#[derive(Debug, Clone, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks every work holding this switch to stop.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// Arms the switch again before a new work starts.
    pub fn reset(&self) {
        self.0.store(false, Ordering::SeqCst);
    }

    /// How many clones of this switch exist: a registry that keeps one can
    /// tell a work still holding its switch from one that has finished.
    #[must_use]
    pub fn holders(&self) -> usize {
        Arc::strong_count(&self.0)
    }
}

/// Which local work a [`WorkProgress`] is about, or which one a stop is
/// asked for ("Stop the stacking" stops the stacking and nothing else).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkKind {
    /// Walking a folder and hashing its files.
    Scan,
    /// Reading the first header of each frame (detection).
    Headers,
    /// Reading the logs and the headers for the night's analysis.
    Analysis,
    /// The stacking of a target (Siril behind the script). Reports through
    /// its own event; named here so it can be stopped on its own.
    Stacking,
    /// The progression images of a target (the same remark).
    Progression,
    /// "Tidy the files" of "Detect sessions": copied, moved
    /// or linked, file by file (`crate::session_detection::organise`).
    Organise,
    /// "Measure the frames": the script measuring each light frame, one
    /// batch after the other.
    PoseStats,
}

/// How far a local work is: `done` of `total` units (files), the bytes
/// behind them, and the file being read. Sent to the window as an event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkProgress {
    pub kind: WorkKind,
    pub done: u64,
    pub total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    /// Forward-slash path, relative to the folder being read (the reading of
    /// headers of "Detect sessions": the whole path of the file).
    pub current: Option<String>,
}

/// Where a work reports its progress. `&dyn Fn` so a caller can hand in a
/// closure that emits an event, collects for a test, or does nothing.
pub type ProgressSink<'a> = &'a (dyn Fn(&WorkProgress) + Sync);

/// A sink that drops every report.
pub fn no_progress(_: &WorkProgress) {}

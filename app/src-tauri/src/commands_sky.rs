//! The sky annotations after an upload: once a stacked, plan or final image
//! is uploaded, it is solved here and its solution sent
//! ([`astro_sync_core::sky_after_upload`]), off the upload thread so the next
//! file never waits for a solver. The result is kept beside the queue's row
//! and told to the window by [`SKY_EVENT`]; « Réessayer » is [`sky_retry`].

use std::sync::{Mutex, MutexGuard, PoisonError};

use tauri::{AppHandle, Emitter, Manager, Runtime};

use astro_sync_core::sky_after_upload::{self, SkyAnnotation, UploadedImage};
use astro_sync_core::sky_solution::{self, SolveSetup, SolverChoice};
use astro_sync_core::upload_queue::{QueueState, UploadQueue};

use crate::commands::require_portal_and_token;
use crate::commands_night::{solve_setup_parts, transport};
use crate::error::CommandError;
use crate::paths;

/// The event of the annotations of an uploaded image
/// (`astro_sync_core::sky_after_upload::SkyAnnotation`, `camelCase`).
pub const SKY_EVENT: &str = "astro-sync://sky-event";

/// Managed state that turns the hook on: the application manages it, a test
/// that does not exercise the annotations does not (no thread of a test is
/// left solving in the shared configuration directory).
#[derive(Debug, Clone, Copy, Default)]
pub struct SkyAfterUpload;

/// One resolution at a time: ASTAP takes the computer's processor, and the
/// night wizard's own « Annotations du ciel » step takes the same lock, so an
/// image is never solved by two of them at once (the second finds it
/// already solved).
static SOLVING: Mutex<()> = Mutex::new(());

pub(crate) fn solving_lock() -> MutexGuard<'static, ()> {
    SOLVING.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Solves the image of the queue row `sha256` and sends the solution, on a
/// thread of its own. Does nothing for a row that is not an uploaded result
/// the annotations are made for.
pub(crate) fn spawn_annotation<R: Runtime>(
    app: &AppHandle<R>,
    sha256: String,
    portal_url: String,
    token: String,
) {
    if app.try_state::<SkyAfterUpload>().is_none() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        // Nothing to tell anyone when the work cannot even start (the file is
        // not in the queue): the upload itself was told.
        let _ = annotate(&app, &sha256, &portal_url, &token);
    });
}

fn annotate<R: Runtime>(
    app: &AppHandle<R>,
    sha256: &str,
    portal_url: &str,
    token: &str,
) -> Result<(), CommandError> {
    let paths = paths::resolve(app)?;
    let queue = paths::open_queue(&paths)?;
    let Some(entry) = queue.find_by_sha256(sha256)? else {
        return Ok(());
    };
    let Some(capture_id) = entry
        .capture_id
        .clone()
        .filter(|_| entry.state == QueueState::Completed)
    else {
        return Ok(());
    };
    if !sky_after_upload::applies_to(entry.kind) {
        return Ok(());
    }
    let transport = transport(app)?;
    let (solvers, data) = solve_setup_parts(&paths);
    let image = UploadedImage {
        sha256: entry.sha256.clone(),
        capture_id,
        path: entry.source_path.clone(),
        kind: entry.kind,
    };
    let _one_at_a_time = solving_lock();
    let now = astro_sync_core::now_iso8601_utc();
    let setup = SolveSetup {
        solvers: &solvers,
        choice: SolverChoice::default(),
        originals_dir: &data.originals,
        work_root: &data.solve,
        timeout: sky_solution::DEFAULT_SOLVE_TIMEOUT,
        cancel: None,
        now: &now,
    };
    sky_after_upload::annotate_uploaded(
        &transport,
        portal_url,
        token,
        &queue,
        &image,
        &setup,
        &mut |annotation| {
            let _ = app.emit(SKY_EVENT, annotation);
        },
    );
    Ok(())
}

/// « Réessayer » of an uploaded image whose annotations are not there:
/// solves and sends again, in the background. The answer is the
/// [`SKY_EVENT`] that follows.
#[tauri::command]
pub fn sky_retry<R: Runtime>(app: AppHandle<R>, sha256: String) -> Result<(), CommandError> {
    let paths = paths::resolve(&app)?;
    let queue = paths::open_queue(&paths)?;
    if queue.find_by_sha256(&sha256)?.is_none() {
        return Err(CommandError::new(
            "queue_error",
            "This file is no longer in the upload list.",
        ));
    }
    // Said at once, before the thread has started.
    if let Some(mut kept) = queue.sky_of(&sha256)? {
        kept.state = sky_after_upload::SkyState::Running;
        kept.failure = None;
        kept.error_code = None;
        kept.message = None;
        let _ = queue.set_sky(&kept);
        let _ = app.emit(SKY_EVENT, &kept);
    }
    let (portal_url, token) = require_portal_and_token(&app)?;
    spawn_annotation(&app, sha256, portal_url, token);
    Ok(())
}

/// The resolutions an application that stopped left `running` are ended once,
/// when it starts (see `UploadQueue::end_interrupted_sky`).
pub(crate) fn end_interrupted<R: Runtime>(app: &AppHandle<R>) {
    if let Ok(paths) = paths::resolve(app) {
        if let Ok(queue) = paths::open_queue(&paths) {
            let _ = queue.end_interrupted_sky();
        }
    }
}

/// The annotations kept, for the window's first read.
pub(crate) fn kept(queue: &UploadQueue) -> Vec<SkyAnnotation> {
    queue.sky_annotations().unwrap_or_default()
}

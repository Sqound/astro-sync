//! Resolves [`AppPaths`] from Tauri's own app-config directory, creating it
//! if needed. Kept separate from `state.rs` (the plain data) and
//! `commands.rs` (the command surface) so the one Tauri-specific
//! filesystem decision -- *where* the GUI's own files live -- has a single
//! home.

use astro_sync_core::upload_queue::UploadQueue;
use tauri::{AppHandle, Manager, Runtime};

use crate::error::CommandError;
use crate::state::AppPaths;

pub fn resolve<R: Runtime>(app: &AppHandle<R>) -> Result<AppPaths, CommandError> {
    let base = app
        .path()
        .app_config_dir()
        .map_err(|error| CommandError::other(error.to_string()))?;
    std::fs::create_dir_all(&base).map_err(|error| CommandError::other(error.to_string()))?;
    Ok(AppPaths {
        settings: base.join("settings.json"),
        device_token: base.join("device-token"),
        queue: base.join("upload-queue.sqlite"),
        capabilities_cache: base.join("capabilities-cache.json"),
        space_record: base.join("space.json"),
        installation_id: base.join("installation-id"),
        error_reports: base.join("error-reports.json"),
        diagnostic_id: base.join("diagnostic-id"),
        reference_stars: base.join("reference-stars.bin"),
        digest_cache: base.join("digest-cache.sqlite"),
        local_settings: base.join("local-settings.json"),
        data_dir: base,
    })
}

/// The space the queue is seen from: `space.json` › `link.space.id`, or `None`
/// when this computer is not linked to a space by e-mail (a paired computer:
/// the rows without a space). A record that cannot be read counts as none.
pub fn linked_space_id(paths: &AppPaths) -> Option<String> {
    astro_sync_core::space::load_record(&paths.space_record)
        .ok()
        .and_then(|record| record.link)
        .map(|link| link.space.id)
}

/// Opens the upload queue **seen from the linked space**: its rows, its
/// history, its « already known », its sky annotations
/// ([`UploadQueue::for_space`]). Every command of the window opens it here.
pub fn open_queue(paths: &AppPaths) -> Result<UploadQueue, CommandError> {
    Ok(UploadQueue::open(&paths.queue)?.for_space(linked_space_id(paths)))
}

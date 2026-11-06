//! « Liste des incidents »: what Astro Sync detects in
//! a night's files, the signalements the session already holds, and « Ajouter à
//! la session » (`astro_sync_core::incidents`). Thin calls into the core; the
//! window merges, pairs and chooses, the core detects and writes.

use std::path::Path;

use tauri::{AppHandle, Runtime};

use astro_sync_core::analysis::log_offset::ForcedLogOffsets;
use astro_sync_core::detection::NightWindow;
use astro_sync_core::incidents::{
    self, Apply, ApplyError, ApplyOutcome, DeclaredEvents, DetectInput, Detection,
};

use crate::commands::{ending_sign_in_on_error, require_portal_and_token, run_blocking};
use crate::commands_night::transport;
use crate::error::CommandError;

/// What the night's files say, with no network: the incidents of the target's
/// poses and logs, and what is detected but not proposed.
#[tauri::command]
pub async fn incidents_detect(
    night_folder: String,
    target_folder: Option<String>,
    evening_date: String,
    longitude: Option<f64>,
) -> Result<Detection, CommandError> {
    run_blocking(move || {
        Ok(incidents::detect_incidents(&DetectInput {
            night_folder: Path::new(&night_folder).to_path_buf(),
            target_folder,
            window: NightWindow::of_evening(&evening_date, longitude),
            log_offsets: ForcedLogOffsets::default(),
        }))
    })
    .await
}

/// The signalements the session holds (`GET /v1/sessions/{id}/events`), the
/// ones a rework hid left out.
#[tauri::command]
pub async fn incidents_declared<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
) -> Result<DeclaredEvents, CommandError> {
    run_blocking(move || {
        let (portal_url, token) = require_portal_and_token(&app)?;
        let result =
            incidents::read_declared_events(&transport(&app)?, &portal_url, &token, &session_id)
                .map_err(CommandError::from);
        ending_sign_in_on_error(&app, result)
    })
    .await
}

/// « Ajouter N incidents à la session »: the additions (idempotent), then the
/// replacements (all or none). A refused rework is an answer
/// (`replacementsRefused`), never an error.
#[tauri::command]
pub async fn incidents_apply<R: Runtime>(
    app: AppHandle<R>,
    request: Apply,
) -> Result<ApplyOutcome, CommandError> {
    run_blocking(move || {
        let (portal_url, token) = require_portal_and_token(&app)?;
        let result =
            incidents::apply(&transport(&app)?, &portal_url, &token, &request).map_err(|failure| {
                match failure {
                    ApplyError::Invalid(reason) => CommandError::new("invalid_request", reason),
                    ApplyError::Service { source, partial } => {
                        let mut error = CommandError::from(source);
                        error.details = serde_json::to_value(&*partial).ok();
                        error
                    }
                }
            });
        ending_sign_in_on_error(&app, result)
    })
    .await
}

/// The `eventId` each detection key takes in the session (derived from the
/// session and the key): a detection whose event is already in the session was
/// added before and is not proposed again.
#[tauri::command]
pub fn incidents_event_ids(session_id: String, keys: Vec<String>) -> Vec<String> {
    keys.iter()
        .map(|key| incidents::event_id_for(&session_id, key))
        .collect()
}

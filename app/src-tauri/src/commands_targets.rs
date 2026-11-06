//! The commands of the step « Cibles »: the catalogue searched in the
//! application, a capture's FITS read for its target (its header only, on
//! this computer), a new target outside the catalogue, the night's targets
//! changed. Each is a thin call into `astro_sync_core::catalogue_targets`,
//! the same calls `astro-sync targets`, `night-targets`, `fits-target` and
//! `personal-target` make on the command line.

use std::path::PathBuf;

use tauri::{AppHandle, Runtime};
use tauri_plugin_dialog::DialogExt;

use astro_sync_core::catalogue_targets::{
    self, FitsGuess, FitsGuessFailure, NightTargetChange, PersonalTargetCreate,
    PersonalTargetCreated, PersonalTargetFailure, TargetSearch,
};
use astro_sync_core::gui_bridge::HttpPortalGuiTransport;

use crate::commands::{
    ending_sign_in_on_error, require_portal_and_token, run_blocking, service_wait,
};
use crate::dialogs;
use crate::error::CommandError;
use crate::paths;

fn transport<R: Runtime>(app: &AppHandle<R>) -> Result<HttpPortalGuiTransport, CommandError> {
    let (waiting, hooks) = service_wait(app);
    Ok(HttpPortalGuiTransport::with_options(
        waiting.patient,
        hooks,
    )?)
}

/// « Rechercher une cible »: `limit` cards of the catalogue from `offset`
/// (`sort=planner`, the text normalised first).
#[tauri::command]
pub async fn targets_search<R: Runtime>(
    app: AppHandle<R>,
    text: String,
    locale: String,
    offset: u32,
    limit: u32,
) -> Result<TargetSearch, CommandError> {
    run_blocking(move || {
        let (portal_url, token) = require_portal_and_token(&app)?;
        let result = catalogue_targets::search_targets(
            &transport(&app)?,
            &portal_url,
            Some(&token),
            &text,
            &locale,
            offset,
            limit,
        )
        .map_err(CommandError::from);
        ending_sign_in_on_error(&app, result)
    })
    .await
}

/// « Ajouter NGC 7000 à la nuit », or « Retirer » a target added in this
/// opening of the wizard: the night's targets afterwards.
#[tauri::command]
pub async fn night_targets_change<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
    change: NightTargetChange,
) -> Result<Vec<String>, CommandError> {
    run_blocking(move || {
        let (portal_url, token) = require_portal_and_token(&app)?;
        let result = catalogue_targets::change_night_targets(
            &transport(&app)?,
            &portal_url,
            &token,
            &session_id,
            &change,
        )
        .map_err(CommandError::from);
        ending_sign_in_on_error(&app, result)
    })
    .await
}

/// The system's file picker, for a capture's frame (FITS or XISF). `None`:
/// dismissed. Nothing is read here.
///
/// An `async fn` like `choose_folder`: the dialog must never be waited
/// for on the main thread. See [`crate::dialogs`].
#[tauri::command]
pub async fn fits_choose<R: Runtime>(app: AppHandle<R>) -> Result<Option<String>, CommandError> {
    dialogs::pick(|answer| {
        app.dialog()
            .file()
            .add_filter("FITS, XISF", &["fit", "fits", "fts", "xisf"])
            .pick_file(answer)
    })
    .await
}

/// « Trouver la cible avec un FITS »: the header of `file` read on this computer, the target
/// it names or the targets of its field (nothing of the file is sent).
#[tauri::command]
pub async fn fits_target_guess<R: Runtime>(
    app: AppHandle<R>,
    file: String,
    locale: String,
) -> Result<FitsGuess, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let (portal_url, token) = require_portal_and_token(&app)?;
        let result = catalogue_targets::guess_fits_target(
            &transport(&app)?,
            &portal_url,
            Some(&token),
            &PathBuf::from(file),
            &locale,
            &paths.data_dir.join(catalogue_targets::INDEX_FILE),
        )
        .map_err(|failure| match failure {
            FitsGuessFailure::Service(error) => CommandError::from(error),
            FitsGuessFailure::Unreadable(detail) => CommandError::new("fits_unreadable", detail),
        });
        ending_sign_in_on_error(&app, result)
    })
    .await
}

/// « Créer et ajouter à la nuit »: a personal target of the space, then
/// added to the night.
#[tauri::command]
pub async fn personal_target_create<R: Runtime>(
    app: AppHandle<R>,
    target: PersonalTargetCreate,
    session_id: String,
) -> Result<PersonalTargetCreated, CommandError> {
    run_blocking(move || {
        let (portal_url, token) = require_portal_and_token(&app)?;
        let result = catalogue_targets::create_personal_target(
            &transport(&app)?,
            &portal_url,
            &token,
            &target,
            Some(&session_id),
        )
        .map_err(|failure| match failure {
            PersonalTargetFailure::Service(error) => CommandError::from(error),
            PersonalTargetFailure::Invalid(error) => {
                CommandError::new("invalid_target", error.to_string())
            }
        });
        ending_sign_in_on_error(&app, result)
    })
    .await
}

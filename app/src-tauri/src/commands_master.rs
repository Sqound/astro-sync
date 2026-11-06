//! « Mesurer un master… »: the quality of an image (FWHM, eccentricity,
//! stars) measured on a linear master a person stacked elsewhere
//! (PixInsight, Siril), by the code that measures the stacking of Astro Sync
//! itself. Thin calls into `astro_sync_core::master_quality`, the file
//! picker being `fits_choose` (FITS and XISF).

use std::path::PathBuf;

use tauri::{AppHandle, Runtime};

use astro_sync_core::master_quality::{self, Failure, Outcome};
use astro_sync_core::python_env;
use astro_sync_core::working_folder::NightLocation;

use crate::commands::run_blocking;
use crate::commands_night::{capabilities_cached, locate, stacking_dir};
use crate::error::CommandError;
use crate::paths;
use crate::state::AppPaths;

fn night_folder(paths: &AppPaths, session_id: &str) -> Result<PathBuf, CommandError> {
    match locate(paths, session_id)? {
        NightLocation::Found { folder } => Ok(folder),
        _ => Err(CommandError::new(
            "night_folder_missing",
            "The night folder cannot be found.",
        )),
    }
}

/// « Choisir le fichier » then the measure: the master `file` measured and
/// its figures kept in the folder of the target `folder_name` of the night,
/// to leave with the target's images (`imageQuality`, under `analysis.logs`).
/// A master that cannot be measured comes back as `refused` with a word
/// (`stretched`, `too_few_stars`, `reserved`...), never as an error.
#[tauri::command]
pub async fn master_measure<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
    folder_name: String,
    file: String,
) -> Result<Outcome, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let night = night_folder(&paths, &session_id)?;
        let entitled = capabilities_cached(&paths)?.is_some_and(|c| c.analysis_logs);
        let script = stacking_dir(&app)
            .map(|dir| dir.join("star_quality.py"))
            .unwrap_or_default();
        master_quality::measure_for_target(
            &night,
            &folder_name,
            &PathBuf::from(file),
            entitled,
            &python_env::env_python(&paths.data_dir),
            &script,
            None,
        )
        .map_err(|failure| match failure {
            Failure::NotRun(reason) => CommandError::new("master_not_measured", reason),
            Failure::NoAnswer(reason) => CommandError::new("master_no_answer", reason),
        })
    })
    .await
}

/// What is kept for the target (`null`: no master measured yet).
#[tauri::command]
pub async fn master_measure_current<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
    folder_name: String,
) -> Result<Option<Outcome>, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let night = night_folder(&paths, &session_id)?;
        Ok(master_quality::current(&night, &folder_name))
    })
    .await
}

/// « Retirer la mesure »: forgets the figures kept for the target; its images
/// leave without a quality again.
#[tauri::command]
pub async fn master_measure_forget<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
    folder_name: String,
) -> Result<(), CommandError> {
    run_blocking(move || {
        if !master_quality::is_folder_name(&folder_name) {
            return Ok(());
        }
        let paths = paths::resolve(&app)?;
        let night = night_folder(&paths, &session_id)?;
        master_quality::forget(&night, &folder_name)
            .map_err(|e| CommandError::new("io_error", e.to_string()))
    })
    .await
}

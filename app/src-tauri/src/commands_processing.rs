//! « Fichiers de traitement » of Réglages › Général: what the pipelines
//! produced in the working folder, measured, and cleaned on the person's
//! request -- never the person's own files (see
//! `astro_sync_core::processing_files`).

use tauri::{AppHandle, Manager, Runtime};

use astro_sync_core::local_work::WorkKind;
use astro_sync_core::processing_files::{self, CleanKind, CleanOutcome, ProcessingScan};

use crate::commands::{local_work, run_blocking};
use crate::commands_night::{install_python_env, root_of, settings, EnvAnswer};
use crate::error::CommandError;
use crate::paths;
use crate::state::LocalWorkControl;

/// A stacking, a plan image or the progression images write in the places
/// this cleans: nothing is cleaned (or reinstalled) while one runs.
fn refuse_while_working<R: Runtime>(app: &AppHandle<R>) -> Result<(), CommandError> {
    let control = app.state::<LocalWorkControl>();
    if control.is_running(WorkKind::Stacking) || control.is_running(WorkKind::Progression) {
        return Err(CommandError::new(
            "processing_files_busy",
            "A stacking or a treatment is running.",
        ));
    }
    Ok(())
}

/// The sizes of the produced files, apart from the person's own. Reads only;
/// off the window's thread (the folder may be on an external disk).
#[tauri::command]
pub async fn processing_files_scan<R: Runtime>(
    app: AppHandle<R>,
) -> Result<ProcessingScan, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let root = root_of(&settings(&paths)?)?;
        let sent = paths::open_queue(&paths)
            .map(|queue| processing_files::sent_files(&queue))
            .unwrap_or_default();
        Ok(processing_files::scan(&root, &paths.data_dir, &sent))
    })
    .await
}

/// « Supprimer »: what the ticked boxes name, and only that. The references
/// to the deleted results are dropped from the upload queue; the nights read
/// their folders, so a night whose stacked image is gone is « à empiler »
/// again.
#[tauri::command]
pub async fn processing_files_clean<R: Runtime>(
    app: AppHandle<R>,
    kinds: Vec<CleanKind>,
) -> Result<CleanOutcome, CommandError> {
    refuse_while_working(&app)?;
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let root = root_of(&settings(&paths)?)?;
        let queue = paths::open_queue(&paths)?;
        let sent = processing_files::sent_files(&queue);
        let outcome = processing_files::clean(&root, &paths.data_dir, &sent, &kinds)
            .map_err(|error| CommandError::new("folder_not_found", error.to_string()))?;
        queue.forget_unfinished_files(&outcome.removed)?;
        Ok(outcome)
    })
    .await
}

/// « Réinstaller… » of the Python environment: removed, then made again by
/// the installation of « Dépendances ». A failure leaves the interrupted
/// installation, which « Dépendances » resumes.
#[tauri::command]
pub async fn python_env_reinstall<R: Runtime>(
    app: AppHandle<R>,
) -> Result<EnvAnswer, CommandError> {
    refuse_while_working(&app)?;
    let (cancel, _) = local_work(&app);
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        processing_files::remove_python_environment(&paths.data_dir)
            .map_err(|error| CommandError::new("io_error", error.to_string()))?;
        Ok(match install_python_env(&app, &paths, &cancel) {
            Ok(check) => EnvAnswer::Done { check },
            Err(failure) => EnvAnswer::Failed { failure },
        })
    })
    .await
}

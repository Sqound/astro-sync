//! « Mesure des poses »: the families of a target's
//! lights as the measure will take them, and the measure itself, pose by pose
//! (`astro_sync_core::pose_stats`). Thin calls into the core; the measure
//! reports through its own event and a stop on `pose-stats`
//! (`local_work_cancel_kind`) ends it between two poses, keeping what is
//! measured.

use std::path::Path;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

use astro_sync_core::detection::NightWindow;
use astro_sync_core::local_work::WorkKind;
use astro_sync_core::pose_stats::{
    self, MeasureRequest, PoseStats, PythonMeasurer, SAMPLE_ALL_UP_TO, SAMPLE_STEP,
};
use astro_sync_core::python_env;
use astro_sync_core::tools;

use crate::commands::{local_work_of, run_blocking};
use crate::commands_night::{detect_context, stacking_dir};
use crate::error::CommandError;
use crate::paths;

pub const POSE_STATS_EVENT: &str = "astro-sync://pose-stats";

/// One family of the target's lights, and how many of its poses the first
/// pass measures.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FamilyPlan {
    pub label: String,
    pub exposure_seconds: Option<f64>,
    pub total: usize,
    /// The poses of the first pass (all up to `sampleAllUpTo`, then one in
    /// `sampleStep`).
    pub planned: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FamiliesPlan {
    pub families: Vec<FamilyPlan>,
    pub sample_all_up_to: usize,
    pub sample_step: usize,
}

/// What « Mesurer les poses » will measure, said before it starts.
#[tauri::command]
pub async fn pose_stats_plan(
    night_folder: String,
    target_folder: String,
    evening_date: String,
    longitude: Option<f64>,
) -> Result<FamiliesPlan, CommandError> {
    run_blocking(move || {
        let window = NightWindow::of_evening(&evening_date, longitude);
        let families =
            pose_stats::pose_families(Path::new(&night_folder), &target_folder, window.as_ref())
                .into_iter()
                .map(|family| FamilyPlan {
                    planned: pose_stats::sample_indices(family.frames.len()).len(),
                    total: family.frames.len(),
                    label: family.label,
                    exposure_seconds: family.exposure_seconds,
                })
                .collect();
        Ok(FamiliesPlan {
            families,
            sample_all_up_to: SAMPLE_ALL_UP_TO,
            sample_step: SAMPLE_STEP,
        })
    })
    .await
}

/// « Mesurer les poses » of a target (« Reprendre la mesure » gives the poses
/// already measured in `request.resume`). Behind the stop of kind
/// `pose-stats`; a stop answers what is measured (`stopped`).
#[tauri::command]
pub async fn pose_stats_run<R: Runtime>(
    app: AppHandle<R>,
    request: MeasureRequest,
) -> Result<PoseStats, CommandError> {
    let (cancel, _) = local_work_of(&app, Some(WorkKind::PoseStats));
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let python = python_env::env_python(&paths.data_dir);
        let script = stacking_dir(&app)
            .map(|dir| dir.join("pose_stats.py"))
            .filter(|script| script.is_file());
        let (Some(script), true) = (script, python.is_file()) else {
            return Err(CommandError::new(
                "pose_stats_python_missing",
                "Astro Sync's Python environment is not installed.",
            ));
        };
        // Siril converts the camera RAW poses before they are measured.
        let siril = tools::detect_tools(&detect_context(&app, &paths))
            .into_iter()
            .find(|row| row.id == "siril" && row.state == tools::ToolState::Present)
            .and_then(|row| row.path);
        let measurer =
            PythonMeasurer::for_night(&request.night_folder, python, script).with_siril(siril);
        let handle = app.clone();
        pose_stats::measure_target(&request, &measurer, Some(&cancel), &mut |event| {
            let _ = handle.emit(POSE_STATS_EVENT, event);
        })
        .map_err(|reason| CommandError::new("pose_stats_failed", reason))
    })
    .await
}

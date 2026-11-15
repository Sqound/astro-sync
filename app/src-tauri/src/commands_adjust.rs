//! « Régler l'image »: the commands of the screen that sets a finished render
//! (point noir, luminosité, couleur, lissage, étoiles). The pixels are the
//! script's (`stacking/image_adjust.py`, in the stacking's Python
//! environment) and the window's (the preview, `src/lib/imageAdjust.ts`);
//! what the image set leaves on disk and what leaves in its place is the
//! core's (`astro_sync_core::image_adjust`).
//!
//! The preview's pictures (the render shrunk to 1080 px, the colour's mask,
//! the star layer) are kept in Astro Sync's own data
//! (`stacking-work/adjust/<render>/`), never beside the person's files: only
//! the image set and its record are written there, at « Enregistrer l'image
//! réglée ».

use std::path::{Path, PathBuf};

use serde::Serialize;
use std::hash::{DefaultHasher, Hash, Hasher};
use tauri::{AppHandle, Runtime};
use tauri_plugin_dialog::DialogExt;

use astro_sync_core::image_adjust::{
    self, AdjustError, AdjustState, AdjustValues, ImportOutcome, SavedAdjustment,
};
use astro_sync_core::{local_settings, python_env, tools};

use crate::commands::run_blocking;
use crate::commands_night::{stacking_dir, tool_rows};
use crate::dialogs;
use crate::error::CommandError;
use crate::paths;
use crate::state::AppPaths;

const SCRIPT: &str = "image_adjust.py";

/// A render of « Images à envoyer » or of a night's « Récapitulatif » that can
/// be set: a PNG of a `stacked/` folder.
fn render_of(path: &str) -> Result<PathBuf, CommandError> {
    let path = PathBuf::from(path);
    if !path.is_file() || !image_adjust::is_render_png(&path) {
        return Err(CommandError::new(
            "file_missing",
            format!("{} is not a PNG render Astro Sync made", path.display()),
        ));
    }
    Ok(path)
}

/// The preview's folder of `render`, in Astro Sync's data: one per render
/// file as it is now (a render stacked again gets a new one).
fn work_of(paths: &AppPaths, render: &Path) -> PathBuf {
    let meta = std::fs::metadata(render).ok();
    let stamp = format!(
        "{}|{}|{:?}",
        render.display(),
        meta.as_ref().map_or(0, std::fs::Metadata::len),
        meta.and_then(|m| m.modified().ok())
    );
    let mut hasher = DefaultHasher::new();
    stamp.hash(&mut hasher);
    paths
        .data_dir
        .join("stacking-work")
        .join("adjust")
        .join(format!("{:016x}", hasher.finish()))
}

/// Runs `image_adjust.py <args>` in the stacking's environment and reads the
/// JSON it answers.
fn run_script<R: Runtime>(
    app: &AppHandle<R>,
    paths: &AppPaths,
    args: &[String],
) -> Result<serde_json::Value, CommandError> {
    let python = python_env::env_python(&paths.data_dir);
    if !python.is_file() {
        return Err(CommandError::new(
            "pose_stats_python_missing",
            "the stacking environment is not installed",
        ));
    }
    let script = stacking_dir(app)
        .map(|dir| dir.join(SCRIPT))
        .filter(|script| script.is_file())
        .ok_or_else(|| {
            CommandError::new("pose_stats_python_missing", "image_adjust.py is not there")
        })?;
    let output = python_env::script_command(&python, &script)
        .args(args)
        .output()
        .map_err(|error| CommandError::other(error.to_string()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let answer: serde_json::Value = stdout
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str(line.trim()).ok())
        .ok_or_else(|| {
            CommandError::new(
                "adjust_failed",
                String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .last()
                    .unwrap_or("image_adjust.py answered nothing")
                    .to_string(),
            )
        })?;
    if let Some(code) = answer.get("error").and_then(|c| c.as_str()) {
        let message = answer
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or(code)
            .to_string();
        // The script's own code stays in the message (English, for the logs).
        return Err(CommandError::new(
            "adjust_failed",
            format!("{code}: {message}"),
        ));
    }
    Ok(answer)
}

fn starnet_of<R: Runtime>(app: &AppHandle<R>, paths: &AppPaths) -> Option<PathBuf> {
    tool_rows(app, paths)
        .into_iter()
        .find(|row| row.id == "starnet2" && row.state == tools::ToolState::Present)
        .and_then(|row| row.path)
}

/// Where the star layer stands for the screen's « Étoiles ».
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StarsState {
    /// Separated: the slider moves.
    Ready,
    /// StarNet2 is there; the separation runs when the screen opens.
    Todo,
    /// StarNet2 is not installed (Réglages › Dépendances).
    Missing,
    /// The image is under StarNet2's 512 pixels.
    Small,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdjustOpen {
    pub width: u64,
    pub height: u64,
    pub preview_width: u64,
    pub preview_height: u64,
    /// The bytes of the render.
    pub size: u64,
    pub stars: StarsState,
    /// The image set already saved: its values reopen as they were.
    pub state: Option<AdjustState>,
    /// The time the preview took to prepare (0 when it was ready).
    pub seconds: f64,
}

fn number(value: &serde_json::Value, key: &str) -> u64 {
    value
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
}

/// Opens « Régler l'image » on `path`: the preview prepared (once per render),
/// where the star layer stands, the image set already saved.
#[tauri::command]
pub async fn image_adjust_open<R: Runtime>(
    app: AppHandle<R>,
    path: String,
) -> Result<AdjustOpen, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let render = render_of(&path)?;
        let work = work_of(&paths, &render);
        let ready = work.join("info.json").is_file()
            && work.join("base.png").is_file()
            && work.join("mask.png").is_file();
        let (info, seconds) = if ready {
            let text = std::fs::read_to_string(work.join("info.json"))
                .map_err(|error| CommandError::other(error.to_string()))?;
            let info: serde_json::Value = serde_json::from_str(&text)
                .map_err(|error| CommandError::other(error.to_string()))?;
            (info, 0.0)
        } else {
            std::fs::create_dir_all(&work)
                .map_err(|error| CommandError::other(error.to_string()))?;
            let info = run_script(
                &app,
                &paths,
                &[
                    "prepare".into(),
                    "--image".into(),
                    render.display().to_string(),
                    "--work".into(),
                    work.display().to_string(),
                ],
            )?;
            let seconds = info
                .get("seconds")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            (info, seconds)
        };
        let (width, height) = (number(&info, "width"), number(&info, "height"));
        let stars = if work.join("stars.tif").is_file() && work.join("stars.png").is_file() {
            StarsState::Ready
        } else if width.min(height) < 512 {
            StarsState::Small
        } else if starnet_of(&app, &paths).is_none() {
            StarsState::Missing
        } else {
            StarsState::Todo
        };
        Ok(AdjustOpen {
            width,
            height,
            preview_width: number(&info, "previewWidth"),
            preview_height: number(&info, "previewHeight"),
            size: std::fs::metadata(&render).map_or(0, |m| m.len()),
            stars,
            state: image_adjust::state(&render),
            seconds,
        })
    })
    .await
}

/// One picture of the preview, as PNG bytes: `base` (the render shrunk),
/// `mask` (where the signal is), `stars` (the star layer).
#[tauri::command]
pub async fn image_adjust_layer<R: Runtime>(
    app: AppHandle<R>,
    path: String,
    layer: String,
) -> Result<tauri::ipc::Response, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let render = render_of(&path)?;
        let name = match layer.as_str() {
            "base" => "base.png",
            "mask" => "mask.png",
            "stars" => "stars.png",
            _ => {
                return Err(CommandError::new(
                    "adjust_failed",
                    format!("no layer {layer}"),
                ));
            }
        };
        let bytes = std::fs::read(work_of(&paths, &render).join(name))
            .map_err(|error| CommandError::other(error.to_string()))?;
        Ok(tauri::ipc::Response::new(bytes))
    })
    .await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StarsAnswer {
    pub stars: StarsState,
    pub seconds: f64,
}

/// The star layer separated by StarNet2 (where « Dépendances » found it),
/// once per render; nothing is installed.
#[tauri::command]
pub async fn image_adjust_stars<R: Runtime>(
    app: AppHandle<R>,
    path: String,
) -> Result<StarsAnswer, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let render = render_of(&path)?;
        let work = work_of(&paths, &render);
        if work.join("stars.tif").is_file() && work.join("stars.png").is_file() {
            return Ok(StarsAnswer {
                stars: StarsState::Ready,
                seconds: 0.0,
            });
        }
        let Some(starnet) = starnet_of(&app, &paths) else {
            return Ok(StarsAnswer {
                stars: StarsState::Missing,
                seconds: 0.0,
            });
        };
        let mut args: Vec<String> = vec![
            "stars".into(),
            "--image".into(),
            render.display().to_string(),
            "--work".into(),
            work.display().to_string(),
            "--starnet".into(),
            starnet.display().to_string(),
        ];
        // A render with a TIFF twin: StarNet2 reads the 16-bit data the image
        // set is computed from.
        if let Some(twin) = image_adjust::twin_of(&render) {
            args.extend(["--tiff".into(), twin.display().to_string()]);
        }
        let answer = run_script(&app, &paths, &args)?;
        Ok(StarsAnswer {
            stars: if answer.get("state").and_then(|s| s.as_str()) == Some("ready") {
                StarsState::Ready
            } else {
                StarsState::Small
            },
            seconds: answer
                .get("seconds")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0),
        })
    })
    .await
}

/// The preview smoothed at `amount` %, as PNG bytes (NL-means, ≈ 1 s).
#[tauri::command]
pub async fn image_adjust_smooth<R: Runtime>(
    app: AppHandle<R>,
    path: String,
    amount: f64,
) -> Result<tauri::ipc::Response, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let render = render_of(&path)?;
        let work = work_of(&paths, &render);
        let answer = run_script(
            &app,
            &paths,
            &[
                "smooth".into(),
                "--work".into(),
                work.display().to_string(),
                "--amount".into(),
                amount.to_string(),
            ],
        )?;
        let file = answer
            .get("path")
            .and_then(|p| p.as_str())
            .map(PathBuf::from)
            .ok_or_else(|| CommandError::new("adjust_failed", "no smoothed preview"))?;
        let bytes = std::fs::read(&file).map_err(|error| CommandError::other(error.to_string()))?;
        Ok(tauri::ipc::Response::new(bytes))
    })
    .await
}

/// « Enregistrer l'image réglée »: the whole image computed with `values`,
/// written beside the render (`stacked/adjusted/`), never over it, with its
/// record. From now on it leaves in place of the render. A render with a TIFF
/// twin is set from that 16-bit TIFF: the image set is written as a TIFF and
/// as the PNG that is its 8-bit reduction, and its TIFF leaves in place of
/// the twin when the person joins the TIFF.
#[tauri::command]
pub async fn image_adjust_save<R: Runtime>(
    app: AppHandle<R>,
    path: String,
    values: AdjustValues,
) -> Result<AdjustState, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let render = render_of(&path)?;
        let values = values.clamped();
        if values.is_neutral() {
            return Err(CommandError::new(
                "adjust_unchanged",
                "every setting is the render's: nothing to write",
            ));
        }
        let out = image_adjust::adjusted_path(&render)
            .ok_or_else(|| CommandError::new("file_missing", "no place for the image set"))?;
        let work = work_of(&paths, &render);
        let mut args: Vec<String> = vec![
            "save".into(),
            "--image".into(),
            render.display().to_string(),
            "--work".into(),
            work.display().to_string(),
            "--values".into(),
            serde_json::to_string(&values).unwrap_or_default(),
            "--out".into(),
            out.display().to_string(),
        ];
        let out_tiff = image_adjust::adjusted_tiff_path(&render);
        match (image_adjust::twin_of(&render), &out_tiff) {
            (Some(twin), Some(out_tiff)) => args.extend([
                "--tiff".into(),
                twin.display().to_string(),
                "--out-tiff".into(),
                out_tiff.display().to_string(),
            ]),
            // No twin: an image set's TIFF left from a render of before goes.
            (None, Some(out_tiff)) => {
                let _ = std::fs::remove_file(out_tiff);
            }
            _ => {}
        }
        run_script(&app, &paths, &args)?;
        image_adjust::write_record(&render, values)
            .map_err(|error| CommandError::new("adjust_failed", error.to_string()))
    })
    .await
}

/// The image set of `path` when it has a valid one (the row of « Images à
/// envoyer » says « Réglée »).
#[tauri::command]
pub async fn image_adjust_state(path: String) -> Result<Option<AdjustState>, CommandError> {
    run_blocking(move || {
        Ok(render_of(&path)
            .ok()
            .and_then(|render| image_adjust::state(&render)))
    })
    .await
}

/// « Revenir au rendu d'origine »: the image set and its record removed; the
/// render leaves again.
#[tauri::command]
pub async fn image_adjust_revert(path: String) -> Result<(), CommandError> {
    run_blocking(move || {
        let render = render_of(&path)?;
        image_adjust::revert(&render)
            .map_err(|error| CommandError::new("adjust_failed", error.to_string()))
    })
    .await
}

/* ----------------------------- Saved settings ----------------------------- */
// « Appliquer un réglage enregistré » / « Enregistrer ce réglage »: five
// values under a name, kept with the application's settings, free (no device,
// no target). Applying one is the window's: it moves the sliders of the
// preview, and nothing is written before « Enregistrer l'image réglée ».

fn adjust_error(error: AdjustError) -> CommandError {
    CommandError::new(error.code(), error.to_string())
}

/// Reads the saved settings, changes them with `change`, writes them back.
fn with_saved<R: Runtime, T>(
    app: &AppHandle<R>,
    change: impl FnOnce(&mut Vec<SavedAdjustment>) -> Result<T, AdjustError>,
) -> Result<(Vec<SavedAdjustment>, T), CommandError> {
    let paths = paths::resolve(app)?;
    let mut settings = local_settings::load(&paths.local_settings)?;
    let answer = change(&mut settings.saved_adjustments).map_err(adjust_error)?;
    local_settings::save(&paths.local_settings, &settings)?;
    Ok((settings.saved_adjustments, answer))
}

/// The saved settings, in the order they were saved.
#[tauri::command]
pub async fn image_adjust_saved_list<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<SavedAdjustment>, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        Ok(local_settings::load(&paths.local_settings)?.saved_adjustments)
    })
    .await
}

/// « Enregistrer le réglage »: `values` kept under `name` (a name already
/// taken, whatever its case, is refused: `adjust_name_taken`).
#[tauri::command]
pub async fn image_adjust_saved_save<R: Runtime>(
    app: AppHandle<R>,
    name: String,
    values: AdjustValues,
) -> Result<Vec<SavedAdjustment>, CommandError> {
    run_blocking(move || {
        with_saved(&app, |list| image_adjust::save_named(list, &name, values))
            .map(|(list, ())| list)
    })
    .await
}

/// « Renommer »: the setting `from` named `to`; its values unchanged.
#[tauri::command]
pub async fn image_adjust_saved_rename<R: Runtime>(
    app: AppHandle<R>,
    from: String,
    to: String,
) -> Result<Vec<SavedAdjustment>, CommandError> {
    run_blocking(move || {
        with_saved(&app, |list| image_adjust::rename_named(list, &from, &to)).map(|(list, ())| list)
    })
    .await
}

/// « Supprimer »: the setting removed; the images already set with it keep
/// their image set and its values.
#[tauri::command]
pub async fn image_adjust_saved_remove<R: Runtime>(
    app: AppHandle<R>,
    name: String,
) -> Result<Vec<SavedAdjustment>, CommandError> {
    run_blocking(move || {
        with_saved(&app, |list| image_adjust::remove_named(list, &name)).map(|(list, ())| list)
    })
    .await
}

/// « Exporter… »: the saved settings written where the person chooses (a
/// small JSON file). `None`: the dialog was dismissed.
#[tauri::command]
pub async fn image_adjust_saved_export<R: Runtime>(
    app: AppHandle<R>,
    file_name: String,
) -> Result<Option<String>, CommandError> {
    let picked = dialogs::pick(|answer| {
        app.dialog()
            .file()
            .add_filter("JSON", &["json"])
            .set_file_name(&file_name)
            .save_file(answer)
    })
    .await?;
    let Some(path) = picked else {
        return Ok(None);
    };
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let list = local_settings::load(&paths.local_settings)?.saved_adjustments;
        image_adjust::export_saved_to(Path::new(&path), &list).map_err(adjust_error)?;
        Ok(Some(path))
    })
    .await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportAnswer {
    pub list: Vec<SavedAdjustment>,
    #[serde(flatten)]
    pub outcome: ImportOutcome,
}

/// « Importer… »: the settings of a file the person chooses, added to hers; a
/// name already taken is left out, never overwritten, and named in the
/// answer. `None`: the dialog was dismissed.
#[tauri::command]
pub async fn image_adjust_saved_import<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Option<ImportAnswer>, CommandError> {
    let picked = dialogs::pick(|answer| {
        app.dialog()
            .file()
            .add_filter("JSON", &["json"])
            .pick_file(answer)
    })
    .await?;
    let Some(path) = picked else {
        return Ok(None);
    };
    run_blocking(move || {
        with_saved(&app, |list| {
            image_adjust::import_saved_from(list, Path::new(&path))
        })
        .map(|(list, outcome)| Some(ImportAnswer { list, outcome }))
    })
    .await
}

//! A master stacked elsewhere, measured by Astro Sync.
//!
//! Astro Sync measures the quality of an image (FWHM, eccentricity, stars --
//! `stacking/star_quality.py`) on the linear master of a stacking it made
//! itself. Someone who stacks in PixInsight or Siril has no such stacking:
//! "Measure a master…" hands Astro Sync the master (FITS or XISF, **linear**)
//! and measures it with the very same code.
//!
//! - **The measure** is `star_quality.py --measure <file>`, in the stacking
//!   environment: it answers one JSON line, `measured` with the figures or
//!   `refused` with a code (`stretched`, `too_few_stars`, `unsupported_format`,
//!   `unreadable`, `not_an_image`). Nothing is measured on an image that is not
//!   linear: the width of a star on a stretched image means nothing.
//! - **The place** of the result is a hidden file in the target's folder of the
//!   night, [`FILE_NAME`] (`.astro-sync-master-quality.json`, hidden like
//!   Astro Sync's other files there, so the scan of the night never counts
//!   it): the name of the master and its figures, as the stacker writes them in
//!   `<name>_stacking.json`.
//! - **The way out** is the one of a quality Astro Sync measured on its own
//!   stacking ([`crate::metadata::proposals_for_image`]): `imageQuality` of the
//!   images of the target, with the arc seconds when the frames' pixel scale is
//!   known, under the right `analysis.logs`, sent with the image.
//!
//! Nothing here sends a byte of the master: only its figures leave.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::external_process;
use crate::local_work::CancelFlag;
use crate::metadata::{ImageQuality, MeasuredQuality, frames_pixel_scale};
use crate::progression_images;
use crate::python_env;

/// The hidden file, in the target's folder of the night, that keeps the
/// measure of the master a person gave.
pub const FILE_NAME: &str = ".astro-sync-master-quality.json";

/// A long image on a slow disk: the measure is given a few minutes.
pub const TIMEOUT: Duration = Duration::from_secs(300);

/// What is kept of a measured master.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    /// The file name of the master (never its folder: only a name is shown).
    pub master: String,
    pub quality: MeasuredQuality,
}

/// What the script answered.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Measured(MeasuredQuality),
    /// A master that cannot be measured: the script's `code`, and its
    /// English words for the log.
    Refused {
        code: String,
        message: String,
    },
}

/// Why the measure could not be asked for or read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The script could not be started, ran out of time, or was stopped.
    NotRun(String),
    /// The script ended without the answer it must give.
    NoAnswer(String),
}

/// The words of a refusal the window knows, else `unreadable`.
const REFUSAL_CODES: [&str; 5] = [
    "stretched",
    "too_few_stars",
    "unsupported_format",
    "unreadable",
    "not_an_image",
];

/// Reads the script's answer (its last non-empty line of standard output).
#[must_use]
pub fn parse_answer(stdout: &str) -> Option<Answer> {
    let line = stdout.lines().rev().find(|l| !l.trim().is_empty())?;
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    match value.get("status")?.as_str()? {
        "measured" => {
            let quality: MeasuredQuality =
                serde_json::from_value(value.get("quality")?.clone()).ok()?;
            Some(Answer::Measured(quality))
        }
        "refused" => {
            let code = value.get("code")?.as_str()?;
            Some(Answer::Refused {
                code: if REFUSAL_CODES.contains(&code) {
                    code.to_string()
                } else {
                    "unreadable".to_string()
                },
                message: value
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            })
        }
        _ => None,
    }
}

/// Measures the master at `file` with `star_quality.py` (`script`) in the
/// environment of `python`.
pub fn measure(
    python: &Path,
    script: &Path,
    file: &Path,
    cancel: Option<&CancelFlag>,
) -> Result<Answer, Failure> {
    let mut command = python_env::script_command(python, script);
    command.arg("--measure").arg(file);
    let end = external_process::run(command, TIMEOUT, cancel)
        .map_err(|e| Failure::NotRun(e.to_string()))?;
    if end.cancelled {
        return Err(Failure::NotRun("stopped".to_string()));
    }
    if end.timed_out {
        return Err(Failure::NotRun("timed out".to_string()));
    }
    parse_answer(&end.stdout).ok_or_else(|| {
        let reason = end.stderr.lines().rev().find(|l| !l.trim().is_empty());
        Failure::NoAnswer(reason.unwrap_or("no answer").to_string())
    })
}

/// A target's folder name is one path component: nothing the window sends
/// can name a place outside the night's folder.
#[must_use]
pub fn is_folder_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', '\0'])
        && !name.starts_with('.')
}

/// The file that keeps the measure of `target_folder_name` in the night.
#[must_use]
pub fn record_path(night_folder: &Path, target_folder_name: &str) -> PathBuf {
    night_folder.join(target_folder_name).join(FILE_NAME)
}

/// Keeps the measure of a master in the target's folder.
pub fn save(
    night_folder: &Path,
    target_folder_name: &str,
    master_file: &Path,
    quality: &MeasuredQuality,
) -> std::io::Result<()> {
    let record = Record {
        master: master_file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        quality: quality.clone(),
    };
    let path = record_path(night_folder, target_folder_name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(&record).map_err(std::io::Error::other)?;
    std::fs::write(path, text)
}

/// What is kept for a target, if anything readable.
#[must_use]
pub fn load(night_folder: &Path, target_folder_name: &str) -> Option<Record> {
    let text = std::fs::read_to_string(record_path(night_folder, target_folder_name)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Forgets the measure of a target's master (the file may not exist).
pub fn forget(night_folder: &Path, target_folder_name: &str) -> std::io::Result<()> {
    match std::fs::remove_file(record_path(night_folder, target_folder_name)) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// The measure kept for the target an `image` of the night belongs to: the
/// file is looked for in the folders above the image, up to the night's folder
/// (`<night>/<target>/final/x.tif` and `<night>/<target>/stacked/x.tif` both
/// find `<night>/<target>/`).
#[must_use]
pub fn record_near(image: &Path, night_folder: &Path) -> Option<Record> {
    for folder in image.ancestors().skip(1) {
        if folder == night_folder || folder.parent().is_none() {
            return None;
        }
        if folder.starts_with(night_folder)
            && let Ok(text) = std::fs::read_to_string(folder.join(FILE_NAME))
        {
            return serde_json::from_str(&text).ok();
        }
    }
    None
}

/// The quality as it is sent, for a target's measure: in arc seconds when the
/// frames of the target say their pixel scale (`None`: the figure is outside
/// what the contract allows).
#[must_use]
pub fn quality_of(
    night_folder: &Path,
    target_folder_name: &str,
    measure: &MeasuredQuality,
) -> Option<ImageQuality> {
    let families = progression_images::target_families(night_folder, target_folder_name);
    ImageQuality::from_measure(measure, frames_pixel_scale(&families, None))
}

/// What "Measure a master…" ends with.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "result",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Outcome {
    /// Measured and kept: the quality as it will be sent, and the master's
    /// file name.
    Measured {
        master: String,
        quality: ImageQuality,
    },
    /// Not measured, with the word the window has a sentence for:
    /// `reserved` (no `analysis.logs`), `python_missing`, `night_missing`
    /// (the target has no folder in the night) and the script's own codes.
    Refused { code: String },
}

/// Measures the master `file` of a target of the night and keeps the result.
///
/// `entitled`: the space holds `analysis.logs`, the right under which the
/// quality of an image leaves (without it nothing is measured: the figures
/// could not be sent). A previous measure of the target is replaced only by
/// a new one that succeeded: a refusal changes nothing.
pub fn measure_for_target(
    night_folder: &Path,
    target_folder_name: &str,
    file: &Path,
    entitled: bool,
    python: &Path,
    script: &Path,
    cancel: Option<&CancelFlag>,
) -> Result<Outcome, Failure> {
    let refused = |code: &str| Ok(Outcome::Refused { code: code.into() });
    if !entitled {
        return refused("reserved");
    }
    if !is_folder_name(target_folder_name) || !night_folder.join(target_folder_name).is_dir() {
        return refused("night_missing");
    }
    if !python.is_file() || !script.is_file() {
        return refused("python_missing");
    }
    if !file.is_file() {
        return refused("unreadable");
    }
    match measure(python, script, file, cancel)? {
        Answer::Refused { code, .. } => refused(&code),
        Answer::Measured(measure) => {
            let Some(quality) = quality_of(night_folder, target_folder_name, &measure) else {
                // A figure the contract would refuse (no star width).
                return refused("too_few_stars");
            };
            save(night_folder, target_folder_name, file, &measure)
                .map_err(|e| Failure::NotRun(e.to_string()))?;
            Ok(Outcome::Measured {
                master: file
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                quality,
            })
        }
    }
}

/// Measures the master `file` and keeps nothing (the command line's
/// `measure-master <file>` without a night): the same script, the same
/// answers and the same [`Outcome`] as [`measure_for_target`], with the width
/// in pixels only (the pixel scale is the frames' of a night).
pub fn measure_alone(
    file: &Path,
    python: &Path,
    script: &Path,
    cancel: Option<&CancelFlag>,
) -> Result<Outcome, Failure> {
    let refused = |code: &str| Ok(Outcome::Refused { code: code.into() });
    if !python.is_file() || !script.is_file() {
        return refused("python_missing");
    }
    if !file.is_file() {
        return refused("unreadable");
    }
    match measure(python, script, file, cancel)? {
        Answer::Refused { code, .. } => refused(&code),
        Answer::Measured(measure) => match ImageQuality::from_measure(&measure, None) {
            Some(quality) => Ok(Outcome::Measured {
                master: file
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                quality,
            }),
            None => refused("too_few_stars"),
        },
    }
}

/// What is kept for a target, as the window shows it (`None`: no master was
/// measured, or what was kept is not readable any more).
#[must_use]
pub fn current(night_folder: &Path, target_folder_name: &str) -> Option<Outcome> {
    if !is_folder_name(target_folder_name) {
        return None;
    }
    let record = load(night_folder, target_folder_name)?;
    let quality = quality_of(night_folder, target_folder_name, &record.quality)?;
    Some(Outcome::Measured {
        master: record.master,
        quality,
    })
}

#[cfg(test)]
mod tests;

//! What the frame measurement and the stacking leave behind for the night's
//! report: the measures of each pose (width and eccentricity of its stars,
//! its stars, its sky, the reason it departs from the night) and the poses
//! the stacking left out. Kept in the night's scratch
//! (`<night>/.astro-sync-work/pose-stats/`), one file per target and kind,
//! replaced at each measure or stacking; the report v3 reads them
//! ([`load`]) with the stacking's own record (`<target>/stacked/
//! *_stacking.json`, `registration.unregistered`). Poses are named by their
//! file name: a symbolic link in `lights/` and its target share it. Nothing
//! here ever leaves the computer but the values the report carries.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::{PoseStats, Reason, SCRATCH_DIR};
use crate::working_folder::WORK_DIR;

const FORMAT: u32 = 1;

/// One measured pose, as the report reads it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PoseRecord {
    pub fwhm_pixels: Option<f64>,
    pub eccentricity: Option<f64>,
    pub star_count: Option<u32>,
    pub sky_background: Option<f64>,
    /// Why the measure flags it (`dawn`, `sky`, `clouds`, `trailed`,
    /// `wide`), `None` for a pose in line with the night.
    pub flag: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MeasuresFile {
    format: u32,
    target: String,
    /// By file name.
    poses: HashMap<String, PoseRecord>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LeftOutFile {
    format: u32,
    target: String,
    /// The file names the last stacking of the target left out.
    left_out: Vec<String>,
}

/// What the night's scratch and stackings say of its poses.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NightPoseRecords {
    /// By file name.
    pub measures: HashMap<String, PoseRecord>,
    /// Left out of the stacking by the person's choice (« Écarter les poses
    /// signalées »), by file name.
    pub left_out: HashSet<String>,
    /// Not registered by the stacking (the stacking could not align them).
    pub unregistered: HashSet<String>,
}

/// `dawn`, `sky`, `clouds`, `trailed`, `wide`: the words of the contract.
#[must_use]
pub fn reason_word(reason: &Reason) -> &'static str {
    match reason {
        Reason::Dawn { .. } => "dawn",
        Reason::Sky { .. } => "sky",
        Reason::Clouds => "clouds",
        Reason::Trailed => "trailed",
        Reason::Wide => "wide",
    }
}

fn file_name(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
}

/// The target's part of a file name: its folder, other characters than
/// letters and digits made `_`.
fn key_of(target_folder: &str) -> String {
    target_folder
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}

fn scratch(night_folder: &Path) -> PathBuf {
    night_folder.join(WORK_DIR).join(SCRATCH_DIR)
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string(value).map_err(std::io::Error::other)?;
    let partial = path.with_extension("json.partial");
    std::fs::write(&partial, text)?;
    std::fs::rename(&partial, path)
}

/// Keeps the measures of a target's poses for the report (replacing the
/// previous ones). A failure to write only loses the figures of the report.
pub fn save_measures(
    night_folder: &Path,
    target_folder: &str,
    stats: &PoseStats,
) -> std::io::Result<()> {
    let mut poses = HashMap::new();
    for family in &stats.families {
        let flags: HashMap<&Path, &Reason> = family
            .flagged
            .iter()
            .map(|flagged| (flagged.path.as_path(), &flagged.reason))
            .collect();
        for measure in &family.measured {
            if measure.unreadable {
                continue;
            }
            let Some(name) = file_name(&measure.path) else {
                continue;
            };
            poses.insert(
                name,
                PoseRecord {
                    fwhm_pixels: measure.fwhm_pixels,
                    eccentricity: measure.eccentricity,
                    star_count: measure.star_count,
                    sky_background: measure.sky_background,
                    flag: flags
                        .get(measure.path.as_path())
                        .map(|reason| reason_word(reason).to_string()),
                },
            );
        }
    }
    write_json(
        &scratch(night_folder).join(format!("measures-{}.json", key_of(target_folder))),
        &MeasuresFile {
            format: FORMAT,
            target: target_folder.to_string(),
            poses,
        },
    )
}

/// Keeps the poses the stacking of a target leaves out (replacing the
/// previous list; an empty list when none is).
pub fn save_left_out(
    night_folder: &Path,
    target_folder: &str,
    left_out: &[PathBuf],
) -> std::io::Result<()> {
    let mut names: Vec<String> = left_out.iter().filter_map(|p| file_name(p)).collect();
    names.sort();
    names.dedup();
    write_json(
        &scratch(night_folder).join(format!("left-out-{}.json", key_of(target_folder))),
        &LeftOutFile {
            format: FORMAT,
            target: target_folder.to_string(),
            left_out: names,
        },
    )
}

/// The frames a stacking record (`*_stacking.json`) says it could not
/// register.
fn unregistered_of(record: &serde_json::Value) -> Vec<String> {
    record
        .get("registration")
        .and_then(|r| r.get("unregistered"))
        .and_then(serde_json::Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(serde_json::Value::as_str)
                .filter_map(|name| file_name(Path::new(name)))
                .collect()
        })
        .unwrap_or_default()
}

/// Reads what the night's scratch and the stacking records of its targets
/// (`<target>/stacked/*_stacking.json`) say of its poses; a file that cannot
/// be read is skipped.
#[must_use]
pub fn load(night_folder: &Path) -> NightPoseRecords {
    let mut records = NightPoseRecords::default();
    if let Ok(entries) = std::fs::read_dir(scratch(night_folder)) {
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            let name = file_name(&path).unwrap_or_default();
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if name.starts_with("measures-") && name.ends_with(".json") {
                if let Ok(file) = serde_json::from_str::<MeasuresFile>(&text) {
                    records.measures.extend(file.poses);
                }
            } else if name.starts_with("left-out-")
                && name.ends_with(".json")
                && let Ok(file) = serde_json::from_str::<LeftOutFile>(&text)
            {
                records.left_out.extend(file.left_out);
            }
        }
    }
    if let Ok(targets) = std::fs::read_dir(night_folder) {
        for target in targets.flatten() {
            let stacked = target.path().join("stacked");
            let Ok(files) = std::fs::read_dir(&stacked) else {
                continue;
            };
            for file in files.flatten() {
                let path = file.path();
                if !file_name(&path).is_some_and(|n| n.ends_with("_stacking.json")) {
                    continue;
                }
                let Some(record) = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
                else {
                    continue;
                };
                records.unregistered.extend(unregistered_of(&record));
            }
        }
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pose_stats::{FamilyStats, FlaggedPose, Medians, PoseMeasure};
    use crate::test_support::TempDir;

    fn measure(path: &str, fwhm: f64) -> PoseMeasure {
        PoseMeasure {
            path: PathBuf::from(path),
            index: 0,
            date_obs: None,
            unix: None,
            fwhm_pixels: Some(fwhm),
            eccentricity: Some(0.4),
            star_count: Some(120),
            sky_background: Some(800.0),
            star_widths: Vec::new(),
            star_eccentricities: Vec::new(),
            unreadable: false,
        }
    }

    #[test]
    fn the_measures_the_left_out_and_the_unregistered_poses_are_read_back_by_name() {
        let night = TempDir::new("pose-records");
        let stats = PoseStats {
            families: vec![FamilyStats {
                label: "L_90s".into(),
                exposure_seconds: Some(90.0),
                total: 2,
                measured: vec![
                    measure("/nights/n/T/lights/Light_0001.fit", 2.6),
                    measure("/nights/n/T/lights/Light_0002.fit", 4.1),
                ],
                judged: true,
                medians: Medians::default(),
                shape_missing: None,
                family_shape: None,
                flagged: vec![FlaggedPose {
                    path: PathBuf::from("/nights/n/T/lights/Light_0002.fit"),
                    index: 1,
                    reason: Reason::Clouds,
                }],
            }],
            total: 2,
            planned: 2,
            stopped: false,
        };
        save_measures(night.path(), "T", &stats).unwrap();
        save_left_out(
            night.path(),
            "T",
            &[PathBuf::from("/nights/n/T/lights/Light_0002.fit")],
        )
        .unwrap();
        std::fs::create_dir_all(night.path().join("T/stacked")).unwrap();
        std::fs::write(
            night.path().join("T/stacked/T_2026-08-10_stacking.json"),
            r#"{"registration":{"unregistered":["Light_0003.fit"]}}"#,
        )
        .unwrap();

        let records = load(night.path());
        assert_eq!(records.measures["Light_0001.fit"].fwhm_pixels, Some(2.6));
        assert_eq!(records.measures["Light_0001.fit"].flag, None);
        assert_eq!(
            records.measures["Light_0002.fit"].flag.as_deref(),
            Some("clouds")
        );
        assert!(records.left_out.contains("Light_0002.fit"));
        assert!(records.unregistered.contains("Light_0003.fit"));

        // A stacking that leaves nothing out replaces the list.
        save_left_out(night.path(), "T", &[]).unwrap();
        assert!(load(night.path()).left_out.is_empty());
    }
}

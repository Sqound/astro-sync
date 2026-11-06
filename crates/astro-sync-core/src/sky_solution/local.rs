//! The results of the working folder that are online without a sky solution.
//!
//! A TIFF Astro Sync rendered carries no coordinates, and a very wide field
//! (a 3.5° nebula) cannot be solved blind: the star database does not reach
//! so far. The folder it was made in does hold what the solver needs -- the
//! integration masters keep the pointing (`RA`/`DEC`) and the optics
//! (`XPIXSZ`, `FOCALLEN`) of the frames. [`local_results`] finds the images
//! of the folder that the space lacks a solution for, by their SHA-256, and
//! gives each the position and the scale of the master it was made from:
//! the master of the night's target beside it, or, for the image of a
//! multi-night plan, the master of the nights its record names. Nothing is
//! written; a file is read only to compare its digest.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{MissingCapture, SolveHints};
use crate::analysis::fits::read_primary_header_file;
use crate::analysis::sky::sensor_pixel_scale;
use crate::catalogue_targets::header_target;
use crate::multi_night::{IntegrationMaster, night_masters, target_masters};
use crate::plan_image::read_plan_identity;
use crate::scanner::file_sha256;
use crate::working_folder::{STACKED_DIR, list_night_folders, read_target_identity};

/// An image of the working folder the space holds as a capture, and what its
/// master says of where it points and how big its pixels are.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalResult {
    pub path: PathBuf,
    pub hints: SolveHints,
}

fn is_image(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "tif" | "tiff" | "png" | "jpg"
        )
    })
}

/// The position and the scale a master gives: the pointing of its frames and
/// the scale of its pixels (a plan's image is aligned on the frame of one
/// master, its pixels are that master's).
#[must_use]
pub fn hints_of_master(master: &IntegrationMaster) -> SolveHints {
    let Ok(header) = read_primary_header_file(&master.path) else {
        return SolveHints::default();
    };
    let target = header_target(&header);
    SolveHints {
        ra_deg: target.ra_degrees,
        dec_deg: target.dec_degrees,
        pixel_scale_arcsec: sensor_pixel_scale(&header).map(|s| s.arcsec_per_pixel),
    }
}

/// The first master of `masters` that says where it points.
fn first_hints(masters: &[IntegrationMaster]) -> SolveHints {
    masters
        .iter()
        .map(hints_of_master)
        .find(|h| h.ra_deg.is_some() && h.dec_deg.is_some())
        .or_else(|| {
            masters
                .iter()
                .map(hints_of_master)
                .find(|h| h.pixel_scale_arcsec.is_some())
        })
        .unwrap_or_default()
}

/// The nights `<name>_plan.json` (the record of an image of a plan) combined.
fn plan_record_nights(stacked: &Path, image: &Path) -> Vec<String> {
    let Some(stem) = image.file_stem().and_then(|s| s.to_str()) else {
        return Vec::new();
    };
    // `IC_1805_plan_raw.tif` -> `IC_1805_plan.json`; the stem may carry a
    // pipeline, a contrast and a frame after `_plan`.
    let Some(end) = stem.find("_plan") else {
        return Vec::new();
    };
    let record = stacked.join(format!("{}_plan.json", &stem[..end]));
    let Ok(text) = std::fs::read_to_string(record) else {
        return Vec::new();
    };
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|value| value["masters"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|m| m["nightId"].as_str().map(str::to_string))
        .collect()
}

/// Where an image Astro Sync made points and how big its pixels are, from
/// the master it was made from: a render of a night (`<night>/<target>/
/// stacked/`) takes the masters beside it, the image of a plan (`<plan>/
/// stacked/`) those of the nights its record names. Empty hints for any other
/// image. Nothing is written.
#[must_use]
pub fn hints_of_image(image: &Path) -> SolveHints {
    let Some(stacked) = image.parent().filter(|p| p.ends_with(STACKED_DIR)) else {
        return SolveHints::default();
    };
    let Some(container) = stacked.parent() else {
        return SolveHints::default();
    };
    if let Some(plan) = read_plan_identity(container) {
        let Some(root) = container.parent() else {
            return SolveHints::default();
        };
        let ids = plan_record_nights(stacked, image);
        let masters: Vec<IntegrationMaster> = list_night_folders(root)
            .unwrap_or_default()
            .iter()
            .filter(|n| ids.is_empty() || ids.contains(&n.night_id))
            .flat_map(|n| target_masters(&n.folder, &plan.target_id))
            .collect();
        return first_hints(&masters);
    }
    match (container.parent(), container.file_name()) {
        (Some(night), Some(target)) if read_target_identity(container).is_some() => {
            first_hints(&night_masters(night, &target.to_string_lossy()))
        }
        _ => SolveHints::default(),
    }
}

/// The images of `root` (the working folder) whose SHA-256 is one of
/// `wanted`'s, by lower-case digest, with [`hints_of_image`]. Only a file of
/// the size of a wanted capture is hashed.
#[must_use]
pub fn local_results(root: &Path, wanted: &[MissingCapture]) -> BTreeMap<String, LocalResult> {
    let mut found = BTreeMap::new();
    if wanted.is_empty() {
        return found;
    }
    let by_digest: BTreeMap<String, u64> = wanted
        .iter()
        .map(|c| (c.sha256.to_ascii_lowercase(), c.byte_size))
        .collect();
    let sizes: Vec<u64> = by_digest.values().copied().collect();
    let mut look = |stacked: &Path| {
        let Ok(entries) = std::fs::read_dir(stacked) else {
            return;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if !is_image(&path) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() || !sizes.contains(&meta.len()) {
                continue;
            }
            let Ok(digest) = file_sha256(&path) else {
                continue;
            };
            let digest = digest.to_ascii_lowercase();
            if by_digest.contains_key(&digest) && !found.contains_key(&digest) {
                let hints = hints_of_image(&path);
                found.insert(digest, LocalResult { path, hints });
            }
        }
    };
    // The night folders and the plan folders directly under the root.
    let Ok(children) = std::fs::read_dir(root) else {
        return found;
    };
    for child in children.filter_map(Result::ok) {
        let folder = child.path();
        if !folder.is_dir() {
            continue;
        }
        if read_plan_identity(&folder).is_some() {
            look(&folder.join(STACKED_DIR));
        } else if let Ok(targets) = std::fs::read_dir(&folder) {
            for target in targets.filter_map(Result::ok) {
                look(&target.path().join(STACKED_DIR));
            }
        }
    }
    found
}

impl SolveHints {
    /// These hints completed by `other`'s: what this one does not say. The
    /// pointing is taken as a pair, never half of each.
    #[must_use]
    pub fn completed_by(self, other: Self) -> Self {
        let own_position = self.ra_deg.is_some() && self.dec_deg.is_some();
        let (ra_deg, dec_deg) = if own_position {
            (self.ra_deg, self.dec_deg)
        } else {
            (other.ra_deg, other.dec_deg)
        };
        Self {
            ra_deg,
            dec_deg,
            pixel_scale_arcsec: self.pixel_scale_arcsec.or(other.pixel_scale_arcsec),
        }
    }
}

#[cfg(test)]
mod tests;

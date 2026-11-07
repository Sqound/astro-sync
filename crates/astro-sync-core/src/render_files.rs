//! The two files of a render, and which one leaves.
//!
//! The stacker writes each render twice in `stacked/`: `<name>.png` (8-bit,
//! about 13 MB) and `<name>.tif` (16-bit, about 47 MB). The gallery of the
//! space shows an image derived from the original, PNG or TIFF alike; the
//! TIFF only serves the download in full resolution. Sending both by default
//! filled a space about four times faster than the gallery needs (nine TIFF,
//! about 420 MB, for one capture on 06/11/2026), without the person ever
//! choosing it. Hence **the one rule** ([`pair_render_files`]):
//!
//! - a render written in both formats **leaves as its PNG**; its TIFF is its
//!   *full-resolution twin*, which leaves only when the person joins the TIFF
//!   for that sending (the box « Joindre aussi les TIFF », whose starting
//!   value is [`crate::local_settings::RendersUpload`]);
//! - a render written in one format only leaves as that file: the linear
//!   stack ("No processing") exists only as a TIFF, and the person asked for
//!   it by ticking its pipeline;
//! - the small picture of the plan (`*.preview.png`) is never a render.
//!
//! The person's own final image (`<target>/final/`) is not a render: this
//! module never sees it, and it leaves as it always did.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The suffix of the small picture `plan_stack.py` writes beside the record of
/// a plan: never a render, never sent.
pub const PREVIEW_SUFFIX: &str = ".preview.png";

/// One file of a render, as the queue needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderFile {
    pub path: PathBuf,
    pub sha256: String,
    pub size: u64,
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
}

/// A TIFF, by its name.
#[must_use]
pub fn is_tiff(path: &Path) -> bool {
    matches!(extension(path).as_deref(), Some("tif" | "tiff"))
}

/// A PNG, by its name.
#[must_use]
pub fn is_png(path: &Path) -> bool {
    extension(path).as_deref() == Some("png")
}

/// A file the stacker writes as a render: a TIFF or a PNG, never the small
/// picture of a plan.
#[must_use]
pub fn is_render_file(path: &Path) -> bool {
    let preview = path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.to_ascii_lowercase().ends_with(PREVIEW_SUFFIX));
    !preview && (is_tiff(path) || is_png(path))
}

/// What names a render whatever its format: its path without the extension.
fn render_key(path: &Path) -> PathBuf {
    path.with_extension("")
}

/// The render files of `items` paired, in their order: for each render, the
/// file that leaves (its PNG when it has one, else its only file) and its
/// full-resolution twin (the TIFF of the same name, beside a PNG). Items that
/// are not render files ([`is_render_file`]) are left out.
///
/// Each pair keeps the place of the first of its two files, so the order of
/// the renders the stacker reported is the order of the pairs.
pub fn pair_render_files<T>(items: Vec<T>, path_of: impl Fn(&T) -> &Path) -> Vec<(T, Option<T>)> {
    let mut pairs: Vec<(T, Option<T>)> = Vec::new();
    let mut keys: Vec<PathBuf> = Vec::new();
    for item in items {
        let path = path_of(&item);
        if !is_render_file(path) {
            continue;
        }
        let key = render_key(path);
        let png = is_png(path);
        match keys.iter().position(|k| *k == key) {
            // The second format of a render already met: the PNG leaves, the
            // TIFF becomes its twin.
            Some(at) if pairs[at].1.is_none() => {
                let first_is_png = is_png(path_of(&pairs[at].0));
                if png && !first_is_png {
                    let tiff = std::mem::replace(&mut pairs[at].0, item);
                    pairs[at].1 = Some(tiff);
                } else if !png && first_is_png {
                    pairs[at].1 = Some(item);
                } else {
                    // Two TIFF of one name (`.tif` and `.tiff`): each its own.
                    keys.push(key);
                    pairs.push((item, None));
                }
            }
            _ => {
                keys.push(key);
                pairs.push((item, None));
            }
        }
    }
    pairs
}

/// Whether a file of a render leaves with this sending: always, unless it is
/// the full-resolution twin of a PNG (`is_twin`) and the TIFF are not joined.
#[must_use]
pub fn leaves(is_twin: bool, include_tiff: bool) -> bool {
    !is_twin || include_tiff
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paired(names: &[&str]) -> Vec<(String, Option<String>)> {
        pair_render_files(
            names.iter().map(|n| PathBuf::from(*n)).collect(),
            PathBuf::as_path,
        )
        .into_iter()
        .map(|(a, b)| (a.display().to_string(), b.map(|b| b.display().to_string())))
        .collect()
    }

    #[test]
    fn a_render_in_both_formats_leaves_as_its_png_with_the_tiff_as_its_twin_in_either_order() {
        assert_eq!(
            paired(&["s/M31_raw.tif", "s/M31_raw.png"]),
            [("s/M31_raw.png".into(), Some("s/M31_raw.tif".into()))]
        );
        assert_eq!(
            paired(&["s/M31_raw.png", "s/M31_raw.tif"]),
            [("s/M31_raw.png".into(), Some("s/M31_raw.tif".into()))]
        );
    }

    #[test]
    fn the_linear_render_has_no_png_and_leaves_as_its_tiff() {
        assert_eq!(
            paired(&[
                "s/M31_raw.tif",
                "s/M31_raw.png",
                "s/M31_linear.tif",
                "s/M31_dualband-correction.png",
                "s/M31_dualband-correction.tif",
            ]),
            [
                ("s/M31_raw.png".into(), Some("s/M31_raw.tif".into())),
                ("s/M31_linear.tif".into(), None),
                (
                    "s/M31_dualband-correction.png".into(),
                    Some("s/M31_dualband-correction.tif".into())
                ),
            ]
        );
    }

    #[test]
    fn the_preview_of_a_plan_and_other_files_are_never_renders() {
        assert_eq!(
            paired(&[
                "p/IC1396_plan.preview.png",
                "p/IC1396_plan.json",
                "p/IC1396_plan_raw.jpg",
                "p/IC1396_plan_raw.PNG",
            ]),
            [("p/IC1396_plan_raw.PNG".into(), None)]
        );
    }

    #[test]
    fn a_twin_leaves_only_when_the_tiff_are_joined() {
        assert!(leaves(false, false));
        assert!(leaves(false, true));
        assert!(!leaves(true, false));
        assert!(leaves(true, true));
    }
}

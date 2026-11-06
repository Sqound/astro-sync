//! The image a device already stacked, offered as the master of a target
//! ("the device's master first").
//!
//! A Seestar writes, beside its sub-frames, `Stacked_<n>_<target>_<frame>_<filter>_<date>.fit`:
//! the stack of the night it made itself, linear, RGB, 16 bits, with a field
//! solution. Astro Sync used to set such a file aside as a Live stack of the
//! ASIAIR; it now **names** it and, for a device it knows, proposes it as
//! the master of the target (the renders and the contrast are applied on top
//! of it, nothing is stacked again).
//!
//! - **What is a device image.** A FITS file whose name starts with
//!   `Stacked` **and** whose header says a known device (`CREATOR`, today the
//!   Seestar) **and** a linear image (the colour cube a Seestar writes: 16
//!   bits, three channels, the unsigned offset -- an image stretched for the
//!   eyes is a `.jpg`, which is never read here). The headers are all the
//!   detection reads, never a pixel: the pixel guard (a stretched image is
//!   refused, `star_quality.py`'s `stretched`) is the one of the use, the
//!   measure and the render. An ASIAIR Live stack, any `Stacked…` of another
//!   writer, a master of PixInsight: not recognised, unchanged.
//! - **What a family is.** The images are grouped like the poses are
//!   ([`crate::stacking::family_label`]: filter, exposure, gain, binning).
//!   A family is **covered** by an image when that image is the only one of
//!   its family; two images or more **split** it (the Seestar cuts its stack
//!   when the target moves or the capture restarts: the real night of
//!   10/10/2026 has 28 + 8 poses of 10 s).
//! - **What is taken by default**: a covered family takes
//!   the device image; a split family restacks the poses when they are on the
//!   computer, and otherwise takes the biggest image and names the poses it
//!   leaves aside ([`DeviceFamily::aside_poses`]); Astro Sync never combines
//!   two device images.
//! - **Where the file lives**: `<target>/device/` of the night (the working
//!   folder's [`crate::working_folder::DEVICE_DIR`]), where "Tidy the
//!   files" puts it (copy, move or link, as for the poses) and where the
//!   detection of the night finds it.

use serde::{Deserialize, Serialize};

use crate::analysis::fits::{AcquisitionFamily, FitsHeader};

/// The devices whose stacked image is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceKind {
    Seestar,
}

/// What the header of a device image says.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceImageInfo {
    pub kind: DeviceKind,
    /// « Seestar S50 Pro »: the writer's name without the maker's (the
    /// `TELESCOP` of a Seestar carries an identifier of the unit: never kept).
    pub device: String,
    /// The poses it stacked (`STACKCNT`).
    pub poses: u64,
    pub exposure_seconds: Option<f64>,
    /// The exposure it holds in all (`TOTALEXP`), seconds.
    pub total_seconds: Option<f64>,
    pub filter: Option<String>,
    pub gain: Option<f64>,
    pub width: u32,
    pub height: u32,
    /// The header carries a field solution (`CTYPE1`, `CRVAL1`: the Seestar
    /// solves its stack): said by the window, never used to annotate yet.
    pub solved: bool,
    /// The family label of [`crate::stacking::family_label`] (`IRCUT_30s_g200`).
    pub family: String,
}

/// `Stacked…` at the start of a file's name, case ignored.
#[must_use]
pub fn is_stacked_name(file_name: &str) -> bool {
    file_name.to_lowercase().starts_with("stacked")
}

fn device_of(creator: &str) -> Option<(DeviceKind, String)> {
    let lowered = creator.to_lowercase();
    let at = lowered.find("seestar")?;
    let name = creator[at..].trim();
    (!name.is_empty()).then(|| (DeviceKind::Seestar, name.to_string()))
}

fn text(header: &FitsHeader, key: &str) -> Option<String> {
    header
        .get(key)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// The family label of a frame or an image from what its header says.
#[must_use]
pub fn label_of(
    filter: Option<&str>,
    exposure_seconds: Option<f64>,
    gain: Option<f64>,
    binning: Option<&str>,
) -> String {
    crate::stacking::family_label(&AcquisitionFamily {
        key: String::new(),
        filter: filter.map(str::to_string),
        exposure_seconds,
        gain,
        binning: binning.map(str::to_string),
        temperature_class_c: None,
        camera: None,
        frame_paths: Vec::new(),
    })
}

/// Whether the file is a stacked image of a known device (see the module's
/// documentation); `None` for anything else, a Live stack of the ASIAIR
/// included.
#[must_use]
pub fn recognise(header: &FitsHeader, file_name: &str) -> Option<DeviceImageInfo> {
    if !is_stacked_name(file_name) {
        return None;
    }
    let (kind, device) = device_of(&text(header, "CREATOR")?)?;
    let poses = header.get_f64("STACKCNT").filter(|n| *n >= 2.0)? as u64;
    // The colour cube a Seestar writes: 16 bits, three channels, unsigned
    // (BZERO 32768). Nothing else is linear data of this device.
    let linear_cube = header.get_i64("BITPIX") == Some(16)
        && header.get_i64("NAXIS") == Some(3)
        && header.get_i64("NAXIS3") == Some(3)
        && header
            .get_f64("BZERO")
            .is_none_or(|b| (b - 32768.0).abs() < 0.5);
    if !linear_cube {
        return None;
    }
    let exposure_seconds = crate::analysis::fits::exposure_seconds(header);
    let filter = text(header, "FILTER");
    let gain = header.get_f64("GAIN");
    let binning = match (header.get_i64("XBINNING"), header.get_i64("YBINNING")) {
        (Some(x), Some(y)) => Some(format!("{x}x{y}")),
        (Some(x), None) => Some(format!("{x}x{x}")),
        _ => None,
    };
    Some(DeviceImageInfo {
        kind,
        device,
        poses,
        exposure_seconds,
        total_seconds: header.get_f64("TOTALEXP"),
        family: label_of(
            filter.as_deref(),
            exposure_seconds,
            gain,
            binning.as_deref(),
        ),
        filter,
        gain,
        width: header.get_i64("NAXIS1").unwrap_or(0).max(0) as u32,
        height: header.get_i64("NAXIS2").unwrap_or(0).max(0) as u32,
        solved: header.get("CTYPE1").is_some() && header.get_f64("CRVAL1").is_some(),
    })
}

/// One device image of a target's `device/` folder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceImage {
    /// Relative to the night's folder, forward slashes.
    pub relative_path: String,
    /// The file's name (what `StackJob` and the window name it by).
    pub file_name: String,
    pub bytes: u64,
    #[serde(flatten)]
    pub info: DeviceImageInfo,
}

/// How the device images meet a family of poses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Coverage {
    /// One image only for the family: it covers it.
    Covers,
    /// Two images or more: none contains the family.
    Split,
}

/// Where a family's image comes from when nothing is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// The image of the device (the file in [`DeviceFamily::use_file`]).
    Device,
    /// A new stacking of the poses.
    Restack,
}

/// A family of the target that has device images: what covers it and what
/// is taken by default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceFamily {
    pub label: String,
    pub filter: Option<String>,
    pub exposure_seconds: Option<f64>,
    pub gain: Option<f64>,
    /// The poses of the family in `lights/` (0: they are not on this
    /// computer).
    pub lights: usize,
    /// The file names of its images, the biggest first (most poses, then
    /// name).
    pub images: Vec<String>,
    pub coverage: Coverage,
    /// The default: [`Source::Device`] for a covered family and for a split
    /// one whose poses are not here (the biggest image, naming what it
    /// leaves aside); [`Source::Restack`] for a split family whose poses are
    /// here.
    pub default_source: Source,
    /// The image the device choice takes (`None`: the family splits and its
    /// poses are here, so the choice is unavailable).
    pub use_file: Option<String>,
    /// The poses of the images the device choice leaves aside (the images
    /// that are not [`use_file`](Self::use_file)): said by the window.
    pub aside_poses: u64,
}

impl DeviceFamily {
    /// Restacking is possible: the poses of the family are on this computer.
    #[must_use]
    pub fn restackable(&self) -> bool {
        self.lights > 0
    }
}

/// The poses of the target's `lights/`, grouped by family: what the
/// coverage is compared with.
#[derive(Debug, Clone, PartialEq)]
pub struct LightFamily {
    pub label: String,
    pub frames: usize,
}

/// The families that have device images, the biggest first, with their
/// coverage and their default (see the module's documentation).
#[must_use]
pub fn families(images: &[DeviceImage], lights: &[LightFamily]) -> Vec<DeviceFamily> {
    let mut labels: Vec<&str> = Vec::new();
    for image in images {
        if !labels.contains(&image.info.family.as_str()) {
            labels.push(image.info.family.as_str());
        }
    }
    let mut out: Vec<DeviceFamily> = labels
        .into_iter()
        .map(|label| {
            let mut mine: Vec<&DeviceImage> = images
                .iter()
                .filter(|image| image.info.family == label)
                .collect();
            mine.sort_by(|a, b| {
                b.info
                    .poses
                    .cmp(&a.info.poses)
                    .then_with(|| a.file_name.cmp(&b.file_name))
            });
            let first = mine[0];
            let lights_here = lights
                .iter()
                .find(|family| family.label == label)
                .map_or(0, |family| family.frames);
            let coverage = if mine.len() == 1 {
                Coverage::Covers
            } else {
                Coverage::Split
            };
            let restack_first = coverage == Coverage::Split && lights_here > 0;
            let use_file = (!restack_first).then(|| first.file_name.clone());
            let aside_poses = if restack_first {
                0
            } else {
                mine.iter().skip(1).map(|image| image.info.poses).sum()
            };
            DeviceFamily {
                label: label.to_string(),
                filter: first.info.filter.clone(),
                exposure_seconds: first.info.exposure_seconds,
                gain: first.info.gain,
                lights: lights_here,
                images: mine.iter().map(|image| image.file_name.clone()).collect(),
                coverage,
                default_source: if restack_first {
                    Source::Restack
                } else {
                    Source::Device
                },
                use_file,
                aside_poses,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        let poses = |family: &DeviceFamily| {
            images
                .iter()
                .filter(|image| image.info.family == family.label)
                .map(|image| image.info.poses)
                .max()
                .unwrap_or(0)
        };
        poses(b).cmp(&poses(a)).then_with(|| a.label.cmp(&b.label))
    });
    out
}

/// What the device choice of a family says it uses: the file (a name inside
/// `<target>/device/`, never a path).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceUse {
    pub file: String,
}

/// A file's name that cannot leave `<target>/device/`.
#[must_use]
pub fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', '\0'])
        && !name.starts_with('.')
}

#[cfg(test)]
mod tests;

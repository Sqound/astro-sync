//! What is where in a night's folder, decided by the folders and by the
//! first header of each frame. Deterministic: the same folder always gives the same findings.
//!
//! Each frame (FITS or XISF) is read for its **first header only**, never a
//! pixel: `IMAGETYP` (normalised: lower case, « frame » and spaces removed;
//! `light`, `dark`, `flat`, `darkflat`/`flatdark`, `bias`/`offset`),
//! `OBJECT` (compared with each target's designation and catalogue aliases,
//! case and spaces ignored: « M 31 » = « M31 »), `FILTER`, `EXPTIME`,
//! `GAIN`, `DATE-OBS`. Every rule of the sheet's table is a [`Finding`]
//! with a stable message key (`detection.*`), its parameters, and the files
//! it is about, for the ⓘ. **No finding ever blocks**: a calibration gap is
//! a warning, a misfiled frame says where it should go, a missing log is
//! information. **In a calibration folder the folder is right**: a frame of `darks/`,
//! `flats/`, `dark-flats/` or `biases/` is read, counted and matched as that
//! folder's type whatever its header says (the ASIAIR writes `Light` on darks
//! taken in its « Light » mode); when the header says otherwise a warning
//! ([`keys::CALIBRATION_HEADER_DIFFERS`]) asks to check, it never moves
//! anything. A calibration frame in a target's `lights/` stays misfiled. What decides whether a target's capture can be made is the
//! **minimum per target**: a final image in `final/`, already online, or
//! produced by the stacking.
//!
//! The folder is read again at each opening and on "Check again";
//! [`NightDetection::fingerprint`] (and [`night_folder_fingerprint`], which
//! reads no file) lets the window see cheaply that something changed. No
//! file-system watcher: none is a dependency of the core, and none is added
//! (the rule here).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::analysis::fits::{self, FitsHeader};
use crate::device_master::{self, DeviceFamily, DeviceImage, LightFamily};
use crate::local_work::{CancelFlag, WorkKind, WorkProgress};
use crate::nfc::{same_name, to_nfc};
use crate::scanner::{self, FileClass, FolderWalk, LinkIssue, LinkProblem, ScanError, WalkedFile};
use crate::time_util::parse_iso8601_utc_to_unix_seconds;
use crate::working_folder::{
    CALIBRATION_DIR, CALIBRATION_KINDS, DEVICE_DIR, FINAL_DIR, LIGHTS_DIR, LOGS_DIR,
    PROGRESSION_DIR, STACKED_DIR, target_folder_name, target_folders_by_identity,
};

/// The length of a night's window: from local noon of its evening to local
/// noon of the next day.
pub const NIGHT_WINDOW_SECONDS: f64 = 24.0 * 3600.0;

/// The extensions of a final image the portal accepts.
pub const FINAL_IMAGE_EXTENSIONS: [&str; 5] = ["tif", "tiff", "jpg", "jpeg", "png"];

#[derive(Debug, Error)]
pub enum DetectionError {
    #[error(transparent)]
    Scan(#[from] ScanError),
}

impl DetectionError {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            DetectionError::Scan(ScanError::NotADirectory(_)) => "folder_not_found",
            DetectionError::Scan(ScanError::Cancelled) => "cancelled",
            DetectionError::Scan(_) => "io_error",
        }
    }
}

/* ------------------------------ The header ------------------------------ */

/// A frame's type, from `IMAGETYP`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FrameType {
    Light,
    Dark,
    Flat,
    DarkFlat,
    Bias,
}

impl FrameType {
    /// The calibration folder this type belongs in (`None` for a light).
    #[must_use]
    pub fn calibration_folder(self) -> Option<&'static str> {
        match self {
            FrameType::Light => None,
            FrameType::Dark => Some("darks"),
            FrameType::Flat => Some("flats"),
            FrameType::DarkFlat => Some("dark-flats"),
            FrameType::Bias => Some("biases"),
        }
    }

    /// The type a calibration folder holds.
    #[must_use]
    pub fn of_calibration_folder(folder: &str) -> Option<Self> {
        match folder {
            "darks" => Some(FrameType::Dark),
            "flats" => Some(FrameType::Flat),
            "dark-flats" => Some(FrameType::DarkFlat),
            "biases" | crate::working_folder::BIAS_ALIAS => Some(FrameType::Bias),
            _ => None,
        }
    }

    /// As a header writes it, for the messages ("their header says FLAT").
    #[must_use]
    pub fn header_word(self) -> &'static str {
        match self {
            FrameType::Light => "LIGHT",
            FrameType::Dark => "DARK",
            FrameType::Flat => "FLAT",
            FrameType::DarkFlat => "DARKFLAT",
            FrameType::Bias => "BIAS",
        }
    }

    fn name(self, count: usize) -> &'static str {
        let many = count > 1;
        match self {
            FrameType::Light if many => "lights",
            FrameType::Light => "light",
            FrameType::Dark if many => "darks",
            FrameType::Dark => "dark",
            FrameType::Flat if many => "flats",
            FrameType::Flat => "flat",
            FrameType::DarkFlat if many => "dark-flats",
            FrameType::DarkFlat => "dark-flat",
            FrameType::Bias => "bias",
        }
    }
}

/// `IMAGETYP` normalised (sheet § 3): lower case, « frame », spaces, `-`
/// and `_` removed; « Light Frame » → light, « Flat-Dark » → dark-flat,
/// « Offset » → bias. Anything else is unknown (`None`).
#[must_use]
pub fn normalize_image_type(raw: &str) -> Option<FrameType> {
    let lowered = raw.to_lowercase().replace("frame", "");
    let squeezed: String = lowered
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
        .collect();
    match squeezed.as_str() {
        "light" => Some(FrameType::Light),
        "dark" => Some(FrameType::Dark),
        "flat" => Some(FrameType::Flat),
        "darkflat" | "flatdark" => Some(FrameType::DarkFlat),
        "bias" | "offset" => Some(FrameType::Bias),
        _ => None,
    }
}

/// A name compared without case or spaces, in NFC (« M 31 » = « m31 »).
#[must_use]
pub fn normalize_object(raw: &str) -> String {
    to_nfc(raw)
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

/// What the detection reads in a frame's first header.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameHeaderRecord {
    pub image_type: Option<FrameType>,
    pub object: Option<String>,
    pub filter: Option<String>,
    pub exposure_seconds: Option<f64>,
    pub gain: Option<f64>,
    /// `1x1`: with the filter, the exposure and the gain it names the family
    /// the frame stacks with ([`crate::stacking::family_label`]).
    pub binning: Option<String>,
    pub date_obs_utc: Option<String>,
    /// `date_obs_utc` is the camera clock as written, its zone unknown (a
    /// RAW file): the night window compares it with the wall clock of the
    /// night.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub date_zone_unknown: bool,
}

#[must_use]
pub fn frame_record(header: &FitsHeader) -> FrameHeaderRecord {
    let acquisition = fits::extract_acquisition_record(header);
    let text = |keyword: &str| {
        header
            .get(keyword)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    FrameHeaderRecord {
        image_type: header
            .get("IMAGETYP")
            .or_else(|| header.get("FRAMETYP"))
            .and_then(normalize_image_type),
        object: text("OBJECT"),
        filter: acquisition
            .filter
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty()),
        exposure_seconds: acquisition.exposure_seconds,
        gain: acquisition.gain,
        binning: acquisition.binning,
        date_obs_utc: acquisition.date_obs_utc,
        date_zone_unknown: acquisition.date_zone_unknown,
    }
}

/// The first header of a frame (FITS, XISF or a camera RAW file), `None`
/// when it cannot be read. Reads the header's bytes only (a RAW file: its
/// EXIF block, as FITS-style cards, `crate::raw_header`).
#[must_use]
pub fn read_frame_header(path: &Path, class: FileClass) -> Option<FitsHeader> {
    match class {
        FileClass::Fits => fits::read_primary_header_file(path).ok(),
        FileClass::Xisf => crate::xisf::read_header_file(path).ok(),
        FileClass::Raw => crate::raw_header::read_header_file(path).ok(),
        _ => None,
    }
}

/* ------------------------------- The input ------------------------------ */

/// A target of the night, as the detection needs it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectionTarget {
    pub target_id: String,
    pub designation: String,
    /// The catalogue aliases (« Andromeda Galaxy », « NGC 224 »…).
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Ticked at "Targets": the rules apply to ticked targets.
    pub ticked: bool,
    /// The target already has its final image online (phone, web, an
    /// earlier sending).
    #[serde(default)]
    pub online_final: bool,
    /// "Stacking from the FITS" is ticked for this target.
    #[serde(default)]
    pub stacking_planned: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectionInput {
    /// `YYYY-MM-DD`, the evening of the night: with the longitude, it alone
    /// draws the night's window ([`NightWindow`]).
    #[serde(default)]
    pub evening_date: Option<String>,
    /// The night's start, ISO 8601 UTC: read only when the evening date is
    /// unknown, to find it (never to draw the window itself).
    #[serde(default)]
    pub night_started_at: Option<String>,
    /// The longitude of the night's place, degrees east: local noon is
    /// `12:00 UTC - longitude / 15 h`. Unknown: UTC.
    #[serde(default)]
    pub longitude: Option<f64>,
    pub targets: Vec<DetectionTarget>,
}

/* ------------------------------ The findings ---------------------------- */

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    /// Said, nothing to do ("No log").
    Info,
    /// Said, something to check or move; never blocks the sending.
    Warning,
    /// A target with nothing to send (no FITS frame, no final image).
    Error,
}

/// The message keys, one per rule of the sheet's table. An interface looks
/// them up in its dictionaries with the finding's `params`.
pub mod keys {
    /// No final image in `<target>/final/` (nor online, nor stacked), but
    /// FITS frames: a warning, the frames alone are used.
    pub const FINAL_MISSING: &str = "detection.finalMissing";
    /// No final image, but the image a device stacked: it is the source of
    /// the families it covers, and only the poses it does not cover are
    /// stacked ("only the FITS frames will be used" was false there).
    /// `restackFrames` is the number of those poses.
    pub const FINAL_FROM_DEVICE: &str = "detection.finalFromDevice";
    /// FITS frames missing, a final image there (or online): a warning, the
    /// final image alone is sent.
    pub const FRAMES_MISSING: &str = "detection.framesMissing";
    /// Neither FITS frame nor final image: an error, nothing to send.
    pub const TARGET_EMPTY: &str = "detection.targetEmpty";
    /// A FITS or XISF in `final/`: the portal does not take it.
    pub const FINAL_FRAME_REFUSED: &str = "detection.finalFitsRefused";
    /// Frames whose header type is not the folder's: from where to where.
    pub const FRAMES_MISFILED: &str = "detection.framesMisfiled";
    /// Frames of a calibration folder whose header type is not the
    /// folder's: the folder is right, the frames are read as its type; a
    /// warning to check, never a "tidying".
    pub const CALIBRATION_HEADER_DIFFERS: &str = "detection.calibrationHeaderDiffers";
    /// A target's lights in another target's folder (header `OBJECT`).
    pub const FRAMES_OTHER_TARGET: &str = "detection.framesOtherTarget";
    /// No flats of a filter of a target's lights.
    pub const FLATS_MISSING: &str = "detection.flatsMissing";
    /// No darks of an exposure and gain of a target's lights.
    pub const DARKS_MISSING: &str = "detection.darksMissing";
    /// No NINA, PHD2 or ASIAIR log in `logs/`.
    pub const NO_LOGS: &str = "detection.noLogs";
    /// Lights taken outside the night's window (`DATE-OBS`, [`NightWindow`]):
    /// a warning; the frames stay counted in the folder but the stacking
    /// leaves them out of the night's integration.
    pub const FRAMES_OTHER_NIGHT: &str = "detection.framesOtherNight";
    /// Files at the root of the night or of a target, or in no known folder.
    pub const FILES_OUTSIDE_STRUCTURE: &str = "detection.filesOutsideStructure";
    /// "Link to a missing disk".
    pub const LINK_TARGET_MISSING: &str = "detection.linkTargetMissing";
    /// A loop of symbolic links.
    pub const LINK_LOOP: &str = "detection.linkLoop";
}

/// One finding: a rule of the sheet, its parameters, the files concerned.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub key: &'static str,
    pub severity: Severity,
    /// The target it is about, when it is about one.
    pub target_id: Option<String>,
    pub params: Map<String, Value>,
    /// Forward-slash paths relative to the night folder (the ⓘ detail).
    pub files: Vec<String>,
}

impl Finding {
    fn new(key: &'static str, severity: Severity, target_id: Option<&str>, params: Value) -> Self {
        let Value::Object(params) = params else {
            unreachable!("finding parameters are always an object");
        };
        Self {
            key,
            severity,
            target_id: target_id.map(str::to_string),
            params,
            files: Vec::new(),
        }
    }

    fn with_files(mut self, files: Vec<String>) -> Self {
        self.files = files;
        self
    }

    fn text(&self, name: &str) -> &str {
        self.params.get(name).and_then(Value::as_str).unwrap_or("")
    }

    fn count(&self) -> usize {
        self.params
            .get("count")
            .and_then(Value::as_u64)
            .map_or(0, |c| c as usize)
    }

    /// The finding in English, as the sheet writes it (the command line, and
    /// the reference the window's dictionaries follow).
    #[must_use]
    pub fn sentence(&self) -> String {
        let count = self.count();
        let s = if count > 1 { "s" } else { "" };
        match self.key {
            keys::FINAL_MISSING => {
                "No final image: only the FITS frames will be used to create one (stacking)."
                    .to_string()
            }
            keys::FINAL_FROM_DEVICE => {
                let restack = self
                    .params
                    .get("restackFrames")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                if restack == 0 {
                    "No final image: the device’s image will be used as the source.".to_string()
                } else {
                    format!(
                        "No final image: the device’s image will be used as the source; the {restack} other FITS frames will be stacked to create a final image."
                    )
                }
            }
            keys::FRAMES_MISSING => {
                "No FITS frames: only the final image will be sent.".to_string()
            }
            keys::TARGET_EMPTY => format!(
                "Neither FITS frames nor a final image: put the frames in {} or a final image (TIFF, JPEG or PNG) in {}.",
                self.text("lightsFolder"),
                self.text("folder")
            ),
            keys::FINAL_FRAME_REFUSED => {
                "The service does not take FITS: export the final image as TIFF, JPEG or PNG."
                    .to_string()
            }
            keys::FRAMES_MISFILED => {
                let kind = self
                    .params
                    .get("kind")
                    .and_then(|k| serde_json::from_value::<FrameType>(k.clone()).ok())
                    .unwrap_or(FrameType::Light);
                let from = self.text("from");
                let to = self.text("to");
                let header = self.text("header");
                let filed = "filed in";
                let destination = if to.is_empty() {
                    "to move into the lights/ folder of their target".to_string()
                } else {
                    format!("to move into {to}")
                };
                if self.params.get("fromLights").and_then(Value::as_bool) == Some(true) {
                    let their = if count > 1 { "their" } else { "its" };
                    format!(
                        "{count} {} {filed} {from}: {their} header says {header}. {}.",
                        kind.name(count),
                        capitalize(&destination)
                    )
                } else {
                    format!(
                        "{count} {} {filed} {from}: {destination}.",
                        kind.name(count)
                    )
                }
            }
            keys::CALIBRATION_HEADER_DIFFERS => {
                let kind = self
                    .params
                    .get("kind")
                    .and_then(|k| serde_json::from_value::<FrameType>(k.clone()).ok())
                    .unwrap_or(FrameType::Dark);
                let (have, read) = if count > 1 {
                    ("have", "they are read")
                } else {
                    ("has", "it is read")
                };
                format!(
                    "{count} frame{s} from {} {have} a {} header: {read} as {}{}. Check that this is not a filing mistake.",
                    self.text("folder"),
                    self.text("header"),
                    if count > 1 { "" } else { "a " },
                    kind.name(count)
                )
            }
            keys::FRAMES_OTHER_TARGET => format!(
                "{count} frame{s} of {} in {} (OBJECT header).",
                self.text("object"),
                self.text("folder")
            ),
            keys::FLATS_MISSING => match self.params.get("filter").and_then(Value::as_str) {
                Some(filter) => {
                    format!("No flat for the {filter} filter ({}).", self.text("target"))
                }
                None => format!("No flat ({}).", self.text("target")),
            },
            keys::DARKS_MISSING => {
                let number_of = |value: &Value| {
                    value
                        .get("exposureSeconds")
                        .and_then(Value::as_f64)
                        .map(plain_number)
                        .unwrap_or_default()
                };
                let exposure = self
                    .params
                    .get("exposureSeconds")
                    .and_then(Value::as_f64)
                    .map(plain_number)
                    .unwrap_or_default();
                let gain = self
                    .params
                    .get("gain")
                    .and_then(Value::as_f64)
                    .map(|gain| format!(" · gain {}", plain_number(gain)))
                    .unwrap_or_default();
                if self.params.get("none").and_then(Value::as_bool) == Some(true) {
                    format!("No dark of {exposure} s{gain} ({}).", self.text("target"))
                } else {
                    let found = self
                        .params
                        .get("found")
                        .and_then(Value::as_array)
                        .map(|list| list.iter().map(number_of).collect::<Vec<_>>().join(", "))
                        .unwrap_or_default();
                    let count = self.count();
                    let (the, are) = if count > 1 {
                        (format!("the {count} darks"), "are")
                    } else {
                        ("the dark".to_string(), "is")
                    };
                    format!(
                        "No dark of {exposure} s{gain} for {}: {the} in the folder {are} {found} s.",
                        self.text("target")
                    )
                }
            }
            keys::NO_LOGS => "No log: no steps, no guiding quality.".to_string(),
            keys::FRAMES_OTHER_NIGHT => {
                let (pronoun, verb) = if count > 1 {
                    ("they", "stay")
                } else {
                    ("it", "stays")
                };
                format!(
                    "{count} frame{s} outside the night (night of {}), left out of the integration; {pronoun} {verb} counted in {}.",
                    short_date(self.text("date")),
                    self.text("folder")
                )
            }
            keys::FILES_OUTSIDE_STRUCTURE => {
                let them = if count > 1 { "them" } else { "it" };
                format!("{count} file{s} outside the structure: see where to file {them}.")
            }
            keys::LINK_TARGET_MISSING => {
                format!("Link to a missing disk: {}.", self.text("path"))
            }
            keys::LINK_LOOP => format!("Link that loops back on itself: {}.", self.text("path")),
            other => other.to_string(),
        }
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

/// `120` → "120", `0.5` → "0.5".
pub(crate) fn plain_number(value: f64) -> String {
    if (value - value.round()).abs() < 1e-9 {
        format!("{}", value.round() as i64)
    } else {
        let text = format!("{value:.3}");
        text.trim_end_matches('0').to_string()
    }
}

/// `2026-10-05` → "5 Oct".
fn short_date(iso: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut parts = iso.split('-');
    let (Some(_), Some(month), Some(day)) = (parts.next(), parts.next(), parts.next()) else {
        return iso.to_string();
    };
    match (month.parse::<usize>(), day.parse::<u32>()) {
        (Ok(month @ 1..=12), Ok(day)) => format!("{day} {}", MONTHS[month - 1]),
        _ => iso.to_string(),
    }
}

/* ------------------------------- The result ----------------------------- */

/// How a target meets the minimum (a final image), or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MinimumSource {
    /// A final image in `<target>/final/`.
    FinalFile,
    /// The final image is already online.
    Online,
    /// The stacking will produce it.
    Stacking,
    /// None: the target is "to complete", its capture is not created.
    Missing,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetDetection {
    pub target_id: String,
    pub designation: String,
    /// The target's folder in the night, when there is one.
    pub folder_name: Option<String>,
    /// `false`: a folder kept for a target no longer ticked, "outside the
    /// night".
    pub in_night: bool,
    /// Frames in `lights/` whose header says light and names no other
    /// target.
    pub light_frames: usize,
    /// The filters of those frames, sorted (`FILTER`).
    pub filters: Vec<String>,
    /// The final image proposed: the most recent in `final/`.
    pub final_image: Option<String>,
    /// The other final images of `final/`, most recent first.
    pub other_final_images: Vec<String>,
    pub minimum: MinimumSource,
    /// The size of the final image proposed ("412 MB").
    pub final_bytes: Option<u64>,
    /// What "Files" says of the lights: frames per filter, exposures,
    /// gains, bytes.
    pub light_summary: FrameSummary,
    /// The images a device already stacked, found in the target's `device/`;
    /// `None` when there is none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<DeviceDetection>,
}

/// What the target's `device/` folder holds: the
/// images of a known device, how they meet the families of the poses, and
/// the `.jpg` that is never a master.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceDetection {
    /// The recognised images (a stacked image of a known device, linear).
    pub images: Vec<DeviceImage>,
    /// One entry per family that has an image, the biggest first.
    pub families: Vec<DeviceFamily>,
    /// The `.jpg` of `device/` that has no recognised `.fit` of its own name
    /// (relative to the night's folder): stretched, so never a master.
    pub jpgs: Vec<String>,
}

/// Frames of one filter (`FILTER`; `None`: no filter written).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterCount {
    pub filter: Option<String>,
    pub frames: usize,
}

/// A set of frames in a line of "Files": "40 frames · 180 s · Ha 20,
/// OIII 20 · gain 100 · 1.3 GB".
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameSummary {
    /// Per filter, in the order of the filters' names (no filter last).
    pub filters: Vec<FilterCount>,
    /// The distinct exposures in seconds, longest first.
    pub exposures: Vec<f64>,
    /// The distinct gains, smallest first.
    pub gains: Vec<f64>,
    pub bytes: u64,
}

fn push_distinct(values: &mut Vec<f64>, value: Option<f64>) {
    if let Some(value) = value
        && !values.iter().any(|v| (v - value).abs() < 1e-3)
    {
        values.push(value);
    }
}

fn frame_summary<'a>(
    frames: impl Iterator<Item = (Option<&'a FrameHeaderRecord>, Option<String>, u64)>,
) -> FrameSummary {
    let mut per_filter: BTreeMap<Option<String>, usize> = BTreeMap::new();
    let mut summary = FrameSummary::default();
    for (record, filter, bytes) in frames {
        *per_filter.entry(filter).or_default() += 1;
        push_distinct(
            &mut summary.exposures,
            record.and_then(|r| r.exposure_seconds),
        );
        push_distinct(&mut summary.gains, record.and_then(|r| r.gain));
        summary.bytes += bytes;
    }
    // `None` sorts first in a map: written last, "no filter".
    let (named, unnamed): (Vec<_>, Vec<_>) = per_filter
        .into_iter()
        .partition(|(filter, _)| filter.is_some());
    summary.filters = named
        .into_iter()
        .chain(unnamed)
        .map(|(filter, frames)| FilterCount { filter, frames })
        .collect();
    summary
        .exposures
        .sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    summary
        .gains
        .sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    summary
}

/// The calibration as "Files" says it: the darks' exposures and gains,
/// the flats per filter.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalibrationSummary {
    pub darks: FrameSummary,
    pub flats: FrameSummary,
}

/// What kind of log a file of `logs/` is, read from its name and contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LogKind {
    Nina,
    Phd2,
    Asiair,
    /// A file of `logs/` none of the three readers knows (the `_CHN`
    /// Chinese rendition of an ASIAIR log among them: it carries no event of
    /// its own).
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedLog {
    pub relative_path: String,
    pub kind: LogKind,
    /// Its size ("Logs found": "... · 1.4 MB").
    pub bytes: u64,
}

/// The calibration frames found, by header type, anywhere in the night.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalibrationCounts {
    pub darks: usize,
    pub flats: usize,
    pub dark_flats: usize,
    pub biases: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NightDetection {
    pub folder: PathBuf,
    /// Changes whenever a file of the folder is added, removed, resized or
    /// touched ([`night_folder_fingerprint`]).
    pub fingerprint: String,
    pub targets: Vec<TargetDetection>,
    pub logs: Vec<DetectedLog>,
    pub calibration: CalibrationCounts,
    /// The darks and flats in words.
    pub calibration_summary: CalibrationSummary,
    pub findings: Vec<Finding>,
    /// Files at the root of the night or of a target, or in no known folder.
    pub outside_structure: Vec<String>,
    pub links: Vec<LinkIssue>,
}

impl NightDetection {
    /// The findings in English, one per line (the command line).
    #[must_use]
    pub fn sentences(&self) -> Vec<String> {
        self.findings.iter().map(Finding::sentence).collect()
    }
}

/// Where a stop switch and a progress sink come from.
#[derive(Default, Clone, Copy)]
pub struct DetectOptions<'a> {
    pub cancel: Option<&'a CancelFlag>,
    pub progress: Option<&'a (dyn Fn(&WorkProgress) + Sync)>,
}

/* -------------------------------- The rules ----------------------------- */

/// Misfiled frames of one (folder, type): where they go (`None`: the lights
/// folder of their target, unknown), whether they came from a `lights/`
/// folder, and the files.
type Misfiled = (Option<String>, bool, Vec<String>);

/// Where a file of the night is, by its folders.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Place {
    Logs,
    /// `calibration/<kind>/…`: the kind's folder name.
    Calibration(&'static str),
    /// `<target>/lights/…`, by index in the target list.
    Lights(usize),
    Final(usize),
    /// `<target>/device/…`: what a device stacked itself.
    Device(usize),
    /// `<target>/stacked/…` or `<target>/progression/…`: Astro Sync's own
    /// results (the stacking, the progression images).
    AstroSyncOutput,
    /// A folder kept for a target out of the night.
    OutOfNight,
    Outside,
}

/// A target folder of the night: its target index (in the input's list,
/// `None` for a folder whose target is not in the list at all) and name.
struct TargetDir {
    index: Option<usize>,
    name: String,
    in_night: bool,
}

/// The fingerprint of a night folder, without reading any file: its files'
/// paths, sizes and modification times, and its broken links.
pub fn night_folder_fingerprint(folder: &Path) -> Result<String, DetectionError> {
    Ok(fingerprint(&scanner::walk_folder(folder, None)?))
}

fn fingerprint(walk: &FolderWalk) -> String {
    let mut hasher = Sha256::new();
    for file in &walk.files {
        hasher.update(
            format!(
                "{}\0{}\0{}\n",
                file.relative_path, file.stamp.size, file.stamp.modified_ns
            )
            .as_bytes(),
        );
    }
    for link in &walk.links {
        hasher.update(format!("link\0{}\0{:?}\n", link.relative_path, link.problem).as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

/// Reads the night folder and applies every rule of the sheet's table.
pub fn detect_night(
    folder: &Path,
    input: &DetectionInput,
    options: &DetectOptions<'_>,
) -> Result<NightDetection, DetectionError> {
    // The window of the night says what the walk finds before any header is read.
    let report = |found: u64, current: &Path| {
        if let Some(progress) = options.progress {
            progress(&WorkProgress {
                kind: WorkKind::Scan,
                done: found,
                total: 0,
                bytes_done: 0,
                bytes_total: 0,
                current: Some(current.to_string_lossy().into_owned()),
            });
        }
    };
    let walk = scanner::walk_folder_reporting(folder, options.cancel, Some(&report))?;
    let fingerprint = fingerprint(&walk);
    let dirs = target_dirs(folder, input);

    // Where each file is.
    let placed: Vec<(&WalkedFile, Place)> = walk
        .files
        .iter()
        .map(|file| (file, place_of(&file.relative_path, &dirs)))
        .collect();

    // The first header of every frame of lights/ and calibration/.
    let to_read: Vec<usize> = placed
        .iter()
        .enumerate()
        .filter(|(_, (file, place))| {
            file.class.is_frame()
                && matches!(
                    place,
                    Place::Lights(_) | Place::Calibration(_) | Place::Device(_)
                )
        })
        .map(|(index, _)| index)
        .collect();
    let mut headers: BTreeMap<usize, FrameHeaderRecord> = BTreeMap::new();
    // The images of a known device in `<target>/device/`.
    let mut device_infos: BTreeMap<usize, device_master::DeviceImageInfo> = BTreeMap::new();
    let total = to_read.len() as u64;
    for (done, index) in to_read.iter().enumerate() {
        if options.cancel.is_some_and(CancelFlag::is_cancelled) {
            return Err(DetectionError::Scan(ScanError::Cancelled));
        }
        let file = placed[*index].0;
        if let Some(progress) = options.progress {
            progress(&WorkProgress {
                kind: WorkKind::Headers,
                done: done as u64,
                total,
                bytes_done: 0,
                bytes_total: 0,
                current: Some(file.relative_path.clone()),
            });
        }
        if let Some(header) = read_frame_header(&file.path, file.class) {
            if matches!(placed[*index].1, Place::Device(_)) {
                let name = file.relative_path.rsplit('/').next().unwrap_or_default();
                if let Some(info) = device_master::recognise(&header, name) {
                    device_infos.insert(*index, info);
                }
            } else {
                headers.insert(*index, frame_record(&header));
            }
        }
    }

    let mut findings: Vec<Finding> = Vec::new();
    let mut outside: Vec<String> = Vec::new();
    let mut logs: Vec<DetectedLog> = Vec::new();
    let mut calibration = CalibrationCounts::default();

    // Calibration available, by header type, wherever it lies.
    let mut flat_filters: BTreeSet<Option<String>> = BTreeSet::new();
    let mut dark_settings: Vec<(Option<f64>, Option<f64>)> = Vec::new();
    // Whether the night keeps its flats in one folder per filter
    // (`calibration/flats/<filter>/`): the stacking then gives a family the
    // flats of its own filter only.
    let mut flats_by_folder = false;
    // The darks and flats for the summary: (index, filter).
    let mut dark_frames: Vec<usize> = Vec::new();
    let mut flat_frames: Vec<(usize, Option<String>)> = Vec::new();

    // Per target: lights, filters, settings, final images.
    let mut lights: Vec<Vec<usize>> = vec![Vec::new(); input.targets.len()];
    let mut finals: Vec<Vec<&WalkedFile>> = vec![Vec::new(); input.targets.len()];
    let mut refused_finals: Vec<Vec<String>> = vec![Vec::new(); input.targets.len()];
    // Per target: the images of a known device and the `.jpg` of `device/`.
    let mut device_images: Vec<Vec<DeviceImage>> = vec![Vec::new(); input.targets.len()];
    let mut device_jpgs: Vec<Vec<String>> = vec![Vec::new(); input.targets.len()];

    // Misfiled frames, grouped by (from, type): (to, fromLights, files).
    let mut misfiled: BTreeMap<(String, FrameType), Misfiled> = BTreeMap::new();
    // Calibration frames whose header says another type than their
    // folder, grouped by (folder, folder's type, header's type).
    let mut header_differs: BTreeMap<(String, FrameType, FrameType), Vec<String>> = BTreeMap::new();
    // Lights of another target, grouped by (folder, other target).
    let mut other_target: BTreeMap<(String, usize, usize), Vec<String>> = BTreeMap::new();
    // Lights of another night, grouped by (folder, evening date).
    let mut other_night: BTreeMap<(String, String), (Option<usize>, Vec<String>)> = BTreeMap::new();

    let window = night_window(input);
    let longest_flat = longest_flat_exposure(&headers);

    for (index, (file, place)) in placed.iter().enumerate() {
        let header = headers.get(&index);
        // The header's type, a short `Dark` read as a dark-flat when the
        // folder does not say otherwise ([`resolve_dark_flat`]).
        let effective_type = |default: FrameType| {
            let kind = header
                .and_then(|record| record.image_type)
                .unwrap_or(default);
            resolve_dark_flat(kind, header, folder_type_of(place), longest_flat)
        };
        match place {
            Place::Logs => {
                logs.push(DetectedLog {
                    relative_path: file.relative_path.clone(),
                    kind: log_kind(file),
                    bytes: file.stamp.size,
                });
            }
            Place::AstroSyncOutput | Place::OutOfNight => {}
            Place::Outside => outside.push(file.relative_path.clone()),
            Place::Device(target) => {
                let extension = extension_of(&file.relative_path);
                if let Some(info) = device_infos.get(&index) {
                    device_images[*target].push(DeviceImage {
                        relative_path: file.relative_path.clone(),
                        file_name: file
                            .relative_path
                            .rsplit('/')
                            .next()
                            .unwrap_or_default()
                            .to_string(),
                        bytes: file.stamp.size,
                        info: info.clone(),
                    });
                } else if matches!(extension.as_str(), "jpg" | "jpeg") {
                    device_jpgs[*target].push(file.relative_path.clone());
                } else {
                    outside.push(file.relative_path.clone());
                }
            }
            Place::Final(target) => {
                let extension = extension_of(&file.relative_path);
                if file.class.is_frame() {
                    refused_finals[*target].push(file.relative_path.clone());
                } else if FINAL_IMAGE_EXTENSIONS.contains(&extension.as_str()) {
                    finals[*target].push(file);
                } else {
                    outside.push(file.relative_path.clone());
                }
            }
            Place::Lights(target) => {
                if !file.class.is_frame() {
                    outside.push(file.relative_path.clone());
                    continue;
                }
                let kind = effective_type(FrameType::Light);
                let folder_label = format!("{}/{LIGHTS_DIR}/", dirs_name(&dirs, input, *target));
                if kind != FrameType::Light {
                    count_calibration(&mut calibration, kind);
                    record_calibration(kind, header, &mut flat_filters, &mut dark_settings, None);
                    keep_calibration(
                        index,
                        kind,
                        header,
                        None,
                        &mut dark_frames,
                        &mut flat_frames,
                    );
                    let to = kind
                        .calibration_folder()
                        .map(|k| format!("{CALIBRATION_DIR}/{k}/"));
                    misfiled
                        .entry((folder_label, kind))
                        .or_insert_with(|| (to, true, Vec::new()))
                        .2
                        .push(file.relative_path.clone());
                    continue;
                }
                // A light: does its OBJECT name another target?
                let object_target = header
                    .and_then(|record| record.object.as_deref())
                    .and_then(|object| match_target(object, input));
                if let Some(other) = object_target
                    && other != *target
                    && !names_target(header, &input.targets[*target])
                {
                    other_target
                        .entry((folder_label.clone(), *target, other))
                        .or_default()
                        .push(file.relative_path.clone());
                    continue;
                }
                if let Some(date) = other_night_date(header, window.as_ref()) {
                    other_night
                        .entry((folder_label, date))
                        .or_insert_with(|| (Some(*target), Vec::new()))
                        .1
                        .push(file.relative_path.clone());
                    // Said, never dropped from the folder: the frame stays a
                    // light of the target; the stacking leaves it out of the
                    // night's integration.
                }
                lights[*target].push(index);
            }
            Place::Calibration(folder_kind) => {
                if !file.class.is_frame() {
                    outside.push(file.relative_path.clone());
                    continue;
                }
                let expected = FrameType::of_calibration_folder(folder_kind).expect("a known kind");
                // The folder is right. The frame is read as the folder's
                // type, whatever its header says; a header that says
                // otherwise is a warning, never a move.
                let header_type = effective_type(expected);
                let kind = expected;
                count_calibration(&mut calibration, kind);
                let subfolder = flat_subfolder(&file.relative_path);
                if kind == FrameType::Flat && subfolder.is_some() {
                    flats_by_folder = true;
                }
                keep_calibration(
                    index,
                    kind,
                    header,
                    subfolder.clone(),
                    &mut dark_frames,
                    &mut flat_frames,
                );
                record_calibration(
                    kind,
                    header,
                    &mut flat_filters,
                    &mut dark_settings,
                    subfolder,
                );
                if header_type != expected {
                    header_differs
                        .entry((
                            format!("{CALIBRATION_DIR}/{folder_kind}/"),
                            expected,
                            header_type,
                        ))
                        .or_default()
                        .push(file.relative_path.clone());
                }
            }
        }
    }

    // The targets, in the input's order.
    let mut targets = Vec::new();
    for (index, target) in input.targets.iter().enumerate() {
        let dir = dirs.iter().find(|d| d.index == Some(index));
        if !target.ticked && dir.is_none() {
            continue;
        }
        let records: Vec<&FrameHeaderRecord> = lights[index]
            .iter()
            .filter_map(|i| headers.get(i))
            .collect();
        let filters: BTreeSet<String> = records.iter().filter_map(|r| r.filter.clone()).collect();
        let mut candidates = finals[index].clone();
        candidates.sort_by(|a, b| {
            b.stamp
                .modified_ns
                .cmp(&a.stamp.modified_ns)
                .then_with(|| a.relative_path.cmp(&b.relative_path))
        });
        let final_image = candidates.first().map(|f| f.relative_path.clone());
        let final_bytes = candidates.first().map(|f| f.stamp.size);
        let light_summary = frame_summary(lights[index].iter().map(|i| {
            let record = headers.get(i);
            (
                record,
                record.and_then(|r| r.filter.clone()),
                placed[*i].0.stamp.size,
            )
        }));
        let other_final_images = candidates
            .iter()
            .skip(1)
            .map(|f| f.relative_path.clone())
            .collect();
        let device = device_detection(
            &device_images[index],
            &device_jpgs[index],
            &lights[index],
            &headers,
        );
        let has_device = device.as_ref().is_some_and(|d| !d.images.is_empty());
        let minimum = if final_image.is_some() {
            MinimumSource::FinalFile
        } else if target.online_final {
            MinimumSource::Online
        } else if target.stacking_planned {
            MinimumSource::Stacking
        } else {
            MinimumSource::Missing
        };
        let in_night = target.ticked;
        let folder_name = dir.map(|d| d.name.clone());
        let label = folder_name
            .clone()
            .unwrap_or_else(|| target_folder_name(&target.designation));

        if in_night {
            if !refused_finals[index].is_empty() {
                findings.push(
                    Finding::new(
                        keys::FINAL_FRAME_REFUSED,
                        Severity::Warning,
                        Some(&target.target_id),
                        json!({ "target": target.designation, "count": refused_finals[index].len(), "folder": format!("{label}/{FINAL_DIR}/") }),
                    )
                    .with_files(refused_finals[index].clone()),
                );
            }
            // The image a device stacked is a source of the stacking too.
            let has_frames = !lights[index].is_empty() || has_device;
            let has_final = matches!(minimum, MinimumSource::FinalFile | MinimumSource::Online);
            let finding_params = json!({ "target": target.designation, "folder": format!("{label}/{FINAL_DIR}/"), "lightsFolder": format!("{label}/{LIGHTS_DIR}/") });
            if !has_frames && !has_final {
                findings.push(Finding::new(
                    keys::TARGET_EMPTY,
                    Severity::Error,
                    Some(&target.target_id),
                    finding_params,
                ));
            } else if has_frames && minimum == MinimumSource::Missing {
                if has_device {
                    // The poses a device image does not cover are the only
                    // ones stacked.
                    let covered: usize = device
                        .iter()
                        .flat_map(|d| d.families.iter())
                        .filter(|family| family.default_source == device_master::Source::Device)
                        .map(|family| family.lights)
                        .sum();
                    let restack = lights[index].len().saturating_sub(covered);
                    findings.push(Finding::new(
                        keys::FINAL_FROM_DEVICE,
                        Severity::Warning,
                        Some(&target.target_id),
                        json!({
                            "target": target.designation,
                            "folder": format!("{label}/{FINAL_DIR}/"),
                            "lightsFolder": format!("{label}/{LIGHTS_DIR}/"),
                            "restackFrames": restack,
                        }),
                    ));
                } else {
                    findings.push(Finding::new(
                        keys::FINAL_MISSING,
                        Severity::Warning,
                        Some(&target.target_id),
                        finding_params,
                    ));
                }
            } else if !has_frames {
                findings.push(Finding::new(
                    keys::FRAMES_MISSING,
                    Severity::Warning,
                    Some(&target.target_id),
                    finding_params,
                ));
            }
            // Calibration for the target's lights: said, never blocking.
            let light_filters: BTreeSet<Option<String>> =
                records.iter().map(|r| r.filter.clone()).collect();
            for filter in &light_filters {
                let covered = match filter {
                    Some(filter) => flat_filters
                        .iter()
                        .flatten()
                        .any(|f| f.eq_ignore_ascii_case(filter)),
                    None => !flat_filters.is_empty(),
                };
                // A smart telescope's stack is the source: it calibrated on
                // board, so « no flat at all » is no omission there.
                let on_board = has_device && flat_filters.is_empty();
                if !covered && !on_board {
                    // No flat at all is said as an information (the person
                    // may skip flats on purpose); flats that exist and do not
                    // fit are a warning.
                    let none = flat_filters.is_empty();
                    findings.push(Finding::new(
                        keys::FLATS_MISSING,
                        if none {
                            Severity::Info
                        } else {
                            Severity::Warning
                        },
                        Some(&target.target_id),
                        json!({
                            "target": target.designation,
                            "filter": filter,
                            "none": none,
                            // The stacking takes no flat for this filter.
                            "stackedWithout": none || flats_by_folder,
                        }),
                    ));
                }
            }
            let mut settings: Vec<(Option<f64>, Option<f64>)> = Vec::new();
            for record in &records {
                let pair = (record.exposure_seconds, record.gain);
                if record.exposure_seconds.is_some()
                    && !settings.iter().any(|s| same_pair(*s, pair))
                {
                    settings.push(pair);
                }
            }
            settings.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            for pair in settings {
                let on_board = has_device && dark_settings.is_empty();
                if !on_board && !dark_settings.iter().any(|d| same_pair(*d, pair)) {
                    // Same two levels as the flats: darks that do not fit are
                    // a warning, none at all an information.
                    let none = dark_settings.is_empty();
                    let mut found: Vec<(Option<f64>, Option<f64>)> = Vec::new();
                    for dark in &dark_settings {
                        if !found.iter().any(|f| same_pair(*f, *dark)) {
                            found.push(*dark);
                        }
                    }
                    found.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    findings.push(Finding::new(
                        keys::DARKS_MISSING,
                        if none {
                            Severity::Info
                        } else {
                            Severity::Warning
                        },
                        Some(&target.target_id),
                        json!({
                            "target": target.designation,
                            "exposureSeconds": pair.0,
                            "gain": pair.1,
                            "none": none,
                            "count": dark_settings.len(),
                            "found": found
                                .iter()
                                .map(|(exposure, gain)| json!({ "exposureSeconds": exposure, "gain": gain }))
                                .collect::<Vec<_>>(),
                        }),
                    ));
                }
            }
        }
        targets.push(TargetDetection {
            target_id: target.target_id.clone(),
            designation: target.designation.clone(),
            folder_name,
            in_night,
            light_frames: lights[index].len(),
            filters: filters.into_iter().collect(),
            final_image,
            other_final_images,
            minimum,
            final_bytes,
            light_summary,
            device,
        });
    }
    // Folders whose target is not in the list at all: kept, out of the night.
    for dir in dirs.iter().filter(|d| d.index.is_none()) {
        targets.push(TargetDetection {
            target_id: String::new(),
            designation: dir.name.clone(),
            folder_name: Some(dir.name.clone()),
            in_night: false,
            light_frames: 0,
            filters: Vec::new(),
            final_image: None,
            other_final_images: Vec::new(),
            minimum: MinimumSource::Missing,
            final_bytes: None,
            light_summary: FrameSummary::default(),
            device: None,
        });
    }

    for ((from, kind), (to, from_lights, files)) in misfiled {
        findings.push(
            Finding::new(
                keys::FRAMES_MISFILED,
                Severity::Warning,
                None,
                json!({
                    "count": files.len(),
                    "kind": kind,
                    "header": kind.header_word(),
                    "from": from,
                    "to": to,
                    "fromLights": from_lights,
                }),
            )
            .with_files(files),
        );
    }
    for ((folder, kind, header_type), files) in header_differs {
        findings.push(
            Finding::new(
                keys::CALIBRATION_HEADER_DIFFERS,
                Severity::Warning,
                None,
                json!({
                    "count": files.len(),
                    "folder": folder,
                    "header": header_type.header_word(),
                    "kind": kind,
                }),
            )
            .with_files(files),
        );
    }
    for ((folder_label, target, other), files) in other_target {
        findings.push(
            Finding::new(
                keys::FRAMES_OTHER_TARGET,
                Severity::Warning,
                Some(&input.targets[target].target_id),
                json!({
                    "count": files.len(),
                    "object": input.targets[other].designation,
                    "folder": folder_label,
                }),
            )
            .with_files(files),
        );
    }
    for ((folder_label, date), (target, files)) in other_night {
        findings.push(
            Finding::new(
                keys::FRAMES_OTHER_NIGHT,
                Severity::Warning,
                target.map(|t| input.targets[t].target_id.as_str()),
                json!({ "count": files.len(), "date": date, "folder": folder_label }),
            )
            .with_files(files),
        );
    }
    if !logs.iter().any(|log| log.kind != LogKind::Unknown) {
        findings.push(Finding::new(keys::NO_LOGS, Severity::Info, None, json!({})));
    }
    if !outside.is_empty() {
        findings.push(
            Finding::new(
                keys::FILES_OUTSIDE_STRUCTURE,
                Severity::Warning,
                None,
                json!({ "count": outside.len() }),
            )
            .with_files(outside.clone()),
        );
    }
    for link in &walk.links {
        let key = match link.problem {
            LinkProblem::MissingTarget => keys::LINK_TARGET_MISSING,
            LinkProblem::Loop => keys::LINK_LOOP,
        };
        findings.push(
            Finding::new(
                key,
                Severity::Warning,
                None,
                json!({ "path": link.relative_path, "target": link.target }),
            )
            .with_files(vec![link.relative_path.clone()]),
        );
    }

    let calibration_summary = CalibrationSummary {
        darks: frame_summary(
            dark_frames
                .iter()
                .map(|i| (headers.get(i), None, placed[*i].0.stamp.size)),
        ),
        flats: frame_summary(
            flat_frames
                .iter()
                .map(|(i, filter)| (headers.get(i), filter.clone(), placed[*i].0.stamp.size)),
        ),
    };
    Ok(NightDetection {
        folder: folder.to_path_buf(),
        fingerprint,
        targets,
        logs,
        calibration,
        calibration_summary,
        findings,
        outside_structure: outside,
        links: walk.links,
    })
}

/// What a target's `device/` says: the families its images cover, against
/// the families of the poses in `lights/`; the `.jpg` that has no
/// recognised `.fit` of its name. `None` when the folder holds nothing of
/// the kind.
fn device_detection(
    images: &[DeviceImage],
    jpgs: &[String],
    lights: &[usize],
    headers: &BTreeMap<usize, FrameHeaderRecord>,
) -> Option<DeviceDetection> {
    if images.is_empty() && jpgs.is_empty() {
        return None;
    }
    let mut counts: Vec<LightFamily> = Vec::new();
    for record in lights.iter().filter_map(|i| headers.get(i)) {
        let label = device_master::label_of(
            record.filter.as_deref(),
            record.exposure_seconds,
            record.gain,
            record.binning.as_deref(),
        );
        match counts.iter_mut().find(|family| family.label == label) {
            Some(family) => family.frames += 1,
            None => counts.push(LightFamily { label, frames: 1 }),
        }
    }
    let stem = |path: &str| {
        let name = path.rsplit('/').next().unwrap_or(path);
        let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
        // The thumbnail the Seestar writes beside an image (`…_thn.jpg`).
        stem.strip_suffix("_thn").unwrap_or(stem).to_lowercase()
    };
    let known: BTreeSet<String> = images.iter().map(|image| stem(&image.file_name)).collect();
    let mut alone: Vec<String> = jpgs
        .iter()
        .filter(|jpg| !known.contains(&stem(jpg)))
        .filter(|jpg| !stem(jpg).is_empty())
        .cloned()
        .collect();
    alone.sort();
    let mut images = images.to_vec();
    images.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Some(DeviceDetection {
        families: device_master::families(&images, &counts),
        images,
        jpgs: alone,
    })
}

fn target_dirs(folder: &Path, input: &DetectionInput) -> Vec<TargetDir> {
    let mut dirs: Vec<TargetDir> = Vec::new();
    for (target_id, name) in target_folders_by_identity(folder) {
        let index = input.targets.iter().position(|t| t.target_id == target_id);
        dirs.push(TargetDir {
            in_night: index.is_some_and(|i| input.targets[i].ticked),
            index,
            name,
        });
    }
    // A ticked target without an identity folder: its folder by name.
    let Ok(entries) = std::fs::read_dir(folder) else {
        return dirs;
    };
    let names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    for (index, target) in input.targets.iter().enumerate() {
        if dirs.iter().any(|d| d.index == Some(index)) {
            continue;
        }
        let wanted = target_folder_name(&target.designation);
        if let Some(name) = names
            .iter()
            .find(|name| same_name(name, &wanted) && !dirs.iter().any(|d| same_name(&d.name, name)))
        {
            dirs.push(TargetDir {
                index: Some(index),
                name: name.clone(),
                in_night: target.ticked,
            });
        }
    }
    dirs
}

fn dirs_name(dirs: &[TargetDir], input: &DetectionInput, target: usize) -> String {
    dirs.iter().find(|d| d.index == Some(target)).map_or_else(
        || target_folder_name(&input.targets[target].designation),
        |d| d.name.clone(),
    )
}

fn place_of(relative: &str, dirs: &[TargetDir]) -> Place {
    let parts: Vec<&str> = relative.split('/').collect();
    if parts.len() < 2 {
        return Place::Outside;
    }
    let top = parts[0];
    if same_name(top, LOGS_DIR) {
        return Place::Logs;
    }
    if same_name(top, CALIBRATION_DIR) {
        if parts.len() >= 3
            && let Some(kind) = CALIBRATION_KINDS.iter().find(|k| same_name(k, parts[1]))
        {
            return Place::Calibration(kind);
        }
        if parts.len() >= 3 && same_name(crate::working_folder::BIAS_ALIAS, parts[1]) {
            return Place::Calibration(CALIBRATION_KINDS[3]);
        }
        return Place::Outside;
    }
    let Some(dir) = dirs
        .iter()
        .find(|d| d.name == top || same_name(&d.name, top))
    else {
        return Place::Outside;
    };
    if !dir.in_night {
        return Place::OutOfNight;
    }
    let index = dir.index.expect("a folder in the night has its target");
    if parts.len() < 3 {
        return Place::Outside;
    }
    match parts[1] {
        sub if same_name(sub, LIGHTS_DIR) => Place::Lights(index),
        sub if same_name(sub, FINAL_DIR) => Place::Final(index),
        sub if same_name(sub, DEVICE_DIR) => Place::Device(index),
        sub if same_name(sub, STACKED_DIR) || same_name(sub, PROGRESSION_DIR) => {
            Place::AstroSyncOutput
        }
        _ => Place::Outside,
    }
}

fn extension_of(relative: &str) -> String {
    Path::new(relative)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// The target `object` names: its designation, its folder name or one of
/// its aliases, case and spaces ignored.
fn match_target(object: &str, input: &DetectionInput) -> Option<usize> {
    let wanted = normalize_object(object);
    input
        .targets
        .iter()
        .position(|target| names_of(target).any(|name| name == wanted))
}

fn names_of(target: &DetectionTarget) -> impl Iterator<Item = String> + '_ {
    std::iter::once(normalize_object(&target.designation))
        .chain(target.aliases.iter().map(|a| normalize_object(a)))
}

fn names_target(header: Option<&FrameHeaderRecord>, target: &DetectionTarget) -> bool {
    header
        .and_then(|record| record.object.as_deref())
        .is_some_and(|object| {
            let wanted = normalize_object(object);
            names_of(target).any(|name| name == wanted)
        })
}

/// The type the calibration folder of a place holds, `None` elsewhere.
fn folder_type_of(place: &Place) -> Option<FrameType> {
    match place {
        Place::Calibration(folder) => FrameType::of_calibration_folder(folder),
        _ => None,
    }
}

/// The longest exposure of the flats the night's headers declare.
fn longest_flat_exposure(headers: &BTreeMap<usize, FrameHeaderRecord>) -> Option<f64> {
    headers
        .values()
        .filter(|record| record.image_type == Some(FrameType::Flat))
        .filter_map(|record| record.exposure_seconds)
        .reduce(f64::max)
}

/// A dark-flat is written `IMAGETYP = Dark` by some capture programs (the
/// ASIAIR does), so a `Dark` header is ambiguous. Rule: **the folder says**
/// when it is a calibration folder of darks or dark-flats; elsewhere (a
/// `lights/` folder, `flats/`, `biases/`) a `Dark` as short as the night's
/// flats is a dark-flat, a longer one a dark. Any other header type stands.
fn resolve_dark_flat(
    kind: FrameType,
    header: Option<&FrameHeaderRecord>,
    folder: Option<FrameType>,
    longest_flat: Option<f64>,
) -> FrameType {
    if kind != FrameType::Dark {
        return kind;
    }
    match folder {
        Some(FrameType::DarkFlat) => FrameType::DarkFlat,
        Some(FrameType::Dark) => FrameType::Dark,
        _ => match (header.and_then(|r| r.exposure_seconds), longest_flat) {
            (Some(exposure), Some(flat)) if exposure <= flat + 1e-3 => FrameType::DarkFlat,
            _ => FrameType::Dark,
        },
    }
}

fn count_calibration(counts: &mut CalibrationCounts, kind: FrameType) {
    match kind {
        FrameType::Dark => counts.darks += 1,
        FrameType::Flat => counts.flats += 1,
        FrameType::DarkFlat => counts.dark_flats += 1,
        FrameType::Bias => counts.biases += 1,
        FrameType::Light => {}
    }
}

/// A flat's filter subfolder (`calibration/flats/Ha/…` → `Ha`), used when
/// its header names no filter.
fn flat_subfolder(relative: &str) -> Option<String> {
    let parts: Vec<&str> = relative.split('/').collect();
    (parts.len() >= 4 && same_name(parts[1], "flats")).then(|| parts[2].to_string())
}

fn keep_calibration(
    index: usize,
    kind: FrameType,
    header: Option<&FrameHeaderRecord>,
    subfolder: Option<String>,
    darks: &mut Vec<usize>,
    flats: &mut Vec<(usize, Option<String>)>,
) {
    match kind {
        FrameType::Dark => darks.push(index),
        FrameType::Flat => flats.push((index, header.and_then(|r| r.filter.clone()).or(subfolder))),
        _ => {}
    }
}

fn record_calibration(
    kind: FrameType,
    header: Option<&FrameHeaderRecord>,
    flat_filters: &mut BTreeSet<Option<String>>,
    dark_settings: &mut Vec<(Option<f64>, Option<f64>)>,
    subfolder: Option<String>,
) {
    match kind {
        FrameType::Flat => {
            let filter = header.and_then(|r| r.filter.clone()).or(subfolder);
            flat_filters.insert(filter);
        }
        FrameType::Dark => {
            if let Some(record) = header {
                dark_settings.push((record.exposure_seconds, record.gain));
            }
        }
        _ => {}
    }
}

fn same_pair(a: (Option<f64>, Option<f64>), b: (Option<f64>, Option<f64>)) -> bool {
    let close = |x: Option<f64>, y: Option<f64>| match (x, y) {
        (Some(x), Some(y)) => (x - y).abs() < 1e-3,
        (None, None) => true,
        _ => false,
    };
    close(a.0, b.0) && close(a.1, b.1)
}

/// The night's window: from local noon of the evening to local noon of the next day,
/// local noon drawn from the place's longitude alone (mean solar noon,
/// `12:00 UTC - longitude / 15 h`; never a time zone, never the start hour
/// typed). It is read on the instants (`DATE-OBS` in UTC), never on
/// civil days, so a night that crosses midnight (UTC or local) stays one
/// night. A light outside it is said ([`keys::FRAMES_OTHER_NIGHT`]), stays
/// counted in the folder, and is left out of the night's integration
/// ([`crate::stacking::plan_target`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NightWindow {
    /// Local noon of the evening, Unix seconds.
    pub start: f64,
    /// The place's longitude, degrees east (`0` when unknown: UTC).
    pub longitude: f64,
}

impl NightWindow {
    /// The window of the night of `evening_date` (`YYYY-MM-DD`) at
    /// `longitude` (degrees east; `None`: UTC). `None` for an unreadable date.
    #[must_use]
    pub fn of_evening(evening_date: &str, longitude: Option<f64>) -> Option<Self> {
        let midnight = parse_iso8601_utc_to_unix_seconds(&format!("{evening_date}T00:00:00Z"))?;
        let longitude = longitude.filter(|l| l.is_finite()).unwrap_or(0.0);
        Some(Self {
            start: midnight + 12.0 * 3600.0 - longitude / 15.0 * 3600.0,
            longitude,
        })
    }

    /// The window of a detection: its evening date, else the evening of its
    /// start ([`crate::working_folder::evening_date_of`]), at its longitude.
    #[must_use]
    pub fn of_input(input: &DetectionInput) -> Option<Self> {
        let evening = match input.evening_date.as_deref() {
            Some(date) => date.to_string(),
            None => crate::working_folder::evening_date_of(
                input.night_started_at.as_deref()?,
                input.longitude,
            )?,
        };
        Self::of_evening(&evening, input.longitude)
    }

    /// Whether the instant `unix` (seconds) is inside the window.
    #[must_use]
    pub fn contains(&self, unix: f64) -> bool {
        (self.start..self.start + NIGHT_WINDOW_SECONDS).contains(&unix)
    }

    /// Whether a frame taken at `date_obs` (ISO 8601, UTC) is inside the
    /// window; a frame without a readable date is (nothing says otherwise).
    #[must_use]
    pub fn holds(&self, date_obs: Option<&str>) -> bool {
        date_obs
            .and_then(parse_iso8601_utc_to_unix_seconds)
            .is_none_or(|taken| self.contains(taken))
    }

    /// Whether a frame taken at `date_obs` is inside the window, the zone of
    /// its clock being known or not: a camera clock of unknown zone (a RAW
    /// file without `OffsetTimeOriginal` nor GPS time) is the wall clock of
    /// the place, so it is compared with the window drawn on the wall clock
    /// (local noon at 12:00) rather than on UTC. A frame without a readable
    /// date is inside.
    #[must_use]
    pub fn holds_clock(&self, date_obs: Option<&str>, zone_unknown: bool) -> bool {
        if !zone_unknown {
            return self.holds(date_obs);
        }
        let start = self.start + self.longitude / 15.0 * 3600.0;
        date_obs
            .and_then(parse_iso8601_utc_to_unix_seconds)
            .is_none_or(|taken| (start..start + NIGHT_WINDOW_SECONDS).contains(&taken))
    }

    /// The evening date (`YYYY-MM-DD`) of the night an instant belongs to,
    /// by the same rule.
    #[must_use]
    pub fn evening_of(&self, unix: f64) -> String {
        let solar = unix + self.longitude / 15.0 * 3600.0 - 12.0 * 3600.0;
        let days = (solar / 86_400.0).floor() as i64;
        let (year, month, day) = crate::time_util::civil_from_days(days);
        format!("{year:04}-{month:02}-{day:02}")
    }

    /// [`NightWindow::evening_of`] of an instant that is a wall clock (zone
    /// unknown): the evening is the date of the clock twelve hours before.
    #[must_use]
    pub fn evening_of_clock(unix: f64) -> String {
        let days = ((unix - 12.0 * 3600.0) / 86_400.0).floor() as i64;
        let (year, month, day) = crate::time_util::civil_from_days(days);
        format!("{year:04}-{month:02}-{day:02}")
    }
}

fn night_window(input: &DetectionInput) -> Option<NightWindow> {
    NightWindow::of_input(input)
}

/// The evening date of a light taken outside the night's window, if it is.
fn other_night_date(
    header: Option<&FrameHeaderRecord>,
    window: Option<&NightWindow>,
) -> Option<String> {
    let window = window?;
    let header = header?;
    let taken = parse_iso8601_utc_to_unix_seconds(header.date_obs_utc.as_deref()?)?;
    if window.holds_clock(header.date_obs_utc.as_deref(), header.date_zone_unknown) {
        return None;
    }
    Some(if header.date_zone_unknown {
        NightWindow::evening_of_clock(taken)
    } else {
        window.evening_of(taken)
    })
}

/// A file of `logs/`: NINA, PHD2 or ASIAIR, by name then by contents.
pub(crate) fn log_kind(file: &WalkedFile) -> LogKind {
    let name = file
        .relative_path
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if name.starts_with("phd2_guidelog") {
        return LogKind::Phd2;
    }
    if name.starts_with("autorun_log") {
        // `_CHN`: the Chinese rendition of a sibling log, read by no one.
        return if crate::analysis::is_asiair_translation(&name, "") {
            LogKind::Unknown
        } else {
            LogKind::Asiair
        };
    }
    let Ok(mut handle) = std::fs::File::open(&file.path) else {
        return LogKind::Unknown;
    };
    let mut head = vec![0_u8; 16 * 1024];
    let read = std::io::Read::read(&mut handle, &mut head).unwrap_or(0);
    let text = String::from_utf8_lossy(&head[..read]);
    if crate::analysis::is_asiair_translation(&name, &text) {
        LogKind::Unknown
    } else if crate::analysis::looks_like_phd2_guide_log(&text) {
        LogKind::Phd2
    } else if looks_like_nina_log(&text) {
        LogKind::Nina
    } else if crate::analysis::looks_like_asiair_autorun_log(&text) {
        LogKind::Asiair
    } else {
        LogKind::Unknown
    }
}

/// A NINA log: lines `2026-10-07T21:03:12.1234|INFO|File.cs|Member|12|…`.
#[must_use]
pub fn looks_like_nina_log(contents: &str) -> bool {
    contents.lines().take(40).any(|line| {
        let fields: Vec<&str> = line.splitn(4, '|').collect();
        fields.len() == 4
            && fields[0].len() >= 19
            && fields[0].as_bytes().get(4) == Some(&b'-')
            && fields[0].as_bytes().get(10) == Some(&b'T')
            && matches!(
                fields[1].trim(),
                "INFO" | "DEBUG" | "WARNING" | "ERROR" | "TRACE"
            )
    })
}

#[cfg(test)]
mod tests;

//! "Frame measurement": each light frame is
//! measured on its own pixels -- the width of its stars, their eccentricity,
//! how many stars it holds, how high its sky is -- and the poses that depart
//! from the rest of the night are named, with their reason: **dawn** or a
//! brighter sky, **cloud** (few stars), **trailing** (elongated stars), wider
//! stars. Nothing is sent, nothing is moved: the poses stay in `lights/`; the
//! only use is the stacking of "Processing", which can leave the poses
//! flagged out of the integration ([`apply_discard`]), never a deletion.
//!
//! - **No composite score.** A pose is flagged with **one reason** (the first
//!   that applies, in the order of [`Reason`]) or it is not; the figures are
//!   shown as they are measured (rule of the analysis module, kept).
//! - **A dawn is told by its shape.** Going back from the last pose of a
//!   family, the poses whose sky rises without falling (to
//!   [`DAWN_RISE_TOLERANCE`]) and is at least [`DAWN_ONSET_FACTOR`] times the
//!   median are one rise; when it ends above [`SKY_FACTOR`] times the median
//!   it is the dawn (or, with no twilight at that time, a bright sky), and
//!   all its poses carry that reason: the first ones, whose stars are already
//!   fewer, are not « clouds ».
//! - **A family is judged by itself.** The poses of two exposures (10 s and
//!   30 s) are never compared: each family has its own medians.
//! - **Nothing is guessed.** Under twenty stars a pose gives no width and no
//!   eccentricity (`star_quality.py`'s rule) but its **number of stars** and
//!   its sky stay; under [`MIN_POSES_FOR_MEDIAN`] measured poses in a family
//!   there is no median, so no pose is flagged.
//! - **A family width, said as such.** A family whose poses are too sparse to
//!   have a width of their own (a narrow band on a mono sensor: thirty stars
//!   a pose) pools the stars its poses did read, and gives their median as
//!   **the family's** width ([`FamilyShape`]), never as a pose's: no pose is
//!   judged wide or trailed by it. Under [`MIN_POOLED_STARS`] stars read in
//!   all, nothing is given.
//! - **A sample**, said on the screen: every pose of a family of
//!   [`SAMPLE_ALL_UP_TO`] poses or fewer; above, one in [`SAMPLE_STEP`], then
//!   **the neighbours of every pose that departs** (so the edge of a passing
//!   cloud or of the dawn is exact).
//! - **The measure is a script** (`stacking/pose_stats.py`, on the stars of
//!   `star_quality.py`), one run per batch of frames, one JSON line per frame
//!   as it goes ([`PythonMeasurer`]); a stop switch ends it between frames and
//!   what is measured is kept. The thresholds below are named constants,
//!   to be tuned on the real nights.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::analysis::ephemeris::sun_altitude_deg;
use crate::detection::NightWindow;
use crate::external_process;
use crate::local_work::CancelFlag;
use crate::python_env;
use crate::stacking::{TargetPlan, family_label, stack_families_within};
use crate::time_util::parse_iso8601_utc_to_unix_seconds;
use crate::working_folder::WORK_DIR;

pub mod record;

/// Every pose of a family up to this many is measured.
pub const SAMPLE_ALL_UP_TO: usize = 200;
/// Above [`SAMPLE_ALL_UP_TO`]: one pose in this many.
pub const SAMPLE_STEP: usize = 4;
/// A sky more than this many times the family's median is flagged.
pub const SKY_FACTOR: f64 = 3.0;
/// A pose with less than this share of the median star count is a probable
/// cloud.
pub const STARS_SHARE: f64 = 0.5;
/// Eccentricity at least this far above the local median: trailed stars; the
/// least margin, whatever the noise of the night (see
/// [`ECCENTRICITY_SIGMAS`]).
pub const ECCENTRICITY_MARGIN: f64 = 0.15;
/// The margin of trailed stars is also this many times the noise of the
/// night's eccentricity (robust spread of the poses around their local
/// median): a Seestar night, whose small stars make each pose's eccentricity
/// jump by 0.07, is not told « trailed » for a pose in its normal scatter,
/// while a cooled camera whose scatter is 0.03 keeps the margin above.
pub const ECCENTRICITY_SIGMAS: f64 = 4.0;
/// The « local median » of the eccentricity: the median of this many
/// neighbouring poses (by time) of the family. The field rotation of an
/// alt-azimuth mount makes the eccentricity of a Seestar night go from 0.5 to
/// 0.9 and back, so the median of the whole night is no reference.
pub const ECCENTRICITY_WINDOW: usize = 21;
/// A dawn is a rise of the sky that goes on to the end of the night: its
/// first pose is the first one, going back from the last, whose sky is under
/// this many times the median, or lower than the next pose (to
/// [`DAWN_RISE_TOLERANCE`]).
pub const DAWN_ONSET_FACTOR: f64 = 1.5;
/// A pose of the dawn may be lower than the next by this share (noise of the
/// sky measure) and still belong to the same rise.
pub const DAWN_RISE_TOLERANCE: f64 = 0.05;
/// A width this many times the median: wider stars.
pub const FWHM_FACTOR: f64 = 1.4;
/// Under this many measured poses a family has no median.
pub const MIN_POSES_FOR_MEDIAN: usize = 10;
/// Under this many stars read in all, a family has no pooled width either:
/// nothing is guessed.
pub const MIN_POOLED_STARS: usize = 8;
/// The Sun below the horizon by less than this many degrees: twilight, said
/// "dawn".
pub const TWILIGHT_DEGREES: f64 = 18.0;
/// The scratch of the lists of frames, under `<night>/.astro-sync-work/`.
const SCRATCH_DIR: &str = "pose-stats";
/// The longest a batch may run.
const TIMEOUT: Duration = Duration::from_secs(3 * 3600);

/// One family of a target's lights, as the measure groups them (the stacking's
/// families, with the exposure).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PoseFamily {
    /// `IRCUT_30s_g200`.
    pub label: String,
    pub exposure_seconds: Option<f64>,
    /// The frames, sorted by path.
    pub frames: Vec<PathBuf>,
}

/// The families of the target's lights inside the night's window, the largest
/// first (the stacking's order).
#[must_use]
pub fn pose_families(
    night_folder: &Path,
    target_folder: &str,
    window: Option<&NightWindow>,
) -> Vec<PoseFamily> {
    let (found, _) =
        crate::progression_images::target_families_within(night_folder, target_folder, window);
    let mut families: Vec<PoseFamily> = Vec::new();
    for family in found {
        let label = family_label(&family);
        let frames = family
            .frame_paths
            .iter()
            .map(PathBuf::from)
            .filter(|p| is_fits(p));
        match families.iter_mut().find(|f| f.label == label) {
            Some(known) => known.frames.extend(frames),
            None => families.push(PoseFamily {
                label,
                exposure_seconds: family.exposure_seconds,
                frames: frames.collect(),
            }),
        }
    }
    families.retain(|f| !f.frames.is_empty());
    for family in &mut families {
        family.frames.sort();
    }
    families.sort_by(|a, b| {
        b.frames
            .len()
            .cmp(&a.frames.len())
            .then(a.label.cmp(&b.label))
    });
    families
}

/// A pose the script measures: a FITS frame, or a camera RAW file
/// (converted by Siril before it is measured).
fn is_fits(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(e.to_ascii_lowercase().as_str(), "fit" | "fits" | "fts")
            || crate::raw_header::is_raw_extension(e)
    })
}

/// The indices of a family of `count` poses measured in the first pass: all
/// of them up to [`SAMPLE_ALL_UP_TO`], else one in [`SAMPLE_STEP`] and the
/// last.
#[must_use]
pub fn sample_indices(count: usize) -> Vec<usize> {
    if count <= SAMPLE_ALL_UP_TO {
        return (0..count).collect();
    }
    let mut indices: Vec<usize> = (0..count).step_by(SAMPLE_STEP).collect();
    if indices.last() != Some(&(count - 1)) {
        indices.push(count - 1);
    }
    indices
}

/// What the measure of one pose says.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PoseMeasure {
    pub path: PathBuf,
    /// The rank of the pose in its family (sorted by path).
    pub index: usize,
    /// `DATE-OBS`, as the header says it.
    pub date_obs: Option<String>,
    /// `DATE-OBS` in Unix seconds (UTC).
    pub unix: Option<f64>,
    pub fwhm_pixels: Option<f64>,
    pub eccentricity: Option<f64>,
    pub star_count: Option<u32>,
    /// The sky background, in ADU.
    pub sky_background: Option<f64>,
    /// A pose with too few stars for a width of its own hands over the width
    /// of each star it read (frame pixels), for the figure of its family.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub star_widths: Vec<f64>,
    /// The eccentricity of each of those stars (same order).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub star_eccentricities: Vec<f64>,
    /// The file could not be read.
    #[serde(default)]
    pub unreadable: bool,
}

/// Why a pose departs from the rest of its family; the first that applies.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reason {
    /// Sky `factor` times the median, the Sun below the horizon by less than
    /// [`TWILIGHT_DEGREES`].
    Dawn { factor: f64 },
    /// Sky `factor` times the median, no twilight (or no place known).
    Sky { factor: f64 },
    /// Less than [`STARS_SHARE`] of the median star count.
    Clouds,
    /// Eccentricity [`ECCENTRICITY_MARGIN`] above the median.
    Trailed,
    /// Width [`FWHM_FACTOR`] times the median.
    Wide,
}

/// A pose flagged, by its path (the key of the choice "Leave out").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlaggedPose {
    pub path: PathBuf,
    pub index: usize,
    pub reason: Reason,
}

/// The medians of a family, over the poses measured.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Medians {
    pub fwhm_pixels: Option<f64>,
    pub eccentricity: Option<f64>,
    pub star_count: Option<f64>,
    pub sky_background: Option<f64>,
}

/// The width of the stars of a whole family, when its poses have too few stars
/// each for a width of their own: the median over the stars they did read,
/// pooled. It is the family's, not a pose's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FamilyShape {
    pub fwhm_pixels: f64,
    pub eccentricity: Option<f64>,
    /// The stars pooled.
    pub stars: usize,
    /// The poses they come from.
    pub poses: usize,
}

/// One family: its poses measured so far (by time), its medians and the
/// poses that depart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FamilyStats {
    pub label: String,
    pub exposure_seconds: Option<f64>,
    /// The poses of the family in the night.
    pub total: usize,
    /// Measured so far, by time.
    pub measured: Vec<PoseMeasure>,
    /// Whether the family has a median (at least [`MIN_POSES_FOR_MEDIAN`]
    /// poses measured): otherwise nothing is flagged.
    pub judged: bool,
    pub medians: Medians,
    /// How many measured poses have no width (under twenty stars), when the
    /// width has no median.
    pub shape_missing: Option<usize>,
    /// The width of the family's stars pooled over its poses, when the poses
    /// have none of their own (see [`FamilyShape`]).
    #[serde(default)]
    pub family_shape: Option<FamilyShape>,
    pub flagged: Vec<FlaggedPose>,
}

/// What a measure of a target comes to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PoseStats {
    pub families: Vec<FamilyStats>,
    /// The poses of the target in the night (all families).
    pub total: usize,
    /// The poses planned so far (the sample, then the neighbours).
    pub planned: usize,
    /// The measure was stopped before its end.
    pub stopped: bool,
}

impl PoseStats {
    /// The poses measured, all families.
    #[must_use]
    pub fn measured_count(&self) -> usize {
        self.families.iter().map(|f| f.measured.len()).sum()
    }

    /// The poses flagged, all families.
    #[must_use]
    pub fn flagged_count(&self) -> usize {
        self.families.iter().map(|f| f.flagged.len()).sum()
    }
}

fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    Some(if values.len() % 2 == 1 {
        values[middle]
    } else {
        (values[middle - 1] + values[middle]) / 2.0
    })
}

/// Where the night is: for the Sun's height at the time of a pose.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub latitude: f64,
    pub longitude: f64,
}

/// Judges a family: medians, then the first reason of each pose that departs.
#[must_use]
pub fn judge(
    label: &str,
    exposure_seconds: Option<f64>,
    total: usize,
    measured: &[PoseMeasure],
    place: Option<Place>,
) -> FamilyStats {
    let mut poses: Vec<PoseMeasure> = measured.iter().filter(|p| !p.unreadable).cloned().collect();
    poses.sort_by(|a, b| {
        a.unix
            .unwrap_or(f64::MAX)
            .total_cmp(&b.unix.unwrap_or(f64::MAX))
            .then(a.index.cmp(&b.index))
    });
    let column = |pick: fn(&PoseMeasure) -> Option<f64>| -> Vec<f64> {
        poses.iter().filter_map(pick).collect()
    };
    let judged = poses.len() >= MIN_POSES_FOR_MEDIAN;
    let mut skies = column(|p| p.sky_background);
    let mut stars = column(|p| p.star_count.map(f64::from));
    let mut widths = column(|p| p.fwhm_pixels);
    let mut eccentricities = column(|p| p.eccentricity);
    let have_shape = widths.len() >= MIN_POSES_FOR_MEDIAN;
    let medians = Medians {
        fwhm_pixels: if have_shape {
            median(&mut widths)
        } else {
            None
        },
        eccentricity: if have_shape {
            median(&mut eccentricities)
        } else {
            None
        },
        star_count: if judged { median(&mut stars) } else { None },
        sky_background: if judged { median(&mut skies) } else { None },
    };
    let mut flagged = Vec::new();
    if judged {
        let rise_from = dawn_start(&poses, &medians);
        let trails = trailed_limits(&poses);
        for (position, pose) in poses.iter().enumerate() {
            let rising = position >= rise_from;
            let trail = trails.get(position).copied().flatten();
            if let Some(reason) = reason_of(pose, &medians, place, rising, trail) {
                flagged.push(FlaggedPose {
                    path: pose.path.clone(),
                    index: pose.index,
                    reason,
                });
            }
        }
    }
    FamilyStats {
        label: label.to_string(),
        exposure_seconds,
        total,
        shape_missing: (judged && !have_shape).then(|| poses.len() - widths_len(&poses)),
        family_shape: if have_shape {
            None
        } else {
            pooled_shape(&poses)
        },
        measured: poses,
        judged,
        medians,
        flagged,
    }
}

/// The width of a family from the stars its poses handed over: their median,
/// when [`MIN_POOLED_STARS`] stars or more were read in all.
fn pooled_shape(poses: &[PoseMeasure]) -> Option<FamilyShape> {
    let mut widths: Vec<f64> = Vec::new();
    let mut eccentricities: Vec<f64> = Vec::new();
    let mut contributing = 0;
    for pose in poses {
        let before = widths.len();
        widths.extend(
            pose.star_widths
                .iter()
                .copied()
                .filter(|w| w.is_finite() && *w > 0.0),
        );
        eccentricities.extend(
            pose.star_eccentricities
                .iter()
                .copied()
                .filter(|e| e.is_finite()),
        );
        if widths.len() > before {
            contributing += 1;
        }
    }
    let stars = widths.len();
    if stars < MIN_POOLED_STARS {
        return None;
    }
    Some(FamilyShape {
        fwhm_pixels: median(&mut widths)?,
        eccentricity: if eccentricities.len() >= MIN_POOLED_STARS {
            median(&mut eccentricities)
        } else {
            None
        },
        stars,
        poses: contributing,
    })
}

fn widths_len(poses: &[PoseMeasure]) -> usize {
    poses.iter().filter(|p| p.fwhm_pixels.is_some()).count()
}

/// The position, in `poses` (by time), of the first pose of the dawn; the
/// length of `poses` when there is none (see the module documentation).
fn dawn_start(poses: &[PoseMeasure], medians: &Medians) -> usize {
    let Some(base) = medians.sky_background.filter(|b| *b > 0.0) else {
        return poses.len();
    };
    let mut start = poses.len();
    let mut next: Option<f64> = None;
    for (position, pose) in poses.iter().enumerate().rev() {
        let Some(sky) = pose.sky_background else {
            break;
        };
        if sky < DAWN_ONSET_FACTOR * base
            || next.is_some_and(|n| sky > n * (1.0 + DAWN_RISE_TOLERANCE))
        {
            break;
        }
        start = position;
        next = Some(sky);
    }
    let peak = poses[start.min(poses.len())..]
        .iter()
        .filter_map(|p| p.sky_background)
        .fold(0.0_f64, f64::max);
    if start < poses.len() && peak > SKY_FACTOR * base {
        start
    } else {
        poses.len()
    }
}

/// For each pose (by position), how far above its local median its
/// eccentricity is and the limit that makes it « trailed »: `None` for a pose
/// with no eccentricity, or when the family has no eccentricity median.
/// The limit is [`ECCENTRICITY_MARGIN`] or [`ECCENTRICITY_SIGMAS`] times the
/// night's own noise, the larger.
fn trailed_limits(poses: &[PoseMeasure]) -> Vec<Option<(f64, f64)>> {
    let shaped: Vec<(usize, f64)> = poses
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.eccentricity.map(|e| (i, e)))
        .collect();
    let mut limits = vec![None; poses.len()];
    if shaped.len() < MIN_POSES_FOR_MEDIAN {
        return limits;
    }
    let window = ECCENTRICITY_WINDOW.min(shaped.len());
    let excess: Vec<f64> = (0..shaped.len())
        .map(|j| {
            let low = j.saturating_sub(window / 2).min(shaped.len() - window);
            let mut near: Vec<f64> = shaped[low..low + window].iter().map(|(_, e)| *e).collect();
            shaped[j].1 - median(&mut near).unwrap_or(0.0)
        })
        .collect();
    let centre = median(&mut excess.clone()).unwrap_or(0.0);
    let mut spread: Vec<f64> = excess.iter().map(|x| (x - centre).abs()).collect();
    let noise = 1.4826 * median(&mut spread).unwrap_or(0.0);
    let limit = ECCENTRICITY_MARGIN.max(ECCENTRICITY_SIGMAS * noise);
    for (j, (i, _)) in shaped.iter().enumerate() {
        limits[*i] = Some((excess[j], limit));
    }
    limits
}

fn sky_reason(pose: &PoseMeasure, base: f64, place: Option<Place>) -> Reason {
    let sky = pose.sky_background.unwrap_or(base);
    let factor = (sky / base * 10.0).round() / 10.0;
    let twilight = match (pose.unix, place) {
        (Some(unix), Some(place)) => {
            let altitude = sun_altitude_deg(unix, place.latitude, place.longitude);
            altitude < 0.0 && altitude > -TWILIGHT_DEGREES
        }
        _ => false,
    };
    if twilight {
        Reason::Dawn { factor }
    } else {
        Reason::Sky { factor }
    }
}

fn reason_of(
    pose: &PoseMeasure,
    medians: &Medians,
    place: Option<Place>,
    rising: bool,
    trail: Option<(f64, f64)>,
) -> Option<Reason> {
    if let (Some(sky), Some(base)) = (pose.sky_background, medians.sky_background)
        && base > 0.0
        && (rising || sky > SKY_FACTOR * base)
    {
        return Some(sky_reason(pose, base, place));
    }
    if let (Some(count), Some(base)) = (pose.star_count, medians.star_count)
        && base > 0.0
        && f64::from(count) < STARS_SHARE * base
    {
        return Some(Reason::Clouds);
    }
    if let Some((excess, limit)) = trail
        && excess > limit
    {
        return Some(Reason::Trailed);
    }
    if let (Some(width), Some(base)) = (pose.fwhm_pixels, medians.fwhm_pixels)
        && base > 0.0
        && width > FWHM_FACTOR * base
    {
        return Some(Reason::Wide);
    }
    None
}

/* ----------------------------------- Measure ----------------------------- */

/// What measures frames: the script ([`PythonMeasurer`]), or a stand-in in the
/// tests.
pub trait Measurer {
    /// Measures `frames` in order, handing each answer to `on_pose` as it
    /// comes; stops between two frames when `cancel` says so. An error only
    /// when the measure cannot run at all.
    fn measure(
        &self,
        frames: &[PathBuf],
        cancel: Option<&CancelFlag>,
        on_pose: &mut dyn FnMut(RawPose),
    ) -> Result<(), String>;
}

/// One answer of the script, before its place in a family is known.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawPose {
    pub path: PathBuf,
    pub status: String,
    #[serde(default)]
    pub date_obs: Option<String>,
    #[serde(default)]
    pub fwhm_pixels: Option<f64>,
    #[serde(default)]
    pub eccentricity: Option<f64>,
    #[serde(default)]
    pub star_count: Option<u32>,
    #[serde(default)]
    pub sky_background: Option<f64>,
    #[serde(default)]
    pub star_widths: Vec<f64>,
    #[serde(default)]
    pub star_eccentricities: Vec<f64>,
}

/// `date_obs` in Unix seconds: the header's text is UTC without a zone.
fn unix_of(date_obs: &str) -> Option<f64> {
    let text = date_obs.trim();
    if text.ends_with('Z') {
        parse_iso8601_utc_to_unix_seconds(text)
    } else {
        parse_iso8601_utc_to_unix_seconds(&format!("{text}Z"))
    }
}

impl RawPose {
    fn into_measure(self, index: usize) -> PoseMeasure {
        let unreadable = self.status != "measured";
        PoseMeasure {
            unix: self.date_obs.as_deref().and_then(unix_of),
            path: self.path,
            index,
            date_obs: self.date_obs,
            fwhm_pixels: self.fwhm_pixels,
            eccentricity: self.eccentricity,
            star_count: self.star_count,
            sky_background: self.sky_background,
            star_widths: self.star_widths,
            star_eccentricities: self.star_eccentricities,
            unreadable,
        }
    }
}

/// The date of a camera RAW pose: the script measures the pixels of Siril's
/// copy, whose date is the camera clock turned into UTC by the time zone of
/// this computer, so it gives none; the date is the RAW file's own
/// (`crate::raw_header`). A camera clock whose zone the file does not say is
/// the wall clock of the place: with the place of the night it is brought to
/// UTC by its mean solar time, without one it is left as written.
fn raw_clock(pose: &mut PoseMeasure, place: Option<Place>) {
    let is_raw = pose
        .path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(crate::raw_header::is_raw_extension);
    if !is_raw {
        return;
    }
    let Some(header) = crate::raw_header::read_header_file(&pose.path).ok() else {
        return;
    };
    let record = crate::analysis::fits::extract_acquisition_record(&header);
    pose.date_obs = record.date_obs_utc.clone();
    pose.unix = record.date_obs_utc.as_deref().and_then(unix_of);
    if record.date_zone_unknown
        && let (Some(unix), Some(place)) = (pose.unix, place)
    {
        pose.unix = Some(unix - place.longitude / 15.0 * 3600.0);
    }
}

/// The script `pose_stats.py` in the stacking environment, one run per batch.
#[derive(Debug, Clone)]
pub struct PythonMeasurer {
    pub python: PathBuf,
    pub script: PathBuf,
    /// Where the list of the batch's frames is written (a folder of the
    /// night's scratch).
    pub scratch: PathBuf,
    /// Siril, which converts the camera RAW poses before they are
    /// measured; `None`: the script looks for it on the PATH.
    pub siril: Option<PathBuf>,
}

impl PythonMeasurer {
    /// A measurer whose scratch is `<night>/.astro-sync-work/pose-stats/`.
    #[must_use]
    pub fn for_night(night_folder: &Path, python: PathBuf, script: PathBuf) -> Self {
        Self {
            python,
            script,
            scratch: night_folder.join(WORK_DIR).join(SCRATCH_DIR),
            siril: None,
        }
    }

    /// The same measurer with the Siril that converts the RAW poses.
    #[must_use]
    pub fn with_siril(mut self, siril: Option<PathBuf>) -> Self {
        self.siril = siril;
        self
    }
}

impl Measurer for PythonMeasurer {
    fn measure(
        &self,
        frames: &[PathBuf],
        cancel: Option<&CancelFlag>,
        on_pose: &mut dyn FnMut(RawPose),
    ) -> Result<(), String> {
        if frames.is_empty() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.scratch).map_err(|e| e.to_string())?;
        let list = self.scratch.join("frames.txt");
        let text: String = frames
            .iter()
            .map(|p| format!("{}\n", p.display()))
            .collect();
        std::fs::write(&list, text).map_err(|e| e.to_string())?;
        let mut command = python_env::script_command(&self.python, &self.script);
        command.arg("--frames-from").arg(&list);
        if let Some(siril) = &self.siril {
            command.arg("--siril").arg(siril);
        }
        let end = external_process::run_streaming(command, TIMEOUT, cancel, &mut |line| {
            if let Ok(raw) = serde_json::from_str::<RawPose>(line.trim()) {
                on_pose(raw);
            }
        })
        .map_err(|e| e.to_string());
        let _ = std::fs::remove_file(&list);
        let _ = std::fs::remove_dir(&self.scratch);
        let end = end?;
        if end.cancelled {
            return Ok(());
        }
        if end.timed_out {
            return Err("timed out".to_string());
        }
        if !end.succeeded() {
            let reason = end.stderr.lines().rev().find(|l| !l.trim().is_empty());
            return Err(reason.unwrap_or("the measure failed").to_string());
        }
        Ok(())
    }
}

/// What the window is told as the measure goes.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "event",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
// An event is built, sent to the window and dropped: boxing its pose would only add an allocation.
#[allow(clippy::large_enum_variant)]
pub enum PoseEvent {
    /// A pose is measured.
    Pose {
        family: String,
        measure: PoseMeasure,
        /// Measured so far, all families.
        done: usize,
        /// Planned so far: grows when a pose that departs brings its
        /// neighbours.
        planned: usize,
        /// The family judged with what is measured so far, so that the
        /// curves and the poses flagged come as they are measured.
        judged: bool,
        medians: Medians,
        flagged: Vec<FlaggedPose>,
    },
    /// The first pass is over and `count` neighbours of poses that depart
    /// are added.
    Neighbours { count: usize, planned: usize },
}

/// What to measure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasureRequest {
    pub night_folder: PathBuf,
    pub target_folder: String,
    /// `YYYY-MM-DD`, with `longitude`: the night's window.
    pub evening_date: String,
    pub longitude: Option<f64>,
    pub latitude: Option<f64>,
    /// What an earlier (stopped) measure of the target kept: not measured
    /// again.
    #[serde(default)]
    pub resume: Vec<PoseMeasure>,
}

/// One run of the measurer over a batch of frames, its answers filed in their
/// family.
struct Batch<'a> {
    measurer: &'a dyn Measurer,
    cancel: Option<&'a CancelFlag>,
    places: &'a BTreeMap<PathBuf, (usize, usize)>,
    families: &'a [PoseFamily],
    place: Option<Place>,
}

impl Batch<'_> {
    fn run(
        &self,
        frames: &[PathBuf],
        planned: usize,
        measured: &mut [Vec<PoseMeasure>],
        done_paths: &mut HashSet<PathBuf>,
        on_event: &mut dyn FnMut(&PoseEvent),
    ) -> Result<(), String> {
        let (places, families, place) = (self.places, self.families, self.place);
        self.measurer
            .measure(frames, self.cancel, &mut |raw: RawPose| {
                let Some(&(f, i)) = places.get(&raw.path) else {
                    return;
                };
                let mut pose = raw.into_measure(i);
                raw_clock(&mut pose, place);
                done_paths.insert(pose.path.clone());
                measured[f].push(pose.clone());
                let family = &families[f];
                let stats = judge(
                    &family.label,
                    family.exposure_seconds,
                    family.frames.len(),
                    &measured[f],
                    place,
                );
                on_event(&PoseEvent::Pose {
                    family: family.label.clone(),
                    measure: pose,
                    done: done_paths.len(),
                    planned,
                    judged: stats.judged,
                    medians: stats.medians,
                    flagged: stats.flagged,
                });
            })
    }
}

/// Measures a target's poses: the sample, then the neighbours of the poses
/// that depart. A stop gives what is measured so far (`stopped`).
pub fn measure_target(
    request: &MeasureRequest,
    measurer: &dyn Measurer,
    cancel: Option<&CancelFlag>,
    on_event: &mut dyn FnMut(&PoseEvent),
) -> Result<PoseStats, String> {
    let window = NightWindow::of_evening(&request.evening_date, request.longitude);
    let families = pose_families(
        &request.night_folder,
        &request.target_folder,
        window.as_ref(),
    );
    let place = match (request.latitude, request.longitude) {
        (Some(latitude), Some(longitude)) if latitude.is_finite() && longitude.is_finite() => {
            Some(Place {
                latitude,
                longitude,
            })
        }
        _ => None,
    };
    // path -> (family, index)
    let mut places: BTreeMap<PathBuf, (usize, usize)> = BTreeMap::new();
    for (f, family) in families.iter().enumerate() {
        for (i, frame) in family.frames.iter().enumerate() {
            places.insert(frame.clone(), (f, i));
        }
    }
    let mut measured: Vec<Vec<PoseMeasure>> = vec![Vec::new(); families.len()];
    let mut done_paths: HashSet<PathBuf> = HashSet::new();
    for kept in &request.resume {
        if let Some(&(f, i)) = places.get(&kept.path) {
            let mut kept = kept.clone();
            kept.index = i;
            done_paths.insert(kept.path.clone());
            measured[f].push(kept);
        }
    }
    let total: usize = families.iter().map(|f| f.frames.len()).sum();
    let mut queue: Vec<PathBuf> = Vec::new();
    for family in &families {
        for i in sample_indices(family.frames.len()) {
            let frame = &family.frames[i];
            if !done_paths.contains(frame) {
                queue.push(frame.clone());
            }
        }
    }
    let mut planned = done_paths.len() + queue.len();
    let mut stopped = false;
    let mut failure: Option<String> = None;

    let batch = Batch {
        measurer,
        cancel,
        places: &places,
        families: &families,
        place,
    };
    if let Err(error) = batch.run(&queue, planned, &mut measured, &mut done_paths, on_event) {
        failure = Some(error);
    }
    stopped |= cancel.is_some_and(CancelFlag::is_cancelled);

    // The second pass: the neighbours of every pose that departs, in the
    // families that were sampled.
    if failure.is_none() && !stopped {
        let mut neighbours: Vec<PathBuf> = Vec::new();
        for (f, family) in families.iter().enumerate() {
            if family.frames.len() <= SAMPLE_ALL_UP_TO {
                continue;
            }
            let stats = judge(
                &family.label,
                family.exposure_seconds,
                family.frames.len(),
                &measured[f],
                place,
            );
            let mut wanted: Vec<usize> = Vec::new();
            for flagged in &stats.flagged {
                let low = flagged.index.saturating_sub(SAMPLE_STEP - 1);
                let high = (flagged.index + SAMPLE_STEP - 1).min(family.frames.len() - 1);
                wanted.extend(low..=high);
            }
            wanted.sort_unstable();
            wanted.dedup();
            neighbours.extend(
                wanted
                    .into_iter()
                    .map(|i| family.frames[i].clone())
                    .filter(|p| !done_paths.contains(p)),
            );
        }
        if !neighbours.is_empty() {
            planned += neighbours.len();
            on_event(&PoseEvent::Neighbours {
                count: neighbours.len(),
                planned,
            });
            if let Err(error) = batch.run(
                &neighbours,
                planned,
                &mut measured,
                &mut done_paths,
                on_event,
            ) {
                failure = Some(error);
            }
            stopped |= cancel.is_some_and(CancelFlag::is_cancelled);
        }
    }
    if let Some(error) = failure {
        if done_paths.is_empty() {
            return Err(error);
        }
        // Some poses were measured before the failure: they are kept, the
        // measure counts as stopped.
        stopped = true;
    }
    let stats = families
        .iter()
        .enumerate()
        .map(|(f, family)| {
            judge(
                &family.label,
                family.exposure_seconds,
                family.frames.len(),
                &measured[f],
                place,
            )
        })
        .collect();
    let stats = PoseStats {
        families: stats,
        total,
        planned,
        stopped,
    };
    // The night's report sends the figures of each pose: kept beside the
    // night (a failure to write loses only those figures).
    let _ = record::save_measures(&request.night_folder, &request.target_folder, &stats);
    Ok(stats)
}

/* --------------------------------- Discarding ---------------------------- */

/// Leaves the poses in `discarded` out of every stacking of `plan`: each job
/// lists the frames it stacks (without them); nothing is deleted or moved. A
/// job whose frames would all be discarded is left as it is (a stacking
/// without a pose cannot run), and so is a job that renders the image a device
/// stacked (it reads no pose). Returns how many poses were left out.
pub fn apply_discard(
    plan: &mut TargetPlan,
    discarded: &HashSet<PathBuf>,
    night_folder: &Path,
    target_folder: &str,
    window: Option<&NightWindow>,
) -> usize {
    let mut left: Vec<PathBuf> = Vec::new();
    discard_jobs(
        plan,
        discarded,
        night_folder,
        target_folder,
        window,
        &mut left,
    )
}

/// [`apply_discard`] for a stacking that runs (never a dry run): the poses it
/// leaves out are also kept beside the night ([`record::save_left_out`]), so
/// the night's report says which poses the stacking left out, and why. A
/// stacking that leaves none out clears the list.
pub fn apply_discard_and_record(
    plan: &mut TargetPlan,
    discarded: &HashSet<PathBuf>,
    night_folder: &Path,
    target_folder: &str,
    window: Option<&NightWindow>,
) -> usize {
    let mut left: Vec<PathBuf> = Vec::new();
    let count = discard_jobs(
        plan,
        discarded,
        night_folder,
        target_folder,
        window,
        &mut left,
    );
    // A failure to write loses only the report's reasons.
    let _ = record::save_left_out(night_folder, target_folder, &left);
    count
}

fn discard_jobs(
    plan: &mut TargetPlan,
    discarded: &HashSet<PathBuf>,
    night_folder: &Path,
    target_folder: &str,
    window: Option<&NightWindow>,
    left: &mut Vec<PathBuf>,
) -> usize {
    if discarded.is_empty() {
        return 0;
    }
    let (families, _) = stack_families_within(night_folder, target_folder, window);
    let mut left_out = 0;
    for job in &mut plan.jobs {
        // The image of a device reads no pose: nothing to leave out.
        if job.device_master.is_some() {
            continue;
        }
        let frames: Vec<PathBuf> = if job.frames.is_empty() {
            match &job.family {
                Some(label) => families
                    .iter()
                    .find(|f| &f.label == label)
                    .map(|f| f.frames.clone())
                    .unwrap_or_default(),
                None => families
                    .first()
                    .map(|f| f.frames.clone())
                    .unwrap_or_default(),
            }
        } else {
            job.frames.clone()
        };
        let kept: Vec<PathBuf> = frames
            .iter()
            .filter(|f| !discarded.contains(*f))
            .cloned()
            .collect();
        if kept.is_empty() || kept.len() == frames.len() {
            continue;
        }
        left_out += frames.len() - kept.len();
        left.extend(frames.iter().filter(|f| discarded.contains(*f)).cloned());
        job.frames = kept;
    }
    left_out
}

/// The poses a list names (`stack --leave-out <file>`, the command line's
/// "Leave out the flagged frames"): one name a line, a file name
/// (`Light_0007.fit`) or a path ending as the pose's does
/// (`lights/Light_0007.fit`, `M31/lights/Light_0007.fit`); blank lines and
/// lines starting with `#` are skipped, case counts. Returns the poses of
/// `poses` named, and the names that are no pose of them (said by the
/// caller, never an error: the others are left out all the same).
#[must_use]
pub fn poses_named(list: &str, poses: &[PathBuf]) -> (HashSet<PathBuf>, Vec<String>) {
    let mut named = HashSet::new();
    let mut unknown: Vec<String> = Vec::new();
    for line in list.lines() {
        let name = line.trim().trim_matches('"');
        if name.is_empty() || name.starts_with('#') {
            continue;
        }
        let wanted = Path::new(&name.replace('\\', "/")).to_path_buf();
        let hits: Vec<&PathBuf> = poses
            .iter()
            .filter(|pose| pose.ends_with(&wanted))
            .collect();
        if hits.is_empty() {
            if !unknown.iter().any(|known| known == name) {
                unknown.push(name.to_string());
            }
        } else {
            named.extend(hits.into_iter().cloned());
        }
    }
    (named, unknown)
}

#[cfg(test)]
mod tests;

//! The images of the progression: "A practical
//! aid, not a processing tool."
//!
//! For one target of a night and one family of its frames (same filter,
//! exposure, gain, binning, temperature), Siril stacks the frames by
//! milestones -- one frame, then about a quarter, half, three quarters and
//! all of them -- and each milestone becomes a JPEG, so the person sees the
//! night grow. **The settings are fixed** (no screen to tune them):
//! calibration by a master dark of the family's exposure and gain and a
//! master flat of its filter when the night's `calibration/` holds such
//! frames, registration, a stack with rejection, a display stretch, a JPEG
//! at quality 90.
//!
//! - **Colour**: a colour sensor's frames (a `BAYERPAT`
//!   in the first frame's header, the stacking's own test) are debayered
//!   -- `calibrate … -cfa -equalize_cfa -debayer`, or `convert … -debayer`
//!   without calibration -- and the milestones stacked with `-rgb_equal`.
//! - **The display** ([`display_commands`]): `autostretch -linked` with a
//!   black point at about the 0.05th percentile of the background
//!   ([`STRETCH_SHADOWS_CLIP`], in background sigmas) and a dark sky
//!   ([`STRETCH_TARGET_BACKGROUND`]), so the milestones read like the
//!   stacked image and not on Siril's default grey (0.25). The first
//!   milestone of a colour sensor is a single frame, which no stack has
//!   equalised: its stretch is unlinked, which levels the three channels'
//!   backgrounds as `-rgb_equal` does for the others (a linked stretch
//!   leaves it green, measured on a night of an ASI533MC).
//! - **Where the work is done**: on the disk of the
//!   night, in `<night>/.astro-sync-work/progression/<target>/<family>/`
//!   ([`crate::working_folder::progression_scratch_root`]), never in Astro
//!   Sync's data on the system disk. The room is measured there first
//!   (about [`BYTES_PER_LIGHT`] per frame, as the stacking;
//!   [`ProgressionFailure::NotEnoughSpace`]). The frames are reached through
//!   links (a copy only where a link cannot be made), Siril reads them and
//!   writes beside them. When the run ends -- success, failure or stop --
//!   the scratch is removed, and `.astro-sync-work/` with it when empty.
//! - **What stays**: the JPEG of each milestone, copied before the
//!   scratch goes into `<night>/<target>/progression/milestone_NN.jpg`
//!   ([`crate::working_folder::PROGRESSION_DIR`]), a result of Astro Sync
//!   like `stacked/`, and the record of the poses each one stacks
//!   ([`PROGRESSION_RECORD`]), which a later sending reads
//!   ([`recorded_candidates`]). A new run replaces the milestones of that
//!   folder.
//! - **What leaves** ([`progression_candidates`]): each JPEG as
//!   [`ResultKind::Progression`] with the frames it stacks; the precheck
//!   attaches it to the target's capture as its history
//!   ([`crate::precheck::plan_images`]), never as a capture of its own.
//!   The option is ticked at "Processing" or used from the
//!   "Progression" tab; the whole is behind the capability
//!   `analysis.progression`, and Siril is the one Settings › "Dependencies"
//!   found.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::analysis::fits::{self, AcquisitionFamily, group_into_families};
use crate::analysis::siril::script::{MILESTONE_OUTPUT_MARKER, Milestone, build_milestones};
use crate::detection::{FrameType, NightWindow, frame_record, read_frame_header};
use crate::external_process;
use crate::local_work::CancelFlag;
use crate::precheck::ImageCandidate;
use crate::scanner::{FileClass, walk_folder};
use crate::siril_progress::{POLL_EVERY, SirilFollower, now_ms};
use crate::stacking::BYTES_PER_LIGHT;
use crate::upload_queue::ResultKind;
use crate::working_folder::{
    CALIBRATION_DIR, LIGHTS_DIR, PROGRESSION_DIR, progression_scratch_root, remove_scratch,
};

/// The fixed settings, written in every script and record.
pub const REJECTION: &str = "rej 3 3";
pub const NORMALIZATION: &str = "addscale";
pub const JPEG_QUALITY: u32 = 90;
/// The black point of the display, in sigmas of the background below its
/// median (`autostretch`'s `shadowsclip`): -3.29 σ is the 0.05th percentile
/// of a Gaussian background, so almost no sky pixel is clipped to black.
pub const STRETCH_SHADOWS_CLIP: &str = "-3.29";
/// Where the display puts the sky background (`autostretch`'s `targetbg`,
/// Siril's default 0.25 gives the grey the sky check ("contrôle du ciel") of the portal
/// reads as "not a sky"): a dark sky, like the stacked image.
pub const STRETCH_TARGET_BACKGROUND: &str = "0.10";

/// One frame read: its path and its acquisition record.
fn frames_of(folder: &Path) -> Vec<(PathBuf, fits::FitsAcquisitionRecord, Option<FrameType>)> {
    let Ok(walk) = walk_folder(folder, None) else {
        return Vec::new();
    };
    let mut frames: Vec<_> = walk
        .files
        .iter()
        .filter_map(|file| {
            let class = FileClass::classify(
                file.path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or_default(),
            );
            let header = read_frame_header(&file.path, class)?;
            let kind = frame_record(&header).image_type;
            Some((
                file.path.clone(),
                fits::extract_acquisition_record(&header),
                kind,
            ))
        })
        .collect();
    frames.sort_by(|a, b| (&a.1.date_obs_utc, &a.0).cmp(&(&b.1.date_obs_utc, &b.0)));
    frames
}

/// The families of a target's light frames (`<target>/lights/`, links
/// followed; a frame whose header says another type is left out).
#[must_use]
pub fn target_families(night_folder: &Path, target_folder: &str) -> Vec<AcquisitionFamily> {
    target_families_within(night_folder, target_folder, None).0
}

/// [`target_families`] of the frames taken inside the night's window, and
/// the frames left out because they are outside it (`DATE-OBS`; a frame
/// without a date stays in). `None`: every frame.
#[must_use]
pub fn target_families_within(
    night_folder: &Path,
    target_folder: &str,
    window: Option<&NightWindow>,
) -> (Vec<AcquisitionFamily>, Vec<PathBuf>) {
    let lights = night_folder.join(target_folder).join(LIGHTS_DIR);
    // The filter the target folder records, for the lights whose headers name
    // none (the frames are never written).
    let assumed = crate::working_folder::read_target_filter(&night_folder.join(target_folder));
    let mut outside = Vec::new();
    let records: Vec<(String, fits::FitsAcquisitionRecord)> = frames_of(&lights)
        .into_iter()
        .map(|(path, mut record, kind)| {
            if record.filter.is_none() {
                record.filter.clone_from(&assumed);
            }
            (path, record, kind)
        })
        .filter(|(_, _, kind)| kind.is_none_or(|k| k == FrameType::Light))
        .filter(|(path, record, _)| {
            let inside = window.is_none_or(|w| {
                w.holds_clock(record.date_obs_utc.as_deref(), record.date_zone_unknown)
            });
            if !inside {
                outside.push(path.clone());
            }
            inside
        })
        .map(|(path, record, _)| (path.display().to_string(), record))
        .collect();
    (group_into_families(&records), outside)
}

/// One progression to make.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressionJob {
    pub night_folder: PathBuf,
    pub target_folder: String,
    pub family: AcquisitionFamily,
    /// The scratch, `<night>/.astro-sync-work/progression/<target>/<family>/`.
    pub work_dir: PathBuf,
    /// Where the milestones' JPEGs stay, `<night>/<target>/progression/`.
    pub output_dir: PathBuf,
    /// The frames come from a colour sensor (a `BAYERPAT`): they are
    /// debayered.
    pub colour: bool,
    /// The frames are Fujifilm X-Trans RAW files: Siril's
    /// `calibrate -debayer` leaves that mosaic as it is, so every frame is
    /// debayered when it is converted, the calibration frames too.
    #[serde(default)]
    pub xtrans: bool,
}

fn safe(component: &str) -> String {
    component
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Whether `frame` comes from a colour sensor: a two-dimensional image with
/// a Bayer pattern in its header (`BAYERPAT`), the test of the stacking
/// script (`astro_sync_stack.py`'s `is_colour`).
#[must_use]
pub fn is_colour_frame(frame: &Path) -> bool {
    let class = FileClass::classify(
        frame
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default(),
    );
    read_frame_header(frame, class).is_some_and(|header| {
        let two_dimensional = header.get("NAXIS").is_none_or(|naxis| naxis.trim() == "2");
        let bayer = header
            .get("BAYERPAT")
            .is_some_and(|pattern| !pattern.trim().is_empty());
        two_dimensional && bayer
    })
}

/// Whether `frame` is a Fujifilm X-Trans RAW file (`BAYERPAT` says `XTRANS`).
#[must_use]
pub fn is_xtrans_frame(frame: &Path) -> bool {
    let class = FileClass::classify(
        frame
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default(),
    );
    read_frame_header(frame, class).is_some_and(|header| {
        header
            .get("BAYERPAT")
            .is_some_and(|pattern| pattern.trim().eq_ignore_ascii_case("XTRANS"))
    })
}

impl ProgressionJob {
    /// The job of `family`, a family of the lights of `target_folder` in
    /// `night_folder`: its scratch and its output on the night's disk, and
    /// whether its first frame is a colour one.
    #[must_use]
    pub fn new(night_folder: &Path, target_folder: &str, family: AcquisitionFamily) -> Self {
        let work_dir = progression_scratch_root(night_folder)
            .join(safe(target_folder))
            .join(safe(&family.key));
        let colour = family
            .frame_paths
            .first()
            .is_some_and(|first| is_colour_frame(Path::new(first)));
        let xtrans = family
            .frame_paths
            .first()
            .is_some_and(|first| is_xtrans_frame(Path::new(first)));
        Self {
            night_folder: night_folder.to_path_buf(),
            target_folder: target_folder.to_string(),
            output_dir: night_folder.join(target_folder).join(PROGRESSION_DIR),
            family,
            work_dir,
            colour,
            xtrans,
        }
    }
}

/// The calibration frames of the night that fit `family`: darks of its
/// exposure and gain, flats of its filter (by header, or by a sub-folder of
/// `flats/` named after the filter).
fn calibration_for(
    night_folder: &Path,
    family: &AcquisitionFamily,
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let close = |a: Option<f64>, b: Option<f64>| match (a, b) {
        (Some(a), Some(b)) => (a - b).abs() < 1e-3,
        _ => false,
    };
    let calibration = night_folder.join(CALIBRATION_DIR);
    let darks = frames_of(&calibration.join("darks"))
        .into_iter()
        .filter(|(_, _, kind)| kind.is_none_or(|k| k == FrameType::Dark))
        .filter(|(_, r, _)| {
            close(r.exposure_seconds, family.exposure_seconds) && close(r.gain, family.gain)
        })
        .map(|(path, _, _)| path)
        .collect();
    let flats_folder = calibration.join("flats");
    let flats = frames_of(&flats_folder)
        .into_iter()
        .filter(|(_, _, kind)| kind.is_none_or(|k| k == FrameType::Flat))
        .filter(|(path, r, _)| {
            let by_folder = path
                .strip_prefix(&flats_folder)
                .ok()
                .and_then(|rest| rest.components().next())
                .filter(|_| path.parent() != Some(flats_folder.as_path()))
                .map(|c| c.as_os_str().to_string_lossy().to_string());
            let filter = r.filter.clone().or(by_folder);
            match (&filter, &family.filter) {
                (Some(a), Some(b)) => a.trim().eq_ignore_ascii_case(b.trim()),
                (None, None) => true,
                _ => false,
            }
        })
        .map(|(path, _, _)| path)
        .collect();
    (darks, flats)
}

/// What the script is made from, besides the work folder and the family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptInputs<'a> {
    pub siril_version: &'a str,
    pub has_darks: bool,
    pub has_flats: bool,
    /// A colour sensor: the frames are debayered (see the module).
    pub colour: bool,
    /// Fujifilm X-Trans RAW files: debayered at every conversion.
    pub xtrans: bool,
}

/// The first part of the script, from the work folder's sub-folders
/// `frames/`, `darks/`, `flats/` (links to the night's files) into
/// `process/`: the master calibration frames, the calibration (and the
/// debayering of a colour sensor's frames) and the registration. Relative
/// `-out=` paths: Siril's `convert` does not accept a quoted path, and a
/// night's folder name holds spaces. Returns the script and the name of the
/// registered sequence.
#[must_use]
pub fn registration_script(
    work_dir: &Path,
    family: &AcquisitionFamily,
    inputs: ScriptInputs<'_>,
) -> (String, String) {
    let ScriptInputs {
        siril_version,
        has_darks,
        has_flats,
        colour,
        xtrans,
    } = inputs;
    // Siril's `calibrate -debayer` leaves an X-Trans mosaic as it is: such
    // frames are debayered as they are converted, the calibration frames too,
    // and calibrated as colour frames.
    let debayer_calibration = if xtrans { " -debayer" } else { "" };
    let process = work_dir.join("process");
    let mut lines = vec![
        "# Astro Sync -- progression, fixed settings (1/2: calibration, registration).".to_string(),
        format!("# Family: {}", family.key),
        format!("# Frames: {}", family.frame_paths.len()),
        format!("requires {siril_version}"),
    ];
    let mut calibrate = Vec::new();
    if has_darks {
        lines.extend([
            format!("cd \"{}\"", work_dir.join("darks").display()),
            format!("convert dark{debayer_calibration} -out=../process"),
            format!("cd \"{}\"", process.display()),
            format!("stack dark_ {REJECTION} -nonorm -out=master_dark"),
        ]);
        calibrate.push("-dark=master_dark".to_string());
    }
    if has_flats {
        lines.extend([
            format!("cd \"{}\"", work_dir.join("flats").display()),
            format!("convert flat{debayer_calibration} -out=../process"),
            format!("cd \"{}\"", process.display()),
            format!("stack flat_ {REJECTION} -norm=mul -out=master_flat"),
        ]);
        calibrate.push("-flat=master_flat".to_string());
    }
    // A colour sensor's frames are debayered once: at the calibration when
    // there is one (the CFA calibrated first, its channels equalised by the
    // flat), else when they are converted.
    let debayer_at_convert = xtrans || (colour && calibrate.is_empty());
    lines.extend([
        format!("cd \"{}\"", work_dir.join("frames").display()),
        format!(
            "convert light{} -out=../process",
            if debayer_at_convert { " -debayer" } else { "" }
        ),
        format!("cd \"{}\"", process.display()),
    ]);
    let calibrated = if calibrate.is_empty() {
        "light_".to_string()
    } else {
        if colour && !xtrans {
            calibrate.extend(["-cfa", "-equalize_cfa", "-debayer"].map(String::from));
        }
        lines.push(format!("calibrate light_ {}", calibrate.join(" ")));
        "pp_light_".to_string()
    };
    lines.push(format!("register {calibrated}"));
    (lines.join("\n") + "\n", format!("r_{calibrated}"))
}

/// The registered sequence as Siril leaves it: `register` keeps
/// only the frames it could align, **renumbering nothing** (the file of an
/// image that failed is just missing) and listing them in `<name>.seq`.
/// `select` and `unselect` count by **position, from 1**, in that list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredSequence {
    /// The file index of each registered image (`r_light_00019` → 19), in
    /// the sequence's order.
    pub indices: Vec<usize>,
}

impl RegisteredSequence {
    /// Reads `<process>/<name>.seq`, the lines `I <index> <selected>`.
    pub fn read(process: &Path, name: &str) -> Result<Self, String> {
        let file = process.join(format!("{name}.seq"));
        let text = fs::read_to_string(&file)
            .map_err(|e| format!("the registered sequence {name} is missing ({e})"))?;
        let indices: Vec<usize> = text
            .lines()
            .filter_map(|line| line.strip_prefix("I "))
            .filter_map(|rest| rest.split_whitespace().next()?.parse().ok())
            .collect();
        if indices.is_empty() {
            return Err("Siril registered no frame (not enough stars?)".to_string());
        }
        Ok(Self { indices })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.indices.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

/// The second part: the milestones over the frames Siril **registered**.
/// Milestone 1 is the first registered image; each later one stacks the
/// first `frame_count` of the sequence, selected from 1 (`unselect` and
/// `select` are 1-based and inclusive, bounded by the registered count: the
/// frames that failed to register are not in the sequence).
#[must_use]
pub fn milestones_script(
    work_dir: &Path,
    registered_name: &str,
    registered: &RegisteredSequence,
    milestones: &[Milestone],
    siril_version: &str,
    colour: bool,
) -> String {
    let process = work_dir.join("process");
    let count = registered.len();
    let mut lines = vec![
        "# Astro Sync -- progression, fixed settings (2/2: milestones).".to_string(),
        format!("# Registered frames: {count}"),
        format!("requires {siril_version}"),
        format!("cd \"{}\"", process.display()),
        // The frames are already aligned: the registration data would only
        // make Siril 1.4.4 stop in OpenCV (`gemm`, the stack of a sequence
        // `register` wrote), so it is cleared before the first stack.
        format!("seqclean {registered_name} -reg"),
    ];
    for milestone in milestones {
        lines.push(format!(
            "# Milestone {}: {} frame(s).",
            milestone.index, milestone.frame_count
        ));
        if milestone.index == 1 {
            lines.push(format!(
                "load {registered_name}{:05}",
                registered.indices[0]
            ));
        } else {
            lines.push(format!("unselect {registered_name} 1 {count}"));
            lines.push(format!(
                "select {registered_name} 1 {}",
                milestone.frame_count.min(count)
            ));
            lines.push(format!(
                "stack {registered_name} {REJECTION} -filter-included -norm={NORMALIZATION}{} -out={}",
                if colour { " -rgb_equal" } else { "" },
                milestone.label
            ));
            lines.push(format!("load {}", milestone.label));
        }
        lines.extend(display_commands(colour && milestone.index == 1));
        lines.push(format!("savejpg {} {JPEG_QUALITY}", milestone.label));
        lines.push(format!("{MILESTONE_OUTPUT_MARKER}{}.jpg", milestone.label));
    }
    lines.join("\n") + "\n"
}

/// The display of a milestone before its JPEG (see the module): one linked
/// stretch -- the channels keep the ratios the stack equalised, as in the
/// stacked image -- with the black point and the dark sky of the fixed
/// settings; unlinked when `level_channels` (a colour frame no stack has
/// equalised: the first milestone).
#[must_use]
pub fn display_commands(level_channels: bool) -> Vec<String> {
    let linked = if level_channels { "" } else { "-linked " };
    vec![format!(
        "autostretch {linked}{STRETCH_SHADOWS_CLIP} {STRETCH_TARGET_BACKGROUND}"
    )]
}

/// The record a progression leaves beside its milestones
/// (`<target>/progression/progression.json`): the poses each one stacks, so a
/// later sending ([`recorded_candidates`]) knows them without Siril.
pub const PROGRESSION_RECORD: &str = "progression.json";

/// One image of the progression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressionImage {
    pub index: usize,
    pub frame_count: usize,
    pub path: PathBuf,
    pub sha256: String,
    pub size: u64,
}

/// A progression made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressionImages {
    pub family_key: String,
    pub siril_version: String,
    /// Both parts of the script, one after the other (the files themselves
    /// went with the scratch).
    pub script: String,
    /// The frames were debayered (a colour sensor).
    pub colour: bool,
    /// The frames of the family and how many of them Siril registered: the
    /// milestones count the latter.
    pub frames_total: usize,
    pub frames_registered: usize,
    pub calibrated_with_darks: bool,
    pub calibrated_with_flats: bool,
    pub images: Vec<ProgressionImage>,
}

/// Why no progression came out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "why",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ProgressionFailure {
    /// `analysis.progression` is not granted ("The progression is reserved
    /// to ...", "See the offers").
    NotInSubscription,
    /// "Siril cannot be found: the progression cannot be generated
    /// here."
    SirilMissing,
    NoFrames,
    Cancelled,
    /// Siril stopped with an error (its last lines, for the ⓘ).
    Failed {
        detail: String,
    },
    /// A milestone's image was not written.
    MissingImage {
        file: String,
    },
    /// Less free room on the disk of the night than the scratch needs
    /// (about [`BYTES_PER_LIGHT`] per frame); `folder` is the night's
    /// folder, the disk to free.
    NotEnoughSpace {
        needed: u64,
        available: u64,
        folder: String,
    },
}

fn io_failure(error: &std::io::Error) -> ProgressionFailure {
    ProgressionFailure::Failed {
        detail: error.to_string(),
    }
}

fn link_or_copy(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(from, to)
    }
    #[cfg(not(unix))]
    {
        fs::hard_link(from, to).or_else(|_| fs::copy(from, to).map(|_| ()))
    }
}

fn link_all(frames: &[PathBuf], folder: &Path, stem: &str) -> std::io::Result<()> {
    fs::create_dir_all(folder)?;
    for (index, frame) in frames.iter().enumerate() {
        let extension = frame
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("fits")
            .to_ascii_lowercase();
        link_or_copy(
            frame,
            &folder.join(format!("{stem}_{:05}.{extension}", index + 1)),
        )?;
    }
    Ok(())
}

fn digest(path: &Path) -> std::io::Result<(String, u64)> {
    use sha2::{Digest, Sha256};
    let bytes = fs::read(path)?;
    Ok((format!("{:x}", Sha256::digest(&bytes)), bytes.len() as u64))
}

/// What the progression says while it works: which of its two Siril runs
/// is going, which milestone is being made, and what Siril's journal says
/// (the percentage of its current command, its last useful line).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ProgressionEvent {
    /// A part starts: the registration of the frames, then the milestones.
    Stage { stage: ProgressionStage },
    /// The milestone being made, `index` of `total` (1-based).
    Milestone { index: u32, total: u32 },
    /// See [`crate::siril_progress::SirilReport`].
    Progress {
        percent: Option<f64>,
        line: Option<String>,
        at: u64,
    },
}

/// The two parts of a progression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressionStage {
    Registration,
    Milestones,
}

/// Runs one script of the progression with Siril, reading what it says as it
/// goes. `made`: the files the script writes one after the other (the
/// milestones' JPEGs): how many exist says how far it is.
fn run_siril(
    siril_path: &Path,
    script_path: &Path,
    work_dir: &Path,
    timeout: Duration,
    cancel: Option<&CancelFlag>,
    made: &[PathBuf],
    on_event: &mut dyn FnMut(&ProgressionEvent),
) -> Result<(), ProgressionFailure> {
    let mut command = Command::new(siril_path);
    // Siril's own configuration, never the person's.
    let config =
        external_process::siril_config_in(work_dir).map_err(|e| ProgressionFailure::Failed {
            detail: e.to_string(),
        })?;
    command
        .arg("-i")
        .arg(config)
        .arg("-s")
        .arg(script_path)
        .current_dir(work_dir);
    let watch = std::cell::RefCell::new((SirilFollower::new(), 0_usize, on_event));
    let report = |draw_files: bool| {
        let mut guard = watch.borrow_mut();
        let (follower, counted, on_event) = &mut *guard;
        if draw_files && !made.is_empty() {
            let done = made.iter().filter(|path| path.is_file()).count();
            if done != *counted {
                *counted = done;
                on_event(&ProgressionEvent::Milestone {
                    index: u32::try_from((done + 1).min(made.len())).unwrap_or(u32::MAX),
                    total: u32::try_from(made.len()).unwrap_or(u32::MAX),
                });
            }
        }
        if let Some(report) = follower.take(now_ms()) {
            on_event(&ProgressionEvent::Progress {
                percent: report.percent,
                line: report.line,
                at: report.at,
            });
        }
    };
    let end = external_process::run_polled(
        command,
        timeout,
        cancel,
        None,
        POLL_EVERY,
        &mut |line| watch.borrow_mut().0.feed(line, now_ms()),
        &mut || report(true),
    )
    .map_err(|e| ProgressionFailure::Failed {
        detail: e.to_string(),
    })?;
    if end.cancelled {
        return Err(ProgressionFailure::Cancelled);
    }
    if !end.succeeded() {
        let text = format!("{}\n{}", end.stdout, end.stderr);
        let detail: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        return Err(ProgressionFailure::Failed {
            detail: detail[detail.len().saturating_sub(6)..].join(" | "),
        });
    }
    report(true);
    Ok(())
}

/// Makes the progression of `job` with the Siril found by
/// "Dependencies" (`siril`: its path and version), when `allowed`
/// (`analysis.progression`). The room is checked first on the disk of the
/// night (`free`: the free bytes of a folder,
/// [`crate::stacking::free_bytes`] outside the tests); the scratch is gone
/// when this returns, whatever the outcome, and only the milestones' JPEGs
/// stay, in `job.output_dir`.
pub fn run_progression_images(
    job: &ProgressionJob,
    allowed: bool,
    siril: Option<(&Path, &str)>,
    free: &dyn Fn(&Path) -> Option<u64>,
    timeout: Duration,
    cancel: Option<&CancelFlag>,
) -> Result<ProgressionImages, ProgressionFailure> {
    run_progression_images_with(job, allowed, siril, free, timeout, cancel, &mut |_| {})
}

/// [`run_progression_images`], saying what it is doing as it goes
/// ([`ProgressionEvent`]).
pub fn run_progression_images_with(
    job: &ProgressionJob,
    allowed: bool,
    siril: Option<(&Path, &str)>,
    free: &dyn Fn(&Path) -> Option<u64>,
    timeout: Duration,
    cancel: Option<&CancelFlag>,
    on_event: &mut dyn FnMut(&ProgressionEvent),
) -> Result<ProgressionImages, ProgressionFailure> {
    if !allowed {
        return Err(ProgressionFailure::NotInSubscription);
    }
    let (siril_path, siril_version) = siril.ok_or(ProgressionFailure::SirilMissing)?;
    if job.family.frame_paths.is_empty() {
        return Err(ProgressionFailure::NoFrames);
    }
    // What a hard stop of the app left of this job goes first.
    remove_scratch(&job.work_dir);
    let needed = job.family.frame_paths.len() as u64 * BYTES_PER_LIGHT;
    if let Some(available) = free(&job.night_folder)
        && available < needed
    {
        return Err(ProgressionFailure::NotEnoughSpace {
            needed,
            available,
            folder: job.night_folder.display().to_string(),
        });
    }
    let made = make_progression(job, siril_path, siril_version, timeout, cancel, on_event);
    remove_scratch(&job.work_dir);
    made
}

/// The work itself, in the scratch: links, the two scripts, Siril, then the
/// JPEGs copied out.
fn make_progression(
    job: &ProgressionJob,
    siril_path: &Path,
    siril_version: &str,
    timeout: Duration,
    cancel: Option<&CancelFlag>,
    on_event: &mut dyn FnMut(&ProgressionEvent),
) -> Result<ProgressionImages, ProgressionFailure> {
    let frames: Vec<PathBuf> = job.family.frame_paths.iter().map(PathBuf::from).collect();
    let (darks, flats) = calibration_for(&job.night_folder, &job.family);
    link_all(&frames, &job.work_dir.join("frames"), "light").map_err(|e| io_failure(&e))?;
    if !darks.is_empty() {
        link_all(&darks, &job.work_dir.join("darks"), "dark").map_err(|e| io_failure(&e))?;
    }
    if !flats.is_empty() {
        link_all(&flats, &job.work_dir.join("flats"), "flat").map_err(|e| io_failure(&e))?;
    }
    let process = job.work_dir.join("process");
    fs::create_dir_all(&process).map_err(|e| io_failure(&e))?;
    let (registration, registered_name) = registration_script(
        &job.work_dir,
        &job.family,
        ScriptInputs {
            siril_version,
            has_darks: !darks.is_empty(),
            has_flats: !flats.is_empty(),
            colour: job.colour,
            xtrans: job.xtrans,
        },
    );
    let script_path = job.work_dir.join("progression.ssf");
    fs::write(&script_path, &registration).map_err(|e| io_failure(&e))?;
    on_event(&ProgressionEvent::Stage {
        stage: ProgressionStage::Registration,
    });
    run_siril(
        siril_path,
        &script_path,
        &job.work_dir,
        timeout,
        cancel,
        &[],
        on_event,
    )?;

    // Siril keeps the frames it could register: the milestones count them.
    let registered = RegisteredSequence::read(&process, &registered_name)
        .map_err(|detail| ProgressionFailure::Failed { detail })?;
    let milestones = build_milestones(registered.len());
    let milestones_text = milestones_script(
        &job.work_dir,
        &registered_name,
        &registered,
        &milestones,
        siril_version,
        job.colour,
    );
    let milestones_path = job.work_dir.join("progression-milestones.ssf");
    fs::write(&milestones_path, &milestones_text).map_err(|e| io_failure(&e))?;
    on_event(&ProgressionEvent::Stage {
        stage: ProgressionStage::Milestones,
    });
    on_event(&ProgressionEvent::Milestone {
        index: 1,
        total: u32::try_from(milestones.len()).unwrap_or(u32::MAX),
    });
    let made: Vec<PathBuf> = milestones
        .iter()
        .map(|milestone| process.join(format!("{}.jpg", milestone.label)))
        .collect();
    run_siril(
        siril_path,
        &milestones_path,
        &job.work_dir,
        timeout,
        cancel,
        &made,
        on_event,
    )?;
    let script = format!("{registration}\n{milestones_text}");

    let mut written = Vec::new();
    for milestone in &milestones {
        let file = format!("{}.jpg", milestone.label);
        let path = process.join(&file);
        if !path.is_file() {
            return Err(ProgressionFailure::MissingImage { file });
        }
        written.push((milestone, file, path));
    }
    // Every milestone is there: the new set replaces the old one.
    remove_old_milestones(&job.output_dir).map_err(|e| io_failure(&e))?;
    fs::create_dir_all(&job.output_dir).map_err(|e| io_failure(&e))?;
    let mut images = Vec::new();
    for (milestone, file, path) in written {
        let kept = job.output_dir.join(&file);
        fs::copy(&path, &kept).map_err(|e| io_failure(&e))?;
        let (sha256, size) = digest(&kept).map_err(|e| io_failure(&e))?;
        images.push(ProgressionImage {
            index: milestone.index,
            frame_count: milestone.frame_count,
            path: kept,
            sha256,
            size,
        });
    }
    write_record(&job.output_dir, &images).map_err(|e| io_failure(&e))?;
    Ok(ProgressionImages {
        family_key: job.family.key.clone(),
        siril_version: siril_version.to_string(),
        script,
        colour: job.colour,
        frames_total: frames.len(),
        frames_registered: registered.len(),
        calibrated_with_darks: !darks.is_empty(),
        calibrated_with_flats: !flats.is_empty(),
        images,
    })
}

#[derive(Serialize, Deserialize)]
struct RecordedImage {
    file: String,
    frames: u64,
}

#[derive(Serialize, Deserialize)]
struct Record {
    format: u32,
    images: Vec<RecordedImage>,
}

fn write_record(output_dir: &Path, images: &[ProgressionImage]) -> std::io::Result<()> {
    let record = Record {
        format: 1,
        images: images
            .iter()
            .filter_map(|image| {
                Some(RecordedImage {
                    file: image.path.file_name()?.to_string_lossy().into_owned(),
                    frames: image.frame_count as u64,
                })
            })
            .collect(),
    };
    let mut text = serde_json::to_string_pretty(&record).map_err(std::io::Error::other)?;
    text.push('\n');
    fs::write(output_dir.join(PROGRESSION_RECORD), text)
}

/// The milestones a progression left in `<night>/<target>/progression/`,
/// as results that could leave: each JPEG the record names and the folder
/// still holds, with the poses it stacks. What a sending from the command
/// line adds to the images of the night's folder, as the window adds the
/// progression it has just made.
#[must_use]
pub fn recorded_candidates(
    night_folder: &Path,
    target_folder: &str,
    target_id: &str,
) -> Vec<ImageCandidate> {
    let folder = night_folder.join(target_folder).join(PROGRESSION_DIR);
    let Some(record) = fs::read_to_string(folder.join(PROGRESSION_RECORD))
        .ok()
        .and_then(|text| serde_json::from_str::<Record>(&text).ok())
    else {
        return Vec::new();
    };
    record
        .images
        .into_iter()
        .filter(|image| is_milestone_jpeg(&image.file))
        .filter_map(|image| {
            let path = folder.join(&image.file);
            let (sha256, size) = digest(&path).ok()?;
            Some(ImageCandidate {
                render_rank: None,
                target_id: target_id.to_string(),
                path,
                sha256,
                size,
                kind: ResultKind::Progression,
                render: None,
                contrast: crate::stacking::Contrast::None,
                retouch: crate::stacking::Retouch::default(),
                frame_count: Some(image.frames),
                twin_sha256: None,
            })
        })
        .collect()
}

/// Whether `name` is the JPEG of a milestone as Astro Sync names it
/// (`milestone_01.jpg`).
fn is_milestone_jpeg(name: &str) -> bool {
    name.strip_prefix("milestone_")
        .and_then(|rest| rest.strip_suffix(".jpg"))
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// Removes the milestones' JPEGs a previous run wrote in `output_dir`
/// (nothing else of the folder).
fn remove_old_milestones(output_dir: &Path) -> std::io::Result<()> {
    let Ok(entries) = fs::read_dir(output_dir) else {
        return Ok(());
    };
    for entry in entries.filter_map(Result::ok) {
        if entry.file_type().is_ok_and(|t| t.is_file())
            && is_milestone_jpeg(&entry.file_name().to_string_lossy())
        {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// The images as results that could leave, each with the frames it
/// stacks: the history of the target's capture
/// ([`crate::precheck::plan_images`]), never a capture of its own.
#[must_use]
pub fn progression_candidates(images: &ProgressionImages, target_id: &str) -> Vec<ImageCandidate> {
    images
        .images
        .iter()
        .map(|image| ImageCandidate {
            render_rank: None,
            target_id: target_id.to_string(),
            path: image.path.clone(),
            sha256: image.sha256.clone(),
            size: image.size,
            kind: ResultKind::Progression,
            render: None,
            contrast: crate::stacking::Contrast::None,
            retouch: crate::stacking::Retouch::default(),
            frame_count: Some(image.frame_count as u64),
            twin_sha256: None,
        })
        .collect()
}

#[cfg(test)]
mod tests;

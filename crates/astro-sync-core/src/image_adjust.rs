//! « Régler l'image »: the settings a person applies to a finished
//! render (never to the linear data), what they leave on disk, and what
//! leaves in its place.
//!
//! - [`AdjustValues`]: point noir, luminosité, couleur, lissage, étoiles,
//!   aigrettes (strength and reach: light added, not in the person's data;
//!   the record of the values is where a later reader finds that an image
//!   set carries them: `spikes > 0`).
//!   The arithmetic is `stacking/image_adjust.py`'s (the whole image) and
//!   the window's (the preview); this module never touches pixels.
//! - The image set is written **beside** the render, never over it:
//!   `<target>/stacked/adjusted/<stem>_adjusted.png`, with its record
//!   `<target>/stacked/adjusted/<render file name>.json` ([`AdjustRecord`]:
//!   the values, so the screen reopens them as they were, and what the two
//!   files were when it was written). The folder is under `stacked/` but one
//!   level down, so the reading of a night's folder never takes the image set
//!   for a render of its own (`night_steps::local_images` reads `stacked/`
//!   only).
//! - **The image set replaces the render in the sending** ([`sent_file`]):
//!   the check of a night counts its digest and size
//!   ([`substitute_candidates`]), and the queue takes its bytes
//!   ([`substitute_results`]) -- a variant that names the render's digest
//!   names the image set's. « Revenir au rendu d'origine » ([`revert`])
//!   removes the image set and its record; the render was never changed.
//! - **A render already online is replaced, never doubled**
//!   ([`place_replacements`]): when the render's own bytes are the main image
//!   of a capture online, the image set leaves for that capture with
//!   `placement: "replace"` (`api/uploads.md`, « Choosing where the image
//!   goes »): it takes the place of the main image, no new capture, no
//!   variant. When the render is online as a variant, a device cannot
//!   replace it (only the web session changes a variant): the image set is
//!   set aside and the summary says why ([`crate::precheck::NotSent::AdjustedVariantOnline`]).
//! - **The TIFF of a render is set with it** ([`AdjustState::tiff`]): a
//!   render the stacker wrote twice (`<name>.png` and its full-resolution
//!   twin `<name>.tif`, [`crate::render_files`]) is set from its 16-bit TIFF,
//!   and the script writes both files of the image set from that one result:
//!   `<stem>_adjusted.tif` (16 bits) and `<stem>_adjusted.png`, its exact
//!   8-bit reduction. When the person joins the TIFF (« Joindre aussi les
//!   TIFF »), the twin leaves as the image set's TIFF, never as the render's:
//!   the PNG and the TIFF that leave are always the same image. When the
//!   render's TIFF is already online as a variant of the capture, the image
//!   set's TIFF **takes its place** (`replacesSha256`, `api/uploads.md`, «
//!   Replacing a variant»; [`place_replacements`], [`replace_sent_renders`]):
//!   one TIFF online, never the old one beside the new one. A service that
//!   does not know the field receives it as a variant, as before. A record
//!   written before the TIFF was set (no `adjustedTiff`) for a render that
//!   has a twin is stale: the render and its TIFF leave as they are, until
//!   the image is set again.
//! - A record whose render changed since (stacked again) is stale: it is
//!   ignored, the new render leaves as it is.
//! - [`SavedAdjustment`]: a setting kept under a name to apply to another
//!   image, kept with the application's settings
//!   (`local_settings::LocalSettings::saved_adjustments`) and exportable as a
//!   small JSON document ([`export_saved`], [`import_saved`]). Applying one
//!   only moves the sliders of the preview: nothing is written before
//!   « Enregistrer l'image réglée ». An import never overwrites a name
//!   already taken: it is left out, and the answer names it
//!   ([`ImportOutcome`]).

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::night_online::NightOnline;
use crate::precheck::{Decision, ImageCandidate, ImagePlan, NotSent};
use crate::upload_queue::ResultFile;

/// The folder of the images set, under a target's `stacked/`.
pub const ADJUSTED_DIR: &str = "adjusted";
/// What the name of an image set ends with, before `.png`.
pub const ADJUSTED_SUFFIX: &str = "_adjusted";
/// The version of [`AdjustRecord`] written.
pub const RECORD_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum AdjustError {
    #[error("{0} is not a PNG render Astro Sync made")]
    NotARender(PathBuf),
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid adjustment record at {path}: {source}")]
    Invalid {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("a setting needs a name")]
    EmptyName,
    #[error("a setting named {0:?} already exists")]
    NameTaken(String),
    #[error("no setting is named {0:?}")]
    UnknownName(String),
    #[error("not a file of settings: {0}")]
    NotAnExport(String),
}

impl AdjustError {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            AdjustError::NotARender(_) => "adjust_not_a_render",
            AdjustError::Io { .. } => "io_error",
            AdjustError::Invalid { .. } => "adjust_invalid_record",
            AdjustError::EmptyName => "adjust_empty_name",
            AdjustError::NameTaken(_) => "adjust_name_taken",
            AdjustError::UnknownName(_) => "adjust_unknown_name",
            AdjustError::NotAnExport(_) => "adjust_not_an_export",
        }
    }
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> AdjustError + '_ {
    move |source| AdjustError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// The values; each at its neutral value leaves the render as it is.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AdjustValues {
    /// 0..20 %: a black level on the luminance.
    pub black_point: f64,
    /// -50..50: a power on the luminance, the blacks and the whites kept.
    pub brightness: f64,
    /// 0..3: a factor on the chroma of the signal (1: the render's).
    pub colour: f64,
    /// 0..100 %: NL-means.
    pub smoothing: f64,
    /// -100..100 %: the star layer taken away or added back.
    pub stars: f64,
    /// 0..100 %: diffraction spikes added on the bright stars (0: none). Light
    /// that is not in the person's data.
    pub spikes: f64,
    /// 0..100: which stars get spikes, the brightest only (0) to the medium
    /// ones too (100). Says nothing while `spikes` is 0, and is then 50.
    pub spikes_reach: f64,
}

// The values are always brought into their range ([`AdjustValues::clamped`]):
// never NaN.
impl Eq for AdjustValues {}

impl Default for AdjustValues {
    fn default() -> Self {
        Self::NEUTRAL
    }
}

impl AdjustValues {
    pub const NEUTRAL: Self = Self {
        black_point: 0.0,
        brightness: 0.0,
        colour: 1.0,
        smoothing: 0.0,
        stars: 0.0,
        spikes: 0.0,
        spikes_reach: 50.0,
    };

    /// Each value in its range; a value that is not a number is neutral.
    #[must_use]
    pub fn clamped(self) -> Self {
        let keep = |value: f64, neutral: f64, low: f64, high: f64| {
            if value.is_finite() {
                value.clamp(low, high)
            } else {
                neutral
            }
        };
        let spikes = keep(self.spikes, 0.0, 0.0, 100.0);
        Self {
            black_point: keep(self.black_point, 0.0, 0.0, 20.0),
            brightness: keep(self.brightness, 0.0, -50.0, 50.0),
            colour: keep(self.colour, 1.0, 0.0, 3.0),
            smoothing: keep(self.smoothing, 0.0, 0.0, 100.0),
            stars: keep(self.stars, 0.0, -100.0, 100.0),
            spikes,
            // With no spikes the reach says nothing: two settings that draw
            // the same image are equal.
            spikes_reach: if spikes == 0.0 {
                Self::NEUTRAL.spikes_reach
            } else {
                keep(self.spikes_reach, 50.0, 0.0, 100.0)
            },
        }
    }

    /// Every value at the render's: nothing to write.
    #[must_use]
    pub fn is_neutral(&self) -> bool {
        self.clamped() == Self::NEUTRAL
    }

    /// Whether the star layer is needed (StarNet2): the stars setting reads it,
    /// and the spikes read the list of the stars measured from it.
    #[must_use]
    pub fn needs_stars(&self) -> bool {
        let values = self.clamped();
        values.stars != 0.0 || values.spikes != 0.0
    }
}

/// What « Enregistrer l'image réglée » left beside the render.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdjustRecord {
    pub version: u32,
    pub values: AdjustValues,
    /// The quarter turns, clockwise (0..3), the image set was written with:
    /// its pixels are turned, never the render's. Not one of the five
    /// values: a turn belongs to the image, so the saved settings applied to
    /// other images never carry it. Absent from a record of before (0).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub quarter_turns: u8,
    /// The image set's file name, in the same folder as the record.
    pub adjusted: String,
    pub adjusted_sha256: String,
    pub adjusted_size: u64,
    /// The render as it was: a render stacked again since makes the record
    /// stale.
    pub original_size: u64,
    pub original_modified: u64,
    /// The render's own digest when the image set was written: the image it
    /// replaces when the render is already online. Absent from a record of
    /// before (then read from the render, unchanged since).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_sha256: Option<String>,
    /// The image set's TIFF, when the render has a TIFF twin: its file name
    /// (same folder), digest and size, and the twin as it was (a twin
    /// written again since makes the record stale). Absent from a record of
    /// before, and for a render written as a PNG only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjusted_tiff: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjusted_tiff_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjusted_tiff_size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_tiff_size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_tiff_modified: Option<u64>,
    /// The twin's digest when the image set was written: the variant online
    /// its TIFF replaces. Absent from a record of before (then read from the
    /// twin, unchanged since or the record would be stale).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_tiff_sha256: Option<String>,
}

fn is_zero(value: &u8) -> bool {
    *value == 0
}

/// A number of quarter turns brought into 0..3.
#[must_use]
pub fn clamp_turns(turns: u8) -> u8 {
    turns % 4
}

/// One file of an image set: where it is, its digest and size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdjustedFile {
    pub path: PathBuf,
    pub sha256: String,
    pub size: u64,
}

/// The image set of a render, and the state the window shows on its row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdjustState {
    pub values: AdjustValues,
    /// The quarter turns, clockwise, written into the image set (0 when it
    /// is not turned).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub quarter_turns: u8,
    pub path: PathBuf,
    pub sha256: String,
    pub size: u64,
    /// The image set's TIFF, in place of the render's TIFF twin; `None` for
    /// a render that has no twin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiff: Option<AdjustedFile>,
}

/// A render that can be set: a PNG directly in a `stacked/` folder, the image
/// set itself excluded. The linear file is no image: the window never offers
/// it, and its PNG, when there is one, is refused by the caller's own check.
#[must_use]
pub fn is_render_png(path: &Path) -> bool {
    let png = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("png"));
    let in_stacked = path
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == crate::working_folder::STACKED_DIR);
    png && in_stacked
}

/// `<stacked>/adjusted/`.
fn adjusted_dir(render: &Path) -> Option<PathBuf> {
    Some(render.parent()?.join(ADJUSTED_DIR))
}

/// Where the image set of `render` is written.
#[must_use]
pub fn adjusted_path(render: &Path) -> Option<PathBuf> {
    let stem = render.file_stem()?.to_str()?;
    Some(adjusted_dir(render)?.join(format!("{stem}{ADJUSTED_SUFFIX}.png")))
}

/// The TIFF twin of `render` (`<name>.tif`, else `<name>.tiff`, beside it),
/// when there is one: the full-resolution file of the same render
/// ([`crate::render_files::pair_render_files`]).
#[must_use]
pub fn twin_of(render: &Path) -> Option<PathBuf> {
    ["tif", "tiff"]
        .into_iter()
        .map(|extension| render.with_extension(extension))
        .find(|path| path.is_file())
}

/// Where the image set's TIFF of `render` is written.
#[must_use]
pub fn adjusted_tiff_path(render: &Path) -> Option<PathBuf> {
    adjusted_path(render).map(|png| png.with_extension("tif"))
}

/// The render PNG a TIFF twin belongs to (`<name>.png` beside it), when it is
/// one: a TIFF directly in a `stacked/` folder whose PNG is there too.
fn render_of_twin(tiff: &Path) -> Option<PathBuf> {
    if !crate::render_files::is_tiff(tiff) {
        return None;
    }
    let png = tiff.with_extension("png");
    (is_render_png(&png) && png.is_file()).then_some(png)
}

/// Where the record of `render` is written.
#[must_use]
pub fn record_path(render: &Path) -> Option<PathBuf> {
    let name = render.file_name()?.to_str()?;
    Some(adjusted_dir(render)?.join(format!("{name}.json")))
}

/// Whether `path` is an image set (`stacked/adjusted/<…>_adjusted.png`).
#[must_use]
pub fn is_adjusted_image(path: &Path) -> bool {
    let in_adjusted = path
        .parent()
        .is_some_and(|dir| dir.file_name().is_some_and(|n| n == ADJUSTED_DIR))
        && path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .is_some_and(|n| n == crate::working_folder::STACKED_DIR);
    in_adjusted
        && path
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.ends_with(ADJUSTED_SUFFIX))
}

fn modified_secs(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

/// The SHA-256 of a file, in lowercase hexadecimal.
pub fn file_sha256(path: &Path) -> Result<String, AdjustError> {
    let mut file = fs::File::open(path).map_err(io(path))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer).map_err(io(path))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// The record of `render`, as written; `None` when there is none.
pub fn read_record(render: &Path) -> Result<Option<AdjustRecord>, AdjustError> {
    let Some(path) = record_path(render) else {
        return Ok(None);
    };
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(AdjustError::Io { path, source }),
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|source| AdjustError::Invalid { path, source })
}

/// The image set of `render` when it is valid: its record readable, the
/// image set there with the size recorded, the render unchanged since.
/// Anything else is `None`: the render leaves as it is.
#[must_use]
pub fn state(render: &Path) -> Option<AdjustState> {
    let record = read_record(render).ok()??;
    let original = fs::metadata(render).ok()?;
    if original.len() != record.original_size
        || modified_secs(&original) != record.original_modified
    {
        return None;
    }
    let path = adjusted_dir(render)?.join(&record.adjusted);
    let adjusted = fs::metadata(&path).ok()?;
    if !(adjusted.is_file() && adjusted.len() == record.adjusted_size) {
        return None;
    }
    // A render with a TIFF twin needs the image set's TIFF, made from that
    // twin as it is now: without it the TIFF that would leave is not the
    // image set's.
    let tiff = match twin_of(render) {
        None => None,
        Some(twin) => {
            let twin_meta = fs::metadata(&twin).ok()?;
            if record.original_tiff_size != Some(twin_meta.len())
                || record.original_tiff_modified != Some(modified_secs(&twin_meta))
            {
                return None;
            }
            let tiff_path = adjusted_dir(render)?.join(record.adjusted_tiff.as_deref()?);
            let size = record.adjusted_tiff_size?;
            let meta = fs::metadata(&tiff_path).ok()?;
            if !(meta.is_file() && meta.len() == size) {
                return None;
            }
            Some(AdjustedFile {
                path: tiff_path,
                sha256: record.adjusted_tiff_sha256.clone()?,
                size,
            })
        }
    };
    Some(AdjustState {
        values: record.values.clamped(),
        quarter_turns: clamp_turns(record.quarter_turns),
        path,
        sha256: record.adjusted_sha256,
        size: record.adjusted_size,
        tiff,
    })
}

/// What leaves for `path`: the image set when the render has a valid one.
#[must_use]
pub fn sent_file(path: &Path) -> Option<AdjustState> {
    is_render_png(path).then(|| state(path)).flatten()
}

/// What leaves for the TIFF twin `path` of a render that has a valid image
/// set: the image set's TIFF, and the image set itself (its PNG, the twin's
/// own twin).
#[must_use]
pub fn sent_twin(path: &Path) -> Option<(AdjustedFile, AdjustState)> {
    let set = state(&render_of_twin(path)?)?;
    Some((set.tiff.clone()?, set))
}

/// The record of the image set the script just wrote at
/// [`adjusted_path`]`(render)` (and, for a render with a TIFF twin, at
/// [`adjusted_tiff_path`]`(render)`): their digests and sizes, the render's
/// (and its twin's) size and date, the values. Written beside, then renamed.
/// A render with a twin whose image set has no TIFF is refused: the TIFF
/// that would leave would not be the image set's.
pub fn write_record(render: &Path, values: AdjustValues) -> Result<AdjustState, AdjustError> {
    write_record_turned(render, values, 0)
}

/// [`write_record`] for an image set the script wrote turned by
/// `quarter_turns` quarter turns clockwise.
pub fn write_record_turned(
    render: &Path,
    values: AdjustValues,
    quarter_turns: u8,
) -> Result<AdjustState, AdjustError> {
    let quarter_turns = clamp_turns(quarter_turns);
    if !is_render_png(render) {
        return Err(AdjustError::NotARender(render.to_path_buf()));
    }
    let adjusted =
        adjusted_path(render).ok_or_else(|| AdjustError::NotARender(render.to_path_buf()))?;
    let record_file =
        record_path(render).ok_or_else(|| AdjustError::NotARender(render.to_path_buf()))?;
    let original = fs::metadata(render).map_err(io(render))?;
    let size = fs::metadata(&adjusted).map_err(io(&adjusted))?.len();
    let sha256 = file_sha256(&adjusted)?;
    let twin = match twin_of(render) {
        None => None,
        Some(twin) => {
            let twin_meta = fs::metadata(&twin).map_err(io(&twin))?;
            let twin_sha256 = file_sha256(&twin)?;
            let tiff = adjusted_tiff_path(render)
                .ok_or_else(|| AdjustError::NotARender(render.to_path_buf()))?;
            let tiff_size = fs::metadata(&tiff).map_err(io(&tiff))?.len();
            Some((
                (twin_meta, twin_sha256),
                AdjustedFile {
                    sha256: file_sha256(&tiff)?,
                    size: tiff_size,
                    path: tiff,
                },
            ))
        }
    };
    let record = AdjustRecord {
        version: RECORD_VERSION,
        values: values.clamped(),
        quarter_turns,
        adjusted: adjusted
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string(),
        adjusted_sha256: sha256.clone(),
        adjusted_size: size,
        original_size: original.len(),
        original_modified: modified_secs(&original),
        original_sha256: Some(file_sha256(render)?),
        adjusted_tiff: twin.as_ref().map(|(_, tiff)| {
            tiff.path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string()
        }),
        adjusted_tiff_sha256: twin.as_ref().map(|(_, tiff)| tiff.sha256.clone()),
        adjusted_tiff_size: twin.as_ref().map(|(_, tiff)| tiff.size),
        original_tiff_size: twin.as_ref().map(|((meta, _), _)| meta.len()),
        original_tiff_modified: twin.as_ref().map(|((meta, _), _)| modified_secs(meta)),
        original_tiff_sha256: twin.as_ref().map(|((_, sha256), _)| sha256.clone()),
    };
    let part = record_file.with_extension("json.part");
    let text = serde_json::to_string_pretty(&record).unwrap_or_default();
    fs::write(&part, text).map_err(io(&part))?;
    fs::rename(&part, &record_file).map_err(io(&record_file))?;
    Ok(AdjustState {
        values: record.values,
        quarter_turns,
        path: adjusted,
        sha256,
        size,
        tiff: twin.map(|(_, tiff)| tiff),
    })
}

/// « Revenir au rendu d'origine »: the image set (its PNG and its TIFF) and its
/// record removed; the render and its TIFF, never changed, leave again.
/// Nothing there is not an error.
pub fn revert(render: &Path) -> Result<(), AdjustError> {
    if !is_render_png(render) {
        return Err(AdjustError::NotARender(render.to_path_buf()));
    }
    for path in [
        adjusted_path(render),
        adjusted_tiff_path(render),
        record_path(render),
    ]
    .into_iter()
    .flatten()
    {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(AdjustError::Io { path, source }),
        }
    }
    Ok(())
}

/// The candidates of a night's check with each render that has an image set
/// counted as the image set (its digest and size; its path stays the
/// render's, which the proposals of the sending are read from), and the
/// render's TIFF twin counted as the image set's TIFF. The two files of the
/// image set name each other as twins; the render's own TIFF is never the
/// image set's twin: online, it does not make the image set online.
#[must_use]
pub fn substitute_candidates(candidates: Vec<ImageCandidate>) -> Vec<ImageCandidate> {
    candidates
        .into_iter()
        .map(|mut candidate| {
            if let Some(set) = sent_file(&candidate.path) {
                candidate.sha256 = set.sha256;
                candidate.size = set.size;
                candidate.twin_sha256 = set.tiff.map(|tiff| tiff.sha256);
            } else if candidate.twin_sha256.is_some()
                && let Some((tiff, set)) = sent_twin(&candidate.path)
            {
                candidate.sha256 = tiff.sha256;
                candidate.size = tiff.size;
                candidate.twin_sha256 = Some(set.sha256);
            }
            candidate
        })
        .collect()
}

/// The files of a sending with each render that has an image set replaced by
/// it (path, digest, size), the render's TIFF twin by the image set's TIFF,
/// and every variant that names the render's digest naming the image set's.
#[must_use]
pub fn substitute_results(files: &[ResultFile]) -> Vec<ResultFile> {
    let mut renamed: HashMap<String, String> = HashMap::new();
    let sent = |file: &ResultFile| -> Option<AdjustedFile> {
        match sent_file(&file.path) {
            Some(set) => Some(AdjustedFile {
                path: set.path,
                sha256: set.sha256,
                size: set.size,
            }),
            None => sent_twin(&file.path).map(|(tiff, _)| tiff),
        }
    };
    let mut out: Vec<ResultFile> = files
        .iter()
        .map(|file| match sent(file) {
            Some(set) => {
                if !file.sha256.eq_ignore_ascii_case(&set.sha256) {
                    renamed.insert(file.sha256.to_ascii_lowercase(), set.sha256.clone());
                }
                ResultFile {
                    path: set.path,
                    sha256: set.sha256,
                    size: set.size,
                    ..file.clone()
                }
            }
            None => file.clone(),
        })
        .collect();
    if !renamed.is_empty() {
        let prefix = crate::uploader::VARIANT_OF_SHA256;
        for file in &mut out {
            let Some(of) = file.target.variant_of.as_deref() else {
                continue;
            };
            if let Some(digest) = of.strip_prefix(prefix)
                && let Some(new) = renamed.get(&digest.to_ascii_lowercase())
            {
                file.target.variant_of = Some(format!("{prefix}{new}"));
            }
        }
    }
    out
}

/// The digest of the render an image set was made from, when the image set
/// is valid ([`state`]): the one recorded, else the render's own (unchanged
/// since, or the record would be stale).
#[must_use]
pub fn original_sha256(render: &Path) -> Option<String> {
    state(render)?;
    match read_record(render).ok()??.original_sha256 {
        Some(digest) => Some(digest),
        None => file_sha256(render).ok(),
    }
}

/// The value of `placement` that takes the place of a capture's main image
/// (`capture.schema.json`, `uploadMetadata.placement`).
pub const PLACEMENT_REPLACE: &str = "replace";

/// After [`crate::precheck::plan_images_within`]: an image set whose render
/// is already online takes its place instead of leaving beside it. The
/// render online as the **main image** of a capture: the image set leaves
/// for that capture as [`Decision::Replace`] (`placement: "replace"`: no new
/// capture, no variant, whatever `capture.variant_limit`). The render online
/// as a **variant**: a device cannot replace a variant, so the image set is
/// set aside ([`NotSent::AdjustedVariantOnline`]) rather than doubled. An
/// image set already online, or whose render never left, is untouched.
///
/// The image set's **TIFF**, leaving as a variant of a capture that holds the
/// render's TIFF as a variant, takes its place there (`replaces_sha256`):
/// one TIFF online, the old one released.
pub fn place_replacements(online: &NightOnline, plans: &mut [ImagePlan]) {
    for plan in plans.iter_mut() {
        if let Decision::Variant {
            capture_id,
            replaces_sha256,
        } = &mut plan.decision
            && replaces_sha256.is_none()
            && let Some(old) = replaced_tiff(&plan.image.path, &plan.image.sha256)
            && let Some((capture, image)) = online.image_with(&old)
            && capture.id == *capture_id
            && !image.main
        {
            *replaces_sha256 = Some(old);
            continue;
        }
        if matches!(
            plan.decision,
            Decision::AlreadyOnline { .. } | Decision::FileOnline { .. }
        ) {
            continue;
        }
        let Some(set) = sent_file(&plan.image.path) else {
            continue;
        };
        if !set.sha256.eq_ignore_ascii_case(&plan.image.sha256) {
            continue;
        }
        let Some(original) = original_sha256(&plan.image.path) else {
            continue;
        };
        let Some((capture, image)) = online.image_with(&original) else {
            continue;
        };
        plan.decision = if image.main {
            Decision::Replace {
                capture_id: capture.id.clone(),
                image_id: image.id.clone(),
            }
        } else {
            Decision::NotSent {
                reason: NotSent::AdjustedVariantOnline,
            }
        };
    }
}

/// Before the queue takes an image set: when the render it was made from
/// already left through this queue as the main image of a capture
/// (`sent`: the capture of the render's digest, and whether it was sent as
/// that capture's main image), the image set leaves for that capture in its
/// place (`placement: "replace"`). A file that already names its capture is
/// left as it is (the night's check decided). The image set's TIFF, when the
/// render's TIFF already left through this queue as a **variant** of a
/// capture, takes its place there (`replaces_sha256`), unless the night's
/// check already decided where it goes.
pub fn replace_sent_renders(
    files: &mut [ResultFile],
    sent: impl Fn(&str) -> Option<(String, bool)>,
) {
    for file in files.iter_mut() {
        if file.target.replaces_sha256.is_some() || file.target.placement.is_some() {
            continue;
        }
        let Some(render) = adjusted_tiff_render_of(&file.path) else {
            continue;
        };
        // Only a TIFF the window or the queue attached to its PNG: one the
        // check sent to a capture online is left as the check decided.
        if file
            .target
            .variant_of
            .as_deref()
            .is_some_and(|of| !of.starts_with(crate::uploader::VARIANT_OF_SHA256))
        {
            continue;
        }
        let Some(original) = original_tiff_sha256(&render) else {
            continue;
        };
        if let Some((capture_id, false)) = sent(&original) {
            file.target.variant_of = Some(capture_id);
            file.target.replaces_sha256 = Some(original);
        }
    }
    for file in files.iter_mut() {
        if file.target.variant_of.is_some() || file.target.placement.is_some() {
            continue;
        }
        let Some(render) = adjusted_render_of(&file.path) else {
            continue;
        };
        let Some(original) = original_sha256(&render) else {
            continue;
        };
        if let Some((capture_id, true)) = sent(&original) {
            file.target.variant_of = Some(capture_id);
            file.target.placement = Some(PLACEMENT_REPLACE.to_string());
        }
    }
}

/// The digest of the render's TIFF twin that the image set's TIFF leaving for
/// `path` (`sha256`) replaces online: `path` is the twin of a render whose
/// image set is valid and has a TIFF, and `sha256` that TIFF's (the check
/// counts the image set's TIFF on the twin's path). `None` for anything else.
fn replaced_tiff(path: &Path, sha256: &str) -> Option<String> {
    let (tiff, _) = sent_twin(path)?;
    if !tiff.sha256.eq_ignore_ascii_case(sha256) {
        return None;
    }
    original_tiff_sha256(&render_of_twin(path)?)
}

/// The digest of the render's TIFF twin an image set's TIFF was made from,
/// when the image set is valid and has one: the one recorded, else the
/// twin's own (unchanged since, or the record would be stale).
#[must_use]
pub fn original_tiff_sha256(render: &Path) -> Option<String> {
    state(render)?.tiff?;
    match read_record(render).ok()??.original_tiff_sha256 {
        Some(digest) => Some(digest),
        None => file_sha256(&twin_of(render)?).ok(),
    }
}

/// The render an image set's TIFF (`stacked/adjusted/<stem>_adjusted.tif`)
/// was made from, when its record names it.
fn adjusted_tiff_render_of(path: &Path) -> Option<PathBuf> {
    if !crate::render_files::is_tiff(path) {
        return None;
    }
    let in_adjusted = path
        .parent()
        .is_some_and(|dir| dir.file_name().is_some_and(|n| n == ADJUSTED_DIR));
    if !in_adjusted {
        return None;
    }
    let stem = path.file_stem()?.to_str()?.strip_suffix(ADJUSTED_SUFFIX)?;
    let render = path.parent()?.parent()?.join(format!("{stem}.png"));
    state(&render)
        .filter(|set| set.tiff.as_ref().is_some_and(|tiff| tiff.path == path))
        .map(|_| render)
}

/// The render an image set (`stacked/adjusted/<stem>_adjusted.png`) was made
/// from, when its record names it.
fn adjusted_render_of(path: &Path) -> Option<PathBuf> {
    if !is_adjusted_image(path) {
        return None;
    }
    let stem = path.file_stem()?.to_str()?.strip_suffix(ADJUSTED_SUFFIX)?;
    let render = path.parent()?.parent()?.join(format!("{stem}.png"));
    state(&render)
        .filter(|set| set.path == path)
        .map(|_| render)
}

/* ------------------------------ Saved settings ---------------------------- */

/// The longest name of a saved setting, in characters.
pub const NAME_MAX_CHARS: usize = 60;
/// The `format` of an exported file of settings.
pub const EXPORT_FORMAT: &str = "astro-sync-image-adjustments";

/// A setting kept under a name, to apply to another image (« enregistrer un
/// réglage »): five named values, nothing of an image nor of a device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedAdjustment {
    pub name: String,
    pub values: AdjustValues,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportFile {
    format: String,
    version: u32,
    adjustments: Vec<SavedAdjustment>,
}

fn clean_name(name: &str) -> Result<String, AdjustError> {
    let name: String = name.trim().chars().take(NAME_MAX_CHARS).collect();
    let name = name.trim_end().to_string();
    if name.is_empty() {
        return Err(AdjustError::EmptyName);
    }
    Ok(name)
}

fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// `values` kept under `name` (trimmed, at most [`NAME_MAX_CHARS`]); a name
/// already taken, whatever its case, is refused.
pub fn save_named(
    list: &mut Vec<SavedAdjustment>,
    name: &str,
    values: AdjustValues,
) -> Result<(), AdjustError> {
    let name = clean_name(name)?;
    if list.iter().any(|item| same_name(&item.name, &name)) {
        return Err(AdjustError::NameTaken(name));
    }
    list.push(SavedAdjustment {
        name,
        values: values.clamped(),
    });
    Ok(())
}

/// The setting `from` renamed `to`; its values unchanged.
pub fn rename_named(list: &mut [SavedAdjustment], from: &str, to: &str) -> Result<(), AdjustError> {
    let to = clean_name(to)?;
    if list
        .iter()
        .any(|item| same_name(&item.name, &to) && !same_name(&item.name, from))
    {
        return Err(AdjustError::NameTaken(to));
    }
    let item = list
        .iter_mut()
        .find(|item| same_name(&item.name, from))
        .ok_or_else(|| AdjustError::UnknownName(from.to_string()))?;
    item.name = to;
    Ok(())
}

/// The setting `name` removed.
pub fn remove_named(list: &mut Vec<SavedAdjustment>, name: &str) -> Result<(), AdjustError> {
    let before = list.len();
    list.retain(|item| !same_name(&item.name, name));
    if list.len() == before {
        return Err(AdjustError::UnknownName(name.to_string()));
    }
    Ok(())
}

/// The settings as a file to keep or to give: a small JSON document.
#[must_use]
pub fn export_saved(list: &[SavedAdjustment]) -> String {
    serde_json::to_string_pretty(&ExportFile {
        format: EXPORT_FORMAT.to_string(),
        version: RECORD_VERSION,
        adjustments: list.to_vec(),
    })
    .unwrap_or_default()
}

/// What an import did: the names added, and those left out because the name
/// was already taken (never overwritten) or empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOutcome {
    pub added: Vec<String>,
    pub skipped: Vec<String>,
}

/// The settings of an exported file, each brought into its range; added to
/// `list` by [`save_named`] (a name already there is left out, never
/// overwritten, and named in [`ImportOutcome::skipped`]).
pub fn import_saved(
    list: &mut Vec<SavedAdjustment>,
    text: &str,
) -> Result<ImportOutcome, AdjustError> {
    let file: ExportFile =
        serde_json::from_str(text).map_err(|error| AdjustError::NotAnExport(error.to_string()))?;
    if file.format != EXPORT_FORMAT {
        return Err(AdjustError::NotAnExport(file.format));
    }
    let mut outcome = ImportOutcome::default();
    for item in file.adjustments {
        match save_named(list, &item.name, item.values) {
            Ok(()) => outcome
                .added
                .push(list.last().map(|s| s.name.clone()).unwrap_or_default()),
            Err(_) => outcome.skipped.push(item.name.trim().to_string()),
        }
    }
    Ok(outcome)
}

/// [`export_saved`] written to `path` (beside, then renamed).
pub fn export_saved_to(path: &Path, list: &[SavedAdjustment]) -> Result<(), AdjustError> {
    let part = path.with_extension("json.part");
    fs::write(&part, export_saved(list)).map_err(io(&part))?;
    fs::rename(&part, path).map_err(io(path))
}

/// [`import_saved`] from the file at `path`.
pub fn import_saved_from(
    list: &mut Vec<SavedAdjustment>,
    path: &Path,
) -> Result<ImportOutcome, AdjustError> {
    let text = fs::read_to_string(path).map_err(io(path))?;
    import_saved(list, &text)
}

#[cfg(test)]
mod tests;

//! The capture metadata Astro Sync proposes (the service's contract is
//! `capture.schema.json` `uploadMetadata`, `manage-captures` `proposed`,
//! `capture-variant` `PATCH /v1/captures/{id}/variants/{imageId}`;
//! `api/v1.md`, « The metadata of a capture»).
//!
//! Astro Sync is a **producer**: everything here is a *proposal* under
//! `metadataSource: "astro-sync"`. The service decides: a value the owner
//! typed on the portal is never overwritten, and the answer says what it left
//! (`skipped`, [`skipped_sentence`]). Nothing is proposed that a device may
//! not write -- the notes, the equipment, the night, the place, a capture's
//! title or description are the owner's (`403 web_session_required` for a
//! device token); this module has no type that could carry them.
//!
//! What is read, and from where:
//!
//! - **acquisition** (lines per filter, exposure and gain: frames kept,
//!   `EXPTIME`, `GAIN`) and **sensor** (`CCD-TEMP`, `XBINNING`) from the
//!   headers of the target's light frames
//!   ([`crate::progression_images::target_families`], the families of
//!   `analysis::fits`);
//! - **calibration** from the counts of the night's `calibration/*` folders
//!   ([`crate::detection::CalibrationCounts`]): « 30 darks · 30 flats »;
//! - **processing** and the **title and description of the image** from what
//!   the stacking wrote next to a render (`<name>_stacking.json`: Siril's
//!   version, the frames used, the pipeline of each file) and the pipeline's
//!   `name` and `description`, in the ten languages of the service (the
//!   image's generated texts, kept apart from the title its owner types).
//!
//! Never sent: a FITS, a raw log, a position, a date of exposure, the name of
//! a folder. Only numbers and short words leave.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use crate::detection::{CalibrationCounts, NightDetection};
use crate::night_online::NightOnline;
use crate::pipelines::{self, LINEAR_PIPELINE, PipelineList};
use crate::precheck::{Decision, ImagePlan};
use crate::progression_images;
use crate::tools;
use crate::upload_queue::ResultKind;

/// The producer the service records (`metadataSource`, `source`).
pub const SOURCE: &str = "astro-sync";

/// The contract's limits (`capture.schema.json`).
pub const MAX_ACQUISITION_LINES: usize = 40;
pub const MAX_FILTER: usize = 40;
pub const MAX_CALIBRATION: usize = 120;
pub const MAX_SOFTWARE: usize = 120;
pub const MAX_STEPS: usize = 500;
pub const MAX_VARIANT_TITLE: usize = 90;
pub const MAX_VARIANT_DESCRIPTION: usize = 280;
const MAX_FRAME_COUNT: usize = 100_000;
const MAX_EXPOSURE_SECONDS: f64 = 86_400.0;

/// One acquisition line: a filter, a count, an exposure, a gain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcquisitionLine {
    pub filter: Option<String>,
    pub frame_count: u32,
    pub exposure_seconds: f64,
    pub gain: Option<i64>,
}

/// `CCD-TEMP` and `XBINNING`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SensorProposal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature_c: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binning: Option<u8>,
}

/// The program that made the image, and the steps (technical words, the same
/// in every language).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingProposal {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub software: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<String>,
}

/// The program that measured an image (`imageQuality.tool`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityTool {
    pub name: String,
    pub version: String,
}

/// The quality of a stacked image, as Astro Sync measured it on the stars of
/// the **linear** master before any render (`capture.schema.json`
/// `$defs. imageQuality`). Never measured on an image processed elsewhere:
/// the figures are read on a **linear** master by
/// `stacking/star_quality.py` -- the one the stacker of Astro Sync
/// makes, or the one a person hands over (`master_quality`), refused when its
/// histogram says it was stretched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageQuality {
    /// The median FWHM of the stars, in pixels of the master.
    pub fwhm_pixels: f64,
    /// The same in arc seconds; `null` when the pixel scale is not known.
    pub fwhm_arcsec: Option<f64>,
    /// The median eccentricity (0 round, towards 1 trailed); `null` when not
    /// measured.
    pub eccentricity: Option<f64>,
    /// The stars the figures rest on.
    pub star_count: u64,
    /// Always `linear`.
    pub measured_on: String,
    pub measured_at: String,
    pub tool: QualityTool,
}

/// What `capture.schema.json` allows an `imageQuality` to hold.
const MAX_FWHM: f64 = 1000.0;
const MAX_STAR_COUNT: u64 = 100_000_000;
const MEASURED_ON_LINEAR: &str = "linear";

impl ImageQuality {
    /// The quality of a measure the stacker wrote, with the pixel scale of the
    /// frames (`arcsec_per_pixel`): `None` for a figure the contract would
    /// refuse (a width at 0 or beyond 1000 px, an eccentricity outside 0..1,
    /// a measure that is not on the linear master) -- no measure, no field.
    #[must_use]
    pub fn from_measure(measure: &MeasuredQuality, arcsec_per_pixel: Option<f64>) -> Option<Self> {
        let finite_positive = |v: f64| v.is_finite() && v > 0.0 && v <= MAX_FWHM;
        if !finite_positive(measure.fwhm_pixels)
            || measure.measured_on.as_deref() != Some(MEASURED_ON_LINEAR)
            || measure.star_count > MAX_STAR_COUNT
        {
            return None;
        }
        let fwhm_arcsec = arcsec_per_pixel
            .map(|scale| measure.fwhm_pixels * scale)
            .filter(|v| finite_positive(*v))
            .map(|v| (v * 100.0).round() / 100.0)
            .filter(|v| *v > 0.0);
        let eccentricity = measure
            .eccentricity
            .filter(|e| e.is_finite() && (0.0..=1.0).contains(e));
        let fwhm_pixels = (measure.fwhm_pixels * 100.0).round() / 100.0;
        if fwhm_pixels <= 0.0 {
            return None;
        }
        Some(Self {
            fwhm_pixels,
            fwhm_arcsec,
            eccentricity: eccentricity.map(|e| (e * 100.0).round() / 100.0),
            star_count: measure.star_count,
            measured_on: MEASURED_ON_LINEAR.to_string(),
            measured_at: measure
                .measured_at
                .clone()
                .unwrap_or_else(crate::time_util::now_iso8601_utc),
            tool: QualityTool {
                name: "Astro Sync".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        })
    }
}

/// The `quality` block the stacker writes (`<name>_stacking.json`,
/// `<name>_plan.json`): what `star_quality.py` measured on the linear master.
/// Also what is kept of a master measured by hand ([`crate::master_quality`]).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasuredQuality {
    #[serde(default)]
    pub fwhm_pixels: f64,
    #[serde(default)]
    pub eccentricity: Option<f64>,
    #[serde(default)]
    pub star_count: u64,
    #[serde(default)]
    pub measured_on: Option<String>,
    #[serde(default)]
    pub measured_at: Option<String>,
}

/// Everything Astro Sync proposes for an image and its capture. Its fields
/// are the proposal fields of `uploadMetadata`, so it is flattened into the
/// create step as it is; empty fields are not written.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposals {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acquisition: Vec<AcquisitionLine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calibration: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensor: Option<SensorProposal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processing: Option<ProcessingProposal>,
    /// What made *this* image (its software and steps, the finish it carries
    /// included): the image's own, kept with it by the service. The capture
    /// shows the steps of its main image, so they follow it when another image
    /// becomes the main one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant_processing: Option<ProcessingProposal>,
    /// The rendering's name and its settings, in the ten languages
    /// ([`generated_render_texts`]): the image's generated title, kept by
    /// the service apart from the title its owner types.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant_generated_title: Option<GeneratedText>,
    /// The rendering's description, in the ten languages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant_generated_description: Option<GeneratedText>,
    /// The quality of the stacked image, measured on its linear master --
    /// the image's own, not the capture's; only under the capability
    /// `analysis.logs` (the service drops it without).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_quality: Option<ImageQuality>,
}

// Every number is finite (read from a header, filtered below): the equality
// is total.
impl Eq for Proposals {}

impl Proposals {
    /// Whether there is nothing to propose.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// What concerns the capture (acquisition, calibration, sensor,
    /// processing): `PATCH /v1/captures/{id}` `proposed`.
    #[must_use]
    pub fn capture_part(&self) -> Self {
        Self {
            variant_processing: None,
            variant_generated_title: None,
            variant_generated_description: None,
            image_quality: None,
            ..self.clone()
        }
    }

    /// What concerns the image sent (its title, its description and its quality).
    #[must_use]
    pub fn image_part(&self) -> Self {
        Self {
            variant_processing: self.variant_processing.clone(),
            variant_generated_title: self.variant_generated_title.clone(),
            variant_generated_description: self.variant_generated_description.clone(),
            image_quality: self.image_quality.clone(),
            ..Self::default()
        }
    }

    /// Whether the capture has something proposed.
    #[must_use]
    pub fn has_capture_part(&self) -> bool {
        !self.capture_part().is_empty()
    }

    /// Whether the image has a title, a description, a quality or its own steps proposed.
    #[must_use]
    pub fn has_image_part(&self) -> bool {
        !self.image_part().is_empty()
    }

    /// Without the processing: a variant's rendering is not the capture's.
    #[must_use]
    pub fn without_processing(&self) -> Self {
        Self {
            processing: None,
            ..self.clone()
        }
    }
}

/// What the service left of the proposals (`skipped: [{ field, reason }]`),
/// `reason` being `edited` (the owner's value), `deleted` (a line the owner
/// deleted) or `capability` (a proposal the space has no right to keep).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedField {
    pub field: String,
    pub reason: String,
}

/// The `capability` reason: the space has no right to keep the field
/// (`imageQuality` without `analysis.logs`). The image is sent all the same
/// and nothing is said of it: not a value the owner typed, nothing alarming.
pub const SKIPPED_FOR_CAPABILITY: &str = "capability";

/// What the person is told the service left: the owner's values and the
/// lines they deleted, never a proposal that a missing right dropped.
#[must_use]
pub fn visible_skipped(skipped: &[SkippedField]) -> Vec<SkippedField> {
    skipped
        .iter()
        .filter(|s| s.reason != SKIPPED_FOR_CAPABILITY)
        .cloned()
        .collect()
}

/// The sentence that says it, in plain words (`None` when nothing
/// was left). The window has the same sentences in its dictionaries.
#[must_use]
pub fn skipped_sentence(skipped: &[SkippedField]) -> Option<String> {
    let skipped = visible_skipped(skipped);
    if skipped.is_empty() {
        return None;
    }
    let edited = skipped.iter().any(|s| s.reason != "deleted");
    let deleted = skipped.iter().any(|s| s.reason == "deleted");
    let mut parts = Vec::new();
    if edited {
        parts.push("The values you entered in your Astro Portal space are kept.");
    }
    if deleted {
        parts.push("The rows you deleted in your Astro Portal space are not put back.");
    }
    Some(parts.join(" "))
}

/* ------------------------------ Bounds ---------------------------------- */

fn clip(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(limit.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

fn non_empty(text: String) -> Option<String> {
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

/* --------------------------- Acquisition, sensor ------------------------- */

/// The lines of the target's lights: one per filter, exposure and gain (the
/// families of the headers merged over temperature and binning), the most
/// numerous first. Lines outside the contract (no exposure, an exposure over
/// a day, a count over 100 000) are left out; 40 at most.
#[must_use]
pub fn acquisition_lines(
    families: &[crate::analysis::fits::AcquisitionFamily],
) -> Vec<AcquisitionLine> {
    let mut lines: Vec<AcquisitionLine> = Vec::new();
    for family in families {
        let Some(exposure) = family
            .exposure_seconds
            .filter(|e| e.is_finite() && *e > 0.0 && *e <= MAX_EXPOSURE_SECONDS)
        else {
            continue;
        };
        let frames = family.frame_paths.len();
        if frames == 0 || frames > MAX_FRAME_COUNT {
            continue;
        }
        let filter = family
            .filter
            .as_deref()
            .map(|f| clip(f, MAX_FILTER))
            .and_then(non_empty);
        let gain = family
            .gain
            .filter(|g| g.is_finite())
            .map(|g| g.round() as i64);
        match lines.iter_mut().find(|l| {
            l.filter == filter && (l.exposure_seconds - exposure).abs() < 1e-3 && l.gain == gain
        }) {
            Some(line) => line.frame_count += frames as u32,
            None => lines.push(AcquisitionLine {
                filter,
                frame_count: frames as u32,
                exposure_seconds: exposure,
                gain,
            }),
        }
    }
    lines.sort_by(|a, b| {
        b.frame_count
            .cmp(&a.frame_count)
            .then_with(|| a.filter.cmp(&b.filter))
    });
    lines.truncate(MAX_ACQUISITION_LINES);
    lines
}

/// The sensor of the most numerous family: its temperature in whole degrees
/// and its binning (`1x1` → 1).
#[must_use]
pub fn sensor_of(families: &[crate::analysis::fits::AcquisitionFamily]) -> Option<SensorProposal> {
    let family = families.iter().max_by_key(|f| f.frame_paths.len())?;
    let temperature_c = family
        .temperature_class_c
        .map(|t| t as f64)
        .filter(|t| (-100.0..=60.0).contains(t));
    let binning = family
        .binning
        .as_deref()
        .and_then(|b| b.split(['x', 'X']).next())
        .and_then(|n| n.trim().parse::<u8>().ok())
        .filter(|n| (1..=8).contains(n));
    (temperature_c.is_some() || binning.is_some()).then_some(SensorProposal {
        temperature_c,
        binning,
    })
}

/* ------------------------------ Calibration ------------------------------ */

/// « 30 darks · 30 flats · 30 dark-flats · 10 biases »: the counts of the
/// night's calibration folders, the same words in every language (like the
/// folders' names). `None` when there is none.
#[must_use]
pub fn calibration_text(counts: &CalibrationCounts) -> Option<String> {
    let part = |count: usize, one: &str, many: &str| {
        (count > 0).then(|| format!("{count} {}", if count == 1 { one } else { many }))
    };
    let parts: Vec<String> = [
        part(counts.darks, "dark", "darks"),
        part(counts.flats, "flat", "flats"),
        part(counts.dark_flats, "dark-flat", "dark-flats"),
        part(counts.biases, "bias", "biases"),
    ]
    .into_iter()
    .flatten()
    .collect();
    non_empty(parts.join(" · ")).map(|text| clip(&text, MAX_CALIBRATION))
}

/* ------------------------------- Stacking -------------------------------- */

/// `<name>_stacking.json`, what the stacking script wrote beside its renders
/// (`astro_sync_stack.py`, `provenance`): only the fields read here.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Provenance {
    #[serde(default)]
    frames: BTreeMap<String, u64>,
    #[serde(default)]
    stacked_frames: u64,
    #[serde(default)]
    tools: ProvenanceTools,
    #[serde(default)]
    outputs: Vec<ProvenanceOutput>,
    /// The quality the stacker read on the linear master; absent from a
    /// provenance written before, `null` when it could not be measured.
    #[serde(default)]
    quality: Option<MeasuredQuality>,
    /// The master kept of the family stacked (`integration_master.py`): its
    /// label tells which of the target's families the image is.
    #[serde(default)]
    integration: Option<ProvenanceIntegration>,
    /// The image a device stacked, when the renders started from
    /// it: Astro Sync stacked no pose.
    #[serde(default)]
    device_master: Option<ProvenanceDevice>,
}

#[derive(Debug, Default, Deserialize)]
struct ProvenanceDevice {
    /// « Seestar S50 Pro ».
    #[serde(default)]
    device: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ProvenanceIntegration {
    #[serde(default)]
    family: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ProvenanceTools {
    #[serde(default)]
    siril: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ProvenanceOutput {
    #[serde(default)]
    pipeline: String,
    #[serde(default)]
    file: String,
    /// The contrast of presentation the image carries; absent for
    /// a render and from a provenance written before.
    #[serde(default)]
    contrast: crate::stacking::Contrast,
    /// The colour intensity the image carries; absent for a render.
    #[serde(default)]
    colour: crate::stacking::Colour,
    /// The softened stars the image carries; absent for a render.
    #[serde(default, rename = "softStars")]
    soft_stars: crate::stacking::SoftStars,
}

/// The `_stacking.json` of the folder of `image` that lists it among its
/// outputs.
fn provenance_of(image: &Path) -> Option<(Provenance, String)> {
    let file_name = image.file_name()?.to_string_lossy().into_owned();
    let folder = image.parent()?;
    for entry in std::fs::read_dir(folder).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with("_stacking.json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Ok(provenance) = serde_json::from_str::<Provenance>(&text) else {
            continue;
        };
        if let Some(output) = provenance.outputs.iter().find(|o| o.file == file_name) {
            let pipeline = output.pipeline.clone();
            return Some((provenance, pipeline));
        }
    }
    None
}

/// The pipeline the stacker recorded for `image` (its `_stacking.json` lists
/// it among its outputs): `raw`, `dualband-correction`, `linear`…; `None` for an
/// image Astro Sync did not stack.
#[must_use]
pub fn recorded_pipeline(image: &Path) -> Option<String> {
    provenance_of(image).map(|(_, pipeline)| pipeline)
}

/// The place of the pipeline of `image` among the pipelines its stacking was
/// asked for, the first asked being 0 (the order of the outputs the stacker
/// recorded in its `_stacking.json`); `None` for an image Astro Sync did not
/// stack. "No processing" is written last, so it never ranks first.
#[must_use]
pub fn recorded_render_rank(image: &Path) -> Option<u32> {
    let (provenance, pipeline) = provenance_of(image)?;
    let mut asked: Vec<&str> = Vec::new();
    for output in &provenance.outputs {
        if !asked.contains(&output.pipeline.as_str()) {
            asked.push(&output.pipeline);
        }
    }
    asked
        .iter()
        .position(|id| *id == pipeline)
        .and_then(|place| u32::try_from(place).ok())
}

/// Whether the renders of `image` started from the image a device stacked
/// : its `_stacking.json` says so (`deviceMaster`).
#[must_use]
pub fn recorded_device_master(image: &Path) -> bool {
    provenance_of(image).is_some_and(|(provenance, _)| provenance.device_master.is_some())
}

/// The contrast of presentation the stacker recorded for `image`:
/// `None` for a render, an image Astro Sync did not stack, or a record
/// written before the option.
#[must_use]
pub fn recorded_contrast(image: &Path) -> crate::stacking::Contrast {
    let Some(name) = image.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return crate::stacking::Contrast::None;
    };
    provenance_of(image)
        .and_then(|(provenance, _)| {
            provenance
                .outputs
                .iter()
                .find(|o| o.file == name)
                .map(|o| o.contrast)
        })
        .unwrap_or_default()
}

/// The colour intensity and the softened stars the stacker recorded for
/// `image`: none for a render, an image Astro Sync did not stack, or a record
/// written before the options.
#[must_use]
pub fn recorded_retouch(image: &Path) -> crate::stacking::Retouch {
    let Some(name) = image.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return crate::stacking::Retouch::default();
    };
    provenance_of(image)
        .and_then(|(provenance, _)| {
            provenance
                .outputs
                .iter()
                .find(|o| o.file == name)
                .map(|o| crate::stacking::Retouch {
                    colour: o.colour,
                    soft_stars: o.soft_stars,
                })
        })
        .unwrap_or_default()
}

/* ------------------------------ Image quality ---------------------------- */

/// The pixel scale, in arc seconds, of the frames of the family a stacked
/// image is made of: the header of its first frame (the WCS, a scale
/// keyword, else the pixel size over the focal length --
/// [`crate::analysis::sky::sensor_pixel_scale`]). The stacker resamples
/// nothing (the registration only crops to the common frame), so the master
/// has the frames' scale. `family` is the label the stacker recorded
/// (`ProvenanceIntegration`); without it, the families of the target must
/// agree on one scale (within 1 %), else it is unknown.
pub(crate) fn frames_pixel_scale(
    families: &[crate::analysis::fits::AcquisitionFamily],
    family: Option<&str>,
) -> Option<f64> {
    let scale_of = |one: &crate::analysis::fits::AcquisitionFamily| {
        one.frame_paths.iter().find_map(|frame| {
            let path = Path::new(frame);
            let header = match path.extension().and_then(|e| e.to_str()) {
                Some(e) if e.eq_ignore_ascii_case("xisf") => {
                    crate::xisf::read_header_file(path).ok()
                }
                _ => crate::analysis::fits::read_primary_header_file(path).ok(),
            }?;
            crate::analysis::sky::sensor_pixel_scale(&header)
        })
    };
    if let Some(label) = family
        && let Some(found) = families
            .iter()
            .find(|f| crate::stacking::family_label(f) == label)
    {
        return scale_of(found).map(|s| s.arcsec_per_pixel);
    }
    let scales: Vec<f64> = families
        .iter()
        .filter_map(|f| scale_of(f).map(|s| s.arcsec_per_pixel))
        .collect();
    let first = *scales.first()?;
    (scales.len() == families.len() && scales.iter().all(|s| (s - first).abs() <= 0.01 * first))
        .then_some(first)
}

/// « Siril 1.4.4 » from the line the script recorded (« siril 1.4.4 »); just
/// « Siril » when the version is unknown.
fn siril_label(recorded: Option<&str>) -> String {
    let version = recorded
        .map(str::trim)
        .filter(|line| line.to_ascii_lowercase().starts_with("siril"))
        .map(|line| line["siril".len()..].trim())
        .filter(|rest| !rest.is_empty());
    match version {
        Some(version) => format!("Siril {version}"),
        None => "Siril".to_string(),
    }
}

/// The software of an image Astro Sync made: « Astro Sync 0.9.1 · Siril 1.4.4 »
/// (`siril`: the line the script recorded; `used_siril` false when Siril took
/// no part), or « Astro Sync 0.9.1 » alone. Never the library that wrote the
/// TIFF: the service reads the file's `Software` tag only when nothing is
/// proposed.
#[must_use]
pub fn software_label(siril: Option<&str>, used_siril: bool) -> String {
    let own = format!("Astro Sync {}", crate::version());
    if used_siril {
        format!("{own} \u{b7} {}", siril_label(siril))
    } else {
        own
    }
}

/// The processing proposed for an image Astro Sync made, from its software
/// and the steps it went through (clipped to what the service keeps).
#[must_use]
pub fn processing_proposal(software: &str, steps: &[String]) -> ProcessingProposal {
    ProcessingProposal {
        software: non_empty(clip(software, MAX_SOFTWARE)),
        steps: non_empty(clip(&steps.join(" · "), MAX_STEPS)),
    }
}

/// « Astro Sync 0.9.1 · Siril 1.4.4 » and « raw · 24 lights · 30 darks · 30 flats ».
fn processing_of(
    provenance: &Provenance,
    pipeline: &str,
    contrast: crate::stacking::Contrast,
    retouch: crate::stacking::Retouch,
) -> ProcessingProposal {
    // Siril stacked nothing when the renders started from the image of a
    // device: the software is Astro Sync's own, and the steps name the image.
    let software = software_label(
        provenance.tools.siril.as_deref(),
        provenance.device_master.is_none(),
    );
    let count = |kind: &str| provenance.frames.get(kind).copied().unwrap_or(0);
    let lights = if provenance.stacked_frames > 0 {
        provenance.stacked_frames
    } else {
        count("lights")
    };
    let mut steps = vec![pipeline.to_string()];
    if let Some(device) = &provenance.device_master {
        steps.push(match device.device.as_deref().filter(|d| !d.is_empty()) {
            Some(name) => format!("{name} stack"),
            None => "device stack".to_string(),
        });
    }
    if !contrast.is_none() {
        steps.push(format!("contrast {}", contrast.token()));
    }
    steps.extend(retouch_steps(retouch));
    for (n, word) in [
        (lights, "lights"),
        (count("darks"), "darks"),
        (count("flats"), "flats"),
        (count("dark_flats"), "dark-flats"),
        (count("biases"), "biases"),
    ] {
        if n > 0 {
            steps.push(format!("{n} {word}"));
        }
    }
    ProcessingProposal {
        software: non_empty(clip(&software, MAX_SOFTWARE)),
        steps: non_empty(clip(&steps.join(" · "), MAX_STEPS)),
    }
}

/// The steps the finish adds to the processing line: `colour enhanced`,
/// `soft stars light` (nothing for « none »).
#[must_use]
pub fn retouch_steps(retouch: crate::stacking::Retouch) -> Vec<String> {
    let mut steps = Vec::new();
    if !retouch.colour.is_none() {
        steps.push(format!("colour {}", retouch.colour.token()));
    }
    if !retouch.soft_stars.is_none() {
        steps.push(format!("soft stars {}", retouch.soft_stars.token()));
    }
    steps
}

/// The name and the description of a pipeline's rendering, in `language`
/// (the language of the application, then its base, then English).
#[must_use]
pub fn render_texts(
    pipelines: &PipelineList,
    pipeline: &str,
    language: &str,
) -> (Option<String>, Option<String>) {
    if pipeline == LINEAR_PIPELINE {
        return linear_render_texts(language);
    }
    let Some(found) = pipelines.get(pipeline) else {
        return (None, None);
    };
    let title = tools::localized(&found.name, language);
    let description = tools::localized(&found.description, language);
    (
        non_empty(clip(&title, MAX_VARIANT_TITLE)),
        non_empty(clip(&description, MAX_VARIANT_DESCRIPTION)),
    )
}

/// The ten languages Astro Sync writes the texts of an image in (the locale
/// registry's): the service shows each reader the one of its language, else
/// English.
pub const TEXT_LANGUAGES: [&str; 10] = [
    "en", "fr", "de", "es", "it", "ja", "pt-BR", "ru", "zh-Hans", "zh-Hant",
];

/// A text written in the [`TEXT_LANGUAGES`], keyed by the language's tag,
/// English always there (`capture-variant.schema.json` `generatedTitle`).
pub type GeneratedText = BTreeMap<String, String>;

/// The title and the description a render gives its image, in the ten
/// languages: the pipeline's name followed by the finish the image carries
/// ([`finish_title`]: contrast, colours, softened stars) and the pipeline's
/// description (« No processing » has its own texts). `None` for a pipeline
/// the list does not hold, or without the list. Sent apart from the title
/// the owner types, whatever the language of the application: the service
/// shows each reader its own language.
#[must_use]
pub fn generated_render_texts(
    pipelines: Option<&PipelineList>,
    pipeline: &str,
    contrast: crate::stacking::Contrast,
    retouch: crate::stacking::Retouch,
) -> (Option<GeneratedText>, Option<GeneratedText>) {
    let mut titles = GeneratedText::new();
    let mut descriptions = GeneratedText::new();
    for language in TEXT_LANGUAGES {
        let texts = if pipeline == LINEAR_PIPELINE {
            linear_render_texts(language)
        } else {
            match pipelines {
                Some(list) => render_texts(list, pipeline, language),
                None => (None, None),
            }
        };
        if let Some(title) = texts.0 {
            titles.insert(
                language.to_string(),
                finish_title(&title, contrast, retouch, language),
            );
        }
        if let Some(description) = texts.1 {
            descriptions.insert(language.to_string(), description);
        }
    }
    let with_english = |text: GeneratedText| text.contains_key("en").then_some(text);
    (with_english(titles), with_english(descriptions))
}

/// The generated title of an image that carries a finish: the render's own
/// title in each language (`base`, as [`generated_render_texts`] wrote it for the
/// render alone) followed by the contrast, the colours and the softened
/// stars it carries ([`finish_title`]). The render without any keeps its
/// title as it is.
#[must_use]
pub fn finished_title(
    base: &GeneratedText,
    contrast: crate::stacking::Contrast,
    retouch: crate::stacking::Retouch,
) -> GeneratedText {
    if contrast.is_none() && retouch.is_none() {
        return base.clone();
    }
    base.iter()
        .map(|(language, title)| {
            (
                language.clone(),
                finish_title(title, contrast, retouch, language),
            )
        })
        .collect()
}

/// The title of the variant that carries the contrast of presentation:
/// the render's title, then the word « contrast » and the gain
/// ("Natural render · Contrast 2.0"), so the two images of one render do not
/// carry the same name. Ten languages; English for any other.
#[must_use]
pub fn contrast_title(title: &str, contrast: crate::stacking::Contrast, language: &str) -> String {
    let Some(gain) = contrast.gain() else {
        return title.to_string();
    };
    let base = language.split(['-', '_']).next().unwrap_or(language);
    let (word, comma) = match (language, base) {
        ("zh-Hant" | "zh-TW" | "zh-HK", _) => ("對比度", false),
        (_, "zh") => ("对比度", false),
        (_, "fr") => ("Contraste", true),
        (_, "de") => ("Kontrast", true),
        (_, "es" | "pt") => ("Contraste", true),
        (_, "it") => ("Contrasto", true),
        (_, "ja") => ("コントラスト", false),
        (_, "ru") => ("Контраст", true),
        _ => ("Contrast", false),
    };
    // 3.6 and 2.0 as they always read; a gain of the slider keeps its second decimal.
    let gain = if (gain * 10.0).fract().abs() < 1e-6 {
        format!("{gain:.1}")
    } else {
        format!("{gain:.2}")
    };
    let gain = if comma { gain.replace('.', ",") } else { gain };
    clip(&format!("{title} · {word} {gain}"), MAX_VARIANT_TITLE)
}

/// The title of the variant that carries a finish: the title with its
/// contrast ([`contrast_title`]), then the colour intensity and the softened
/// stars with their value ("Natural render · Contrast 3.6 · Colours +40% ·
/// Softened stars 85%"), so every setting is said and two finishes of one
/// render never carry the same name. The service keeps 90 characters of a
/// variant title ([`MAX_VARIANT_TITLE`]): when the full form does not fit, the
/// word of the stars is shortened ("Stars 85%"), then the values of the colours
/// and the stars are left out ("Colours · Stars"; the processing steps keep
/// them), and only then is the end cut. Ten languages; English for any other.
#[must_use]
pub fn finish_title(
    title: &str,
    contrast: crate::stacking::Contrast,
    retouch: crate::stacking::Retouch,
    language: &str,
) -> String {
    let with_contrast = contrast_title(title, contrast, language);
    if retouch.is_none() {
        return with_contrast;
    }
    let base = language.split(['-', '_']).next().unwrap_or(language);
    // (colours, softened stars, stars alone, no-break space before the %)
    let (colours, stars, stars_short, spaced) = match (language, base) {
        ("zh-Hant" | "zh-TW" | "zh-HK", _) => ("色彩", "柔化星點", "星點", false),
        (_, "zh") => ("色彩", "柔化星点", "星点", false),
        (_, "fr") => ("Couleurs", "Étoiles adoucies", "Étoiles", true),
        (_, "de") => ("Farben", "Weichere Sterne", "Sterne", true),
        (_, "es") => ("Colores", "Estrellas suavizadas", "Estrellas", true),
        (_, "pt") => ("Cores", "Estrelas suavizadas", "Estrelas", false),
        (_, "it") => ("Colori", "Stelle attenuate", "Stelle", false),
        (_, "ja") => ("色の強さ", "星をやわらげる", "星", false),
        (_, "ru") => ("Цвета", "Смягчённые звёзды", "Звёзды", true),
        _ => ("Colours", "Softened stars", "Stars", false),
    };
    let percent =
        |value: u32, sign: &str| format!("{sign}{value}{}%", if spaced { "\u{a0}" } else { "" });
    let colour_value =
        (!retouch.colour.is_none()).then(|| percent(u32::from(retouch.colour.percent()), "+"));
    let stars_value = retouch.soft_stars.percent().map(|share| percent(share, ""));
    // The forms from the fullest to the shortest.
    let build = |stars_word: &str, values: bool| {
        let mut text = with_contrast.clone();
        if !retouch.colour.is_none() {
            text += &match (&colour_value, values) {
                (Some(value), true) => format!(" · {colours} {value}"),
                _ => format!(" · {colours}"),
            };
        }
        if !retouch.soft_stars.is_none() {
            text += &match (&stars_value, values) {
                (Some(value), true) => format!(" · {stars_word} {value}"),
                _ => format!(" · {stars_word}"),
            };
        }
        text
    };
    for (stars_word, values) in [(stars, true), (stars_short, true), (stars_short, false)] {
        let text = build(stars_word, values);
        if text.chars().count() <= MAX_VARIANT_TITLE {
            return text;
        }
    }
    clip(&build(stars_short, false), MAX_VARIANT_TITLE)
}

/// The name and the help of "No processing", which is not a pipeline
/// (no definition to read): fixed texts in ten languages.
fn linear_render_texts(language: &str) -> (Option<String>, Option<String>) {
    let (title, description) = pipelines::linear_texts(language);
    (
        non_empty(clip(&title, MAX_VARIANT_TITLE)),
        non_empty(clip(&description, MAX_VARIANT_DESCRIPTION)),
    )
}

/* ------------------------------ The proposals ---------------------------- */

/// What the proposals are built from.
pub struct ProposalContext<'a> {
    pub night_folder: &'a Path,
    pub detection: &'a NightDetection,
    /// The language of the application (`fr`, `ja`, `pt-BR`...); the texts
    /// of an image are written in the ten languages whatever it is.
    pub language: &'a str,
    /// The pipelines of the person (for the title and description of a
    /// rendering).
    pub pipelines: Option<&'a PipelineList>,
    /// The space holds `analysis.logs`: the stacked image's quality is built
    /// and proposed. Without it nothing is measured for the service, and the
    /// image is sent as it is.
    pub image_quality: bool,
}

/// The proposals for one image: the target's acquisition, sensor and the
/// night's calibration; for a stacked image its processing, the title and
/// description of its rendering and, under `analysis.logs`, its quality. `families` is the target's, read once.
#[must_use]
pub fn proposals_for_image(
    image: &Path,
    kind: ResultKind,
    render: Option<&str>,
    families: &[crate::analysis::fits::AcquisitionFamily],
    ctx: &ProposalContext<'_>,
) -> Proposals {
    let mut proposals = Proposals {
        acquisition: acquisition_lines(families),
        calibration: calibration_text(&ctx.detection.calibration),
        sensor: sensor_of(families),
        ..Proposals::default()
    };
    if kind == ResultKind::Stacked
        && let Some((provenance, recorded)) = provenance_of(image)
    {
        let pipeline = render.map_or(recorded, str::to_string);
        proposals.processing = Some(processing_of(
            &provenance,
            &pipeline,
            recorded_contrast(image),
            recorded_retouch(image),
        ));
        // The same steps are the image's own too: the capture shows the ones
        // of its main image, so they follow it when another image takes over.
        proposals.variant_processing = proposals.processing.clone();
        if ctx.image_quality
            && let Some(measure) = provenance.quality.as_ref()
        {
            let family = provenance
                .integration
                .as_ref()
                .and_then(|i| i.family.as_deref());
            proposals.image_quality =
                ImageQuality::from_measure(measure, frames_pixel_scale(families, family));
        }
        let (title, description) = generated_render_texts(
            ctx.pipelines,
            &pipeline,
            recorded_contrast(image),
            recorded_retouch(image),
        );
        proposals.variant_generated_title = title;
        proposals.variant_generated_description = description;
    }
    // A master stacked elsewhere and measured by hand ("Measure a master…")
    // gives its quality to the images of its target that Astro Sync did not
    // stack -- the same field, the same contract, the same right
    // (`analysis.logs`) as the quality of its own stacking.
    if proposals.image_quality.is_none()
        && ctx.image_quality
        && matches!(kind, ResultKind::Final | ResultKind::Stacked)
        && let Some(record) = crate::master_quality::record_near(image, ctx.night_folder)
    {
        proposals.image_quality =
            ImageQuality::from_measure(&record.quality, frames_pixel_scale(families, None));
    }
    proposals
}

/// Puts the proposals on the plans that send an image or find it online, as
/// the decision allows:
///
/// - a **new capture** carries everything;
/// - a **variant** of a capture online carries its title and description and
///   what the capture could still be missing (acquisition, calibration,
///   sensor) -- never the processing, which is the main image's;
/// - a variant of a capture **this sending creates** carries its title and
///   description only (the first image carries the rest);
/// - an image **already online** is proposed after the fact (`PATCH`): the
///   same as a variant, and the processing too when it is the main image.
pub fn attach_proposals(plans: &mut [ImagePlan], online: &NightOnline, ctx: &ProposalContext<'_>) {
    let mut families: BTreeMap<String, Vec<crate::analysis::fits::AcquisitionFamily>> =
        BTreeMap::new();
    for plan in plans.iter_mut() {
        if matches!(plan.decision, Decision::NotSent { .. })
            || plan.image.kind == ResultKind::Progression
        {
            continue;
        }
        let folder = ctx
            .detection
            .targets
            .iter()
            .find(|t| t.target_id == plan.image.target_id)
            .and_then(|t| t.folder_name.clone());
        let target_families = match &folder {
            Some(folder) => families
                .entry(plan.image.target_id.clone())
                .or_insert_with(|| progression_images::target_families(ctx.night_folder, folder))
                .as_slice(),
            None => &[],
        };
        let all = proposals_for_image(
            &plan.image.path,
            plan.image.kind,
            plan.image.render.as_deref(),
            target_families,
            ctx,
        );
        plan.from_device =
            plan.image.kind == ResultKind::Stacked && recorded_device_master(&plan.image.path);
        let proposals = match &plan.decision {
            Decision::NewCapture => all,
            Decision::Variant { .. } => all.without_processing(),
            Decision::VariantOfNew { .. } => all.image_part(),
            Decision::AlreadyOnline { image_id, .. } => {
                let main = online
                    .images()
                    .any(|(_, image)| image.id == *image_id && image.main);
                if main { all } else { all.without_processing() }
            }
            // A progression image proposes nothing (it is the history of
            // a capture, never one of its images to describe).
            Decision::NotSent { .. }
            | Decision::Progression { .. }
            | Decision::ProgressionOfNew { .. } => continue,
        };
        plan.metadata = (!proposals.is_empty()).then_some(proposals);
    }
}

#[cfg(test)]
mod tests;

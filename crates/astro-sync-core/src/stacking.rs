//! The place of the stacking: "Stacking from the FITS" turns a target's
//! `lights/`, with the night's `calibration/`, into a stacked image written
//! by Astro Sync in **`<target>/stacked/`, never in `final/`**.
//!
//! - **The slot** is a trait, [`Stacker`]: a job in, renders out, progress,
//!   the stop switch, a failure in words. [`run_stacking`] holds the rules
//!   around it: **the stacking is local and never gated** by a capability
//!   (every account, a private server and no account at all stack; no
//!   capability names the stacking, and `analysis.progression` keeps only
//!   the progressions, the intermediate stackings sent at regular intervals:
//!   [`crate::progression_images`], [`crate::plan_progression`]); only the
//!   pipeline row of "Dependencies" is asked for. The renders asked are the
//!   **pipelines chosen** (`pipelineId`, repeatable; `raw` by default) --
//!   several only when the space keeps variants (`capture.variant_limit` >
//!   0: each render after the first is a variant; every profile keeps them
//!   today, and a stacking without a space is not limited), one otherwise;
//!   every file the stacker
//!   names must be inside `stacked/`, and no file there is ever replaced (a
//!   name already taken gets `-2`, `-3`…).
//! - **One call per family** ([`plan_target`]): the script
//!   stacks what it is given, and the pipeline never stacks two filters, two
//!   exposures or two gains together. The target's lights are grouped by the
//!   families of `analysis::fits` (filter, exposure, gain, binning; the
//!   sensor temperature, rounded to a degree, does not split a stacking);
//!   one family: the whole `lights/` folder, as it is; several: one stacking
//!   each (`--frames-from`, the list written in Astro Sync's data), the
//!   largest first, its name ending with the family (`M31_2026-10-07_Ha_300s_g100`).
//!   A space that keeps no variant gets the largest family only, the others
//!   said (`set_aside`).
//! - **The launcher of the stacking script** ([`PythonStacker`],
//!   `python -B -E -s -X utf8 astro_sync_stack.py --json`, in the environment of
//!   [`crate::python_env`]): the pipelines as `--pipeline <id>`, the digest
//!   the person trusted for a script of their own as
//!   `--trusted <id>=<sha256>` (`trustedSha256`, recorded by
//!   [`crate::pipeline_trust`]; the script refuses an untrusted one,
//!   `script_not_trusted`), the calibration folders that exist, Siril and
//!   StarNet2 where "Dependencies" found them, its JSON events read as they
//!   come. A stop sends it `SIGTERM`: it stops Siril and answers
//!   `cancelled` (code 130). Its failures keep its error code
//!   (`too_few_lights`, `siril_failed`…); without one, the exit code says the
//!   kind: 2 `bad_input`, 3 `tool_missing`, 4 `tool_failed`.
//! - **Room first** ([`run_target_stacking`]): about 210 MB per light frame
//!   in the scratch folder (a calibrated and a registered copy, 32-bit colour,
//!   as measured); less free room **on the disk of the night** refuses before
//!   anything runs (`not_enough_space`, with the night's folder in it).
//! - **Scratch**: the stacking's scratch lives on the disk
//!   of the night, in `<night>/.astro-sync-work/stacking/<job name>/`
//!   ([`crate::working_folder::stacking_scratch_root`]), never in Astro
//!   Sync's data on the system disk. It is created for the job and removed
//!   when the job ends, whether it succeeded, failed or was stopped;
//!   `.astro-sync-work/` goes too when it is empty. It is the only thing
//!   Astro Sync removes in the person's tree, and only inside that hidden
//!   folder, which the scanner skips (a dot-directory).
//!   Before it goes, on a failure or a stop, its journals (`siril.log`, the
//!   script's `render.log`, any `*.log` or `*.txt`, never an image) are
//!   copied to `<Astro Sync data>/stacking-logs/<job name>-<YYYYMMDD-HHMMSS>/`
//!   ([`keep_logs`]; the ten most recent folders are kept) and the failure
//!   names that folder; a success keeps nothing. A scratch left behind by a
//!   hard stop of the app is removed when the next stacking of the night
//!   starts ([`clear_stale_scratch`]).
//! - **"No processing"**: the choice
//!   [`PipelineChoice::linear`] (id [`LINEAR_PIPELINE`], `linear`) is not a
//!   pipeline. It excludes every render: the script is run with
//!   `--linear-only`, no render runs (no StarNet2, no pipeline definition,
//!   no trust question) and it writes the linear stack, not stretched, as
//!   `<target>/stacked/<name>_linear.tif` (16-bit TIFF). That file leaves like
//!   a render does (the capture's main image, or a variant when one is
//!   online), its `render` being `linear`.
//! - **The night's window**: [`plan_target_within`]
//!   leaves out of the integration the lights taken outside the night
//!   ([`crate::detection::NightWindow`], local noon to local noon); they stay
//!   in `lights/`, counted by the detection, and [`TargetPlan::outside`]
//!   names them.
//! - **The master kept**: every job
//!   names its family ([`StackJob::integration`], `--integration <family>`)
//!   and the script keeps the 32-bit linear master of the family as
//!   `<target>/stacked/<family>_integration.fit` -- the only intermediate
//!   kept, so that a later night of a multi-night plan can be combined with
//!   it ([`crate::plan_image`]). A new stacking of the same family replaces
//!   it. [`StackOutcome::integrations`] describes each one
//!   ([`crate::multi_night::read_master`]).
//! - **The image of a device**: a family may take the
//!   image its device already stacked (the Seestar's `Stacked_*.fit`, found
//!   in `<target>/device/`, [`crate::device_master`]) instead of a stacking of
//!   its poses. The job then carries [`StackJob::device_master`]: the script
//!   runs with `--from-master <file>`, stacks and calibrates nothing, and
//!   renders from that image like it does from its own master (the
//!   pipelines, the contrast, "No processing"). It keeps no master of
//!   the family (the image of a multi-night plan is made from poses), reads
//!   no pose and needs no room beyond the render's. The choice reaches
//!   [`plan_target_with_device`] as the files taken ([`DeviceUse`]); without
//!   it nothing changes.
//! - **What leaves** ([`stacked_candidates`]): each render's 16-bit TIFF,
//!   as [`ResultKind::Stacked`] (source `stacking`), the pipeline's id as
//!   its `render`; [`crate::precheck::plan_images`] makes the first the
//!   capture (or a variant of the capture online) and each other a variant.

use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::analysis::fits::AcquisitionFamily;
use crate::detection::NightWindow;
use crate::device_master::{self, DeviceImageInfo, DeviceUse};
use crate::external_process;
use crate::gui_bridge::Capabilities;
use crate::local_work::CancelFlag;
use crate::multi_night::{self, IntegrationMaster};
use crate::pipelines::{DEFAULT_PIPELINE, LINEAR_PIPELINE};
use crate::precheck::ImageCandidate;
use crate::progression_images;
use crate::python_env;
use crate::siril_progress::{LogTail, POLL_EVERY, SirilFollower, now_ms};
use crate::upload_queue::ResultKind;
use crate::working_folder::{
    CALIBRATION_DIR, CALIBRATION_KINDS, DEVICE_DIR, LIGHTS_DIR, STACKED_DIR, WORK_DIR,
    stacking_scratch_root, target_folder_name,
};

/// A number written as a word (`presentation`, `strong`) or as itself in a
/// job, a setting or an option: what the three finish settings read.
fn number_of(word: &str, words: &[(&str, f64)], maximum: f64, what: &str) -> Result<f64, String> {
    let text = word.trim().to_ascii_lowercase();
    if text.is_empty() || text == "none" {
        return Ok(0.0);
    }
    if let Some((_, value)) = words.iter().find(|(name, _)| *name == text) {
        return Ok(*value);
    }
    let number: f64 = text.parse().map_err(|_| {
        let names: Vec<&str> = std::iter::once("none")
            .chain(words.iter().map(|(name, _)| *name))
            .collect();
        format!(
            "unknown {what} {word:?}: a number from 0 to {maximum}, or {}",
            names.join(", ")
        )
    })?;
    if !(0.0..=maximum).contains(&number) {
        return Err(format!("{what} {word:?} is not between 0 and {maximum}"));
    }
    Ok(number)
}

/// What a JSON value may be for a finish setting: a word (`"strong"`, `"none"`),
/// a number as text (`"2.5"`) or a number.
#[derive(Deserialize)]
#[serde(untagged)]
enum Setting {
    Text(String),
    Number(f64),
}

impl Setting {
    fn text(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Number(number) => number.to_string(),
        }
    }
}

/// The contrast of presentation (`--contrast` of the stacking
/// script): an S-curve on the luminance around the sky level, written as a
/// second image beside each render (`<stem>_presentation`, `<stem>_strong`, or
/// `<stem>_contrast-2.5`). The setting is continuous: a gain from 0 to
/// [`Contrast::MAX_GAIN`], kept in hundredths. `0` (the default) writes nothing
/// more than the render itself; `2.0` and `3.6` are the two values the window
/// always offered and keep their words (`presentation`, `strong`) in the job
/// file, the settings and the file names; the linear master of an
/// integration is never touched, whatever the choice.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Contrast(u16);

#[allow(non_upper_case_globals)]
impl Contrast {
    /// The master of the catalogue as it is.
    pub const None: Self = Self(0);
    /// Gain 2.0, the presentation reference of the screen.
    pub const Presentation: Self = Self(200);
    /// Gain 3.6, the print gain, good on a screen as well.
    pub const Strong: Self = Self(360);
    /// The largest gain the window and the options accept.
    pub const MAX_GAIN: f64 = 6.0;
    const WORDS: [(&'static str, f64); 2] = [("presentation", 2.0), ("strong", 3.6)];

    /// The contrast of a gain, in hundredths; an error outside 0..=[`MAX_GAIN`].
    pub fn from_gain(gain: f64) -> Result<Self, String> {
        if !(0.0..=Self::MAX_GAIN).contains(&gain) {
            return Err(format!(
                "contrast {gain} is not between 0 and {}",
                Self::MAX_GAIN
            ));
        }
        Ok(Self((gain * 100.0).round() as u16))
    }

    /// The word of `--contrast`, of the job file and of the settings: `none`,
    /// `presentation`, `strong`, or the gain (`2.5`).
    #[must_use]
    pub fn token(self) -> String {
        match self.0 {
            0 => "none".to_string(),
            200 => "presentation".to_string(),
            360 => "strong".to_string(),
            hundredths => format!("{}", f64::from(hundredths) / 100.0),
        }
    }

    /// The gain of the curve; `None` for « none ».
    #[must_use]
    pub fn gain(self) -> Option<f64> {
        (self.0 > 0).then(|| f64::from(self.0) / 100.0)
    }

    /// For `skip_serializing_if`: the default writes nothing.
    #[must_use]
    pub fn is_none(&self) -> bool {
        self.0 == 0
    }

    /// The suffix of the files that carry the curve: `_presentation`,
    /// `_strong`, `_contrast-2.5`; empty for « none ».
    #[must_use]
    pub fn suffix(self) -> String {
        match self.0 {
            0 => String::new(),
            200 | 360 => format!("_{}", self.token()),
            _ => format!("_contrast-{}", self.token()),
        }
    }
}

impl std::str::FromStr for Contrast {
    type Err = String;

    fn from_str(word: &str) -> Result<Self, Self::Err> {
        Self::from_gain(number_of(word, &Self::WORDS, Self::MAX_GAIN, "contrast")?)
    }
}

impl Serialize for Contrast {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            0 | 200 | 360 => serializer.serialize_str(&self.token()),
            hundredths => serializer.serialize_f64(f64::from(hundredths) / 100.0),
        }
    }
}

impl<'de> Deserialize<'de> for Contrast {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Setting::deserialize(deserializer)?
            .text()
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// A percent, kept as a whole number, of the finish settings.
fn percent_of(word: &str, words: &[(&str, f64)], what: &str) -> Result<u8, String> {
    Ok(number_of(word, words, 100.0, what)?.round() as u8)
}

/// The colour intensity of the finish: the colours of each render
/// strengthened by a percent, from 0 to 100, its luminance kept (`--colour` of
/// the stacking script, a number). Written in the same extra image as the
/// contrast and the softened stars. `0` (the default) changes nothing; `20`
/// and `40` are the window's detents.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Colour(u8);

#[allow(non_upper_case_globals)]
impl Colour {
    /// The colours of the render as they are.
    pub const None: Self = Self(0);
    /// Colours +20 %, the first detent.
    pub const Enhanced: Self = Self(20);
    /// Colours +40 %, the second detent.
    pub const Strong: Self = Self(40);
    /// The largest percent.
    pub const MAX_PERCENT: u8 = 100;
    const WORDS: [(&'static str, f64); 2] = [("enhanced", 20.0), ("strong", 40.0)];

    /// The colour intensity of a percent; an error above 100.
    pub fn from_percent(percent: u8) -> Result<Self, String> {
        if percent > Self::MAX_PERCENT {
            return Err(format!(
                "colour intensity {percent} is not between 0 and 100"
            ));
        }
        Ok(Self(percent))
    }

    /// The percent the colours are strengthened by.
    #[must_use]
    pub fn percent(self) -> u8 {
        self.0
    }

    /// The word of `--colour`: `none` or the percent (`40`).
    #[must_use]
    pub fn token(self) -> String {
        if self.0 == 0 {
            "none".to_string()
        } else {
            self.0.to_string()
        }
    }

    /// The factor the colours are multiplied by; `None` for « none ».
    #[must_use]
    pub fn gain(self) -> Option<f64> {
        (self.0 > 0).then(|| 1.0 + f64::from(self.0) / 100.0)
    }

    /// For `skip_serializing_if`: the default writes nothing.
    #[must_use]
    pub fn is_none(&self) -> bool {
        self.0 == 0
    }
}

impl std::str::FromStr for Colour {
    type Err = String;

    fn from_str(word: &str) -> Result<Self, Self::Err> {
        Self::from_percent(percent_of(word, &Self::WORDS, "colour intensity")?)
    }
}

impl Serialize for Colour {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(self.0)
    }
}

impl<'de> Deserialize<'de> for Colour {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Setting::deserialize(deserializer)?
            .text()
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// The softened stars of the finish: the brightest stars pulled towards the
/// image without them, by a percent from 0 to 100 (`--soft-stars` of the
/// stacking script, a number). `0` (the default) changes nothing; `50` and `85`
/// are the window's detents.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SoftStars(u8);

#[allow(non_upper_case_globals)]
impl SoftStars {
    /// The stars of the render as they are.
    pub const None: Self = Self(0);
    /// Half of each bright star taken out, the first detent.
    pub const Light: Self = Self(50);
    /// 85 % of each bright star taken out, the second detent.
    pub const Strong: Self = Self(85);
    /// The largest percent.
    pub const MAX_PERCENT: u8 = 100;
    const WORDS: [(&'static str, f64); 2] = [("light", 50.0), ("strong", 85.0)];

    /// The star softening of a percent; an error above 100.
    pub fn from_percent(percent: u8) -> Result<Self, String> {
        if percent > Self::MAX_PERCENT {
            return Err(format!("star softening {percent} is not between 0 and 100"));
        }
        Ok(Self(percent))
    }

    /// The word of `--soft-stars`: `none` or the percent (`85`).
    #[must_use]
    pub fn token(self) -> String {
        if self.0 == 0 {
            "none".to_string()
        } else {
            self.0.to_string()
        }
    }

    /// The share of the stars taken out, in percent; `None` for « none ».
    #[must_use]
    pub fn percent(self) -> Option<u32> {
        (self.0 > 0).then_some(u32::from(self.0))
    }

    /// For `skip_serializing_if`: the default writes nothing.
    #[must_use]
    pub fn is_none(&self) -> bool {
        self.0 == 0
    }
}

impl std::str::FromStr for SoftStars {
    type Err = String;

    fn from_str(word: &str) -> Result<Self, Self::Err> {
        Self::from_percent(percent_of(word, &Self::WORDS, "star softening")?)
    }
}

impl Serialize for SoftStars {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(self.0)
    }
}

impl<'de> Deserialize<'de> for SoftStars {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Setting::deserialize(deserializer)?
            .text()
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// What the finish adds to the contrast: the colour intensity and the
/// softened stars. Flattened beside `contrast` wherever an image or a job
/// carries it (`colour` and `softStars` in the JSON of the job and of the
/// events, nothing at all for the defaults), so a file written before the
/// options reads as « none, none ».
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Retouch {
    #[serde(default, skip_serializing_if = "Colour::is_none")]
    pub colour: Colour,
    #[serde(default, skip_serializing_if = "SoftStars::is_none")]
    pub soft_stars: SoftStars,
}

impl Retouch {
    /// For `skip_serializing_if`: the defaults write nothing.
    #[must_use]
    pub fn is_none(&self) -> bool {
        self.colour.is_none() && self.soft_stars.is_none()
    }

    /// The suffix the files that carry the finish have after the contrast's:
    /// `_colour-40`, `_stars-85`, both in that order; empty for « none ».
    #[must_use]
    pub fn suffix(self) -> String {
        let mut suffix = String::new();
        if !self.colour.is_none() {
            suffix += &format!("_colour-{}", self.colour.token());
        }
        if !self.soft_stars.is_none() {
            suffix += &format!("_stars-{}", self.soft_stars.token());
        }
        suffix
    }
}

/// The engine of the background and of the denoising of the "polished render"
/// (R348b, `--polished-engine` of the stacking script, `recipe.render.polished.engine`
/// of the provenance). GraXpert's AI models are the default; Siril's own
/// `subsky -rbf` and `denoise` need no model and are also what the script falls
/// back on when GraXpert is missing or fails.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PolishedEngine {
    /// GraXpert's AI models (background 1.0.1, denoising 3.0.2).
    #[default]
    GraxpertAi,
    /// Siril alone.
    Siril,
}

impl std::str::FromStr for PolishedEngine {
    type Err = String;

    fn from_str(word: &str) -> Result<Self, Self::Err> {
        [Self::GraxpertAi, Self::Siril]
            .into_iter()
            .find(|engine| engine.as_str() == word)
            .ok_or_else(|| format!("unknown engine {word:?}: graxpert-ai or siril"))
    }
}

impl PolishedEngine {
    /// The word of `--polished-engine` and of the job file.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GraxpertAi => "graxpert-ai",
            Self::Siril => "siril",
        }
    }

    /// For `skip_serializing_if`: the default writes nothing.
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// The finish of one pipeline for one target (« Finition » in its card of
/// the window « Pipelines pour … »): the contrast, the colour intensity and
/// the softened stars of the image written beside that pipeline's render.
/// The three at « none » ask for the render alone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineFinish {
    #[serde(default, skip_serializing_if = "Contrast::is_none")]
    pub contrast: Contrast,
    #[serde(default, flatten)]
    pub retouch: Retouch,
}

impl PipelineFinish {
    /// The value of `--finish <id>=…` of the stacking script:
    /// `strong,20,50`, `none,none,none`.
    #[must_use]
    pub fn tokens(self) -> String {
        format!(
            "{},{},{}",
            self.contrast.token(),
            self.retouch.colour.token(),
            self.retouch.soft_stars.token()
        )
    }
}

/// A pipeline chosen at "Processing", with the digest the person
/// trusted when it runs a script of their own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineChoice {
    pub pipeline_id: String,
    #[serde(default)]
    pub trusted_sha256: Option<String>,
    /// The finish chosen for this pipeline of this target; `None`: the
    /// default finish of the night (the job's `contrast` and `retouch`).
    /// "No processing" never takes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish: Option<PipelineFinish>,
}

impl PipelineChoice {
    #[must_use]
    pub fn new(pipeline_id: &str) -> Self {
        Self {
            pipeline_id: pipeline_id.to_string(),
            trusted_sha256: None,
            finish: None,
        }
    }

    /// The same choice with a finish of its own.
    #[must_use]
    pub fn with_finish(mut self, finish: PipelineFinish) -> Self {
        self.finish = Some(finish);
        self
    }

    /// "No processing": no render, only the linear stack as a TIFF.
    #[must_use]
    pub fn linear() -> Self {
        Self::new(LINEAR_PIPELINE)
    }

    /// Whether this is "No processing" rather than a pipeline.
    #[must_use]
    pub fn is_linear(&self) -> bool {
        self.pipeline_id == LINEAR_PIPELINE
    }
}

/// One stacking: a target of a night.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackJob {
    pub lights: PathBuf,
    pub darks: Option<PathBuf>,
    pub flats: Option<PathBuf>,
    pub dark_flats: Option<PathBuf>,
    pub biases: Option<PathBuf>,
    /// `<target>/stacked/`.
    pub output_dir: PathBuf,
    /// The stem of the files (`<name>_<pipeline>.tif`).
    pub name: String,
    /// The scratch folder, `<night>/.astro-sync-work/stacking/<name>/`.
    pub work_dir: PathBuf,
    pub pipelines: Vec<PipelineChoice>,
    /// The family of exposures stacked (`Ha_300s_g100`), when the target has
    /// several.
    #[serde(default)]
    pub family: Option<String>,
    /// The light frames of that family; empty: every frame of `lights`.
    #[serde(default)]
    pub frames: Vec<PathBuf>,
    /// The family the master kept is named after
    /// (`<target>/stacked/<integration>_integration.fit`), even when the
    /// target has one family only; `None`: no master kept.
    #[serde(default)]
    pub integration: Option<String>,
    /// The contrast of presentation: each render is also written
    /// with the curve, beside it.
    #[serde(default, skip_serializing_if = "Contrast::is_none")]
    pub contrast: Contrast,
    /// The colour intensity and the softened stars of that same image: it is
    /// written when either is asked, with or without a contrast.
    #[serde(default, flatten)]
    pub retouch: Retouch,
    /// The engine of the "polished render"'s background and denoising (R348b).
    #[serde(default, skip_serializing_if = "PolishedEngine::is_default")]
    pub polished_engine: PolishedEngine,
    /// The image a device already stacked (`<target>/device/<file>`):
    /// rendered as it is, nothing stacked, no calibration, no master kept.
    /// `None`: the poses are stacked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_master: Option<PathBuf>,
    /// The filter of the family when the night keeps its flats by filter
    /// (`calibration/flats/<filter>/`) and has none for it: the family is
    /// stacked without flats, never with another filter's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flats_missing_for: Option<String>,
    /// The filter the lights were taken with when their headers name none
    /// (`--capture-filter` of the script): the target folder's record
    /// ([`crate::working_folder::TargetIdentity::filter`]). The script stamps
    /// it on the master it keeps and reads a dual-band one for the polished
    /// render; the frames are never written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_filter: Option<String>,
    /// The frame of the stacking (« Cadre de l'empilement » of the target):
    /// the rank of a notch, 0 (the area every light covers) to
    /// [`FRAME_NOTCHES`] − 1 (the whole field); `None`: Automatic, the
    /// framing the measured rotation chooses (`--frame-size` of the script,
    /// `night_framing.py`). Only a field that turns has notches: a field that
    /// does not keeps its common area whatever the rank.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<u8>,
}

impl StackJob {
    /// Whether the job is "No processing" alone: no render, the linear
    /// stack only (`--linear-only`).
    #[must_use]
    pub fn is_linear_only(&self) -> bool {
        !self.pipelines.is_empty() && self.pipelines.iter().all(PipelineChoice::is_linear)
    }

    /// Whether "No processing" comes beside renders
    /// (combinable with them): the linear stack is written
    /// too (`--linear`), after them.
    #[must_use]
    pub fn adds_linear(&self) -> bool {
        !self.is_linear_only() && self.pipelines.iter().any(PipelineChoice::is_linear)
    }

    /// The list of `frames` given to the script (`--frames-from`), beside
    /// the scratch folder.
    #[must_use]
    pub fn frames_list(&self) -> PathBuf {
        let mut name = self.work_dir.file_name().unwrap_or_default().to_os_string();
        name.push(".lights.txt");
        self.work_dir.with_file_name(name)
    }

    /// The light frames the script will read (FITS only, like it).
    #[must_use]
    pub fn light_count(&self) -> u64 {
        // The image of a device reads no pose.
        if self.device_master.is_some() {
            return 0;
        }
        if !self.frames.is_empty() {
            return self.frames.len() as u64;
        }
        walkdir::WalkDir::new(&self.lights)
            .follow_links(true)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file() && is_fits(entry.path()))
            .count() as u64
    }
}

/// A light the script reads: a FITS frame, or a camera RAW file (converted
/// by Siril).
fn is_fits(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(e.to_ascii_lowercase().as_str(), "fit" | "fits" | "fts")
            || crate::raw_header::is_raw_extension(e)
    })
}

fn non_empty_dir(path: PathBuf) -> Option<PathBuf> {
    fs::read_dir(&path)
        .ok()?
        .filter_map(Result::ok)
        .any(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .then_some(path)
}

/// The renders of a job in the order they are made: each pipeline once, in
/// the order chosen, "No processing" last; none: the default render.
#[must_use]
pub fn ordered_pipelines(pipelines: &[PipelineChoice]) -> Vec<PipelineChoice> {
    let mut ordered: Vec<PipelineChoice> = Vec::new();
    for choice in pipelines.iter().filter(|p| !p.is_linear()) {
        if !ordered.iter().any(|p| p.pipeline_id == choice.pipeline_id) {
            ordered.push(choice.clone());
        }
    }
    if pipelines.iter().any(PipelineChoice::is_linear) {
        ordered.push(PipelineChoice::linear());
    }
    if ordered.is_empty() {
        ordered.push(PipelineChoice::new(DEFAULT_PIPELINE));
    }
    ordered
}

/// The job of `target_folder` (a folder name of the night) in
/// `night_folder`: its lights, the calibration folders that hold files, the
/// output in `stacked/` under a name no file there has, the scratch folder
/// under `<night>/.astro-sync-work/stacking/`. `pipelines` empty: the
/// default render. "No processing" ([`PipelineChoice::linear`]) alone
/// runs no render; beside renders it is written too, last, so the
/// first render stays the capture's image. Each pipeline is kept once.
#[must_use]
pub fn plan_stack(
    night_folder: &Path,
    target_folder: &str,
    evening_date: &str,
    pipelines: &[PipelineChoice],
) -> StackJob {
    let target = night_folder.join(target_folder);
    let calibration = |name: &str| non_empty_dir(night_folder.join(CALIBRATION_DIR).join(name));
    let pipelines = ordered_pipelines(pipelines);
    let output_dir = target.join(STACKED_DIR);
    let stem: String = format!("{target_folder}_{evening_date}")
        .chars()
        .map(|c| if c.is_whitespace() { '_' } else { c })
        .collect();
    let taken = |name: &str| {
        pipelines.iter().any(|p| {
            ["tif", "png"].iter().any(|ext| {
                output_dir
                    .join(format!("{name}_{}.{ext}", p.pipeline_id))
                    .exists()
            })
        })
    };
    let mut name = stem.clone();
    let mut n = 2;
    while taken(&name) {
        name = format!("{stem}-{n}");
        n += 1;
    }
    let [darks, flats, dark_flats, biases] = CALIBRATION_KINDS;
    StackJob {
        lights: target.join(LIGHTS_DIR),
        darks: calibration(darks),
        flats: calibration(flats),
        dark_flats: calibration(dark_flats),
        biases: calibration(biases).or_else(|| calibration(crate::working_folder::BIAS_ALIAS)),
        output_dir,
        work_dir: stacking_scratch_root(night_folder).join(&name),
        name,
        pipelines,
        family: None,
        frames: Vec::new(),
        integration: None,
        contrast: Contrast::None,
        retouch: Retouch::default(),
        polished_engine: PolishedEngine::default(),
        device_master: None,
        flats_missing_for: None,
        capture_filter: None,
        frame: None,
    }
}

/// The flats a family of `filter` stacks with.
///
/// A night that keeps its flats by filter (`calibration/flats/<filter>/`, as
/// `organise` writes them) gives a family the folder of **its** filter and
/// nothing else: another filter's flats flatten another light path (up to a
/// tenth of the signal in the corners, measured on a four-filter CCD night).
/// A family whose filter has no folder takes no flats and says which filter
/// ([`StackJob::flats_missing_for`]). A family without a filter takes the
/// only filter folder there is, if there is only one. A night that keeps its
/// flats loose in `calibration/flats/` is read as before: the folder itself.
fn family_flats(night_folder: &Path, filter: Option<&str>) -> (Option<PathBuf>, Option<String>) {
    let root = night_folder
        .join(CALIBRATION_DIR)
        .join(CALIBRATION_KINDS[1]);
    let visible = |entry: &fs::DirEntry| !entry.file_name().to_string_lossy().starts_with('.');
    let (mut folders, mut loose) = (Vec::new(), false);
    for entry in fs::read_dir(&root)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(visible)
    {
        // A link to a folder is a folder.
        if entry.path().is_dir() {
            if let Some(folder) = non_empty_dir(entry.path()) {
                folders.push(folder);
            }
        } else {
            loose = true;
        }
    }
    if folders.is_empty() {
        return (loose.then_some(root), None);
    }
    let name = |folder: &Path| {
        folder
            .file_name()
            .map(|n| target_folder_name(&n.to_string_lossy()).to_lowercase())
            .unwrap_or_default()
    };
    match filter.map(str::trim).filter(|f| !f.is_empty()) {
        Some(filter) => {
            let wanted = target_folder_name(filter).to_lowercase();
            match folders.into_iter().find(|f| name(f) == wanted) {
                Some(folder) => (Some(folder), None),
                None => (None, Some(filter.to_string())),
            }
        }
        None if folders.len() == 1 && !loose => (folders.pop(), None),
        None => (None, None),
    }
}

/// `job` with the flats of its family's filter (see [`family_flats`]).
fn with_family_flats(mut job: StackJob, night_folder: &Path, filter: Option<&str>) -> StackJob {
    let (flats, missing) = family_flats(night_folder, filter);
    job.flats = flats;
    job.flats_missing_for = missing;
    job
}

/// One family of the target's exposures, as the stacking groups them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackFamily {
    /// `Ha_300s_g100` (filter, exposure, gain; binning when not 1×1).
    pub label: String,
    /// The filter the poses name (`FILTER`), as written.
    #[serde(default)]
    pub filter: Option<String>,
    pub frames: Vec<PathBuf>,
}

fn number(value: f64) -> String {
    if (value - value.round()).abs() < 1e-6 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value}")
    }
}

pub(crate) fn family_label(family: &AcquisitionFamily) -> String {
    let filter = family.filter.as_deref().unwrap_or("nofilter");
    let mut parts = vec![filter.to_string()];
    if let Some(exposure) = family.exposure_seconds {
        parts.push(format!("{}s", number(exposure)));
    }
    if let Some(gain) = family.gain {
        parts.push(format!("g{}", number(gain)));
    }
    if let Some(binning) = family.binning.as_deref().filter(|b| *b != "1x1") {
        parts.push(format!("bin{binning}"));
    }
    // RAW poses: the camera too, so that two cameras (or a camera and a
    // cooled sensor) of the same settings are never stacked together.
    if let Some(camera) = family.camera.as_deref() {
        parts.push(camera.to_string());
    }
    parts
        .join("_")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The target's light frames by family (filter, exposure, gain, binning:
/// the families of `analysis::fits`, their temperatures merged), the
/// largest first. FITS only, as the script reads them.
#[must_use]
pub fn stack_families(night_folder: &Path, target_folder: &str) -> Vec<StackFamily> {
    stack_families_within(night_folder, target_folder, None).0
}

/// [`stack_families`] of the frames inside the night's window, and the
/// frames outside it.
#[must_use]
pub fn stack_families_within(
    night_folder: &Path,
    target_folder: &str,
    window: Option<&NightWindow>,
) -> (Vec<StackFamily>, Vec<PathBuf>) {
    let (found, outside) =
        progression_images::target_families_within(night_folder, target_folder, window);
    let mut families: Vec<StackFamily> = Vec::new();
    for family in found {
        let label = family_label(&family);
        let frames = family
            .frame_paths
            .iter()
            .map(PathBuf::from)
            .filter(|p| is_fits(p));
        match families.iter_mut().find(|f| f.label == label) {
            Some(known) => known.frames.extend(frames),
            None => families.push(StackFamily {
                label,
                filter: family.filter.clone(),
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
    let mut outside: Vec<PathBuf> = outside.into_iter().filter(|p| is_fits(p)).collect();
    outside.sort();
    (families, outside)
}

/// A family left out: the space keeps no variant, so one stacking only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetAsideFamily {
    pub label: String,
    pub frames: u64,
}

/// The stackings of a target: one per family (see the module
/// documentation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetPlan {
    pub jobs: Vec<StackJob>,
    pub set_aside: Vec<SetAsideFamily>,
    /// The lights outside the night's window, left out of every job: "n
    /// frames outside the night, not integrated".
    #[serde(default)]
    pub outside: Vec<PathBuf>,
}

impl TargetPlan {
    /// The same plan with `contrast` on every job.
    #[must_use]
    pub fn with_contrast(mut self, contrast: Contrast) -> Self {
        for job in &mut self.jobs {
            job.contrast = contrast;
        }
        self
    }

    /// The same plan with `retouch` (colour intensity, softened stars) on every job.
    #[must_use]
    pub fn with_retouch(mut self, retouch: Retouch) -> Self {
        for job in &mut self.jobs {
            job.retouch = retouch;
        }
        self
    }

    /// The same plan with the frame `frame` on every job (one frame for the
    /// target: each family takes the notch of that rank on its own field).
    /// A rank beyond the notches is Automatic.
    #[must_use]
    pub fn with_frame(mut self, frame: Option<u8>) -> Self {
        let frame = frame.filter(|rank| *rank < FRAME_NOTCHES);
        for job in &mut self.jobs {
            job.frame = frame;
        }
        self
    }

    /// The same plan with `engine` on every job (the "polished render"'s engine).
    #[must_use]
    pub fn with_polished_engine(mut self, engine: PolishedEngine) -> Self {
        for job in &mut self.jobs {
            job.polished_engine = engine;
        }
        self
    }
}

/// [`plan_stack`] for each family of the target's lights.
#[must_use]
pub fn plan_target(
    night_folder: &Path,
    target_folder: &str,
    evening_date: &str,
    pipelines: &[PipelineChoice],
    variant_limit: u64,
) -> TargetPlan {
    plan_target_within(
        night_folder,
        target_folder,
        evening_date,
        pipelines,
        variant_limit,
        None,
    )
}

/// [`plan_target`] with the night's window: only the lights inside it
/// are integrated, each job keeps its family's master.
#[must_use]
pub fn plan_target_within(
    night_folder: &Path,
    target_folder: &str,
    evening_date: &str,
    pipelines: &[PipelineChoice],
    variant_limit: u64,
    window: Option<&NightWindow>,
) -> TargetPlan {
    plan_target_with_device(
        night_folder,
        target_folder,
        evening_date,
        pipelines,
        variant_limit,
        window,
        &[],
    )
}

/// An image of the device taken for a family: its label, the file and what
/// its header says.
struct DeviceSource {
    label: String,
    path: PathBuf,
    info: DeviceImageInfo,
}

/// The files of `<target>/device/` the person chose to take, read again (the
/// header says the family, never the window): a name that is not a plain file
/// name, a file that is not there and an image that is not a device's are
/// left out; a family takes one image, the first named.
fn device_sources(
    night_folder: &Path,
    target_folder: &str,
    uses: &[DeviceUse],
) -> Vec<DeviceSource> {
    let folder = night_folder.join(target_folder).join(DEVICE_DIR);
    let mut sources: Vec<DeviceSource> = Vec::new();
    for item in uses {
        if !device_master::is_plain_file_name(&item.file) {
            continue;
        }
        let path = folder.join(&item.file);
        let Ok(header) = crate::analysis::fits::read_primary_header_file(&path) else {
            continue;
        };
        let Some(info) = device_master::recognise(&header, &item.file) else {
            continue;
        };
        if sources.iter().any(|source| source.label == info.family) {
            continue;
        }
        sources.push(DeviceSource {
            label: info.family.clone(),
            path,
            info,
        });
    }
    sources
}

/// [`plan_target_within`] where the families named by `device` take the image
/// their device stacked: the job of such a family renders that image
/// ([`StackJob::device_master`]) and stacks nothing; a family the device
/// image names but whose poses are not here is a family of its own. The
/// families are ordered by their poses (an image counts its `STACKCNT`) and a
/// space that keeps no variant gets the largest only, the others said. With
/// no `device`, exactly [`plan_target_within`]'s plan.
#[must_use]
pub fn plan_target_with_device(
    night_folder: &Path,
    target_folder: &str,
    evening_date: &str,
    pipelines: &[PipelineChoice],
    variant_limit: u64,
    window: Option<&NightWindow>,
    device: &[DeviceUse],
) -> TargetPlan {
    let mut plan = plan_families(
        night_folder,
        target_folder,
        evening_date,
        pipelines,
        variant_limit,
        window,
        device,
    );
    if let Some(filter) =
        crate::working_folder::read_target_filter(&night_folder.join(target_folder))
    {
        for job in &mut plan.jobs {
            job.capture_filter = Some(filter.clone());
        }
    }
    plan
}

fn plan_families(
    night_folder: &Path,
    target_folder: &str,
    evening_date: &str,
    pipelines: &[PipelineChoice],
    variant_limit: u64,
    window: Option<&NightWindow>,
    device: &[DeviceUse],
) -> TargetPlan {
    let (families, outside) = stack_families_within(night_folder, target_folder, window);
    let taken = device_sources(night_folder, target_folder, device);
    let mut base = plan_stack(night_folder, target_folder, evening_date, pipelines);
    if taken.is_empty() && families.len() <= 1 {
        if let Some(family) = families.first() {
            base = with_family_flats(base, night_folder, family.filter.as_deref());
            base.integration = Some(family.label.clone());
            // Frames were left out: the job lists the ones it stacks.
            if !outside.is_empty() {
                base.frames = family.frames.clone();
            }
        }
        return TargetPlan {
            jobs: vec![base],
            set_aside: Vec::new(),
            outside,
        };
    }
    // The families, each from its poses or from the image of its device.
    enum Origin {
        Poses(StackFamily),
        Device(DeviceSource),
    }
    let mut entries: Vec<(String, u64, Origin)> = Vec::new();
    let mut taken = taken;
    for family in families {
        match taken.iter().position(|d| d.label == family.label) {
            Some(at) => {
                let source = taken.remove(at);
                entries.push((
                    source.label.clone(),
                    source.info.poses,
                    Origin::Device(source),
                ));
            }
            None => entries.push((
                family.label.clone(),
                family.frames.len() as u64,
                Origin::Poses(family),
            )),
        }
    }
    for source in taken {
        entries.push((
            source.label.clone(),
            source.info.poses,
            Origin::Device(source),
        ));
    }
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    if entries.len() == 1 {
        let (label, _, origin) = entries.remove(0);
        match origin {
            Origin::Poses(family) => {
                base = with_family_flats(base, night_folder, family.filter.as_deref());
                base.integration = Some(family.label.clone());
                if !outside.is_empty() {
                    base.frames = family.frames;
                }
            }
            Origin::Device(source) => {
                base.family = Some(label);
                base.device_master = Some(source.path);
            }
        }
        return TargetPlan {
            jobs: vec![base],
            set_aside: Vec::new(),
            outside,
        };
    }
    let keep = if variant_limit == 0 { 1 } else { entries.len() };
    let mut jobs = Vec::new();
    let mut set_aside = Vec::new();
    for (index, (label, size, origin)) in entries.into_iter().enumerate() {
        if index >= keep {
            set_aside.push(SetAsideFamily {
                label,
                frames: size,
            });
            continue;
        }
        let date_family = format!("{evening_date}_{label}");
        let mut job = plan_stack(night_folder, target_folder, &date_family, pipelines);
        job.family = Some(label.clone());
        match origin {
            Origin::Poses(family) => {
                job = with_family_flats(job, night_folder, family.filter.as_deref());
                job.integration = Some(label);
                job.frames = family.frames;
            }
            Origin::Device(source) => job.device_master = Some(source.path),
        }
        jobs.push(job);
    }
    TargetPlan {
        jobs,
        set_aside,
        outside,
    }
}

/// The frame the registered lights are brought onto, chosen by the
/// stacker from the field rotation the registration measured.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Framing {
    /// The area every light covers: the field hardly turns (equatorial
    /// mount, meridian flip included).
    #[default]
    Common,
    /// The whole field of the reference light, its edges covered by fewer
    /// lights: the field turns progressively (alt-azimuth mount).
    Full,
}

/// The notches of the frame of a stacking (`night_framing.NOTCH_COUNT`): the
/// common area, three frames between, the whole field.
pub const FRAME_NOTCHES: u8 = 5;

/// The master of the whole field a stacking keeps beside its renders when
/// the field turns (`<name>_field.fit`, with `<name>_field.json`): another
/// frame is cut from it without stacking again.
pub const FIELD_SUFFIX: &str = "_field.fit";

/// One notch of the frame of a stacking, measured at the registration.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameNotch {
    pub rank: u8,
    pub top: u32,
    pub left: u32,
    pub height: u32,
    pub width: u32,
    /// The share of the whole field it keeps (0 to 1).
    pub share_of_frame: f64,
    /// The share of it every light covers (0 to 1).
    pub full_share: f64,
    /// The lights at its lowest point.
    pub lowest_frames: u32,
}

/// The frame of a stacking (`frame` of the `registration` event and of the
/// provenance): the size of a light, the notches on offer (none when the
/// field does not turn: nothing to choose), the rank Automatic gives and
/// the one cut (`None`: the master kept whole).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameMeasure {
    #[serde(default)]
    pub frame_height: Option<u32>,
    #[serde(default)]
    pub frame_width: Option<u32>,
    #[serde(default)]
    pub notches: Vec<FrameNotch>,
    #[serde(default)]
    pub automatic: Option<u8>,
    #[serde(default)]
    pub rank: Option<u8>,
}

/// What the last stacking of a target measured of its frame, for the
/// « Cadre de l'empilement » of its window: the field rotation and the
/// notches. Read from the records of `<target>/stacked/*_stacking.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetFrame {
    pub field_rotation_degrees: Option<f64>,
    pub registered: u32,
    pub measure: FrameMeasure,
}

/// The frame the stackings of `<night_folder>/<target_folder>` measured;
/// `None` before the first stacking (or with the image of a device, which
/// nobody registered). A target of several families gives the measure of
/// the family with the most lights registered.
#[must_use]
pub fn target_frame(night_folder: &Path, target_folder: &str) -> Option<TargetFrame> {
    let stacked = night_folder.join(target_folder).join(STACKED_DIR);
    let mut best: Option<TargetFrame> = None;
    for entry in fs::read_dir(stacked).ok()?.flatten() {
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with("_stacking.json"))
        {
            continue;
        }
        let Some(registration) = fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|record| record.get("registration").cloned())
        else {
            continue;
        };
        let Some(measure) = registration
            .get("frame")
            .and_then(|frame| serde_json::from_value::<FrameMeasure>(frame.clone()).ok())
        else {
            continue;
        };
        let found = TargetFrame {
            field_rotation_degrees: registration
                .get("fieldRotationDegrees")
                .and_then(serde_json::Value::as_f64),
            registered: registration
                .get("registered")
                .and_then(serde_json::Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(0),
            measure,
        };
        if best
            .as_ref()
            .is_none_or(|b| found.registered > b.registered)
        {
            best = Some(found);
        }
    }
    best
}

/// What the frames of a registration are: the lights of a night, or the
/// masters of a plan's nights (the image of a multi-night plan takes the
/// same decision on them).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegisteredFrames {
    #[default]
    Lights,
    Masters,
}

/// What a stacker reports as it goes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "event",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum StackEvent {
    Step {
        step: String,
        index: u32,
        total: u32,
    },
    /// A file written in `stacked/`.
    Output {
        pipeline: String,
        file: String,
        /// The curve the image carries; « none » for a render.
        #[serde(default, skip_serializing_if = "Contrast::is_none")]
        contrast: Contrast,
        /// The colour intensity the image carries; « none » for a render.
        #[serde(default, skip_serializing_if = "Colour::is_none")]
        colour: Colour,
        /// The softened stars the image carries; « none » for a render.
        #[serde(default, skip_serializing_if = "SoftStars::is_none")]
        soft_stars: SoftStars,
    },
    /// A stacking of the target starts (one per family).
    Family {
        family: Option<String>,
        index: u32,
        total: u32,
    },
    /// What the registration said of the lights, once they are registered:
    /// how far the field turned along the night, the framing that follows,
    /// and the lights Siril could not align (a frame at dawn, a cloud),
    /// left out of the stack and named.
    Registration {
        /// What the frames are: lights, or the masters of a plan.
        #[serde(default)]
        unit: RegisteredFrames,
        frames: u32,
        registered: u32,
        /// The file names of the lights left out (a plan: the nights of the
        /// masters left out).
        #[serde(default)]
        unregistered: Vec<String>,
        /// The span of the field rotation in degrees; `None` when it could
        /// not be measured.
        #[serde(default)]
        field_rotation_degrees: Option<f64>,
        framing: Framing,
        /// The frame of the stacking of a night: its notches, measured from
        /// the transforms (`None`: a plan, or a script that says none).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        frame: Option<FrameMeasure>,
    },
    /// What the combination of a plan's masters said of the frame: the share
    /// of the image (0 to 1) the dual-band masters do not reach, the corners
    /// that keep their colour from the broadband night but have no Hα and
    /// OIII. Only sent when that share is not nil.
    Coverage {
        #[serde(default)]
        no_dual_band_share: f64,
    },
    /// What the image of a plan was cut to (`plan_crop.py`): the largest
    /// axis-aligned rectangle without a pixel no night covers (`clean`), or
    /// that every night covers (`common`). `applied` is false when nothing
    /// was cut (`reason`: the whole field kept on request, nothing to cut, no
    /// common area, a common area too small).
    Crop {
        /// `clean`, `common` or `whole` (`plan_image::PlanField`).
        mode: String,
        applied: bool,
        reason: String,
        top: u32,
        left: u32,
        height: u32,
        width: u32,
        frame_height: u32,
        frame_width: u32,
        /// The share of the frame kept (0 to 1).
        kept_share: f64,
    },
    /// What Siril's journal says it is doing: the last percentage of its
    /// current command and the last useful line, read while it works.
    /// `at`: when `line` was read (Unix milliseconds).
    Progress {
        percent: Option<f64>,
        line: Option<String>,
        at: u64,
    },
}

impl StackEvent {
    /// The words of a [`StackEvent::Progress`] for a text output (the
    /// command line's, like the window's cadre): « 28 % · Integrating… ».
    /// `None` for any other event.
    #[must_use]
    pub fn progress_text(&self) -> Option<String> {
        let Self::Progress { percent, line, .. } = self else {
            return None;
        };
        let percent = percent.map(|p| format!("{p:.0}\u{a0}%"));
        match (percent, line) {
            (Some(percent), Some(line)) => Some(format!("{percent} · {line}")),
            (Some(percent), None) => Some(percent),
            (None, Some(line)) => Some(line.clone()),
            (None, None) => None,
        }
    }
}

/// A render written by the stacker (its file name in `stacked/`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackerOutput {
    pub pipeline: String,
    pub file: String,
    /// The contrast of presentation the image carries: `None` for
    /// the render itself, the curve's choice for the image written beside it.
    pub contrast: Contrast,
    /// The colour intensity and the softened stars it carries.
    pub retouch: Retouch,
}

/// Why the stacking did not give an image (said in words).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "why",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum StackFailure {
    /// The image of a multi-night plan is not in the space's offer
    /// ([`crate::plan_image`]'s gate). The stacking of a night never says it:
    /// it is never gated.
    NotInSubscription,
    /// The pipeline is not ready ("Settings › Dependencies").
    ToolMissing,
    /// Several renders while the space keeps no variant: one is chosen
    /// before the sending.
    OneRenderOnly {
        asked: u32,
    },
    /// No light frame in the target's `lights/`.
    NoLights,
    /// Less free room on the disk of the night than the stacking needs
    /// (about 210 MB per light frame); `folder` is the night's folder, the
    /// disk to free.
    NotEnoughSpace {
        needed: u64,
        available: u64,
        folder: String,
    },
    Cancelled,
    /// A file named outside `<target>/stacked/`: refused, never sent.
    OutsideStacked {
        file: String,
    },
    /// The stacker's own failure: its code (`script_not_trusted`,
    /// `unknown_pipeline`, `siril_failed`…) and its words. `logs` is the
    /// folder of Astro Sync's data where the scratch's journals were kept
    /// before it was removed (see [`KEPT_LOG_FOLDERS`]); `None` when none
    /// were kept.
    Failed {
        code: String,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        logs: Option<String>,
    },
}

/// The slot. The real one is [`PythonStacker`]; tests give a fake.
pub trait Stacker {
    fn stack(
        &self,
        job: &StackJob,
        cancel: &CancelFlag,
        on_event: &mut dyn FnMut(&StackEvent),
    ) -> Result<Vec<StackerOutput>, StackFailure>;
}

/// The conditions of the stacking: never a capability of the offer (the
/// stacking is local and never gated), only the tools and the variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackGate {
    /// The pipeline row of "Dependencies" is present.
    pub tool_present: bool,
    /// `capture.variant_limit`.
    pub variant_limit: u64,
}

/// The unlimited value of a numeric capability (`CAPABILITY_UNLIMITED` of
/// the contract, `Number.MAX_SAFE_INTEGER`).
pub const UNLIMITED: u64 = 9_007_199_254_740_991;

impl StackGate {
    /// The gate of a paired space: its `capture.variant_limit`.
    #[must_use]
    pub fn new(capabilities: &Capabilities, tool_present: bool) -> Self {
        Self {
            tool_present,
            variant_limit: capabilities.capture_variant_limit,
        }
    }

    /// The gate without a space (no account, or its rights not read yet):
    /// nothing leaves, so nothing limits the renders.
    #[must_use]
    pub fn without_space(tool_present: bool) -> Self {
        Self {
            tool_present,
            variant_limit: UNLIMITED,
        }
    }

    /// The gate of `capabilities` when they are known, else
    /// [`Self::without_space`].
    #[must_use]
    pub fn of(capabilities: Option<&Capabilities>, tool_present: bool) -> Self {
        capabilities.map_or_else(
            || Self::without_space(tool_present),
            |capabilities| Self::new(capabilities, tool_present),
        )
    }

    /// The rule that refuses the stacking of `job` before anything runs, if
    /// one does: the pipeline row of "Dependencies" is missing, or several
    /// renders are asked of a space that keeps no variant. [`run_stacking`]
    /// and a dry run ask the same question, so a dry run never says yes to
    /// what the real run refuses.
    #[must_use]
    pub fn refusal(&self, job: &StackJob) -> Option<StackFailure> {
        if !self.tool_present {
            return Some(StackFailure::ToolMissing);
        }
        if job.pipelines.len() > 1 && self.variant_limit == 0 {
            return Some(StackFailure::OneRenderOnly {
                asked: job.pipelines.len() as u32,
            });
        }
        None
    }
}

/// One render written in `stacked/`, with its digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackedRender {
    pub pipeline_id: String,
    /// The contrast of presentation this image carries: the
    /// render itself has none and stays the main image of the capture, the
    /// image with the curve is its variant.
    #[serde(default, skip_serializing_if = "Contrast::is_none")]
    pub contrast: Contrast,
    /// The colour intensity and the softened stars this image carries; none
    /// for the render itself.
    #[serde(default, flatten)]
    pub retouch: Retouch,
    pub path: PathBuf,
    pub sha256: String,
    pub size: u64,
    /// The family of exposures, when the target has several.
    #[serde(default)]
    pub family: Option<String>,
}

/// What [`run_stacking`] gives: the TIFF of each render (the image sent)
/// and the other files written (the PNG to look at).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackOutcome {
    pub renders: Vec<StackedRender>,
    pub other_files: Vec<PathBuf>,
    /// The families not stacked (a space without variants).
    #[serde(default)]
    pub set_aside: Vec<SetAsideFamily>,
    /// The masters kept, one per family stacked.
    #[serde(default)]
    pub integrations: Vec<IntegrationMaster>,
    /// The lights left out because they are outside the night.
    #[serde(default)]
    pub outside_frames: u64,
}

/// About what the scratch folder takes per light frame (3.2 GB for 12
/// colour lights and their calibration; a calibrated and a registered
/// copy in 32 bits).
pub const BYTES_PER_LIGHT: u64 = 210 * 1000 * 1000;

/// The free bytes of the volume holding `path` (`df`; `None` when it cannot
/// be read, on Windows today: the room is then not checked).
#[must_use]
pub fn free_bytes(path: &Path) -> Option<u64> {
    if !cfg!(unix) {
        return None;
    }
    let output = Command::new("df").arg("-Pk").arg(path).output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().nth(1)?;
    let available: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    Some(available * 1024)
}

/// Every stacking of a target ([`plan_target`]), one after the other, under
/// the rules of [`run_stacking`]; the room is checked first, on the disk of
/// `night_folder` where the scratch lives (`free`: the free bytes of a
/// folder, [`free_bytes`] outside the tests). The scratch is gone when this
/// returns, whatever the outcome; one a hard stop of the app left behind in
/// `<night>/.astro-sync-work/stacking/` is removed first, before the room is
/// measured ([`clear_stale_scratch`]). `logs_dir`: see [`run_stacking`].
#[allow(clippy::too_many_arguments)]
pub fn run_target_stacking(
    stacker: &dyn Stacker,
    plan: &TargetPlan,
    night_folder: &Path,
    gate: StackGate,
    free: &dyn Fn(&Path) -> Option<u64>,
    cancel: &CancelFlag,
    logs_dir: Option<&Path>,
    on_event: &mut dyn FnMut(&StackEvent),
) -> Result<StackOutcome, StackFailure> {
    if !gate.tool_present {
        return Err(StackFailure::ToolMissing);
    }
    clear_stale_scratch(night_folder);
    let needed = plan
        .jobs
        .iter()
        .map(|job| job.light_count() * BYTES_PER_LIGHT)
        .max()
        .unwrap_or(0);
    if let Some(available) = free(night_folder)
        && available < needed
    {
        return Err(StackFailure::NotEnoughSpace {
            needed,
            available,
            folder: night_folder.display().to_string(),
        });
    }
    let result = run_plan(stacker, plan, gate, cancel, logs_dir, on_event);
    remove_empty_scratch(night_folder);
    result
}

fn run_plan(
    stacker: &dyn Stacker,
    plan: &TargetPlan,
    gate: StackGate,
    cancel: &CancelFlag,
    logs_dir: Option<&Path>,
    on_event: &mut dyn FnMut(&StackEvent),
) -> Result<StackOutcome, StackFailure> {
    let mut outcome = StackOutcome {
        renders: Vec::new(),
        other_files: Vec::new(),
        set_aside: plan.set_aside.clone(),
        integrations: Vec::new(),
        outside_frames: plan.outside.len() as u64,
    };
    let total = plan.jobs.len() as u32;
    for (index, job) in plan.jobs.iter().enumerate() {
        if cancel.is_cancelled() {
            return Err(StackFailure::Cancelled);
        }
        on_event(&StackEvent::Family {
            family: job.family.clone(),
            index: index as u32 + 1,
            total,
        });
        let done = run_stacking(stacker, job, gate, cancel, logs_dir, on_event)?;
        outcome
            .renders
            .extend(done.renders.into_iter().map(|render| StackedRender {
                family: job.family.clone(),
                ..render
            }));
        outcome.other_files.extend(done.other_files);
        outcome.integrations.extend(done.integrations);
    }
    Ok(outcome)
}

/// Removes `<night>/.astro-sync-work/stacking/` and `.astro-sync-work/`
/// when they hold nothing (`remove_dir` only: a folder with anything in it
/// stays).
fn remove_empty_scratch(night_folder: &Path) {
    let _ = fs::remove_dir(stacking_scratch_root(night_folder));
    let _ = fs::remove_dir(night_folder.join(WORK_DIR));
}

/// Removes what a stacking left in `<night>/.astro-sync-work/stacking/`
/// (a hard stop of the app: the scratch is otherwise removed at the end of
/// each job), then the hidden folder if it is empty. Nothing else of
/// `.astro-sync-work/` is touched.
pub fn clear_stale_scratch(night_folder: &Path) {
    let _ = fs::remove_dir_all(stacking_scratch_root(night_folder));
    let _ = fs::remove_dir(night_folder.join(WORK_DIR));
}

/// The folder of Astro Sync's data (`<data>/stacking-logs/`) where the
/// journals of a failed or stopped stacking are kept.
pub const LOGS_DIR: &str = "stacking-logs";

/// How many folders of journals Astro Sync keeps in its data
/// (`stacking-logs/`): the most recent ones.
pub const KEPT_LOG_FOLDERS: usize = 10;

/// `YYYYMMDD-HHMMSS` of now, in UTC, for the name of a journals folder.
fn log_stamp() -> String {
    let iso = crate::time_util::system_time_to_iso8601_utc(std::time::SystemTime::now());
    // 2026-10-12T14:03:09Z
    iso.chars()
        .filter(|c| c.is_ascii_digit() || *c == 'T')
        .map(|c| if c == 'T' { '-' } else { c })
        .collect()
}

/// The `YYYYMMDD-HHMMSS` a journals folder name ends with, ignoring a
/// `-2`, `-3`… added when the name was taken.
fn log_stamp_of(name: &str) -> Option<&str> {
    let stamp_at_end = |text: &str| -> Option<usize> {
        let start = text.len().checked_sub(15)?;
        let tail = text.get(start..)?;
        let ok = tail.bytes().enumerate().all(|(i, b)| match i {
            8 => b == b'-',
            _ => b.is_ascii_digit(),
        });
        ok.then_some(start)
    };
    if let Some(start) = stamp_at_end(name) {
        return Some(&name[start..]);
    }
    let (head, count) = name.rsplit_once('-')?;
    if !count.bytes().all(|b| b.is_ascii_digit()) || count.is_empty() {
        return None;
    }
    stamp_at_end(head).map(|start| &head[start..])
}

/// Copies the journals of a job's scratch (`siril.log`, the script's own
/// `render.log`, any other `*.log` or `*.txt`, never an image) into a new
/// folder `<logs_dir>/<job name>-<YYYYMMDD-HHMMSS>/`, then keeps only the
/// [`KEPT_LOG_FOLDERS`] most recent folders. Gives that folder, or `None`
/// when the scratch holds no journal or it cannot be written (the stacking's
/// own failure is what the person needs to read, not this one).
pub fn keep_logs(job: &StackJob, logs_dir: &Path) -> Option<PathBuf> {
    let journals: Vec<PathBuf> = walkdir::WalkDir::new(&job.work_dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_file()
                && entry
                    .path()
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "log" | "txt"))
        })
        .map(walkdir::DirEntry::into_path)
        .collect();
    if journals.is_empty() {
        return None;
    }
    let stem = format!("{}-{}", job.name, log_stamp());
    let mut folder = logs_dir.join(&stem);
    let mut n = 2;
    while folder.exists() {
        folder = logs_dir.join(format!("{stem}-{n}"));
        n += 1;
    }
    let mut copied = false;
    for journal in &journals {
        let Ok(relative) = journal.strip_prefix(&job.work_dir) else {
            continue;
        };
        let target = folder.join(relative);
        if let Some(parent) = target.parent()
            && fs::create_dir_all(parent).is_err()
        {
            continue;
        }
        copied |= fs::copy(journal, &target).is_ok();
    }
    prune_logs(logs_dir, KEPT_LOG_FOLDERS);
    copied.then_some(folder)
}

/// Keeps the `keep` most recent journals folders of `logs_dir` (by the stamp
/// in their names; a folder without one is not ours and stays).
pub fn prune_logs(logs_dir: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(logs_dir) else {
        return;
    };
    let mut folders: Vec<(String, String, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let stamp = log_stamp_of(&name)?.to_string();
            Some((stamp, name, entry.path()))
        })
        .collect();
    folders.sort();
    let excess = folders.len().saturating_sub(keep);
    for (_, _, path) in folders.into_iter().take(excess) {
        let _ = fs::remove_dir_all(path);
    }
}

/// Removes a job's scratch folder and its list of frames, if they lie in
/// `.astro-sync-work/` (never anything else).
fn remove_job_scratch(job: &StackJob) {
    let in_work_dir = job
        .work_dir
        .components()
        .any(|part| part.as_os_str() == WORK_DIR);
    if !in_work_dir {
        return;
    }
    let _ = fs::remove_dir_all(&job.work_dir);
    let _ = fs::remove_file(job.frames_list());
}

fn has_lights(lights: &Path) -> bool {
    walkdir::WalkDir::new(lights)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .any(|entry| {
            entry.file_type().is_file()
                && entry
                    .path()
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| {
                        matches!(
                            e.to_ascii_lowercase().as_str(),
                            "fit" | "fits" | "fts" | "xisf"
                        ) || crate::raw_header::is_raw_extension(e)
                    })
        })
}

pub(crate) fn sha256_and_size(path: &Path) -> std::io::Result<(String, u64)> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut size = 0;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        hasher.update(&buffer[..n]);
    }
    Ok((format!("{:x}", hasher.finalize()), size))
}

/// Runs the stacking of `job` under its rules (see the module
/// documentation). Nothing runs when a rule refuses it. `logs_dir`: where
/// the journals of a failed or stopped stacking are kept before its scratch
/// is removed ([`keep_logs`]); `None` keeps nothing.
pub fn run_stacking(
    stacker: &dyn Stacker,
    job: &StackJob,
    gate: StackGate,
    cancel: &CancelFlag,
    logs_dir: Option<&Path>,
    on_event: &mut dyn FnMut(&StackEvent),
) -> Result<StackOutcome, StackFailure> {
    if let Some(refusal) = gate.refusal(job) {
        return Err(refusal);
    }
    match &job.device_master {
        // The image of a device reads no pose: it must be there.
        Some(image) if !image.is_file() => {
            return Err(StackFailure::Failed {
                code: "folder_missing".to_string(),
                message: format!("the image of the device is not there: {}", image.display()),
                logs: None,
            });
        }
        Some(_) => {}
        None if !has_lights(&job.lights) => return Err(StackFailure::NoLights),
        None => {}
    }
    fs::create_dir_all(&job.output_dir).map_err(|e| StackFailure::Failed {
        code: "io_error".to_string(),
        message: e.to_string(),
        logs: None,
    })?;
    let stacked_by = stacker.stack(job, cancel, on_event);
    // A failure or a stop: the journals are copied out first. A success
    // keeps nothing.
    let logs = match (&stacked_by, logs_dir) {
        (Err(_), Some(dir)) => keep_logs(job, dir),
        _ => None,
    };
    // Success, failure or stop: the scratch is the job's, and it ends here.
    remove_job_scratch(job);
    let outputs = stacked_by.map_err(|failure| match failure {
        StackFailure::Failed { code, message, .. } => StackFailure::Failed {
            code,
            message,
            logs: logs.map(|dir| dir.display().to_string()),
        },
        other => other,
    })?;
    let stacked = fs::canonicalize(&job.output_dir).map_err(|e| StackFailure::Failed {
        code: "io_error".to_string(),
        message: e.to_string(),
        logs: None,
    })?;
    let mut outcome = StackOutcome {
        renders: Vec::new(),
        other_files: Vec::new(),
        set_aside: Vec::new(),
        integrations: Vec::new(),
        outside_frames: 0,
    };
    for output in outputs {
        let path = job.output_dir.join(&output.file);
        let inside = !output.file.contains(['/', '\\'])
            && fs::canonicalize(&path).is_ok_and(|p| p.parent() == Some(stacked.as_path()));
        if !inside {
            return Err(StackFailure::OutsideStacked { file: output.file });
        }
        let is_tiff = Path::new(&output.file)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "tif" | "tiff"));
        if !is_tiff {
            outcome.other_files.push(path);
            continue;
        }
        let (sha256, size) = sha256_and_size(&path).map_err(|e| StackFailure::Failed {
            code: "io_error".to_string(),
            message: e.to_string(),
            logs: None,
        })?;
        outcome.renders.push(StackedRender {
            pipeline_id: output.pipeline,
            contrast: output.contrast,
            retouch: output.retouch,
            path,
            sha256,
            size,
            family: job.family.clone(),
        });
    }
    if outcome.renders.is_empty() {
        return Err(StackFailure::Failed {
            code: "no_output".to_string(),
            message: "the stacking wrote no image".to_string(),
            logs: None,
        });
    }
    // The master kept: described from its own header.
    if let Some(family) = &job.integration
        && let Some(master) = multi_night::read_master(&integration_path(job, family))
    {
        outcome.integrations.push(master);
    }
    Ok(outcome)
}

/// The suffix of a master kept: `<family>_integration.fit`.
pub const INTEGRATION_SUFFIX: &str = "_integration.fit";

/// Where the script keeps the master of `family` for `job`.
#[must_use]
pub fn integration_path(job: &StackJob, family: &str) -> PathBuf {
    job.output_dir.join(format!("{family}{INTEGRATION_SUFFIX}"))
}

/// The renders as images that could leave, in the order of the pipelines
/// chosen (the first is the capture or its variant, the others variants).
#[must_use]
pub fn stacked_candidates(outcome: &StackOutcome, target_id: &str) -> Vec<ImageCandidate> {
    // The rank of a render is the place of its pipeline among the pipelines
    // asked for (the same for the image with a curve and the one without).
    let mut asked: Vec<&str> = Vec::new();
    for render in &outcome.renders {
        if !asked.contains(&render.pipeline_id.as_str()) {
            asked.push(&render.pipeline_id);
        }
    }
    outcome
        .renders
        .iter()
        .map(|render| ImageCandidate {
            target_id: target_id.to_string(),
            path: render.path.clone(),
            sha256: render.sha256.clone(),
            size: render.size,
            kind: ResultKind::Stacked,
            render: Some(render.pipeline_id.clone()),
            render_rank: asked
                .iter()
                .position(|id| *id == render.pipeline_id)
                .and_then(|place| u32::try_from(place).ok()),
            contrast: render.contrast,
            retouch: render.retouch,
            frame_count: None,
        })
        .collect()
}

/* ------------------------- The stacking script ---------------------------- */

/// `astro_sync_stack.py` in its Python environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PythonStacker {
    pub python: PathBuf,
    pub script: PathBuf,
    /// `<Astro Sync data>/pipelines`, the person's definitions.
    pub pipelines_dirs: Vec<PathBuf>,
    pub siril: Option<PathBuf>,
    /// StarNet2 where "Dependencies" found it (a window of macOS does not
    /// inherit the `PATH` of a terminal: the script would not find it, and
    /// the render would differ).
    pub starnet: Option<PathBuf>,
    /// GraXpert where "Dependencies" found it, for the "polished render"
    /// (same reason as StarNet2).
    pub graxpert: Option<PathBuf>,
    pub timeout: Duration,
}

/// How long the script is given to stop Siril and say it stopped.
pub const STOP_GRACE: Duration = Duration::from_secs(30);

/// The script's command line for `job`.
#[must_use]
pub fn stack_args(
    job: &StackJob,
    pipelines_dirs: &[PathBuf],
    siril: Option<&Path>,
    starnet: Option<&Path>,
    graxpert: Option<&Path>,
) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(image) = &job.device_master {
        // The image a device stacked: nothing to stack, no pose, no
        // calibration.
        args.extend(["--from-master".to_string(), image.display().to_string()]);
    } else {
        args.extend(["--lights".to_string(), job.lights.display().to_string()]);
        if !job.frames.is_empty() {
            args.extend([
                "--frames-from".to_string(),
                job.frames_list().display().to_string(),
            ]);
        }
        for (flag, folder) in [
            ("--darks", &job.darks),
            ("--flats", &job.flats),
            ("--dark-flats", &job.dark_flats),
            ("--biases", &job.biases),
        ] {
            if let Some(folder) = folder {
                args.extend([flag.to_string(), folder.display().to_string()]);
            }
        }
    }
    args.extend([
        "--work".to_string(),
        job.work_dir.display().to_string(),
        "--out".to_string(),
        job.output_dir.display().to_string(),
        "--name".to_string(),
        job.name.clone(),
    ]);
    if let Some(family) = &job.integration {
        args.extend(["--integration".to_string(), family.clone()]);
    }
    // The frame chosen by hand; the image of a device is registered by
    // nobody: it has none.
    if let Some(rank) = job.frame.filter(|_| job.device_master.is_none()) {
        args.extend(["--frame-size".to_string(), rank.to_string()]);
    }
    let linear_only = job.is_linear_only();
    // "No processing" has no twin: the curve, the colours and the stars do
    // nothing visible on a linear image.
    if !job.contrast.is_none() && !linear_only {
        args.extend(["--contrast".to_string(), job.contrast.token().to_string()]);
    }
    if !job.retouch.colour.is_none() && !linear_only {
        args.extend([
            "--colour".to_string(),
            job.retouch.colour.token().to_string(),
        ]);
    }
    if !job.retouch.soft_stars.is_none() && !linear_only {
        args.extend([
            "--soft-stars".to_string(),
            job.retouch.soft_stars.token().to_string(),
        ]);
    }
    if let Some(filter) = &job.capture_filter {
        args.extend(["--capture-filter".to_string(), filter.clone()]);
    }
    if !job.polished_engine.is_default() && !linear_only {
        args.extend([
            "--polished-engine".to_string(),
            job.polished_engine.as_str().to_string(),
        ]);
    }
    if linear_only {
        // No render: no pipeline, no definition folder, no StarNet2.
        args.push("--linear-only".to_string());
    } else {
        let renders = job.pipelines.iter().filter(|p| !p.is_linear());
        for pipeline in renders.clone() {
            args.extend(["--pipeline".to_string(), pipeline.pipeline_id.clone()]);
        }
        // The finish of a pipeline that has its own (« Finition » of its card):
        // the others keep the job's, the default of the night.
        let default = PipelineFinish {
            contrast: job.contrast,
            retouch: job.retouch,
        };
        for pipeline in renders.clone() {
            if let Some(finish) = pipeline.finish.filter(|finish| *finish != default) {
                args.extend([
                    "--finish".to_string(),
                    format!("{}={}", pipeline.pipeline_id, finish.tokens()),
                ]);
            }
        }
        // "No processing" beside the renders: the linear
        // stack is written too.
        if job.adds_linear() {
            args.push("--linear".to_string());
        }
        for pipeline in renders {
            if let Some(sha256) = &pipeline.trusted_sha256 {
                args.extend([
                    "--trusted".to_string(),
                    format!("{}={}", pipeline.pipeline_id, sha256.to_ascii_lowercase()),
                ]);
            }
        }
        for dir in pipelines_dirs {
            args.extend(["--pipelines-dir".to_string(), dir.display().to_string()]);
        }
    }
    if let Some(siril) = siril {
        args.extend(["--siril".to_string(), siril.display().to_string()]);
    }
    if let Some(starnet) = starnet.filter(|_| !linear_only) {
        args.extend(["--starnet".to_string(), starnet.display().to_string()]);
    }
    if let Some(graxpert) = graxpert.filter(|_| !linear_only) {
        args.extend(["--graxpert".to_string(), graxpert.display().to_string()]);
    }
    args.push("--json".to_string());
    args
}

/// The kind of a failure the script gave no code for, from its exit code.
#[must_use]
pub fn failure_code_of_exit(code: Option<i32>) -> &'static str {
    match code {
        Some(2) => "bad_input",
        Some(3) => "tool_missing",
        Some(4) => "tool_failed",
        _ => "stacking_failed",
    }
}

/// What the loop that waits for the stacking script holds: where the events
/// go, what the script said, and the reading of Siril's journal.
struct StackWatch<'a> {
    on_event: &'a mut dyn FnMut(&StackEvent),
    outputs: Vec<StackerOutput>,
    error: Option<(String, String)>,
    follower: SirilFollower,
    tail: LogTail,
}

impl StackWatch<'_> {
    /// Reads the journal and says what is new.
    fn poll(&mut self) {
        let now = now_ms();
        self.tail.poll(&mut self.follower, now);
        if let Some(report) = self.follower.take(now) {
            (self.on_event)(&StackEvent::Progress {
                percent: report.percent,
                line: report.line,
                at: report.at,
            });
        }
    }

    /// One line of the script's own JSON on its standard output.
    fn line(&mut self, line: &str) {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            return;
        };
        let text = |key: &str| {
            event
                .get(key)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        match event.get("event").and_then(|e| e.as_str()) {
            Some("step") => {
                // What the previous command wrote is read before its
                // percentage is dropped: the new step starts from none.
                self.poll();
                self.follower.restart_percent();
                (self.on_event)(&StackEvent::Step {
                    step: text("step"),
                    index: event
                        .get("index")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0) as u32,
                    total: event
                        .get("total")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0) as u32,
                });
            }
            Some("output") => {
                let output = StackerOutput {
                    pipeline: text("pipeline"),
                    file: text("file"),
                    contrast: text("contrast").parse().unwrap_or_default(),
                    retouch: Retouch {
                        colour: text("colour").parse().unwrap_or_default(),
                        soft_stars: text("softStars").parse().unwrap_or_default(),
                    },
                };
                (self.on_event)(&StackEvent::Output {
                    pipeline: output.pipeline.clone(),
                    file: output.file.clone(),
                    contrast: output.contrast,
                    colour: output.retouch.colour,
                    soft_stars: output.retouch.soft_stars,
                });
                self.outputs.push(output);
            }
            Some("registration") => {
                let count = |key: &str| {
                    event
                        .get(key)
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0) as u32
                };
                (self.on_event)(&StackEvent::Registration {
                    unit: if text("unit") == "masters" {
                        RegisteredFrames::Masters
                    } else {
                        RegisteredFrames::Lights
                    },
                    frames: count("frames"),
                    registered: count("registered"),
                    unregistered: event
                        .get("unregistered")
                        .and_then(|v| v.as_array())
                        .map(|names| {
                            names
                                .iter()
                                .filter_map(|n| n.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default(),
                    field_rotation_degrees: event
                        .get("fieldRotationDegrees")
                        .and_then(serde_json::Value::as_f64),
                    framing: if text("framing") == "full" {
                        Framing::Full
                    } else {
                        Framing::Common
                    },
                    frame: event
                        .get("frame")
                        .and_then(|frame| serde_json::from_value(frame.clone()).ok()),
                });
            }
            Some("crop") => {
                let number = |key: &str| {
                    event
                        .get(key)
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0) as u32
                };
                (self.on_event)(&StackEvent::Crop {
                    mode: text("mode"),
                    applied: event
                        .get("applied")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                    reason: text("reason"),
                    top: number("top"),
                    left: number("left"),
                    height: number("height"),
                    width: number("width"),
                    frame_height: number("frameHeight"),
                    frame_width: number("frameWidth"),
                    kept_share: event
                        .get("keptShare")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(1.0),
                });
            }
            Some("coverage") => {
                (self.on_event)(&StackEvent::Coverage {
                    no_dual_band_share: event
                        .get("noDualBandShare")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0),
                });
            }
            Some("error") => self.error = Some((text("code"), text("message"))),
            _ => {}
        }
    }
}

impl Stacker for PythonStacker {
    fn stack(
        &self,
        job: &StackJob,
        cancel: &CancelFlag,
        on_event: &mut dyn FnMut(&StackEvent),
    ) -> Result<Vec<StackerOutput>, StackFailure> {
        let list = job.frames_list();
        if !job.frames.is_empty() {
            let text: String = job
                .frames
                .iter()
                .map(|frame| format!("{}\n", frame.display()))
                .collect();
            if let Some(parent) = list.parent() {
                let _ = fs::create_dir_all(parent);
            }
            fs::write(&list, text).map_err(|e| StackFailure::Failed {
                code: "io_error".to_string(),
                message: e.to_string(),
                logs: None,
            })?;
        }
        let mut command = python_env::script_command(&self.python, &self.script);
        command.args(stack_args(
            job,
            &self.pipelines_dirs,
            self.siril.as_deref(),
            self.starnet.as_deref(),
            self.graxpert.as_deref(),
        ));
        let result = run_script(command, &job.work_dir, self.timeout, cancel, on_event);
        if !job.frames.is_empty() {
            let _ = fs::remove_file(&list);
        }
        result
    }
}

/// Runs a script of the stacking environment that speaks the events of
/// `astro_sync_stack.py` (`step`, `output`, `error` on its standard output,
/// one JSON object per line; Siril's journal in `<work_dir>/siril.log`):
/// the stacking, and the image of a multi-night plan (`plan_stack.py`,
/// [`crate::plan_image`]). Gives the files it wrote, or its failure in its
/// own code.
pub(crate) fn run_script(
    command: Command,
    work_dir: &Path,
    timeout: Duration,
    cancel: &CancelFlag,
    on_event: &mut dyn FnMut(&StackEvent),
) -> Result<Vec<StackerOutput>, StackFailure> {
    // The reading of Siril's journal rides on the loop that waits for the
    // script: every `POLL_EVERY` the lines written since are read and
    // the news goes out as a `Progress` event.
    let state = RefCell::new(StackWatch {
        on_event,
        outputs: Vec::new(),
        error: None,
        follower: SirilFollower::new(),
        tail: LogTail::new(&work_dir.join("siril.log")),
    });
    let run = external_process::run_polled(
        command,
        timeout,
        Some(cancel),
        Some(STOP_GRACE),
        POLL_EVERY,
        &mut |line| state.borrow_mut().line(line),
        &mut || state.borrow_mut().poll(),
    );
    state.borrow_mut().poll();
    let StackWatch { outputs, error, .. } = state.into_inner();
    let end = run.map_err(|e| StackFailure::Failed {
        code: "not_started".to_string(),
        message: e.to_string(),
        logs: None,
    })?;
    if end.cancelled || end.code() == Some(130) {
        return Err(StackFailure::Cancelled);
    }
    if end.timed_out {
        return Err(StackFailure::Failed {
            code: "timeout".to_string(),
            message: format!("stopped after {} s", timeout.as_secs()),
            logs: None,
        });
    }
    if let Some((code, message)) = error {
        return Err(if code == "cancelled" {
            StackFailure::Cancelled
        } else {
            StackFailure::Failed {
                code,
                message,
                logs: None,
            }
        });
    }
    if !end.succeeded() {
        return Err(StackFailure::Failed {
            code: failure_code_of_exit(end.code()).to_string(),
            message: end.stderr.trim().chars().take(300).collect(),
            logs: None,
        });
    }
    Ok(outputs)
}

#[cfg(test)]
mod tests;

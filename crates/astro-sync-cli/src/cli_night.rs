//! The arguments of the commands of the rewritten journey: one command per
//! step of the window (`README.md`, "CLI / GUI parity"), each with
//! `--json`, and `--dry-run` wherever it would write or send something.

use clap::Args;
use std::path::PathBuf;

/// The service and the token of this computer.
#[derive(Args, Debug, Clone)]
pub struct ServiceArgs {
    /// Base URL of the service.
    #[arg(long)]
    pub portal: String,
    /// File containing the device token (never an argument that gets displayed).
    #[arg(long)]
    pub token: PathBuf,
}

/// A night: its folder, its targets.
#[derive(Args, Debug, Clone)]
pub struct NightArgs {
    /// Folder of the night (read-only).
    #[arg(long)]
    pub folder: PathBuf,
    /// Ticked target of the night: IDENTIFIER=DESIGNATION (repeatable), e.g. t-m31=M31.
    #[arg(long = "target", value_name = "ID=DESIGNATION")]
    pub targets: Vec<String>,
    /// Evening date of the night (YYYY-MM-DD).
    #[arg(long)]
    pub evening_date: Option<String>,
    /// Start of the night, ISO 8601 UTC: read only without an evening date, to find it.
    #[arg(long)]
    pub night_started_at: Option<String>,
    /// Longitude of the site, degrees east: the night runs from local noon to local noon the next day (without it: UTC).
    #[arg(long, allow_hyphen_values = true)]
    pub longitude: Option<f64>,
}

#[derive(Args, Debug)]
pub struct WorkingFolderArgs {
    /// Working folder (default: the system's, Documents/Astro Sync).
    #[arg(long)]
    pub root: Option<PathBuf>,
    /// Astro Sync data (keeps the fingerprint of ASSISTANT-IA.md there).
    #[arg(long)]
    pub data: Option<PathBuf>,
    /// Create the working folder and ASSISTANT-IA.md.
    #[arg(long)]
    pub prepare: bool,
    /// ASSISTANT-IA.md with the "Multi-night plans" section (multi-night active for the space).
    #[arg(long)]
    pub multi_night: bool,
    /// Say what would be done, without writing anything.
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct NightsArgs {
    /// Working folder (default: the system's).
    #[arg(long)]
    pub root: Option<PathBuf>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct NightFolderArgs {
    #[arg(long)]
    pub root: Option<PathBuf>,
    /// Identifier of the night.
    #[arg(long)]
    pub night_id: String,
    /// URL of the service, written to the identity file.
    #[arg(long)]
    pub service_url: String,
    /// Evening date (YYYY-MM-DD).
    #[arg(long)]
    pub evening_date: String,
    /// Plan, or site outside a plan: the middle part of the name.
    #[arg(long)]
    pub label: String,
    /// Target: IDENTIFIER=DESIGNATION, ticked (repeatable).
    #[arg(long = "target", value_name = "ID=DESIGNATION")]
    pub targets: Vec<String>,
    /// Unticked target of the night (no folder created for it).
    #[arg(long = "unticked", value_name = "ID=DESIGNATION")]
    pub unticked: Vec<String>,
    /// Two folders carry the night: the one to follow (it must carry the night).
    #[arg(long, value_name = "DOSSIER")]
    pub follow: Option<PathBuf>,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct CheckArgs {
    #[command(flatten)]
    pub service: ServiceArgs,
    #[command(flatten)]
    pub night: NightArgs,
    /// Identifier of the night (the run) online.
    #[arg(long)]
    pub session_id: String,
    /// The pipeline whose render is the main image of each capture that has one (default: the
    /// first pipeline the stacking was asked for).
    #[arg(long, value_name = "ID")]
    pub main: Option<String>,
    /// Join the TIFF of the renders (full resolution) to the sending; without it, a render written as
    /// PNG and TIFF leaves as its PNG, and a render that exists only as a TIFF (`linear`) as its TIFF.
    #[arg(long)]
    pub with_tiff: bool,
    /// Language of the titles and descriptions proposed for a render (fr, en, ja...).
    #[arg(long, default_value = "en")]
    pub lang: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct SendNightArgs {
    #[command(flatten)]
    pub service: ServiceArgs,
    #[command(flatten)]
    pub night: NightArgs,
    #[arg(long)]
    pub session_id: String,
    /// The pipeline whose render is the main image of each capture that has one (default: the
    /// first pipeline the stacking was asked for).
    #[arg(long, value_name = "ID")]
    pub main: Option<String>,
    /// Join the TIFF of the renders (full resolution) to the sending; without it, a render written as
    /// PNG and TIFF leaves as its PNG, and a render that exists only as a TIFF (`linear`) as its TIFF.
    #[arg(long)]
    pub with_tiff: bool,
    /// Upload queue (SQLite).
    #[arg(long)]
    pub queue: PathBuf,
    /// Astro Sync data (originals, solvers, register of the nights).
    #[arg(long)]
    pub data: PathBuf,
    /// Analyse the logs and send the report (it replaces the online analysis).
    #[arg(long)]
    pub logs: bool,
    /// UTC offset of the PHD2 logs (e.g. "+02:00"); absent: found from the
    /// `DATE-OBS` of the frames.
    #[arg(long)]
    pub phd2_utc_offset: Option<String>,
    /// UTC offset of the ASIAIR logs (e.g. "+02:00"); absent: found from
    /// the `DATE-OBS` of the frames.
    #[arg(long)]
    pub asiair_utc_offset: Option<String>,
    /// UTC offset of the NINA logs (e.g. "+02:00"); absent: found from the
    /// `DATE-OBS` of the frames.
    #[arg(long)]
    pub nina_utc_offset: Option<String>,
    /// Compute and send the missing sky solutions.
    #[arg(long)]
    pub solve: bool,
    /// Solver: astap (default) or astrometry-net.
    #[arg(long, default_value = "astap")]
    pub solver: String,
    /// Language of the titles and descriptions proposed for a render (fr, en, ja...).
    #[arg(long, default_value = "en")]
    pub lang: String,
    /// The summary only: nothing is queued or sent.
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct SendLogsArgs {
    /// Folder of the night (read-only).
    #[arg(long)]
    pub folder: PathBuf,
    /// Identifier of the night online (unnecessary with --dry-run).
    #[arg(long)]
    pub session_id: Option<String>,
    #[arg(long)]
    pub portal: Option<String>,
    #[arg(long)]
    pub token: Option<PathBuf>,
    /// Astro Sync data (register of the nights).
    #[arg(long)]
    pub data: Option<PathBuf>,
    /// Reference star list, for the measured sky background.
    #[arg(long)]
    pub stars: Option<PathBuf>,
    /// UTC offset of the PHD2 logs (e.g. "+02:00"); absent: found from the
    /// `DATE-OBS` of the frames.
    #[arg(long)]
    pub phd2_utc_offset: Option<String>,
    /// UTC offset of the ASIAIR logs (e.g. "+02:00"); absent: found from
    /// the `DATE-OBS` of the frames.
    #[arg(long)]
    pub asiair_utc_offset: Option<String>,
    /// UTC offset of the NINA logs (e.g. "+02:00"); absent: found from the
    /// `DATE-OBS` of the frames.
    #[arg(long)]
    pub nina_utc_offset: Option<String>,
    /// The report only, nothing is sent.
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub language_of_messages: crate::cli::LangArg,
}

#[derive(Args, Debug)]
pub struct SolveArgs {
    /// An image to solve (TIFF, JPEG or PNG).
    #[arg(long)]
    pub image: Option<PathBuf>,
    /// The capture of this image, to send the solution.
    #[arg(long)]
    pub capture_id: Option<String>,
    /// All the captures of the space without a sky solution.
    #[arg(long)]
    pub missing: bool,
    #[arg(long)]
    pub portal: Option<String>,
    #[arg(long)]
    pub token: Option<PathBuf>,
    /// Astro Sync data (tools, originals, solver work).
    #[arg(long)]
    pub data: PathBuf,
    /// Solver: astap (default) or astrometry-net.
    #[arg(long, default_value = "astap")]
    pub solver: String,
    /// Right ascension of the target, degrees (hint for the solver).
    #[arg(long)]
    pub ra: Option<f64>,
    /// Declination of the target, degrees.
    #[arg(long)]
    pub dec: Option<f64>,
    /// Scale of the image, arcseconds per pixel.
    #[arg(long)]
    pub scale: Option<f64>,
    /// With --missing: the working folder. A capture whose image is there (found by its
    /// SHA-256) is solved from that file, pointed where its master points and with its
    /// pixel scale (RA, DEC, XPIXSZ and FOCALLEN of the header): a very wide field is not
    /// solved blind.
    #[arg(long)]
    pub root: Option<PathBuf>,
    /// Solve without sending (one image), or list without solving (--missing).
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub language_of_messages: crate::cli::LangArg,
}

/// `image-texts`: the generated title and description of the images of
/// captures already online, in the ten languages (`astro_sync_core::image_texts`).
#[derive(Args, Debug)]
pub struct ImageTextsArgs {
    /// A capture whose images to name (repeatable).
    #[arg(long = "capture-id", required = true)]
    pub capture_ids: Vec<String>,
    /// The working folder: the records Astro Sync wrote beside its renders
    /// (`*_stacking.json`, `*_plan.json`) are read under it, each image found by its SHA-256.
    #[arg(long)]
    pub root: PathBuf,
    /// Astro Sync data (the person's own pipelines, under `pipelines/`).
    #[arg(long)]
    pub data: Option<PathBuf>,
    #[arg(long)]
    pub portal: Option<String>,
    #[arg(long)]
    pub token: Option<PathBuf>,
    /// Say what would be sent, send nothing.
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ProgressionImagesArgs {
    /// Folder of the night (read-only).
    #[arg(long)]
    pub night: PathBuf,
    /// Identifier of the night.
    #[arg(long)]
    pub night_id: String,
    /// Folder of the target in the night ("M31").
    #[arg(long)]
    pub target_folder: String,
    /// Key of the family (default: the most numerous).
    #[arg(long)]
    pub family: Option<String>,
    /// Filter of the lights when their headers name none (an ASIAIR writes none), e.g.
    /// "Dualband", "UV/IR cut": recorded in the target folder's identity (the lights are
    /// never written), then used by the stacking, the master it keeps and the plans.
    #[arg(long, value_name = "NAME")]
    pub filter: Option<String>,
    /// Astro Sync data (the images are written there).
    #[arg(long)]
    pub data: PathBuf,
    /// Read `analysis.progression` on this service.
    #[arg(long)]
    pub portal: Option<String>,
    #[arg(long)]
    pub token: Option<PathBuf>,
    /// The families and the script only, nothing is run.
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub language_of_messages: crate::cli::LangArg,
}

#[derive(Args, Debug)]
pub struct ToolsArgs {
    /// Astro Sync data (downloaded tools, Tycho-2 list, stacking-env/ environment).
    #[arg(long)]
    pub data: PathBuf,
    #[command(flatten)]
    pub language_of_messages: crate::cli::LangArg,
    /// Folder of the stacking scripts (stacking).
    #[arg(long)]
    pub stacking: Option<PathBuf>,
    /// "Install all": download the missing required tools and create the Python environment.
    #[arg(long)]
    pub install: bool,
    /// "Check again": check the Python environment (check_environment.py).
    #[arg(long)]
    pub verify: bool,
    /// With --install: say what would be downloaded.
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ListPipelinesArgs {
    /// Astro Sync data (pipelines added in pipelines/, stacking-env/ environment).
    #[arg(long)]
    pub data: Option<PathBuf>,
    /// Folder of the stacking scripts, for the script to list when the environment is there.
    #[arg(long)]
    pub stacking: Option<PathBuf>,
    /// Folder of the shipped pipelines (default: the ones Astro Sync embeds).
    #[arg(long)]
    pub shipped: Option<PathBuf>,
    #[arg(long, default_value = "en")]
    pub lang: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct StackArgs {
    /// Folder of the night.
    #[arg(long)]
    pub night: PathBuf,
    /// Folder of the target in the night ("M31").
    #[arg(long)]
    pub target_folder: String,
    /// Identifier of the target (for the upload).
    #[arg(long)]
    pub target_id: String,
    /// Evening date (YYYY-MM-DD), in the file names.
    #[arg(long)]
    pub evening_date: String,
    /// Longitude of the site, degrees east: frames outside the night (local noon → local noon) are not integrated (without it: UTC).
    #[arg(long, allow_hyphen_values = true)]
    pub longitude: Option<f64>,
    /// Rendering pipeline (repeatable; default: raw; `linear` = "No processing").
    #[arg(long = "pipeline", value_name = "ID")]
    pub pipelines: Vec<String>,
    /// Pipelines of a target: TARGET=ID,ID (folder or identifier of the target; repeatable). For the
    /// stacked target, they replace --pipeline; those of the other targets are ignored.
    #[arg(long = "target-pipeline", value_name = "TARGET=ID,ID")]
    pub target_pipelines: Vec<String>,
    /// The pipeline whose render is the capture's main image (the others are its variants):
    /// it is made first. Without it, the first --pipeline asked is the main image.
    #[arg(long, value_name = "ID")]
    pub main: Option<String>,
    /// Filter of the lights when their headers name none (an ASIAIR writes none), e.g.
    /// "Dualband", "UV/IR cut": recorded in the target folder's identity (the lights are
    /// never written), then used by the stacking, the master it keeps and the plans.
    #[arg(long, value_name = "NAME")]
    pub filter: Option<String>,
    /// "No processing": the linear stack (calibrated, aligned, unstretched) as a 16-bit TIFF,
    /// `<target>/stacked/<name>_linear.tif`; alone, no render; with pipelines, written in addition, after them.
    #[arg(long)]
    pub no_render: bool,
    /// Presentation contrast: each image is also written with an S curve on the luminance
    /// (`presentation`, gain 2.0, `strong`, gain 3.6, or any gain from 0 to 6, as `2.5`), under
    /// the name `<image>_presentation`, `<image>_strong` or `<image>_contrast-2.5`; the image
    /// without a curve stays. Default `presentation`, like the window; `none` writes only the
    /// image without a curve.
    #[arg(
        long,
        value_name = "none|presentation|strong|GAIN",
        default_value = "presentation"
    )]
    pub contrast: astro_sync_core::stacking::Contrast,
    /// Colour intensity of the finish: the extra image has its colours strengthened by this percent
    /// (0 to 100, 20 and 40 are the window's detents), the luminance kept, after the contrast, under
    /// a name that says it (`<image>_colour-40`). Default `none`.
    #[arg(long, value_name = "none|PERCENT", default_value = "none")]
    pub colour: astro_sync_core::stacking::Colour,
    /// Softened stars of the finish: this percent (0 to 100, 50 and 85 are the window's detents) of
    /// the brightest stars of the extra image is taken out, after the contrast, under a name that
    /// says it (`<image>_stars-85`). Default `none`.
    #[arg(
        long = "soft-stars",
        value_name = "none|PERCENT",
        default_value = "none"
    )]
    pub soft_stars: astro_sync_core::stacking::SoftStars,
    /// The finish of one pipeline instead of --contrast, --colour and --soft-stars, like the
    /// « Finition » of its card in the window: ID=CONTRAST,COLOUR,STARS, each value written as
    /// for those options (`hoo=strong,20,50`; `none,none,none`: the render alone). Repeatable.
    #[arg(long = "finish", value_name = "ID=CONTRAST,COLOUR,STARS")]
    pub finishes: Vec<String>,
    /// The frame of the stacking when the field turns (« Cadre de l'empilement »): `auto` (the
    /// common area when the field does not turn, the whole field when it does) or the rank of a
    /// notch, 0 (the area every frame covers) to 4 (the whole field). Default `auto`.
    #[arg(long = "frame-size", value_name = "auto|RANK", default_value = "auto")]
    pub frame_size: String,
    /// Engine of the sky background and denoising of the "polished render": `graxpert-ai` (GraXpert's
    /// AI models, the default; Siril takes over if GraXpert is missing or fails) or `siril`
    /// (Siril's `subsky -rbf` and `denoise`, no AI model).
    #[arg(long, value_name = "graxpert-ai|siril", default_value = "graxpert-ai")]
    pub polished_engine: astro_sync_core::stacking::PolishedEngine,
    /// Trust fingerprint of an added script: ID=SHA256 (repeatable).
    #[arg(long = "trusted", value_name = "ID=SHA256")]
    pub trusted: Vec<String>,
    /// Astro Sync data (stacking-env/ environment, cached capabilities).
    #[arg(long)]
    pub data: PathBuf,
    /// Folder of the stacking scripts (stacking).
    #[arg(long)]
    pub stacking: Option<PathBuf>,
    /// Service (otherwise: the cached capabilities of Astro Sync, like the window).
    #[arg(long)]
    pub portal: Option<String>,
    #[arg(long)]
    pub token: Option<PathBuf>,
    /// Leave out the frames named in this file (one per line), like "Leave out the flagged
    /// frames" in the window: they stay in lights/, nothing is deleted.
    #[arg(long, value_name = "FICHIER")]
    pub leave_out: Option<PathBuf>,
    /// An image of the device (a file of `<target>/device/`) taken as the source of its family
    /// instead of a stack of its frames (repeatable). Without this option, the CLI does like the window:
    /// each family covered by an image of the device takes it; `--restack` stacks them all.
    #[arg(long = "device", value_name = "FICHIER")]
    pub device: Vec<String>,
    /// Uses no image of the device: every family is stacked from its frames.
    #[arg(long, conflicts_with = "device")]
    pub restack: bool,
    /// The work and the command line only, nothing is run.
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub language_of_messages: crate::cli::LangArg,
}

#[derive(Args, Debug)]
pub struct AssistantPromptArgs {
    /// Working folder.
    #[arg(long, required_unless_present = "file")]
    pub root: Option<PathBuf>,
    /// Name of the night's folder (its leading date is the evening's).
    #[arg(long, required_unless_present = "file")]
    pub night_folder: Option<String>,
    /// Folder of a ticked target (repeatable).
    #[arg(long = "target")]
    pub targets: Vec<String>,
    /// A single target ("Files").
    #[arg(long)]
    pub focus: Option<String>,
    /// "Files" message for this target (repeatable).
    #[arg(long = "message")]
    pub messages: Vec<String>,
    /// Common name of a target, `<folder>=<name>` (repeatable): "M31=Andromeda Galaxy".
    #[arg(long = "name")]
    pub names: Vec<String>,
    /// Show the text of ASSISTANT-IA.md (English, the same for everyone) instead of the prompt.
    #[arg(long)]
    pub file: bool,
    /// With --file: the text of a space whose multi-night is active ("Multi-night plans" section).
    #[arg(long, requires = "file")]
    pub multi_night: bool,
    /// Language of the application (fr, en, ja, de, es, it, pt-BR, ru, zh-Hans, zh-Hant).
    #[arg(long, default_value = "en")]
    pub lang: String,
    #[arg(long)]
    pub json: bool,
}

/// `scan`, with the detection of a night folder (`--detect`).
#[derive(Args, Debug)]
pub struct ScanArgs {
    #[command(flatten)]
    pub folder: crate::cli::FolderCommandArgs,
    /// Apply the detection rules of a night folder (FITS, XISF and RAW headers).
    #[arg(long)]
    pub detect: bool,
    /// Ticked target: IDENTIFIER=DESIGNATION (repeatable, with --detect).
    #[arg(long = "target", value_name = "ID=DESIGNATION")]
    pub targets: Vec<String>,
    #[arg(long)]
    pub evening_date: Option<String>,
    #[arg(long)]
    pub night_started_at: Option<String>,
    /// Longitude of the site, degrees east (with --detect).
    #[arg(long, allow_hyphen_values = true)]
    pub longitude: Option<f64>,
    #[command(flatten)]
    pub language_of_messages: crate::cli::LangArg,
}

/// `measure-master <file>`: the same measure as the window's "Measure a master…"
/// (`master_quality`, the `star_quality.py` script), same output.
#[derive(Args, Debug)]
pub struct MeasureMasterArgs {
    /// The linear master of the stack (FITS or XISF), as it comes out of the software.
    pub file: PathBuf,
    /// Astro Sync data (stacking-env/ environment, cached capabilities).
    #[arg(long)]
    pub data: PathBuf,
    /// Folder of the stacking scripts (stacking).
    #[arg(long)]
    pub stacking: PathBuf,
    /// Folder of the night: the measure is kept for the target's images
    /// (as in the window); without it, nothing is kept.
    #[arg(long, requires = "target_folder")]
    pub night: Option<PathBuf>,
    /// Folder of the target in the night (with --night).
    #[arg(long, requires = "night")]
    pub target_folder: Option<String>,
    #[arg(long)]
    pub json: bool,
}

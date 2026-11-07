//! The multi-night plans on the command line (parity with the window,
//! `README.md` § "CLI / GUI parity"): the same calls of
//! `astro_sync_core::multi_night` and `plan_image` as `commands_multi_night`.
//!
//! - `astro-sync integrations --night <folder> --project <id>`: the lines of
//!   the night's masters for the plan, sent (`PUT …/integrations`) under the
//!   plan's right; `--dry-run` prints them, nothing leaves.
//! - `astro-sync plan-image --plan <plan folder>` (or `--project <id>
//!   --root <working folder>`): the image of the plan made here (Siril,
//!   `plan_stack.py`), queued with `--queue` as the capture of the plan;
//!   `--dry-run` prints the integrations and the job.
//! - `astro-sync plan-attach --night <folder>`: the
//!   multi-night plans proposed for the night and the one it belongs to;
//!   `--project <id>` attaches it (`--detach`: detaches it); `--create`
//!   makes a plan of the night's target with the night in it; `--search
//!   <mot>` finds a plan by the name of its target. The same calls as the
//!   window's "Attach to a multi-night plan"
//!   (`astro_sync_core::multi_night_attach`); `--dry-run` asks nothing that
//!   writes.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::Args;
use serde::Serialize;

use astro_sync_core::gui_bridge::HttpPortalGuiTransport;
use astro_sync_core::local_work::CancelFlag;
use astro_sync_core::metadata;
use astro_sync_core::multi_night::{
    self, AttachGate, Computer, IntegrationLine, IntegrationsSent, MultiNightClosed,
    MultiNightGate, MultiNightTransport, NightProject,
};
use astro_sync_core::multi_night_attach::{
    self, NewPlanTarget, NightPlanChange, PlanChoice, PlanChoices, PlanCreated,
};
use astro_sync_core::night_service::NightServiceTransport;
use astro_sync_core::pipelines;
use astro_sync_core::plan_image::{
    self, CurrentNight, PlanImageGate, PlanImagePlan, PlanLine, PythonPlanImager,
};
use astro_sync_core::plan_progression::{self, PlanProgressionStep};
use astro_sync_core::precheck::PrecheckTransport;
use astro_sync_core::python_env::{self, StackingResources};
use astro_sync_core::service_wait::WakePolicy;
use astro_sync_core::space;
use astro_sync_core::stacking::{PipelineChoice, StackFailure};
use astro_sync_core::tools;
use astro_sync_core::upload_queue::UploadQueue;
use astro_sync_core::working_folder::{
    NAME_SEPARATOR, read_night_identity, target_folders_by_identity,
};

use crate::night::{
    detect_context, fail, main_first, print, service, stack_progress_printer, target_pipeline_ids,
};

type Outcome = Result<(), Box<dyn Error>>;

/// `--plan-field`: the frame of the plan's image, in the words of the setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum PlanFieldArg {
    /// « Bords nets »: the largest rectangle without a pixel no night covers.
    Nets,
    /// « Zone commune à toutes les nuits ».
    Commun,
    /// « Tout le champ ».
    Entier,
}

impl From<PlanFieldArg> for plan_image::PlanField {
    fn from(arg: PlanFieldArg) -> Self {
        match arg {
            PlanFieldArg::Nets => plan_image::PlanField::Clean,
            PlanFieldArg::Commun => plan_image::PlanField::Common,
            PlanFieldArg::Entier => plan_image::PlanField::Whole,
        }
    }
}

/// `--frame HAUT,GAUCHE,HAUTEUR,LARGEUR`: four whole numbers, a non-empty rectangle.
fn parse_frame(text: &str) -> Result<plan_image::PlanFrame, String> {
    let numbers: Vec<u32> = text
        .split(',')
        .map(|part| part.trim().parse::<u32>())
        .collect::<Result<_, _>>()
        .map_err(|_| "four integers separated by commas: TOP,LEFT,HEIGHT,WIDTH".to_string())?;
    match numbers.as_slice() {
        [top, left, height, width] if *height > 0 && *width > 0 => Ok(plan_image::PlanFrame {
            top: *top,
            left: *left,
            height: *height,
            width: *width,
        }),
        _ => Err("four integers, the height and the width not zero".to_string()),
    }
}

/// The service and the space: `--portal`, `--token`.
#[derive(Args, Debug, Clone)]
pub struct ServiceArgs {
    /// Address of the service.
    #[arg(long)]
    pub portal: Option<String>,
    /// Device token file.
    #[arg(long)]
    pub token: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct IntegrationsArgs {
    /// Folder of the night (its identity gives the night).
    #[arg(long)]
    pub night: PathBuf,
    /// The multi-night plan (identifier).
    #[arg(long)]
    pub project: String,
    /// Evening date (YYYY-MM-DD); without it: the one in the folder name.
    #[arg(long)]
    pub evening_date: Option<String>,
    /// Name of this computer ("office iMac").
    #[arg(long)]
    pub computer_label: Option<String>,
    /// Astro Sync data (installation identifier).
    #[arg(long)]
    pub data: Option<PathBuf>,
    #[command(flatten)]
    pub service: ServiceArgs,
    /// Show the lines, send nothing.
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub language_of_messages: crate::cli::LangArg,
}

#[derive(Args, Debug, Clone)]
pub struct PlanImageArgs {
    /// Folder of the plan (created by Astro Sync, its identity gives the plan).
    #[arg(long, required_unless_present = "project")]
    pub plan: Option<PathBuf>,
    /// The multi-night plan, when its folder does not exist yet (with --root).
    #[arg(long, requires = "root")]
    pub project: Option<String>,
    /// Working folder (with --project).
    #[arg(long)]
    pub root: Option<PathBuf>,
    /// The night being sent, if the service does not know it yet.
    #[arg(long)]
    pub night: Option<PathBuf>,
    /// A plan the import has not created yet (--project is then its provisional identifier); its
    /// nights are those of the import (provisional identifier, repeatable). Nothing is asked of the
    /// service.
    #[arg(long = "local-night", value_name = "NIGHT", requires_all = ["project", "target_id"])]
    pub local_nights: Vec<String>,
    /// The target of the local plan (with --local-night).
    #[arg(long)]
    pub target_id: Option<String>,
    /// The name of the local plan (its target, "M45"); by default the target's identifier.
    #[arg(long)]
    pub name: Option<String>,
    /// Integration not to combine: <night>|<family> (repeatable).
    #[arg(long = "exclude", value_name = "KEY")]
    pub exclude: Vec<String>,
    /// Integration for which the device's image is taken (the `Stacked_*.fit` of `device/`) and not
    /// the Astro Sync master: <night>|<family> (repeatable; by default the master when it exists, otherwise
    /// the device's image).
    #[arg(long = "device", value_name = "KEY")]
    pub device: Vec<String>,
    /// Kept framing of the plan's image: TOP,LEFT,HEIGHT,WIDTH (the four integers of a step of
    /// `coverage.framings` of the latest `..._plan.json` sheet); the whole image stays in stacked/,
    /// the cropped image (`..._framed.png`, its TIFF with --with-tiff) is the one that is sent.
    #[arg(long, value_name = "TOP,LEFT,HEIGHT,WIDTH", value_parser = parse_frame)]
    pub frame: Option<plan_image::PlanFrame>,
    /// Frame of the plan's image and of its progression: `nets` (the default: the largest rectangle
    /// without a pixel no night covers), `commun` (the largest rectangle every night covers: a
    /// smaller field, one exposure) or `entier` (the whole field, irregular edges).
    #[arg(
        long,
        value_enum,
        value_name = "nets|commun|entier",
        default_value = "nets"
    )]
    pub plan_field: PlanFieldArg,
    /// Rendering pipeline of the plan's target (repeatable; one image of the plan per pipeline). By
    /// default, those of a target: raw, and dualband-correction if a dual-band integration is
    /// combined; `linear` = "No processing".
    #[arg(long = "pipeline", value_name = "ID")]
    pub pipelines: Vec<String>,
    /// Pipelines of the plan's target: TARGET=ID,ID (identifier of the target; repeatable, like
    /// for `stack`). They replace --pipeline; those of the other targets are ignored.
    #[arg(long = "target-pipeline", value_name = "TARGET=ID,ID")]
    pub target_pipelines: Vec<String>,
    /// The pipeline whose render is the capture's main image (one of those asked for): its
    /// image is made first and is the capture of the plan, the others are its variants, named
    /// after their treatment. Without it, the first one asked for.
    #[arg(long = "main", value_name = "ID")]
    pub main: Option<String>,
    /// Join the TIFF of the renders (full resolution) to the sending; without it, a render written as
    /// PNG and TIFF leaves as its PNG, and a render that exists only as a TIFF (`linear`) as its TIFF.
    #[arg(long)]
    pub with_tiff: bool,
    /// "No processing": the combined integration, linear, as a 16-bit TIFF; alone, no
    /// render; with pipelines, written in addition, after them.
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
    /// Also make the progression of the plan: its image after each night (the first night, the
    /// first two, ... up to the plan's image), one JPEG each in `<plan>/progression/`, with the
    /// main render's pipeline. With --queue they leave as the history of the plan's capture.
    #[arg(long)]
    pub progression: bool,
    /// Make the progression only, not the plan's images (the capture is already online:
    /// --capture, else the plan's newest image). Implies --progression.
    #[arg(long)]
    pub only_progression: bool,
    /// The capture of the plan the progression belongs to (with --only-progression); by default
    /// the image of the plan the service holds, or the one this run queues.
    #[arg(long, value_name = "CAPTURE")]
    pub capture: Option<String>,
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
    /// Astro Sync data (stacking-env/ environment).
    #[arg(long)]
    pub data: PathBuf,
    /// Folder of the stacking scripts (stacking).
    #[arg(long)]
    pub stacking: Option<PathBuf>,
    /// Upload queue in which to put the plan's image (otherwise it stays on disk).
    #[arg(long)]
    pub queue: Option<PathBuf>,
    #[command(flatten)]
    pub service: ServiceArgs,
    /// Show the integrations and the work, run nothing.
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub language_of_messages: crate::cli::LangArg,
}

/// The project of `project_id` as the service answers it, its target the
/// plan's.
fn project_of(
    transport: &HttpPortalGuiTransport,
    portal: &str,
    token: &str,
    project_id: &str,
) -> Result<NightProject, Box<dyn Error>> {
    let value = transport.project(portal, token, project_id)?;
    let target = value
        .pointer("/target/targetId")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    multi_night::project_of(&value, &target)
        .ok_or_else(|| fail(format!("multi-night plan unreadable: {project_id}")))
}

/// Whether the service sells the multi-night plans, as it says itself
/// (`GET /v1/service`): never guessed from its address. `None`
/// when it does not say (the gates then stay closed, "unknown").
fn sold_by(portal: &str) -> Option<bool> {
    multi_night::service_sells_multi_night(portal, WakePolicy::default()).ok()
}

/// The right of the multi-night plans: the space's capabilities on a
/// service that sells them; always open on a private server.
fn gate_of(
    transport: &HttpPortalGuiTransport,
    portal: &str,
    token: &str,
) -> Result<MultiNightGate, Box<dyn Error>> {
    let Some(sold) = sold_by(portal) else {
        return Ok(MultiNightGate::unknown());
    };
    let capabilities = transport.space_storage(portal, token)?.capabilities;
    Ok(MultiNightGate::new(Some(&capabilities), sold))
}

/// The right of the gestures on a plan itself (attach, detach, create from
/// a night): `planner.mosaic`, "Advanced planning".
fn attach_gate_of(
    transport: &HttpPortalGuiTransport,
    portal: &str,
    token: &str,
) -> Result<AttachGate, Box<dyn Error>> {
    let Some(sold) = sold_by(portal) else {
        return Ok(AttachGate::unknown());
    };
    let capabilities = transport.space_storage(portal, token)?.capabilities;
    Ok(AttachGate::new(Some(&capabilities), sold))
}

fn evening_of(night: &Path, given: Option<&String>) -> String {
    given.cloned().unwrap_or_else(|| {
        night
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .and_then(|n| n.split(NAME_SEPARATOR).next().map(str::to_string))
            .unwrap_or_default()
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IntegrationsAnswer {
    project_id: String,
    lines: Vec<IntegrationLine>,
    sent: Option<IntegrationsSent>,
    dry_run: bool,
}

/// `astro-sync integrations`.
pub fn integrations(args: &IntegrationsArgs) -> Outcome {
    let identity = read_night_identity(&args.night)
        .ok_or_else(|| fail("this folder holds no night (.astro-sync-night.json)"))?;
    let (portal, token) = service(args.service.portal.as_ref(), args.service.token.as_ref())?;
    let transport = crate::words::portal_transport()?;
    let project = project_of(&transport, &portal, &token, &args.project)?;
    // The masters kept, and the image of the device for a family no master covers.
    let masters = multi_night::night_sources(
        &multi_night::target_masters(&args.night, &project.target_id),
        &multi_night::target_device_images(&args.night, &project.target_id),
    );
    let computer = Computer {
        label: args.computer_label.clone(),
        installation_id: args
            .data
            .as_ref()
            .and_then(|d| space::load_or_create_installation_id(&d.join("installation-id")).ok()),
    };
    let lines = multi_night::integration_lines(
        &identity.night_id,
        &project.target_id,
        &evening_of(&args.night, args.evening_date.as_ref()),
        &masters,
        &computer,
    );
    let sent = if args.dry_run {
        None
    } else {
        let gate = gate_of(&transport, &portal, &token)?;
        Some(multi_night::send_integrations(
            &transport, &portal, &token, &project, &lines, gate,
        )?)
    };
    let answer = IntegrationsAnswer {
        project_id: project.project_id.clone(),
        lines,
        sent,
        dry_run: args.dry_run,
    };
    print(args.json, &answer, || {
        for line in &answer.lines {
            println!(
                "{} · {} · {} × {} s · {}",
                line.evening_date,
                line.filter.as_deref().unwrap_or("no filter"),
                line.frame_count,
                line.exposure_seconds,
                line.target_id
            );
        }
        match &answer.sent {
            None => println!("Dry run: nothing was sent."),
            Some(IntegrationsSent::Sent { lines, .. }) => println!("{lines} row(s) sent."),
            Some(IntegrationsSent::Closed { closed, .. }) => {
                println!(
                    "Nothing is sent: multi-night plan closed ({}).",
                    closed_sentence(*closed)
                );
            }
            Some(IntegrationsSent::Nothing { .. }) => println!("No master in this night."),
            Some(IntegrationsSent::Refused { code, message, .. }) => {
                println!("Refused by the service ({code}): {message}");
            }
        }
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanImageAnswer {
    plan_dir: PathBuf,
    lines: Vec<PlanLine>,
    /// One job per pipeline (a single answer when nothing is to be made).
    plans: Vec<PlanImagePlan>,
    outcomes: Vec<plan_image::PlanImageOutcome>,
    enqueued: bool,
    /// The milestones of the progression of the plan: planned (a dry run) or made.
    progression: Vec<PlanProgressionStep>,
    progression_enqueued: bool,
}

/// A path made absolute against the current folder (itself when that cannot be
/// known).
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Why the multi-night plans are closed, in a sentence (never a private
/// server: it never restricts them).
fn closed_sentence(closed: MultiNightClosed) -> &'static str {
    match closed {
        MultiNightClosed::NotInSubscription => {
            "outside the subscription: multi-night plans are not in your space's offer"
        }
        MultiNightClosed::Unknown => {
            "space rights unknown: the service does not answer, or sign in again"
        }
    }
}

/// Why the image of a plan was not made, in a sentence.
fn plan_image_failure_sentence(failure: &StackFailure, gate: &MultiNightGate) -> String {
    let cause = match failure {
        StackFailure::NotInSubscription => match gate.closed {
            Some(MultiNightClosed::Unknown) => {
                "space rights unknown: the service does not answer, or sign in again".to_string()
            }
            _ => "outside the subscription: the image of a multi-night plan is not in your offer"
                .to_string(),
        },
        StackFailure::ToolMissing => {
            "stacking is not ready: install the tools (astro-sync tools --install)".to_string()
        }
        StackFailure::OneRenderOnly { asked } => {
            format!("{asked} renderings asked, but the space keeps only one: choose a single one")
        }
        StackFailure::NoLights => "no frame to stack".to_string(),
        StackFailure::NotEnoughSpace {
            needed,
            available,
            folder,
        } => format!(
            "not enough room on the disk of {folder}: {} MB needed, {} MB free",
            needed / 1_000_000,
            available / 1_000_000
        ),
        StackFailure::Cancelled => "cancelled".to_string(),
        StackFailure::OutsideStacked { file } => {
            format!("file refused, outside the stacked/ folder: {file}")
        }
        StackFailure::Failed { code, message, .. } => format!("{message} ({code})"),
    };
    format!("plan image impossible: {cause}")
}

/// `astro-sync plan-progression`: the progression of a plan whose capture is online, `plan-image`
/// without the images.
pub fn plan_progression(args: &PlanImageArgs) -> Outcome {
    let mut only = args.clone();
    only.progression = true;
    only.only_progression = true;
    plan_image(&only)
}

/// `astro-sync plan-image`.
pub fn plan_image(args: &PlanImageArgs) -> Outcome {
    // Every path of the job is absolute, whatever the way the folders were
    // given: the script that runs it does not start where the command did.
    let data = absolute(&args.data);
    let given_root = args.root.as_deref().map(absolute);
    let given_plan = args.plan.as_deref().map(absolute);
    let local = !args.local_nights.is_empty();
    let (project_id, root) = match (&given_plan, &args.project) {
        (Some(plan), _) => {
            let identity = plan_image::read_plan_identity(plan)
                .ok_or_else(|| fail("this folder holds no plan (.astro-sync-plan.json)"))?;
            let root = plan
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| fail("plan folder without a parent"))?;
            (identity.project_id, root)
        }
        (None, Some(project)) => (project.clone(), given_root.clone().unwrap_or_default()),
        (None, None) => return Err(fail("provide --plan, or --project and --root")),
    };
    let (project, lines) = if local {
        let target_id = args.target_id.clone().unwrap_or_default();
        let project = multi_night::NightProject {
            project_id: project_id.clone(),
            name: args.name.clone().unwrap_or_else(|| target_id.clone()),
            target_id,
            common_name: None,
            integrations: Vec::new(),
            total_seconds: 0.0,
            night_count: args.local_nights.len() as u64,
            plan_capture: None,
        };
        let lines = plan_image::local_plan_lines(&root, &project, &args.local_nights);
        (project, lines)
    } else {
        let (portal, token) = service(args.service.portal.as_ref(), args.service.token.as_ref())?;
        let transport = crate::words::portal_transport()?;
        let project = project_of(&transport, &portal, &token, &project_id)?;
        let current = args.night.as_ref().and_then(|night| {
            read_night_identity(night).map(|identity| CurrentNight {
                night_id: identity.night_id,
                evening_date: evening_of(night, None),
                folder: night.clone(),
            })
        });
        let lines = plan_image::plan_lines(&root, &project, current.as_ref());
        (project, lines)
    };
    let chosen: Vec<String> = lines
        .iter()
        .map(|l| l.key.clone())
        .filter(|key| !args.exclude.contains(key))
        .collect();
    // The pipelines of the plan's target, as the window takes them
    // from "Processing" (the target's defaults when none is named).
    let named = if args.pipelines.is_empty() && args.target_pipelines.is_empty() {
        Vec::new()
    } else {
        target_pipeline_ids(
            "",
            &project.target_id,
            &args.pipelines,
            &args.target_pipelines,
            false,
        )?
    };
    let mut picked: Vec<PipelineChoice> = if named.is_empty() && !args.no_render {
        plan_image::default_plan_pipelines(&lines, &chosen)
    } else {
        named.iter().map(|id| PipelineChoice::new(id)).collect()
    };
    if args.no_render
        && !picked
            .iter()
            .any(|p| p.pipeline_id == astro_sync_core::pipelines::LINEAR_PIPELINE)
    {
        picked.push(PipelineChoice::new(
            astro_sync_core::pipelines::LINEAR_PIPELINE,
        ));
    }
    // The render of `--main` is made first: it is the capture of the plan.
    let first = main_first(
        picked.iter().map(|p| p.pipeline_id.clone()).collect(),
        args.main.as_deref(),
    )?;
    picked.sort_by_key(|p| first.iter().position(|id| *id == p.pipeline_id));
    let plan_dir = match &given_plan {
        Some(plan) => plan.clone(),
        None if args.dry_run => root.join(plan_image::plan_folder_name(
            &project.name,
            &project.project_id,
        )),
        None => plan_image::plan_folder(&root, &project)?,
    };
    let context = detect_context(&data, args.stacking.as_deref().map(absolute).as_deref());
    let rows = tools::detect_tools(&context);
    let path_of = |id: &str| {
        rows.iter()
            .find(|r| r.id == id && r.state == tools::ToolState::Present)
            .and_then(|r| r.path.clone())
    };
    let making = plan_image::PlanMaking {
        sources: args
            .device
            .iter()
            .map(|key| (key.clone(), plan_image::PlanSource::Device))
            .collect(),
        frame: args.frame,
        plan_field: args.plan_field.into(),
    };
    let listed = pipelines::read_manifests(
        None,
        Some(&data.join("pipelines")),
        crate::words::requested_language(),
    );
    let mut plans =
        plan_image::plan_image_jobs_with(&project, &lines, &chosen, &plan_dir, &picked, &making);
    for plan in &mut plans {
        if let PlanImagePlan::Job(job) = plan {
            job.siril = path_of("siril");
            job.starnet = path_of("starnet2");
            job.graxpert = path_of("graxpert");
            job.pipelines_dirs = vec![data.join("pipelines")];
            job.contrast = args.contrast;
            // The render's name and help in the ten languages: the
            // generated title of the image on the service.
            job.render_texts = metadata::generated_render_texts(
                Some(&listed),
                &job.pipeline.pipeline_id,
                astro_sync_core::stacking::Contrast::None,
                astro_sync_core::stacking::Retouch::default(),
            );
            job.retouch = astro_sync_core::stacking::Retouch {
                colour: args.colour,
                soft_stars: args.soft_stars,
            };
        }
    }
    let jobs: Vec<&plan_image::PlanImageJob> = plans
        .iter()
        .filter_map(|plan| match plan {
            PlanImagePlan::Job(job) => Some(job.as_ref()),
            _ => None,
        })
        .collect();
    let mut answer = PlanImageAnswer {
        plan_dir,
        lines,
        plans: plans.clone(),
        outcomes: Vec::new(),
        enqueued: false,
        progression: Vec::new(),
        progression_enqueued: false,
    };
    // The progression of the plan: its image after each night, with the main render's pipeline.
    let wants_progression = args.progression || args.only_progression;
    let milestones = if wants_progression {
        let main = picked
            .first()
            .cloned()
            .unwrap_or_else(|| PipelineChoice::new(astro_sync_core::pipelines::DEFAULT_PIPELINE));
        let mut milestones = plan_progression::plan_progression_jobs(
            &project,
            &answer.lines,
            &chosen,
            &answer.plan_dir,
            &main,
            &making,
        );
        for milestone in &mut milestones {
            // The milestones belong to the run of the plan's images: one number.
            if let Some(job) = jobs.first() {
                milestone.job.name.clone_from(&job.name);
            }
            milestone.job.siril = path_of("siril");
            milestone.job.starnet = path_of("starnet2");
            milestone.job.graxpert = path_of("graxpert");
            milestone.job.pipelines_dirs = vec![data.join("pipelines")];
            milestone.job.contrast = astro_sync_core::stacking::Contrast::None;
            milestone.job.retouch = astro_sync_core::stacking::Retouch::default();
        }
        milestones
    } else {
        Vec::new()
    };
    answer.progression = milestones.iter().map(|m| m.step.clone()).collect();
    let make_images = !args.only_progression && !jobs.is_empty();
    if (make_images || !milestones.is_empty()) && !args.dry_run {
        // A local plan without a service named: the import's review proposed
        // it, nothing is asked of the space.
        let asked = if local && args.service.portal.is_none() {
            None
        } else {
            let (portal, token) =
                service(args.service.portal.as_ref(), args.service.token.as_ref())?;
            let transport = crate::words::portal_transport()?;
            let gate = gate_of(&transport, &portal, &token)?;
            let quality = transport
                .space_storage(&portal, &token)?
                .capabilities
                .analysis_logs;
            Some((gate, quality))
        };
        let (gate, image_quality) = asked.unwrap_or((
            MultiNightGate {
                allowed: true,
                closed: None,
            },
            false,
        ));
        let imager = PythonPlanImager {
            python: python_env::env_python(&data),
            script: context
                .stacking
                .as_ref()
                .map(StackingResources::script)
                .and_then(|script| script.parent().map(|dir| dir.join("plan_stack.py")))
                .unwrap_or_default(),
            timeout: Duration::from_secs(6 * 3600),
        };
        let image_gate = PlanImageGate {
            allowed: gate.allowed,
            tool_present: tools::tools_present(&rows).stacking_pipeline,
            image_quality,
        };
        // The capture the milestones belong to: the one this run queues, else
        // the one named, else the plan's newest image online.
        let mut capture_ref: Option<String> = args.capture.clone().or_else(|| {
            project
                .plan_capture
                .as_ref()
                .and_then(|c| c["id"].as_str())
                .map(str::to_string)
        });
        if args.queue.is_some()
            && !milestones.is_empty()
            && args.only_progression
            && capture_ref.is_none()
        {
            return Err(fail(
                "no capture of the plan to attach the progression to: give --capture",
            ));
        }
        if make_images {
            let run = plan_image::run_plan_images(
                &imager,
                &jobs,
                image_gate,
                &CancelFlag::new(),
                &mut stack_progress_printer(),
            );
            // The images made stay made, and leave, whatever stopped the next.
            if let (Some(queue), false) = (&args.queue, run.outcomes.is_empty()) {
                // One capture per plan: when the service holds the plan's
                // capture already, every image joins it as one of its images;
                // else the main render creates it.
                let results = plan_image::plan_results_of(
                    &run.outcomes,
                    args.main.as_deref(),
                    capture_ref.as_deref(),
                    args.with_tiff,
                );
                if capture_ref.is_none()
                    && let Some(main) = results.first()
                {
                    capture_ref = Some(format!(
                        "{}{}",
                        astro_sync_core::uploader::VARIANT_OF_SHA256,
                        main.sha256
                    ));
                }
                UploadQueue::open(queue)?.enqueue_results(&results)?;
                answer.enqueued = true;
            }
            answer.outcomes = run.outcomes;
            if let Some(failure) = run.failure {
                return Err(fail(plan_image_failure_sentence(&failure, &gate)));
            }
        }
        if !milestones.is_empty() {
            let made = plan_progression::run_plan_progression(
                &imager,
                &milestones,
                &answer.plan_dir,
                image_gate,
                &CancelFlag::new(),
                &mut stack_progress_printer(),
            );
            // Made, they leave as the history of the plan's capture.
            if let (Some(queue), Some(capture), false) =
                (&args.queue, &capture_ref, made.steps.is_empty())
            {
                UploadQueue::open(queue)?.enqueue_results(
                    &plan_progression::plan_progression_results(
                        &made.steps,
                        &project.target_id,
                        capture,
                    ),
                )?;
                answer.progression_enqueued = true;
            }
            answer.progression = made.steps;
            if let Some(failure) = made.failure {
                return Err(fail(plan_image_failure_sentence(&failure, &gate)));
            }
        }
    }
    print(args.json, &answer, || {
        for line in &answer.lines {
            println!(
                "{} · {} · {} frames · {:?}{}",
                line.evening_date,
                line.filter.as_deref().unwrap_or("no filter"),
                line.frame_count,
                line.place,
                if args.exclude.contains(&line.key) {
                    " (left out)"
                } else {
                    ""
                }
            );
        }
        match answer.plans.first() {
            Some(PlanImagePlan::OneNight { night_id }) => {
                println!("A single night combined ({night_id}): the plan's image is the night's.")
            }
            Some(PlanImagePlan::Job(_)) if answer.outcomes.is_empty() && !args.only_progression => {
                for plan in &answer.plans {
                    if let PlanImagePlan::Job(job) = plan {
                        println!(
                            "Dry run: {} · {} master(s) including {} device image(s), reference {}, output {}{}.",
                            job.pipeline.pipeline_id,
                            job.masters.len(),
                            job.masters
                                .iter()
                                .filter(|m| m.source == plan_image::PlanSource::Device)
                                .count(),
                            job.masters[job.reference].evening_date,
                            job.out.display(),
                            job.frame.map_or_else(String::new, |f| format!(
                                ", frame {} × {} at ({}, {})",
                                f.width, f.height, f.left, f.top
                            ))
                        );
                    }
                }
            }
            Some(PlanImagePlan::Job(_)) => {
                for outcome in &answer.outcomes {
                    for render in std::iter::once(&outcome.render)
                        .chain(outcome.finishes.iter().map(|finish| &finish.render))
                    {
                        println!(
                            "Plan image: {} ({} nights){}",
                            render.path.display(),
                            outcome.combined_night_ids.len(),
                            if answer.enqueued {
                                " · put in the upload queue"
                            } else {
                                ""
                            }
                        );
                    }
                }
            }
            Some(PlanImagePlan::Nothing) | None => println!("Nothing to combine on this computer."),
        }
        for step in &answer.progression {
            println!(
                "Progression: night {} ({}) · {} frames{}{}",
                step.nights,
                step.evening_date,
                step.frames,
                if step.jpeg.as_os_str().is_empty() {
                    String::new()
                } else {
                    format!(" · {}", step.jpeg.display())
                },
                if answer.progression_enqueued {
                    " · put in the upload queue"
                } else {
                    ""
                }
            );
        }
    })
}

/* ------------------------- Attaching a night -------------------- */

#[derive(Args, Debug)]
pub struct PlanAttachArgs {
    /// Folder of the night (its identity gives the night).
    #[arg(long, required_unless_present = "search")]
    pub night: Option<PathBuf>,
    /// The multi-night plan to attach the night to (identifier).
    #[arg(long, conflicts_with_all = ["create", "search"])]
    pub project: Option<String>,
    /// Detach the night from the plan given by --project.
    #[arg(long, requires = "project")]
    pub detach: bool,
    /// Create a multi-night plan for the night's target, with this night in it.
    #[arg(long, conflicts_with = "search", requires = "night")]
    pub create: bool,
    /// The target of the plan to create (identifier); without it: the folder's only target.
    #[arg(long, requires = "create")]
    pub target: Option<String>,
    /// The name of the target of the plan to create ("IC 1805"); without it: the name of its folder.
    #[arg(long, requires = "create")]
    pub target_name: Option<String>,
    /// The common name of the target ("Heart Nebula").
    #[arg(long, requires = "create")]
    pub common_name: Option<String>,
    /// Search for a plan by the name of its target (accent- and case-insensitive).
    #[arg(long)]
    pub search: Option<String>,
    #[command(flatten)]
    pub service: ServiceArgs,
    /// Write nothing: say what would be requested.
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub language_of_messages: crate::cli::LangArg,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanAttachAnswer {
    night_id: Option<String>,
    /// `list`, `search`, `attach`, `detach` or `create`.
    mode: &'static str,
    choices: Option<PlanChoices>,
    found: Option<Vec<PlanChoice>>,
    change: Option<NightPlanChange>,
    created: Option<PlanCreated>,
    dry_run: bool,
}

/// `astro-sync plan-attach`.
pub fn plan_attach(args: &PlanAttachArgs) -> Outcome {
    let identity = match &args.night {
        Some(night) => Some(
            read_night_identity(night)
                .ok_or_else(|| fail("this folder holds no night (.astro-sync-night.json)"))?,
        ),
        None => None,
    };
    let night_id = identity.as_ref().map(|i| i.night_id.clone());
    let (portal, token) = service(args.service.portal.as_ref(), args.service.token.as_ref())?;
    let transport = crate::words::portal_transport()?;
    let mut answer = PlanAttachAnswer {
        night_id: night_id.clone(),
        mode: "list",
        choices: None,
        found: None,
        change: None,
        created: None,
        dry_run: args.dry_run,
    };
    if let Some(query) = &args.search {
        answer.mode = "search";
        answer.found = Some(multi_night_attach::search_plans(
            &transport,
            &portal,
            &token,
            night_id.as_deref(),
            query,
        )?);
    } else {
        let night = args.night.as_ref().expect("clap requires --night");
        let night_id = night_id.expect("the identity was read");
        let targets = target_folders_by_identity(night);
        if args.project.is_none() && !args.create {
            answer.choices = Some(multi_night_attach::proposed_plans(
                &transport,
                &portal,
                &token,
                &night_id,
                !targets.is_empty(),
            )?);
        } else if !args.dry_run {
            let gate = attach_gate_of(&transport, &portal, &token)?;
            if let Some(project) = &args.project {
                if args.detach {
                    answer.mode = "detach";
                    answer.change = Some(multi_night_attach::detach_night(
                        &transport, &portal, &token, project, &night_id, gate,
                    )?);
                } else {
                    answer.mode = "attach";
                    answer.change = Some(multi_night_attach::attach_night(
                        &transport, &portal, &token, project, &night_id, gate,
                    )?);
                }
            } else {
                answer.mode = "create";
                let (target_id, folder) = match &args.target {
                    Some(id) => (
                        id.clone(),
                        targets
                            .iter()
                            .find(|(found, _)| found == id)
                            .map(|(_, folder)| folder.clone()),
                    ),
                    None => match targets.as_slice() {
                        [(id, folder)] => (id.clone(), Some(folder.clone())),
                        _ => {
                            return Err(fail(
                                "several targets (or none) in this folder: provide --target",
                            ));
                        }
                    },
                };
                let target = NewPlanTarget {
                    name: args
                        .target_name
                        .clone()
                        .or(folder)
                        .unwrap_or_else(|| target_id.clone()),
                    target_id,
                    common_name: args.common_name.clone(),
                };
                // The night's place is read only when the plan may be made.
                let place = match gate.closed {
                    Some(_) => None,
                    None => multi_night_attach::place_of_detail(
                        &transport.session_detail(&portal, &token, &night_id)?,
                    ),
                };
                answer.created = Some(multi_night_attach::create_plan_from_night(
                    &transport,
                    &portal,
                    &token,
                    &night_id,
                    &target,
                    place.as_ref(),
                    gate,
                )?);
            }
        } else {
            answer.mode = if args.create {
                "create"
            } else if args.detach {
                "detach"
            } else {
                "attach"
            };
        }
    }
    print(args.json, &answer, || print_attach(&answer))
}

fn print_attach(answer: &PlanAttachAnswer) {
    let plan_line = |choice: &PlanChoice| {
        let state = match choice.state {
            multi_night_attach::ChoiceState::Available => "",
            multi_night_attach::ChoiceState::Current => " · this night is in it",
            multi_night_attach::ChoiceState::Moves => " · moves the night from another plan",
            multi_night_attach::ChoiceState::OtherTarget => " · other target",
        };
        println!(
            "{} · {}{} · {} night(s){state}",
            choice.project_id,
            choice.name,
            choice
                .common_name
                .as_deref()
                .map(|c| format!(" ({c})"))
                .unwrap_or_default(),
            choice.night_count,
        );
    };
    if let Some(choices) = &answer.choices {
        if choices.plans.is_empty() {
            println!("No multi-night plan for this night's target.");
        }
        for choice in &choices.plans {
            plan_line(choice);
        }
        if choices.no_target {
            println!("This night has no target: all the plans are offered.");
        }
    }
    if let Some(found) = &answer.found {
        if found.is_empty() {
            println!("No plan matches this search.");
        }
        for choice in found {
            plan_line(choice);
        }
    }
    if answer.dry_run && answer.change.is_none() && answer.created.is_none() {
        if answer.mode == "list" || answer.mode == "search" {
            return;
        }
        println!("Dry run: nothing was asked of the service.");
    }
    match &answer.change {
        Some(NightPlanChange::Attached { project_id, nights }) => {
            println!("Night attached to the plan {project_id} ({nights} night(s)).");
        }
        Some(NightPlanChange::Detached { project_id, nights }) => {
            println!("Night detached from the plan {project_id} ({nights} night(s)).");
        }
        Some(NightPlanChange::Closed { closed, .. }) => {
            println!(
                "Nothing is asked: multi-night plan closed ({}).",
                closed_sentence(*closed)
            );
        }
        Some(NightPlanChange::Refused { code, message, .. }) => {
            println!("Refused by the service ({code}): {message}");
        }
        None => {}
    }
    match &answer.created {
        Some(PlanCreated::Created {
            project_id, name, ..
        }) => println!("Multi-night plan {name} created ({project_id}), the night in it."),
        Some(PlanCreated::Closed { closed }) => {
            println!(
                "Nothing is asked: multi-night plan closed ({}).",
                closed_sentence(*closed)
            );
        }
        Some(PlanCreated::Missing { missing }) => {
            println!("The night lacks what it takes to create the plan ({missing:?}).");
        }
        Some(PlanCreated::Refused { code, message }) => {
            println!("Refused by the service ({code}): {message}");
        }
        None => {}
    }
}

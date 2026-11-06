//! The multi-night plans in the window: what « Traitements »,
//! « Récapitulatif » and « Nuits » read, the sending of a night's
//! integrations and the making of the image of the plan. Each command is a
//! thin call into `astro_sync_core::multi_night`, `plan_image` and
//! `night_steps`, the calls `astro-sync integrations` and `astro-sync
//! plan-image` make on the command line.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

use astro_sync_core::detection::{DetectionInput, NightWindow};
use astro_sync_core::local_work::WorkKind;
use astro_sync_core::multi_night::{
    self, AttachGate, Computer, IntegrationPreview, IntegrationsSent, MultiNightGate,
    NightMultiNight, NightProject, PlanTag,
};
use astro_sync_core::multi_night_attach::{
    self, NewPlanTarget, NightPlanChange, PlanChoice, PlanChoices, PlanCreated,
};
use astro_sync_core::night_service::NightServiceTransport;
use astro_sync_core::night_steps;
use astro_sync_core::plan_image::{
    self, CurrentNight, PlanField, PlanFrame, PlanFraming, PlanImageGate, PlanImageOutcome,
    PlanImagePlan, PlanLine, PlanMaking, PlanSource, PythonPlanImager,
};
use astro_sync_core::plan_progression::{self, PlanProgressionStep};
use astro_sync_core::python_env;
use astro_sync_core::service_wait::WakePolicy;
use astro_sync_core::space;
use astro_sync_core::stacking::PipelineChoice;
use astro_sync_core::tools;
use astro_sync_core::upload_queue::EnqueueOutcome;
use astro_sync_core::working_folder::NightLocation;
use astro_sync_core::{gui_bridge, pipeline_trust};

use crate::commands::{
    ending_sign_in_on_error, local_work_of, require_portal_and_token, run_blocking,
};
use crate::commands_night::{
    capabilities_cached, locate, pipeline_list_of, root_of, settings, stacking_dir, tool_rows,
    transport,
};
use crate::error::CommandError;
use crate::paths;
use crate::state::AppPaths;

/// The event of the image of a plan in progress
/// (`astro_sync_core::stacking::StackEvent`, as the stacking's).
pub const PLAN_IMAGE_EVENT: &str = "astro-sync://plan-image";

/// How long an answer of the service about selling the multi-night plans is
/// trusted before it is asked again, in this process.
const SOLD_ANSWER_LIFETIME: Duration = Duration::from_secs(600);

/// What the service said of selling the multi-night plans, by address, and
/// when it was heard.
type SoldAnswers = HashMap<String, (Option<bool>, Instant)>;

/// The service's last answers (`GET /v1/service`), by address, with when they
/// were heard: the gates are read often, the service is asked once in a
/// while.
fn sold_answers() -> &'static Mutex<SoldAnswers> {
    static ANSWERS: OnceLock<Mutex<SoldAnswers>> = OnceLock::new();
    ANSWERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Whether the service in force sells the multi-night plans, as it says
/// itself (Astro Sync never guesses « hosted » from the
/// address); `None` while it was never heard.
fn sold_by_service(paths: &AppPaths, service_url: &str) -> Option<bool> {
    let key = service_url
        .trim()
        .trim_end_matches('/')
        .to_ascii_lowercase();
    let remembered = sold_answers().lock().ok().and_then(|answers| {
        answers
            .get(&key)
            .filter(|(_, heard)| heard.elapsed() < SOLD_ANSWER_LIFETIME)
            .map(|(sold, _)| *sold)
    });
    if let Some(sold) = remembered {
        return sold;
    }
    let sold = multi_night::service_sells_multi_night_kept(
        service_url,
        WakePolicy::background_refresh(),
        &paths
            .capabilities_cache
            .with_file_name("service-sells-multi-night.json"),
    );
    if let Ok(mut answers) = sold_answers().lock() {
        answers.insert(key, (sold, Instant::now()));
    }
    sold
}

/// Whether the service in force sells the multi-night plans, and this
/// computer as a line names it (the label of the paired device, the
/// installation).
fn sold_and_computer(paths: &AppPaths) -> (Option<bool>, Computer) {
    let settings = gui_bridge::load_settings(&paths.settings).unwrap_or_default();
    let paired = settings
        .portal_url
        .as_deref()
        .and_then(|url| gui_bridge::load_device_token(&paths.device_token, url).ok())
        .flatten()
        .is_some();
    let status = space::space_status(&paths.space_files(), paired).ok();
    let sold = sold_by_service(paths, &space::service_url(&settings));
    let label = status
        .and_then(|s| s.device.map(|d| d.name))
        .or_else(|| settings.device_label.clone());
    let installation_id = space::load_or_create_installation_id(&paths.installation_id).ok();
    (
        sold,
        Computer {
            label,
            installation_id,
        },
    )
}

/// Whether the multi-night plans are open here (`analysis.progression` on a
/// service that sells them, always on a private server): `ASSISTANT-IA.md`
/// then carries their section.
pub(crate) fn multi_night_open(paths: &AppPaths) -> bool {
    gate_of(paths).is_ok_and(|(gate, _)| gate.allowed)
}

fn gate_of(paths: &AppPaths) -> Result<(MultiNightGate, Computer), CommandError> {
    let (sold, computer) = sold_and_computer(paths);
    let capabilities = capabilities_cached(paths)?;
    let gate = match sold {
        Some(sold) => MultiNightGate::new(capabilities.as_ref(), sold),
        None => MultiNightGate::unknown(),
    };
    Ok((gate, computer))
}

/// The right of the gestures on a plan itself (attach, detach, create from a
/// night): `planner.mosaic`, « Planification avancée », where [`gate_of`]
/// reads the integrations' `analysis.progression`.
pub(crate) fn attach_gate_of(paths: &AppPaths) -> Result<AttachGate, CommandError> {
    let (sold, _) = sold_and_computer(paths);
    let capabilities = capabilities_cached(paths)?;
    Ok(match sold {
        Some(sold) => AttachGate::new(capabilities.as_ref(), sold),
        None => AttachGate::unknown(),
    })
}

/// One multi-night plan of the night, with what can be combined.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    pub project: NightProject,
    /// Its integrations: here (this night, another night) or on another
    /// computer.
    pub lines: Vec<PlanLine>,
}

/// The multi-night side of a night, for « Traitements » and
/// « Récapitulatif ».
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiNightView {
    pub evening_date: Option<String>,
    pub projects: Vec<ProjectView>,
    pub gate: MultiNightGate,
    pub computer: Computer,
}

fn current_night(paths: &AppPaths, night_id: &str, evening: &str) -> Option<CurrentNight> {
    match locate(paths, night_id).ok()? {
        NightLocation::Found { folder } => Some(CurrentNight {
            night_id: night_id.to_string(),
            evening_date: evening.to_string(),
            folder,
        }),
        _ => None,
    }
}

fn night_side(
    paths: &AppPaths,
    portal_url: &str,
    token: &str,
    transport: &gui_bridge::HttpPortalGuiTransport,
    session_id: &str,
    plan_id: Option<&str>,
    evening_date: &str,
) -> Result<(NightMultiNight, Vec<ProjectView>), CommandError> {
    let night =
        multi_night::night_projects(transport, portal_url, token, plan_id, Some(session_id))?;
    let evening = night
        .evening_date
        .clone()
        .unwrap_or_else(|| evening_date.to_string());
    let root = root_of(&settings(paths)?)?;
    let current = current_night(paths, session_id, &evening);
    let projects = night
        .projects
        .iter()
        .map(|project| ProjectView {
            lines: plan_image::plan_lines(&root, project, current.as_ref()),
            project: project.clone(),
        })
        .collect();
    Ok((night, projects))
}

/// The multi-night plans the night's targets work for (`GET /v1/plans/{id}`,
/// `GET /v1/projects/{id}`), their integrations here and elsewhere, and
/// whether they are open here. Reading never needs the right.
#[tauri::command]
pub async fn night_multi_night<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
    plan_id: Option<String>,
    evening_date: String,
) -> Result<MultiNightView, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let (portal_url, token) = require_portal_and_token(&app)?;
        let (gate, computer) = gate_of(&paths)?;
        let transport = transport(&app)?;
        let result = night_side(
            &paths,
            &portal_url,
            &token,
            &transport,
            &session_id,
            plan_id.as_deref(),
            &evening_date,
        )
        .map(|(night, projects)| MultiNightView {
            evening_date: night.evening_date,
            projects,
            gate,
            computer,
        });
        ending_sign_in_on_error(&app, result)
    })
    .await
}

/// « Intégration de la nuit », per target: its families inside the night's
/// window (local noon to local noon at the place's longitude), the frames
/// left out, the master kept of each family. Reads headers only.
#[tauri::command]
pub async fn night_integration_preview(
    night_folder: String,
    target_folder: String,
    input: DetectionInput,
) -> Result<IntegrationPreview, CommandError> {
    run_blocking(move || {
        let window = NightWindow::of_input(&input);
        Ok(multi_night::integration_preview(
            Path::new(&night_folder),
            &target_folder,
            window.as_ref(),
        ))
    })
    .await
}

/// After the sending of a night: the lines of its masters for each
/// multi-night plan, under the right (`analysis.progression`, the hosted
/// service). A refusal of the service is an outcome, never an error.
#[tauri::command]
pub async fn night_integrations_send<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
    plan_id: Option<String>,
    night_folder: String,
    evening_date: String,
) -> Result<Vec<IntegrationsSent>, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let (portal_url, token) = require_portal_and_token(&app)?;
        let (gate, computer) = gate_of(&paths)?;
        let transport = transport(&app)?;
        let result = (|| {
            let night = multi_night::night_projects(
                &transport,
                &portal_url,
                &token,
                plan_id.as_deref(),
                Some(&session_id),
            )?;
            Ok::<_, CommandError>(night_steps::send_night_integrations(
                &transport,
                &portal_url,
                &token,
                &session_id,
                Path::new(&night_folder),
                &night,
                &evening_date,
                &computer,
                gate,
            )?)
        })();
        ending_sign_in_on_error(&app, result)
    })
    .await
}

/// What « Image du plan multi-nuits » gave.
#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PlanImageRun {
    /// Made and queued as captures of the plan: one image per pipeline.
    Made {
        outcomes: Vec<PlanImageOutcome>,
        enqueued: EnqueueOutcome,
        /// The image of the plan after each night (asked with `progression`):
        /// queued as the history of the plan's capture unless `queue` is
        /// `false`, in which case the sending of the images queues them.
        progression: Vec<PlanProgressionStep>,
        /// Why the progression stopped, when it did: the images are made all
        /// the same.
        progression_failure: Option<String>,
    },
    /// One night only: the image of the plan is that night's image.
    OneNight { night_id: String },
    /// Nothing combinable was ticked.
    Nothing,
}

/// « Image du plan multi-nuits »: the masters ticked (their keys), aligned,
/// combined (on the dual-band master when there is one), rendered by each of
/// `pipelines` (the ones chosen for
/// the plan's target in « Traitements »; none: the target's
/// defaults), in the plan's folder of the working folder; each image queued
/// as a capture of the plan (unless `queue` is `false`). Behind the
/// right and the pipeline row of « Dépendances »; the scratch on the plan's
/// own disk, removed at the end.
// The arguments are the window's own call (one per field it sends).
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn plan_image_run<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
    plan_id: Option<String>,
    project_id: String,
    chosen: Vec<String>,
    pipelines: Vec<PipelineChoice>,
    evening_date: String,
    queue: Option<bool>,
    sources: Option<BTreeMap<String, PlanSource>>,
    frame: Option<PlanFrame>,
    progression: Option<bool>,
) -> Result<PlanImageRun, CommandError> {
    let (cancel, _) = local_work_of(&app, Some(WorkKind::Stacking));
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let (portal_url, token) = require_portal_and_token(&app)?;
        let (gate, _) = gate_of(&paths)?;
        let transport = transport(&app)?;
        let (_, projects) = night_side(
            &paths,
            &portal_url,
            &token,
            &transport,
            &session_id,
            plan_id.as_deref(),
            &evening_date,
        )?;
        let view = projects
            .into_iter()
            .find(|p| p.project.project_id == project_id)
            .ok_or_else(|| CommandError::new("project_not_found", "Multi-night plan not found."))?;
        make_plan_images(
            &app,
            &paths,
            gate,
            &view,
            &chosen,
            pipelines,
            queue,
            PlanMaking {
                sources: sources.unwrap_or_default(),
                frame,
                plan_field: PlanField::default(),
            },
            progression.unwrap_or(false),
            &cancel,
        )
    })
    .await
}

/// A multi-night plan the import's review proposed and the space does not
/// hold yet: its provisional identifier, its target and the import's own
/// nights (their provisional identifiers).
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalPlan {
    pub project_id: String,
    pub target_id: String,
    pub name: String,
    pub nights: Vec<String>,
}

/// « Image du plan multi-nuits » of a plan the import will create: the plan
/// is the review's proposal, its integrations the masters of the import's
/// nights -- nothing is asked of the space, nothing is queued
/// (`plan_image_run`'s `queue: false`); the plan's folder takes the
/// provisional identifier until the « Récapitulatif » creates the plan.
#[tauri::command]
pub async fn plan_image_run_local<R: Runtime>(
    app: AppHandle<R>,
    plan: LocalPlan,
    pipelines: Vec<PipelineChoice>,
    progression: Option<bool>,
) -> Result<PlanImageRun, CommandError> {
    let (cancel, _) = local_work_of(&app, Some(WorkKind::Stacking));
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let (gate, _) = gate_of(&paths)?;
        let root = root_of(&settings(&paths)?)?;
        let project = NightProject {
            project_id: plan.project_id,
            target_id: plan.target_id,
            name: plan.name,
            common_name: None,
            integrations: Vec::new(),
            total_seconds: 0.0,
            night_count: plan.nights.len() as u64,
            plan_capture: None,
        };
        let lines = plan_image::local_plan_lines(&root, &project, &plan.nights);
        let chosen: Vec<String> = lines
            .iter()
            .filter(|line| line.combinable())
            .map(|line| line.key.clone())
            .collect();
        let view = ProjectView { project, lines };
        make_plan_images(
            &app,
            &paths,
            gate,
            &view,
            &chosen,
            pipelines,
            Some(false),
            PlanMaking::default(),
            progression.unwrap_or(false),
            &cancel,
        )
    })
    .await
}

/// The images of the plan of `view` made from the integrations `chosen`, one
/// per pipeline, queued unless `queue` is `false`.
#[allow(clippy::too_many_arguments)]
fn make_plan_images<R: Runtime>(
    app: &AppHandle<R>,
    paths: &AppPaths,
    gate: MultiNightGate,
    view: &ProjectView,
    chosen: &[String],
    pipelines: Vec<PipelineChoice>,
    queue: Option<bool>,
    making: PlanMaking,
    progression: bool,
    cancel: &astro_sync_core::local_work::CancelFlag,
) -> Result<PlanImageRun, CommandError> {
    let root = root_of(&settings(paths)?)?;
    let plan_dir = plan_image::plan_folder(&root, &view.project)
        .map_err(|e| CommandError::new("io_error", e.to_string()))?;
    // « Image du plan multi-nuits » (Réglages › Général): the frame the image
    // and its milestones are cut to.
    let making = PlanMaking {
        plan_field: astro_sync_core::local_settings::load(&paths.local_settings)?.plan_field,
        ..making
    };
    // The digest the person trusted for a script of their own.
    let store = pipeline_trust::load(&paths.data_dir);
    let listed = pipeline_list_of(app, paths, "en");
    let language = gui_bridge::load_settings(&paths.settings)
        .ok()
        .and_then(|s| s.locale)
        .unwrap_or_else(|| "en".to_string());
    let titled = pipeline_list_of(app, paths, &language);
    let pipelines: Vec<PipelineChoice> = pipelines
        .into_iter()
        .map(|pipeline| PipelineChoice {
            trusted_sha256: listed
                .get(&pipeline.pipeline_id)
                .and_then(|p| store.trusted_sha256(p)),
            ..pipeline
        })
        .collect();
    let mut jobs = Vec::new();
    for plan in plan_image::plan_image_jobs_with(
        &view.project,
        &view.lines,
        chosen,
        &plan_dir,
        &pipelines,
        &making,
    ) {
        match plan {
            PlanImagePlan::Job(job) => jobs.push(*job),
            PlanImagePlan::OneNight { night_id } => {
                return Ok(PlanImageRun::OneNight { night_id });
            }
            PlanImagePlan::Nothing => return Ok(PlanImageRun::Nothing),
        }
    }
    let rows = tool_rows(app, paths);
    let path_of = |id: &str| {
        rows.iter()
            .find(|r| r.id == id && r.state == tools::ToolState::Present)
            .and_then(|r| r.path.clone())
    };
    // The slider of « Traitements »: one choice for every render.
    let settings = astro_sync_core::local_settings::load(&paths.local_settings)?;
    let contrast = settings.contrast;
    let retouch = astro_sync_core::stacking::Retouch {
        colour: settings.colour,
        soft_stars: settings.soft_stars,
    };
    for job in &mut jobs {
        job.siril = path_of("siril");
        job.starnet = path_of("starnet2");
        job.graxpert = path_of("graxpert");
        job.pipelines_dirs = vec![paths.data_dir.join("pipelines")];
        // The finish chosen for the pipeline (the wizard's contrast, a
        // custom one included) wins over the settings' default.
        job.apply_finish(astro_sync_core::stacking::PipelineFinish { contrast, retouch });
        // The name the steps give a pipeline the service has no word for.
        job.pipeline_label =
            astro_sync_core::metadata::render_texts(&listed, &job.pipeline.pipeline_id, "en").0;
        // The render's name and help in the ten languages: the generated
        // title of the image on the service, shown in each reader's language.
        job.render_texts = astro_sync_core::metadata::generated_render_texts(
            Some(&titled),
            &job.pipeline.pipeline_id,
            astro_sync_core::stacking::Contrast::None,
            astro_sync_core::stacking::Retouch::default(),
        );
    }
    let imager = PythonPlanImager {
        python: python_env::env_python(&paths.data_dir),
        script: stacking_dir(app)
            .map(|dir| dir.join("plan_stack.py"))
            .unwrap_or_default(),
        timeout: Duration::from_secs(6 * 3600),
    };
    let handle = app.clone();
    let job_refs: Vec<&plan_image::PlanImageJob> = jobs.iter().collect();
    let gate = PlanImageGate {
        allowed: gate.allowed,
        tool_present: tools::tools_present(&rows).stacking_pipeline,
        // The quality of the image leaves with it under `analysis.logs`.
        image_quality: capabilities_cached(paths)?.is_some_and(|c| c.analysis_logs),
    };
    let run = plan_image::run_plan_images(&imager, &job_refs, gate, cancel, &mut |event| {
        let _ = handle.emit(PLAN_IMAGE_EVENT, event);
    });
    // The images made stay made, and are queued, whatever stopped the next
    // (`queue: false` keeps them here, « Images à envoyer » queues the
    // ones the person ticks).
    // One capture per plan: the one the service holds already takes every
    // image as one of its own.
    let held = view
        .project
        .plan_capture
        .as_ref()
        .and_then(|c| c["id"].as_str());
    let results = plan_image::plan_results_of(&run.outcomes, None, held);
    let enqueued = if run.outcomes.is_empty() || queue == Some(false) {
        EnqueueOutcome::default()
    } else {
        paths::open_queue(paths)?.enqueue_results(&results)?
    };
    if let Some(failure) = run.failure {
        let mut error = CommandError::new("plan_image_failed", format!("{failure:?}"));
        error.details = serde_json::to_value(&failure).ok();
        return Err(error);
    }
    // The image of the plan after each night, with the pipeline of the main
    // image: the history of the capture. A progression that stops keeps the
    // milestones made, and never fails the images.
    let mut milestones = Vec::new();
    let mut progression_failure = None;
    if let (true, Some(main), Some(job)) = (progression, results.first(), jobs.first()) {
        let mut planned = plan_progression::plan_progression_jobs(
            &view.project,
            &view.lines,
            chosen,
            &plan_dir,
            &job.pipeline,
            &making,
        );
        for milestone in &mut planned {
            milestone.job.name.clone_from(&job.name);
            milestone.job.siril = job.siril.clone();
            milestone.job.starnet = job.starnet.clone();
            milestone.job.graxpert = job.graxpert.clone();
            milestone.job.pipelines_dirs = job.pipelines_dirs.clone();
            milestone.job.contrast = astro_sync_core::stacking::Contrast::None;
        }
        let made = plan_progression::run_plan_progression(
            &imager,
            &planned,
            &plan_dir,
            PlanImageGate {
                image_quality: false,
                ..gate
            },
            cancel,
            &mut |event| {
                let _ = handle.emit(PLAN_IMAGE_EVENT, event);
            },
        );
        progression_failure = made.failure.map(|failure| format!("{failure:?}"));
        if queue != Some(false) && !made.steps.is_empty() {
            paths::open_queue(paths)?.enqueue_results(
                &plan_progression::plan_progression_results(
                    &made.steps,
                    &view.project.target_id,
                    &held.map_or_else(
                        || {
                            format!(
                                "{}{}",
                                astro_sync_core::uploader::VARIANT_OF_SHA256,
                                main.sha256
                            )
                        },
                        str::to_string,
                    ),
                ),
            )?;
        }
        milestones = made.steps;
    }
    Ok(PlanImageRun::Made {
        outcomes: run.outcomes,
        enqueued,
        progression: milestones,
        progression_failure,
    })
}

/// The frames the person may keep for the image of a plan: the notches `plan_stack.py` measured on
/// the last image made with the same integrations, and a picture of it. Nothing is asked of the service and nothing is
/// written: `lines` are the ones the window read (`night_multi_night`), `chosen` the integrations ticked, `sources`
/// the source taken for a night. `None` before the first image of that set was made (it is made whole).
#[tauri::command]
pub async fn plan_framing_read<R: Runtime>(
    app: AppHandle<R>,
    project_id: String,
    lines: Vec<PlanLine>,
    chosen: Vec<String>,
    sources: Option<BTreeMap<String, PlanSource>>,
) -> Result<Option<PlanFraming>, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let root = root_of(&settings(&paths)?)?;
        let Some(plan_dir) = plan_image::find_plan_folder(&root, &project_id) else {
            return Ok(None);
        };
        // The notches of an image cut the way the setting cuts the next one.
        let making = PlanMaking {
            sources: sources.unwrap_or_default(),
            plan_field: astro_sync_core::local_settings::load(&paths.local_settings)?.plan_field,
            ..PlanMaking::default()
        };
        Ok(plan_image::plan_framing(
            &plan_dir, &lines, &chosen, &making,
        ))
    })
    .await
}

/// « Nuits »: the tags of the nights of a multi-night plan, by night
/// (« Plan multi-nuits IC 1805 · 3 nuits · 11 h 08 »). Reading only.
#[tauri::command]
pub async fn multi_night_tags<R: Runtime>(
    app: AppHandle<R>,
) -> Result<BTreeMap<String, Vec<PlanTag>>, CommandError> {
    run_blocking(move || {
        let (portal_url, token) = require_portal_and_token(&app)?;
        let result = multi_night::plan_tags(&transport(&app)?, &portal_url, &token)
            .map_err(CommandError::from);
        ending_sign_in_on_error(&app, result)
    })
    .await
}

/* --------------------- Attaching a night to a plan --------------- */
// « Rattacher à un plan multi-nuits » from the night's wizard. Each command
// is a thin call into `astro_sync_core::multi_night_attach`, the calls
// `astro-sync plan-attach` makes on the command line.

/// A multi-night plan the night belongs to.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachedPlan {
    pub project_id: String,
    pub name: String,
    pub common_name: Option<String>,
    pub night_count: u64,
    pub total_seconds: f64,
}

/// The multi-night plan(s) of a night and whether attaching is open here.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NightAttachment {
    pub plans: Vec<AttachedPlan>,
    pub gate: AttachGate,
}

/// The multi-night plans the night belongs to, by its plan's items or by
/// itself (`GET /v1/projects?sessionId=`), and the right to attach. Reading
/// never needs the right.
#[tauri::command]
pub async fn night_attachment<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
    plan_id: Option<String>,
) -> Result<NightAttachment, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let (portal_url, token) = require_portal_and_token(&app)?;
        let gate = attach_gate_of(&paths)?;
        let result = multi_night::night_projects(
            &transport(&app)?,
            &portal_url,
            &token,
            plan_id.as_deref(),
            Some(&session_id),
        )
        .map(|night| NightAttachment {
            plans: night
                .projects
                .into_iter()
                .map(|project| AttachedPlan {
                    project_id: project.project_id,
                    name: project.name,
                    common_name: project.common_name,
                    night_count: project.night_count,
                    total_seconds: project.total_seconds,
                })
                .collect(),
            gate,
        })
        .map_err(CommandError::from);
        ending_sign_in_on_error(&app, result)
    })
    .await
}

/// The plans proposed for the night: those of its target, the current plan
/// first; `has_target`: the night has a target recorded.
#[tauri::command]
pub async fn night_plan_choices<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
    has_target: bool,
) -> Result<PlanChoices, CommandError> {
    run_blocking(move || {
        let (portal_url, token) = require_portal_and_token(&app)?;
        let result = multi_night_attach::proposed_plans(
            &transport(&app)?,
            &portal_url,
            &token,
            &session_id,
            has_target,
        )
        .map_err(CommandError::from);
        ending_sign_in_on_error(&app, result)
    })
    .await
}

/// The search of a plan by the name of its target, among all the plans of
/// the space.
#[tauri::command]
pub async fn night_plan_search<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
    query: String,
) -> Result<Vec<PlanChoice>, CommandError> {
    run_blocking(move || {
        let (portal_url, token) = require_portal_and_token(&app)?;
        let result = multi_night_attach::search_plans(
            &transport(&app)?,
            &portal_url,
            &token,
            Some(&session_id),
            &query,
        )
        .map_err(CommandError::from);
        ending_sign_in_on_error(&app, result)
    })
    .await
}

/// Attaches the night to the plan (a night of another plan is moved), under
/// the right of the plans themselves (`planner.mosaic`). A refusal of the
/// service is an outcome, never an error.
#[tauri::command]
pub async fn night_plan_attach<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
    project_id: String,
) -> Result<NightPlanChange, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let (portal_url, token) = require_portal_and_token(&app)?;
        let gate = attach_gate_of(&paths)?;
        let result = multi_night_attach::attach_night(
            &transport(&app)?,
            &portal_url,
            &token,
            &project_id,
            &session_id,
            gate,
        )
        .map_err(CommandError::from);
        ending_sign_in_on_error(&app, result)
    })
    .await
}

/// Detaches the night from the plan; the same right, the same outcomes.
#[tauri::command]
pub async fn night_plan_detach<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
    project_id: String,
) -> Result<NightPlanChange, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let (portal_url, token) = require_portal_and_token(&app)?;
        let gate = attach_gate_of(&paths)?;
        let result = multi_night_attach::detach_night(
            &transport(&app)?,
            &portal_url,
            &token,
            &project_id,
            &session_id,
            gate,
        )
        .map_err(CommandError::from);
        ending_sign_in_on_error(&app, result)
    })
    .await
}

/// Creates a blank multi-night plan for the night's target (the night's
/// place, a goal of 10 h) with the night in it.
#[tauri::command]
pub async fn night_plan_create<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
    target: NewPlanTarget,
) -> Result<PlanCreated, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let (portal_url, token) = require_portal_and_token(&app)?;
        let gate = attach_gate_of(&paths)?;
        let transport = transport(&app)?;
        let result = (|| {
            // The plan inherits the night's own place.
            // … read only when the plan may be made.
            let place = match gate.closed {
                Some(_) => None,
                None => multi_night_attach::place_of_detail(&transport.session_detail(
                    &portal_url,
                    &token,
                    &session_id,
                )?),
            };
            multi_night_attach::create_plan_from_night(
                &transport,
                &portal_url,
                &token,
                &session_id,
                &target,
                place.as_ref(),
                gate,
            )
            .map_err(CommandError::from)
        })();
        ending_sign_in_on_error(&app, result)
    })
    .await
}

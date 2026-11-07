//! The handlers of the rewritten journey's commands: the same
//! core calls as the window's commands
//! (`app/src-tauri/src/commands_night.rs`), mostly through
//! `astro_sync_core::night_steps`.

use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use astro_sync_core::analysis::log_offset::{ForcedLogOffsets, LogsUtcOffset};
use astro_sync_core::analysis::photometry::ReferenceStarList;
use astro_sync_core::analysis::sky_night::SkyOptions;
use astro_sync_core::assistant_prompt::{self, PromptInput};
use astro_sync_core::detection::{
    self, DetectOptions, DetectionInput, DetectionTarget, NightDetection,
};
use astro_sync_core::device_master::{self, DeviceUse};
use astro_sync_core::gui_bridge::{self, DriveOptions, QueueDriveEvent};
use astro_sync_core::local_settings;
use astro_sync_core::local_work::CancelFlag;
use astro_sync_core::master_quality;
use astro_sync_core::metadata;
use astro_sync_core::night_analysis;
use astro_sync_core::night_online::{self, NightOnlineTransport};
use astro_sync_core::night_resume::{self, FolderState};
use astro_sync_core::night_steps::{self, DataPaths, NightPrecheck, SentRecord};
use astro_sync_core::pipeline_trust;
use astro_sync_core::pipelines::{self, StackingEnvironment};
use astro_sync_core::precheck::{self, ImageCandidate, PrecheckTransport};
use astro_sync_core::progression_images::{self, ProgressionJob};
use astro_sync_core::python_env::{self, EnvTimeouts, StackingResources};
use astro_sync_core::sky_solution::{
    self, SolveHints, SolveJob, SolveOutcome, SolveSetup, SolverChoice,
};
use astro_sync_core::stacking::{
    self, Framing, PipelineChoice, PipelineFinish, PythonStacker, RegisteredFrames, Retouch,
    StackEvent, StackGate,
};
use astro_sync_core::tools::{self, DetectContext, HttpToolFetcher};
use astro_sync_core::upload_queue::UploadQueue;
use astro_sync_core::uploader::{self, UploadTarget};
use astro_sync_core::working_folder::{self, NightFolderSpec, NightTargetSpec, RootEnvironment};

use crate::cli::FolderCommandArgs;
use crate::cli_night::{
    AssistantPromptArgs, CheckArgs, ListPipelinesArgs, MeasureMasterArgs, NightArgs,
    NightFolderArgs, NightsArgs, ProgressionImagesArgs, ScanArgs, SendLogsArgs, SendNightArgs,
    SolveArgs, StackArgs, ToolsArgs, WorkingFolderArgs,
};
use crate::output;
use crate::words;

type Outcome = Result<(), Box<dyn Error>>;

pub(crate) fn fail(message: impl Into<String>) -> Box<dyn Error> {
    message.into().into()
}

pub(crate) fn token_of(path: &Path) -> Result<String, Box<dyn Error>> {
    let token = std::fs::read_to_string(path).map_err(|e| {
        fail(format!(
            "could not read the token ({}): {e}",
            path.display()
        ))
    })?;
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err(fail(format!("the token file {} is empty", path.display())));
    }
    Ok(token)
}

pub(crate) fn service(
    portal: Option<&String>,
    token: Option<&PathBuf>,
) -> Result<(String, String), Box<dyn Error>> {
    match (portal, token) {
        (Some(portal), Some(token)) => Ok((portal.clone(), token_of(token)?)),
        _ => Err(fail("provide --portal and --token")),
    }
}

fn root_or_default(root: Option<&PathBuf>) -> Result<PathBuf, Box<dyn Error>> {
    root.cloned()
        .or_else(|| working_folder::default_root(&RootEnvironment::current()))
        .ok_or_else(|| fail("no Documents folder: provide --root"))
}

fn pair(raw: &str) -> Result<(String, String), Box<dyn Error>> {
    raw.split_once('=')
        .map(|(a, b)| (a.trim().to_string(), b.trim().to_string()))
        .filter(|(a, b)| !a.is_empty() && !b.is_empty())
        .ok_or_else(|| fail(format!("expected ID=VALUE, got {raw:?}")))
}

/// `--finish ID=CONTRAST,COLOUR,STARS`: a pipeline and its own finish.
fn pipeline_finish(raw: &str) -> Result<(String, PipelineFinish), Box<dyn Error>> {
    let (id, values) = pair(raw)?;
    let parts: Vec<&str> = values.split(',').map(str::trim).collect();
    let [contrast, colour, stars] = parts.as_slice() else {
        return Err(fail(format!(
            "expected ID=CONTRAST,COLOUR,STARS, got {raw:?}"
        )));
    };
    Ok((
        id,
        PipelineFinish {
            contrast: contrast.parse().map_err(fail)?,
            retouch: Retouch {
                colour: colour.parse().map_err(fail)?,
                soft_stars: stars.parse().map_err(fail)?,
            },
        },
    ))
}

/// `--frame-size auto|RANK`: `None` for Automatic.
fn frame_size(raw: &str) -> Result<Option<u8>, Box<dyn Error>> {
    if raw == "auto" {
        return Ok(None);
    }
    raw.parse::<u8>()
        .ok()
        .filter(|rank| *rank < stacking::FRAME_NOTCHES)
        .map(Some)
        .ok_or_else(|| {
            fail(format!(
                "--frame-size: expected auto or a notch from 0 to {}, got {raw:?}",
                stacking::FRAME_NOTCHES - 1
            ))
        })
}

pub(crate) fn print<T: Serialize>(json: bool, value: &T, lines: impl FnOnce()) -> Outcome {
    if json {
        output::print_json(value)?;
    } else {
        lines();
    }
    Ok(())
}

fn detection_input(night: &NightArgs) -> Result<DetectionInput, Box<dyn Error>> {
    Ok(DetectionInput {
        evening_date: night.evening_date.clone(),
        night_started_at: night.night_started_at.clone(),
        longitude: night.longitude,
        targets: night
            .targets
            .iter()
            .map(|raw| {
                pair(raw).map(|(id, designation)| DetectionTarget {
                    target_id: id,
                    designation,
                    aliases: Vec::new(),
                    ticked: true,
                    online_final: false,
                    stacking_planned: false,
                })
            })
            .collect::<Result<_, _>>()?,
    })
}

fn detect(night: &NightArgs) -> Result<NightDetection, Box<dyn Error>> {
    Ok(detection::detect_night(
        &night.folder,
        &detection_input(night)?,
        &DetectOptions::default(),
    )?)
}

/* ------------------------------ working-folder ---------------------------- */

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkingFolderAnswer {
    root: PathBuf,
    exists: bool,
    assistant_file: Option<assistant_prompt::AssistantFile>,
    dry_run: bool,
}

pub fn working_folder(args: &WorkingFolderArgs) -> Outcome {
    let root = root_or_default(args.root.as_ref())?;
    let settings_path = args.data.as_ref().map(|d| d.join("local-settings.json"));
    let mut settings = match &settings_path {
        Some(path) => local_settings::load(path)?,
        None => local_settings::LocalSettings::default(),
    };
    let mut answer = WorkingFolderAnswer {
        exists: root.is_dir(),
        root: root.clone(),
        assistant_file: None,
        dry_run: args.dry_run || !args.prepare,
    };
    if args.prepare && !args.dry_run {
        let (_, file) = assistant_prompt::prepare_root_for(
            &root,
            settings.assistant_file_sha256.as_deref(),
            args.multi_night,
        )?;
        settings.assistant_file_sha256 = file.sha256.clone();
        if let Some(path) = &settings_path {
            local_settings::save(path, &settings)?;
        }
        answer.assistant_file = Some(file);
        answer.exists = true;
    }
    print(args.json, &answer, || {
        println!("Working folder: {}", answer.root.display());
        match &answer.assistant_file {
            Some(file) => println!("ASSISTANT-IA.md: {:?}", file.outcome),
            None if answer.exists => println!("It exists. Nothing was written."),
            None => {
                println!("It does not exist yet; --prepare creates it with ASSISTANT-IA.md.")
            }
        }
    })
}

/* --------------------------------- nights -------------------------------- */

pub fn nights(args: &NightsArgs) -> Outcome {
    let root = root_or_default(args.root.as_ref())?;
    let entries = if root.is_dir() {
        working_folder::list_night_folders(&root)?
    } else {
        Vec::new()
    };
    print(args.json, &entries, || {
        if entries.is_empty() {
            println!("No night in {}.", root.display());
        }
        for entry in &entries {
            println!("{}  {}", entry.night_id, entry.name);
        }
    })
}

/* ------------------------------- night-folder ----------------------------- */

pub fn night_folder(args: &NightFolderArgs) -> Outcome {
    let root = root_or_default(args.root.as_ref())?;
    let mut targets = Vec::new();
    for (raw, ticked) in args
        .targets
        .iter()
        .map(|t| (t, true))
        .chain(args.unticked.iter().map(|t| (t, false)))
    {
        let (target_id, designation) = pair(raw)?;
        targets.push(NightTargetSpec {
            target_id,
            designation,
            ticked,
        });
    }
    let spec = NightFolderSpec {
        night_id: args.night_id.clone(),
        service_url: args.service_url.clone(),
        evening_date: args.evening_date.clone(),
        label: args.label.clone(),
        targets,
        follow: args.follow.clone(),
    };
    let now = astro_sync_core::now_iso8601_utc();
    let outcome = if args.dry_run {
        working_folder::plan_night_folder(&root, &[], &spec, &now)?
    } else {
        working_folder::prepare_night_folder(&root, &[], &spec, &now)?
    };
    print(args.json, &outcome, || {
        println!("Night folder: {}", outcome.folder.display());
        let verb = if args.dry_run {
            "would be created"
        } else {
            "created"
        };
        for path in &outcome.created_paths {
            println!("  {verb}: {path}");
        }
    })
}

/* ---------------------------------- scan ---------------------------------- */

/// `scan --detect`: the rules of the night folder; `scan` alone: the read-only
/// walk of the folder.
pub fn scan(args: &ScanArgs, plain: impl FnOnce(&FolderCommandArgs) -> Outcome) -> Outcome {
    if !args.detect {
        return plain(&args.folder);
    }
    let folder = args
        .folder
        .folder_source
        .folder
        .clone()
        .ok_or_else(|| fail("--detect needs --folder (the night folder)"))?;
    let night = NightArgs {
        folder,
        targets: args.targets.clone(),
        evening_date: args.evening_date.clone(),
        night_started_at: args.night_started_at.clone(),
        longitude: args.longitude,
    };
    let detection = detect(&night)?;
    let locale = words::requested_language();
    print(args.folder.json, &detection, || {
        for target in &detection.targets {
            let frames = words::say_plural(
                locale,
                "wizard.files.frames",
                target.light_frames as u64,
                &[],
            )
            .unwrap_or_default();
            let line = match target.final_image.as_deref() {
                Some(image) => words::say(
                    locale,
                    "cli.scan.targetLine",
                    &[
                        ("target", &target.designation),
                        ("frames", &frames),
                        ("image", image),
                    ],
                ),
                None => words::say(
                    locale,
                    "cli.scan.targetLineMissing",
                    &[("target", &target.designation), ("frames", &frames)],
                ),
            };
            println!("{}", line.unwrap_or_default());
        }
        for finding in &detection.findings {
            println!("- {}", crate::findings::sentence(finding, locale));
        }
    })
}

/* -------------------------------- check ----------------------------------- */

fn night_candidates(
    night: &NightArgs,
) -> Result<(NightDetection, night_resume::ResumeLocal), Box<dyn Error>> {
    let detection = detect(night)?;
    let local = night_steps::resume_local(
        FolderState::Found,
        Some(&night.folder),
        Some(&detection),
        None,
        None,
    )?;
    Ok((detection, local))
}

/// The proposals in a few words (the whole is in `--json`).
fn describe_proposals(proposals: &metadata::Proposals) -> String {
    let mut parts = Vec::new();
    if !proposals.acquisition.is_empty() {
        parts.push(format!(
            "{} acquisition row(s)",
            proposals.acquisition.len()
        ));
    }
    if let Some(calibration) = &proposals.calibration {
        parts.push(format!("calibration ({calibration})"));
    }
    if proposals.sensor.is_some() {
        parts.push("sensor".to_string());
    }
    if let Some(software) = proposals
        .processing
        .as_ref()
        .and_then(|p| p.software.as_ref())
    {
        parts.push(format!("processing ({software})"));
    }
    if let Some(title) = &proposals.variant_generated_title {
        let title = astro_sync_core::tools::localized(title, crate::words::requested_language());
        parts.push(format!("image title “{title}”"));
    }
    parts.join(", ")
}

/// The file name of an image, for the sentences that name it.
fn file_name_of(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Why an image is not sent, in the person's words (the window's sentences),
/// never the name of a variant.
fn not_sent_sentence(
    plan: &precheck::ImagePlan,
    reason: precheck::NotSent,
    storage: &precheck::SpaceStorage,
    lang: &str,
) -> String {
    use precheck::NotSent;
    let file = file_name_of(&plan.image.path);
    let key = match reason {
        NotSent::VariantsNotAllowed => "wizard.summary.setAsideNotKept",
        NotSent::ReplacedByStacking => "wizard.treatments.final.setAside",
        NotSent::OneRenderOnly => "wizard.summary.setAsideOneRender",
        NotSent::OtherFinalImage => "wizard.treatments.final.otherFinal",
        NotSent::NoCaptureForProgression => "wizard.summary.setAsideNoCapture",
        NotSent::TooLargeForSpace => {
            let size = words::size_text(lang, plan.image.size);
            let max = words::size_text(lang, storage.max_file_bytes);
            return words::say(
                lang,
                "wizard.summary.setAsideTooLarge",
                &[("file", &file), ("size", &size), ("max", &max)],
            )
            .unwrap_or(file);
        }
    };
    words::say(lang, key, &[("file", &file)]).unwrap_or(file)
}

/// One limit that stops the sending, in the window's sentences with their
/// numbers.
fn finding_sentence(finding: &precheck::QuotaFinding, lang: &str) -> String {
    use precheck::QuotaFinding;
    let size = |bytes: u64| words::size_text(lang, bytes);
    let sentence = match finding {
        QuotaFinding::StorageQuota {
            to_send_bytes,
            remaining_bytes,
            quota_bytes,
        } => words::say(
            lang,
            "wizard.summary.refused.storage",
            &[
                ("toSend", &size(*to_send_bytes)),
                ("remaining", &size(*remaining_bytes)),
                ("quota", &size(*quota_bytes)),
            ],
        ),
        QuotaFinding::FileTooLarge {
            path,
            size: bytes,
            max_file_bytes,
        } => words::say(
            lang,
            "wizard.summary.refused.fileTooLarge",
            &[
                ("file", &file_name_of(path)),
                ("size", &size(*bytes)),
                ("max", &size(*max_file_bytes)),
            ],
        ),
        QuotaFinding::VariantLimit {
            target_id,
            existing,
            adding,
            variant_limit,
            ..
        } => words::say(
            lang,
            "wizard.summary.refused.variants",
            &[
                ("target", target_id),
                ("existing", &existing.to_string()),
                ("adding", &adding.to_string()),
                ("limit", &variant_limit.to_string()),
            ],
        ),
        QuotaFinding::CaptureCount {
            adding,
            capture_count,
            capture_count_limit,
        } => words::say(
            lang,
            "wizard.summary.refused.captures",
            &[
                ("count", &capture_count.to_string()),
                ("limit", &capture_count_limit.to_string()),
                ("adding", &adding.to_string()),
            ],
        ),
        QuotaFinding::Refused { path, code, .. } => {
            let reason = if code == "target_not_found" {
                words::say(lang, "cli.targetNotFound", &[])
            } else {
                words::say(lang, &format!("errors.{code}"), &[])
            }
            .unwrap_or_else(|| code.clone());
            words::say(
                lang,
                "wizard.summary.refused.service",
                &[("file", &file_name_of(path)), ("reason", &reason)],
            )
        }
    };
    sentence.unwrap_or_else(|| finding.code().to_string())
}

/// The contract's value for a ceiling the profile does not set (`2^53 - 1`).
const UNLIMITED_BYTES: u64 = 9_007_199_254_740_991;

fn print_precheck(check: &NightPrecheck, lang: &str) {
    for plan in &check.plans {
        let decision = match &plan.decision {
            precheck::Decision::AlreadyOnline { .. } => "already online, identical".to_string(),
            precheck::Decision::NewCapture => "new capture".to_string(),
            precheck::Decision::Variant { .. } | precheck::Decision::VariantOfNew { .. } => {
                "variant of the capture".to_string()
            }
            precheck::Decision::Progression { .. }
            | precheck::Decision::ProgressionOfNew { .. } => match plan.image.frame_count {
                Some(frames) => format!("progression of the capture ({frames} frame(s))"),
                None => "progression of the capture".to_string(),
            },
            precheck::Decision::NotSent { reason } => {
                not_sent_sentence(plan, *reason, &check.storage, lang)
            }
        };
        println!("{}: {decision}", plan.image.path.display());
        if let Some(proposals) = &plan.metadata {
            println!("  proposals: {}", describe_proposals(proposals));
        }
    }
    let size = |bytes: u64| words::size_text(lang, bytes);
    // A server that keeps no quota (a private server) has no space left to
    // count: the contract's unlimited value is never printed as a size.
    let line = if check.storage.quota_bytes >= UNLIMITED_BYTES {
        words::say(
            lang,
            "wizard.summary.check.placeUnlimited",
            &[("toSend", &size(check.quota.to_send_bytes))],
        )
    } else {
        words::say(
            lang,
            "wizard.summary.check.placeShort",
            &[
                ("toSend", &size(check.quota.to_send_bytes)),
                ("remaining", &size(check.storage.remaining_bytes)),
                ("quota", &size(check.storage.quota_bytes)),
            ],
        )
    };
    if let Some(line) = line {
        println!("{line}");
    }
    // A space that bounds its number of captures (the free account): the
    // captures this sending adds and those left.
    let limit = check.storage.capture_count_limit;
    if limit > 0 && limit < stacking::UNLIMITED {
        let adding = check
            .plans
            .iter()
            .filter(|plan| matches!(plan.decision, precheck::Decision::NewCapture))
            .count();
        let left = limit.saturating_sub(check.storage.capture_count);
        if let Some(line) = words::say(
            lang,
            "wizard.summary.check.captures",
            &[
                ("adding", &adding.to_string()),
                ("left", &left.to_string()),
                ("limit", &limit.to_string()),
            ],
        ) {
            println!("{line}");
        }
    }
    let already_online = check
        .plans
        .iter()
        .any(|plan| matches!(plan.decision, precheck::Decision::AlreadyOnline { .. }));
    if check.quota.allowed && check.quota.to_send_files == 0 && !already_online {
        // Nothing leaves, and the person is told why and what to do (a
        // night without a final image, in a space that cannot stack).
        if let Some(line) = words::say(lang, "wizard.summary.nothingToSend", &[]) {
            println!("{line}");
        }
    }
    if check.quota.allowed {
        if let Some(line) = words::say(lang, "wizard.summary.check.serviceShort", &[]) {
            println!("{line}");
        }
    } else {
        if let Some(line) = words::say(lang, "wizard.summary.impossible", &[]) {
            println!("{line}.");
        }
        for finding in &check.quota.findings {
            println!("  - {}", finding_sentence(finding, lang));
        }
        if let Some(line) = words::say(lang, "wizard.summary.nothingSent", &[]) {
            println!("{line}");
        }
    }
}

/// The milestones of the progression each ticked target of the night's folder
/// holds (`<target>/progression/`), with the poses they stack.
fn recorded_progression(
    night_folder: &Path,
    detection: &NightDetection,
) -> Vec<precheck::ImageCandidate> {
    detection
        .targets
        .iter()
        .filter(|target| target.in_night)
        .filter_map(|target| {
            let folder = target.folder_name.as_deref()?;
            Some(progression_images::recorded_candidates(
                night_folder,
                folder,
                &target.target_id,
            ))
        })
        .flatten()
        .collect()
}

pub fn check(args: &CheckArgs) -> Outcome {
    let token = token_of(&args.service.token)?;
    let transport = crate::words::portal_transport()?;
    let online = transport.night_online(&args.service.portal, &token, &args.session_id)?;
    let (detection, local) = night_candidates(&args.night)?;
    // The milestones of the progression the folder holds leave as the history of
    // the target's capture, as the window sends the ones it has just made; the
    // TIFF of a render only with --with-tiff (`render_files`).
    let mut candidates = night_steps::precheck_candidates(
        &local,
        recorded_progression(&args.night.folder, &detection),
        &[],
        args.with_tiff,
    );
    if let Some(main) = &args.main {
        precheck::prefer_main_render(&mut candidates, main);
    }
    let mut check = night_steps::precheck_night(
        &transport,
        &args.service.portal,
        &token,
        &online,
        &candidates,
    )?;
    // What the box « Joindre aussi les TIFF » of the window says.
    check.full_resolution = night_steps::full_resolution_of(
        &night_steps::precheck_candidates(&local, Vec::new(), &[], true),
        args.with_tiff,
    );
    propose(
        &mut check,
        &online,
        &args.night.folder,
        &detection,
        &args.lang,
        None,
    );
    print(args.json, &check, || print_precheck(&check, &args.lang))
}

/// What Astro Sync proposes for each image of the check (`metadata` of
/// each plan: nothing is written, the plans only carry it). The titles of
/// a rendering come from the pipelines, the person's too.
fn propose(
    check: &mut NightPrecheck,
    online: &night_online::NightOnline,
    night_folder: &Path,
    detection: &NightDetection,
    language: &str,
    pipelines_dir: Option<&Path>,
) {
    let list = pipelines::read_manifests(None, pipelines_dir, language);
    metadata::attach_proposals(
        &mut check.plans,
        online,
        &metadata::ProposalContext {
            night_folder,
            detection,
            language,
            pipelines: Some(&list),
            image_quality: check.storage.capabilities.analysis_logs,
        },
    );
}

/* -------------------------------- send-night ------------------------------ */

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct SendNightAnswer {
    dry_run: bool,
    precheck: Option<NightPrecheck>,
    enqueued: usize,
    sent: usize,
    analysis: Option<night_analysis::NightAnalysisSent>,
    solutions: Vec<SolveOutcome>,
    /// The proposals for the images already online.
    online_proposals: Vec<uploader::OnlineProposal>,
}

fn drive(
    portal: &str,
    token: &str,
    queue: &UploadQueue,
    session_id: &str,
    quiet: bool,
) -> Result<usize, Box<dyn Error>> {
    let transport = crate::words::upload_transport(portal)?;
    let target = UploadTarget {
        session_id: Some(session_id.to_string()),
        plan_id: None,
        target_ids: Vec::new(),
    };
    let mut sent = 0;
    gui_bridge::drive_queue(
        &transport,
        queue,
        token,
        &target,
        &DriveOptions::command_line(),
        || true,
        |event| match event {
            QueueDriveEvent::Completed {
                capture_id,
                skipped,
                ..
            } => {
                sent += 1;
                if !quiet {
                    println!("  sent -> capture {capture_id}");
                    if let Some(note) = astro_sync_core::metadata::skipped_sentence(&skipped) {
                        println!("  {note}");
                    }
                }
            }
            QueueDriveEvent::Refused { message, .. }
            | QueueDriveEvent::Held { message, .. }
            | QueueDriveEvent::Failed { message, .. } => {
                if !quiet {
                    println!("  {message}");
                }
            }
            _ => {}
        },
    )?;
    Ok(sent)
}

fn solver_choice(raw: &str) -> Result<SolverChoice, Box<dyn Error>> {
    match raw {
        "astap" => Ok(SolverChoice::Astap),
        "astrometry-net" => Ok(SolverChoice::AstrometryNet),
        other => Err(fail(format!(
            "unknown solver {other:?} (astap or astrometry-net)"
        ))),
    }
}

fn reference_stars(path: Option<&Path>) -> Option<ReferenceStarList> {
    path.filter(|p| p.is_file())
        .and_then(|p| ReferenceStarList::read(p).ok())
}

/// What the proposals of a sending are built from.
struct Proposing<'a> {
    night_folder: &'a Path,
    detection: &'a NightDetection,
    language: &'a str,
    pipelines_dir: &'a Path,
}

type Sent = (NightPrecheck, usize, usize, Vec<uploader::OnlineProposal>);

/// Precheck, enqueue when allowed, drive the queue: the sending of
/// `candidates` for `session_id`, with the proposals of capture metadata
/// (the images already online get theirs after the fact).
fn send_images(
    service: (&str, &str),
    session_id: &str,
    queue_path: &Path,
    candidates: &[ImageCandidate],
    proposing: &Proposing<'_>,
    dry_run: bool,
    quiet: bool,
) -> Result<Sent, Box<dyn Error>> {
    let (portal, token) = service;
    let transport = crate::words::portal_transport()?;
    let online = transport.night_online(portal, token, session_id)?;
    let mut check = night_steps::precheck_night(&transport, portal, token, &online, candidates)?;
    propose(
        &mut check,
        &online,
        proposing.night_folder,
        proposing.detection,
        proposing.language,
        Some(proposing.pipelines_dir),
    );
    if dry_run || !check.quota.allowed {
        return Ok((check, 0, 0, Vec::new()));
    }
    let queue = UploadQueue::open(queue_path)?;
    let enqueued = match precheck::enqueue_checked(&queue, session_id, &check.plans, &check.quota)?
    {
        Ok(outcome) => outcome.inserted,
        Err(_) => 0,
    };
    let sent = drive(portal, token, &queue, session_id, quiet)?;
    let upload = crate::words::upload_transport(portal)?;
    let online_proposals = uploader::propose_for_online_images(&upload, token, &check.plans)?;
    Ok((check, enqueued, sent, online_proposals))
}

fn solve_setup_parts(data: &DataPaths) -> sky_solution::Solvers {
    let rows = tools::detect_tools(&DetectContext::system(
        &data.data_dir,
        &data.data_dir.join("reference-stars.bin"),
    ));
    tools::solvers(&rows)
}

/// The offsets the person gave for the clocks of the logs; a kind left out is
/// found from the frames (`night_analysis::analyze_night_detecting`).
fn forced_offsets(
    phd2: &Option<String>,
    asiair: &Option<String>,
    nina: &Option<String>,
) -> ForcedLogOffsets {
    ForcedLogOffsets {
        phd2: phd2.clone(),
        asiair: asiair.clone(),
        nina: nina.clone(),
    }
}

/// The clocks the logs were read with, in a sentence.
fn print_logs_clock(clock: &Option<LogsUtcOffset>) {
    print_logs_clock_to(clock, &mut std::io::stdout());
}

/// [`print_logs_clock`] on the stream given.
pub(crate) fn print_logs_clock_to(clock: &Option<LogsUtcOffset>, out: &mut dyn std::io::Write) {
    let Some(clock) = clock else { return };
    let kinds = [
        ("PHD2", &clock.phd2, clock.phd2_matched_frames),
        ("ASIAIR", &clock.asiair, clock.asiair_matched_frames),
        ("NINA", &clock.nina, clock.nina_matched_frames),
    ];
    let parts: Vec<String> = kinds
        .iter()
        .filter_map(|(name, offset, matched)| {
            let offset = offset.as_ref()?;
            Some(match matched {
                Some(count) => format!("{name} {offset} ({count} frame(s) of {})", clock.frames),
                None => format!("{name} {offset}"),
            })
        })
        .collect();
    if parts.is_empty() {
        return;
    }
    let _ = writeln!(
        out,
        "Log clock ({}): {}{}.",
        if clock.detected {
            "found from the frames"
        } else {
            "given"
        },
        parts.join(", "),
        if clock.phd2_from.as_deref() == Some("asiair") {
            " -- PHD2 read on the ASIAIR's clock"
        } else {
            ""
        }
    );
}

pub fn send_night(args: &SendNightArgs) -> Outcome {
    let token = token_of(&args.service.token)?;
    let portal = args.service.portal.as_str();
    let data = DataPaths::new(&args.data);
    let choice = solver_choice(&args.solver)?;
    let (detection, local) = night_candidates(&args.night)?;
    // The milestones of the progression the folder holds leave as the history of
    // the target's capture, as the window sends the ones it has just made; the
    // TIFF of a render only with --with-tiff (`render_files`).
    let mut candidates = night_steps::precheck_candidates(
        &local,
        recorded_progression(&args.night.folder, &detection),
        &[],
        args.with_tiff,
    );
    if let Some(main) = &args.main {
        precheck::prefer_main_render(&mut candidates, main);
    }
    let (check, enqueued, sent, online_proposals) = send_images(
        (portal, &token),
        &args.session_id,
        &args.queue,
        &candidates,
        &Proposing {
            night_folder: &args.night.folder,
            detection: &detection,
            language: &args.lang,
            pipelines_dir: &data.pipelines,
        },
        args.dry_run,
        args.json,
    )?;
    let mut answer = SendNightAnswer {
        dry_run: args.dry_run,
        enqueued,
        sent,
        online_proposals,
        ..SendNightAnswer::default()
    };
    let allowed = check.quota.allowed;
    answer.precheck = Some(check);
    if !args.dry_run && allowed {
        let transport = crate::words::portal_transport()?;
        if args.logs {
            let night = night_analysis::analyze_night_detecting(
                &args.night.folder,
                &forced_offsets(
                    &args.phd2_utc_offset,
                    &args.asiair_utc_offset,
                    &args.nina_utc_offset,
                ),
                Some(SkyOptions {
                    stars: reference_stars(Some(&args.data.join("reference-stars.bin"))),
                    ..SkyOptions::default()
                }),
            )?;
            let inputs = night_resume::analysis_inputs(&args.night.folder)?;
            answer.analysis = Some(night_analysis::send_night_analysis(
                &transport,
                portal,
                &token,
                &args.session_id,
                &night,
            )?);
            night_steps::record_sending(
                &data,
                &args.session_id,
                &SentRecord {
                    analysis_fingerprint: Some(inputs.fingerprint),
                    progression_frames: BTreeMap::new(),
                },
            )?;
        }
        if args.solve {
            let online = transport.night_online(portal, &token, &args.session_id)?;
            let solvers = solve_setup_parts(&data);
            let now = astro_sync_core::now_iso8601_utc();
            let setup = SolveSetup {
                solvers: &solvers,
                choice,
                originals_dir: &data.originals,
                work_root: &data.solve,
                timeout: sky_solution::DEFAULT_SOLVE_TIMEOUT,
                cancel: None,
                now: &now,
            };
            answer.solutions = night_steps::solve_night(
                &transport,
                portal,
                &token,
                &online,
                &local,
                &BTreeMap::new(),
                &setup,
            )?;
        }
    }
    let refused = !answer.precheck.as_ref().is_some_and(|c| c.quota.allowed);
    print(args.json, &answer, || {
        if let Some(check) = &answer.precheck {
            print_precheck(check, &args.lang);
        }
        if args.dry_run {
            println!("Dry run: nothing was queued or sent.");
        } else {
            println!("{} image(s) queued, {} sent.", answer.enqueued, answer.sent);
        }
        if let Some(sent) = &answer.analysis {
            print_logs_clock(&sent.logs_utc_offset);
        }
        for proposal in &answer.online_proposals {
            if let Some(note) = &proposal.note {
                println!("Capture {}: {note}", proposal.capture_id);
            }
        }
        for outcome in &answer.solutions {
            if let SolveOutcome::Failed {
                capture_id,
                message,
                ..
            } = outcome
            {
                println!("Sky solution of {capture_id}: {message}");
            }
        }
    })?;
    if refused && !args.dry_run {
        return Err(fail(
            words::say(&args.lang, "wizard.summary.nothingSent", &[])
                .unwrap_or_else(|| "nothing was sent".to_string()),
        ));
    }
    Ok(())
}

/* -------------------------------- send-logs ------------------------------- */

pub fn send_logs(args: &SendLogsArgs) -> Outcome {
    let night = night_analysis::analyze_night_detecting(
        &args.folder,
        &forced_offsets(
            &args.phd2_utc_offset,
            &args.asiair_utc_offset,
            &args.nina_utc_offset,
        ),
        Some(SkyOptions {
            stars: reference_stars(args.stars.as_deref()),
            ..SkyOptions::default()
        }),
    )?;
    if args.dry_run {
        return print(args.json, &night, || {
            println!(
                "Report: {} step(s), {} quality row(s). Nothing was sent.",
                night.report.acquisition_steps.len(),
                night.report.quality.len()
            );
            print_logs_clock(&night.logs_utc_offset);
        });
    }
    let (portal, token) = service(args.portal.as_ref(), args.token.as_ref())?;
    let session_id = args
        .session_id
        .as_deref()
        .ok_or_else(|| fail("provide --session-id"))?;
    let inputs = night_resume::analysis_inputs(&args.folder)?;
    let sent = night_analysis::send_night_analysis(
        &crate::words::portal_transport()?,
        &portal,
        &token,
        session_id,
        &night,
    )?;
    if let Some(data) = &args.data {
        night_steps::record_sending(
            &DataPaths::new(data),
            session_id,
            &SentRecord {
                analysis_fingerprint: Some(inputs.fingerprint),
                progression_frames: BTreeMap::new(),
            },
        )?;
    }
    print(args.json, &sent, || {
        println!(
            "Report sent: it replaces the night's analysis. The raw logs stay on this computer."
        );
        print_logs_clock(&sent.logs_utc_offset);
    })
}

/* ------------------------------- image-texts ------------------------------ */

/// The generated texts of the images of captures already online, in the ten
/// languages: every image whose digest a record of Astro Sync under `--root`
/// holds; the others are said and left.
pub fn image_texts(args: &crate::cli_night::ImageTextsArgs) -> Outcome {
    use astro_sync_core::image_texts;
    let (portal, token) = service(args.portal.as_ref(), args.token.as_ref())?;
    let transport = crate::words::portal_transport()?;
    let mut local = image_texts::local_renders(&args.root);
    let recorded = local.records.len();
    let user_pipelines = args.data.as_ref().map(|data| data.join("pipelines"));
    let pipelines = astro_sync_core::pipelines::read_manifests(
        None,
        user_pipelines.as_deref(),
        crate::words::requested_language(),
    );
    let mut lines = Vec::new();
    for capture_id in &args.capture_ids {
        lines.extend(image_texts::regenerate_capture(
            &transport,
            &portal,
            &token,
            capture_id,
            &mut local,
            &pipelines,
            args.dry_run,
        )?);
    }
    print(args.json, &lines, || {
        println!(
            "{} render(s) recorded under the folder; {} image(s) read.",
            recorded,
            lines.len()
        );
        for line in &lines {
            println!(
                "{} {} [{}{}] {} -> {}",
                line.capture_id,
                line.image_id,
                line.result,
                if line.title_kept {
                    ", own title kept"
                } else {
                    ""
                },
                line.title_before.as_deref().unwrap_or("-"),
                line.generated_en.as_deref().unwrap_or("-"),
            );
        }
    })
}

/* ---------------------------------- solve --------------------------------- */

pub fn solve(args: &SolveArgs) -> Outcome {
    let data = DataPaths::new(&args.data);
    let solvers = solve_setup_parts(&data);
    let choice = solver_choice(&args.solver)?;
    let now = astro_sync_core::now_iso8601_utc();
    if args.missing {
        let (portal, token) = service(args.portal.as_ref(), args.token.as_ref())?;
        let transport = crate::words::portal_transport()?;
        let missing = sky_solution::list_missing(&transport, &portal, &token)?;
        if args.dry_run {
            return print(args.json, &missing, || {
                println!("{} capture(s) without a sky solution.", missing.len());
                for capture in &missing {
                    println!("  {}  {}", capture.id, capture.original_name);
                }
            });
        }
        let setup = SolveSetup {
            solvers: &solvers,
            choice,
            originals_dir: &data.originals,
            work_root: &data.solve,
            timeout: sky_solution::DEFAULT_SOLVE_TIMEOUT,
            cancel: None,
            now: &now,
        };
        // The images of the working folder: solved where they are, pointed where
        // their master points.
        let local = args
            .root
            .as_deref()
            .map(|root| sky_solution::local_results(root, &missing))
            .unwrap_or_default();
        let here: BTreeMap<String, PathBuf> = local
            .iter()
            .map(|(digest, found)| (digest.clone(), found.path.clone()))
            .collect();
        let mut outcomes = Vec::new();
        for capture in &missing {
            let source = capture.solve_from(&here);
            let hints = local.get(&capture.sha256.to_ascii_lowercase()).map_or_else(
                || capture.hints(),
                |found| found.hints.completed_by(capture.hints()),
            );
            outcomes.push(sky_solution::solve_and_send(
                &transport, &portal, &token, &source, hints, &setup,
            )?);
        }
        return print(args.json, &outcomes, || print_outcomes(&outcomes));
    }
    let image = args
        .image
        .as_ref()
        .ok_or_else(|| fail("provide --image or --missing"))?;
    let hints = SolveHints {
        ra_deg: args.ra,
        dec_deg: args.dec,
        pixel_scale_arcsec: args.scale,
    };
    let work = data.solve.join("command-line");
    let job = SolveJob {
        image,
        hints,
        work_dir: &work,
        timeout: sky_solution::DEFAULT_SOLVE_TIMEOUT,
        cancel: None,
        solved_at: &now,
    };
    let solved = sky_solution::solve_image(&solvers, choice, &job);
    let outcome = match (solved, &args.capture_id, args.dry_run) {
        (Ok(solution), Some(capture_id), false) => {
            let (portal, token) = service(args.portal.as_ref(), args.token.as_ref())?;
            let digest = astro_sync_core::scanner::file_sha256(image)?;
            let written = sky_solution::send_solution(
                &crate::words::portal_transport()?,
                &portal,
                &token,
                capture_id,
                &digest,
                &solution,
            )?;
            SolveOutcome::Sent {
                capture_id: capture_id.clone(),
                solution,
                written,
            }
        }
        (Ok(solution), capture_id, _) => {
            return print(args.json, &solution, || {
                println!(
                    "Solution ({:?}): centre {:.5}° {:+.5}°, {:.3}″/px, rotation {:.2}°{}",
                    solution.source,
                    solution.ra_deg,
                    solution.dec_deg,
                    solution.pixel_scale_arcsec,
                    solution.rotation_deg,
                    if capture_id.is_some() {
                        " (dry run: nothing was sent)"
                    } else {
                        ""
                    }
                );
            });
        }
        (Err(failure), capture_id, _) => SolveOutcome::Failed {
            capture_id: capture_id.clone().unwrap_or_default(),
            message: failure.sentence(),
            failure,
        },
    };
    print(args.json, &outcome, || {
        print_outcomes(std::slice::from_ref(&outcome))
    })
}

fn print_outcomes(outcomes: &[SolveOutcome]) {
    for outcome in outcomes {
        match outcome {
            SolveOutcome::Sent { capture_id, .. } => {
                println!("{capture_id}: solved and sent")
            }
            SolveOutcome::Failed {
                capture_id,
                message,
                ..
            } => println!("{capture_id}: {message}"),
        }
    }
}

/* ---------------------------- progression-images -------------------------- */

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressionPlan {
    families: Vec<astro_sync_core::analysis::fits::AcquisitionFamily>,
    chosen: Option<String>,
    work_dir: Option<PathBuf>,
}

fn allowed_progression(
    portal: Option<&String>,
    token: Option<&PathBuf>,
) -> Result<bool, Box<dyn Error>> {
    let (portal, token) = service(portal, token)?;
    Ok(crate::words::portal_transport()?
        .space_storage(&portal, &token)?
        .capabilities
        .analysis_progression)
}

pub fn progression_images(args: &ProgressionImagesArgs) -> Outcome {
    record_filter(
        &args.night,
        &args.target_folder,
        args.filter.as_deref(),
        args.dry_run,
    )?;
    let mut families = progression_images::target_families(&args.night, &args.target_folder);
    families.sort_by_key(|f| std::cmp::Reverse(f.frame_paths.len()));
    let family = match &args.family {
        Some(key) => families.iter().find(|f| &f.key == key).cloned(),
        None => families.first().cloned(),
    };
    if args.dry_run {
        let plan = ProgressionPlan {
            chosen: family.as_ref().map(|f| f.key.clone()),
            work_dir: family
                .clone()
                .map(|f| ProgressionJob::new(&args.night, &args.target_folder, f).work_dir),
            families,
        };
        return print(args.json, &plan, || {
            for f in &plan.families {
                println!("{} — {} frame(s)", f.key, f.frame_paths.len());
            }
            println!("Dry run: Siril was not started.");
        });
    }
    let family = family.ok_or_else(|| fail("no frame family in lights/"))?;
    let allowed = allowed_progression(args.portal.as_ref(), args.token.as_ref())?;
    let rows = tools::detect_tools(&DetectContext::system(
        &args.data,
        &args.data.join("reference-stars.bin"),
    ));
    let siril = tools::siril_for_progression(&rows);
    let job = ProgressionJob::new(&args.night, &args.target_folder, family);
    let made = progression_images::run_progression_images(
        &job,
        allowed,
        siril.as_ref().map(|(p, v)| (p.as_path(), v.as_str())),
        &astro_sync_core::stacking::free_bytes,
        Duration::from_secs(3 * 3600),
        None,
    )
    .map_err(|failure| fail(format!("progression impossible: {failure:?}")))?;
    print(args.json, &made, || {
        for image in &made.images {
            println!(
                "Milestone {} — {} frame(s): {}",
                image.index,
                image.frame_count,
                image.path.display()
            );
        }
    })
}

/* ----------------------------- measure-master ----------------------------- */

/// What the window says of a refusal, in a sentence.
fn master_refusal_sentence(code: &str) -> &'static str {
    match code {
        "stretched" => {
            "Stretched image: measure the linear master, as it comes out of the stacking, before any stretch or processing (FITS or XISF)."
        }
        "too_few_stars" => {
            "Fewer than twenty isolated, measurable stars were found in this image: nothing is measured. Check that this is the master of the stacking."
        }
        "unsupported_format" => {
            "This file is neither a FITS nor an XISF. Give the linear master of the stacking (FITS or XISF); a JPEG or a TIFF cannot be measured."
        }
        "not_an_image" => {
            "This file holds no measurable image: a two-dimensional image, in grey levels or with three channels, is expected."
        }
        "reserved" => "Measuring a master is reserved to Advanced Sync+.",
        "python_missing" => {
            "Astro Sync's Python environment is not installed: run “astro-sync tools --install”."
        }
        "night_missing" => "The folder of this target cannot be found in the night’s folder.",
        _ => {
            "This file could not be read: it is damaged, or compressed in a way Astro Sync does not read."
        }
    }
}

/// "Measure a master…" on the command line: the window's measure
/// (`master_quality`), the same script, the same `Outcome` printed as is. With
/// `--night` and `--target-folder` the figures are kept for the target's images,
/// as the window does (under `analysis.logs`, from the cached capabilities).
pub fn measure_master(args: &MeasureMasterArgs) -> Outcome {
    let python = python_env::env_python(&args.data);
    let script = args.stacking.join("star_quality.py");
    let outcome = match (&args.night, &args.target_folder) {
        (Some(night), Some(target_folder)) => {
            let entitled =
                gui_bridge::load_capabilities_cache(&args.data.join("capabilities-cache.json"))?
                    .is_some_and(|cache| cache.capabilities.analysis_logs);
            master_quality::measure_for_target(
                night,
                target_folder,
                &args.file,
                entitled,
                &python,
                &script,
                None,
            )
        }
        _ => master_quality::measure_alone(&args.file, &python, &script, None),
    }
    .map_err(|failure| match failure {
        master_quality::Failure::NotRun(reason) => fail(format!("measure impossible: {reason}")),
        master_quality::Failure::NoAnswer(reason) => {
            fail(format!("the measure did not answer: {reason}"))
        }
    })?;
    print(args.json, &outcome, || match &outcome {
        master_quality::Outcome::Measured { master, quality } => {
            println!(
                "Master measured: {master}. Star width {} px, eccentricity {}, {} stars.",
                quality.fwhm_pixels,
                quality
                    .eccentricity
                    .map_or_else(|| "inconnue".to_string(), |e| e.to_string()),
                quality.star_count
            );
        }
        master_quality::Outcome::Refused { code } => {
            println!("{}", master_refusal_sentence(code));
        }
    })?;
    match outcome {
        master_quality::Outcome::Measured { .. } => Ok(()),
        master_quality::Outcome::Refused { code } => {
            Err(fail(format!("master not measured: {code}")))
        }
    }
}

/* ---------------------------------- tools --------------------------------- */

pub(crate) fn detect_context(data: &Path, stacking: Option<&Path>) -> DetectContext {
    DetectContext::system(data, &data.join("reference-stars.bin")).with_stacking(stacking)
}

/// The state of a row of "Dependencies" as the window's badge says it.
fn tool_state_text(row: &tools::ToolRow, locale: &str) -> String {
    let version = row.version.clone().unwrap_or_default();
    let (key, plain) = match (row.state, row.version.is_some()) {
        (tools::ToolState::Present, true) => ("deps.badge.present", "Present"),
        (tools::ToolState::Present | tools::ToolState::NotChecked, _) => {
            ("deps.badge.presentPlain", "Present")
        }
        (tools::ToolState::Missing, _) if row.required => ("deps.badge.missing", "Missing"),
        (tools::ToolState::Missing, _) => ("deps.badge.notInstalled", "Not installed"),
        (tools::ToolState::Outdated, _) => ("deps.badge.outdated", "Too old"),
        (tools::ToolState::Downloaded, _) => ("deps.badge.downloaded", "Downloaded, to install"),
    };
    words::say(locale, key, &[("version", &version)]).unwrap_or_else(|| plain.to_string())
}

pub fn tools(args: &ToolsArgs) -> Outcome {
    let context = detect_context(&args.data, args.stacking.as_deref());
    if args.verify {
        let resources = context
            .stacking
            .as_ref()
            .ok_or_else(|| fail("provide --stacking (the folder of the stacking scripts)"))?;
        let check =
            python_env::verify_environment(&args.data, resources, EnvTimeouts::default().check);
        return print(args.json, &check, || match &check {
            Ok(check) => println!("Python environment {}: checked.", check.python_version),
            Err(failure) => println!("Python environment: {failure:?}"),
        });
    }
    let rows = tools::detect_tools(&context);
    if !args.install {
        let locale = words::requested_language();
        return print(args.json, &rows, || {
            let names: Vec<String> = rows
                .iter()
                .map(|row| tools::localized(&row.name, locale))
                .collect();
            let width = names
                .iter()
                .map(|name| name.chars().count())
                .max()
                .unwrap_or(0);
            for (row, name) in rows.iter().zip(&names) {
                let padding = " ".repeat(width.saturating_sub(name.chars().count()));
                println!("{name}{padding}  {}", tool_state_text(row, locale));
            }
        });
    }
    let wanted: Vec<_> = tools::to_install(&rows);
    let environment = rows
        .iter()
        .find(|r| r.id == "python-env")
        .is_some_and(|r| r.installable && r.state != tools::ToolState::Present);
    if args.dry_run {
        let mut ids: Vec<&str> = wanted.iter().map(|r| r.id.as_str()).collect();
        if environment {
            ids.push("python-env");
        }
        return print(args.json, &ids, || {
            if ids.is_empty() {
                println!(
                    "Nothing to download: no missing required tool has a download for this system."
                );
            }
            for id in &ids {
                println!("would be downloaded and installed: {id}");
            }
        });
    }
    let fetcher = HttpToolFetcher::new().map_err(fail)?;
    let mut results = BTreeMap::new();
    for row in wanted {
        let result = tools::download_and_install(
            &args.data,
            &row.id,
            row.download.as_ref(),
            &fetcher,
            None,
            &mut |_| {},
        );
        results.insert(
            row.id.clone(),
            result.map(|_| ()).map_err(|f| format!("{f:?}")),
        );
    }
    if environment {
        let result = python_env::install_environment(
            &context,
            context.stacking.as_ref(),
            EnvTimeouts::default(),
            &CancelFlag::new(),
            &mut |progress| {
                eprintln!(
                    "[{}/{}] {:?} {}/{}",
                    progress.index,
                    progress.total,
                    progress.step,
                    progress.packages_done,
                    progress.packages_total
                );
            },
        );
        results.insert(
            "python-env".to_string(),
            result.map(|_| ()).map_err(|f| format!("{f:?}")),
        );
    }
    print(args.json, &results, || {
        for (id, result) in &results {
            println!(
                "{id}: {}",
                if result.is_ok() {
                    "downloaded and installed"
                } else {
                    "failed"
                }
            );
        }
    })
}

/* ----------------------------- list-pipelines ----------------------------- */

pub fn list_pipelines(args: &ListPipelinesArgs) -> Outcome {
    let user = args.data.as_ref().map(|d| d.join("pipelines"));
    let resources = args.stacking.as_deref().and_then(StackingResources::find);
    let environment = match (&args.data, &resources) {
        (Some(data), Some(resources)) => StackingEnvironment::find(data, &resources.script()),
        _ => None,
    };
    let list = pipelines::list_pipelines(
        environment.as_ref(),
        args.shipped.as_deref(),
        user.as_deref(),
        &args.lang,
    );
    let store = args
        .data
        .as_deref()
        .map(pipeline_trust::load)
        .unwrap_or_default();
    let list = pipelines::with_trust(list, &store);
    print(args.json, &list, || {
        for pipeline in &list.pipelines {
            println!(
                "{:<22} {}",
                pipeline.id,
                pipeline.display_name.clone().unwrap_or_default()
            );
        }
        for ignored in &list.ignored {
            println!("ignored: {} ({})", ignored.folder, ignored.reason);
        }
    })
}

/* ---------------------------------- stack --------------------------------- */

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StackPlanned {
    job: stacking::StackJob,
    command: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StackPlan {
    jobs: Vec<StackPlanned>,
    set_aside: Vec<stacking::SetAsideFamily>,
    /// The lights outside the night, left out of the integration.
    outside: Vec<PathBuf>,
    /// About 210 MB per light frame in the scratch folder.
    needed_bytes: u64,
    available_bytes: Option<u64>,
    /// The night's folder, whose disk holds the scratch and is measured
    /// (as in the refusal for lack of room).
    folder: String,
    /// Why the real stacking would be refused (the pipeline row of
    /// "Dependencies" is missing, several renders asked of a space without
    /// variants); absent when the real run would not be refused for that.
    /// The stacking is local and never gated by the offer.
    #[serde(skip_serializing_if = "Option::is_none")]
    refusal: Option<StackRefusal>,
}

/// A refusal said before the real stacking, in the person's language.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StackRefusal {
    /// `tool_missing` or `one_render_only`.
    code: &'static str,
    message: String,
}

/// The question the real stacking asks first, asked of a dry run: the same
/// gate (`capabilities` unknown: the gate without a space, as the real run).
fn dry_run_refusal(
    capabilities: Option<&gui_bridge::Capabilities>,
    stacking_pipeline: bool,
    plan: &stacking::TargetPlan,
    language: &str,
) -> Option<StackRefusal> {
    let gate = StackGate::of(capabilities, stacking_pipeline);
    let failure = plan.jobs.iter().find_map(|job| gate.refusal(job))?;
    let code = match failure {
        stacking::StackFailure::ToolMissing => "tool_missing",
        _ => "one_render_only",
    };
    Some(StackRefusal {
        code,
        message: words::stack_failure_text(&failure, language),
    })
}

/// The space's capabilities: from the service when `--portal` and `--token`
/// are given, else from Astro Sync's cache (`capabilities-cache.json` of
/// its data, the one the window reads).
fn stacking_capabilities(
    args: &StackArgs,
) -> Result<Option<gui_bridge::Capabilities>, Box<dyn Error>> {
    if args.portal.is_some() || args.token.is_some() {
        let (portal, token) = service(args.portal.as_ref(), args.token.as_ref())?;
        return Ok(Some(
            crate::words::portal_transport()?
                .space_storage(&portal, &token)?
                .capabilities,
        ));
    }
    Ok(
        gui_bridge::load_capabilities_cache(&args.data.join("capabilities-cache.json"))?
            .map(|cache| cache.capabilities),
    )
}

fn stack_event_line(event: &StackEvent) {
    match event {
        StackEvent::Family {
            family,
            index,
            total,
        } => eprintln!(
            "Stacking {index}/{total}{}",
            family
                .as_ref()
                .map(|f| format!(" ({f})"))
                .unwrap_or_default()
        ),
        StackEvent::Step { step, index, total } => eprintln!("  [{index}/{total}] {step}"),
        StackEvent::Output { file, .. } => eprintln!("  written: {file}"),
        StackEvent::Registration {
            unit,
            frames,
            unregistered,
            field_rotation_degrees,
            framing,
            ..
        } => {
            let plan = *unit == RegisteredFrames::Masters;
            if let (Framing::Full, Some(degrees)) = (framing, field_rotation_degrees) {
                eprintln!(
                    "  The field rotates by {degrees:.0}° {}: full field kept",
                    if plan {
                        "over all the nights of the plan"
                    } else {
                        "during the night"
                    }
                );
            }
            if !unregistered.is_empty() {
                eprintln!(
                    "  {} {} of {frames} could not be aligned and were left out: {}",
                    unregistered.len(),
                    if plan { "integration(s)" } else { "frame(s)" },
                    unregistered.join(", ")
                );
            }
        }
        StackEvent::Crop {
            mode,
            applied: true,
            height,
            width,
            frame_height,
            frame_width,
            kept_share,
            ..
        } => eprintln!(
            "  Cropped to {}: {width} × {height} of {frame_width} × {frame_height} ({:.0}% of the field kept)",
            if mode == "common" {
                "the area every night covers"
            } else {
                "the edges with data"
            },
            kept_share * 100.0
        ),
        StackEvent::Crop { .. } => {}
        StackEvent::Coverage { no_dual_band_share } => eprintln!(
            "  The corners lack Hα / OIII: {:.0}% of the image has no dual-band data",
            no_dual_band_share * 100.0
        ),
        StackEvent::Progress { .. } => {
            if let Some(text) = event.progress_text() {
                eprintln!("  {text}");
            }
        }
    }
}

/// Siril's percentage and last line (`StackEvent::Progress`, the same words
/// as the window's cadre), at most once a second and only when they changed:
/// a long integration would otherwise fill the terminal.
pub(crate) fn stack_progress_printer() -> impl FnMut(&StackEvent) {
    let mut last: Option<(std::time::Instant, String)> = None;
    move |event| {
        let Some(text) = event.progress_text() else {
            stack_event_line(event);
            return;
        };
        let due = last
            .as_ref()
            .is_none_or(|(at, shown)| *shown != text && at.elapsed() >= Duration::from_secs(1));
        if due {
            stack_event_line(event);
            last = Some((std::time::Instant::now(), text));
        }
    }
}

/// The pipelines of the target stacked (chosen target by target, as
/// in the window): its `--target-pipeline TARGET=ID,ID` when one names its
/// folder or its id, else `--pipeline`; `--no-render` adds "No
/// processing" (`linear`), alone or after the renders.
pub fn target_pipeline_ids(
    target_folder: &str,
    target_id: &str,
    pipelines: &[String],
    target_pipelines: &[String],
    no_render: bool,
) -> Result<Vec<String>, Box<dyn Error>> {
    let mut chosen: Option<Vec<String>> = None;
    for raw in target_pipelines {
        let (target, ids) = raw
            .split_once('=')
            .ok_or_else(|| format!("--target-pipeline expects TARGET=ID,ID (got: {raw:?})"))?;
        let ids: Vec<String> = ids
            .split(',')
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .collect();
        if ids.is_empty() {
            return Err(format!("--target-pipeline {target:?}: no pipeline").into());
        }
        let target = target.trim();
        if target == target_folder || target == target_id {
            chosen = Some(ids);
        }
    }
    let mut ids = chosen.unwrap_or_else(|| pipelines.to_vec());
    if no_render && !ids.iter().any(|id| id == pipelines::LINEAR_PIPELINE) {
        ids.push(pipelines::LINEAR_PIPELINE.to_string());
    }
    Ok(ids)
}

/// `--main <id>`: the pipeline whose render is the capture's main image is
/// made first (the first render made is the capture, the others its
/// variants). It must be one of the renders asked for.
pub(crate) fn main_first(
    mut ids: Vec<String>,
    main: Option<&str>,
) -> Result<Vec<String>, Box<dyn Error>> {
    let Some(main) = main.map(str::trim).filter(|id| !id.is_empty()) else {
        return Ok(ids);
    };
    if main == pipelines::LINEAR_PIPELINE {
        return Err("--main: \"No processing\" (linear) is never the main image".into());
    }
    let Some(place) = ids.iter().position(|id| id == main) else {
        return Err(format!(
            "--main {main:?} is not among the pipelines asked for ({})",
            ids.join(", ")
        )
        .into());
    };
    let chosen = ids.remove(place);
    ids.insert(0, chosen);
    Ok(ids)
}

/// Applies `--leave-out <file>` to the plan, and says what it did on the error
/// output: how many poses were left out, and the names that are no pose of
/// this night (which stop nothing).
fn leave_out(
    plan: &mut stacking::TargetPlan,
    file: &std::path::Path,
    args: &StackArgs,
    window: Option<&astro_sync_core::detection::NightWindow>,
) -> Result<(), Box<dyn std::error::Error>> {
    let list = std::fs::read_to_string(file)
        .map_err(|error| format!("could not read {}: {error}", file.display()))?;
    let (families, _) = stacking::stack_families_within(&args.night, &args.target_folder, window);
    let poses: Vec<PathBuf> = families.into_iter().flat_map(|f| f.frames).collect();
    let (named, unknown) = astro_sync_core::pose_stats::poses_named(&list, &poses);
    // A stacking that runs keeps the list for the night's report; a dry run
    // writes nothing.
    let apply = if args.dry_run {
        astro_sync_core::pose_stats::apply_discard
    } else {
        astro_sync_core::pose_stats::apply_discard_and_record
    };
    let left = apply(plan, &named, &args.night, &args.target_folder, window);
    let name = file.file_name().map_or_else(
        || file.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    eprintln!(
        "{left} frame{} left out according to {name} (they stay in lights/, nothing is deleted)",
        if left > 1 { "s" } else { "" },
    );
    if !unknown.is_empty() {
        eprintln!(
            "{} name{} in {name} {}: {}",
            unknown.len(),
            if unknown.len() > 1 { "s" } else { "" },
            if unknown.len() > 1 {
                "are not frames of this night"
            } else {
                "is not a frame of this night"
            },
            unknown.join(", ")
        );
    }
    Ok(())
}

/// The paths of the command made absolute: the stacking script starts in its
/// own work folder, so a relative path would mean another folder there.
fn stack_args_made_absolute(args: &StackArgs) -> StackArgs {
    let mut absolute_args = args.clone();
    absolute_args.night = absolute(&args.night);
    absolute_args.data = absolute(&args.data);
    absolute_args.stacking = args.stacking.as_deref().map(absolute);
    absolute_args.leave_out = args.leave_out.as_deref().map(absolute);
    absolute_args
}

/// A path made absolute against the current folder (itself when that cannot
/// be known).
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The images of the device the stacking takes as the source of their
/// families, as the window does by default: the ones the person named
/// (`--device`), none with `--restack`, else the image the detection of
/// the target's `device/` folder takes for each family it covers.
fn device_uses(args: &StackArgs) -> Vec<DeviceUse> {
    if args.restack {
        return Vec::new();
    }
    if !args.device.is_empty() {
        return args
            .device
            .iter()
            .map(|file| DeviceUse { file: file.clone() })
            .collect();
    }
    if !args
        .night
        .join(&args.target_folder)
        .join(working_folder::DEVICE_DIR)
        .is_dir()
    {
        return Vec::new();
    }
    let input = DetectionInput {
        evening_date: Some(args.evening_date.clone()),
        night_started_at: None,
        longitude: args.longitude,
        targets: vec![DetectionTarget {
            target_id: args.target_id.clone(),
            designation: args.target_folder.clone(),
            aliases: Vec::new(),
            ticked: true,
            online_final: false,
            stacking_planned: true,
        }],
    };
    let Ok(detection) = detection::detect_night(&args.night, &input, &DetectOptions::default())
    else {
        return Vec::new();
    };
    detection
        .targets
        .iter()
        .find(|target| target.target_id == args.target_id)
        .and_then(|target| target.device.as_ref())
        .map(|device| {
            device
                .families
                .iter()
                .filter(|family| family.default_source == device_master::Source::Device)
                .filter_map(|family| family.use_file.clone())
                .map(|file| DeviceUse { file })
                .collect()
        })
        .unwrap_or_default()
}

/// `--filter`: records the filter in the target folder's identity (Astro Sync's
/// own file), where the stacking, the progression and the plans read it. A dry
/// run writes nothing.
fn record_filter(
    night: &Path,
    target_folder: &str,
    filter: Option<&str>,
    dry_run: bool,
) -> Result<(), Box<dyn Error>> {
    let Some(filter) = filter.map(str::trim).filter(|f| !f.is_empty()) else {
        return Ok(());
    };
    let folder = night.join(target_folder);
    if dry_run {
        eprintln!("Dry run: the filter {filter} would be recorded for {target_folder}.");
        return Ok(());
    }
    if astro_sync_core::working_folder::read_target_identity(&folder).is_none() {
        return Err(fail(format!(
            "--filter: {target_folder} holds no target identity (.astro-sync-target.json)"
        )));
    }
    astro_sync_core::working_folder::set_target_filter(&folder, filter)
        .map_err(|e| fail(format!("could not record the filter: {e}")))?;
    Ok(())
}

pub fn stack(args: &StackArgs) -> Outcome {
    let args = &stack_args_made_absolute(args);
    record_filter(
        &args.night,
        &args.target_folder,
        args.filter.as_deref(),
        args.dry_run,
    )?;
    let data = DataPaths::new(&args.data);
    let trusted: BTreeMap<String, String> = args
        .trusted
        .iter()
        .map(|raw| pair(raw))
        .collect::<Result<_, _>>()?;
    let store = pipeline_trust::load(&args.data);
    let context = detect_context(&args.data, args.stacking.as_deref());
    let listed = pipelines::read_manifests(None, Some(&data.pipelines), "fr");
    let ids = main_first(
        target_pipeline_ids(
            &args.target_folder,
            &args.target_id,
            &args.pipelines,
            &args.target_pipelines,
            args.no_render,
        )?,
        args.main.as_deref(),
    )?;
    let finishes: BTreeMap<String, PipelineFinish> = args
        .finishes
        .iter()
        .map(|raw| pipeline_finish(raw))
        .collect::<Result<_, _>>()?;
    let frame = frame_size(&args.frame_size)?;
    let choices: Vec<PipelineChoice> = ids
        .iter()
        .map(|id| PipelineChoice {
            pipeline_id: id.clone(),
            trusted_sha256: trusted
                .get(id)
                .cloned()
                .or_else(|| listed.get(id).and_then(|p| store.trusted_sha256(p))),
            finish: finishes.get(id).copied(),
        })
        .collect();
    let capabilities = stacking_capabilities(args)?;
    let variant_limit = capabilities
        .as_ref()
        .map_or(stacking::UNLIMITED, |c| c.capture_variant_limit);
    // The lights outside the night are left out of the integration.
    let window =
        astro_sync_core::detection::NightWindow::of_evening(&args.evening_date, args.longitude);
    let device = device_uses(args);
    let mut plan = stacking::plan_target_with_device(
        &args.night,
        &args.target_folder,
        &args.evening_date,
        &choices,
        variant_limit,
        window.as_ref(),
        &device,
    )
    .with_contrast(args.contrast)
    .with_retouch(astro_sync_core::stacking::Retouch {
        colour: args.colour,
        soft_stars: args.soft_stars,
    })
    .with_polished_engine(args.polished_engine)
    .with_frame(frame);
    // `--leave-out`: the poses named in a file are left out of every
    // stacking, as "Leave out the flagged frames" does in the window;
    // nothing is moved or deleted.
    if let Some(file) = &args.leave_out {
        leave_out(&mut plan, file, args, window.as_ref())?;
    }
    // The night keeps its flats by filter and has none for this family's:
    // said before anything runs, the family is stacked without.
    for job in &plan.jobs {
        if let Some(filter) = &job.flats_missing_for {
            eprintln!(
                "No flat for the {filter} filter: stacking without flats{}",
                job.family
                    .as_ref()
                    .map(|f| format!(" ({f})"))
                    .unwrap_or_default()
            );
        }
    }
    let rows = tools::detect_tools(&context);
    let path_of = |id: &str| {
        rows.iter()
            .find(|r| r.id == id && r.state == tools::ToolState::Present)
            .and_then(|r| r.path.clone())
    };
    let (siril, starnet, graxpert) = (path_of("siril"), path_of("starnet2"), path_of("graxpert"));
    if args.dry_run {
        let needed_bytes = plan
            .jobs
            .iter()
            .map(|job| job.light_count() * stacking::BYTES_PER_LIGHT)
            .max()
            .unwrap_or(0);
        let summary = StackPlan {
            jobs: plan
                .jobs
                .iter()
                .map(|job| StackPlanned {
                    command: stacking::stack_args(
                        job,
                        std::slice::from_ref(&data.pipelines),
                        siril.as_deref(),
                        starnet.as_deref(),
                        graxpert.as_deref(),
                    ),
                    job: job.clone(),
                })
                .collect(),
            set_aside: plan.set_aside.clone(),
            outside: plan.outside.clone(),
            needed_bytes,
            available_bytes: stacking::free_bytes(&args.night),
            folder: args.night.display().to_string(),
            refusal: dry_run_refusal(
                capabilities.as_ref(),
                tools::tools_present(&rows).stacking_pipeline,
                &plan,
                words::requested_language(),
            ),
        };
        return print(args.json, &summary, || {
            for planned in &summary.jobs {
                if let Some(image) = &planned.job.device_master {
                    println!(
                        "Device image taken as the source: {} (no frame stacked for this family)",
                        file_name_of(image)
                    );
                }
                println!("Output: {}", planned.job.output_dir.display());
                println!("astro_sync_stack.py {}", planned.command.join(" "));
            }
            for family in &summary.set_aside {
                println!(
                    "Not stacked (a single stacking without a variant): {} ({} frames)",
                    family.label, family.frames
                );
            }
            if let Some(refusal) = &summary.refusal {
                println!(
                    "Warning: the real stacking would be refused: {}",
                    refusal.message
                );
            }
            println!("Dry run: nothing was run.");
        });
    }
    // The stacking is local and never gated: without a space (no account,
    // its rights not read), it runs all the same, nothing limiting the renders.
    let gate = StackGate::of(
        capabilities.as_ref(),
        tools::tools_present(&rows).stacking_pipeline,
    );
    let resources = context.stacking.clone();
    let stacker = PythonStacker {
        python: python_env::env_python(&args.data),
        script: resources
            .as_ref()
            .map(StackingResources::script)
            .unwrap_or_default(),
        pipelines_dirs: vec![data.pipelines.clone()],
        siril,
        starnet,
        graxpert,
        timeout: Duration::from_secs(6 * 3600),
    };
    let outcome = stacking::run_target_stacking(
        &stacker,
        &plan,
        &args.night,
        gate,
        &stacking::free_bytes,
        &CancelFlag::new(),
        Some(&args.data.join(stacking::LOGS_DIR)),
        &mut stack_progress_printer(),
    )
    .map_err(|failure| {
        fail(words::stack_failure_text(
            &failure,
            words::requested_language(),
        ))
    })?;
    let candidates = stacking::stacked_candidates(&outcome, &args.target_id);
    print(
        args.json,
        &serde_json::json!({ "outcome": outcome, "candidates": candidates }),
        || {
            for render in &outcome.renders {
                println!(
                    "{}: {} ({} bytes, SHA-256 {})",
                    render.pipeline_id,
                    render.path.display(),
                    render.size,
                    render.sha256
                );
            }
            for family in &outcome.set_aside {
                println!("Not stacked: {} ({} frames)", family.label, family.frames);
            }
            for master in &outcome.integrations {
                println!(
                    "Master kept: {} ({} bytes)",
                    master.path.display(),
                    master.bytes
                );
            }
            if outcome.outside_frames > 0 {
                println!(
                    "{} frame(s) outside the night, not integrated.",
                    outcome.outside_frames
                );
            }
        },
    )
}

/* ----------------------------- assistant-prompt --------------------------- */

pub fn assistant_prompt(args: &AssistantPromptArgs) -> Outcome {
    if args.file {
        let text = assistant_prompt::assistant_text(args.multi_night);
        return print(
            args.json,
            &serde_json::json!({
                "file": astro_sync_core::working_folder::ASSISTANT_FILE_NAME,
                "sha256": assistant_prompt::assistant_file_sha256_for(args.multi_night),
                "text": text,
            }),
            || {
                print!("{text}");
            },
        );
    }
    let mut names = std::collections::BTreeMap::new();
    for pair in &args.names {
        let Some((folder, name)) = pair.split_once('=') else {
            return Err(fail(format!(
                "--name expects <folder>=<name>, got “{pair}”"
            )));
        };
        names.insert(folder.trim().to_string(), name.trim().to_string());
    }
    let input = PromptInput {
        root: args.root.clone().unwrap_or_default(),
        night_folder: args.night_folder.clone().unwrap_or_default(),
        targets: args.targets.clone(),
        focus: args.focus.clone(),
        messages: args.messages.clone(),
        names,
    };
    let prompt = assistant_prompt::build_prompt(&args.lang, &input);
    print(
        args.json,
        &serde_json::json!({ "language": args.lang, "prompt": prompt }),
        || {
            print!("{prompt}");
        },
    )
}

#[cfg(test)]
mod tests {
    use super::{describe_proposals, main_first};
    use astro_sync_core::metadata::{Proposals, SensorProposal};

    #[test]
    fn the_proposals_are_described_in_english_only() {
        let proposals = Proposals {
            calibration: Some("20 darks".to_string()),
            sensor: Some(SensorProposal {
                temperature_c: Some(-10.0),
                binning: Some(1),
            }),
            ..Proposals::default()
        };
        let line = describe_proposals(&proposals);
        assert_eq!(line, "calibration (20 darks), sensor");
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| (*id).to_string()).collect()
    }

    #[test]
    fn the_pipeline_named_main_is_made_first_the_others_keep_their_order() {
        let asked = ids(&["raw", "dualband-correction", "hoo", "polished"]);
        assert_eq!(main_first(asked.clone(), None).unwrap(), asked);
        assert_eq!(main_first(asked.clone(), Some("  ")).unwrap(), asked);
        assert_eq!(
            main_first(asked.clone(), Some("dualband-correction")).unwrap(),
            ids(&["dualband-correction", "raw", "hoo", "polished"])
        );
        assert_eq!(main_first(asked.clone(), Some("raw")).unwrap(), asked);
    }

    #[test]
    fn a_main_that_is_not_asked_for_or_is_no_render_is_refused() {
        let asked = ids(&["raw", "linear"]);
        let missing = main_first(asked.clone(), Some("hoo")).unwrap_err();
        assert!(missing.to_string().contains("not among the pipelines"));
        let linear = main_first(asked, Some("linear")).unwrap_err();
        assert!(linear.to_string().contains("never the main image"));
    }
}

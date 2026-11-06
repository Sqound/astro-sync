//! The commands of the rewritten journey: the working folder, the
//! nights and their folders, the detection, the resume, the treatments, the
//! « Récapitulatif »'s check, the sending of a night and of its logs, the
//! sky solutions, « Dépendances », the pipelines, the stacking, the
//! progression images, the AI prompt. The screens call them; each is a thin
//! call into `astro_sync_core` (mostly `night_steps`), the same calls
//! `astro-sync` makes on the command line.
//!
//! Every command that reads a night's files, runs a tool or waits for the
//! service runs off the window's thread ([`run_blocking`]); the local work
//! reports through `astro-sync://local-work` and stops on `local_work_cancel`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use astro_sync_core::analysis::fits::AcquisitionFamily;
use astro_sync_core::analysis::log_offset::ForcedLogOffsets;
use astro_sync_core::analysis::photometry::ReferenceStarList;
use astro_sync_core::analysis::sky_night::SkyOptions;
use astro_sync_core::assistant_prompt::{self, AssistantFile, PromptInput};
use astro_sync_core::detection::{self, DetectOptions, DetectionInput, NightDetection};
use astro_sync_core::device_master::DeviceUse;
use astro_sync_core::digest_cache::DigestCache;
use astro_sync_core::folder_files::{self, ManagedFile};
use astro_sync_core::gui_bridge::{self, Capabilities, HttpPortalGuiTransport};
use astro_sync_core::guide_prompt::{self, GuidePromptInput};
use astro_sync_core::local_settings::{self, LocalSettings};
use astro_sync_core::local_work::{CancelFlag, WorkKind, WorkProgress};
use astro_sync_core::metadata;
use astro_sync_core::multi_night::{self, IntegrationMaster};
use astro_sync_core::night_analysis::{self, NightAnalysis, NightAnalysisSent};
use astro_sync_core::night_online::{NightOnline, NightOnlineTransport};
use astro_sync_core::night_resume::{self, FolderState, NightLedger, NightResume};
use astro_sync_core::night_service;
use astro_sync_core::night_steps::{self, DataPaths, NightPrecheck, SentRecord};
use astro_sync_core::pipeline_trust;
use astro_sync_core::pipelines::{
    self, IgnoredPipeline, PipelineList, PipelineSection, StackingEnvironment,
};
use astro_sync_core::precheck::{
    self, ImageCandidate, ImagePlan, QuotaCheck, QuotaFinding, TreatmentState,
};
use astro_sync_core::progression_images::{self, ProgressionImages, ProgressionJob};
use astro_sync_core::python_env::{self, EnvCheck, EnvFailure, EnvTimeouts, StackingResources};
use astro_sync_core::sky_solution::{
    self, MissingCapture, SolveHints, SolveOutcome, SolveSetup, SolverChoice,
};
use astro_sync_core::stacking::{self, PipelineChoice, PythonStacker, StackGate, StackOutcome};
use astro_sync_core::tools::{self, DetectContext, DownloadFailure, Downloaded, ToolRow};
use astro_sync_core::upload_queue::EnqueueOutcome;
use astro_sync_core::uploader;
use astro_sync_core::working_folder::{
    self, NightFolderEntry, NightFolderOutcome, NightFolderSpec, NightLocation, RootEnvironment,
};

use crate::commands::{
    ending_sign_in_on_error, forced_log_offsets, local_work, local_work_of,
    require_portal_and_token, run_blocking, service_wait, LOCAL_WORK_EVENT,
};
use crate::error::CommandError;
use crate::paths;
use crate::state::AppPaths;

/// The event of a stacking in progress (`astro_sync_core::stacking::StackEvent`).
pub const STACKING_EVENT: &str = "astro-sync://stacking";
/// The event of the progression images in progress
/// (`astro_sync_core::progression_images::ProgressionEvent`).
pub const PROGRESSION_EVENT: &str = "astro-sync://progression";
/// The event of a tool download (`astro_sync_core::tools::DownloadProgress`).
pub const TOOL_DOWNLOAD_EVENT: &str = "astro-sync://tool-download";
/// The event of a sky solution done (`astro_sync_core::sky_solution::SolveOutcome`).
pub const SKY_SOLUTION_EVENT: &str = "astro-sync://sky-solution";
/// The event of the Python environment's installation
/// (`astro_sync_core::python_env::EnvProgress`).
pub const PYTHON_ENV_EVENT: &str = "astro-sync://python-env";

pub(crate) fn data_paths(paths: &AppPaths) -> DataPaths {
    DataPaths::new(&paths.data_dir)
}

pub(crate) fn settings(paths: &AppPaths) -> Result<LocalSettings, CommandError> {
    Ok(local_settings::load(&paths.local_settings)?)
}

pub(crate) fn root_of(settings: &LocalSettings) -> Result<PathBuf, CommandError> {
    settings
        .root(&RootEnvironment::current())
        .ok_or_else(|| CommandError::new("no_documents_folder", "No Documents folder found."))
}

pub(crate) fn capabilities_cached(paths: &AppPaths) -> Result<Option<Capabilities>, CommandError> {
    Ok(gui_bridge::load_capabilities_cache(&paths.capabilities_cache)?.map(|c| c.capabilities))
}

pub(crate) fn transport<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<HttpPortalGuiTransport, CommandError> {
    let (waiting, hooks) = service_wait(app);
    Ok(HttpPortalGuiTransport::with_options(
        waiting.patient,
        hooks,
    )?)
}

/// The stacking's scripts beside the app (`<resources>/stacking`, bundled
/// by `tauri.conf.json`); a debug build without them reads the
/// repository's `stacking`.
pub(crate) fn stacking_dir<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    let bundled = app
        .path()
        .resource_dir()
        .ok()
        .map(|resources| resources.join("stacking"))
        .filter(|dir| StackingResources::find(dir).is_some());
    bundled.or_else(|| {
        cfg!(debug_assertions)
            .then(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stacking"))
            .filter(|dir| StackingResources::find(dir).is_some())
    })
}

pub(crate) fn detect_context<R: Runtime>(app: &AppHandle<R>, paths: &AppPaths) -> DetectContext {
    DetectContext::system(&paths.data_dir, &paths.reference_stars)
        .with_stacking(stacking_dir(app).as_deref())
}

pub(crate) fn tool_rows<R: Runtime>(app: &AppHandle<R>, paths: &AppPaths) -> Vec<ToolRow> {
    tools::detect_tools(&detect_context(app, paths))
}

/// The stacking script and the shipped pipelines, when they are there.
pub(crate) fn stacking_resources<R: Runtime>(
    app: &AppHandle<R>,
) -> (Option<PathBuf>, Option<PathBuf>) {
    let Some(stacking) = stacking_dir(app) else {
        return (None, None);
    };
    let shipped = stacking.join("pipelines");
    (
        Some(stacking.join("astro_sync_stack.py")),
        shipped.is_dir().then_some(shipped),
    )
}

/* ------------------------------ Working folder ---------------------------- */
// The commands that read or write the person's folders (which can be on an
// external or network disk that answers slowly or not at all) run on a
// blocking thread ([`run_blocking`]), like the rest of the local work,
// never on the thread that received the request.

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkingFolderState {
    pub root: PathBuf,
    /// Chosen in Réglages › Général (`false`: the system's default).
    pub chosen: bool,
    pub exists: bool,
}

/// Réglages › Général: the working folder in force. Writes nothing.
#[tauri::command]
pub async fn working_folder_get<R: Runtime>(
    app: AppHandle<R>,
) -> Result<WorkingFolderState, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let settings = settings(&paths)?;
        let root = root_of(&settings)?;
        Ok(WorkingFolderState {
            exists: root.is_dir(),
            chosen: settings.working_folder.is_some(),
            root,
        })
    })
    .await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedRoot {
    pub root: PathBuf,
    pub created: bool,
    pub assistant_file: AssistantFile,
    /// `README.md` in the application's language, when the caller gave
    /// the language and the file could be made ready; its failure
    /// never fails the folder.
    pub readme: Option<ManagedFile>,
}

fn prepare_root(
    paths: &AppPaths,
    settings: &mut LocalSettings,
    language: Option<&str>,
) -> Result<PreparedRoot, CommandError> {
    let root = root_of(settings)?;
    // The section « Multi-night plans » while they are open.
    let (created, assistant_file) = assistant_prompt::prepare_root_for(
        &root,
        settings.assistant_file_sha256.as_deref(),
        crate::commands_multi_night::multi_night_open(paths),
    )?;
    settings.assistant_file_sha256 = assistant_file.sha256.clone();
    // `README.md`, under the same rule as `ASSISTANT-IA.md`.
    let readme = language.and_then(|language| {
        let file = folder_files::write_readme(
            &root,
            language,
            settings.managed_file_sha256(folder_files::README_FILE_NAME),
        )
        .ok()?;
        settings.record_managed_file(&file.name, file.sha256.clone());
        Some(file)
    });
    local_settings::save(&paths.local_settings, settings)?;
    Ok(PreparedRoot {
        root,
        created,
        assistant_file,
        readme,
    })
}

/// The first signed-in launch: the working folder created when absent, with
/// `ASSISTANT-IA.md` (rewritten only while it is Astro Sync's).
#[tauri::command]
pub async fn working_folder_prepare<R: Runtime>(
    app: AppHandle<R>,
    language: Option<String>,
) -> Result<PreparedRoot, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let mut settings = settings(&paths)?;
        prepare_root(&paths, &mut settings, language.as_deref())
    })
    .await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplacedAssistantFile {
    /// The person's file, renamed beside it; none when nothing was kept.
    pub kept_copy: Option<PathBuf>,
    pub assistant_file: AssistantFile,
}

/// « Remplacer par la version d'Astro Sync »: the person's `ASSISTANT-IA.md`
/// is renamed `ASSISTANT-IA.modifie-<date>.md`, then today's text is
/// written. Nothing is deleted.
#[tauri::command]
pub async fn working_folder_replace_assistant_file<R: Runtime>(
    app: AppHandle<R>,
) -> Result<ReplacedAssistantFile, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let mut settings = settings(&paths)?;
        let root = root_of(&settings)?;
        let (kept_copy, assistant_file) = assistant_prompt::replace_assistant_file_for(
            &root,
            &assistant_prompt::today_utc(),
            crate::commands_multi_night::multi_night_open(&paths),
        )
        .map_err(|source| working_folder::WorkingFolderError::Io {
            path: root.join(working_folder::ASSISTANT_FILE_NAME),
            source,
        })?;
        settings.assistant_file_sha256 = assistant_file.sha256.clone();
        local_settings::save(&paths.local_settings, &settings)?;
        Ok(ReplacedAssistantFile {
            kept_copy,
            assistant_file,
        })
    })
    .await
}

/// « Changer de dossier » (« Utiliser ce dossier »): an existing folder
/// becomes the working folder; nothing is moved from the old one.
#[tauri::command]
pub async fn working_folder_set<R: Runtime>(
    app: AppHandle<R>,
    folder: String,
    language: Option<String>,
) -> Result<PreparedRoot, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let mut settings = settings(&paths)?;
        settings.working_folder = Some(working_folder::check_new_root(Path::new(&folder))?);
        prepare_root(&paths, &mut settings, language.as_deref())
    })
    .await
}

/* ---------------- The guide for an AI assistant -------------- */

/// What `guide_write` made ready.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WrittenGuide {
    pub root: PathBuf,
    pub guide: ManagedFile,
}

/// « Poser une question à une IA »: the Guide page in Markdown (`text`,
/// rendered by the window from its dictionaries, in `language`) is written
/// at the root of the working folder as `GUIDE-ASTRO-SYNC.<language>.md`,
/// under the rule of `ASSISTANT-IA.md`: written again while it holds bytes
/// Astro Sync wrote, never over a file the person changed. A failure is an
/// error the window turns into the prompt without a path.
#[tauri::command]
pub async fn guide_write<R: Runtime>(
    app: AppHandle<R>,
    language: String,
    text: String,
) -> Result<WrittenGuide, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let mut settings = settings(&paths)?;
        let root = root_of(&settings)?;
        let name = folder_files::guide_file_name(&language);
        let guide =
            folder_files::write_guide(&root, &language, &text, settings.managed_file_sha256(&name))
                .map_err(|source| working_folder::WorkingFolderError::Io {
                    path: root.join(&name),
                    source,
                })?;
        settings.record_managed_file(&guide.name, guide.sha256.clone());
        local_settings::save(&paths.local_settings, &settings)?;
        Ok(WrittenGuide { root, guide })
    })
    .await
}

/// What `managed_file_replace` did.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplacedManagedFile {
    /// The person's file, renamed beside it; none when nothing was kept.
    pub kept_copy: Option<PathBuf>,
    pub file: ManagedFile,
}

/// « Remplacer par la version d'Astro Sync » for the guide (`kind` =
/// `guide`, with its `text`) or the README (`kind` = `readme`): the
/// person's file is renamed `<name>.modifie-<date>.md`, then Astro Sync's
/// text is written. Nothing is deleted.
#[tauri::command]
pub async fn managed_file_replace<R: Runtime>(
    app: AppHandle<R>,
    kind: String,
    language: String,
    text: Option<String>,
) -> Result<ReplacedManagedFile, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let mut settings = settings(&paths)?;
        let root = root_of(&settings)?;
        let date = assistant_prompt::today_utc();
        let (name, replaced) = match kind.as_str() {
            "guide" => (
                folder_files::guide_file_name(&language),
                folder_files::replace_guide(&root, &language, text.as_deref().unwrap_or(""), &date),
            ),
            "readme" => (
                folder_files::README_FILE_NAME.to_string(),
                folder_files::replace_readme(&root, &language, &date),
            ),
            other => {
                return Err(CommandError::new(
                    "unknown_error",
                    format!("unknown managed file: {other}"),
                ));
            }
        };
        let (kept_copy, file) =
            replaced.map_err(|source| working_folder::WorkingFolderError::Io {
                path: root.join(&name),
                source,
            })?;
        settings.record_managed_file(&file.name, file.sha256.clone());
        local_settings::save(&paths.local_settings, &settings)?;
        Ok(ReplacedManagedFile { kept_copy, file })
    })
    .await
}

/* --------------------------------- Nights -------------------------------- */

/// The night folders of the working folder, and those found again outside it.
#[tauri::command]
pub async fn night_folders_list<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<NightFolderEntry>, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let settings = settings(&paths)?;
        let root = root_of(&settings)?;
        let mut entries = if root.is_dir() {
            working_folder::list_night_folders(&root)?
        } else {
            Vec::new()
        };
        for (night_id, folder) in &settings.found_again {
            if entries.iter().all(|e| &e.folder != folder)
                && working_folder::check_found_again(folder, night_id).is_ok()
            {
                entries.push(NightFolderEntry {
                    night_id: night_id.clone(),
                    folder: folder.clone(),
                    name: folder
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                });
            }
        }
        Ok(entries)
    })
    .await
}

pub(crate) fn locate(paths: &AppPaths, night_id: &str) -> Result<NightLocation, CommandError> {
    let settings = settings(paths)?;
    let root = root_of(&settings)?;
    let location = working_folder::locate_night(&root, &settings.remembered_folders(), night_id)?;
    // Two copies, one followed (« Suivre ce dossier »): the night is there.
    Ok(working_folder::apply_followed(
        location,
        settings.followed.get(night_id).map(PathBuf::as_path),
    ))
}

/// Where the folder of a night is (found, two copies, missing).
#[tauri::command]
pub async fn night_locate<R: Runtime>(
    app: AppHandle<R>,
    night_id: String,
) -> Result<NightLocation, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        locate(&paths, &night_id)
    })
    .await
}

/// « Retrouver le dossier »: a folder chosen, which must carry the night.
#[tauri::command]
pub async fn night_found_again<R: Runtime>(
    app: AppHandle<R>,
    night_id: String,
    folder: String,
) -> Result<PathBuf, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let found = working_folder::check_found_again(Path::new(&folder), &night_id)?;
        let mut settings = settings(&paths)?;
        settings.found_again.insert(night_id, found.clone());
        local_settings::save(&paths.local_settings, &settings)?;
        Ok(found)
    })
    .await
}

/// « Suivre ce dossier »: two folders carry the night, the person chose one.
/// It must carry the night; every step then reads and fills that one.
#[tauri::command]
pub async fn night_follow<R: Runtime>(
    app: AppHandle<R>,
    night_id: String,
    folder: String,
) -> Result<PathBuf, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let chosen = working_folder::check_found_again(Path::new(&folder), &night_id)?;
        let mut settings = settings(&paths)?;
        settings.followed.insert(night_id, chosen.clone());
        local_settings::save(&paths.local_settings, &settings)?;
        Ok(chosen)
    })
    .await
}

fn night_folder(
    app_paths: &AppPaths,
    spec: &NightFolderSpec,
    apply: bool,
) -> Result<NightFolderOutcome, CommandError> {
    let settings = settings(app_paths)?;
    let root = root_of(&settings)?;
    let now = astro_sync_core::now_iso8601_utc();
    let remembered = settings.remembered_folders();
    // The copy the person chose to follow, when the window did not say.
    let mut spec = spec.clone();
    if spec.follow.is_none() {
        spec.follow = settings.followed.get(&spec.night_id).cloned();
    }
    Ok(if apply {
        working_folder::prepare_night_folder(&root, &remembered, &spec, &now)?
    } else {
        working_folder::plan_night_folder(&root, &remembered, &spec, &now)?
    })
}

/// « Dossier », shown before: the folder's name and what would be created.
#[tauri::command]
pub async fn night_folder_plan<R: Runtime>(
    app: AppHandle<R>,
    spec: NightFolderSpec,
) -> Result<NightFolderOutcome, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        night_folder(&paths, &spec, false)
    })
    .await
}

/// « Dossier »: the night's folder with the ticked targets' folders only.
#[tauri::command]
pub async fn night_folder_prepare<R: Runtime>(
    app: AppHandle<R>,
    spec: NightFolderSpec,
) -> Result<NightFolderOutcome, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let outcome = night_folder(&paths, &spec, true)?;
        // The folder this step made is a trace: if it is later moved out of
        // the working folder, the step asks to find it again instead of
        // making a second, empty one.
        let mut settings = settings(&paths)?;
        if !settings.found_again.contains_key(&spec.night_id) {
            settings
                .found_again
                .insert(spec.night_id.clone(), outcome.folder.clone());
            local_settings::save(&paths.local_settings, &settings)?;
        }
        Ok(outcome)
    })
    .await
}

/// « Fichiers »: the detection of the night's folder (headers only).
#[tauri::command]
pub async fn night_detect<R: Runtime>(
    app: AppHandle<R>,
    folder: String,
    input: DetectionInput,
) -> Result<NightDetection, CommandError> {
    let (cancel, progress) = local_work(&app);
    run_blocking(move || {
        Ok(detection::detect_night(
            Path::new(&folder),
            &input,
            &DetectOptions {
                cancel: Some(&cancel),
                progress: Some(&progress),
            },
        )?)
    })
    .await
}

/// The folder's fingerprint, compared when the window comes back to the
/// front (no file read).
#[tauri::command]
pub async fn night_fingerprint(folder: String) -> Result<String, CommandError> {
    run_blocking(move || Ok(detection::night_folder_fingerprint(Path::new(&folder))?)).await
}

/// What is online for a night (`GET /v1/sessions/{id}/online`).
#[tauri::command]
pub async fn night_online<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
) -> Result<NightOnline, CommandError> {
    run_blocking(move || {
        let (portal_url, token) = require_portal_and_token(&app)?;
        let result = transport(&app)?
            .night_online(&portal_url, &token, &session_id)
            .map_err(CommandError::from);
        ending_sign_in_on_error(&app, result)
    })
    .await
}

/// A night's page: what is online, what its folder holds, its state and the
/// step its wizard reopens at.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NightView {
    pub location: NightLocation,
    /// This computer has a trace of a folder for the night (a sending from
    /// here, a folder found again or followed): a folder that is now missing
    /// was moved, not never made.
    pub folder_trace: bool,
    pub detection: Option<NightDetection>,
    pub online: NightOnline,
    pub resume: NightResume,
    pub treatments: Vec<TreatmentState>,
    /// The masters kept in the night's folder: « 1 intégration » in
    /// « Nuits », their size at « Récapitulatif ».
    pub integrations: Vec<IntegrationMaster>,
}

struct LocalNight {
    location: NightLocation,
    detection: Option<NightDetection>,
    local: night_resume::ResumeLocal,
}

fn local_night(
    paths: &AppPaths,
    night_id: &str,
    input: &DetectionInput,
    cancel: &CancelFlag,
    progress: &(dyn Fn(&WorkProgress) + Sync),
) -> Result<LocalNight, CommandError> {
    let location = locate(paths, night_id)?;
    let (state, folder) = match &location {
        NightLocation::Found { folder } => (FolderState::Found, Some(folder.clone())),
        NightLocation::Duplicated { .. } => (FolderState::Duplicated, None),
        NightLocation::Missing => (FolderState::Missing, None),
    };
    let detection = match &folder {
        Some(folder) => Some(detection::detect_night(
            folder,
            input,
            &DetectOptions {
                cancel: Some(cancel),
                progress: Some(progress),
            },
        )?),
        None => None,
    };
    let cache = DigestCache::open(&paths.digest_cache)?;
    let local = night_steps::resume_local(
        state,
        folder.as_deref(),
        detection.as_ref(),
        Some(&cache),
        Some(cancel),
    )?;
    Ok(LocalNight {
        location,
        detection,
        local,
    })
}

/// The night's page (« Nuits », the wizard resumed).
#[tauri::command]
pub async fn night_view<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
    input: DetectionInput,
) -> Result<NightView, CommandError> {
    let (cancel, progress) = local_work(&app);
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let (portal_url, token) = require_portal_and_token(&app)?;
        let transport = transport(&app)?;
        let result = (|| {
            let online = transport.night_online(&portal_url, &token, &session_id)?;
            let storage =
                precheck::PrecheckTransport::space_storage(&transport, &portal_url, &token)?;
            let night = local_night(&paths, &session_id, &input, &cancel, &progress)?;
            let queue = paths::open_queue(&paths)?.list()?;
            let ledger = night_resume::load_ledger(&data_paths(&paths).nights, &session_id);
            let kept = settings(&paths)?;
            let folder_trace = ledger.is_some()
                || kept.found_again.contains_key(&session_id)
                || kept.followed.contains_key(&session_id);
            let resume = night_resume::resume_night(
                &online,
                &night.local,
                &queue,
                ledger.as_ref(),
                storage.variant_limit,
            );
            let rows = tool_rows(&app, &paths);
            let treatments = precheck::treatments(
                &storage.capabilities,
                &tools::tools_present(&rows),
                &night_steps::night_material(night.detection.as_ref(), Some(&online)),
            );
            let integrations = match (&night.location, &night.detection) {
                (NightLocation::Found { folder }, Some(detection)) => detection
                    .targets
                    .iter()
                    .filter(|t| t.in_night)
                    .filter_map(|t| t.folder_name.as_deref())
                    .flat_map(|name| multi_night::night_masters(folder, name))
                    .collect(),
                _ => Vec::new(),
            };
            Ok::<_, CommandError>(NightView {
                location: night.location,
                folder_trace,
                detection: night.detection,
                online,
                resume,
                treatments,
                integrations,
            })
        })();
        ending_sign_in_on_error(&app, result)
    })
    .await
}

/// « Récapitulatif »: what each image becomes, the service's answer before
/// any byte, the four limits. `extra` adds the stacked renders and the
/// progression images made for this sending. Nothing is sent.
#[tauri::command]
pub async fn night_precheck<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
    input: DetectionInput,
    extra: Vec<ImageCandidate>,
    language: Option<String>,
    restack: Option<Vec<String>>,
) -> Result<NightPrecheck, CommandError> {
    let (cancel, progress) = local_work(&app);
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let (portal_url, token) = require_portal_and_token(&app)?;
        let transport = transport(&app)?;
        let result = (|| {
            let online = transport.night_online(&portal_url, &token, &session_id)?;
            let night = local_night(&paths, &session_id, &input, &cancel, &progress)?;
            let candidates = night_steps::precheck_candidates(
                &night.local,
                extra,
                restack.as_deref().unwrap_or_default(),
            );
            let mut check =
                night_steps::precheck_night(&transport, &portal_url, &token, &online, &candidates)?;
            // What Astro Sync proposes for each image, in the language
            // of the window (the saved one, else English).
            if let (NightLocation::Found { folder }, Some(detection)) =
                (&night.location, &night.detection)
            {
                let language = language
                    .or_else(|| {
                        gui_bridge::load_settings(&paths.settings)
                            .ok()
                            .and_then(|s| s.locale)
                    })
                    .unwrap_or_else(|| "en".to_string());
                let list = pipeline_list_of(&app, &paths, &language);
                metadata::attach_proposals(
                    &mut check.plans,
                    &online,
                    &metadata::ProposalContext {
                        night_folder: folder,
                        detection,
                        language: &language,
                        pipelines: Some(&list),
                        // The quality of a stacked image goes with it under
                        // `analysis.logs` only.
                        image_quality: check.storage.capabilities.analysis_logs,
                    },
                );
            }
            Ok::<_, CommandError>(check)
        })();
        ending_sign_in_on_error(&app, result)
    })
    .await
}

/// What [`night_enqueue`] answers: the rows added, or the limits passed
/// (nothing enqueued).
#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "result",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum EnqueueAnswer {
    Enqueued {
        outcome: EnqueueOutcome,
        /// The proposals for the images already online (nothing is sent
        /// for them), with what the owner's values left.
        proposals: Vec<uploader::OnlineProposal>,
        /// The code of a failure of those proposals, if any (the images
        /// are enqueued all the same).
        proposals_error: Option<String>,
    },
    Refused {
        findings: Vec<QuotaFinding>,
    },
}

/// « Envoyer cette nuit » / « Envoyer les compléments »: the images of the
/// check enqueued -- only when it allows it; the window then drives the
/// queue (`upload_all`).
#[tauri::command]
pub async fn night_enqueue<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
    plans: Vec<ImagePlan>,
    quota: QuotaCheck,
) -> Result<EnqueueAnswer, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let queue = paths::open_queue(&paths)?;
        Ok(
            match precheck::enqueue_checked(&queue, &session_id, &plans, &quota)? {
                Ok(outcome) => {
                    let (proposals, proposals_error) = propose_for_online(&app, &plans);
                    EnqueueAnswer::Enqueued {
                        outcome,
                        proposals,
                        proposals_error,
                    }
                }
                Err(findings) => EnqueueAnswer::Refused { findings },
            },
        )
    })
    .await
}

/// The proposals for the images « déjà en ligne, identique » (no upload
/// carries them): a failure is a code, never an error -- the images are
/// enqueued all the same, and the same plans can propose again.
fn propose_for_online<R: Runtime>(
    app: &AppHandle<R>,
    plans: &[ImagePlan],
) -> (Vec<uploader::OnlineProposal>, Option<String>) {
    if !plans.iter().any(|p| {
        p.metadata.is_some() && matches!(p.decision, precheck::Decision::AlreadyOnline { .. })
    }) {
        return (Vec::new(), None);
    }
    let sent = (|| {
        let (portal_url, token) = require_portal_and_token(app).ok()?;
        let transport = uploader::HttpUploadTransport::new(portal_url).ok()?;
        Some(uploader::propose_for_online_images(
            &transport, &token, plans,
        ))
    })();
    match sent {
        Some(Ok(proposals)) => (proposals, None),
        Some(Err(error)) => (Vec::new(), Some(error.code().to_string())),
        None => (Vec::new(), Some("not_signed_in".to_string())),
    }
}

fn analyze(
    app_paths: &AppPaths,
    folder: &Path,
    options: &ForcedLogOffsets,
) -> Result<NightAnalysis, CommandError> {
    analyze_with(app_paths, folder, options, None)
}

/// [`analyze`], telling `progress` which file the report reads.
fn analyze_with(
    app_paths: &AppPaths,
    folder: &Path,
    options: &ForcedLogOffsets,
    progress: Option<astro_sync_core::local_work::ProgressSink<'_>>,
) -> Result<NightAnalysis, CommandError> {
    let stars = if app_paths.reference_stars.exists() {
        ReferenceStarList::read(&app_paths.reference_stars).ok()
    } else {
        None
    };
    let sky = SkyOptions {
        stars,
        ..SkyOptions::default()
    };
    Ok(night_analysis::analyze_night_detecting_with(
        folder,
        options,
        Some(sky),
        progress,
    )?)
}

/// « Journaux »: the report of a night's logs, read here (nothing sent).
#[tauri::command]
pub async fn night_analysis<R: Runtime>(
    app: AppHandle<R>,
    folder: String,
) -> Result<NightAnalysis, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let options = forced_log_offsets(&app)?;
        analyze(&paths, Path::new(&folder), &options)
    })
    .await
}

/// « Analyser et envoyer le rapport »: the report replaces the night's
/// analysis, the raw logs stay; the ledger keeps what was read.
#[tauri::command]
pub async fn night_send_logs<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
    folder: String,
) -> Result<NightAnalysisSent, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let options = forced_log_offsets(&app)?;
        let (portal_url, token) = require_portal_and_token(&app)?;
        // Which file the report reads, on the window's local-work event
        // (« fichier 3 sur 41 »).
        let handle = app.clone();
        let sink = move |progress: &WorkProgress| {
            let _ = handle.emit(LOCAL_WORK_EVENT, progress);
        };
        let night = analyze_with(&paths, Path::new(&folder), &options, Some(&sink))?;
        let inputs = night_resume::analysis_inputs(Path::new(&folder))?;
        let transport = transport(&app)?;
        let result = night_analysis::send_night_analysis(
            &transport,
            &portal_url,
            &token,
            &session_id,
            &night,
        )
        .map_err(CommandError::from);
        let sent = ending_sign_in_on_error(&app, result)?;
        night_steps::record_sending(
            &data_paths(&paths),
            &session_id,
            &SentRecord {
                analysis_fingerprint: Some(inputs.fingerprint),
                progression_frames: BTreeMap::new(),
            },
        )
        .map_err(|e| CommandError::other(e.to_string()))?;
        Ok(sent)
    })
    .await
}

/// Writes what a sending that went through changes in the night's ledger
/// (the progression's frames per target after its images were sent).
#[tauri::command]
pub fn night_record_sending<R: Runtime>(
    app: AppHandle<R>,
    night_id: String,
    sent: SentRecord,
) -> Result<NightLedger, CommandError> {
    let paths = paths::resolve(&app)?;
    night_steps::record_sending(&data_paths(&paths), &night_id, &sent)
        .map_err(|e| CommandError::other(e.to_string()))
}

/* ------------------------------- Sky solutions ---------------------------- */

pub(crate) fn solve_setup_parts(paths: &AppPaths) -> (sky_solution::Solvers, DataPaths) {
    let rows = tools::detect_tools(&DetectContext::system(
        &paths.data_dir,
        &paths.reference_stars,
    ));
    (tools::solvers(&rows), data_paths(paths))
}

/// The sky solutions the night's images online still lack, solved here and
/// sent (after the night's images were sent, or from its page).
#[tauri::command]
pub async fn night_solve<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
    input: DetectionInput,
    choice: Option<SolverChoice>,
    hints: Option<BTreeMap<String, SolveHints>>,
) -> Result<Vec<SolveOutcome>, CommandError> {
    let (cancel, progress) = local_work(&app);
    run_blocking(move || {
        // An image the upload just sent is being solved by the hook that
        // follows each upload: wait for it, and the images it solved are no
        // longer missing.
        let _one_at_a_time = crate::commands_sky::solving_lock();
        let paths = paths::resolve(&app)?;
        let (portal_url, token) = require_portal_and_token(&app)?;
        let transport = transport(&app)?;
        let (solvers, data) = solve_setup_parts(&paths);
        let now = astro_sync_core::now_iso8601_utc();
        let result = (|| {
            let online = transport.night_online(&portal_url, &token, &session_id)?;
            let night = local_night(&paths, &session_id, &input, &cancel, &progress)?;
            // The solvers work on the night's disk, even for an original
            // fetched from the space.
            let night_work = match &night.location {
                NightLocation::Found { folder } => {
                    Some(astro_sync_core::working_folder::solve_scratch_root(folder))
                }
                _ => None,
            };
            let setup = SolveSetup {
                solvers: &solvers,
                choice: choice.unwrap_or_default(),
                originals_dir: &data.originals,
                work_root: night_work.as_deref().unwrap_or(&data.solve),
                timeout: sky_solution::DEFAULT_SOLVE_TIMEOUT,
                cancel: Some(&cancel),
                now: &now,
            };
            Ok::<_, CommandError>(night_steps::solve_night(
                &transport,
                &portal_url,
                &token,
                &online,
                &night.local,
                &hints.unwrap_or_default(),
                &setup,
            )?)
        })();
        ending_sign_in_on_error(&app, result)
    })
    .await
}

/// « Annotations »: the captures of the space without a sky solution.
#[tauri::command]
pub async fn sky_missing<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<MissingCapture>, CommandError> {
    run_blocking(move || {
        let (portal_url, token) = require_portal_and_token(&app)?;
        let result = sky_solution::list_missing(&transport(&app)?, &portal_url, &token)
            .map_err(CommandError::from);
        ending_sign_in_on_error(&app, result)
    })
    .await
}

/// « Résoudre et envoyer » (a row of « Annotations »; « Tout résoudre »
/// calls it row by row): from the night's file of the same digest when one
/// is known (`here`: SHA-256 → path), else from the original.
#[tauri::command]
pub async fn sky_solve_capture<R: Runtime>(
    app: AppHandle<R>,
    capture: MissingCapture,
    here: Option<BTreeMap<String, PathBuf>>,
    choice: Option<SolverChoice>,
) -> Result<SolveOutcome, CommandError> {
    let (cancel, _) = local_work(&app);
    run_blocking(move || {
        let _one_at_a_time = crate::commands_sky::solving_lock();
        let paths = paths::resolve(&app)?;
        let (portal_url, token) = require_portal_and_token(&app)?;
        let (solvers, data) = solve_setup_parts(&paths);
        let now = astro_sync_core::now_iso8601_utc();
        let setup = SolveSetup {
            solvers: &solvers,
            choice: choice.unwrap_or_default(),
            originals_dir: &data.originals,
            work_root: &data.solve,
            timeout: sky_solution::DEFAULT_SOLVE_TIMEOUT,
            cancel: Some(&cancel),
            now: &now,
        };
        // The image of the working folder with this digest, when the window
        // did not point at one: solved where it is, with its master's pointing
        // and scale (a very wide field is not solved blind).
        let mut here = here.unwrap_or_default();
        let digest = capture.sha256.to_ascii_lowercase();
        let mut hints = capture.hints();
        if let Ok(root) = settings(&paths).and_then(|kept| root_of(&kept)) {
            let found = sky_solution::local_results(&root, std::slice::from_ref(&capture));
            if let Some(local) = found.get(&digest) {
                here.entry(digest).or_insert_with(|| local.path.clone());
                hints = local.hints.completed_by(hints);
            }
        }
        let source = capture.solve_from(&here);
        let result = sky_solution::solve_and_send(
            &transport(&app)?,
            &portal_url,
            &token,
            &source,
            hints,
            &setup,
        )
        .map_err(CommandError::from);
        let outcome = ending_sign_in_on_error(&app, result)?;
        let _ = app.emit(SKY_SOLUTION_EVENT, &outcome);
        Ok(outcome)
    })
    .await
}

/* -------------------------------- Dependencies ---------------------------- */

/// Réglages › « Dépendances »: one row per tool of the registry, plus ASTAP's
/// star database, the Tycho-2 list and the stacking pipeline.
#[tauri::command]
pub async fn tools_detect<R: Runtime>(app: AppHandle<R>) -> Result<Vec<ToolRow>, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        Ok(tool_rows(&app, &paths))
    })
    .await
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "result",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum DownloadAnswer {
    Done {
        id: String,
        /// `None` for the Python environment (made here, not downloaded).
        downloaded: Option<Downloaded>,
    },
    Failed {
        id: String,
        failure: DownloadFailure,
    },
    /// The Python environment could not be made.
    EnvFailed { id: String, failure: EnvFailure },
}

fn download_one<R: Runtime>(
    app: &AppHandle<R>,
    paths: &AppPaths,
    row: &ToolRow,
    cancel: &CancelFlag,
) -> Result<DownloadAnswer, CommandError> {
    let fetcher = tools::HttpToolFetcher::new().map_err(CommandError::other)?;
    let id = tools::row_of(&row.id).to_string();
    let handle = app.clone();
    let answer = tools::download_and_install(
        &paths.data_dir,
        &row.id,
        row.download.as_ref(),
        &fetcher,
        Some(cancel),
        &mut |progress| {
            let _ = handle.emit(TOOL_DOWNLOAD_EVENT, (&id, progress));
        },
    );
    Ok(match answer {
        Ok(downloaded) => DownloadAnswer::Done {
            id: row.id.clone(),
            downloaded: Some(downloaded),
        },
        Err(failure) => DownloadAnswer::Failed {
            id: row.id.clone(),
            failure,
        },
    })
}

/// Downloads and installs `rows` in order. A row of the star database
/// answers under ASTAP's id (one row of the screen), and the rest of an id
/// is not tried once one of its files failed.
fn download_rows<R: Runtime>(
    app: &AppHandle<R>,
    paths: &AppPaths,
    rows: &[&ToolRow],
    cancel: &CancelFlag,
) -> Result<Vec<DownloadAnswer>, CommandError> {
    let mut answers: Vec<DownloadAnswer> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    for row in rows {
        let shown = tools::row_of(&row.id).to_string();
        if failed.contains(&shown) {
            continue;
        }
        let mut answer = download_one(app, paths, row, cancel)?;
        match &mut answer {
            DownloadAnswer::Done { id, .. } => *id = shown,
            DownloadAnswer::Failed { id, .. } | DownloadAnswer::EnvFailed { id, .. } => {
                failed.push(shown.clone());
                *id = shown;
            }
        }
        answers.push(answer);
    }
    Ok(answers)
}

/// « Installer » (one row) or « Reprendre le téléchargement »: the
/// registry's file for this system, checked by its SHA-256 and unpacked
/// into Astro Sync's tools folder (ASTAP: the program, then its star
/// database).
#[tauri::command]
pub async fn tools_download<R: Runtime>(
    app: AppHandle<R>,
    id: String,
) -> Result<DownloadAnswer, CommandError> {
    let (cancel, _) = local_work(&app);
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let rows = tool_rows(&app, &paths);
        if !rows.iter().any(|r| r.id == id) {
            return Err(CommandError::new(
                "unknown_tool",
                format!("unknown tool {id:?}"),
            ));
        }
        let plan = tools::install_plan(&rows, &id);
        let mut answers = download_rows(&app, &paths, &plan, &cancel)?;
        if answers.is_empty() {
            // Nothing missing with a download here: the manual page only.
            return Ok(DownloadAnswer::Failed {
                id,
                failure: DownloadFailure::NoAutomaticDownload,
            });
        }
        // One answer for the row: the failure if there is one, else the last file.
        let at = answers
            .iter()
            .position(|a| !matches!(a, DownloadAnswer::Done { .. }))
            .unwrap_or(answers.len() - 1);
        Ok(answers.swap_remove(at))
    })
    .await
}

/// « Tout installer »: the required tools missing that have a download,
/// then the stacking's Python environment when a pipeline found needs it
/// and a Python in the range is there (the pipelines the person added
/// included: everything comes from the registry).
#[tauri::command]
pub async fn tools_install_all<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<DownloadAnswer>, CommandError> {
    let (cancel, _) = local_work(&app);
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let rows = tool_rows(&app, &paths);
        let mut answers = download_rows(&app, &paths, &tools::to_install(&rows), &cancel)?;
        let sections = pipeline_sections_of(&app, &paths, &rows, "en");
        if pipelines::to_install(&sections)
            .iter()
            .any(|id| id == "python-env")
        {
            answers.push(match install_python_env(&app, &paths, &cancel) {
                Ok(_) => DownloadAnswer::Done {
                    id: "python-env".to_string(),
                    downloaded: None,
                },
                Err(failure) => DownloadAnswer::EnvFailed {
                    id: "python-env".to_string(),
                    failure,
                },
            });
        }
        Ok(answers)
    })
    .await
}

pub(crate) fn install_python_env<R: Runtime>(
    app: &AppHandle<R>,
    paths: &AppPaths,
    cancel: &CancelFlag,
) -> Result<EnvCheck, EnvFailure> {
    let context = detect_context(app, paths);
    let handle = app.clone();
    python_env::install_environment(
        &context,
        context.stacking.as_ref(),
        EnvTimeouts::default(),
        cancel,
        &mut |progress| {
            let _ = handle.emit(PYTHON_ENV_EVENT, progress);
        },
    )
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "result",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum EnvAnswer {
    Done { check: EnvCheck },
    Failed { failure: EnvFailure },
}

/// « Installer » of the rows « Pipeline d'empilement » and « Environnement
/// Python » (and « Reprendre l'installation »): the environment made in
/// Astro Sync's data, its packages pinned and checked, then verified.
#[tauri::command]
pub async fn python_env_install<R: Runtime>(app: AppHandle<R>) -> Result<EnvAnswer, CommandError> {
    let (cancel, _) = local_work(&app);
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        Ok(match install_python_env(&app, &paths, &cancel) {
            Ok(check) => EnvAnswer::Done { check },
            Err(failure) => EnvAnswer::Failed { failure },
        })
    })
    .await
}

/// « Vérifier à nouveau »: the environment checked by its own script (when
/// it is there), then every row detected again.
#[tauri::command]
pub async fn tools_verify<R: Runtime>(app: AppHandle<R>) -> Result<Vec<ToolRow>, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let context = detect_context(&app, &paths);
        if let Some(resources) = context.stacking.as_ref() {
            if python_env::env_python(&paths.data_dir).is_file() {
                let _ = python_env::verify_environment(
                    &paths.data_dir,
                    resources,
                    EnvTimeouts::default().check,
                );
            }
        }
        Ok(tools::detect_tools(&context))
    })
    .await
}

pub(crate) fn pipeline_list_of<R: Runtime>(
    app: &AppHandle<R>,
    paths: &AppPaths,
    language: &str,
) -> PipelineList {
    let data = data_paths(paths);
    let (script, shipped) = stacking_resources(app);
    let environment = script.and_then(|script| StackingEnvironment::find(&paths.data_dir, &script));
    pipelines::with_trust(
        pipelines::list_pipelines(
            environment.as_ref(),
            shipped.as_deref(),
            Some(&data.pipelines),
            language,
        ),
        &pipeline_trust::load(&paths.data_dir),
    )
}

fn pipeline_sections_of<R: Runtime>(
    app: &AppHandle<R>,
    paths: &AppPaths,
    rows: &[ToolRow],
    language: &str,
) -> Vec<PipelineSection> {
    let list = pipeline_list_of(app, paths, language);
    pipelines::sections(
        &list,
        rows,
        &pipeline_trust::load(&paths.data_dir),
        language,
    )
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineSections {
    pub sections: Vec<PipelineSection>,
    /// The definitions left out, with their reason (never hidden).
    pub ignored: Vec<IgnoredPipeline>,
}

/// The section « Pipelines de traitement » of « Dépendances ».
#[tauri::command]
pub async fn pipeline_sections<R: Runtime>(
    app: AppHandle<R>,
    language: String,
) -> Result<PipelineSections, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let rows = tool_rows(&app, &paths);
        let list = pipeline_list_of(&app, &paths, &language);
        Ok(PipelineSections {
            sections: pipelines::sections(
                &list,
                &rows,
                &pipeline_trust::load(&paths.data_dir),
                &language,
            ),
            ignored: list.ignored,
        })
    })
    .await
}

/// The person's answer to « Lui faites-vous confiance ? » for the script
/// of digest `sha256` of the pipeline `id` they added, kept in Astro
/// Sync's data (never in the pipeline's folder).
#[tauri::command]
pub async fn pipeline_trust_set<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    sha256: String,
    trusted: bool,
) -> Result<(), CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        pipeline_trust::record(&paths.data_dir, &id, &sha256, trusted)
            .map_err(|e| CommandError::other(e.to_string()))
    })
    .await
}

/* --------------------------- Pipelines and stacking ----------------------- */

/// « Traitements »: the render pipelines (by the stacking script when its
/// environment is present, else from the manifests).
#[tauri::command]
pub async fn pipelines_list<R: Runtime>(
    app: AppHandle<R>,
    language: String,
) -> Result<PipelineList, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        Ok(pipeline_list_of(&app, &paths, &language))
    })
    .await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StackRun {
    pub outcome: StackOutcome,
    /// The renders as images that could leave (for `night_precheck`'s
    /// `extra`).
    pub candidates: Vec<ImageCandidate>,
}

/// « Empilement à partir des FITS » for one target: the pipelines chosen
/// (`pipelineId`, each with the digest the person trusted for a script of
/// their own), behind the pipeline row only: the stacking is local and never
/// gated (no account, a free account and a private server stack).
// The arguments are the window's own call (one per field it sends).
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn stacking_run<R: Runtime>(
    app: AppHandle<R>,
    night_folder: String,
    target_folder: String,
    target_id: String,
    evening_date: String,
    longitude: Option<f64>,
    pipelines: Vec<PipelineChoice>,
    // The poses of « Mesure des poses » the person left out: not
    // integrated, never deleted.
    discarded: Option<Vec<String>>,
    // The files of `<cible>/device/` the families take as their image instead
    // of a stacking of their poses; none: every family
    // is stacked from its poses.
    device: Option<Vec<DeviceUse>>,
    // « Cadre de l'empilement » of the target: the rank of a notch set by
    // hand; none: Automatic.
    frame: Option<u8>,
) -> Result<StackRun, CommandError> {
    let (cancel, _) = local_work_of(&app, Some(WorkKind::Stacking));
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let data = data_paths(&paths);
        let capabilities = capabilities_cached(&paths)?;
        let rows = tool_rows(&app, &paths);
        let gate = StackGate::of(
            capabilities.as_ref(),
            tools::tools_present(&rows).stacking_pipeline,
        );
        // The digest the person trusted for a script of their own, from
        // Astro Sync's data (a refused or changed script gets none: the
        // script refuses it, `script_not_trusted`).
        let store = pipeline_trust::load(&paths.data_dir);
        let listed = pipeline_list_of(&app, &paths, "en");
        let pipelines: Vec<PipelineChoice> = pipelines
            .into_iter()
            .map(|choice| PipelineChoice {
                trusted_sha256: listed
                    .get(&choice.pipeline_id)
                    .and_then(|p| store.trusted_sha256(p)),
                ..choice
            })
            .collect();
        // The lights outside the night (local noon to local noon at the
        // place's longitude) are left out of the integration.
        let window = detection::NightWindow::of_evening(&evening_date, longitude);
        let settings = local_settings::load(&paths.local_settings)?;
        let mut plan = stacking::plan_target_with_device(
            Path::new(&night_folder),
            &target_folder,
            &evening_date,
            &pipelines,
            gate.variant_limit,
            window.as_ref(),
            &device.unwrap_or_default(),
        );
        // The poses flagged by the measure and left out by the person.
        let discarded: std::collections::HashSet<PathBuf> = discarded
            .unwrap_or_default()
            .into_iter()
            .map(PathBuf::from)
            .collect();
        // Kept beside the night: its report says which poses were left out.
        astro_sync_core::pose_stats::apply_discard_and_record(
            &mut plan,
            &discarded,
            Path::new(&night_folder),
            &target_folder,
            window.as_ref(),
        );
        let plan = plan
            // « Finition par défaut » of « Traitements »: the finish of every
            // pipeline that has none of its own (`PipelineChoice::finish`, kept
            // above); the engine of the « Rendu soigné » chosen in Réglages.
            .with_contrast(settings.contrast)
            .with_retouch(astro_sync_core::stacking::Retouch {
                colour: settings.colour,
                soft_stars: settings.soft_stars,
            })
            .with_polished_engine(settings.polished_engine)
            .with_frame(frame);
        let (script, _) = stacking_resources(&app);
        let path_of = |id: &str| {
            rows.iter()
                .find(|r| r.id == id && r.state == tools::ToolState::Present)
                .and_then(|r| r.path.clone())
        };
        let stacker = PythonStacker {
            python: python_env::env_python(&paths.data_dir),
            script: script.unwrap_or_default(),
            pipelines_dirs: vec![data.pipelines.clone()],
            siril: path_of("siril"),
            starnet: path_of("starnet2"),
            graxpert: path_of("graxpert"),
            timeout: Duration::from_secs(6 * 3600),
        };
        let handle = app.clone();
        let outcome = stacking::run_target_stacking(
            &stacker,
            &plan,
            Path::new(&night_folder),
            gate,
            &stacking::free_bytes,
            &cancel,
            Some(&paths.data_dir.join(stacking::LOGS_DIR)),
            &mut |event| {
                let _ = handle.emit(STACKING_EVENT, event);
            },
        )
        .map_err(|failure| {
            let details = serde_json::to_value(&failure).ok();
            let mut error = CommandError::new("stacking_failed", format!("{failure:?}"));
            error.details = details;
            error
        })?;
        Ok(StackRun {
            candidates: stacking::stacked_candidates(&outcome, &target_id),
            outcome,
        })
    })
    .await
}

/// « Cadre de l'empilement »: what the last stacking of a target measured of
/// its frame (the field rotation, the notches); `None` before the first one.
#[tauri::command]
pub async fn stacking_frame(
    night_folder: String,
    target_folder: String,
) -> Result<Option<stacking::TargetFrame>, CommandError> {
    run_blocking(move || {
        Ok(stacking::target_frame(
            Path::new(&night_folder),
            &target_folder,
        ))
    })
    .await
}

/* ------------------------------- Progression ----------------------------- */

/// « Progression »: the families of a target's lights.
#[tauri::command]
pub async fn progression_families(
    night_folder: String,
    target_folder: String,
) -> Result<Vec<AcquisitionFamily>, CommandError> {
    run_blocking(move || {
        Ok(progression_images::target_families(
            Path::new(&night_folder),
            &target_folder,
        ))
    })
    .await
}

/// « Générer la progression »: Siril with fixed settings, the work on the
/// disk of the night, the images in `<cible>/progression/`; behind
/// `analysis.progression`.
#[tauri::command]
pub async fn progression_images_run<R: Runtime>(
    app: AppHandle<R>,
    night_id: String,
    night_folder: String,
    target_folder: String,
    family: AcquisitionFamily,
) -> Result<ProgressionImages, CommandError> {
    let (cancel, _) = local_work_of(&app, Some(WorkKind::Progression));
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let allowed = capabilities_cached(&paths)?.is_some_and(|c| c.analysis_progression);
        let rows = tool_rows(&app, &paths);
        let siril = tools::siril_for_progression(&rows);
        // The night's identifier no longer names a folder of Astro Sync's
        // data: the work is on the night's own disk.
        let _ = &night_id;
        let job = ProgressionJob::new(Path::new(&night_folder), &target_folder, family);
        let handle = app.clone();
        progression_images::run_progression_images_with(
            &job,
            allowed,
            siril
                .as_ref()
                .map(|(path, version)| (path.as_path(), version.as_str())),
            &stacking::free_bytes,
            Duration::from_secs(3 * 3600),
            Some(&cancel),
            &mut |event| {
                let _ = handle.emit(PROGRESSION_EVENT, event);
            },
        )
        .map_err(|failure| {
            let mut error = CommandError::new("progression_failed", format!("{failure:?}"));
            error.details = serde_json::to_value(&failure).ok();
            error
        })
    })
    .await
}

/// The progression images as results that could leave (for
/// `night_precheck`'s `extra`, or the « Progression » tab's own sending).
#[tauri::command]
pub fn progression_images_candidates(
    images: ProgressionImages,
    target_id: String,
) -> Vec<ImageCandidate> {
    progression_images::progression_candidates(&images, &target_id)
}

/* -------------------------------- AI prompt ------------------------------ */

/// « Demander à un assistant IA »: the prompt in the app's language (the
/// window copies it; nothing is sent to an AI service).
#[tauri::command]
pub fn assistant_prompt(language: String, input: PromptInput) -> String {
    assistant_prompt::build_prompt(&language, &input)
}

/// « Poser une question à une IA »: the prompt in the language chosen in the
/// window (the application's by default); nothing is sent to an AI service.
#[tauri::command]
pub fn guide_prompt(language: String, input: GuidePromptInput) -> String {
    guide_prompt::build_guide_prompt(&language, &input)
}

/* ------------------------- A night of the space ------------------- */

/// « Nuits », one row: the night as the service holds it (`GET
/// /v1/sessions/{id}`: its title, place, targets and debriefing), passed
/// through as the service wrote it.
#[tauri::command]
pub async fn night_detail<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
) -> Result<serde_json::Value, CommandError> {
    run_blocking(move || {
        let (portal_url, token) = require_portal_and_token(&app)?;
        let result = night_service::NightServiceTransport::session_detail(
            &transport(&app)?,
            &portal_url,
            &token,
            &session_id,
        )
        .map_err(CommandError::from);
        ending_sign_in_on_error(&app, result)
    })
    .await
}

/// « Créer cette nuit »: the night created in the space, named and placed.
/// Nothing is written on disk (the folder comes at the « Dossier » step).
#[tauri::command]
pub async fn night_create<R: Runtime>(
    app: AppHandle<R>,
    night: night_service::NightCreate,
) -> Result<serde_json::Value, CommandError> {
    run_blocking(move || {
        let (portal_url, token) = require_portal_and_token(&app)?;
        let id = night_service::fresh_client_session_id();
        let result =
            night_service::create_night(&transport(&app)?, &portal_url, &token, &night, &id)
                .map_err(|failure| match failure {
                    night_service::CreateNightFailure::Service(error) => CommandError::from(error),
                    night_service::CreateNightFailure::Invalid(error) => {
                        CommandError::new("invalid_night", error.to_string())
                    }
                });
        ending_sign_in_on_error(&app, result)
    })
    .await
}

/// « Ouvrir le dossier des pipelines »: the folder where the
/// person's own definitions go (`<données>/pipelines`), created when it does not
/// exist yet, opened by the system. Nothing else is written.
#[tauri::command]
pub fn pipelines_folder_open<R: Runtime>(app: AppHandle<R>) -> Result<(), CommandError> {
    use tauri_plugin_opener::OpenerExt;
    let paths = paths::resolve(&app)?;
    let folder = data_paths(&paths).pipelines;
    std::fs::create_dir_all(&folder).map_err(|e| CommandError::new("io_error", e.to_string()))?;
    app.opener()
        .open_path(folder.display().to_string(), None::<&str>)
        .map_err(|e| CommandError::other(e.to_string()))
}

/// « Ouvrir dans le Finder » / « l'Explorateur », « Ouvrir le dossier de
/// M31 »: an existing folder opened by the system; « Ouvrir ASSISTANT-IA.md »
/// (the Guide): that one file, opened by its usual application. Nothing is
/// written.
#[tauri::command]
pub fn folder_open<R: Runtime>(app: AppHandle<R>, folder: String) -> Result<(), CommandError> {
    use tauri_plugin_opener::OpenerExt;
    let path = PathBuf::from(&folder);
    let assistant_file = path.is_file()
        && path
            .file_name()
            .is_some_and(|name| name == "ASSISTANT-IA.md");
    if !path.is_dir() && !assistant_file {
        return Err(CommandError::new(
            "folder_missing",
            format!("{folder} is not a folder"),
        ));
    }
    app.opener()
        .open_path(folder, None::<&str>)
        .map_err(|e| CommandError::other(e.to_string()))
}

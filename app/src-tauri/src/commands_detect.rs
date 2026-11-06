//! « Détecter les sessions »: the detection of a folder of FITS (or of one
//! declared session, `DetectRequest::scope`), and the tidying of its files.
//! Each is a thin call into `astro_sync_core::session_detection`, the calls
//! `astro-sync detect-sessions` and `astro-sync organise` make on the command
//! line. The person's data is read from the service (read only) when the window
//! does not give it; nothing is written there.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Emitter, Runtime};

use astro_sync_core::catalogue_targets::{self, CatalogueTarget};
use astro_sync_core::local_work::WorkKind;
use astro_sync_core::session_detection::{
    self, service, AccountRights, DetectOptions, DetectRequest, FoundSoFar, NightNaming,
    OrganiseMode, OrganiseOptions, OrganiseOutcome, OrganisePlan, Proposal, UserData,
};
use astro_sync_core::sky_solution::{self, SolveHints, SolveJob, SolverChoice};
use astro_sync_core::tools::{self, DetectContext};

use crate::commands::{local_work_of, require_portal_and_token, run_blocking};
use crate::commands_multi_night::attach_gate_of;
use crate::commands_night::{capabilities_cached, data_paths, transport};
use crate::error::CommandError;
use crate::paths;

/// « Trouvé jusqu'ici » while the headers are read (`FoundSoFar`).
pub const DETECT_FOUND_EVENT: &str = "detect-found";

/// The rights of the account, from the capabilities kept and whether the
/// service sells the multi-night plans (the gates of the other screens).
fn account<R: Runtime>(app: &AppHandle<R>) -> Result<AccountRights, CommandError> {
    let paths = paths::resolve(app)?;
    // The profile the window kept may be older than the offer (a code
    // redeemed since): the network first, the kept copy offline.
    if let (Ok((portal, token)), Ok(transport)) = (require_portal_and_token(app), transport(app)) {
        let _ = astro_sync_core::gui_bridge::refresh_capabilities(
            &transport,
            &paths.capabilities_cache,
            &portal,
            &token,
        );
    }
    let gate = attach_gate_of(&paths)?;
    let capabilities = capabilities_cached(&paths)?;
    Ok(AccountRights {
        advanced_planning: gate.allowed,
        several_plans_per_night: gate.closed.is_none()
            && capabilities
                .as_ref()
                .is_none_or(|c| c.planner_multiple || gate.allowed),
    })
}

/// The catalogue's index kept here, refreshed from the service when it can
/// be (a failure keeps the copy).
fn kept_catalogue<R: Runtime>(app: &AppHandle<R>, locale: &str) -> Vec<CatalogueTarget> {
    let Ok(paths) = paths::resolve(app) else {
        return Vec::new();
    };
    let index_path = paths.data_dir.join(catalogue_targets::INDEX_FILE);
    let kept = catalogue_targets::load_index(&index_path);
    let refreshed = require_portal_and_token(app)
        .ok()
        .and_then(|(portal, token)| {
            catalogue_targets::refresh_index(
                &transport(app).ok()?,
                &portal,
                Some(&token),
                locale,
                kept.clone(),
            )
            .ok()
        });
    match refreshed {
        Some((index, fresh)) => {
            if fresh {
                let _ = catalogue_targets::save_index(&index_path, &index);
            }
            index.targets
        }
        None => kept.map(|i| i.targets).unwrap_or_default(),
    }
}

/// « Lancer la détection »: the three locations read (headers only), the
/// frames grouped, the person's data matched; nothing is written. `user`:
/// what the window already holds (its places carry their offset, its rights
/// are the account's), else it is read from the service when signed in;
/// `catalogue`: the targets the window holds, else the index kept here,
/// refreshed from the service.
#[tauri::command]
pub async fn sessions_detect<R: Runtime>(
    app: AppHandle<R>,
    request: DetectRequest,
    user: Option<UserData>,
    catalogue: Option<Vec<CatalogueTarget>>,
    locale: Option<String>,
) -> Result<Proposal, CommandError> {
    let (cancel, progress) = local_work_of(&app, Some(WorkKind::Headers));
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let catalogue = match catalogue {
            Some(targets) => targets,
            None => kept_catalogue(&app, locale.as_deref().unwrap_or("fr")),
        };
        // The solvers are looked for only when a frame needs one.
        let solvers = std::sync::OnceLock::new();
        let work = data_paths(&paths).solve.join("detection");
        let now = astro_sync_core::now_iso8601_utc();
        let solve = |image: &Path| -> Result<(f64, f64), sky_solution::SolveFailure> {
            let solvers = solvers.get_or_init(|| {
                tools::solvers(&tools::detect_tools(&DetectContext::system(
                    &paths.data_dir,
                    &paths.reference_stars,
                )))
            });
            let job = SolveJob {
                image,
                hints: SolveHints::default(),
                work_dir: &work,
                timeout: sky_solution::DEFAULT_SOLVE_TIMEOUT,
                cancel: Some(&cancel),
                solved_at: &now,
            };
            let solution = sky_solution::solve_image(solvers, SolverChoice::Astap, &job)?;
            Ok((solution.ra_deg, solution.dec_deg))
        };
        let emitter = app.clone();
        let found = move |found: &FoundSoFar| {
            let _ = emitter.emit(DETECT_FOUND_EVENT, found);
        };
        let options = DetectOptions {
            cancel: Some(&cancel),
            progress: Some(&progress),
            solver: Some(&solve),
            found: Some(&found),
        };
        let reading = session_detection::read_locations(&request, &options)?;
        // The window gives its places (with the offset it computed from each
        // time zone) and its equipment; the sessions and plans of the dates
        // found are read from the service (read only) once they are known.
        let supplied = user.is_some();
        let mut user = user.unwrap_or_default();
        let mut grouping =
            session_detection::group_frames(&reading, &request, &catalogue, &user, &options)?;
        if let Ok((portal, token)) = require_portal_and_token(&app) {
            let read = service::read_user_data(
                &transport(&app)?,
                &portal,
                &token,
                account(&app)?,
                grouping.date_range(),
            )?;
            if supplied {
                user.sessions = read.sessions;
                user.plans = read.plans;
                user.account = read.account;
            } else {
                user = read;
            }
            grouping =
                session_detection::group_frames(&reading, &request, &catalogue, &user, &options)?;
        }
        Ok(session_detection::propose(&grouping, &user))
    })
    .await
}

/// A proposal followed to the rights the account holds now.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RightsFollowed {
    pub proposal: Proposal,
    /// The rights were not the ones the proposal was computed for.
    pub changed: bool,
    /// The ids of the multi-night lines that opened (ticked by default).
    pub unlocked: Vec<String>,
    /// The rights changed in a way the proposal cannot follow: detect again.
    pub stale: bool,
}

/// The proposal the window holds, followed to the rights of the account as
/// they are now (an offer granted or ended since the detection): the
/// multi-night plans open or close without reading the folders again.
/// Nothing is written.
#[tauri::command]
pub async fn sessions_refresh_rights<R: Runtime>(
    app: AppHandle<R>,
    proposal: Proposal,
) -> Result<RightsFollowed, CommandError> {
    run_blocking(move || {
        let account = account(&app)?;
        let mut proposal = proposal;
        let (changed, unlocked, stale) = match proposal.refresh_rights(account) {
            session_detection::RightsRefresh::Unchanged => (false, Vec::new(), false),
            session_detection::RightsRefresh::Updated { unlocked } => (true, unlocked, false),
            session_detection::RightsRefresh::Stale => (true, Vec::new(), true),
        };
        Ok(RightsFollowed {
            proposal,
            changed,
            unlocked,
            stale,
        })
    })
    .await
}

/// The person's places, equipment and plans, read from the service (read
/// only), for the window to give each place its offset from UTC (computed
/// from its time zone) before « Lancer la détection ». The sessions are
/// read by the detection, for the dates it finds.
#[tauri::command]
pub async fn sessions_user_data<R: Runtime>(app: AppHandle<R>) -> Result<UserData, CommandError> {
    run_blocking(move || {
        let (portal, token) = require_portal_and_token(&app)?;
        Ok(service::read_user_data(
            &transport(&app)?,
            &portal,
            &token,
            account(&app)?,
            None,
        )?)
    })
    .await
}

/// « Ranger les fichiers… », the summary before acting (the dry run);
/// `names`: the place and the session created of each night (its folder
/// `AAAA-MM-JJ · <lieu> · <id court>`).
#[tauri::command]
pub async fn sessions_organise_plan(
    proposal: Proposal,
    destination: String,
    mode: OrganiseMode,
    sessions: Option<Vec<String>>,
    names: Option<std::collections::BTreeMap<String, NightNaming>>,
) -> Result<OrganisePlan, CommandError> {
    run_blocking(move || {
        Ok(session_detection::plan_organise_named(
            &proposal,
            &PathBuf::from(destination),
            mode,
            &sessions.unwrap_or_default(),
            &names.unwrap_or_default(),
        )?)
    })
    .await
}

/// The tidying, file by file, after the summary (« Arrêter le rangement »
/// stops it between two files).
#[tauri::command]
pub async fn sessions_organise<R: Runtime>(
    app: AppHandle<R>,
    plan: OrganisePlan,
) -> Result<OrganiseOutcome, CommandError> {
    let (cancel, progress) = local_work_of(&app, Some(WorkKind::Organise));
    run_blocking(move || {
        Ok(session_detection::organise(
            &plan,
            &OrganiseOptions {
                cancel: Some(&cancel),
                progress: Some(&progress),
            },
        )?)
    })
    .await
}

/* ------------------------------ The creation ------------------------------ */

use astro_sync_core::session_detection::{BulkCreateOutcome, BulkCreateRequest};

/// « Créer … »: the proposal as ticked, written by the service's
/// `POST /v1/sessions/bulk` in one idempotent call (a retry replays); the
/// elements it refuses come back in `skipped` with their code. The same call
/// as `astro-sync detect-sessions --create` on the command line.
#[tauri::command]
pub async fn sessions_create_bulk<R: Runtime>(
    app: AppHandle<R>,
    request: BulkCreateRequest,
) -> Result<BulkCreateOutcome, CommandError> {
    run_blocking(move || {
        let (portal, token) = require_portal_and_token(&app)?;
        Ok(session_detection::create_sessions(
            &transport(&app)?,
            &portal,
            &token,
            &request,
        )?)
    })
    .await
}

/// « Place restante : 2 images sur 3 »: the space's storage and its captures
/// (`GET /v1/space/storage`, read only), for the result page and « Images à
/// envoyer ».
#[tauri::command]
pub async fn space_storage_get<R: Runtime>(
    app: AppHandle<R>,
) -> Result<astro_sync_core::precheck::SpaceStorage, CommandError> {
    run_blocking(move || {
        let (portal, token) = require_portal_and_token(&app)?;
        Ok(astro_sync_core::precheck::PrecheckTransport::space_storage(
            &transport(&app)?,
            &portal,
            &token,
        )?)
    })
    .await
}

/* ------------------- Nothing in the space before step 4 ------------------ */

use astro_sync_core::session_detection::provisional::{
    self, Discarded, ImportProgress, ImportStage, Settled,
};

/// What « Créer … » would make of the review, computed here: a provisional
/// identifier for each session and plan to create. Nothing is asked of the
/// service, nothing is written: the treatments of step 3 run on it.
#[tauri::command]
pub fn sessions_provisional(request: BulkCreateRequest) -> BulkCreateOutcome {
    provisional::provisional_outcome(&request)
}

/// What the first stage of step 4's primary did: the sessions and plans
/// created, and the night folders renamed after them.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCreated {
    pub outcome: BulkCreateOutcome,
    pub nights: Vec<Settled>,
}

/// The working folder and the queue, for the provisional folders.
fn root_and_queue<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<(PathBuf, astro_sync_core::upload_queue::UploadQueue), CommandError> {
    let paths = paths::resolve(app)?;
    let root = crate::commands_night::root_of(&crate::commands_night::settings(&paths)?)?;
    let queue = paths::open_queue(&paths)?;
    Ok((root, queue))
}

fn stage_error(stage: ImportStage, error: provisional::StageError) -> CommandError {
    let mut failure = match error {
        provisional::StageError::Service(error) => CommandError::from(error),
        provisional::StageError::Local(error) => CommandError::new(error.code(), error.to_string()),
    };
    failure.details = serde_json::to_value(serde_json::json!({ "stage": stage })).ok();
    failure
}

/// Step 4's primary, its first stage: the sessions and the multi-night plans
/// created by the service's one call (`POST /v1/sessions/bulk`, a second try
/// replays), each provisional record told its session, then the night folders
/// renamed after their session. A failure says its stage (`details.stage`);
/// what went through stays.
#[tauri::command]
pub async fn detect_import_create<R: Runtime>(
    app: AppHandle<R>,
    request: BulkCreateRequest,
    provisional: BulkCreateOutcome,
) -> Result<ImportCreated, CommandError> {
    run_blocking(move || {
        let (portal, token) = require_portal_and_token(&app)?;
        let (root, queue) = root_and_queue(&app)?;
        let mut progress = ImportProgress::default();
        provisional::run_import_stages(
            &transport(&app)?,
            &portal,
            &token,
            &request,
            &provisional,
            &root,
            Some(&queue),
            &[ImportStage::Sessions, ImportStage::Nights],
            &mut progress,
            &mut |_| {},
        )
        .map_err(|(stage, error)| stage_error(stage, error))?;
        Ok(ImportCreated {
            outcome: progress.created.unwrap_or_default(),
            nights: progress.nights.unwrap_or_default(),
        })
    })
    .await
}

/// Step 4's primary, its second stage: the plan folders renamed after the
/// multi-night plans created (`nights`: the night folders the first stage
/// renamed).
#[tauri::command]
pub async fn detect_import_plans<R: Runtime>(
    app: AppHandle<R>,
    nights: Vec<Settled>,
) -> Result<Vec<Settled>, CommandError> {
    run_blocking(move || {
        let (root, queue) = root_and_queue(&app)?;
        provisional::settle_plans(&root, &nights, Some(&queue))
            .map_err(|error| CommandError::new(error.code(), error.to_string()))
    })
    .await
}

/// « Abandonner l'import », once confirmed: the provisional folders of `ids`
/// removed -- links and Astro Sync's own files only.
#[tauri::command]
pub async fn detect_import_discard<R: Runtime>(
    app: AppHandle<R>,
    ids: Vec<String>,
) -> Result<Discarded, CommandError> {
    run_blocking(move || {
        let (root, _) = root_and_queue(&app)?;
        provisional::discard_provisional(&root, &ids)
            .map_err(|error| CommandError::new(error.code(), error.to_string()))
    })
    .await
}

/// At each start: the renames of provisional folders a stop left half done
/// are finished from their records. Quiet: a working folder that cannot be
/// read is left for the next start.
pub fn reconcile_provisional<R: Runtime>(app: &AppHandle<R>) {
    let Ok((root, queue)) = root_and_queue(app) else {
        return;
    };
    let _ = provisional::reconcile(&root, Some(&queue));
}

/* ----------------------- The treatments of the sessions ------------------- */

/// « Lancer les traitements » (state 6 b): the sessions that hold one of
/// `target_ids` become night folders of the working folder, their frames,
/// calibration and logs linked there (never moved), so that `stacking_run`
/// and `plan_image_run_local` treat them as any night. `created` is the
/// provisional outcome (`sessions_provisional`): a session to create gets a
/// provisional folder; or what the service created, for a session opened from
/// « Terminé ».
#[tauri::command]
pub async fn detect_treatments_prepare<R: Runtime>(
    app: AppHandle<R>,
    proposal: Proposal,
    created: BulkCreateOutcome,
    target_ids: Vec<String>,
) -> Result<Vec<astro_sync_core::session_detection::TreatmentNight>, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let kept = crate::commands_night::settings(&paths)?;
        let root = crate::commands_night::root_of(&kept)?;
        let (portal, _) = require_portal_and_token(&app)?;
        session_detection::prepare_treatments(
            &proposal,
            &created,
            &target_ids,
            &root,
            &kept.remembered_folders(),
            &portal,
            &astro_sync_core::now_iso8601_utc(),
        )
        .map_err(|error| CommandError::new(error.code(), error.to_string()))
    })
    .await
}

/// One image ticked in « Images à envoyer » (state 6 c).
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ImageToSend {
    /// The render of one night (`stacking_run`), a capture of its session.
    #[serde(rename_all = "camelCase")]
    Night {
        session_id: String,
        target_id: String,
        render: astro_sync_core::stacking::StackedRender,
        /// The digest of the image of the same sending that creates the
        /// capture this render is a variant of (`None`: this render is
        /// the capture). The image it names goes first.
        #[serde(default)]
        variant_of_sha256: Option<String>,
    },
    /// The image of a multi-night plan (`plan_image_run` with `queue: false`).
    #[serde(rename_all = "camelCase")]
    Plan {
        /// Boxed: much larger than the night variant (clippy `large_enum_variant`).
        outcome: Box<astro_sync_core::plan_image::PlanImageOutcome>,
        /// The digest of the image of the same sending that creates the capture
        /// of the plan this render is a variant of (`None`: this render is the
        /// capture). The image it names goes first.
        #[serde(default)]
        variant_of_sha256: Option<String>,
        /// The image of the plan after each night, made with this image:
        /// they leave as the history of the capture only when this image
        /// is that capture.
        #[serde(default)]
        progression: Vec<astro_sync_core::plan_progression::PlanProgressionStep>,
    },
}

/// « Envoyer N images »: the images ticked, queued for the sending (the
/// queue sends them as it does every image; nothing else leaves).
#[tauri::command]
pub async fn detect_images_send<R: Runtime>(
    app: AppHandle<R>,
    images: Vec<ImageToSend>,
) -> Result<astro_sync_core::upload_queue::EnqueueOutcome, CommandError> {
    use astro_sync_core::upload_queue::{EntryTarget, ResultFile, ResultKind};
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let mut files = Vec::new();
        for image in images {
            match image {
                ImageToSend::Night {
                    session_id,
                    target_id,
                    render,
                    variant_of_sha256,
                } => files.push(ResultFile {
                    path: render.path,
                    sha256: render.sha256,
                    size: render.size,
                    kind: ResultKind::Stacked,
                    target: EntryTarget {
                        session_id: Some(session_id),
                        target_ids: vec![target_id],
                        variant_of: variant_of_sha256.map(|digest| {
                            format!("{}{digest}", astro_sync_core::uploader::VARIANT_OF_SHA256)
                        }),
                        source: Some("stacking".to_string()),
                        ..EntryTarget::default()
                    },
                }),
                ImageToSend::Plan {
                    outcome,
                    variant_of_sha256,
                    progression,
                } => {
                    // A render that is not the capture is an image of the
                    // capture the first one creates: its own title, and
                    // nothing of how the capture was made.
                    match variant_of_sha256 {
                        Some(digest) => files.extend(
                            astro_sync_core::plan_image::plan_variant_results(&outcome, &digest),
                        ),
                        None => {
                            files.extend(astro_sync_core::plan_image::plan_results(&outcome));
                            files.extend(
                                astro_sync_core::plan_progression::plan_progression_results(
                                    &progression,
                                    &outcome.target_id,
                                    &format!(
                                        "{}{}",
                                        astro_sync_core::uploader::VARIANT_OF_SHA256,
                                        outcome.render.sha256
                                    ),
                                ),
                            );
                        }
                    }
                }
            }
        }
        Ok(paths::open_queue(&paths)?.enqueue_results(&files)?)
    })
    .await
}

/* --------------------- The images of « Images à envoyer » ------------------ */

/// A result Astro Sync made, as the window may open it: a TIFF (or PNG) in a
/// `stacked/` folder. Nothing else is opened from here.
fn result_file(path: &str) -> Result<PathBuf, CommandError> {
    let path = PathBuf::from(path);
    let kind = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let in_stacked = path
        .parent()
        .and_then(|p| p.file_name())
        .is_some_and(|name| name == astro_sync_core::working_folder::STACKED_DIR);
    if !path.is_file() || !in_stacked || !matches!(kind.as_deref(), Some("tif" | "tiff" | "png")) {
        return Err(CommandError::new(
            "file_missing",
            format!("{} is not an image Astro Sync made", path.display()),
        ));
    }
    Ok(path)
}

/// « Ouvrir » on the path of an image of « Images à envoyer »: the file,
/// opened by the system's usual application. Nothing is written.
#[tauri::command]
pub fn result_file_open<R: Runtime>(app: AppHandle<R>, path: String) -> Result<(), CommandError> {
    use tauri_plugin_opener::OpenerExt;
    let file = result_file(&path)?;
    app.opener()
        .open_path(file.display().to_string(), None::<&str>)
        .map_err(|e| CommandError::other(e.to_string()))
}

/// « Afficher dans le Finder » / « l'Explorateur »: the file selected in its
/// folder.
#[tauri::command]
pub fn result_file_reveal<R: Runtime>(app: AppHandle<R>, path: String) -> Result<(), CommandError> {
    use tauri_plugin_opener::OpenerExt;
    let file = result_file(&path)?;
    app.opener()
        .reveal_item_in_dir(file)
        .map_err(|e| CommandError::other(e.to_string()))
}

/// The largest picture read for a preview: a PNG beyond it is not shown.
const PREVIEW_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// The PNG written beside a TIFF render to look at: the one with the curve of
/// presentation when there is one, else the render's own. The TIFF is what
/// leaves; the PNG is never sent.
fn preview_png(tiff: &Path) -> Option<PathBuf> {
    let stem = tiff.file_stem()?.to_str()?;
    let folder = tiff.parent()?;
    [format!("{stem}_presentation.png"), format!("{stem}.png")]
        .into_iter()
        .map(|name| folder.join(name))
        .find(|candidate| {
            candidate
                .metadata()
                .is_ok_and(|m| m.is_file() && m.len() <= PREVIEW_MAX_BYTES)
        })
}

/// The picture to look at for the TIFF render `path`: the bytes of its PNG,
/// which the window shrinks, or nothing when there is none.
#[tauri::command]
pub async fn result_file_preview(path: String) -> Result<tauri::ipc::Response, CommandError> {
    run_blocking(move || {
        let tiff = result_file(&path)?;
        let bytes = preview_png(&tiff)
            .and_then(|png| std::fs::read(png).ok())
            .unwrap_or_default();
        Ok(tauri::ipc::Response::new(bytes))
    })
    .await
}

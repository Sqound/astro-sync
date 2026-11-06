//! Every `#[tauri::command]` the React UI calls. Each one resolves the
//! file paths this GUI installation uses (`crate::paths::resolve`) and
//! calls straight into `astro_sync_core` -- scanner, dry_run, package,
//! upload_queue, uploader, and the GUI-specific `gui_bridge` --
//! never reimplementing any of it here. A command's only "logic" is
//! choosing *which* core call answers a given screen action and shaping
//! its arguments; the JSON a command returns is, field for field, a core
//! struct's own `Serialize` output (see each module's doc comment in
//! `astro-sync-core` for what the shape means).
//!
//! Naming: the command set is named conceptually as "scan, dry_run, package, enqueue,
//! upload_next/upload_all, queue_status, settings_get/set"; every name
//! matches its verb directly. The association of a folder with a
//! night by a pasted identifier left the window: a
//! night's folder is created and found by the working folder
//! (`commands_night`).

use std::path::Path;
use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use tauri_plugin_dialog::DialogExt;

use astro_sync_core::analysis::log_offset::ForcedLogOffsets;
use astro_sync_core::analysis::reference_catalogue::{self, Tycho2Consent};
use astro_sync_core::build_info;
use astro_sync_core::digest_cache::DigestCache;
use astro_sync_core::dry_run::{self, DryRunReport};
use astro_sync_core::gui_bridge::{
    self, Capabilities, GuiSettings, HttpPortalGuiTransport, QueueSummary, SessionListPage,
};
use astro_sync_core::local_settings::{self, LocalSettings};
use astro_sync_core::local_work::{CancelFlag, WorkKind, WorkProgress};
use astro_sync_core::package::{self, PackageOutcome, ProducerInfo, SessionMeta};
use astro_sync_core::scanner::{self, ScanOptions, ScanReport};
use astro_sync_core::service_wait::{ServiceEvent, WaitHooks};
use astro_sync_core::space::{
    self, CodeRequested, DetachOutcome, DeviceIdentity, RenameOutcome, SignInAnnouncement,
    SignInEnd, SpaceClient, SpaceRecord, SpaceStatus, SystemTokenStore,
};
use astro_sync_core::standing::Standing;
use astro_sync_core::upload_queue::{EnqueueOutcome, QueueEntry};
use astro_sync_core::uploader::{upload_entry, HttpUploadTransport, UploadTarget};

use crate::dialogs;
use crate::error::CommandError;
use crate::paths;
use crate::state::{AppPaths, LocalWorkControl, ServiceWait, UploadControl};

/// The Tauri event the interface listens to for the state of the hosted
/// service, carrying `astro_sync_core::service_wait::ServiceEvent`'s own
/// `{"kind":"starting"|"ready"|"stopped"}` JSON. A service can sleep when
/// nobody uses it and answer the first request late:
/// while a call waits, the interface says "Le service démarre" and offers
/// to cancel ([`service_cancel`]).
pub const SERVICE_EVENT: &str = "astro-sync://service-event";

/// The waiting policy and the hooks (state reporting through
/// [`SERVICE_EVENT`], and the cancel switch) for one command's network calls.
pub(crate) fn service_wait<R: Runtime>(app: &AppHandle<R>) -> (ServiceWait, WaitHooks) {
    let waiting = app.state::<ServiceWait>().inner().clone();
    let handle = app.clone();
    let hooks = WaitHooks::new(waiting.control.token(), move |event: ServiceEvent| {
        let _ = handle.emit(SERVICE_EVENT, event);
    });
    (waiting, hooks)
}

/// Runs work that can wait a minute on the hosted service (every network
/// command) on a thread of its own, and answers when it is done.
///
/// A plain `fn` command runs inline on the thread that received the request
/// -- the main thread, which also runs the window and delivers every event
/// to the page. Holding it for a minute freezes the window, keeps the
/// "Le service démarre" event from ever reaching the page until the wait is
/// over, and keeps `service_cancel` itself from being processed.
///
/// A dedicated blocking thread rather than `#[tauri::command(async)]` on a
/// plain `fn`, which would run it on an async worker: measured, `reqwest`'s
/// blocking client panics there in a debug build ("Cannot drop a runtime in
/// a context where blocking is not allowed").
pub(crate) async fn run_blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, CommandError> + Send + 'static,
) -> Result<T, CommandError> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| CommandError::other(error.to_string()))?
}

/// "Annuler" in the banner shown while the service starts: ends every wait
/// in progress. The calls that were waiting fail with the `cancelled` code;
/// nothing that could have taken effect is ever abandoned midway (see
/// `astro_sync_core::service_wait`).
#[tauri::command]
pub fn service_cancel(waiting: State<ServiceWait>) {
    waiting.control.cancel_all();
}

/// Loads the portal URL and device bearer token settings must already
/// hold, or a stable `portal_url_missing`/`not_paired` [`CommandError`] the
/// Settings screen can act on directly (e.g. by opening the pairing
/// dialog).
pub(crate) fn require_portal_and_token<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<(String, String), CommandError> {
    let paths = paths::resolve(app)?;
    let settings = gui_bridge::load_settings(&paths.settings)?;
    let portal_url = settings.portal_url.ok_or_else(|| {
        CommandError::new("portal_url_missing", "No service address is saved yet.")
    })?;
    let token =
        gui_bridge::load_device_token(&paths.device_token, &portal_url)?.ok_or_else(|| {
            CommandError::new(
                "not_paired",
                "This computer is not connected to a space yet.",
            )
        })?;
    Ok((portal_url, token))
}

/* ------------------------------ Build info ----------------------------- */

/// The version of the application (`tauri.conf.json`), the commit and the
/// date of the build, for the menu and « Réglages ». Stamped at compile
/// time by `astro-sync-core`'s `build.rs`: nothing is read from git or the
/// clock here.
#[tauri::command]
pub fn app_build_info<R: Runtime>(app: AppHandle<R>) -> build_info::BuildInfo {
    build_info::build_info(&app.package_info().version.to_string())
}

/* ------------------------------- Settings ------------------------------ */

#[tauri::command]
pub fn settings_get(app: AppHandle) -> Result<GuiSettings, CommandError> {
    let paths = paths::resolve(&app)?;
    Ok(gui_bridge::load_settings(&paths.settings)?)
}

#[tauri::command]
pub fn settings_set(app: AppHandle, settings: GuiSettings) -> Result<GuiSettings, CommandError> {
    let paths = paths::resolve(&app)?;
    gui_bridge::save_settings(&paths.settings, &settings)?;
    Ok(settings)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStatus {
    pub paired: bool,
    pub device_label: Option<String>,
    pub portal_url: Option<String>,
}

#[tauri::command]
pub fn device_status(app: AppHandle) -> Result<DeviceStatus, CommandError> {
    let paths = paths::resolve(&app)?;
    let settings = gui_bridge::load_settings(&paths.settings)?;
    // A device can only be paired against a portal URL that was saved
    // alongside its token (see `device_pair` below), so with no portal URL
    // on record there is nothing to look up in the keyring.
    let token = match &settings.portal_url {
        Some(portal_url) => gui_bridge::load_device_token(&paths.device_token, portal_url)?,
        None => None,
    };
    Ok(DeviceStatus {
        paired: token.is_some(),
        device_label: settings.device_label,
        portal_url: settings.portal_url,
    })
}

/// Exchanges a pairing code (shown on the portal's own account page) for a
/// device bearer token, then
/// persists both the token and the portal URL/device label that produced
/// it.
#[tauri::command]
pub async fn device_pair<R: Runtime>(
    app: AppHandle<R>,
    portal_url: String,
    code: String,
    device_label: String,
) -> Result<GuiSettings, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let (waiting, hooks) = service_wait(&app);
        let transport = HttpPortalGuiTransport::with_options(waiting.patient, hooks)?;
        let token = gui_bridge::claim_device(&transport, &portal_url, &code, &device_label)?;
        // The keyring account: see `gui_bridge`'s "Device token storage" note
        // for why it's the portal URL rather than, say, the device label.
        gui_bridge::save_device_token(&paths.device_token, &portal_url, &token)?;
        let previous_locale = gui_bridge::load_settings(&paths.settings)?.locale;
        let settings = GuiSettings {
            portal_url: Some(portal_url),
            device_label: Some(device_label),
            locale: previous_locale,
        };
        gui_bridge::save_settings(&paths.settings, &settings)?;
        // A pairing has no space of its own: what a sign-in by e-mail code
        // once recorded no longer describes this device.
        space::save_record(&paths.space_record, &SpaceRecord::default())?;
        Ok(settings)
    })
    .await
}

#[tauri::command]
pub fn device_unpair(app: AppHandle) -> Result<(), CommandError> {
    let paths = paths::resolve(&app)?;
    let settings = gui_bridge::load_settings(&paths.settings)?;
    gui_bridge::clear_device_token(&paths.device_token, settings.portal_url.as_deref())?;
    space::save_record(&paths.space_record, &SpaceRecord::default())?;
    Ok(())
}

/* ------------------------------ Capabilities ----------------------------- */

/// The capability-gating doctrine, expressed in two fields so the
/// frontend never has to guess *why* `capabilities` is `None`:
/// - not paired at all -- `paired: false`, `capabilities: None`. Nothing is
///   shown: no Progression/Analysis screen, and not even the neutral "this
///   is part of an offer" notice (there is no account to name an offer
///   for yet).
/// - paired, but the account's `analysis.logs` capability is absent (a
///   standard profile) OR could not be determined at all (a fresh pairing
///   whose very first refresh fails before any cache exists) --
///   `paired: true`, `capabilities` is `None` or has `analysisLogs: false`.
///   The Progression/Analysis screens stay hidden either way; the
///   home/settings screen shows the one neutral line instead.
/// - paired and entitled -- `paired: true`, `capabilities.analysisLogs ==
///   true`. Both screens render.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitiesState {
    pub paired: bool,
    pub capabilities: Option<Capabilities>,
    /// What the billing of the space limits, read with the capabilities and
    /// cached beside them: `{ "kind": "paymentFailed", "since", "readOnlyUntil" }`
    /// while a payment that failed keeps the space read-only, else absent. The
    /// interface compares `readOnlyUntil` with its own clock.
    pub standing: Option<Standing>,
}

/// `GET /v1/capabilities`, refreshed and cached
/// (`gui_bridge::refresh_capabilities`): tries the network first, falls
/// back to whatever was last cached at `paths.capabilities_cache` on any
/// failure, so this never blocks or errors out just because the portal is
/// unreachable -- an offline-but-paired device keeps seeing exactly the
/// screens its last known profile allowed. Never called with a stale portal
/// URL/token pair: both come from the same settings/keyring
/// `require_portal_and_token` reads for every other portal call.
#[tauri::command]
pub async fn capabilities<R: Runtime>(
    app: AppHandle<R>,
) -> Result<CapabilitiesState, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let settings = gui_bridge::load_settings(&paths.settings)?;
        let token = match &settings.portal_url {
            Some(portal_url) => gui_bridge::load_device_token(&paths.device_token, portal_url)?,
            None => None,
        };
        let (portal_url, token) = match (settings.portal_url, token) {
            (Some(portal_url), Some(token)) => (portal_url, token),
            // Not paired: no account to hold a capability profile at all.
            _ => {
                return Ok(CapabilitiesState {
                    paired: false,
                    capabilities: None,
                    standing: None,
                });
            }
        };
        let (waiting, hooks) = service_wait(&app);
        // With a cached profile the refresh must not hold the window up for a
        // sleeping service: one short, silent attempt. With none there is
        // nothing else to show, so it waits like any other call, and says so.
        let (policy, announce) = gui_bridge::capabilities_refresh_plan(
            &paths.capabilities_cache,
            &waiting.patient,
            &waiting.background,
        );
        let hooks = if announce { hooks } else { WaitHooks::silent() };
        let transport = HttpPortalGuiTransport::with_options(policy, hooks)?;
        let (capabilities, standing) = match gui_bridge::refresh_capabilities(
            &transport,
            &paths.capabilities_cache,
            &portal_url,
            &token,
        ) {
            Ok(cache) => (Some(cache.capabilities), cache.standing),
            // The service says this computer is no longer attached to a
            // space: the sign-in ends, and there is nothing paired left.
            Err(gui_bridge::GuiBridgeError::SignInEnded(end)) => {
                if end_sign_in_now(&app, end) {
                    return Ok(CapabilitiesState {
                        paired: false,
                        capabilities: None,
                        standing: None,
                    });
                }
                (None, None)
            }
            Err(_) => (None, None),
        };
        Ok(CapabilitiesState {
            paired: true,
            capabilities,
            standing,
        })
    })
    .await
}

/* -------------------------------- Sessions ------------------------------ */

#[tauri::command]
pub async fn list_sessions<R: Runtime>(
    app: AppHandle<R>,
    since: Option<String>,
    limit: Option<u32>,
) -> Result<SessionListPage, CommandError> {
    run_blocking(move || {
        let (portal_url, token) = require_portal_and_token(&app)?;
        let (waiting, hooks) = service_wait(&app);
        let transport = HttpPortalGuiTransport::with_options(waiting.patient, hooks)?;
        let page =
            gui_bridge::list_sessions(&transport, &portal_url, &token, since.as_deref(), limit)
                .map_err(CommandError::from);
        ending_sign_in_on_error(&app, page)
    })
    .await
}

/* ----------------------------- Association ------------------------------ */

/// Opens the OS-native folder picker (`tauri-plugin-dialog`). `None` means
/// the user dismissed it -- not an error.
///
/// An `async fn`, never a plain one: a plain command runs on the main
/// thread, which is the thread that has to run the dialog --
/// `blocking_pick_folder` there is a deadlock that froze the Settings screen
/// for good. See [`crate::dialogs`].
#[tauri::command]
pub async fn choose_folder<R: Runtime>(
    app: AppHandle<R>,
    start: Option<String>,
) -> Result<Option<String>, CommandError> {
    // The picker opens on the last folder chosen, when it is still there.
    let start = start
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_dir());
    dialogs::pick(|answer| {
        let picker = app.dialog().file();
        match start {
            Some(path) => picker.set_directory(path).pick_folder(answer),
            None => picker.pick_folder(answer),
        }
    })
    .await
}

/* ------------------------------ Local work ------------------------------- */
// Every local command that reads a folder -- the walk and the hashes, the dry
// run, the package, the queue, the analysis, Siril -- runs on a blocking thread
// of its own ([`run_blocking`]), never on the thread that received the request
// (the main thread: the window would freeze on a real night). It reports its
// progress through [`LOCAL_WORK_EVENT`] and stops on [`local_work_cancel`]. The
// digests come from the cache (`digest-cache.sqlite`) when the file is
// unchanged.

/// The Tauri event of a local work's progress, carrying
/// `astro_sync_core::local_work::WorkProgress`'s JSON
/// (`{"kind":"scan"|"headers"|"analysis","done","total","bytesDone",
/// "bytesTotal","current"}`).
pub const LOCAL_WORK_EVENT: &str = "astro-sync://local-work";

/// « Arrêter »: stops the local works in progress (several can run together:
/// each holds a switch of its own), which answer with the `cancelled` code. Nothing is half written: a local work writes nothing
/// in the person's folder.
#[tauri::command]
pub fn local_work_cancel(control: State<LocalWorkControl>) {
    control.cancel_all();
}

/// « Arrêter l'empilement »: stops the local works of one kind in progress
/// (the stacking), the others -- the progression images, the report of the
/// logs -- go on.
#[tauri::command]
pub fn local_work_cancel_kind(kind: WorkKind, control: State<LocalWorkControl>) {
    control.cancel_kind(kind);
}

/// The stop switch, armed again, and a progress sink that emits
/// [`LOCAL_WORK_EVENT`].
pub(crate) fn local_work<R: Runtime>(
    app: &AppHandle<R>,
) -> (CancelFlag, impl Fn(&WorkProgress) + Send + Sync + 'static) {
    local_work_of(app, None)
}

/// [`local_work`] for a work of a known kind, which `local_work_cancel_kind`
/// can stop on its own.
pub(crate) fn local_work_of<R: Runtime>(
    app: &AppHandle<R>,
    kind: Option<WorkKind>,
) -> (CancelFlag, impl Fn(&WorkProgress) + Send + Sync + 'static) {
    let control = app.state::<LocalWorkControl>();
    let cancel = kind.map_or_else(|| control.arm(), |kind| control.arm_kind(kind));
    let handle = app.clone();
    let progress = move |progress: &WorkProgress| {
        let _ = handle.emit(LOCAL_WORK_EVENT, progress);
    };
    (cancel, progress)
}

/// A scan of `folder` with the cache, the stop switch and the progress.
/// The switch is armed by the command itself, before the blocking thread
/// starts (`local_work` in the command's body): armed there, a « Arrêter »
/// sent while the thread is still starting is never wiped out.
fn scan_with_cache<R: Runtime>(
    app: &AppHandle<R>,
    folder: &str,
    cancel: &CancelFlag,
    progress: &(dyn Fn(&WorkProgress) + Sync),
) -> Result<ScanReport, CommandError> {
    let paths = paths::resolve(app)?;
    let cache = DigestCache::open(&paths.digest_cache)?;
    Ok(scanner::scan_folder_with(
        Path::new(folder),
        &ScanOptions {
            cache: Some(&cache),
            cancel: Some(cancel),
            progress: Some(progress),
            hash_only: None,
        },
    )?)
}

/// What Astro Sync keeps about this computer's folders and logs: the
/// working folder chosen, the night folders found again, the UTC offsets of
/// the PHD2 and ASIAIR logs.
#[tauri::command]
pub fn local_settings_get<R: Runtime>(app: AppHandle<R>) -> Result<LocalSettings, CommandError> {
    let paths = paths::resolve(&app)?;
    Ok(local_settings::load(&paths.local_settings)?)
}

/// Sets the clock of the PHD2, ASIAIR and NINA logs (`+HH:MM`, `null` for
/// UTC): the setting the window lacked (the command line has
/// `--phd2-utc-offset` / `--asiair-utc-offset` / `--nina-utc-offset`; an offset left empty is found from the frames). A bad
/// offset is refused with `bad_utc_offset` and nothing is kept.
#[tauri::command]
pub fn local_settings_set_utc_offsets<R: Runtime>(
    app: AppHandle<R>,
    phd2_utc_offset: Option<String>,
    asiair_utc_offset: Option<String>,
    nina_utc_offset: Option<String>,
) -> Result<LocalSettings, CommandError> {
    let paths = paths::resolve(&app)?;
    let mut settings = local_settings::load(&paths.local_settings)?;
    let mut changed = settings.clone();
    changed.set_utc_offsets(phd2_utc_offset.as_deref(), asiair_utc_offset.as_deref())?;
    changed.set_nina_utc_offset(nina_utc_offset.as_deref())?;
    settings = changed;
    local_settings::save(&paths.local_settings, &settings)?;
    Ok(settings)
}

/// Sets the contrast of presentation of every render, night and plan
/// (the slider of « Traitements »: `none`, `presentation` or `strong`).
/// Kept in the local settings; the stacking and the image of a plan read it
/// when they run, as `--contrast` does on the command line.
#[tauri::command]
pub fn local_settings_set_contrast<R: Runtime>(
    app: AppHandle<R>,
    contrast: astro_sync_core::stacking::Contrast,
) -> Result<LocalSettings, CommandError> {
    let paths = paths::resolve(&app)?;
    let mut settings = local_settings::load(&paths.local_settings)?;
    settings.contrast = contrast;
    local_settings::save(&paths.local_settings, &settings)?;
    Ok(settings)
}

/// Sets the colour intensity of the finish of every render, night and plan
/// (the second slider of « Traitements »: `none`, `enhanced` or `strong`).
/// Kept in the local settings, read when the stacking and the image of a plan
/// run, as `--colour` does on the command line.
#[tauri::command]
pub fn local_settings_set_colour<R: Runtime>(
    app: AppHandle<R>,
    colour: astro_sync_core::stacking::Colour,
) -> Result<LocalSettings, CommandError> {
    let paths = paths::resolve(&app)?;
    let mut settings = local_settings::load(&paths.local_settings)?;
    settings.colour = colour;
    local_settings::save(&paths.local_settings, &settings)?;
    Ok(settings)
}

/// Sets the softened stars of the finish of every render, night and plan
/// (the third slider of « Traitements »: `none`, `light` or `strong`).
/// Kept in the local settings, read when the stacking and the image of a plan
/// run, as `--soft-stars` does on the command line.
#[tauri::command]
pub fn local_settings_set_soft_stars<R: Runtime>(
    app: AppHandle<R>,
    soft_stars: astro_sync_core::stacking::SoftStars,
) -> Result<LocalSettings, CommandError> {
    let paths = paths::resolve(&app)?;
    let mut settings = local_settings::load(&paths.local_settings)?;
    settings.soft_stars = soft_stars;
    local_settings::save(&paths.local_settings, &settings)?;
    Ok(settings)
}

/// Sets the engine of the « Rendu soigné » (Réglages › Général:
/// `graxpert-ai`, the default, or `siril`). Kept in the local settings; the
/// stacking reads it when it runs, as `--polished-engine` does on the
/// command line.
#[tauri::command]
pub fn local_settings_set_polished_engine<R: Runtime>(
    app: AppHandle<R>,
    engine: astro_sync_core::stacking::PolishedEngine,
) -> Result<LocalSettings, CommandError> {
    let paths = paths::resolve(&app)?;
    let mut settings = local_settings::load(&paths.local_settings)?;
    settings.polished_engine = engine;
    local_settings::save(&paths.local_settings, &settings)?;
    Ok(settings)
}

/// « Image du plan multi-nuits » (Réglages › Général): the frame the image of
/// a multi-night plan is cut to -- `clean` (« Bords nets », the default),
/// `common` (« Zone commune à toutes les nuits ») or `whole` (« Tout le
/// champ »). Kept in the local settings, read when the image of a plan is
/// made, as `--plan-field` does on the command line.
#[tauri::command]
pub fn local_settings_set_plan_field<R: Runtime>(
    app: AppHandle<R>,
    field: astro_sync_core::plan_image::PlanField,
) -> Result<LocalSettings, CommandError> {
    let paths = paths::resolve(&app)?;
    let mut settings = local_settings::load(&paths.local_settings)?;
    settings.plan_field = field;
    local_settings::save(&paths.local_settings, &settings)?;
    Ok(settings)
}

/// Keeps the last folders chosen: the window of « Détecter les
/// sessions » and the sending's step 2 open on them next time. A folder
/// given `null` is forgotten; the others are kept as sent.
#[tauri::command]
pub fn local_settings_set_last_folders<R: Runtime>(
    app: AppHandle<R>,
    folders: astro_sync_core::local_settings::LastFolders,
) -> Result<LocalSettings, CommandError> {
    let paths = paths::resolve(&app)?;
    let mut settings = local_settings::load(&paths.local_settings)?;
    settings.last_folders = folders;
    local_settings::save(&paths.local_settings, &settings)?;
    Ok(settings)
}

/// The UTC offsets of the logs the person gave in the settings; the others are
/// found from the frames when a night's logs are read (the command line does
/// the same with `--phd2-utc-offset` and its kin).
pub(crate) fn forced_log_offsets<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<ForcedLogOffsets, CommandError> {
    let paths = paths::resolve(app)?;
    Ok(local_settings::load(&paths.local_settings)?.forced_log_offsets())
}

/* --------------------------- Scan / dry-run / package -------------------- */

#[tauri::command]
pub async fn scan_folder<R: Runtime>(
    app: AppHandle<R>,
    folder: String,
) -> Result<ScanReport, CommandError> {
    let (cancel, progress) = local_work(&app);
    run_blocking(move || scan_with_cache(&app, &folder, &cancel, &progress)).await
}

/// Scans, then builds the exact file/byte/hash/destination report a
/// package would contain -- nothing is written or transferred.
#[tauri::command]
pub async fn dry_run_folder<R: Runtime>(
    app: AppHandle<R>,
    folder: String,
) -> Result<DryRunReport, CommandError> {
    let (cancel, progress) = local_work(&app);
    run_blocking(move || {
        let scan = scan_with_cache(&app, &folder, &cancel, &progress)?;
        Ok(dry_run::build_package_report(&scan))
    })
    .await
}

/// `SessionMeta`/`ProducerInfo` (`astro_sync_core::package`) are
/// serialize-only -- they describe what a *produced* package says about
/// itself, never a value the GUI sends in. This is the small,
/// deserializable shape the confirm dialog submits instead.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageSessionInput {
    pub label: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

/// Re-scans `folder`, then writes `astro-session-package/` under
/// `output_parent` -- the explicit-confirmation step of the dry-run
/// screen. Re-scanning (rather than trusting a report the frontend already
/// holds) means the exact bytes/hashes just written are always the ones
/// package.rs itself observed, never a stale copy the UI cached (the cache
/// answers only for a file of the same size and time).
#[tauri::command]
pub async fn produce_package<R: Runtime>(
    app: AppHandle<R>,
    folder: String,
    output_parent: String,
    session: Option<PackageSessionInput>,
) -> Result<PackageOutcome, CommandError> {
    let (cancel, progress) = local_work(&app);
    run_blocking(move || {
        let scan = scan_with_cache(&app, &folder, &cancel, &progress)?;
        let report = dry_run::build_package_report(&scan);
        let session_meta = session.map(|session| SessionMeta {
            label: session.label,
            started_at: session.started_at,
            finished_at: session.finished_at,
        });
        let producer = Some(ProducerInfo {
            name: "Astro Sync (GUI)".to_string(),
            version: Some(astro_sync_core::version().to_string()),
        });
        Ok(package::produce_package(
            &report,
            Path::new(&output_parent),
            session_meta,
            producer,
        )?)
    })
    .await
}

/* --------------------------------- Queue --------------------------------- */

/// Re-scans `folder` and enqueues the results among its files (a final or a
/// stacked image): the frames, the calibration and the raw logs are left out
/// (`leftOut`) and never leave the computer. Idempotent on content hash: a
/// file already known (in any state) is left untouched
/// (`UploadQueue::enqueue_from_dry_run`).
#[tauri::command]
pub async fn enqueue_folder<R: Runtime>(
    app: AppHandle<R>,
    folder: String,
) -> Result<EnqueueOutcome, CommandError> {
    let (cancel, progress) = local_work(&app);
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let scan = scan_with_cache(&app, &folder, &cancel, &progress)?;
        let report = dry_run::build_dry_run_report(&scan);
        let queue = paths::open_queue(&paths)?;
        Ok(queue.enqueue_from_dry_run(&report)?)
    })
    .await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueStatusPayload {
    pub entries: Vec<QueueEntry>,
    pub summary: QueueSummary,
    /// What became of the sky annotations of the files sent (kept beside
    /// their rows).
    pub sky: Vec<astro_sync_core::sky_after_upload::SkyAnnotation>,
    /// The space the queue is seen from (`space.json`), `None` when this
    /// computer is not linked to one: the rows above are that space's only.
    pub space_id: Option<String>,
    /// The rows written before the queue knew its spaces, shown apart
    /// (« Avant la liaison ») while a space is linked; empty otherwise.
    pub before_link: Vec<QueueEntry>,
}

/// The queue of the linked space (`entries`), and apart what came before the
/// link (`beforeLink`).
#[tauri::command]
pub fn queue_status<R: Runtime>(app: AppHandle<R>) -> Result<QueueStatusPayload, CommandError> {
    let paths = paths::resolve(&app)?;
    let queue = paths::open_queue(&paths)?;
    let entries = queue.list()?;
    let summary = gui_bridge::summarize_queue(&entries);
    let sky = crate::commands_sky::kept(&queue);
    Ok(QueueStatusPayload {
        entries,
        summary,
        sky,
        space_id: queue.space_id().map(str::to_string),
        before_link: queue.list_before_link()?,
    })
}

/// « Effacer l'historique », for what came before the link: those rows are
/// deleted (the linked space's history is cleared by the window, as it was).
/// Returns how many rows went.
#[tauri::command]
pub fn queue_forget_before_link<R: Runtime>(app: AppHandle<R>) -> Result<usize, CommandError> {
    let paths = paths::resolve(&app)?;
    Ok(paths::open_queue(&paths)?.forget_before_link()?)
}

/// « Renvoyer » on a file already sent to the linked space: it goes back in
/// the queue; the next [`upload_all`] sends it, and the service answers with
/// the capture it still holds (`deduplicated`) or makes a new one. Returns
/// whether the file was requeued.
#[tauri::command]
pub fn upload_resend<R: Runtime>(app: AppHandle<R>, sha256: String) -> Result<bool, CommandError> {
    let paths = paths::resolve(&app)?;
    Ok(paths::open_queue(&paths)?.requeue_sent(&sha256)?)
}

/// A thin, `Serialize`-able view of `astro_sync_core::uploader::UploadOutcome`
/// (which is deliberately not `Serialize` in the core -- it has no JSON
/// boundary to cross there). Field-for-field, nothing recomputed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadOutcomePayload {
    pub capture_id: String,
    pub deduplicated: bool,
}

impl From<astro_sync_core::uploader::UploadOutcome> for UploadOutcomePayload {
    fn from(outcome: astro_sync_core::uploader::UploadOutcome) -> Self {
        Self {
            capture_id: outcome.capture_id,
            deduplicated: outcome.deduplicated,
        }
    }
}

/// The Tauri event name every progress/lifecycle notification from
/// [`upload_next`] and [`upload_all`] is emitted under. One name for both,
/// carrying `astro_sync_core::gui_bridge::QueueDriveEvent`'s own
/// `#[serde(tag = "kind")]` JSON, which is what the frontend matches on.
pub const UPLOAD_EVENT: &str = "astro-sync://upload-event";

/// Uploads exactly one pending queue entry (the oldest `queued`/
/// `uploading` row), emitting [`UPLOAD_EVENT`] progress notifications as it
/// goes. `Ok(None)` means the queue has nothing left to send.
#[tauri::command]
pub async fn upload_next<R: Runtime>(
    app: AppHandle<R>,
    session_id: Option<String>,
    plan_id: Option<String>,
    target_ids: Vec<String>,
) -> Result<Option<UploadOutcomePayload>, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let (portal_url, token) = require_portal_and_token(&app)?;
        let queue = paths::open_queue(&paths)?;
        // Asking for the next file is the person acting: what was held for
        // them (a full storage they have since made room in) is tried again.
        queue.release_held()?;
        let Some(entry) = queue.next_pending()? else {
            return Ok(None);
        };
        let (waiting, hooks) = service_wait(&app);
        let transport = HttpUploadTransport::with_options(&portal_url, waiting.patient, hooks)?;
        let target = UploadTarget {
            session_id,
            plan_id,
            target_ids,
        };
        let sha256 = entry.sha256.clone();
        let bytes = entry.size;
        let source_path = entry.source_path.clone();
        let _ = app.emit(
            UPLOAD_EVENT,
            gui_bridge::QueueDriveEvent::Started {
                sha256: sha256.clone(),
                source_path,
                bytes,
            },
        );
        let outcome = upload_entry(
            &transport,
            &queue,
            &token,
            &entry,
            &target,
            |sent, total| {
                let _ = app.emit(
                    UPLOAD_EVENT,
                    gui_bridge::QueueDriveEvent::Progress {
                        sha256: sha256.clone(),
                        sent,
                        total,
                    },
                );
            },
        )
        .map_err(CommandError::from);
        // A sign-in the service ended leaves the file queued (it is not at
        // fault) and brings the person back to the sign-in screen.
        let outcome = ending_sign_in_on_error(&app, outcome)?;
        let _ = app.emit(
            UPLOAD_EVENT,
            gui_bridge::QueueDriveEvent::Completed {
                sha256: entry.sha256.clone(),
                capture_id: outcome.capture_id.clone(),
                deduplicated: outcome.deduplicated,
                skipped: outcome.skipped.clone(),
                replaced: outcome.replaced,
            },
        );
        // The sky annotations of the image just sent, off this thread.
        crate::commands_sky::spawn_annotation(
            &app,
            entry.sha256.clone(),
            portal_url.clone(),
            token.clone(),
        );
        Ok(Some(outcome.into()))
    })
    .await
}

/// Drains the whole queue via [`gui_bridge::drive_queue`], emitting
/// [`UPLOAD_EVENT`] for every step, and stopping either when the queue is
/// empty, an entry makes no progress twice in a row, or [`upload_pause`]
/// was called since this run started. Returns the number of files this
/// call finished. A paused run resumes by simply calling `upload_all`
/// again after [`upload_resume`] -- the queue is left exactly where
/// progress had recorded it either way.
#[tauri::command]
pub async fn upload_all<R: Runtime>(
    app: AppHandle<R>,
    session_id: Option<String>,
    plan_id: Option<String>,
    target_ids: Vec<String>,
) -> Result<usize, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let (portal_url, token) = require_portal_and_token(&app)?;
        let queue = paths::open_queue(&paths)?;
        let (waiting, hooks) = service_wait(&app);
        let transport = HttpUploadTransport::with_options(&portal_url, waiting.patient, hooks)?;
        let target = UploadTarget {
            session_id,
            plan_id,
            target_ids,
        };
        let control = app.state::<UploadControl>();
        control.paused.store(false, Ordering::SeqCst);
        let paused = control.paused.clone();
        // The window waits for as long as it takes -- longer each time --
        // until the person pauses: a file the service is too busy to take is
        // never marked, and a refusal for good is set aside, never resent.
        let processed = gui_bridge::drive_queue(
            &transport,
            &queue,
            &token,
            &target,
            &gui_bridge::DriveOptions::interactive(),
            || !paused.load(Ordering::SeqCst),
            |event| {
                // The run stops on a sign-in the service ended (the file
                // stays queued, marked by nothing): forget the token, keep
                // the reason, and bring the person back to the sign-in
                // screen.
                if let gui_bridge::QueueDriveEvent::Failed { code, .. } = &event {
                    if let Some(end) = SignInEnd::from_code(code) {
                        end_sign_in_now(&app, end);
                    }
                }
                // The file is on the service: its sky annotations are made
                // on a thread of their own, so the next file never waits for
                // a solver.
                if let gui_bridge::QueueDriveEvent::Completed { sha256, .. } = &event {
                    crate::commands_sky::spawn_annotation(
                        &app,
                        sha256.clone(),
                        portal_url.clone(),
                        token.clone(),
                    );
                }
                let _ = app.emit(UPLOAD_EVENT, event);
            },
        )?;
        Ok(processed)
    })
    .await
}

/// « Réessayer » on a file set aside because it was not found: the file (or,
/// with `None`, every such file) goes back in the queue, naming the file where
/// a renamed folder now keeps it. The next [`upload_all`] sends it, or says
/// again which file is missing and where it was.
#[tauri::command]
pub fn upload_requeue_missing(
    app: AppHandle,
    sha256: Option<String>,
) -> Result<usize, CommandError> {
    let paths = paths::resolve(&app)?;
    let queue = paths::open_queue(&paths)?;
    let missing = astro_sync_core::uploader::SOURCE_FILE_MISSING;
    let wanted: Vec<String> = queue
        .list()?
        .into_iter()
        .filter(|entry| {
            entry.state == astro_sync_core::upload_queue::QueueState::Failed
                && entry.last_error_code.as_deref() == Some(missing)
                && sha256.as_deref().is_none_or(|sha| sha == entry.sha256)
        })
        .map(|entry| entry.sha256)
        .collect();
    for sha in &wanted {
        queue.requeue_failed(sha)?;
    }
    Ok(wanted.len())
}

/// Requests that the current (or next) [`upload_all`] run stop before its
/// next file. The file already in flight, if any, still runs to its own
/// natural outcome -- every file is independently resumable, so nothing is
/// lost either way.
#[tauri::command]
pub fn upload_pause(control: State<UploadControl>) {
    control.paused.store(true, Ordering::SeqCst);
}

/// Clears the pause flag. Does not by itself resume processing -- the
/// frontend calls [`upload_all`] again to continue.
#[tauri::command]
pub fn upload_resume(control: State<UploadControl>) {
    control.paused.store(false, Ordering::SeqCst);
}

/* ------------------------- The measured sky background ------------------ */

/// Fetches the Tycho-2 reference list (CDS VizieR I/259, about 25 MB once
/// made) and keeps it next to the settings as `reference-stars.bin`, where
/// the night's analysis reads it (`night_analysis`). Only the « Installer »
/// of the Tycho-2 row of Réglages › Dépendances calls it, whose line names
/// the list, its size and the CDS: the call is the person's agreement. The file is written whole or not at all.
/// Several minutes of requests: off the request thread.
#[tauri::command]
pub async fn download_reference_stars<R: Runtime>(
    app: AppHandle<R>,
) -> Result<ReferenceStarsDownloaded, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let list = reference_catalogue::download_tycho2(
            Tycho2Consent::given(),
            reference_catalogue::DEFAULT_FAINTEST,
            &mut |_, _| {},
        )
        .map_err(|error| CommandError::new("reference_stars_download_failed", error.to_string()))?;
        let temporary = paths.reference_stars.with_extension("bin.part");
        let write = std::fs::write(&temporary, list.to_bytes())
            .and_then(|()| std::fs::rename(&temporary, &paths.reference_stars));
        if let Err(error) = write {
            let _ = std::fs::remove_file(&temporary);
            return Err(CommandError::new(
                "reference_stars_download_failed",
                error.to_string(),
            ));
        }
        Ok(ReferenceStarsDownloaded { stars: list.len() })
    })
    .await
}

/// What [`download_reference_stars`] answers.
#[derive(serde::Serialize)]
pub struct ReferenceStarsDownloaded {
    pub stars: usize,
}

/* ------------------------------ The hosted space ------------------------- */
// Astro Sync signs in to the hosted service with an e-mail address, then the
// code of six digits received by e-mail, announcing itself as the
// application `sync`. What it does lives in
// `astro_sync_core::space`; these commands only resolve paths, choose the
// waiting policy and hand the results to the page. A private server is
// untouched: it keeps `device_pair`.

/// The Tauri event the interface listens to for a sign-in the service ended:
/// `{"kind":"signed_out","reason":"device_detached"|"space_deleted"}`. The
/// page goes back to the sign-in screen and says why; the token is already
/// forgotten, and nothing else is lost -- the folders associated with
/// sessions and the upload queue stay as they were.
pub const SPACE_EVENT: &str = "astro-sync://space-event";

#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SpaceEvent {
    SignedOut { reason: SignInEnd },
}

/// Where the device token of the service `settings` points at is kept -- the
/// same place a pairing keeps it.
fn token_store(paths: &AppPaths, settings: &GuiSettings) -> SystemTokenStore {
    SystemTokenStore::new(paths.device_token.clone(), space::service_url(settings))
}

/// The service a sign-in step talks to: the address the page names (a private
/// server it was told about), else the hosted service.
fn chosen_service(portal_url: Option<String>) -> String {
    portal_url
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| space::HOSTED_SERVICE_ORIGIN.to_string())
}

/// The client of the sign-in for `base_url`. `background` is for a refresh
/// nobody is waiting on: one short attempt, never announced.
fn space_client<R: Runtime>(
    app: &AppHandle<R>,
    base_url: &str,
    background: bool,
) -> Result<SpaceClient, CommandError> {
    let (waiting, hooks) = service_wait(app);
    let (policy, hooks) = if background {
        (waiting.background, WaitHooks::silent())
    } else {
        (waiting.patient, hooks)
    };
    Ok(SpaceClient::with_options(base_url, policy, hooks)?)
}

/// The name this computer signs in under: the one it already carries, else
/// « Astro Sync — <machine name> ».
fn sign_in_name(settings: &GuiSettings) -> String {
    settings
        .device_label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map_or_else(space::default_device_name, str::to_string)
}

/// Tells the page this computer was signed out, and forgets what belonged to
/// the space it left: the capabilities of one space must not be served for
/// another.
fn announce_signed_out<R: Runtime>(app: &AppHandle<R>, end: SignInEnd) {
    if let Ok(paths) = paths::resolve(app) {
        let _ = std::fs::remove_file(&paths.capabilities_cache);
    }
    let _ = app.emit(SPACE_EVENT, SpaceEvent::SignedOut { reason: end });
}

/// The service said this computer is no longer attached to its space
/// (`device_detached`, `space_deleted`), or no longer knows the token (a
/// newer sign-in of this installation replaced it,
/// `device_authentication_required`): the token is forgotten, the reason kept
/// for the sign-in screen, the page told. **Nothing else is touched**: not the
/// folders associated with sessions, not the upload queue.
///
/// Returns whether the sign-in ended. It does not when this computer is not
/// signed in to a space and the service only says the token is unknown: on a
/// private server that is an ordinary revoked device, left as it always was.
fn end_sign_in_now<R: Runtime>(app: &AppHandle<R>, end: SignInEnd) -> bool {
    let Ok(paths) = paths::resolve(app) else {
        return false;
    };
    let Ok(settings) = gui_bridge::load_settings(&paths.settings) else {
        return false;
    };
    let files = paths.space_files();
    if !space::ends_this_sign_in(&files, end).unwrap_or(false) {
        return false;
    }
    if space::end_sign_in(&files, &token_store(&paths, &settings), end).is_ok() {
        announce_signed_out(app, end);
        return true;
    }
    false
}

/// Passes a command's result through, first ending the sign-in when its error
/// says the service did.
pub(crate) fn ending_sign_in_on_error<R: Runtime, T>(
    app: &AppHandle<R>,
    result: Result<T, CommandError>,
) -> Result<T, CommandError> {
    if let Err(error) = &result {
        if let Some(end) = SignInEnd::from_code(error.code) {
            end_sign_in_now(app, end);
        }
    }
    result
}

/// What the service in front of us announces about how a person signs in
/// (`GET /api/v1/service`, `signIn`): the page shows the screen that
/// fits -- an e-mail address for the hosted service, the code the server shows
/// for a private one. `portal_url` names a private server; without it, the
/// hosted service.
#[tauri::command]
pub async fn service_announcement<R: Runtime>(
    app: AppHandle<R>,
    portal_url: Option<String>,
) -> Result<SignInAnnouncement, CommandError> {
    run_blocking(move || {
        let client = space_client(&app, &chosen_service(portal_url), false)?;
        Ok(client.announcement()?)
    })
    .await
}

/// The state of the connection, from the files alone (no network): whether
/// this computer is connected, to which space, under what name, or why the
/// last sign-in ended.
#[tauri::command]
pub fn space_status<R: Runtime>(app: AppHandle<R>) -> Result<SpaceStatus, CommandError> {
    let paths = paths::resolve(&app)?;
    let settings = gui_bridge::load_settings(&paths.settings)?;
    let paired = match &settings.portal_url {
        Some(portal_url) => {
            gui_bridge::load_device_token(&paths.device_token, portal_url)?.is_some()
        }
        None => false,
    };
    Ok(space::space_status(&paths.space_files(), paired)?)
}

/// `POST /v1/sign-in/code`: has a code of six digits sent to `email`, in the
/// language of the interface.
#[tauri::command]
pub async fn space_request_code<R: Runtime>(
    app: AppHandle<R>,
    portal_url: Option<String>,
    email: String,
    language: String,
) -> Result<CodeRequested, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let settings = gui_bridge::load_settings(&paths.settings)?;
        let device =
            DeviceIdentity::for_installation(&paths.space_files(), &sign_in_name(&settings))?;
        let client = space_client(&app, &chosen_service(portal_url), false)?;
        Ok(client.request_code(&email, &language, &device)?)
    })
    .await
}

/// What a verified sign-in gives the page: whether it created the space
/// (« Votre espace est prêt ») and the connection as it now stands. Never the
/// token.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignInOutcome {
    pub created: bool,
    pub status: SpaceStatus,
}

/// `POST /v1/sign-in/verify`: exchanges the code for a device token, kept
/// like the token of a pairing (system keyring under the service's address,
/// owner-only file as a fallback), and remembers the space and the name of
/// this computer.
#[tauri::command]
pub async fn space_verify_code<R: Runtime>(
    app: AppHandle<R>,
    portal_url: Option<String>,
    email: String,
    code: String,
) -> Result<SignInOutcome, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let files = paths.space_files();
        let settings = gui_bridge::load_settings(&paths.settings)?;
        let base_url = chosen_service(portal_url);
        let device = DeviceIdentity::for_installation(&files, &sign_in_name(&settings))?;
        let client = space_client(&app, &base_url, false)?;
        let signed_in = client.verify_code(&email, &code, &device)?;
        let store = SystemTokenStore::new(paths.device_token.clone(), base_url.clone());
        let status = space::complete_sign_in(&files, &store, &base_url, &signed_in)?;
        // What a previous space granted is not this one's.
        let _ = std::fs::remove_file(&paths.capabilities_cache);
        Ok(SignInOutcome {
            created: signed_in.created,
            status,
        })
    })
    .await
}

/// Asks the service who this computer is (`GET /v1/space`) and remembers the
/// answer -- the web address of the space, the name of this computer as
/// « Mes appareils » lists it. One short, silent attempt: what is on disk
/// answers when the service does not, and a service that is asleep is not
/// waited for. A sign-in the service ended is announced to the page.
#[tauri::command]
pub async fn space_refresh<R: Runtime>(app: AppHandle<R>) -> Result<SpaceStatus, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let files = paths.space_files();
        let settings = gui_bridge::load_settings(&paths.settings)?;
        let token = match &settings.portal_url {
            Some(portal_url) => gui_bridge::load_device_token(&paths.device_token, portal_url)?,
            None => None,
        };
        let (Some(portal_url), Some(token)) = (settings.portal_url.clone(), token) else {
            return Ok(space::space_status(&files, false)?);
        };
        // A pairing has no space to ask about.
        if space::load_record(&files.record)?.link.is_none() {
            return Ok(space::space_status(&files, true)?);
        }
        let client = space_client(&app, &portal_url, true)?;
        match space::refresh_space(&client, &files, &token_store(&paths, &settings), &token) {
            Ok(status) => Ok(status),
            Err(error) => match error.sign_in_end() {
                Some(end) => {
                    announce_signed_out(&app, end);
                    Ok(space::space_status(&files, false)?)
                }
                None => Ok(space::space_status(&files, true)?),
            },
        }
    })
    .await
}

/// Changes the name of this computer (`PATCH /v1/devices/{id}`), at the
/// service when it answers that route, and here.
#[tauri::command]
pub async fn space_rename<R: Runtime>(
    app: AppHandle<R>,
    name: String,
) -> Result<RenameOutcome, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let (portal_url, token) = require_portal_and_token(&app)?;
        let settings = gui_bridge::load_settings(&paths.settings)?;
        let client = space_client(&app, &portal_url, false)?;
        match space::rename_computer(
            &client,
            &paths.space_files(),
            &token_store(&paths, &settings),
            &token,
            &name,
        ) {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                if let Some(end) = error.sign_in_end() {
                    announce_signed_out(&app, end);
                }
                Err(error.into())
            }
        }
    })
    .await
}

/// « Détacher cet ordinateur »: tells the service as far as it can be told
/// (`DELETE /v1/devices/{id}`), forgets the token and the link. The folders
/// associated with sessions and the files waiting in the upload queue stay:
/// they wait for the next sign-in.
#[tauri::command]
pub async fn space_detach<R: Runtime>(app: AppHandle<R>) -> Result<DetachOutcome, CommandError> {
    run_blocking(move || {
        let paths = paths::resolve(&app)?;
        let settings = gui_bridge::load_settings(&paths.settings)?;
        let portal_url = space::service_url(&settings);
        let token =
            gui_bridge::load_device_token(&paths.device_token, &portal_url)?.unwrap_or_default();
        let client = space_client(&app, &portal_url, false)?;
        Ok(space::detach_computer(
            &client,
            &paths.space_files(),
            &token_store(&paths, &settings),
            &token,
        )?)
    })
    .await
}

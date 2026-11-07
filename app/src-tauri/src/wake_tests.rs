//! The desktop app's half of "the service is asleep", driven through
//! Tauri's own IPC on its mock runtime -- the same command wrappers the
//! window calls, the real transports, and a fake gateway that holds
//! connections while the service starts (a second or so here):
//!
//! - the request thread is never blocked while the service wakes. In the
//!   real runtime that thread is the main thread: a command that waits on
//!   it freezes the window, holds back every event the command emits
//!   (delivering an event to the page runs on the main thread) and even
//!   holds back the very `service_cancel` that would end the wait. So a
//!   waiting command must return from `on_message` at once and answer
//!   later;
//! - the interface is told the service is starting (`SERVICE_EVENT`), then
//!   that it answered, and can cancel;
//! - a service that cannot be reached at all still fails at once.
//!
//! The app's own files live under the platform's config directory, and the
//! device token normally goes through the OS keyring: neither must ever be
//! touched by a test. `HOME` is redirected to a throwaway directory before
//! anything resolves a path, the keyring is replaced by `keyring`'s own mock
//! (the token is then read from the legacy file it falls back to), and every
//! test first checks the app really resolves inside that directory.

#[path = "../../../crates/astro-sync-core/tests/support/fake_service.rs"]
pub(crate) mod fake_service;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use astro_sync_core::dry_run::build_dry_run_report;
use astro_sync_core::scanner::scan_folder;
use astro_sync_core::service_wait::{WaitControl, WakePolicy};
use astro_sync_core::upload_queue::UploadQueue;
use tauri::ipc::{CallbackFn, InvokeBody, InvokeResponse};
use tauri::test::MockRuntime;
use tauri::test::{mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{Listener, Manager, WebviewWindow, WebviewWindowBuilder};

use crate::commands::{self, SERVICE_EVENT, SPACE_EVENT, UPLOAD_EVENT};
use crate::state::{LocalWorkControl, ServiceWait, UploadControl};
use fake_service::{
    capabilities_reply, error_reply, identity_reply, sessions_reply, FakeService, Reply,
    UploadModel, Wake,
};

/* --------------------------------- isolation ------------------------------ */

pub(crate) fn unique_dir(label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "astro-sync-app-{label}-{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Redirects `HOME` and swaps in the mock keyring, once for the whole test
/// binary, before any test resolves a path or touches a credential.
fn throwaway_home() -> &'static Path {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        let home = unique_dir("home");
        std::env::set_var("HOME", &home);
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        home
    })
}

/// The tests share one config directory (there is one `HOME`): run them one
/// at a time.
pub(crate) fn serialized() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/* ---------------------------------- harness ------------------------------- */

/// The same shape as production, on a scale of hundreds of milliseconds.
pub(crate) fn fast() -> WakePolicy {
    WakePolicy {
        total_budget: Duration::from_millis(4_000),
        attempt_timeout: Duration::from_millis(500),
        write_timeout: Duration::from_millis(600),
        chunk_timeout: Duration::from_millis(1_500),
        retry_pause: Duration::from_millis(40),
        notice_after: Duration::from_millis(120),
        connect_timeout: Duration::from_millis(500),
    }
}

pub(crate) struct Harness {
    pub(crate) app: tauri::App<MockRuntime>,
    webview: WebviewWindow<MockRuntime>,
    pub(crate) config_dir: PathBuf,
    /// Payloads of every `SERVICE_EVENT` the interface would receive.
    pub(crate) service_events: Arc<Mutex<Vec<serde_json::Value>>>,
    /// Payloads of every `UPLOAD_EVENT`.
    pub(crate) upload_events: Arc<Mutex<Vec<serde_json::Value>>>,
    /// Payloads of every `SPACE_EVENT` (a sign-in the service ended).
    pub(crate) space_events: Arc<Mutex<Vec<serde_json::Value>>>,
    /// Payloads of every `SKY_EVENT` (the annotations of an uploaded image).
    pub(crate) sky_events: Arc<Mutex<Vec<serde_json::Value>>>,
}

pub(crate) fn harness(policy: WakePolicy) -> Harness {
    build_harness(policy, false)
}

/// The harness of the app with the sky annotations after each upload turned
/// on, as in production.
pub(crate) fn harness_with_sky(policy: WakePolicy) -> Harness {
    build_harness(policy, true)
}

fn build_harness(policy: WakePolicy, sky_after_upload: bool) -> Harness {
    let home = throwaway_home();
    let mut context = mock_context(noop_assets());
    context.config_mut().identifier = "space.planastro.astrosync.test".to_string();
    let builder = mock_builder();
    let builder = if sky_after_upload {
        builder.manage(crate::commands_sky::SkyAfterUpload)
    } else {
        builder
    };
    let app = builder
        .manage(UploadControl::default())
        .manage(LocalWorkControl::default())
        .manage(ServiceWait {
            control: WaitControl::default(),
            patient: policy,
            background: fast(),
        })
        .invoke_handler(tauri::generate_handler![
            commands::device_pair,
            commands::list_sessions,
            commands::capabilities,
            commands::app_build_info,
            commands::upload_all,
            commands::queue_status,
            commands::upload_resend,
            commands::queue_forget_before_link,
            crate::commands_sky::sky_retry,
            commands::service_cancel,
            commands::service_announcement,
            commands::space_status,
            commands::space_request_code,
            commands::space_verify_code,
            commands::space_refresh,
            commands::space_rename,
            commands::space_detach,
            commands::scan_folder,
            commands::local_work_cancel,
            commands::local_work_cancel_kind,
            commands::local_settings_get,
            commands::local_settings_set_utc_offsets,
            commands::local_settings_set_contrast,
            commands::local_settings_set_colour,
            commands::local_settings_set_soft_stars,
            commands::local_settings_set_polished_engine,
            commands::local_settings_set_plan_field,
            commands::local_settings_set_renders_upload,
            crate::commands_night::night_analysis,
            crate::commands_night::night_send_logs,
            crate::commands_night::working_folder_get,
            crate::commands_night::working_folder_set,
            crate::commands_night::night_folders_list,
            crate::commands_night::night_locate,
            crate::commands_night::night_follow,
            crate::commands_night::working_folder_prepare,
            crate::commands_night::working_folder_replace_assistant_file,
            crate::commands_night::guide_write,
            crate::commands_night::managed_file_replace,
            crate::commands_night::guide_prompt,
            crate::commands_night::night_folder_plan,
            crate::commands_night::night_folder_prepare,
            crate::commands_night::night_detect,
            crate::commands_detect::sessions_detect,
            crate::commands_detect::sessions_organise_plan,
            crate::commands_detect::sessions_organise,
            crate::commands_night::night_enqueue,
            crate::commands_night::tools_detect,
            crate::commands_night::pipelines_list,
            crate::commands_night::pipeline_sections,
            crate::commands_night::pipeline_trust_set,
            crate::commands_night::tools_verify,
            crate::commands_night::python_env_install,
            crate::commands_processing::python_env_reinstall,
            crate::commands_processing::processing_files_scan,
            crate::commands_processing::processing_files_clean,
            crate::commands_night::assistant_prompt,
        ])
        .build(context)
        .expect("build the mock app");
    let config_dir = app.path().app_config_dir().unwrap();
    assert!(
        config_dir.starts_with(home),
        "the app resolves {} outside the throwaway HOME {}: refusing to run, nothing must touch a real config directory",
        config_dir.display(),
        home.display()
    );
    let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("build the mock window");

    let service_events: Arc<Mutex<Vec<serde_json::Value>>> = Arc::default();
    let sink = Arc::clone(&service_events);
    app.listen(SERVICE_EVENT, move |event| {
        sink.lock()
            .unwrap()
            .push(serde_json::from_str(event.payload()).unwrap());
    });
    let upload_events: Arc<Mutex<Vec<serde_json::Value>>> = Arc::default();
    let sink = Arc::clone(&upload_events);
    app.listen(UPLOAD_EVENT, move |event| {
        sink.lock()
            .unwrap()
            .push(serde_json::from_str(event.payload()).unwrap());
    });
    let space_events: Arc<Mutex<Vec<serde_json::Value>>> = Arc::default();
    let sink = Arc::clone(&space_events);
    app.listen(SPACE_EVENT, move |event| {
        sink.lock()
            .unwrap()
            .push(serde_json::from_str(event.payload()).unwrap());
    });
    let sky_events: Arc<Mutex<Vec<serde_json::Value>>> = Arc::default();
    let sink = Arc::clone(&sky_events);
    app.listen(crate::commands_sky::SKY_EVENT, move |event| {
        sink.lock()
            .unwrap()
            .push(serde_json::from_str(event.payload()).unwrap());
    });
    Harness {
        app,
        webview,
        config_dir,
        service_events,
        upload_events,
        space_events,
        sky_events,
    }
}

impl Harness {
    /// Saves the portal URL and a device token the way a paired app has
    /// them. With the mock keyring the token is read (and migrated) from the
    /// legacy file, which is consumed by each read: call this before each
    /// command that needs it.
    pub(crate) fn pair_with(&self, portal_url: &str) {
        std::fs::create_dir_all(&self.config_dir).unwrap();
        std::fs::write(
            self.config_dir.join("settings.json"),
            format!(r#"{{"portalUrl":"{portal_url}","deviceLabel":"Test","locale":null}}"#),
        )
        .unwrap();
        std::fs::write(self.config_dir.join("device-token"), "test-device-token").unwrap();
    }

    pub(crate) fn service_events(&self) -> Vec<String> {
        self.service_events
            .lock()
            .unwrap()
            .iter()
            .map(|event| event["kind"].as_str().unwrap().to_string())
            .collect()
    }

    /// Sends a command through the IPC exactly as the page does, and reports
    /// how long the *calling* thread was held. In the real runtime that is
    /// the main thread.
    pub(crate) fn invoke(&self, cmd: &str, args: serde_json::Value) -> Pending {
        let (sender, receiver) = mpsc::channel();
        let request = InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        };
        let started = Instant::now();
        self.webview.as_ref().clone().on_message(
            request,
            Box::new(move |_webview, _cmd, response, _callback, _error| {
                let _ = sender.send(response);
            }),
        );
        Pending {
            held_the_caller_for: started.elapsed(),
            receiver,
        }
    }
}

pub(crate) struct Pending {
    pub(crate) held_the_caller_for: Duration,
    receiver: Receiver<InvokeResponse>,
}

impl Pending {
    /// `Ok(json)` or `Err(json)` -- a `CommandError` is `{code, message}`.
    pub(crate) fn answer(self, within: Duration) -> Result<serde_json::Value, serde_json::Value> {
        match self
            .receiver
            .recv_timeout(within)
            .expect("the command answered")
        {
            InvokeResponse::Ok(body) => Ok(body.deserialize::<serde_json::Value>().unwrap()),
            InvokeResponse::Err(error) => Err(error.0),
        }
    }
}

fn portal_app(request: &fake_service::Recorded) -> Reply {
    let path = request.path.as_str();
    match (request.method.as_str(), path) {
        ("GET", "/api/v1/service") => identity_reply(),
        ("GET", "/api/v1/capabilities") => capabilities_reply(),
        ("GET", path) if path.starts_with("/api/v1/sessions") => sessions_reply(),
        ("POST", "/api/v1/devices/claim") => {
            Reply::Json(201, r#"{"deviceToken":"paired-token"}"#.to_string())
        }
        _ => error_reply(404, "not_found", "no such route"),
    }
}

/* ----------------------------------- tests -------------------------------- */

#[test]
fn a_command_that_waits_for_the_service_never_holds_the_request_thread() {
    let _serial = serialized();
    let harness = harness(fast());
    let service = FakeService::start(Wake::Held(Duration::from_millis(1_300)), portal_app);
    harness.pair_with(&service.base_url());

    let pending = harness.invoke(
        "list_sessions",
        serde_json::json!({"since": null, "limit": null}),
    );

    assert!(
        pending.held_the_caller_for < Duration::from_millis(800),
        "the command held the calling thread for {:?} while the service woke: in the app that \
         is the main thread -- frozen window, no event delivered, no cancel possible",
        pending.held_the_caller_for
    );
    let page = pending
        .answer(Duration::from_secs(5))
        .expect("the answer arrives once the service is awake");
    assert_eq!(page["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(harness.service_events(), ["starting", "ready"]);
    assert_eq!(service.app_hits("GET", "/api/v1/sessions"), 1);
    drop(harness.app);
}

#[test]
fn pairing_wakes_the_service_first_and_presents_the_code_exactly_once() {
    let _serial = serialized();
    let harness = harness(fast());
    let service = FakeService::start(Wake::Held(Duration::from_millis(1_300)), portal_app);
    let _ = std::fs::remove_file(harness.config_dir.join("settings.json"));

    let pending = harness.invoke(
        "device_pair",
        serde_json::json!({
            "portalUrl": service.base_url(),
            "code": "ABCD1234",
            "deviceLabel": "Bureau"
        }),
    );

    assert!(
        pending.held_the_caller_for < Duration::from_millis(800),
        "held the calling thread for {:?}",
        pending.held_the_caller_for
    );
    let settings = pending
        .answer(Duration::from_secs(5))
        .expect("pairing succeeds once the service is awake");
    assert_eq!(settings["deviceLabel"], "Bureau");
    assert_eq!(service.raw_hits("POST", "/api/v1/devices/claim"), 1);
    assert_eq!(harness.service_events(), ["starting", "ready"]);
    drop(harness.app);
}

#[test]
fn cancelling_from_the_interface_ends_the_wait_with_the_cancelled_code() {
    let _serial = serialized();
    // One attempt lasts 3 s: a cancel that were only noticed between
    // attempts would take that long to end the wait.
    let harness = harness(WakePolicy {
        attempt_timeout: Duration::from_millis(3_000),
        total_budget: Duration::from_millis(6_000),
        ..fast()
    });
    let service = FakeService::start(Wake::Held(Duration::from_secs(30)), portal_app);
    harness.pair_with(&service.base_url());

    let pending = harness.invoke(
        "list_sessions",
        serde_json::json!({"since": null, "limit": null}),
    );
    std::thread::sleep(Duration::from_millis(400));
    let cancelled_at = Instant::now();
    let cancel = harness.invoke("service_cancel", serde_json::json!({}));
    cancel.answer(Duration::from_secs(1)).unwrap();

    let error = pending
        .answer(Duration::from_secs(3))
        .expect_err("a cancelled wait is an error the page can translate");
    assert_eq!(error["code"], "cancelled");
    assert!(
        cancelled_at.elapsed() < Duration::from_millis(1_200),
        "the wait must end promptly once cancelled, not at the end of the attempt in flight"
    );
    assert_eq!(harness.service_events(), ["starting", "stopped"]);
    drop(harness.app);
}

#[test]
fn a_wait_that_runs_out_ends_with_the_start_timeout_code() {
    let _serial = serialized();
    let harness = harness(WakePolicy {
        total_budget: Duration::from_millis(1_000),
        ..fast()
    });
    let service = FakeService::start(Wake::Held(Duration::from_secs(30)), portal_app);
    harness.pair_with(&service.base_url());

    let error = harness
        .invoke(
            "list_sessions",
            serde_json::json!({"since": null, "limit": null}),
        )
        .answer(Duration::from_secs(5))
        .expect_err("the service never woke in time");

    assert_eq!(error["code"], "service_start_timeout");
    assert_eq!(harness.service_events(), ["starting", "stopped"]);
    drop(harness.app);
}

#[test]
fn an_unreachable_service_fails_at_once_with_the_network_code_and_no_starting_state() {
    let _serial = serialized();
    let harness = harness(fast());
    let closed = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port())
    };
    harness.pair_with(&closed);

    let started = Instant::now();
    let error = harness
        .invoke(
            "list_sessions",
            serde_json::json!({"since": null, "limit": null}),
        )
        .answer(Duration::from_secs(3))
        .expect_err("nothing listens there");

    assert_eq!(error["code"], "network_error");
    assert!(started.elapsed() < Duration::from_millis(1_500));
    assert!(harness.service_events().is_empty());
    drop(harness.app);
}

#[test]
fn the_startup_capability_refresh_waits_only_when_no_profile_is_cached() {
    let _serial = serialized();
    let harness = harness(fast());
    let service = FakeService::start(Wake::Held(Duration::from_millis(900)), portal_app);
    let _ = std::fs::remove_file(harness.config_dir.join("capabilities-cache.json"));
    harness.pair_with(&service.base_url());

    // No cache: nothing else to show, so it waits (and says so).
    let state = harness
        .invoke("capabilities", serde_json::json!({}))
        .answer(Duration::from_secs(5))
        .unwrap();
    assert_eq!(state["paired"], true);
    assert_eq!(state["capabilities"]["analysis.logs"], true);
    assert_eq!(harness.service_events(), ["starting", "ready"]);

    // With a cache and a service that has gone back to sleep: one short,
    // silent attempt, and the cached profile comes back.
    let asleep = FakeService::start(Wake::Held(Duration::from_secs(30)), portal_app);
    harness.pair_with(&asleep.base_url());
    harness.service_events.lock().unwrap().clear();
    let started = Instant::now();
    let state = harness
        .invoke("capabilities", serde_json::json!({}))
        .answer(Duration::from_secs(5))
        .unwrap();
    assert_eq!(
        state["capabilities"]["analysis.logs"], true,
        "the cached profile"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the background attempt is short: {:?}",
        started.elapsed()
    );
    assert!(
        harness.service_events().is_empty(),
        "a background refresh with a cached profile announces nothing"
    );
    drop(harness.app);
}

#[test]
fn the_upload_queue_run_reports_the_wake_and_its_progress_in_the_shape_the_page_reads() {
    let _serial = serialized();
    let harness = harness(fast());
    let model = Arc::new(UploadModel::default());
    let app_model = Arc::clone(&model);
    let service = FakeService::start(Wake::Held(Duration::from_millis(1_300)), move |request| {
        app_model
            .reply(request, 100)
            .unwrap_or_else(|| error_reply(404, "not_found", "no such route"))
    });
    harness.pair_with(&service.base_url());

    let source = unique_dir("source");
    std::fs::write(source.join("frame.tif"), vec![9_u8; 250]).unwrap();
    let queue_path = harness.config_dir.join("upload-queue.sqlite");
    let _ = std::fs::remove_file(&queue_path);
    let queue = UploadQueue::open(&queue_path).unwrap();
    queue
        .enqueue_from_dry_run(&build_dry_run_report(&scan_folder(&source).unwrap()))
        .unwrap();
    drop(queue);

    let pending = harness.invoke(
        "upload_all",
        serde_json::json!({"sessionId": null, "planId": null, "targetIds": []}),
    );
    assert!(
        pending.held_the_caller_for < Duration::from_millis(800),
        "held the calling thread for {:?}",
        pending.held_the_caller_for
    );
    let processed = pending
        .answer(Duration::from_secs(8))
        .expect("the queue drains once the service is awake");

    assert_eq!(processed, 1);
    assert_eq!(harness.service_events(), ["starting", "ready"]);
    assert_eq!(model.sessions_created(), 1);
    assert_eq!(model.captures_created(), 1);
    let upload_events = harness.upload_events.lock().unwrap().clone();
    assert!(
        upload_events
            .iter()
            .any(|event| event["kind"] == "Completed"
                && event["captureId"].as_str().is_some_and(|id| !id.is_empty())),
        "the page reads kind \"Completed\" and camelCase fields: {upload_events:?}"
    );
    let _ = std::fs::remove_dir_all(&source);
    drop(harness.app);
}

/// A queue of these files (250 bytes each: three chunks of the model's 100),
/// enqueued in `harness`'s config directory.
fn queue_of(harness: &Harness, names: &[&str]) -> (PathBuf, PathBuf) {
    let source = unique_dir("source");
    for (index, name) in names.iter().enumerate() {
        std::fs::write(source.join(name), vec![index as u8 + 3; 250]).unwrap();
    }
    let queue_path = harness.config_dir.join("upload-queue.sqlite");
    let _ = std::fs::remove_file(&queue_path);
    let queue = UploadQueue::open(&queue_path).unwrap();
    queue
        .enqueue_from_dry_run(&build_dry_run_report(&scan_folder(&source).unwrap()))
        .unwrap();
    (source, queue_path)
}

fn run_all(harness: &Harness) -> Result<serde_json::Value, serde_json::Value> {
    harness
        .invoke(
            "upload_all",
            serde_json::json!({"sessionId": null, "planId": null, "targetIds": []}),
        )
        .answer(Duration::from_secs(8))
}

/// The service refuses a file for good (an SVG document): the page reads a
/// `Refused` event with the code it looks up in its three languages, the core's
/// sentence in French as a fallback, and camelCase fields -- and the run goes
/// on with the next file.
#[test]
fn a_refused_file_reaches_the_page_as_the_event_it_reads_and_the_run_goes_on() {
    let _serial = serialized();
    let harness = harness(fast());
    let model = Arc::new(UploadModel::default());
    let app_model = Arc::clone(&model);
    let service = FakeService::start(Wake::Instant, move |request| {
        if request.method == "POST" && request.path == "/api/v1/uploads" {
            let name = request.json()["originalName"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            if name == "drawing.png" {
                return error_reply(415, "capture_content_refused", "The bytes are an SVG.");
            }
        }
        app_model
            .reply(request, 100)
            .unwrap_or_else(|| error_reply(404, "not_found", "no such route"))
    });
    harness.pair_with(&service.base_url());
    let (source, queue_path) = queue_of(&harness, &["drawing.png", "frame.tif"]);

    let processed = run_all(&harness).expect("the queue drains: a refusal is not a failure");

    assert_eq!(processed, 2);
    let events = harness.upload_events.lock().unwrap().clone();
    let refused = events
        .iter()
        .find(|event| event["kind"] == "Refused")
        .unwrap_or_else(|| panic!("the page reads kind \"Refused\": {events:?}"));
    assert_eq!(refused["code"], "capture_content_refused");
    assert!(refused["sha256"]
        .as_str()
        .is_some_and(|sha| sha.len() == 64));
    assert!(
        refused["message"]
            .as_str()
            .is_some_and(|message| message.contains("SVG or HTML") && !message.contains("SVG.")),
        "{refused}"
    );
    assert!(events.iter().any(|event| event["kind"] == "Completed"));
    assert!(!events.iter().any(|event| event["kind"] == "Failed"));
    let queue = UploadQueue::open(&queue_path).unwrap();
    let rows = queue.list().unwrap();
    let refused_row = rows
        .iter()
        .find(|row| row.source_path.ends_with("drawing.png"))
        .unwrap();
    assert_eq!(
        serde_json::to_value(refused_row).unwrap()["state"],
        "refused"
    );
    assert_eq!(
        serde_json::to_value(refused_row).unwrap()["lastErrorCode"],
        "capture_content_refused"
    );
    assert_eq!(model.captures_created(), 1);
    let _ = std::fs::remove_dir_all(&source);
    drop(harness.app);
}

/// The space's storage is full: the page reads `Held` then a `Stopped` for
/// `held`, the file waits with its session, and a run started again after room
/// was made goes on and finishes it.
#[test]
fn a_full_storage_reaches_the_page_as_held_and_the_next_run_finishes_the_file() {
    let _serial = serialized();
    let harness = harness(fast());
    let model = Arc::new(UploadModel::default());
    let app_model = Arc::clone(&model);
    let full = Arc::new(AtomicBool::new(true));
    let is_full = Arc::clone(&full);
    let service = FakeService::start(Wake::Instant, move |request| {
        let later_chunk = request.method == "PUT"
            && request
                .header("upload-offset")
                .is_some_and(|offset| offset != "0");
        if later_chunk && is_full.load(Ordering::SeqCst) {
            return Reply::Json(
                409,
                r#"{"error":{"code":"storage_quota_reached","message":"Full."},"storage":{"usedBytes":100,"quotaBytes":100,"maxFileBytes":104857600}}"#
                    .to_string(),
            );
        }
        app_model
            .reply(request, 100)
            .unwrap_or_else(|| error_reply(404, "not_found", "no such route"))
    });
    harness.pair_with(&service.base_url());
    let (source, queue_path) = queue_of(&harness, &["frame.tif"]);

    let processed = run_all(&harness).expect("a hold is not an error of the command");

    assert_eq!(processed, 0);
    let events = harness.upload_events.lock().unwrap().clone();
    let held = events
        .iter()
        .find(|event| event["kind"] == "Held")
        .unwrap_or_else(|| panic!("the page reads kind \"Held\": {events:?}"));
    assert_eq!(held["code"], "storage_quota_reached");
    let stopped = events.last().unwrap();
    assert_eq!(stopped["kind"], "Stopped");
    assert_eq!(stopped["reason"], "held");
    let row = UploadQueue::open(&queue_path).unwrap().list().unwrap();
    let row = serde_json::to_value(&row[0]).unwrap();
    assert_eq!(row["state"], "held");
    assert_eq!(row["lastErrorCode"], "storage_quota_reached");
    assert_eq!(row["offset"], 100, "the chunk that fit is kept");

    // Room made, the person starts again.
    full.store(false, Ordering::SeqCst);
    harness.pair_with(&service.base_url());
    let processed = run_all(&harness).expect("the queue drains");
    assert_eq!(processed, 1);
    let row = UploadQueue::open(&queue_path).unwrap().list().unwrap();
    assert_eq!(serde_json::to_value(&row[0]).unwrap()["state"], "completed");
    assert_eq!(model.captures_created(), 1);
    let _ = std::fs::remove_dir_all(&source);
    drop(harness.app);
}

/// A night folder holding one PHD2 guide log in `logs/`.
fn night_with_a_guide_log(label: &str) -> std::path::PathBuf {
    let folder = unique_dir(label);
    std::fs::create_dir_all(folder.join("logs")).unwrap();
    std::fs::write(
        folder.join("logs/PHD2_GuideLog_2026-10-07_210000.txt"),
        include_str!("../../../crates/astro-sync-core/tests/fixtures/phd2_guide_log_short.txt"),
    )
    .unwrap();
    folder
}

/// A report over the service's ceiling (`413 payload_too_large`): the page
/// reads the contract's own code, the request is sent once and never again.
#[test]
fn an_analysis_over_the_ceiling_reaches_the_page_with_its_own_code_and_is_sent_once() {
    let _serial = serialized();
    let harness = harness(fast());
    let service = FakeService::start(Wake::Instant, |request| {
        if request.method == "POST" && request.path.ends_with("/analyses") {
            return error_reply(413, "payload_too_large", "The body is too large.");
        }
        portal_app(request)
    });
    harness.pair_with(&service.base_url());

    // A night whose logs give a report (« Analyser et envoyer le rapport »).
    let folder = night_with_a_guide_log("over-the-ceiling");
    let error = harness
        .invoke(
            "night_send_logs",
            serde_json::json!({"sessionId": "session-1", "folder": folder.to_string_lossy()}),
        )
        .answer(Duration::from_secs(5))
        .expect_err("the service refuses the report");

    assert_eq!(error["code"], "payload_too_large");
    assert_eq!(
        service.app_hits("POST", "/api/v1/sessions/session-1/analyses"),
        1,
        "sent once, never again behind the person's back"
    );
    drop(harness.app);
}

/// A report holding nothing never leaves the computer; the page reads
/// `analysis_empty` and the analysis in place is kept.
#[test]
fn an_empty_analysis_is_refused_before_anything_is_sent() {
    let _serial = serialized();
    let harness = harness(fast());
    let service = FakeService::start(Wake::Instant, portal_app);
    harness.pair_with(&service.base_url());

    // A night folder without any log or frame: nothing to report.
    let folder = unique_dir("empty-night");
    std::fs::create_dir_all(folder.join("logs")).unwrap();
    let error = harness
        .invoke(
            "night_send_logs",
            serde_json::json!({"sessionId": "session-1", "folder": folder.to_string_lossy()}),
        )
        .answer(Duration::from_secs(5))
        .expect_err("an empty report is refused here");

    assert_eq!(error["code"], "analysis_empty");
    assert_eq!(
        service.app_hits("POST", "/api/v1/sessions/session-1/analyses"),
        0
    );
    drop(harness.app);
}

/// The wake is only ever visible to the page if the command that waits is
/// not the one holding the request thread: every command that talks to the
/// service must be an `async fn` that hands its blocking work to
/// `run_blocking`. (The behavior above proves it for the commands it
/// drives; this keeps the rest from drifting back.)
#[test]
fn every_command_that_talks_to_the_service_runs_off_the_request_thread() {
    let source = include_str!("commands.rs");
    for name in [
        "device_pair",
        "capabilities",
        "list_sessions",
        "upload_next",
        "upload_all",
        "download_reference_stars",
        "service_announcement",
        "space_request_code",
        "space_verify_code",
        "space_refresh",
        "space_rename",
        "space_detach",
    ] {
        let declaration = format!("pub async fn {name}<");
        let start = source
            .find(&declaration)
            .unwrap_or_else(|| panic!("`{name}` must be `pub async fn {name}<R: Runtime>`"));
        let body = &source[start..];
        let body = &body[..body.find("\n}\n").expect("end of the function")];
        assert!(
            body.contains("run_blocking("),
            "`{name}` must do its blocking work inside `run_blocking`"
        );
    }
}

//! What Astro Sync does when the service is asleep.
//!
//! A service that sleeps when nobody uses it answers the *first* request
//! late -- it starts and the gateway in front of it keeps the connection
//! open meanwhile -- then everything answers in under a second. The
//! applications must therefore be patient. These tests run the real HTTP
//! transports against [`fake_service`], which imitates that behavior with a
//! wake delay of a second or so instead of tens of seconds, and check three
//! things: a request that gets no answer *because the service is waking* is
//! retried until it does (within a budget, never forever); a service that
//! is genuinely unreachable still fails at once; and no write is ever sent
//! twice unless the contract makes a second copy harmless -- counted on the
//! requests the *application* received.

#[path = "support/fake_service.rs"]
mod fake_service;

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use astro_sync_core::dry_run::build_dry_run_report;
use astro_sync_core::gui_bridge::{GuiBridgeError, HttpPortalGuiTransport, PortalGuiTransport};
use astro_sync_core::scanner::scan_folder;
use astro_sync_core::service_wait::{
    ServiceEvent, ServiceWaitError, WaitControl, WaitHooks, WakePolicy,
};
use astro_sync_core::upload_queue::UploadQueue;
use astro_sync_core::uploader::{HttpUploadTransport, UploadError, UploadTarget, upload_entry};
use fake_service::{
    FakeService, Recorded, Reply, UploadModel, Wake, capabilities_reply, error_reply,
    identity_reply, sessions_reply,
};

/* --------------------------------- helpers -------------------------------- */

/// The same policy shape as the default one, on a scale of hundreds of
/// milliseconds so no test waits for a real start.
fn fast() -> WakePolicy {
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

type Events = Arc<Mutex<Vec<ServiceEvent>>>;

fn recording_hooks() -> (WaitHooks, Events) {
    let events: Events = Arc::default();
    let sink = Arc::clone(&events);
    let hooks = WaitHooks::new(WaitControl::default().token(), move |event| {
        sink.lock().unwrap().push(event);
    });
    (hooks, events)
}

fn seen(events: &Events) -> Vec<ServiceEvent> {
    events.lock().unwrap().clone()
}

fn portal_app(request: &Recorded) -> Reply {
    let path = request.path.as_str();
    match (request.method.as_str(), path) {
        ("GET", "/api/v1/service") => identity_reply(),
        ("GET", "/api/v1/capabilities") => capabilities_reply(),
        ("GET", path) if path.starts_with("/api/v1/sessions") => sessions_reply(),
        ("POST", "/api/v1/devices/claim") => {
            Reply::Json(201, r#"{"deviceToken":"device-token-test"}"#.to_string())
        }
        ("POST", path) if path.starts_with("/api/v1/sessions/") && path.ends_with("/analyses") => {
            Reply::Json(
                201,
                r#"{"analysis":{"id":"analysis-1","sessionId":"session-1","reportVersion":2,"createdAt":"2026-09-29T20:00:00Z","originDeviceId":null,"deviceName":null},"replaced":false,"steps":{"proposed":1,"keptEdited":0}}"#
                    .to_string(),
            )
        }
        _ => error_reply(404, "not_found", "no such route"),
    }
}

/// A port nothing listens on: the connection is refused at once.
fn closed_port_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        // Tests run in parallel and the clock is coarse: a counter keeps two
        // directories from ever sharing a name (one test's cleanup must not
        // remove another test's queue database).
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let unique = format!(
            "{}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        );
        let path = std::env::temp_dir().join(format!(
            "astro-sync-wake-{label}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/* --------------------- reads: patient while the service wakes ------------- */

#[test]
fn a_read_waits_for_a_sleeping_service_and_gets_its_answer() {
    let service = FakeService::start(Wake::Held(Duration::from_millis(1_300)), portal_app);
    let (hooks, events) = recording_hooks();
    let transport = HttpPortalGuiTransport::with_options(fast(), hooks).unwrap();

    let started = Instant::now();
    let page = transport
        .list_sessions(&service.base_url(), "token", None, Some(20))
        .expect("a wake shorter than the budget must not fail the request");

    assert_eq!(page.sessions.len(), 1);
    assert!(
        started.elapsed() >= Duration::from_millis(1_200),
        "the answer cannot come before the service is ready"
    );
    assert_eq!(seen(&events), [ServiceEvent::Starting, ServiceEvent::Ready]);
    assert_eq!(
        service.app_hits("GET", "/api/v1/sessions"),
        1,
        "requests abandoned while held never reach the application"
    );
    assert!(
        service.raw_hits("GET", "/api/v1/sessions") >= 2,
        "the first attempt (500 ms) is shorter than the wake and must have been retried"
    );
}

#[test]
fn every_request_to_the_portal_states_what_the_application_is() {
    let service = FakeService::start(Wake::Instant, portal_app);
    let (hooks, _events) = recording_hooks();
    let transport = HttpPortalGuiTransport::with_options(fast(), hooks).unwrap();

    transport
        .list_sessions(&service.base_url(), "token", None, None)
        .unwrap();

    let requests = service.requests();
    assert!(!requests.is_empty());
    for request in &requests {
        let value = request
            .header("x-planastro-client")
            .unwrap_or_else(|| panic!("no identity on {} {}", request.method, request.path));
        assert!(value.starts_with("astro-sync-"), "{value}");
        assert!(value.contains('/') && value.ends_with(')'), "{value}");
    }
}

#[test]
fn a_gateway_503_while_the_service_starts_is_retried_until_it_answers() {
    let service = FakeService::start(
        Wake::GatewayUnavailable(Duration::from_millis(700)),
        portal_app,
    );
    let (hooks, events) = recording_hooks();
    let transport = HttpPortalGuiTransport::with_options(fast(), hooks).unwrap();

    let capabilities = transport
        .fetch_capabilities(&service.base_url(), "token")
        .expect("503 from the gateway during the start is not an answer");

    assert!(capabilities.analysis_logs);
    assert_eq!(seen(&events), [ServiceEvent::Starting, ServiceEvent::Ready]);
    assert_eq!(service.app_hits("GET", "/api/v1/capabilities"), 1);
    assert!(service.raw_hits("GET", "/api/v1/capabilities") >= 2);
}

#[test]
fn a_503_carrying_the_applications_error_envelope_is_an_answer_not_a_wake() {
    // `authentication_unavailable`, `session_analysis_unavailable`, ...: the
    // application itself says 503 and says why. Retrying would only delay a
    // real, permanent refusal.
    let service = FakeService::start(Wake::Instant, |_| {
        error_reply(
            503,
            "authentication_unavailable",
            "Private API is unavailable.",
        )
    });
    let (hooks, events) = recording_hooks();
    let transport = HttpPortalGuiTransport::with_options(fast(), hooks).unwrap();

    let started = Instant::now();
    let error = transport
        .list_sessions(&service.base_url(), "token", None, None)
        .unwrap_err();

    assert!(
        matches!(&error, GuiBridgeError::Refused { code, .. } if code == "authentication_unavailable"),
        "got {error:?}"
    );
    assert!(started.elapsed() < Duration::from_millis(1_500));
    assert_eq!(service.raw_hits("GET", "/api/v1/sessions"), 1);
    assert!(seen(&events).is_empty());
}

/* -------------------- unreachable: still immediate, as today -------------- */

#[test]
fn a_refused_connection_fails_at_once_with_no_starting_state() {
    let (hooks, events) = recording_hooks();
    let transport = HttpPortalGuiTransport::with_options(fast(), hooks).unwrap();

    let started = Instant::now();
    let error = transport
        .list_sessions(&closed_port_url(), "token", None, None)
        .unwrap_err();

    assert!(
        matches!(error, GuiBridgeError::Transport(_)),
        "got {error:?}"
    );
    assert!(
        started.elapsed() < Duration::from_millis(1_500),
        "an unreachable service must not be waited for (a wait would run to the {:?} budget)",
        fast().total_budget
    );
    assert!(seen(&events).is_empty());
}

#[test]
fn a_name_that_does_not_exist_fails_at_once_with_no_starting_state() {
    let (hooks, events) = recording_hooks();
    let transport = HttpPortalGuiTransport::with_options(fast(), hooks).unwrap();

    let started = Instant::now();
    let error = transport
        .list_sessions(
            "http://no-such-host.astro-sync-test.invalid",
            "token",
            None,
            None,
        )
        .unwrap_err();

    assert!(
        matches!(error, GuiBridgeError::Transport(_)),
        "got {error:?}"
    );
    assert!(started.elapsed() < Duration::from_millis(1_500));
    assert!(seen(&events).is_empty());
}

#[test]
fn a_tls_failure_fails_at_once_with_no_starting_state() {
    // A server that answers plain HTTP on a port the client reaches over
    // TLS: the handshake cannot succeed.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        use std::io::Write;
        while let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
        }
    });
    let (hooks, events) = recording_hooks();
    let transport = HttpPortalGuiTransport::with_options(fast(), hooks).unwrap();

    let started = Instant::now();
    let error = transport
        .list_sessions(&format!("https://127.0.0.1:{port}"), "token", None, None)
        .unwrap_err();

    assert!(
        matches!(error, GuiBridgeError::Transport(_)),
        "got {error:?}"
    );
    assert!(started.elapsed() < Duration::from_millis(1_500));
    assert!(seen(&events).is_empty());
}

/* ------------------------- the wait is bounded and cancellable ------------ */

#[test]
fn the_wait_ends_after_the_total_budget_and_says_so() {
    let service = FakeService::start(Wake::Held(Duration::from_secs(30)), portal_app);
    let (hooks, events) = recording_hooks();
    let policy = WakePolicy {
        total_budget: Duration::from_millis(1_200),
        ..fast()
    };
    let transport = HttpPortalGuiTransport::with_options(policy, hooks).unwrap();

    let started = Instant::now();
    let error = transport
        .list_sessions(&service.base_url(), "token", None, None)
        .unwrap_err();

    assert!(
        matches!(
            error,
            GuiBridgeError::Wait(ServiceWaitError::StartTimeout { .. })
        ),
        "got {error:?}"
    );
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(1_100),
        "gave up early: {waited:?}"
    );
    assert!(
        waited < Duration::from_millis(2_500),
        "waited too long: {waited:?}"
    );
    assert_eq!(
        seen(&events),
        [ServiceEvent::Starting, ServiceEvent::Stopped]
    );
}

#[test]
fn cancelling_ends_the_wait_promptly() {
    let service = FakeService::start(Wake::Held(Duration::from_secs(30)), portal_app);
    let control = WaitControl::default();
    let events: Events = Arc::default();
    let sink = Arc::clone(&events);
    let hooks = WaitHooks::new(control.token(), move |event| {
        sink.lock().unwrap().push(event)
    });
    // One attempt lasts 3 s: a cancel that were only noticed between
    // attempts would take that long to end the wait.
    let policy = WakePolicy {
        attempt_timeout: Duration::from_millis(3_000),
        total_budget: Duration::from_millis(6_000),
        ..fast()
    };
    let transport = HttpPortalGuiTransport::with_options(policy, hooks).unwrap();

    let canceller = {
        let control = control.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            control.cancel_all();
        })
    };
    let started = Instant::now();
    let error = transport
        .list_sessions(&service.base_url(), "token", None, None)
        .unwrap_err();
    canceller.join().unwrap();

    assert!(
        matches!(error, GuiBridgeError::Wait(ServiceWaitError::Cancelled)),
        "got {error:?}"
    );
    assert!(
        started.elapsed() < Duration::from_millis(1_200),
        "cancelling must not wait for the attempt in flight"
    );
    assert_eq!(
        seen(&events),
        [ServiceEvent::Starting, ServiceEvent::Stopped]
    );
}

#[test]
fn a_cancel_that_happened_before_the_request_does_not_cancel_it() {
    let service = FakeService::start(Wake::Instant, portal_app);
    let control = WaitControl::default();
    control.cancel_all();
    let hooks = WaitHooks::new(control.token(), |_| {});
    let transport = HttpPortalGuiTransport::with_options(fast(), hooks).unwrap();

    transport
        .list_sessions(&service.base_url(), "token", None, None)
        .expect("a token is only cancelled by a cancel that comes after it");
}

/* ----------------------- writes that must be sent only once --------------- */

#[test]
fn a_pairing_claim_wakes_the_service_first_then_is_sent_exactly_once() {
    let service = FakeService::start(Wake::Held(Duration::from_millis(1_300)), portal_app);
    let (hooks, events) = recording_hooks();
    let transport = HttpPortalGuiTransport::with_options(fast(), hooks).unwrap();

    let token = transport
        .claim_device(&service.base_url(), "ABCD1234", "Bureau")
        .expect("the claim must go through once the service is awake");

    assert_eq!(token, "device-token-test");
    assert_eq!(
        service.raw_hits("POST", "/api/v1/devices/claim"),
        1,
        "the single-use code must be presented exactly once, never while asleep"
    );
    let requests = service.requests();
    let first_app_request = requests.iter().find(|request| request.reached_app).unwrap();
    assert_eq!(
        (
            first_app_request.method.as_str(),
            first_app_request.path.as_str()
        ),
        ("GET", "/api/v1/service"),
        "the service is woken by a read with no effect, before the write"
    );
    assert_eq!(seen(&events), [ServiceEvent::Starting, ServiceEvent::Ready]);
}

#[test]
fn a_pairing_claim_is_never_sent_when_the_service_does_not_wake_in_time() {
    let service = FakeService::start(Wake::Held(Duration::from_secs(30)), portal_app);
    let (hooks, _events) = recording_hooks();
    let policy = WakePolicy {
        total_budget: Duration::from_millis(1_000),
        ..fast()
    };
    let transport = HttpPortalGuiTransport::with_options(policy, hooks).unwrap();

    let error = transport
        .claim_device(&service.base_url(), "ABCD1234", "Bureau")
        .unwrap_err();

    assert!(
        matches!(
            error,
            GuiBridgeError::Wait(ServiceWaitError::StartTimeout { .. })
        ),
        "got {error:?}"
    );
    assert_eq!(
        service.raw_hits("POST", "/api/v1/devices/claim"),
        0,
        "nothing that creates or consumes is sent to a service that never answered"
    );
}

#[test]
fn an_analysis_upload_is_sent_exactly_once_even_when_the_service_was_asleep() {
    let service = FakeService::start(Wake::Held(Duration::from_millis(1_300)), portal_app);
    let (hooks, _events) = recording_hooks();
    let transport = HttpPortalGuiTransport::with_options(fast(), hooks).unwrap();

    let uploaded = transport
        .upload_analysis(
            &service.base_url(),
            "token",
            "session-1",
            &serde_json::json!({"reportVersion": 2}),
        )
        .unwrap();

    assert_eq!(uploaded.analysis.report_version, 2);
    assert_eq!(
        service.raw_hits("POST", "/api/v1/sessions/session-1/analyses"),
        1,
        "every POST replaces the night's analysis: there is exactly one"
    );
    let requests = service.requests();
    let first_app_request = requests.iter().find(|request| request.reached_app).unwrap();
    assert_eq!(
        (
            first_app_request.method.as_str(),
            first_app_request.path.as_str()
        ),
        ("GET", "/api/v1/service"),
        "the service is woken by a read with no effect before the version is created"
    );
}

#[test]
fn a_write_answered_by_a_gateway_503_is_not_retried_and_is_reported_unconfirmed() {
    let service = FakeService::start(Wake::Instant, |request| {
        if request.method == "POST" {
            Reply::Text(
                503,
                "upstream connect error or disconnect/reset before headers".into(),
            )
        } else {
            portal_app(request)
        }
    });
    let (hooks, _events) = recording_hooks();
    let transport = HttpPortalGuiTransport::with_options(fast(), hooks).unwrap();

    let error = transport
        .claim_device(&service.base_url(), "ABCD1234", "Bureau")
        .unwrap_err();

    assert!(
        matches!(error, GuiBridgeError::Wait(ServiceWaitError::NotConfirmed)),
        "got {error:?}"
    );
    assert_eq!(service.raw_hits("POST", "/api/v1/devices/claim"), 1);
}

#[test]
fn a_write_that_gets_no_answer_is_not_retried_and_is_reported_unconfirmed() {
    let service = FakeService::start(Wake::Instant, |request| {
        if request.method == "POST" {
            Reply::Hang
        } else {
            portal_app(request)
        }
    });
    let (hooks, _events) = recording_hooks();
    let policy = WakePolicy {
        chunk_timeout: Duration::from_millis(400),
        ..fast()
    };
    let transport = HttpPortalGuiTransport::with_options(policy, hooks).unwrap();

    let started = Instant::now();
    let error = transport
        .upload_analysis(
            &service.base_url(),
            "token",
            "session-1",
            &serde_json::json!({"reportVersion": 1}),
        )
        .unwrap_err();

    assert!(
        matches!(error, GuiBridgeError::Wait(ServiceWaitError::NotConfirmed)),
        "got {error:?}"
    );
    assert!(started.elapsed() < Duration::from_millis(1_500));
    assert_eq!(
        service.raw_hits("POST", "/api/v1/sessions/session-1/analyses"),
        1
    );
}

#[test]
fn a_write_whose_answer_is_lost_is_not_resent_and_is_reported_unconfirmed() {
    // The service did the work and the connection dropped before the answer:
    // for a write with no idempotency, sending it again would create a
    // second version. It is reported as unconfirmed instead.
    let service = FakeService::start(Wake::Instant, |request| {
        if request.method == "POST" {
            Reply::DropAfterProcessing
        } else {
            portal_app(request)
        }
    });
    let transport = HttpPortalGuiTransport::with_options(fast(), WaitHooks::silent()).unwrap();

    let error = transport
        .upload_analysis(
            &service.base_url(),
            "token",
            "session-1",
            &serde_json::json!({"reportVersion": 1}),
        )
        .unwrap_err();

    assert!(
        matches!(error, GuiBridgeError::Wait(ServiceWaitError::NotConfirmed)),
        "got {error:?}"
    );
    assert_eq!(
        service.app_hits("POST", "/api/v1/sessions/session-1/analyses"),
        1,
        "the version was created once, and never again behind the person's back"
    );
}

#[test]
fn a_refusal_from_the_application_on_a_write_stays_a_refusal() {
    let service = FakeService::start(Wake::Instant, |request| {
        if request.method == "POST" {
            error_reply(403, "analysis_not_entitled", "Not entitled.")
        } else {
            portal_app(request)
        }
    });
    let transport = HttpPortalGuiTransport::with_options(fast(), WaitHooks::silent()).unwrap();

    let error = transport
        .upload_analysis(
            &service.base_url(),
            "token",
            "session-1",
            &serde_json::json!({}),
        )
        .unwrap_err();

    assert!(
        matches!(&error, GuiBridgeError::Refused { code, .. } if code == "analysis_not_entitled"),
        "got {error:?}"
    );
    assert_eq!(
        service.raw_hits("POST", "/api/v1/sessions/session-1/analyses"),
        1
    );
}

/* ---------------------- uploads: retried because they are idempotent ------ */

/// One 300-byte file in a folder, enqueued; the server model uses 100-byte
/// chunks so the transfer is three `PUT`s.
fn queued_file(
    bytes: &[u8],
) -> (
    TempDir,
    TempDir,
    UploadQueue,
    astro_sync_core::upload_queue::QueueEntry,
) {
    let source = TempDir::new("source");
    std::fs::write(source.path().join("frame.tif"), bytes).unwrap();
    let queue_dir = TempDir::new("queue");
    let queue = UploadQueue::open(&queue_dir.path().join("queue.sqlite")).unwrap();
    let report = build_dry_run_report(&scan_folder(source.path()).unwrap());
    queue.enqueue_from_dry_run(&report).unwrap();
    let entry = queue.next_pending().unwrap().unwrap();
    (source, queue_dir, queue, entry)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

fn file_bytes() -> Vec<u8> {
    (0..300_u32).map(|index| (index % 251) as u8).collect()
}

#[test]
fn a_full_upload_survives_a_sleeping_service_without_duplicating_anything() {
    let model = Arc::new(UploadModel::default());
    let app_model = Arc::clone(&model);
    let service = FakeService::start(Wake::Held(Duration::from_millis(1_300)), move |request| {
        app_model
            .reply(request, 100)
            .unwrap_or_else(|| error_reply(404, "not_found", "no such route"))
    });
    let (hooks, events) = recording_hooks();
    let transport = HttpUploadTransport::with_options(service.base_url(), fast(), hooks).unwrap();
    let bytes = file_bytes();
    let (_source, _queue_dir, queue, entry) = queued_file(&bytes);

    let outcome = upload_entry(
        &transport,
        &queue,
        "token",
        &entry,
        &UploadTarget::default(),
        |_, _| {},
    )
    .expect("the transfer must complete once the service is awake");

    assert!(!outcome.capture_id.is_empty());
    assert_eq!(model.sessions_created(), 1);
    assert_eq!(model.captures_created(), 1);
    assert_eq!(model.bytes_for(&sha256_hex(&bytes)).unwrap(), bytes);
    assert_eq!(
        service.app_hits("POST", "/api/v1/uploads/upload-1/finalize"),
        1
    );
    assert_eq!(service.app_hits("PUT", "/api/v1/uploads/upload-1"), 3);
    assert_eq!(
        service.app_hits("POST", "/api/v1/uploads") - service.app_hits("POST", "/api/v1/uploads/"),
        1,
        "exactly one create reached the application"
    );
    assert_eq!(
        seen(&events),
        [ServiceEvent::Starting, ServiceEvent::Ready],
        "the wake is announced once; the rest of the transfer is an awake service"
    );
}

#[test]
fn a_response_lost_after_the_work_was_done_is_replayed_without_a_second_effect() {
    // The worst case for a retry: the service did the work and the answer
    // never arrived. Each of the three idempotent calls loses its first
    // answer; the transfer must still end with one session, one capture and
    // exactly the file's bytes on the server.
    let model = Arc::new(UploadModel::default());
    let app_model = Arc::clone(&model);
    let lost: Arc<Mutex<Vec<String>>> = Arc::default();
    let lost_in_app = Arc::clone(&lost);
    let service = FakeService::start(Wake::Instant, move |request| {
        let reply = app_model
            .reply(request, 100)
            .unwrap_or_else(|| error_reply(404, "not_found", "no such route"));
        let route = match (request.method.as_str(), request.path.as_str()) {
            ("POST", "/api/v1/uploads") => "create",
            ("PUT", _) => "put",
            ("POST", path) if path.ends_with("/finalize") => "finalize",
            _ => return reply,
        };
        let mut lost = lost_in_app.lock().unwrap();
        if lost.iter().any(|done| done == route) {
            reply
        } else {
            lost.push(route.to_string());
            Reply::DropAfterProcessing
        }
    });
    let transport =
        HttpUploadTransport::with_options(service.base_url(), fast(), WaitHooks::silent()).unwrap();
    let bytes = file_bytes();
    let (_source, _queue_dir, queue, entry) = queued_file(&bytes);

    upload_entry(
        &transport,
        &queue,
        "token",
        &entry,
        &UploadTarget::default(),
        |_, _| {},
    )
    .expect("lost answers to idempotent calls must be recovered");

    assert_eq!(
        model.sessions_created(),
        1,
        "the replayed create is not a second session"
    );
    assert_eq!(
        model.captures_created(),
        1,
        "the replayed finalize is not a second capture"
    );
    assert_eq!(
        model.bytes_for(&sha256_hex(&bytes)).unwrap(),
        bytes,
        "the replayed chunk is not appended twice"
    );
    assert_eq!(
        service.app_hits("POST", "/api/v1/uploads/upload-1/finalize"),
        2,
        "finalize reached the application twice: one lost answer, one replay"
    );
}

#[test]
fn an_awake_service_that_answers_slowly_is_not_announced_as_starting() {
    // A large chunk over a slow link is silence too; only a service that
    // has not answered yet may be called "starting".
    let model = Arc::new(UploadModel::default());
    let app_model = Arc::clone(&model);
    let service = FakeService::start(Wake::Instant, move |request| {
        let reply = app_model
            .reply(request, 100)
            .unwrap_or_else(|| error_reply(404, "not_found", "no such route"));
        if request.method == "PUT" {
            Reply::After(Duration::from_millis(350), Box::new(reply))
        } else {
            reply
        }
    });
    let (hooks, events) = recording_hooks();
    let transport = HttpUploadTransport::with_options(service.base_url(), fast(), hooks).unwrap();
    let bytes = file_bytes();
    let (_source, _queue_dir, queue, entry) = queued_file(&bytes);

    upload_entry(
        &transport,
        &queue,
        "token",
        &entry,
        &UploadTarget::default(),
        |_, _| {},
    )
    .unwrap();

    assert!(seen(&events).is_empty(), "got {:?}", seen(&events));
}

#[test]
fn an_awake_service_that_stops_answering_a_chunk_fails_without_retrying_it() {
    let model = Arc::new(UploadModel::default());
    let app_model = Arc::clone(&model);
    let service = FakeService::start(Wake::Instant, move |request| {
        if request.method == "PUT" {
            return Reply::Hang;
        }
        app_model
            .reply(request, 100)
            .unwrap_or_else(|| error_reply(404, "not_found", "no such route"))
    });
    let policy = WakePolicy {
        chunk_timeout: Duration::from_millis(400),
        ..fast()
    };
    let (hooks, events) = recording_hooks();
    let transport = HttpUploadTransport::with_options(service.base_url(), policy, hooks).unwrap();
    let bytes = file_bytes();
    let (_source, _queue_dir, queue, entry) = queued_file(&bytes);

    let error = upload_entry(
        &transport,
        &queue,
        "token",
        &entry,
        &UploadTarget::default(),
        |_, _| {},
    )
    .unwrap_err();

    assert!(matches!(error, UploadError::Transport(_)), "got {error:?}");
    assert!(!error.is_terminal(), "the file stays resumable");
    assert_eq!(service.raw_hits("PUT", "/api/v1/uploads/upload-1"), 1);
    assert!(seen(&events).is_empty());
}

/* ------------------------------ what the front end reads ------------------ */

#[test]
fn service_events_serialize_as_the_front_end_reads_them() {
    assert_eq!(
        serde_json::to_string(&ServiceEvent::Starting).unwrap(),
        r#"{"kind":"starting"}"#
    );
    assert_eq!(
        serde_json::to_string(&ServiceEvent::Ready).unwrap(),
        r#"{"kind":"ready"}"#
    );
    assert_eq!(
        serde_json::to_string(&ServiceEvent::Stopped).unwrap(),
        r#"{"kind":"stopped"}"#
    );
}

#[test]
fn wait_errors_carry_stable_codes_and_plain_polite_english() {
    let timeout = ServiceWaitError::StartTimeout { waited_secs: 100 };
    assert_eq!(timeout.code(), "service_start_timeout");
    assert_eq!(ServiceWaitError::Cancelled.code(), "cancelled");
    assert_eq!(
        ServiceWaitError::NotConfirmed.code(),
        "service_write_unconfirmed"
    );
    for error in [
        timeout,
        ServiceWaitError::Cancelled,
        ServiceWaitError::NotConfirmed,
    ] {
        let text = error.to_string();
        assert!(
            !text.contains("reqwest") && !text.contains("hyper") && !text.contains("http://"),
            "no technical text: {text}"
        );
    }
}

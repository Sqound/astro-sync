//! Every refusal the service can answer to an upload, played against the real
//! HTTP transport and the real queue, on a fake service that models the
//! resumable protocol (`support/fake_service.rs`) and refuses on demand.
//!
//! What each test proves, on the requests the *application* received:
//!
//! - a refusal for good (`415` `capture_content_refused` -- at the creation,
//!   at the first chunk or at finalize --, `413` `file_too_large` and
//!   `payload_too_large`, the ceilings of a session, and any `4xx` nobody
//!   knows yet) reaches the service **once**, marks the file `refused` with
//!   its code and lets the run go on;
//! - a refusal that needs the person (`409` `storage_quota_reached` in the
//!   middle of a transfer or at the finish line, `403` `entitlement_read_only`)
//!   reaches the service once, **holds** the file with the bytes it already
//!   sent, stops the run -- and the next run, started after the person acted,
//!   goes on from where it stopped;
//! - a failure that will pass (`409` `upload_sessions_limit_reached`, a `5xx`)
//!   is waited out by the run, longer each time, without ever marking the file.
//!
//! The first of them is a failure that used to loop: a `409` at finalize was
//! read as "not all bytes arrived" whatever it said, so a full storage made
//! Astro Sync send the same chunk and the same finalize again, without end.

#[path = "support/fake_service.rs"]
mod fake_service;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use astro_sync_core::dry_run::build_dry_run_report;
use astro_sync_core::gui_bridge::{
    Backoff, DriveOptions, QueueDriveEvent, StopReason, drive_queue,
};
use astro_sync_core::scanner::scan_folder;
use astro_sync_core::service_wait::{WaitHooks, WakePolicy};
use astro_sync_core::upload_queue::{QueueState, UploadQueue};
use astro_sync_core::uploader::{HttpUploadTransport, UploadTarget};
use fake_service::{FakeService, Recorded, Reply, UploadModel, Wake, error_reply};

/* --------------------------------- helpers -------------------------------- */

fn fast() -> WakePolicy {
    WakePolicy {
        total_budget: Duration::from_millis(4_000),
        attempt_timeout: Duration::from_millis(1_500),
        write_timeout: Duration::from_millis(1_500),
        chunk_timeout: Duration::from_millis(2_500),
        retry_pause: Duration::from_millis(40),
        notice_after: Duration::from_millis(500),
        connect_timeout: Duration::from_millis(500),
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "astro-sync-refusals-{label}-{}-{nanos}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
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

fn bytes_of(seed: u8, length: u32) -> Vec<u8> {
    (0..length)
        .map(|index| seed.wrapping_add((index % 251) as u8))
        .collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

/// A night folder with these files (each 300 bytes: three chunks of the
/// model's 100), all queued.
struct Night {
    _source: TempDir,
    _queue_dir: TempDir,
    queue: UploadQueue,
    folder: PathBuf,
}

fn night(files: &[&str]) -> Night {
    let source = TempDir::new("source");
    for (index, name) in files.iter().enumerate() {
        std::fs::write(source.path().join(name), bytes_of(index as u8 * 7 + 1, 300)).unwrap();
    }
    let queue_dir = TempDir::new("queue");
    let queue = UploadQueue::open(&queue_dir.path().join("queue.sqlite")).unwrap();
    let report = build_dry_run_report(&scan_folder(source.path()).unwrap());
    queue.enqueue_from_dry_run(&report).unwrap();
    Night {
        folder: source.path().to_path_buf(),
        _source: source,
        _queue_dir: queue_dir,
        queue,
    }
}

/// The upload model of the contract, after `refuse` has had its say: it
/// returns the refusal to send instead, or `None` to let the model answer.
fn service_with(
    model: &Arc<UploadModel>,
    refuse: impl Fn(&Recorded) -> Option<Reply> + Send + Sync + 'static,
) -> FakeService {
    let model = Arc::clone(model);
    FakeService::start(Wake::Instant, move |request| {
        refuse(request).unwrap_or_else(|| {
            model
                .reply(request, 100)
                .unwrap_or_else(|| error_reply(404, "not_found", "no such route"))
        })
    })
}

fn is_create(request: &Recorded) -> bool {
    request.method == "POST" && request.path == "/api/v1/uploads"
}

fn is_chunk(request: &Recorded) -> bool {
    request.method == "PUT" && request.path.starts_with("/api/v1/uploads/")
}

fn is_finalize(request: &Recorded) -> bool {
    request.method == "POST" && request.path.ends_with("/finalize")
}

fn creations(service: &FakeService) -> usize {
    service.app_hits("POST", "/api/v1/uploads") - finalizations(service)
}

fn chunks(service: &FakeService) -> usize {
    service.app_hits("PUT", "/api/v1/uploads/")
}

fn finalizations(service: &FakeService) -> usize {
    service.app_hits("POST", "/api/v1/uploads/")
}

fn total_requests(service: &FakeService) -> usize {
    service.requests().len()
}

fn quick(max_replays: Option<u32>) -> DriveOptions {
    DriveOptions {
        backoff: Backoff::new(vec![
            Duration::from_millis(10),
            Duration::from_millis(20),
            Duration::from_millis(40),
        ]),
        max_replays,
        replay_when_unanswered: true,
    }
}

/// One run of the queue against `service`, with the events it emitted.
fn run(
    night: &Night,
    service: &FakeService,
    options: &DriveOptions,
) -> (usize, Vec<QueueDriveEvent>) {
    let transport =
        HttpUploadTransport::with_options(service.base_url(), fast(), WaitHooks::silent()).unwrap();
    let mut events = Vec::new();
    let processed = drive_queue(
        &transport,
        &night.queue,
        "token",
        &UploadTarget::default(),
        options,
        || true,
        |event| events.push(event),
    )
    .unwrap();
    (processed, events)
}

fn kinds(events: &[QueueDriveEvent]) -> Vec<&'static str> {
    events
        .iter()
        .filter_map(|event| match event {
            QueueDriveEvent::Started { .. } => Some("started"),
            QueueDriveEvent::Progress { .. } => None,
            QueueDriveEvent::Completed { .. } => Some("completed"),
            QueueDriveEvent::Failed { .. } => Some("failed"),
            QueueDriveEvent::Refused { .. } => Some("refused"),
            QueueDriveEvent::Held { .. } => Some("held"),
            QueueDriveEvent::Waiting { .. } => Some("waiting"),
            QueueDriveEvent::Stopped { .. } => Some("stopped"),
        })
        .collect()
}

fn states(night: &Night) -> Vec<QueueState> {
    night
        .queue
        .list()
        .unwrap()
        .iter()
        .map(|entry| entry.state)
        .collect()
}

fn refusal(status: u16, code: &str) -> Reply {
    error_reply(status, code, "The service's own English sentence.")
}

const QUOTA_BODY: &str = r#"{"error":{"code":"storage_quota_reached","message":"The space's storage is full."},"storage":{"usedBytes":100,"quotaBytes":100,"maxFileBytes":104857600}}"#;

/* ------------------------- refused for good, sent once -------------------- */

/// An SVG or HTML document is refused by its bytes, at the creation, at the
/// first chunk or at finalize: each time the file leaves the queue as
/// `refused` with the code, the service was asked once, the run went on, and
/// the file on the computer was not touched.
#[test]
fn a_document_refused_as_a_capture_is_set_aside_wherever_the_service_finds_it() {
    for at in ["the creation", "the first chunk", "finalize"] {
        let model = Arc::new(UploadModel::default());
        let service = service_with(&model, move |request| {
            let here = match at {
                "the creation" => is_create(request),
                "the first chunk" => is_chunk(request),
                _ => is_finalize(request),
            };
            here.then(|| refusal(415, "capture_content_refused"))
        });
        let night = night(&["drawing.png"]);
        let file = night.folder.join("drawing.png");
        let before = (
            std::fs::read(&file).unwrap(),
            std::fs::metadata(&file).unwrap().modified().unwrap(),
        );

        let (processed, events) = run(&night, &service, &quick(Some(3)));

        assert_eq!(processed, 1, "{at}: a refused file is a file finished with");
        assert_eq!(kinds(&events), ["started", "refused", "stopped"], "{at}");
        assert!(
            matches!(
                events.iter().find(|event| matches!(event, QueueDriveEvent::Refused { .. })),
                Some(QueueDriveEvent::Refused { code, message, .. })
                    if code == "capture_content_refused" && message.contains("SVG or HTML")
            ),
            "{at}: {events:?}"
        );
        let entry = &night.queue.list().unwrap()[0];
        assert_eq!(entry.state, QueueState::Refused, "{at}");
        assert_eq!(
            entry.last_error_code.as_deref(),
            Some("capture_content_refused"),
            "{at}"
        );
        // Asked once, where it was refused, and never after.
        let (created, chunked, finalized) = match at {
            "the creation" => (1, 0, 0),
            "the first chunk" => (1, 1, 0),
            _ => (1, 3, 1),
        };
        assert_eq!(creations(&service), created, "{at}");
        assert_eq!(chunks(&service), chunked, "{at}");
        assert_eq!(finalizations(&service), finalized, "{at}");
        // Started again, the queue has nothing to send and asks nothing.
        let requests = total_requests(&service);
        let (again, later) = run(&night, &service, &quick(Some(3)));
        assert_eq!(again, 0);
        assert_eq!(kinds(&later), ["stopped"]);
        assert_eq!(total_requests(&service), requests, "{at}: not asked again");
        // The file on the computer is exactly as it was.
        assert_eq!(std::fs::read(&file).unwrap(), before.0, "{at}");
        assert_eq!(
            std::fs::metadata(&file).unwrap().modified().unwrap(),
            before.1,
            "{at}"
        );
    }
}

/// The refusals for good the contract names, and one nobody knows yet: each
/// reaches the service once, marks the file `refused` with its own code (the
/// unknown one with `upload_refused`), and is said in French without its raw
/// code or the service's own English.
#[test]
fn a_refusal_for_good_reaches_the_service_once_and_never_again() {
    for (status, code, expected) in [
        (413, "file_too_large", "file_too_large"),
        (413, "payload_too_large", "payload_too_large"),
        (409, "session_limit_reached", "session_limit_reached"),
        (
            409,
            "session_event_limit_reached",
            "session_event_limit_reached",
        ),
        (400, "invalid_request", "upload_refused"),
        (404, "plan_not_found", "upload_refused"),
        (422, "a_code_of_a_newer_service", "upload_refused"),
    ] {
        let model = Arc::new(UploadModel::default());
        let service = service_with(&model, move |request| {
            is_create(request).then(|| refusal(status, code))
        });
        let night = night(&["frame.tif"]);

        let (processed, events) = run(&night, &service, &quick(Some(3)));

        assert_eq!(processed, 1, "{code}");
        assert_eq!(kinds(&events), ["started", "refused", "stopped"], "{code}");
        let entry = &night.queue.list().unwrap()[0];
        assert_eq!(entry.state, QueueState::Refused, "{code}");
        assert_eq!(entry.last_error_code.as_deref(), Some(expected), "{code}");
        assert_eq!(creations(&service), 1, "{code}: asked once");
        assert_eq!(chunks(&service), 0, "{code}: no byte was sent");
        let said = events
            .iter()
            .find_map(|event| match event {
                QueueDriveEvent::Refused { message, .. } => Some(message.clone()),
                _ => None,
            })
            .unwrap();
        assert!(
            !said.contains(code) && !said.contains("English sentence"),
            "{code}: {said}"
        );
    }
}

/// A `409` at finalize that is not `upload_incomplete` is a refusal like the
/// rest: read as "bytes are missing" it would send the same chunks and the
/// same finalize again, without end.
#[test]
fn a_conflict_at_finalize_that_is_not_upload_incomplete_is_not_asked_again() {
    let model = Arc::new(UploadModel::default());
    let service = service_with(&model, |request| {
        is_finalize(request).then(|| refusal(409, "a_conflict_nobody_knows"))
    });
    let night = night(&["frame.tif"]);

    let (_, events) = run(&night, &service, &quick(Some(3)));

    assert_eq!(kinds(&events), ["started", "refused", "stopped"]);
    assert_eq!(states(&night), [QueueState::Refused]);
    assert_eq!(chunks(&service), 3, "the three chunks, once");
    assert_eq!(finalizations(&service), 1);
}

/* --------------------------- held for the person, resumed ----------------- */

/// The space's storage fills up between two chunks: the file is held at the
/// offset already reached, the run stops on it (the other file is not even
/// tried: it would be refused the same way), and the next run -- started after
/// the person made room -- goes on from that offset and sends both, once.
#[test]
fn a_full_storage_in_the_middle_of_a_transfer_holds_the_file_and_the_next_run_resumes_it() {
    let model = Arc::new(UploadModel::default());
    let full = Arc::new(AtomicBool::new(true));
    let is_full = Arc::clone(&full);
    let service = service_with(&model, move |request| {
        // The first chunk fits; the second no longer does.
        let later_chunk = is_chunk(request) && request.header("upload-offset") != Some("0");
        (later_chunk && is_full.load(Ordering::SeqCst))
            .then(|| Reply::Json(409, QUOTA_BODY.to_string()))
    });
    let night = night(&["a-first.tif", "b-second.tif"]);
    let first = night.queue.next_pending().unwrap().unwrap();

    let (processed, events) = run(&night, &service, &quick(Some(3)));

    assert_eq!(processed, 0);
    assert_eq!(kinds(&events), ["started", "held", "stopped"], "{events:?}");
    assert!(matches!(
        events.last(),
        Some(QueueDriveEvent::Stopped {
            reason: StopReason::Held,
            ..
        })
    ));
    let held = night.queue.find_by_sha256(&first.sha256).unwrap().unwrap();
    assert_eq!(held.state, QueueState::Held);
    assert_eq!(
        held.last_error_code.as_deref(),
        Some("storage_quota_reached")
    );
    assert_eq!(held.offset, 100, "the chunk that fit is kept");
    assert!(held.upload_id.is_some(), "and so is the session");
    assert_eq!(chunks(&service), 2, "one accepted, one refused -- not more");
    assert_eq!(finalizations(&service), 0);
    assert_eq!(creations(&service), 1, "the other file was not tried");
    assert!(
        events.iter().any(|event| matches!(
            event,
            QueueDriveEvent::Held { message, .. } if message.contains("Hosted storage")
        )),
        "{events:?}"
    );
    assert_eq!(
        states(&night)
            .iter()
            .filter(|state| **state == QueueState::Queued)
            .count(),
        1
    );

    // Still full, the person starts again: one attempt, held again -- not a
    // loop -- and the requests stay few.
    let requests = total_requests(&service);
    let (_, again) = run(&night, &service, &quick(Some(3)));
    assert_eq!(kinds(&again), ["started", "held", "stopped"]);
    assert!(
        total_requests(&service) - requests <= 4,
        "a status, a chunk: {} new requests",
        total_requests(&service) - requests
    );

    // Room made: the run resumes, sends the rest of the first file and the
    // second one, each once.
    full.store(false, Ordering::SeqCst);
    let (processed, events) = run(&night, &service, &quick(Some(3)));
    assert_eq!(processed, 2, "{events:?}");
    assert_eq!(
        states(&night),
        [QueueState::Completed, QueueState::Completed]
    );
    assert_eq!(model.captures_created(), 2);
    let bytes = std::fs::read(night.folder.join("a-first.tif")).unwrap();
    assert_eq!(
        model.bytes_for(&sha256_hex(&bytes)).unwrap(),
        bytes,
        "no chunk was appended twice"
    );
}

const CAPTURE_LIMIT_BODY: &str = r#"{"error":{"code":"capture_limit_reached","message":"This space holds as many captures as its profile allows."},"storage":{"usedBytes":300,"quotaBytes":157286400,"maxFileBytes":26214400,"captureCount":3,"captureCountLimit":3}}"#;

/// The fourth capture of a free space: the service answers
/// `409 capture_limit_reached` at the creation. Nothing is wrong with the
/// file: it is held, the run stops on it (the other file would be refused the
/// same way), it is told in French without the raw code, and once the person
/// has made room -- a capture deleted, or an offer chosen -- the next run
/// sends both, once each.
#[test]
fn the_fourth_capture_of_a_free_space_is_held_for_the_person_and_resumed_once_there_is_room() {
    let model = Arc::new(UploadModel::default());
    let full = Arc::new(AtomicBool::new(true));
    let is_full = Arc::clone(&full);
    let service = service_with(&model, move |request| {
        (is_create(request) && is_full.load(Ordering::SeqCst))
            .then(|| Reply::Json(409, CAPTURE_LIMIT_BODY.to_string()))
    });
    let night = night(&["a-first.tif", "b-second.tif"]);
    let first = night.queue.next_pending().unwrap().unwrap();

    let (processed, events) = run(&night, &service, &quick(Some(3)));

    assert_eq!(processed, 0);
    assert_eq!(kinds(&events), ["started", "held", "stopped"], "{events:?}");
    assert!(matches!(
        events.last(),
        Some(QueueDriveEvent::Stopped {
            reason: StopReason::Held,
            ..
        })
    ));
    let held = night.queue.find_by_sha256(&first.sha256).unwrap().unwrap();
    assert_eq!(held.state, QueueState::Held);
    assert_eq!(
        held.last_error_code.as_deref(),
        Some("capture_limit_reached")
    );
    assert_eq!(creations(&service), 1, "the other file was not tried");
    assert_eq!(chunks(&service), 0, "no byte was sent");
    assert!(
        events.iter().any(|event| matches!(
            event,
            QueueDriveEvent::Held { code, message, .. }
                if code == "capture_limit_reached"
                    && message.contains("Not added")
                    && !message.contains("capture_limit_reached")
                    && !message.contains("English")
        )),
        "{events:?}"
    );
    // Neither of the two files left the queue, and the file on the computer
    // is where it was.
    assert!(night.folder.join("a-first.tif").exists());
    assert_eq!(
        states(&night)
            .iter()
            .filter(|state| **state == QueueState::Queued)
            .count(),
        1
    );

    // Still full, the person starts again: one attempt, held again.
    let requests = total_requests(&service);
    let (_, again) = run(&night, &service, &quick(Some(3)));
    assert_eq!(kinds(&again), ["started", "held", "stopped"]);
    assert!(total_requests(&service) - requests <= 2);

    // Room made: both files go, each once.
    full.store(false, Ordering::SeqCst);
    let (processed, events) = run(&night, &service, &quick(Some(3)));
    assert_eq!(processed, 2, "{events:?}");
    assert_eq!(
        states(&night),
        [QueueState::Completed, QueueState::Completed]
    );
    assert_eq!(model.captures_created(), 2);
}

/// A FITS header card: keyword, value, padded to 80 characters.
fn card(keyword: &str, value: &str) -> String {
    format!("{keyword:<8}= {value:>20}")
        .chars()
        .chain(std::iter::repeat(' '))
        .take(80)
        .collect()
}

/// What Astro Sync sends **with** a capture is only the keys the service
/// accepts (`allowedMetadataKeys`), nothing of the sky and never a site keyword of the header
/// (no sky metadata is asked for, and a place never leaves the machine in a
/// metadata part). A frame whose header holds `SITELAT`, `SITELONG`, `OBSGEO-X`, `LAT-OBS`
/// and `LONG-OBS` is sent whole -- it is the capture -- but its creation request names none
/// of them and carries no coordinate.
#[test]
fn the_metadata_of_a_capture_is_only_the_keys_the_service_knows_and_never_a_site_keyword() {
    let model = Arc::new(UploadModel::default());
    let service = service_with(&model, |_| None);
    let source = TempDir::new("site-keywords");
    let mut header = String::new();
    for (keyword, value) in [
        ("SIMPLE", "T"),
        ("BITPIX", "16"),
        ("NAXIS", "0"),
        ("IMAGETYP", "'Light'"),
        ("SITELAT", "45.123456"),
        ("SITELONG", "-30.654321"),
        ("OBSGEO-X", "4012345.678"),
        ("LAT-OBS", "45.123456"),
        ("LONG-OBS", "-30.654321"),
    ] {
        header.push_str(&card(keyword, value));
    }
    header.push_str(&format!("{:<80}", "END"));
    let mut bytes = header.into_bytes();
    bytes.resize(2880, b' ');
    std::fs::write(source.path().join("M31_Light_300s.tif"), &bytes).unwrap();
    let queue_dir = TempDir::new("queue");
    let queue = UploadQueue::open(&queue_dir.path().join("queue.sqlite")).unwrap();
    let report = build_dry_run_report(&scan_folder(source.path()).unwrap());
    queue.enqueue_from_dry_run(&report).unwrap();
    let night = Night {
        folder: source.path().to_path_buf(),
        _source: source,
        _queue_dir: queue_dir,
        queue,
    };

    let (processed, events) = run(&night, &service, &quick(Some(3)));
    assert_eq!(processed, 1, "{events:?}");

    let created = service
        .requests()
        .into_iter()
        .find(is_create)
        .expect("the creation request");
    let sent = created.json();
    let allowed = [
        "sessionId",
        "planId",
        "targetIds",
        "originalName",
        "sha256",
        "byteSize",
        "contentType",
    ];
    for key in sent.as_object().expect("a JSON object").keys() {
        assert!(allowed.contains(&key.as_str()), "{key} is not a known key");
    }
    let text = String::from_utf8_lossy(&created.body).to_string();
    for forbidden in [
        "SITELAT", "SITELONG", "OBSGEO", "LAT-OBS", "LONG-OBS", "45.123", "30.654", "4012345",
    ] {
        assert!(!text.contains(forbidden), "{forbidden} left in {text}");
    }
}

/// A failure that used to loop. The storage fills while the last chunk is on
/// its way: finalize answers `409 storage_quota_reached`. It used to be read as
/// "not all bytes arrived", so Astro Sync sent the same chunk and the same
/// finalize again, without end. Now the file is held after one finalize; a run
/// started again asks once more, and once room is made the file is finished --
/// without a single chunk resent.
#[test]
fn a_full_storage_at_the_finish_line_is_not_asked_again_and_again() {
    let model = Arc::new(UploadModel::default());
    let full = Arc::new(AtomicBool::new(true));
    let is_full = Arc::clone(&full);
    let service = service_with(&model, move |request| {
        (is_finalize(request) && is_full.load(Ordering::SeqCst))
            .then(|| Reply::Json(409, QUOTA_BODY.to_string()))
    });
    let night = night(&["frame.tif"]);

    let (_, events) = run(&night, &service, &quick(Some(3)));

    assert_eq!(kinds(&events), ["started", "held", "stopped"], "{events:?}");
    assert_eq!(states(&night), [QueueState::Held]);
    assert_eq!(finalizations(&service), 1, "one finalize, not a loop");
    assert_eq!(chunks(&service), 3, "each chunk once");
    assert!(
        total_requests(&service) <= 6,
        "create, three chunks, finalize: {} requests",
        total_requests(&service)
    );

    // Started again while the storage is still full: one more finalize, and
    // no chunk (the service already holds all the bytes).
    let (_, again) = run(&night, &service, &quick(Some(3)));
    assert_eq!(kinds(&again), ["started", "held", "stopped"]);
    assert_eq!(finalizations(&service), 2);
    assert_eq!(chunks(&service), 3, "not a byte resent");

    full.store(false, Ordering::SeqCst);
    let (processed, _) = run(&night, &service, &quick(Some(3)));
    assert_eq!(processed, 1);
    assert_eq!(states(&night), [QueueState::Completed]);
    assert_eq!(finalizations(&service), 3);
    assert_eq!(chunks(&service), 3, "still not a byte resent");
    assert_eq!(model.captures_created(), 1);
}

/// A subscription that ended: `403 entitlement_read_only` at the creation.
/// The file is held, not failed, and goes when the subscription is back.
#[test]
fn a_subscription_that_ended_holds_the_file_until_it_is_back() {
    let model = Arc::new(UploadModel::default());
    let ended = Arc::new(AtomicBool::new(true));
    let is_ended = Arc::clone(&ended);
    let service = service_with(&model, move |request| {
        (is_create(request) && is_ended.load(Ordering::SeqCst))
            .then(|| refusal(403, "entitlement_read_only"))
    });
    let night = night(&["frame.tif"]);

    let (_, events) = run(&night, &service, &quick(Some(3)));
    assert_eq!(kinds(&events), ["started", "held", "stopped"]);
    assert_eq!(states(&night), [QueueState::Held]);
    assert_eq!(
        night.queue.list().unwrap()[0].last_error_code.as_deref(),
        Some("entitlement_read_only")
    );
    assert_eq!(creations(&service), 1);

    ended.store(false, Ordering::SeqCst);
    let (processed, _) = run(&night, &service, &quick(Some(3)));
    assert_eq!(processed, 1);
    assert_eq!(states(&night), [QueueState::Completed]);
}

/* --------------------------- waited out, never marked --------------------- */

/// Too many uploads are open in the space: the run says it waits, waits
/// longer each time, tries the same file again by itself and finishes it --
/// the file is never marked, at any moment.
#[test]
fn too_many_open_uploads_is_waited_out_by_the_run_without_ever_marking_the_file() {
    let model = Arc::new(UploadModel::default());
    let refused = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&refused);
    let service = service_with(&model, move |request| {
        (is_create(request) && counted.fetch_add(1, Ordering::SeqCst) < 3)
            .then(|| refusal(409, "upload_sessions_limit_reached"))
    });
    let night = night(&["frame.tif"]);
    let transport =
        HttpUploadTransport::with_options(service.base_url(), fast(), WaitHooks::silent()).unwrap();
    let mut events = Vec::new();
    let mut marked_while_waiting = Vec::new();
    let started = Instant::now();

    let processed = drive_queue(
        &transport,
        &night.queue,
        "token",
        &UploadTarget::default(),
        &quick(None),
        || true,
        |event| {
            if matches!(event, QueueDriveEvent::Waiting { .. }) {
                let entry = &night.queue.list().unwrap()[0];
                marked_while_waiting.push((entry.state, entry.last_error_code.clone()));
            }
            events.push(event);
        },
    )
    .unwrap();

    assert_eq!(processed, 1);
    assert!(
        started.elapsed() >= Duration::from_millis(70),
        "it waited 10, 20 and 40 ms: {:?}",
        started.elapsed()
    );
    let waits: Vec<(u32, String)> = events
        .iter()
        .filter_map(|event| match event {
            QueueDriveEvent::Waiting { attempt, code, .. } => Some((*attempt, code.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        waits,
        [1, 2, 3].map(|attempt| (attempt, "upload_sessions_limit_reached".to_string()))
    );
    assert_eq!(
        marked_while_waiting,
        vec![(QueueState::Queued, None); 3],
        "queued and unmarked, every time"
    );
    assert!(
        !kinds(&events).contains(&"failed") && !kinds(&events).contains(&"refused"),
        "{events:?}"
    );
    assert_eq!(creations(&service), 4, "three refusals, then the file went");
    assert_eq!(states(&night), [QueueState::Completed]);
    assert_eq!(model.captures_created(), 1);
}

/// A service that fails (`500`) is not asked forever by a run that was given a
/// limit: it waits, asks again the number of times it may, then stops and
/// leaves the file queued -- unmarked, for the next run.
#[test]
fn a_failing_service_is_waited_for_a_while_then_the_file_is_left_queued() {
    let model = Arc::new(UploadModel::default());
    let service = service_with(&model, |request| {
        is_create(request).then(|| refusal(500, "internal_error"))
    });
    let night = night(&["frame.tif"]);

    let (processed, events) = run(&night, &service, &quick(Some(2)));

    assert_eq!(processed, 0);
    assert_eq!(
        kinds(&events),
        ["started", "waiting", "waiting", "failed", "stopped"],
        "{events:?}"
    );
    assert!(matches!(
        &events[3],
        QueueDriveEvent::Failed { code, .. } if code == "service_error"
    ));
    assert_eq!(creations(&service), 3, "the first try and two new ones");
    let entry = &night.queue.list().unwrap()[0];
    assert_eq!(entry.state, QueueState::Queued);
    assert_eq!(entry.last_error_code, None);
}

/// The wait can be interrupted: a pause during it ends the run at once.
#[test]
fn a_pause_during_the_wait_ends_the_run_and_leaves_the_file_queued() {
    let model = Arc::new(UploadModel::default());
    let service = service_with(&model, |request| {
        is_create(request).then(|| refusal(409, "upload_sessions_limit_reached"))
    });
    let night = night(&["frame.tif"]);
    let options = DriveOptions {
        backoff: Backoff::new(vec![Duration::from_secs(600)]),
        max_replays: None,
        replay_when_unanswered: true,
    };
    let transport =
        HttpUploadTransport::with_options(service.base_url(), fast(), WaitHooks::silent()).unwrap();
    let paused = AtomicBool::new(false);
    let events = Mutex::new(Vec::new());
    let started = Instant::now();

    let processed = drive_queue(
        &transport,
        &night.queue,
        "token",
        &UploadTarget::default(),
        &options,
        || !paused.load(Ordering::SeqCst),
        |event| {
            if matches!(event, QueueDriveEvent::Waiting { .. }) {
                paused.store(true, Ordering::SeqCst);
            }
            events.lock().unwrap().push(event);
        },
    )
    .unwrap();

    assert_eq!(processed, 0);
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(matches!(
        events.lock().unwrap().last(),
        Some(QueueDriveEvent::Stopped {
            reason: StopReason::Paused,
            ..
        })
    ));
    assert_eq!(states(&night), [QueueState::Queued]);
    assert_eq!(creations(&service), 1);
}

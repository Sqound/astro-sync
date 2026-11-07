//! End to end: a fixture night sent to a real scratch service and
//! read back through `GET /v1/sessions/{id}` and `…/online`; a second
//! sending sends nothing identical and replaces the analysis.
//!
//! These tests need a running service and a device token of one of its
//! spaces (signed in as the application `sync`):
//!
//! ```sh
//! ASTRO_SYNC_E2E_SERVICE=http://127.0.0.1:9310 \
//! ASTRO_SYNC_E2E_TOKEN_FILE=<file holding the device token> \
//!   cargo test -p astro-sync-core --test e2e_night -- --ignored --test-threads=1
//! ```
//!
//! Without them they are ignored (never counted green). Invented data only:
//! "Col des Étoiles", 45° N 30° W, two personal targets of the space named
//! "M31" and "NGC 7000" (`POST /v1/targets/personal`: a scratch base need
//! not hold the catalogue).

#[path = "support/schema_check.rs"]
mod schema_check;

use std::fs;
use std::path::{Path, PathBuf};

use astro_sync_core::analysis::AnalysisOptions;
use astro_sync_core::analysis::report::generate_report;
use astro_sync_core::gui_bridge::HttpPortalGuiTransport;
use astro_sync_core::night_analysis::{NightAnalysis, send_night_analysis};
use astro_sync_core::night_online::{NightOnline, NightOnlineTransport};
use astro_sync_core::night_resume::{
    FolderState, LocalImage, LocalTarget, NightStatus, ResumeLocal, resume_night,
};
use astro_sync_core::precheck::{
    Decision, PrecheckTransport, check_quotas, check_with_service, enqueue_checked, plan_images,
};
use astro_sync_core::upload_queue::{QueueState, ResultKind, UploadQueue};
use astro_sync_core::uploader::{HttpUploadTransport, UploadTarget, upload_entry};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

struct Service {
    base: String,
    token: String,
}

fn service() -> Service {
    let base = std::env::var("ASTRO_SYNC_E2E_SERVICE")
        .expect("ASTRO_SYNC_E2E_SERVICE: the scratch service's address");
    let token_file = std::env::var("ASTRO_SYNC_E2E_TOKEN_FILE")
        .expect("ASTRO_SYNC_E2E_TOKEN_FILE: a file holding a device token");
    Service {
        base: base.trim_end_matches('/').to_string(),
        token: fs::read_to_string(token_file).unwrap().trim().to_string(),
    }
}

fn http(service: &Service, method: reqwest::Method, path: &str, body: Option<Value>) -> Value {
    let client = reqwest::blocking::Client::new();
    let mut request = client
        .request(method, format!("{}{path}", service.base))
        .bearer_auth(&service.token);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let answer = request.send().unwrap();
    let status = answer.status();
    let text = answer.text().unwrap();
    assert!(status.is_success(), "{path}: {status} {text}");
    serde_json::from_str(&text).unwrap()
}

/// A ULID for `clientSessionId`, unique per run.
fn ulid() -> String {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let seed = format!("{:?}{}", std::time::SystemTime::now(), std::process::id());
    let digest = Sha256::digest(seed.as_bytes());
    let mut out = String::from("01");
    for byte in digest.iter().take(24) {
        out.push(ALPHABET[(*byte as usize) % 32] as char);
    }
    out
}

/// The night's targets: two personal targets of the space.
struct Night {
    id: String,
    m31: String,
    ngc7000: String,
}

fn personal_target(service: &Service, name: &str, label: &str) -> String {
    let created = http(
        service,
        reqwest::Method::POST,
        "/api/v1/targets/personal",
        Some(json!({"name": name, "label": label})),
    );
    created["target"]["id"].as_str().unwrap().to_string()
}

/// A night of the space, with « M31 » and « NGC 7000 » as its targets.
fn new_night(service: &Service) -> Night {
    let m31 = personal_target(service, "M31", "Galaxie");
    let ngc7000 = personal_target(service, "NGC 7000", "Nébuleuse");
    let created = http(
        service,
        reqwest::Method::POST,
        "/api/v1/sessions",
        Some(json!({"startedAt": "2026-10-07T19:00:00.000Z", "clientSessionId": ulid()})),
    );
    let id = created["id"]
        .as_str()
        .or_else(|| created["session"]["id"].as_str())
        .unwrap()
        .to_string();
    http(
        service,
        reqwest::Method::PATCH,
        &format!("/api/v1/sessions/{id}/night"),
        Some(json!({
            "title": "Col des Étoiles",
            "place": {"name": "Col des Étoiles", "latitude": 45.0, "longitude": -30.0},
            "targetIds": [m31, ngc7000],
        })),
    );
    Night { id, m31, ngc7000 }
}

/// A valid one-pixel PNG of `color` (stored zlib block, CRCs computed).
fn png(color: [u8; 3]) -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFF_u32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend((data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend(data);
        out.extend(&body);
        out.extend(crc32(&body).to_be_bytes());
    }
    let raw = [0, color[0], color[1], color[2]];
    let (mut a, mut b) = (1_u32, 0_u32);
    for &byte in &raw {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    let mut zlib = vec![0x78, 0x01, 0x01, 4, 0, 0xFB, 0xFF];
    zlib.extend(raw);
    zlib.extend(((b << 16) | a).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]);
    chunk(&mut out, b"IDAT", &zlib);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn header(cards: &[(&str, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (k, v) in [
        ("SIMPLE", "T".to_string()),
        ("BITPIX", "16".into()),
        ("NAXIS", "0".into()),
    ]
    .iter()
    .chain(cards.iter())
    {
        let mut card = format!("{k:<8}= {v}").into_bytes();
        card.resize(80, b' ');
        out.extend(card);
    }
    let mut end = b"END".to_vec();
    end.resize(80, b' ');
    out.extend(end);
    let len = out.len().div_ceil(2880) * 2880;
    out.resize(len, b' ');
    out
}

/// The night's folder: logs, two M31 lights, a final image of M31 unique to
/// this run.
fn fixture_folder(label: &str, color: [u8; 3]) -> PathBuf {
    let root = std::env::temp_dir().join(format!("astro-sync-e2e-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    for folder in ["logs", "M31/lights", "M31/final", "NGC 7000/lights"] {
        fs::create_dir_all(root.join(folder)).unwrap();
    }
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    fs::copy(
        fixtures.join("nina_log_two_targets.txt"),
        root.join("logs/NINA.log"),
    )
    .unwrap();
    for (n, at) in [(1, "2026-10-07T22:02:00"), (2, "2026-10-07T22:04:10")] {
        fs::write(
            root.join(format!("M31/lights/l-000{n}.fits")),
            header(&[
                ("IMAGETYP", "'LIGHT'".into()),
                ("OBJECT", "'M 31'".into()),
                ("FILTER", "'L'".into()),
                ("EXPTIME", "120.0".into()),
                ("GAIN", "100".into()),
                ("DATE-OBS", format!("'{at}'")),
            ]),
        )
        .unwrap();
    }
    fs::write(root.join("M31/final/m31.png"), png(color)).unwrap();
    root
}

fn local_of(root: &Path, night: &Night) -> ResumeLocal {
    let bytes = fs::read(root.join("M31/final/m31.png")).unwrap();
    ResumeLocal {
        folder: FolderState::Found,
        targets: vec![
            LocalTarget {
                target_id: night.m31.clone(),
                has_folder: true,
                light_frames: 2,
            },
            LocalTarget {
                target_id: night.ngc7000.clone(),
                has_folder: true,
                light_frames: 0,
            },
        ],
        logs: 1,
        images: vec![LocalImage {
            target_id: night.m31.clone(),
            path: root.join("M31/final/m31.png"),
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            size: bytes.len() as u64,
            kind: ResultKind::Final,
            twin_sha256: None,
        }],
        analysis_inputs: None,
    }
}

/// One sending of the night as the wizard does it: the report, then the
/// images the pre-check lets through. Returns the analysis's `replaced` and
/// how many images left.
fn send(service: &Service, night: &Night, root: &Path, queue: &UploadQueue) -> (bool, usize) {
    let night_id = night.id.as_str();
    let transport = HttpPortalGuiTransport::new().unwrap();
    let analysis = NightAnalysis {
        report: generate_report(root, &AnalysisOptions::default()).unwrap(),
        sky_measure: None,
        sky_missing_reason: Some("no_reference_stars".into()),
        logs_utc_offset: None,
    };
    let sent = send_night_analysis(
        &transport,
        &service.base,
        &service.token,
        night_id,
        &analysis,
    )
    .unwrap();
    let replaced = sent.analysis.unwrap().replaced;

    let online = transport
        .night_online(&service.base, &service.token, night_id)
        .unwrap();
    let storage = transport
        .space_storage(&service.base, &service.token)
        .unwrap();
    let mut plans = plan_images(
        &online,
        &local_of(root, night).candidates(),
        storage.variant_limit,
    );
    let refused = check_with_service(
        &transport,
        &service.base,
        &service.token,
        night_id,
        &mut plans,
    )
    .unwrap();
    assert!(refused.is_empty(), "{refused:?}");
    let check = check_quotas(&storage, &online, &plans);
    assert!(check.allowed, "{:?}", check.findings);
    enqueue_checked(queue, night_id, &plans, &check)
        .unwrap()
        .unwrap();
    let uploads = HttpUploadTransport::new(&service.base).unwrap();
    let mut sent_images = 0;
    while let Some(entry) = queue.next_pending().unwrap() {
        upload_entry(
            &uploads,
            queue,
            &service.token,
            &entry,
            &UploadTarget::default(),
            |_, _| {},
        )
        .unwrap();
        sent_images += 1;
    }
    (replaced, sent_images)
}

fn queue_in(label: &str) -> UploadQueue {
    let dir = std::env::temp_dir().join(format!(
        "astro-sync-e2e-queue-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    UploadQueue::open(&dir.join("queue.sqlite")).unwrap()
}

#[test]
#[ignore = "needs a scratch service: ASTRO_SYNC_E2E_SERVICE and ASTRO_SYNC_E2E_TOKEN_FILE"]
fn a_fixture_night_is_sent_and_read_back_through_the_night_and_online() {
    let service = service();
    let night = new_night(&service);
    let night_id = night.id.clone();
    let root = fixture_folder("first", [200, 40, 90]);
    let queue = queue_in("first");
    let (replaced, images) = send(&service, &night, &root, &queue);
    assert!(!replaced, "the night's first analysis");
    assert_eq!(images, 1);
    let row = &queue.list().unwrap()[0];
    assert_eq!(row.state, QueueState::Completed);

    // The night as the other clients read it.
    let session = http(
        &service,
        reqwest::Method::GET,
        &format!("/api/v1/sessions/{night_id}"),
        None,
    );
    let text = session.to_string();
    assert!(
        text.contains("m-31-l-120s-g100"),
        "the step of the report: {text}"
    );

    // What is online, as the resume reads it.
    let online_value = http(
        &service,
        reqwest::Method::GET,
        &format!("/api/v1/sessions/{night_id}/online"),
        None,
    );
    schema_check::assert_valid("session-online.schema.json", "", &online_value);
    let online: NightOnline = serde_json::from_value(online_value).unwrap();
    assert!(online.analysis.present);
    assert_eq!(online.analysis.producer.as_deref(), Some("astro-sync"));
    let m31 = online.target(&night.m31).unwrap();
    assert_eq!(m31.captures.len(), 1);
    let main = m31.captures[0].main_image.as_ref().unwrap();
    assert_eq!(main.sha256, row.sha256);
    assert_eq!(main.source.as_deref(), Some("astro-sync"));
    assert_eq!(
        m31.captures[0].stage, "draft",
        "a device's sending is a draft"
    );

    // The resume: NGC 7000 has no image, M31's has no sky solution yet.
    let resume = resume_night(
        &online,
        &local_of(&root, &night),
        &queue.list().unwrap(),
        None,
        12,
    );
    assert_eq!(resume.status, NightStatus::ToComplete);
    assert!(
        resume.sentence().contains("final image of NGC 7000"),
        "{}",
        resume.sentence()
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
#[ignore = "needs a scratch service: ASTRO_SYNC_E2E_SERVICE and ASTRO_SYNC_E2E_TOKEN_FILE"]
fn a_second_sending_sends_nothing_identical_and_replaces_the_analysis() {
    let service = service();
    let night = new_night(&service);
    let night_id = night.id.clone();
    let root = fixture_folder("second", [20, 160, 230]);
    let (replaced, images) = send(&service, &night, &root, &queue_in("second-a"));
    assert!(!replaced);
    assert_eq!(images, 1);

    // Again, from a fresh queue (another computer, or the queue cleared):
    // the identical image is "already online" and nothing leaves; the
    // analysis replaces the first one.
    let fresh = queue_in("second-b");
    let (replaced, images) = send(&service, &night, &root, &fresh);
    assert!(replaced, "one analysis per night: the new one replaces it");
    assert_eq!(images, 0, "nothing identical is sent again");
    assert!(fresh.list().unwrap().is_empty());

    let transport = HttpPortalGuiTransport::new().unwrap();
    let online = transport
        .night_online(&service.base, &service.token, &night_id)
        .unwrap();
    let plans = plan_images(&online, &local_of(&root, &night).candidates(), 12);
    assert!(matches!(plans[0].decision, Decision::AlreadyOnline { .. }));
    let analyses = http(
        &service,
        reqwest::Method::GET,
        &format!("/api/v1/sessions/{night_id}/analyses"),
        None,
    );
    assert_eq!(analyses["reportVersion"], 3);
    let _ = fs::remove_dir_all(&root);
}

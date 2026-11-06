//! The four limits of `GET /v1/space/storage` checked before sending
//! (the total against the place left, each file against the per-file
//! ceiling, the variants of a capture, the new captures), nothing enqueued
//! when one is passed, a private server's limits, the service's refusal
//! after the fact kept in the queue with its code and resumable, and the
//! treatments decided by the capabilities and the tools -- never by an
//! offer's name. Invented nights.

#[path = "support/fake_service.rs"]
mod fake_service;
#[path = "support/schema_check.rs"]
mod schema_check;

use std::path::{Path, PathBuf};
use std::time::Duration;

use astro_sync_core::gui_bridge::Capabilities;
use astro_sync_core::night_online::NightOnline;
use astro_sync_core::precheck::{
    Decision, ImageCandidate, NightMaterial, QuotaFinding, SpaceStorage, ToolsPresent, Treatment,
    Unavailable, check_quotas, enqueue_checked, plan_images, treatments,
};
use astro_sync_core::service_wait::{WaitHooks, WakePolicy};
use astro_sync_core::upload_queue::{QueueState, ResultKind, UploadQueue};
use astro_sync_core::uploader::{HttpUploadTransport, UploadTarget, Verdict, upload_entry};
use fake_service::{FakeService, Wake, error_reply};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/online")
            .join(name),
    )
    .unwrap()
}

fn online(name: &str) -> NightOnline {
    serde_json::from_str(&fixture(name)).unwrap()
}

fn storage() -> SpaceStorage {
    let text = fixture("space_storage.json");
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    schema_check::assert_valid("space-storage.schema.json", "", &value);
    serde_json::from_str(&text).unwrap()
}

fn image(target: &str, path: &str, sha: char, size: u64, kind: ResultKind) -> ImageCandidate {
    ImageCandidate {
        render_rank: None,
        target_id: target.into(),
        path: PathBuf::from(path),
        sha256: sha.to_string().repeat(64),
        size,
        kind,
        render: None,
        contrast: Default::default(),
        retouch: Default::default(),
        frame_count: None,
    }
}

const M31_CAPTURE: &str = "33333333-3333-4333-8333-333333333333";

/// The unlimited sentinel of the service (`Number.MAX_SAFE_INTEGER`).
const UNLIMITED: u64 = 9_007_199_254_740_991;

#[test]
fn each_limit_alone_and_exactly_at_the_limit() {
    let night = online("night_in_progress.json");
    let plans = plan_images(
        &night,
        &[
            image("m31", "/n/M31/final/m31.tif", 'c', 30, ResultKind::Final),
            image(
                "ic1396",
                "/n/IC 1396/final/ic1396.tif",
                'd',
                40,
                ResultKind::Final,
            ),
        ],
        12,
    );
    let roomy = storage();
    let codes = |s: &SpaceStorage| -> Vec<String> {
        check_quotas(s, &night, &plans)
            .findings
            .iter()
            .map(|f| f.code().to_string())
            .collect()
    };
    assert!(codes(&roomy).is_empty());

    let at_the_limit = SpaceStorage {
        remaining_bytes: 70,
        max_file_bytes: 40,
        variant_limit: 1,
        capture_count: 2,
        capture_count_limit: 3,
        ..roomy
    };
    assert!(
        codes(&at_the_limit).is_empty(),
        "exactly at each limit is allowed"
    );

    for (storage, code) in [
        (
            SpaceStorage {
                remaining_bytes: 69,
                ..at_the_limit
            },
            "storage_quota_reached",
        ),
        (
            SpaceStorage {
                max_file_bytes: 39,
                ..at_the_limit
            },
            "file_too_large",
        ),
        (
            SpaceStorage {
                capture_count: 3,
                ..at_the_limit
            },
            "capture_limit_reached",
        ),
    ] {
        assert_eq!(codes(&storage), [code]);
    }
    // M31's capture already holds one variant: a second one passes a limit
    // of one.
    let mut fuller = night.clone();
    let capture = &mut fuller.targets[1].captures[0];
    capture.image_count = 2;
    let check = check_quotas(&at_the_limit, &fuller, &plans);
    assert_eq!(
        check.findings,
        [QuotaFinding::VariantLimit {
            target_id: "m31".into(),
            capture_id: Some(M31_CAPTURE.into()),
            existing: 1,
            adding: 1,
            variant_limit: 1
        }]
    );
}

#[test]
fn a_private_server_announcing_no_cap_passes_every_check() {
    let night = online("night_bare.json");
    let open = SpaceStorage {
        quota_bytes: UNLIMITED,
        remaining_bytes: UNLIMITED,
        max_file_bytes: UNLIMITED,
        capture_count_limit: UNLIMITED,
        variant_limit: UNLIMITED,
        ..storage()
    };
    let plans = plan_images(
        &night,
        &[
            image(
                "m31",
                "/n/M31/stacked/a.tif",
                'e',
                3_000_000_000,
                ResultKind::Stacked,
            ),
            image(
                "m31",
                "/n/M31/stacked/b.tif",
                'f',
                3_000_000_000,
                ResultKind::Stacked,
            ),
        ],
        open.variant_limit,
    );
    assert!(matches!(plans[1].decision, Decision::VariantOfNew { .. }));
    let check = check_quotas(&open, &night, &plans);
    assert!(check.allowed);
    assert_eq!(check.to_send_bytes, 6_000_000_000);
}

#[test]
fn the_four_limits_are_said_with_their_numbers_and_nothing_is_enqueued() {
    let night = online("night_in_progress.json");
    let mut tight = storage();
    tight.remaining_bytes = 50;
    tight.max_file_bytes = 35;
    tight.variant_limit = 0;
    tight.capture_count = 3;
    tight.capture_count_limit = 3;
    // Planned with variants (the space's profile changed since): the check
    // still holds the line.
    let plans = plan_images(
        &night,
        &[
            image("m31", "/n/M31/final/m31.tif", 'c', 30, ResultKind::Final),
            image(
                "ic1396",
                "/n/IC 1396/final/ic1396.tif",
                'd',
                40,
                ResultKind::Final,
            ),
        ],
        12,
    );
    let check = check_quotas(&tight, &night, &plans);
    assert!(!check.allowed);
    assert_eq!(check.to_send_bytes, 70);
    let codes: Vec<&str> = check.findings.iter().map(QuotaFinding::code).collect();
    assert_eq!(
        codes,
        [
            "storage_quota_reached",
            "file_too_large",
            "capture_variant_limit_reached",
            "capture_limit_reached"
        ]
    );
    assert!(check.findings.contains(&QuotaFinding::FileTooLarge {
        path: PathBuf::from("/n/IC 1396/final/ic1396.tif"),
        size: 40,
        max_file_bytes: 35
    }));

    let dir = std::env::temp_dir().join(format!("astro-sync-precheck-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let queue = UploadQueue::open(&dir.join("queue.sqlite")).unwrap();
    let refused = enqueue_checked(&queue, &night.session_id, &plans, &check).unwrap();
    assert_eq!(refused.unwrap_err().len(), 4);
    assert!(queue.list().unwrap().is_empty(), "nothing is enqueued");

    // Within the limits: each image goes with its night, its target, and
    // the capture it is a variant of.
    let check = check_quotas(&storage(), &night, &plans);
    assert!(check.allowed, "{:?}", check.findings);
    assert_eq!(check.new_captures, 1);
    let outcome = enqueue_checked(&queue, &night.session_id, &plans, &check)
        .unwrap()
        .unwrap();
    assert_eq!(outcome.inserted, 2);
    let rows = queue.list().unwrap();
    assert_eq!(rows[0].target.variant_of.as_deref(), Some(M31_CAPTURE));
    assert_eq!(
        rows[0].target.session_id.as_deref(),
        Some(night.session_id.as_str())
    );
    assert_eq!(rows[0].target.target_ids, ["m31"]);
    assert_eq!(rows[1].target.variant_of, None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_treatments_follow_the_capabilities_and_the_tools_never_an_offer() {
    let all = storage().capabilities;
    let without = Capabilities {
        analysis_logs: false,
        analysis_progression: false,
        ..all
    };
    let material = NightMaterial {
        light_frames: true,
        device_images: false,
        logs: true,
        final_image: true,
    };
    let tools = ToolsPresent {
        stacking_pipeline: true,
        astap: false,
        siril: true,
    };
    let reserved = treatments(&without, &tools, &material);
    let why = |states: &[astro_sync_core::precheck::TreatmentState], t: Treatment| {
        states
            .iter()
            .find(|s| s.treatment == t)
            .unwrap()
            .unavailable
            .clone()
    };
    // The stacking is local and never gated: a space without
    // `analysis.progression` (the free account) stacks all the same.
    assert_eq!(why(&reserved, Treatment::Stacking), None);
    for (treatment, capability) in [
        (Treatment::Logs, "analysis.logs"),
        (Treatment::Progression, "analysis.progression"),
    ] {
        assert_eq!(
            why(&reserved, treatment),
            Some(Unavailable::NotInSubscription {
                capability: capability.into()
            })
        );
    }
    assert_eq!(why(&reserved, Treatment::FinalImage), None);
    assert_eq!(
        why(&reserved, Treatment::SkyAnnotations),
        Some(Unavailable::ToolMissing {
            tool: "astap".into()
        })
    );
    // A private server announces every capability: all open but the tool.
    let open = treatments(&all, &tools, &material);
    assert!(
        open.iter()
            .filter(|s| s.treatment != Treatment::SkyAnnotations)
            .all(|s| s.available)
    );
    let rendered = serde_json::to_string(&open).unwrap();
    assert!(!rendered.contains("avancée"), "no offer's name in the core");
    let empty = treatments(&all, &tools, &NightMaterial::default());
    assert_eq!(
        why(&empty, Treatment::Logs),
        Some(Unavailable::NothingToTreat)
    );
}

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

#[test]
fn a_variant_the_service_refuses_after_the_check_stays_in_the_queue_resumable() {
    let source = std::env::temp_dir().join(format!("astro-sync-quota-late-{}", std::process::id()));
    std::fs::create_dir_all(source.join("M31/final")).unwrap();
    std::fs::write(source.join("M31/final/m31.tif"), vec![3_u8; 30]).unwrap();
    let service = FakeService::start(Wake::Instant, |request| {
        if request.method == "POST" && request.path == "/api/v1/uploads" {
            // Another sending took the last variant meanwhile.
            return error_reply(
                409,
                "capture_variant_limit_reached",
                "This capture holds as many images as the profile allows.",
            );
        }
        error_reply(404, "not_found", "no such route")
    });
    let night = online("night_in_progress.json");
    let mut candidate = image("m31", "", 'c', 30, ResultKind::Final);
    candidate.path = source.join("M31/final/m31.tif");
    let plans = plan_images(&night, &[candidate], 12);
    let check = check_quotas(&storage(), &night, &plans);
    let queue = UploadQueue::open(&source.join("queue.sqlite")).unwrap();
    enqueue_checked(&queue, &night.session_id, &plans, &check)
        .unwrap()
        .unwrap();
    let entry = queue.next_pending().unwrap().unwrap();
    let transport =
        HttpUploadTransport::with_options(service.base_url(), fast(), WaitHooks::silent()).unwrap();
    let error = upload_entry(
        &transport,
        &queue,
        "t",
        &entry,
        &UploadTarget::default(),
        |_, _| {},
    )
    .unwrap_err();
    assert_eq!(error.verdict(), Verdict::NeedsPerson);
    assert_eq!(error.code(), "capture_variant_limit_reached");
    let held = queue.find_by_sha256(&"c".repeat(64)).unwrap().unwrap();
    assert_eq!(held.state, QueueState::Held);
    assert_eq!(
        held.last_error_code.as_deref(),
        Some("capture_variant_limit_reached")
    );
    assert!(astro_sync_core::refusal::plain_sentence("capture_variant_limit_reached").is_some());
    // The person acted: the next run takes it again.
    assert_eq!(queue.release_held().unwrap(), 1);
    assert_eq!(
        queue.next_pending().unwrap().unwrap().sha256,
        "c".repeat(64)
    );
    let _ = std::fs::remove_dir_all(&source);
}

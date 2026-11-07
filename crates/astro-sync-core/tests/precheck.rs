//! Before anything is sent, the SHA-256 of each image is compared with what is
//! online and with the service's own answer (`POST /v1/uploads/check`); an image for a target whose
//! capture is online becomes a variant, or "will not be sent"; the space's storage is read from
//! `GET /v1/space/storage` (its limits and the treatments: `tests/quotas.rs`). Invented nights
//! (`tests/fixtures/online/`), the fake service.

#[path = "support/fake_service.rs"]
mod fake_service;
#[path = "support/schema_check.rs"]
mod schema_check;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use astro_sync_core::gui_bridge::HttpPortalGuiTransport;
use astro_sync_core::night_online::{NightOnline, NightOnlineTransport};
use astro_sync_core::precheck::{
    Decision, ImageCandidate, NotSent, PrecheckTransport, SpaceStorage, check_quotas,
    check_with_service, enqueue_checked, plan_images, plan_images_within, rebind_progressions,
    several_renders_allowed,
};
use astro_sync_core::service_wait::{WaitHooks, WakePolicy};
use astro_sync_core::upload_queue::{QueueState, ResultKind, UploadQueue};
use astro_sync_core::uploader::{HttpUploadTransport, UploadTarget, upload_entry};
use fake_service::{FakeService, Reply, Wake, error_reply};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/online")
            .join(name),
    )
    .unwrap()
}

fn online(name: &str) -> NightOnline {
    let text = fixture(name);
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    schema_check::assert_valid("session-online.schema.json", "", &value);
    serde_json::from_str(&text).unwrap()
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
        twin_sha256: None,
    }
}

fn decisions(plans: &[astro_sync_core::precheck::ImagePlan]) -> Vec<Decision> {
    plans.iter().map(|p| p.decision.clone()).collect()
}

const NGC7000_CAPTURE: &str = "11111111-1111-4111-8111-111111111111";
const M31_CAPTURE: &str = "33333333-3333-4333-8333-333333333333";

#[test]
fn an_identical_image_stays_online_a_different_one_is_a_variant() {
    let night = online("night_in_progress.json");
    let plans = plan_images(
        &night,
        &[
            // Same bytes as the capture online: "already online, identical".
            image(
                "ngc7000",
                "/n/NGC 7000/final/ngc7000-final.tif",
                'a',
                48,
                ResultKind::Final,
            ),
            // M31 has a capture from the phone: a different final is a variant.
            image("m31", "/n/M31/final/m31.tif", 'c', 30, ResultKind::Final),
            // IC 1396 has nothing online: a new capture.
            image(
                "ic1396",
                "/n/IC 1396/final/ic1396.tif",
                'd',
                20,
                ResultKind::Final,
            ),
        ],
        12,
    );
    assert_eq!(
        decisions(&plans),
        [
            Decision::AlreadyOnline {
                capture_id: NGC7000_CAPTURE.into(),
                image_id: "21111111-1111-4111-8111-111111111111".into()
            },
            Decision::Variant {
                capture_id: M31_CAPTURE.into()
            },
            Decision::NewCapture,
        ]
    );
}

#[test]
fn without_variants_a_different_image_will_not_be_sent() {
    let night = online("night_in_progress.json");
    let plans = plan_images(
        &night,
        &[image(
            "m31",
            "/n/M31/final/m31.tif",
            'c',
            30,
            ResultKind::Final,
        )],
        0,
    );
    assert_eq!(
        decisions(&plans),
        [Decision::NotSent {
            reason: NotSent::VariantsNotAllowed
        }]
    );
}

#[test]
fn the_stacking_and_a_final_file_on_one_target_make_one_variant() {
    let night = online("night_in_progress.json");
    let plans = plan_images(
        &night,
        &[
            image("m31", "/n/M31/final/m31.tif", 'c', 30, ResultKind::Final),
            image(
                "m31",
                "/n/M31/stacked/m31-raw.tif",
                'e',
                40,
                ResultKind::Stacked,
            ),
        ],
        12,
    );
    assert_eq!(
        decisions(&plans),
        [
            Decision::NotSent {
                reason: NotSent::ReplacedByStacking
            },
            Decision::Variant {
                capture_id: M31_CAPTURE.into()
            },
        ]
    );
}

/// A stacked image above the space's per-file ceiling cannot leave; the
/// person's final image of the target is sent instead -- the night is not left
/// with nothing, and the stacked file says why it stays.
#[test]
fn a_stacked_image_above_the_ceiling_leaves_the_final_image_to_be_sent() {
    let night = online("night_bare.json");
    let candidates = [
        image(
            "m31",
            "/n/M31/final/m31.jpg",
            'c',
            2_000_000,
            ResultKind::Final,
        ),
        image(
            "m31",
            "/n/M31/stacked/m31-raw.tif",
            'e',
            48_000_000,
            ResultKind::Stacked,
        ),
        image(
            "m31",
            "/n/M31/stacked/m31-raw-presentation.tif",
            'f',
            48_000_000,
            ResultKind::Stacked,
        ),
    ];
    let plans = plan_images_within(&night, &candidates, 12, 25_000_000);
    assert_eq!(
        decisions(&plans),
        [
            Decision::NewCapture,
            Decision::NotSent {
                reason: NotSent::TooLargeForSpace
            },
            Decision::NotSent {
                reason: NotSent::TooLargeForSpace
            },
        ]
    );
    let storage = SpaceStorage {
        max_file_bytes: 25_000_000,
        ..storage()
    };
    assert!(
        check_quotas(&storage, &night, &plans).allowed,
        "nothing exceeds the ceiling any more"
    );
    // A ceiling the stacked image fits under changes nothing: it takes the
    // final image's place, as before.
    let roomy = plan_images_within(&night, &candidates, 12, 100_000_000);
    assert_eq!(
        roomy[0].decision,
        Decision::NotSent {
            reason: NotSent::ReplacedByStacking
        }
    );
    assert_eq!(roomy[1].decision, Decision::NewCapture);
    // Only the heavy render is set aside when another one fits.
    let mixed = [
        candidates[0].clone(),
        candidates[1].clone(),
        image(
            "m31",
            "/n/M31/stacked/m31-small.tif",
            'g',
            9_000_000,
            ResultKind::Stacked,
        ),
    ];
    let plans = plan_images_within(&night, &mixed, 12, 25_000_000);
    assert_eq!(
        decisions(&plans),
        [
            Decision::NotSent {
                reason: NotSent::ReplacedByStacking
            },
            Decision::NotSent {
                reason: NotSent::TooLargeForSpace
            },
            Decision::NewCapture,
        ]
    );
}

#[test]
fn several_renders_are_variants_only_when_the_space_keeps_variants() {
    let night = online("night_bare.json");
    let renders = [
        image(
            "m31",
            "/n/M31/stacked/m31-raw.tif",
            'e',
            40,
            ResultKind::Stacked,
        ),
        image(
            "m31",
            "/n/M31/stacked/m31-dual-band.tif",
            'f',
            40,
            ResultKind::Stacked,
        ),
    ];
    assert_eq!(
        decisions(&plan_images(&night, &renders, 12)),
        [
            Decision::NewCapture,
            Decision::VariantOfNew {
                first_sha256: "e".repeat(64)
            }
        ]
    );
    assert_eq!(
        decisions(&plan_images(&night, &renders, 0)),
        [
            Decision::NewCapture,
            Decision::NotSent {
                reason: NotSent::OneRenderOnly
            }
        ]
    );
    assert!(several_renders_allowed(&storage()));
}

/// An image that carries softened stars or stronger colours is a variant, even
/// without a contrast: the plain render stays the main image.
#[test]
fn a_render_with_a_finish_is_a_variant_of_the_plain_one() {
    use astro_sync_core::stacking::{Colour, Retouch, SoftStars};

    let night = online("night_bare.json");
    let render = |path: &str, sha: char, retouch: Retouch| ImageCandidate {
        render_rank: None,
        render: Some("raw".to_string()),
        retouch,
        ..image("m31", path, sha, 40, ResultKind::Stacked)
    };
    let renders = [
        render(
            "/n/M31/stacked/m31_raw_colour-strong.tif",
            'c',
            Retouch {
                colour: Colour::Strong,
                soft_stars: SoftStars::None,
            },
        ),
        render(
            "/n/M31/stacked/m31_raw_stars-light.tif",
            's',
            Retouch {
                colour: Colour::None,
                soft_stars: SoftStars::Light,
            },
        ),
        render("/n/M31/stacked/m31_raw.tif", 'r', Retouch::default()),
    ];
    assert_eq!(
        decisions(&plan_images(&night, &renders, 12)),
        [
            Decision::VariantOfNew {
                first_sha256: "r".repeat(64)
            },
            Decision::VariantOfNew {
                first_sha256: "r".repeat(64)
            },
            Decision::NewCapture
        ]
    );
}

/// The natural render with the contrast of presentation is a variant;
/// the natural render without it stays the main image, even when the files of
/// the curve stand first in the list.
#[test]
fn the_natural_render_without_contrast_stays_the_main_image() {
    use astro_sync_core::stacking::Contrast;

    let night = online("night_bare.json");
    let render = |path: &str, sha: char, contrast: Contrast| ImageCandidate {
        render_rank: None,
        render: Some("raw".to_string()),
        contrast,
        ..image("m31", path, sha, 40, ResultKind::Stacked)
    };
    let renders = [
        render("/n/M31/stacked/m31_raw_strong.tif", 's', Contrast::Strong),
        render(
            "/n/M31/stacked/m31_raw_presentation.tif",
            'p',
            Contrast::Presentation,
        ),
        render("/n/M31/stacked/m31_raw.tif", 'r', Contrast::None),
    ];
    assert_eq!(
        decisions(&plan_images(&night, &renders, 12)),
        [
            Decision::VariantOfNew {
                first_sha256: "r".repeat(64)
            },
            Decision::VariantOfNew {
                first_sha256: "r".repeat(64)
            },
            Decision::NewCapture
        ]
    );
    assert_eq!(
        decisions(&plan_images(&night, &renders, 0)),
        [
            Decision::NotSent {
                reason: NotSent::OneRenderOnly
            },
            Decision::NotSent {
                reason: NotSent::OneRenderOnly
            },
            Decision::NewCapture
        ],
        "without variants only the render without the curve leaves"
    );
}

/// C4: with several renders ticked, the capture's main image is the natural
/// render (`raw`), wherever its file stands in the folder (« …dualband… »
/// sorts before « …raw… »); the other renders are its variants, and the capture
/// is queued before them.
#[test]
fn the_natural_render_is_the_main_image_and_the_others_its_variants() {
    let night = online("night_bare.json");
    let render = |path: &str, sha: char, pipeline: &str| ImageCandidate {
        render_rank: None,
        render: Some(pipeline.to_string()),
        ..image("m31", path, sha, 40, ResultKind::Stacked)
    };
    let renders = [
        render(
            "/n/M31/stacked/m31_dualband-correction.tif",
            'd',
            "dualband-correction",
        ),
        render("/n/M31/stacked/m31_raw.tif", 'r', "raw"),
    ];
    assert_eq!(
        decisions(&plan_images(&night, &renders, 12)),
        [
            Decision::VariantOfNew {
                first_sha256: "r".repeat(64)
            },
            Decision::NewCapture
        ],
        "the natural render creates the capture"
    );
    assert_eq!(
        decisions(&plan_images(&night, &renders, 0)),
        [
            Decision::NotSent {
                reason: NotSent::OneRenderOnly
            },
            Decision::NewCapture
        ],
        "without variants only the natural render leaves"
    );
    // Without a natural render among them: the first one, as before.
    let others = [
        render("/n/M31/stacked/a.tif", 'a', "dualband-correction"),
        render("/n/M31/stacked/b.tif", 'b', "mono-ha"),
    ];
    assert_eq!(
        decisions(&plan_images(&night, &others, 12)),
        [
            Decision::NewCapture,
            Decision::VariantOfNew {
                first_sha256: "a".repeat(64)
            }
        ]
    );
    // The capture is queued before its variant.
    let plans = plan_images(&night, &renders, 12);
    let check = check_quotas(&storage(), &night, &plans);
    let queue_file = std::env::temp_dir().join(format!(
        "astro-sync-natural-first-{}.sqlite",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&queue_file);
    let queue = UploadQueue::open(&queue_file).unwrap();
    enqueue_checked(&queue, &night.session_id, &plans, &check)
        .unwrap()
        .unwrap();
    let first = queue.next_pending().unwrap().unwrap();
    assert_eq!(first.sha256, "r".repeat(64));
    let _ = std::fs::remove_file(&queue_file);
}

/// The first render the stacking was asked for is the capture's main image,
/// whatever the pipeline (a dual-band correction asked first beats the
/// natural render); an explicit choice moves another render to the front, and
/// leaves a target that has no such render as it was. A render with a curve is
/// never the main one.
#[test]
fn the_first_render_asked_is_the_main_image_unless_one_is_named() {
    use astro_sync_core::precheck::prefer_main_render;
    use astro_sync_core::stacking::Contrast;
    let night = online("night_bare.json");
    let render =
        |path: &str, sha: char, pipeline: &str, rank: u32, contrast: Contrast| ImageCandidate {
            render_rank: Some(rank),
            render: Some(pipeline.to_string()),
            contrast,
            ..image("m31", path, sha, 40, ResultKind::Stacked)
        };
    let mut renders = vec![
        render("/n/M31/stacked/raw.tif", 'r', "raw", 1, Contrast::None),
        render(
            "/n/M31/stacked/dual_presentation.tif",
            'p',
            "dualband-correction",
            0,
            Contrast::Presentation,
        ),
        render(
            "/n/M31/stacked/dual.tif",
            'd',
            "dualband-correction",
            0,
            Contrast::None,
        ),
    ];
    // Asked first: the dual-band correction creates the capture.
    assert_eq!(
        decisions(&plan_images(&night, &renders, 12)),
        [
            Decision::VariantOfNew {
                first_sha256: "d".repeat(64)
            },
            Decision::VariantOfNew {
                first_sha256: "d".repeat(64)
            },
            Decision::NewCapture,
        ]
    );
    // Named: the natural render is the main image.
    prefer_main_render(&mut renders, "raw");
    assert_eq!(
        decisions(&plan_images(&night, &renders, 12)),
        [
            Decision::NewCapture,
            Decision::VariantOfNew {
                first_sha256: "r".repeat(64)
            },
            Decision::VariantOfNew {
                first_sha256: "r".repeat(64)
            },
        ]
    );
    // A pipeline this target was not rendered with changes nothing.
    let before = decisions(&plan_images(&night, &renders, 12));
    prefer_main_render(&mut renders, "hoo");
    assert_eq!(decisions(&plan_images(&night, &renders, 12)), before);
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
fn the_service_says_present_or_refused_before_any_byte() {
    let storage_json = fixture("space_storage.json");
    let online_json = fixture("night_in_progress.json");
    let service = FakeService::start(Wake::Instant, move |request| {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/api/v1/space/storage") => Reply::Json(200, storage_json.clone()),
            ("GET", path) if path.ends_with("/online") => Reply::Json(200, online_json.clone()),
            ("POST", "/api/v1/uploads/check") => {
                let body = request.json();
                let numbers = r#"{"usedBytes":1,"quotaBytes":100,"maxFileBytes":50,"captureCount":2,"captureCountLimit":3,"variantLimit":12}"#;
                if body["sha256"] == "d".repeat(64) {
                    // Another night of the space already holds these bytes.
                    Reply::Json(
                        200,
                        format!(
                            r#"{{"result":"present","captureId":"99999999-9999-4999-8999-999999999999","imageId":"img-9","main":true,"storage":{numbers}}}"#
                        ),
                    )
                } else if body["captureId"] == M31_CAPTURE {
                    Reply::Json(
                        200,
                        format!(
                            r#"{{"result":"refused","code":"capture_variant_limit_reached","message":"This capture holds as many images as the profile allows.","storage":{numbers}}}"#
                        ),
                    )
                } else {
                    Reply::Json(
                        200,
                        format!(r#"{{"result":"accepted","adds":"capture","storage":{numbers}}}"#),
                    )
                }
            }
            _ => error_reply(404, "not_found", "no such route"),
        }
    });
    let transport = HttpPortalGuiTransport::with_options(fast(), WaitHooks::silent()).unwrap();
    let base = service.base_url();
    let space = transport.space_storage(&base, "token").unwrap();
    assert_eq!(space.variant_limit, 12);
    assert!(space.capabilities.analysis_logs);
    let night = transport
        .night_online(&base, "token", "7f3a2c91-0000-4000-8000-00000000000a")
        .unwrap();

    let mut plans = plan_images(
        &night,
        &[
            image("m31", "/n/M31/final/m31.tif", 'c', 30, ResultKind::Final),
            image(
                "ic1396",
                "/n/IC 1396/final/ic1396.tif",
                'd',
                20,
                ResultKind::Final,
            ),
        ],
        space.variant_limit,
    );
    let findings =
        check_with_service(&transport, &base, "token", &night.session_id, &mut plans).unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code(), "capture_variant_limit_reached");
    assert!(matches!(plans[1].decision, Decision::AlreadyOnline { .. }));

    let checks: Vec<serde_json::Value> = service
        .requests()
        .into_iter()
        .filter(|r| r.path == "/api/v1/uploads/check")
        .map(|r| r.json())
        .collect();
    assert_eq!(checks.len(), 2);
    assert_eq!(checks[0]["captureId"], M31_CAPTURE);
    assert_eq!(checks[0]["source"], "astro-sync");
    assert_eq!(checks[0]["sessionId"], night.session_id);
    assert_eq!(checks[1].get("captureId"), None);
    assert_eq!(
        service.app_hits("POST", "/api/v1/uploads"),
        2,
        "two checks, and no upload created"
    );
}

#[test]
fn the_resumable_create_answers_present_before_its_first_chunk() {
    let source = std::env::temp_dir().join(format!("astro-sync-present-{}", std::process::id()));
    std::fs::create_dir_all(source.join("M31/final")).unwrap();
    std::fs::write(source.join("M31/final/m31.tif"), vec![7_u8; 64]).unwrap();
    let service = FakeService::start(Wake::Instant, |request| {
        match (request.method.as_str(), request.path.as_str()) {
            ("POST", "/api/v1/uploads") => Reply::Json(
                200,
                format!(
                    r#"{{"uploadId":"u-1","offset":64,"byteSize":64,"chunkSize":8388608,"completed":true,"captureId":"{M31_CAPTURE}","present":true,"imageId":"43333333-3333-4333-8333-333333333333"}}"#
                ),
            ),
            _ => error_reply(404, "not_found", "no such route"),
        }
    });
    let queue = UploadQueue::open(&source.join("queue.sqlite")).unwrap();
    queue
        .enqueue_results(&[astro_sync_core::upload_queue::ResultFile {
            path: source.join("M31/final/m31.tif"),
            sha256: "b".repeat(64),
            size: 64,
            kind: ResultKind::Final,
            target: Default::default(),
        }])
        .unwrap();
    let entry = queue.next_pending().unwrap().unwrap();
    let transport =
        HttpUploadTransport::with_options(service.base_url(), fast(), WaitHooks::silent()).unwrap();
    let outcome = upload_entry(
        &transport,
        &queue,
        "token",
        &entry,
        &UploadTarget::default(),
        |_, _| {},
    )
    .unwrap();
    assert!(outcome.deduplicated, "« already online, identical »");
    assert_eq!(outcome.capture_id, M31_CAPTURE);
    assert_eq!(
        service.app_hits("PUT", "/api/v1/uploads/"),
        0,
        "no byte sent"
    );
    let _ = std::fs::remove_dir_all(&source);
}

#[test]
fn a_render_after_the_first_is_sent_as_a_variant_of_the_capture_it_created() {
    let source = std::env::temp_dir().join(format!("astro-sync-renders-{}", std::process::id()));
    std::fs::create_dir_all(source.join("M31/stacked")).unwrap();
    std::fs::write(source.join("M31/stacked/raw.tif"), vec![1_u8; 16]).unwrap();
    std::fs::write(source.join("M31/stacked/dual-band.tif"), vec![2_u8; 16]).unwrap();
    let creates = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
    let seen = Arc::clone(&creates);
    let counter = Arc::new(AtomicUsize::new(0));
    let service = FakeService::start(Wake::Instant, move |request| {
        if request.method == "POST" && request.path == "/api/v1/uploads" {
            seen.lock().unwrap().push(request.json());
            let n = counter.fetch_add(1, Ordering::SeqCst);
            return Reply::Json(
                200,
                format!(
                    r#"{{"uploadId":"u-{n}","offset":16,"byteSize":16,"chunkSize":8388608,"completed":true,"captureId":"capture-new"}}"#
                ),
            );
        }
        error_reply(404, "not_found", "no such route")
    });
    let night = online("night_bare.json");
    let candidates = [
        ImageCandidate {
            render_rank: None,
            sha256: "e".repeat(64),
            ..image("m31", "", 'e', 16, ResultKind::Stacked)
        },
        ImageCandidate {
            render_rank: None,
            sha256: "f".repeat(64),
            ..image("m31", "", 'f', 16, ResultKind::Stacked)
        },
    ];
    let mut candidates = candidates.to_vec();
    candidates[0].path = source.join("M31/stacked/raw.tif");
    candidates[1].path = source.join("M31/stacked/dual-band.tif");
    let plans = plan_images(&night, &candidates, 12);
    let check = check_quotas(&storage(), &night, &plans);
    let queue = UploadQueue::open(&source.join("queue.sqlite")).unwrap();
    enqueue_checked(&queue, &night.session_id, &plans, &check)
        .unwrap()
        .unwrap();

    let transport =
        HttpUploadTransport::with_options(service.base_url(), fast(), WaitHooks::silent()).unwrap();
    // The variant first, before its capture exists: held, nothing sent.
    let variant = queue.find_by_sha256(&"f".repeat(64)).unwrap().unwrap();
    let error = upload_entry(
        &transport,
        &queue,
        "t",
        &variant,
        &UploadTarget::default(),
        |_, _| {},
    )
    .unwrap_err();
    assert_eq!(error.code(), "variant_capture_pending");
    assert_eq!(
        queue
            .find_by_sha256(&"f".repeat(64))
            .unwrap()
            .unwrap()
            .state,
        QueueState::Held
    );
    assert_eq!(creates.lock().unwrap().len(), 0);

    // In the queue's order: the first creates the capture, the second is its
    // variant.
    queue.release_held().unwrap();
    while let Some(entry) = queue.next_pending().unwrap() {
        upload_entry(
            &transport,
            &queue,
            "t",
            &entry,
            &UploadTarget::default(),
            |_, _| {},
        )
        .unwrap();
    }
    let bodies = creates.lock().unwrap().clone();
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0].get("captureId"), None);
    assert_eq!(bodies[0]["source"], "stacking");
    assert_eq!(bodies[1]["captureId"], "capture-new");
    let _ = std::fs::remove_dir_all(&source);
}

/// A progression image of `target`: `frames` light frames stacked.
fn milestone(target: &str, path: PathBuf, sha: char, frames: u64) -> ImageCandidate {
    ImageCandidate {
        render_rank: None,
        path,
        frame_count: Some(frames),
        twin_sha256: None,
        ..image(target, "", sha, 8, ResultKind::Progression)
    }
}

/// The progression images are the history of their target's capture --
/// the one this sending creates, or the one online -- never captures of
/// their own and never variants: they count against neither
/// `capture.count_limit` nor `capture.variant_limit`.
#[test]
fn progression_images_are_the_history_of_their_targets_capture() {
    let bare = online("night_bare.json");
    let stacked = image(
        "ic1805",
        "/n/IC 1805/stacked/raw.tif",
        'e',
        46,
        ResultKind::Stacked,
    );
    let milestones: Vec<ImageCandidate> = [(1, '1'), (23, '2'), (45, '3'), (68, '4'), (90, '5')]
        .into_iter()
        .map(|(frames, sha)| {
            milestone(
                "ic1805",
                PathBuf::from(format!("/n/IC 1805/progression/milestone_0{sha}.jpg")),
                sha,
                frames,
            )
        })
        .collect();
    let mut candidates = vec![stacked.clone()];
    candidates.extend(milestones.iter().cloned());
    // Without variants at all: the history still goes with the capture.
    let plans = plan_images(&bare, &candidates, 0);
    let mut expected = vec![Decision::NewCapture];
    expected.extend(std::iter::repeat_n(
        Decision::ProgressionOfNew {
            first_sha256: "e".repeat(64),
        },
        5,
    ));
    assert_eq!(decisions(&plans), expected);
    let check = check_quotas(&storage(), &bare, &plans);
    assert!(check.allowed, "{:?}", check.findings);
    assert_eq!(check.new_captures, 1);
    assert_eq!(check.to_send_files, 6);
    let mut tight = storage();
    tight.variant_limit = 0;
    tight.capture_count_limit = tight.capture_count + 1;
    let check = check_quotas(&tight, &bare, &plans);
    assert!(check.allowed, "{:?}", check.findings);

    // The capture online when the night already has one (M31).
    let in_progress = online("night_in_progress.json");
    let plans = plan_images(
        &in_progress,
        &[milestone(
            "m31",
            PathBuf::from("/n/M31/progression/milestone_01.jpg"),
            '9',
            1,
        )],
        0,
    );
    assert_eq!(
        decisions(&plans),
        [Decision::Progression {
            capture_id: M31_CAPTURE.into()
        }]
    );
    // A target with no capture anywhere: nothing to be the history of.
    let plans = plan_images(
        &bare,
        &[milestone(
            "ic1396",
            PathBuf::from("/n/IC 1396/progression/milestone_01.jpg"),
            '8',
            1,
        )],
        12,
    );
    assert_eq!(
        decisions(&plans),
        [Decision::NotSent {
            reason: NotSent::NoCaptureForProgression
        }]
    );

    // The service says the stacked image is already online: the history
    // goes to that capture.
    let mut plans = plan_images(&bare, &candidates, 12);
    plans[0].decision = Decision::AlreadyOnline {
        capture_id: "capture-online".into(),
        image_id: "image-online".into(),
    };
    rebind_progressions(&mut plans);
    assert!(plans[1..].iter().all(|p| p.decision
        == Decision::Progression {
            capture_id: "capture-online".into()
        }));
}

/// The milestones leave after the image that creates the capture, each
/// with its `captureId`, the role `progression` and its frames.
#[test]
fn the_milestones_leave_after_their_capture_with_their_role_and_frames() {
    let source = std::env::temp_dir().join(format!("astro-sync-history-{}", std::process::id()));
    std::fs::create_dir_all(source.join("IC 1805/stacked")).unwrap();
    std::fs::create_dir_all(source.join("IC 1805/progression")).unwrap();
    std::fs::write(source.join("IC 1805/stacked/raw.tif"), vec![1_u8; 16]).unwrap();
    for n in 1..=2 {
        std::fs::write(
            source.join(format!("IC 1805/progression/milestone_0{n}.jpg")),
            vec![n; 8],
        )
        .unwrap();
    }
    let creates = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
    let seen = Arc::clone(&creates);
    let service = FakeService::start(Wake::Instant, move |request| {
        if request.method == "POST" && request.path == "/api/v1/uploads" {
            let body = request.json();
            let size = body["byteSize"].as_u64().unwrap_or(0);
            seen.lock().unwrap().push(body);
            return Reply::Json(
                200,
                format!(
                    r#"{{"uploadId":"u-1","offset":{size},"byteSize":{size},"chunkSize":8388608,"completed":true,"captureId":"capture-ic1805"}}"#
                ),
            );
        }
        error_reply(404, "not_found", "no such route")
    });
    let night = online("night_bare.json");
    // The milestones come first in the candidates: the queue puts them last.
    let candidates = vec![
        milestone(
            "ic1805",
            source.join("IC 1805/progression/milestone_01.jpg"),
            '1',
            1,
        ),
        milestone(
            "ic1805",
            source.join("IC 1805/progression/milestone_02.jpg"),
            '2',
            90,
        ),
        ImageCandidate {
            render_rank: None,
            path: source.join("IC 1805/stacked/raw.tif"),
            ..image("ic1805", "", 'e', 16, ResultKind::Stacked)
        },
    ];
    let plans = plan_images(&night, &candidates, 0);
    let check = check_quotas(&storage(), &night, &plans);
    let queue = UploadQueue::open(&source.join("queue.sqlite")).unwrap();
    enqueue_checked(&queue, &night.session_id, &plans, &check)
        .unwrap()
        .unwrap();
    let transport =
        HttpUploadTransport::with_options(service.base_url(), fast(), WaitHooks::silent()).unwrap();
    while let Some(entry) = queue.next_pending().unwrap() {
        upload_entry(
            &transport,
            &queue,
            "t",
            &entry,
            &UploadTarget::default(),
            |_, _| {},
        )
        .unwrap();
    }
    let bodies = creates.lock().unwrap().clone();
    assert_eq!(bodies.len(), 3);
    assert_eq!(bodies[0]["originalName"], "raw.tif");
    assert_eq!(bodies[0].get("role"), None);
    assert_eq!(bodies[0].get("captureId"), None);
    for (body, frames) in bodies[1..].iter().zip([1, 90]) {
        assert_eq!(body["captureId"], "capture-ic1805");
        assert_eq!(body["role"], "progression");
        assert_eq!(body["progressionFrames"], frames);
        assert_eq!(body["source"], "astro-sync");
    }
    let _ = std::fs::remove_dir_all(&source);
}

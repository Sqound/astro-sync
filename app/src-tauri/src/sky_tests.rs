//! The sky annotations after an upload, through the app's own wiring: a file
//! that reaches the service is followed by its annotations on a thread of its
//! own, the result is told to the window (`astro-sync://sky-event`) and kept
//! beside the queue's row, and « Réessayer » (`sky_retry`) runs it again. What
//! is solved and sent is the core's (`sky_after_upload`); here the service
//! answers the listing only.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use astro_sync_core::dry_run::build_dry_run_report;
use astro_sync_core::scanner::scan_folder;
use astro_sync_core::sky_after_upload::SkyState;
use astro_sync_core::upload_queue::UploadQueue;

use crate::wake_tests::fake_service::{error_reply, FakeService, Reply, UploadModel, Wake};
use crate::wake_tests::{fast, harness_with_sky, serialized, unique_dir, Harness};

/// Waits until the window has been told `count` sky events.
fn sky_events(harness: &Harness, count: usize) -> Vec<serde_json::Value> {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let events = harness.sky_events.lock().unwrap().clone();
        if events.len() >= count || Instant::now() > deadline {
            return events;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn a_file_sent_is_followed_by_its_annotations_and_they_can_be_tried_again() {
    let _serial = serialized();
    let harness = harness_with_sky(fast());
    let model = Arc::new(UploadModel::default());
    let app_model = Arc::clone(&model);
    // The service is down for the listing at first, then answers: nothing is
    // missing (the capture already has its solution).
    let up = Arc::new(AtomicBool::new(false));
    let service_up = Arc::clone(&up);
    let service = FakeService::start(Wake::Instant, move |request| {
        if request.method == "GET" && request.path.starts_with("/api/v1/captures") {
            return if service_up.load(Ordering::SeqCst) {
                Reply::Json(
                    200,
                    r#"{"captures":[],"hasMore":false,"nextSince":null}"#.to_string(),
                )
            } else {
                error_reply(404, "not_found", "no such route yet")
            };
        }
        app_model
            .reply(request, 100)
            .unwrap_or_else(|| error_reply(404, "not_found", "no such route"))
    });
    harness.pair_with(&service.base_url());

    let source = unique_dir("source");
    std::fs::write(source.join("IC_1805_plan_raw.tif"), vec![7_u8; 250]).unwrap();
    let queue_path = harness.config_dir.join("upload-queue.sqlite");
    let _ = std::fs::remove_file(&queue_path);
    let queue = UploadQueue::open(&queue_path).unwrap();
    queue
        .enqueue_from_dry_run(&build_dry_run_report(&scan_folder(&source).unwrap()))
        .unwrap();
    let sha256 = queue.list().unwrap()[0].sha256.clone();
    drop(queue);

    let processed = harness
        .invoke(
            "upload_all",
            serde_json::json!({"sessionId": null, "planId": null, "targetIds": []}),
        )
        .answer(Duration::from_secs(8))
        .expect("the queue drains");
    assert_eq!(processed, 1);

    // The upload is done and the next file never waited: the annotations
    // follow on their own, « running » then the failure to ask the service.
    let events = sky_events(&harness, 2);
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[0]["state"], "running");
    assert_eq!(events[0]["sha256"], sha256);
    assert_eq!(events[1]["state"], "failed");
    assert!(events[1]["errorCode"].as_str().is_some(), "{events:?}");
    let kept = UploadQueue::open(&queue_path)
        .unwrap()
        .sky_of(&sha256)
        .unwrap()
        .unwrap();
    assert_eq!(kept.state, SkyState::Failed);
    assert!(kept.capture_id.is_some());

    // « Réessayer »: the service answers this time, nothing is missing.
    up.store(true, Ordering::SeqCst);
    // (The mock keyring reads the token from its file once: pair again.)
    harness.pair_with(&service.base_url());
    harness
        .invoke("sky_retry", serde_json::json!({ "sha256": sha256 }))
        .answer(Duration::from_secs(8))
        .expect("the retry is started");
    let events = sky_events(&harness, 5);
    let last = events.last().unwrap();
    assert_eq!(last["state"], "already", "{events:?}");
    assert_eq!(
        UploadQueue::open(&queue_path)
            .unwrap()
            .sky_of(&sha256)
            .unwrap()
            .unwrap()
            .state,
        SkyState::Already
    );
    let _ = std::fs::remove_dir_all(&source);
    drop(harness.app);
}

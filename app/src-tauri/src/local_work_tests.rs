//! The local commands run off
//! the thread that received the request (the main thread of the window),
//! report their progress and stop on « Arrêter »; the UTC offset of the logs
//! is a setting of the window. Driven through Tauri's own IPC on its mock
//! runtime, like `wake_tests.rs`, in the same throwaway home.

use std::time::Duration;

use crate::commands::LOCAL_WORK_EVENT;
use crate::wake_tests::{fast, harness, serialized, unique_dir};
use std::sync::{Arc, Mutex};
use tauri::Listener;

/// A folder heavy enough that hashing it takes a while in a debug build.
fn heavy_folder(label: &str, files: usize, megabytes: usize) -> std::path::PathBuf {
    let dir = unique_dir(label);
    for index in 0..files {
        std::fs::write(
            dir.join(format!("Light_{index:04}.fits")),
            vec![index as u8; megabytes * 1024 * 1024],
        )
        .unwrap();
    }
    dir
}

#[test]
fn a_scan_runs_off_the_request_thread_and_reports_its_progress() {
    let _serial = serialized();
    let harness = harness(fast());
    let events: Arc<Mutex<Vec<serde_json::Value>>> = Arc::default();
    let sink = Arc::clone(&events);
    harness.app.listen(LOCAL_WORK_EVENT, move |event| {
        sink.lock()
            .unwrap()
            .push(serde_json::from_str(event.payload()).unwrap());
    });
    let folder = heavy_folder("local-scan", 6, 4);

    let pending = harness.invoke(
        "scan_folder",
        serde_json::json!({ "folder": folder.to_string_lossy() }),
    );
    assert!(
        pending.held_the_caller_for < Duration::from_millis(150),
        "the scan held the request thread for {:?}: the window would freeze",
        pending.held_the_caller_for
    );
    let report = pending
        .answer(Duration::from_secs(60))
        .expect("the scan answers");
    assert_eq!(report["files"].as_array().unwrap().len(), 6);
    let events = events.lock().unwrap();
    assert!(events.iter().all(|e| e["kind"] == "scan"));
    assert_eq!(events.last().unwrap()["done"], 6);
    assert_eq!(events.last().unwrap()["total"], 6);
    drop(harness.app);
}

#[test]
fn a_scan_stops_on_the_cancel_command() {
    let _serial = serialized();
    let harness = harness(fast());
    let folder = heavy_folder("local-cancel", 40, 4);

    let pending = harness.invoke(
        "scan_folder",
        serde_json::json!({ "folder": folder.to_string_lossy() }),
    );
    // « Arrêter », while the scan reads its first files.
    std::thread::sleep(Duration::from_millis(30));
    harness
        .invoke("local_work_cancel", serde_json::json!({}))
        .answer(Duration::from_secs(5))
        .expect("cancel answers");
    let error = pending
        .answer(Duration::from_secs(60))
        .expect_err("a stopped scan answers with an error");
    assert_eq!(error["code"], "cancelled");

    // The next work arms the switch again.
    let small = unique_dir("local-after-cancel");
    std::fs::write(small.join("a.fits"), b"x").unwrap();
    let report = harness
        .invoke(
            "scan_folder",
            serde_json::json!({ "folder": small.to_string_lossy() }),
        )
        .answer(Duration::from_secs(10))
        .expect("a new scan runs");
    assert_eq!(report["files"].as_array().unwrap().len(), 1);
    drop(harness.app);
}

/// The contrast of presentation is a local setting, `none` by default,
/// kept between two runs of the window and refused when it is not one of the
/// three words.
#[test]
fn the_contrast_of_presentation_is_a_local_setting() {
    let _serial = serialized();
    let harness = harness(fast());
    let _ = std::fs::remove_file(harness.config_dir.join("local-settings.json"));

    let fresh = harness
        .invoke("local_settings_get", serde_json::json!({}))
        .answer(Duration::from_secs(5))
        .expect("the settings are read");
    // « Présentation 2,0 » until chosen, GraXpert for the
    // « Rendu soigné ».
    assert_eq!(fresh["contrast"], "presentation");
    assert_eq!(fresh["polishedEngine"], "graxpert-ai");
    // The finish asks for nothing more until chosen.
    assert_eq!(fresh["colour"], 0);
    assert_eq!(fresh["softStars"], 0);
    let siril = harness
        .invoke(
            "local_settings_set_polished_engine",
            serde_json::json!({ "engine": "siril" }),
        )
        .answer(Duration::from_secs(5))
        .expect("a known engine is kept");
    assert_eq!(siril["polishedEngine"], "siril");
    harness
        .invoke(
            "local_settings_set_polished_engine",
            serde_json::json!({ "engine": "magic" }),
        )
        .answer(Duration::from_secs(5))
        .expect_err("an unknown engine is refused");

    let kept = harness
        .invoke(
            "local_settings_set_contrast",
            serde_json::json!({ "contrast": "strong" }),
        )
        .answer(Duration::from_secs(5))
        .expect("a known contrast is kept");
    assert_eq!(kept["contrast"], "strong");
    let again = harness
        .invoke("local_settings_get", serde_json::json!({}))
        .answer(Duration::from_secs(5))
        .expect("the settings are read again");
    assert_eq!(again["contrast"], "strong");

    harness
        .invoke(
            "local_settings_set_contrast",
            serde_json::json!({ "contrast": "extreme" }),
        )
        .answer(Duration::from_secs(5))
        .expect_err("an unknown contrast is refused");
    let unchanged = harness
        .invoke("local_settings_get", serde_json::json!({}))
        .answer(Duration::from_secs(5))
        .expect("the settings are read once more");
    assert_eq!(unchanged["contrast"], "strong");
    harness
        .invoke(
            "local_settings_set_contrast",
            serde_json::json!({ "contrast": "none" }),
        )
        .answer(Duration::from_secs(5))
        .expect("back to none");

    // The three settings are continuous: any value in their range is kept as it is (the two
    // contrast words the window always offered stay words), and refused outside it.
    let custom = harness
        .invoke(
            "local_settings_set_contrast",
            serde_json::json!({ "contrast": 2.5 }),
        )
        .answer(Duration::from_secs(5))
        .expect("a gain between the detents is kept");
    assert_eq!(custom["contrast"], 2.5);
    let word = harness
        .invoke(
            "local_settings_set_contrast",
            serde_json::json!({ "contrast": 3.6 }),
        )
        .answer(Duration::from_secs(5))
        .expect("a gain on a detent is kept");
    assert_eq!(word["contrast"], "strong");
    harness
        .invoke(
            "local_settings_set_contrast",
            serde_json::json!({ "contrast": 7 }),
        )
        .answer(Duration::from_secs(5))
        .expect_err("a gain above the range is refused");
    let colour = harness
        .invoke(
            "local_settings_set_colour",
            serde_json::json!({ "colour": 35 }),
        )
        .answer(Duration::from_secs(5))
        .expect("a colour intensity in its range is kept");
    assert_eq!(colour["colour"], 35);
    let stars = harness
        .invoke(
            "local_settings_set_soft_stars",
            serde_json::json!({ "softStars": 70 }),
        )
        .answer(Duration::from_secs(5))
        .expect("a star softening in its range is kept");
    assert_eq!(stars["softStars"], 70);
    assert_eq!(stars["colour"], 35);
    let again = harness
        .invoke("local_settings_get", serde_json::json!({}))
        .answer(Duration::from_secs(5))
        .expect("the settings are read again");
    assert_eq!(
        (again["colour"].clone(), again["softStars"].clone()),
        (35.into(), 70.into())
    );
    for (command, key, bad) in [
        ("local_settings_set_colour", "colour", 101),
        ("local_settings_set_soft_stars", "softStars", 101),
        ("local_settings_set_colour", "colour", -1),
    ] {
        harness
            .invoke(command, serde_json::json!({ key: bad }))
            .answer(Duration::from_secs(5))
            .expect_err("a value outside 0..100 is refused");
    }
    harness
        .invoke(
            "local_settings_set_colour",
            serde_json::json!({ "colour": "vivid" }),
        )
        .answer(Duration::from_secs(5))
        .expect_err("an unknown word is refused");
    for (command, key) in [
        ("local_settings_set_colour", "colour"),
        ("local_settings_set_soft_stars", "softStars"),
    ] {
        harness
            .invoke(command, serde_json::json!({ key: 0 }))
            .answer(Duration::from_secs(5))
            .expect("back to none");
    }
}

#[test]
fn the_utc_offset_of_the_logs_is_a_setting_the_analysis_reads() {
    let _serial = serialized();
    let harness = harness(fast());
    let _ = std::fs::remove_file(harness.config_dir.join("local-settings.json"));

    let refused = harness
        .invoke(
            "local_settings_set_utc_offsets",
            serde_json::json!({ "phd2UtcOffset": "UTC+9", "asiairUtcOffset": null }),
        )
        .answer(Duration::from_secs(5))
        .expect_err("a bad offset is refused");
    assert_eq!(refused["code"], "bad_utc_offset");

    let settings = harness
        .invoke(
            "local_settings_set_utc_offsets",
            serde_json::json!({ "phd2UtcOffset": "+09:00", "asiairUtcOffset": "+09:00" }),
        )
        .answer(Duration::from_secs(5))
        .expect("a good offset is kept");
    assert_eq!(settings["phd2UtcOffset"], "+09:00");

    // A PHD2 log at 21:00 local time on a clock of UTC+9 is 12:00 UTC.
    let folder = unique_dir("local-offset");
    std::fs::write(
        folder.join("PHD2_GuideLog_2026-10-07_210000.txt"),
        include_str!("../../../crates/astro-sync-core/tests/fixtures/phd2_guide_log_short.txt"),
    )
    .unwrap();
    let report = harness
        .invoke(
            "night_analysis",
            serde_json::json!({ "folder": folder.to_string_lossy() }),
        )
        .answer(Duration::from_secs(20))
        .expect("the analysis runs");
    let text = report.to_string();
    let without = harness.invoke(
        "local_settings_set_utc_offsets",
        serde_json::json!({ "phd2UtcOffset": null, "asiairUtcOffset": null }),
    );
    without.answer(Duration::from_secs(5)).unwrap();
    let utc = harness
        .invoke(
            "night_analysis",
            serde_json::json!({ "folder": folder.to_string_lossy() }),
        )
        .answer(Duration::from_secs(20))
        .expect("the analysis runs");
    assert_ne!(
        text,
        utc.to_string(),
        "the offset of the settings changes the instants of the report"
    );
    drop(harness.app);
}

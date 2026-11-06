//! `processing_files_scan` and `processing_files_clean` driven through
//! Tauri's IPC, in the throwaway home of `wake_tests.rs`.

use std::time::Duration;

use astro_sync_core::local_work::WorkKind;
use tauri::Manager;

use crate::state::LocalWorkControl;
use crate::wake_tests::{fast, harness, serialized, unique_dir};

fn write(path: &std::path::Path, bytes: usize) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, vec![b'x'; bytes]).unwrap();
}

#[test]
fn the_working_folder_is_scanned_cleaned_and_refused_while_a_stacking_runs() {
    let _serial = serialized();
    let harness = harness(fast());
    let _ = std::fs::remove_file(harness.config_dir.join("local-settings.json"));
    let root = unique_dir("processing-commands");
    let call = |command: &str, args: serde_json::Value| {
        harness.invoke(command, args).answer(Duration::from_secs(5))
    };
    call(
        "working_folder_set",
        serde_json::json!({ "folder": root.to_string_lossy() }),
    )
    .unwrap();

    let night = root.join("2026-10-07 · Col · aaaaaa");
    write(&night.join(".astro-sync-night.json"), 10);
    write(&night.join("M31/.astro-sync-target.json"), 10);
    write(&night.join("M31/lights/Light_0001.fits"), 1_000);
    write(&night.join("M31/stacked/M31_natural.tif"), 3_000);
    write(&night.join("M31/stacked/IRCUT_30s_integration.fit"), 4_000);
    write(&night.join(".astro-sync-work/stacking/job/frame"), 50);

    let scan = call("processing_files_scan", serde_json::json!({})).unwrap();
    assert_eq!(scan["reachable"], true);
    assert_eq!(scan["userBytes"], 1_000);
    let kinds: Vec<&str> = scan["categories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|category| category["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["masters", "nightResults", "planRenders", "temp"]);
    assert_eq!(scan["categories"][0]["bytes"], 4_000);
    // No row in the queue: the render counts as not sent.
    assert_eq!(scan["categories"][1]["unsentBytes"], 3_000);
    assert_eq!(scan["categories"][3]["bytes"], 50);

    // A stacking is running: nothing is deleted.
    let stacking = harness
        .app
        .state::<LocalWorkControl>()
        .arm_kind(WorkKind::Stacking);
    let refused = call(
        "processing_files_clean",
        serde_json::json!({ "kinds": ["masters", "unsentResults", "temp"] }),
    )
    .unwrap_err();
    assert_eq!(refused["code"], "processing_files_busy");
    assert!(night.join("M31/stacked/M31_natural.tif").is_file());
    drop(stacking);

    // Only the masters and the temporaries are asked for.
    let cleaned = call(
        "processing_files_clean",
        serde_json::json!({ "kinds": ["masters", "temp"] }),
    )
    .unwrap();
    assert_eq!(cleaned["freedBytes"], 4_050);
    assert!(cleaned["failed"].as_array().unwrap().is_empty());
    assert!(night.join("M31/stacked/M31_natural.tif").is_file());
    assert!(night.join("M31/lights/Light_0001.fits").is_file());
    assert!(!night.join(".astro-sync-work").exists());

    // The folder goes away (a disk unplugged): not reachable, no error.
    std::fs::remove_dir_all(&root).unwrap();
    let scan = call("processing_files_scan", serde_json::json!({})).unwrap();
    assert_eq!(scan["reachable"], false);
    let refused = call(
        "processing_files_clean",
        serde_json::json!({ "kinds": ["masters"] }),
    )
    .unwrap_err();
    assert_eq!(refused["code"], "folder_not_found");
}

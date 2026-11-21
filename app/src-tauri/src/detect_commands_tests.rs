//! « Détecter les sessions » and « Ranger les fichiers » driven
//! through Tauri's IPC on its mock runtime, as the window will call them.

use std::time::Duration;

use crate::wake_tests::{fast, harness, serialized, unique_dir};

/// A FITS primary header with these cards, padded to a block.
fn fits(cards: &[(&str, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (keyword, value) in cards.iter().chain([("END", "")].iter()) {
        let mut card = if *keyword == "END" {
            b"END".to_vec()
        } else {
            format!("{keyword:<8}= {value}").into_bytes()
        };
        card.resize(80, b' ');
        out.extend_from_slice(&card);
    }
    out.resize(out.len().div_ceil(2880) * 2880, b' ');
    out
}

#[test]
fn detect_then_organise_through_the_window_commands() {
    let _serial = serialized();
    let harness = harness(fast());
    // The tests share one HOME: a test before this one may have left a
    // service address and a device token there (`pair_with`), and the
    // detection would then read the service — a fake one long gone, or
    // answering for another test. This one runs signed out, on the window's
    // own data.
    let _ = std::fs::remove_file(harness.config_dir.join("settings.json"));
    let root = unique_dir("detect-commands");
    let lights = root.join("fits").join("lights");
    std::fs::create_dir_all(&lights).unwrap();
    for (i, time) in ["2026-08-17T22:00:00", "2026-08-17T22:03:10"]
        .iter()
        .enumerate()
    {
        let date = format!("'{time}'");
        std::fs::write(
            lights.join(format!("l{i}.fit")),
            fits(&[
                ("IMAGETYP", "'Light'"),
                ("OBJECT", "'NGC 7000'"),
                ("DATE-OBS", &date),
                ("EXPTIME", "180."),
                ("SITELAT", "43.88"),
                ("SITELONG", "7.43"),
            ]),
        )
        .unwrap();
    }
    let call = |command: &str, args: serde_json::Value| {
        harness
            .invoke(command, args)
            .answer(Duration::from_secs(10))
            .unwrap_or_else(|error| panic!("{command}: {error:?}"))
    };
    let proposal = call(
        "sessions_detect",
        serde_json::json!({
            "request": { "fits": [root.join("fits")] },
            "user": { "account": { "advancedPlanning": true } },
            "catalogue": []
        }),
    );
    assert_eq!(proposal["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(proposal["sessions"][0]["eveningDate"], "2026-08-17");
    assert_eq!(proposal["sessions"][0]["targets"][0]["frames"], 2);
    assert_eq!(proposal["reading"]["lights"], 2);

    let destination = root.join("organised");
    let plan = call(
        "sessions_organise_plan",
        serde_json::json!({
            "proposal": proposal,
            "destination": destination,
            "mode": "copy",
            // The result page names each night by its session created.
            "names": { proposal["sessions"][0]["id"].as_str().unwrap(): {
                "place": "Maison",
                "sessionId": "3e7a91c0-0000-4000-8000-000000000001"
            } }
        }),
    );
    assert_eq!(plan["files"], 2);
    assert!(!destination.exists(), "the summary writes nothing");
    let outcome = call("sessions_organise", serde_json::json!({ "plan": plan }));
    assert_eq!(outcome["done"], 2);
    assert!(destination
        .join("2026-08-17 · Maison · 3e7a91")
        .join("NGC 7000")
        .join("input")
        .join("lights")
        .join("l0.fit")
        .is_file());
    assert!(lights.join("l0.fit").is_file(), "a copy leaves the source");

    // The folder of FITS is required: the window says why.
    let refused = harness
        .invoke(
            "sessions_detect",
            serde_json::json!({ "request": { "fits": [] } }),
        )
        .answer(Duration::from_secs(5))
        .unwrap_err();
    assert!(format!("{refused:?}").contains("no_fits_folder"));
}

/// The window gives the places, equipment, plans and rights it read a moment
/// before (`sessions_user_data`): the detection reads only the sessions of
/// the dates it found -- not the rest again, nor the capabilities -- and says
/// what it is doing (`detect-phase`), never a count that stays at 0 while the
/// service answers (the smoke on Windows: ~30 s on « 0 found » for 5 files).
#[test]
fn a_detection_given_the_person_s_data_reads_only_the_sessions_and_says_its_phases() {
    use std::sync::{Arc, Mutex};
    use tauri::Listener;

    use crate::wake_tests::fake_service::{FakeService, Wake};

    let _serial = serialized();
    let harness = harness(fast());
    let service = FakeService::start(Wake::Instant, crate::wake_tests::portal_app);
    harness.pair_with(&service.base_url());
    let phases: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = Arc::clone(&phases);
    harness
        .app
        .listen(crate::commands_detect::DETECT_PHASE_EVENT, move |event| {
            let phase: String = serde_json::from_str(event.payload()).unwrap();
            sink.lock().unwrap().push(phase);
        });
    let root = unique_dir("detect-phases");
    let lights = root.join("fits").join("lights");
    std::fs::create_dir_all(&lights).unwrap();
    std::fs::write(
        lights.join("l0.fit"),
        fits(&[
            ("IMAGETYP", "'Light'"),
            ("OBJECT", "'NGC 7000'"),
            ("DATE-OBS", "'2026-08-17T22:00:00'"),
            ("EXPTIME", "180."),
        ]),
    )
    .unwrap();

    let proposal = harness
        .invoke(
            "sessions_detect",
            serde_json::json!({
                "request": { "fits": [root.join("fits")] },
                "user": { "account": { "advancedPlanning": true } },
                "catalogue": []
            }),
        )
        .answer(Duration::from_secs(10))
        .unwrap();
    assert_eq!(proposal["reading"]["lights"], 1);

    for route in [
        "/api/v1/places",
        "/api/v1/equipment",
        "/api/v1/plans",
        "/api/v1/capabilities",
    ] {
        assert_eq!(service.app_hits("GET", route), 0, "{route} read again");
    }
    assert!(service.app_hits("GET", "/api/v1/sessions") >= 1);
    assert_eq!(*phases.lock().unwrap(), ["files", "sessions"]);
    let _ = std::fs::remove_dir_all(&root);
    drop(harness.app);
}

/// « Mise en file des images… » says where it is: each image as it is
/// prepared, of how many, then the rows written. Without it the window showed
/// one sentence for the whole wait.
#[test]
fn queuing_the_images_of_an_import_says_which_one_it_prepares() {
    use tauri::Listener;

    let _serial = serialized();
    let harness = harness(fast());
    // Signed out: nothing is asked of a service (no « check » step).
    let _ = std::fs::remove_file(harness.config_dir.join("settings.json"));
    let steps: std::sync::Arc<std::sync::Mutex<Vec<crate::commands_detect::ImportProgressStep>>> =
        std::sync::Arc::default();
    let sink = std::sync::Arc::clone(&steps);
    harness.app.listen(
        crate::commands_detect::IMPORT_PROGRESS_EVENT,
        move |event| {
            sink.lock()
                .unwrap()
                .push(serde_json::from_str(event.payload()).unwrap());
        },
    );
    let root = unique_dir("images-send-steps");
    let stacked = root.join("night").join("M45").join("stacked");
    std::fs::create_dir_all(&stacked).unwrap();
    let image = |name: &str, digest: &str, of: Option<&str>| {
        let path = stacked.join(name);
        std::fs::write(&path, digest).unwrap();
        serde_json::json!({
            "kind": "night",
            "sessionId": "session-1",
            "targetId": "m45",
            "render": { "pipelineId": "raw", "path": path, "sha256": digest, "size": digest.len() },
            "variantOfSha256": of,
        })
    };
    let first = "a".repeat(64);
    let second = "b".repeat(64);
    let outcome = harness
        .invoke(
            "detect_images_send",
            serde_json::json!({ "images": [
                image("M45_raw.png", &first, None),
                image("M45_polished.png", &second, Some(&first)),
            ] }),
        )
        .answer(Duration::from_secs(20))
        .unwrap();
    assert_eq!(outcome["inserted"], 2);
    let said: Vec<(String, Option<usize>, Option<usize>)> = steps
        .lock()
        .unwrap()
        .iter()
        .map(|s| (s.step.clone(), s.done, s.total))
        .collect();
    assert_eq!(
        said,
        [
            ("prepare".to_string(), Some(1), Some(2)),
            ("prepare".to_string(), Some(2), Some(2)),
            ("queue".to_string(), None, Some(2)),
        ]
    );
    let _ = std::fs::remove_dir_all(&root);
    drop(harness.app);
}

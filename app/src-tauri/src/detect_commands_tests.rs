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

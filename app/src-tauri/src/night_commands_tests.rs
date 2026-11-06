//! The commands the screens call, driven through Tauri's IPC on its
//! mock runtime in the throwaway home of `wake_tests.rs`.

use std::time::Duration;

use crate::wake_tests::{fast, harness, serialized, unique_dir};

/// `working_folder_prepare` (called at startup and before the copy of the
/// prompt) writes `ASSISTANT-IA.md` again when it is missing and keeps a
/// file the person changed.
#[test]
fn working_folder_prepare_keeps_the_assistant_file_up_to_date() {
    let _serial = serialized();
    let harness = harness(fast());
    let _ = std::fs::remove_file(harness.config_dir.join("local-settings.json"));
    let root = unique_dir("night-commands-prepare");
    let call = |command: &str| {
        harness
            .invoke(command, serde_json::json!({}))
            .answer(Duration::from_secs(5))
            .unwrap_or_else(|error| panic!("{command}: {error:?}"))
    };
    harness
        .invoke(
            "working_folder_set",
            serde_json::json!({ "folder": root.to_string_lossy() }),
        )
        .answer(Duration::from_secs(5))
        .unwrap();
    let file = root.join("ASSISTANT-IA.md");

    let again = call("working_folder_prepare");
    assert_eq!(again["assistantFile"]["outcome"], "unchanged");

    std::fs::remove_file(&file).unwrap();
    let again = call("working_folder_prepare");
    assert_eq!(again["assistantFile"]["outcome"], "created");
    assert!(file.is_file());

    std::fs::write(&file, "mine").unwrap();
    let again = call("working_folder_prepare");
    assert_eq!(again["assistantFile"]["outcome"], "kept_as_changed");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "mine");

    // « Remplacer par la version d'Astro Sync »: the copy is kept beside it.
    let replaced = call("working_folder_replace_assistant_file");
    let copy = std::path::PathBuf::from(replaced["keptCopy"].as_str().unwrap());
    assert!(copy
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("ASSISTANT-IA.modifie-"));
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "mine");
    assert_eq!(
        call("working_folder_prepare")["assistantFile"]["outcome"],
        "unchanged"
    );
}

/// « Poser une question à une IA ». The README is made
/// ready with the folder (in the application's language, under the rule of
/// `ASSISTANT-IA.md`), the guide is written on a click -- on every click the
/// fingerprints are compared, so a file the person changed is kept and said --
/// and the prompt gives the guide's full path, or none when it was not written.
#[test]
fn the_guide_and_the_readme_follow_the_rule_of_the_assistant_file() {
    let _serial = serialized();
    let harness = harness(fast());
    let _ = std::fs::remove_file(harness.config_dir.join("local-settings.json"));
    let root = unique_dir("night-commands-guide");
    let call = |command: &str, args: serde_json::Value| {
        harness
            .invoke(command, args)
            .answer(Duration::from_secs(5))
            .unwrap_or_else(|error| panic!("{command}: {error:?}"))
    };
    let prepared = call(
        "working_folder_set",
        serde_json::json!({ "folder": root.to_string_lossy(), "language": "fr" }),
    );
    assert_eq!(prepared["readme"]["outcome"], "created");
    let readme = root.join("README.md");
    assert!(std::fs::read_to_string(&readme)
        .unwrap()
        .starts_with("# Astro Sync\u{a0}: votre dossier de travail"));

    // Without a language the folder is made ready, the README left alone.
    let plain = call("working_folder_prepare", serde_json::json!({}));
    assert!(plain["readme"].is_null());

    // A change of the application's language rewrites the README (it is
    // still Astro Sync's text).
    let english = call(
        "working_folder_prepare",
        serde_json::json!({ "language": "en" }),
    );
    assert_eq!(english["readme"]["outcome"], "rewritten");

    // The guide: created, then unchanged, then rewritten when the
    // application's text moved on.
    let first = call(
        "guide_write",
        serde_json::json!({ "language": "fr", "text": "# Guide\n" }),
    );
    assert_eq!(first["guide"]["outcome"], "created");
    assert_eq!(first["guide"]["name"], "GUIDE-ASTRO-SYNC.fr.md");
    let guide = root.join("GUIDE-ASTRO-SYNC.fr.md");
    assert_eq!(std::fs::read_to_string(&guide).unwrap(), "# Guide\n");
    let again = call(
        "guide_write",
        serde_json::json!({ "language": "fr", "text": "# Guide\n" }),
    );
    assert_eq!(again["guide"]["outcome"], "unchanged");
    let moved = call(
        "guide_write",
        serde_json::json!({ "language": "fr", "text": "# Guide 2\n" }),
    );
    assert_eq!(moved["guide"]["outcome"], "rewritten");

    // The person changed the guide and the README: both kept, said.
    std::fs::write(&guide, "mine").unwrap();
    std::fs::write(&readme, "mine too").unwrap();
    let kept = call(
        "guide_write",
        serde_json::json!({ "language": "fr", "text": "# Guide 2\n" }),
    );
    assert_eq!(kept["guide"]["outcome"], "kept_as_changed");
    assert_eq!(std::fs::read_to_string(&guide).unwrap(), "mine");
    let kept = call(
        "working_folder_prepare",
        serde_json::json!({ "language": "en" }),
    );
    assert_eq!(kept["readme"]["outcome"], "kept_as_changed");
    assert_eq!(std::fs::read_to_string(&readme).unwrap(), "mine too");

    // « Remplacer par la version d'Astro Sync »: the changed copies are kept.
    let replaced = call(
        "managed_file_replace",
        serde_json::json!({ "kind": "guide", "language": "fr", "text": "# Guide 2\n" }),
    );
    let copy = std::path::PathBuf::from(replaced["keptCopy"].as_str().unwrap());
    assert!(copy
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("GUIDE-ASTRO-SYNC.fr.modifie-"));
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "mine");
    assert_eq!(std::fs::read_to_string(&guide).unwrap(), "# Guide 2\n");
    let replaced = call(
        "managed_file_replace",
        serde_json::json!({ "kind": "readme", "language": "en" }),
    );
    let copy = std::path::PathBuf::from(replaced["keptCopy"].as_str().unwrap());
    assert!(copy
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("README.modifie-"));
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "mine too");
    assert!(std::fs::read_to_string(&readme)
        .unwrap()
        .starts_with("# Astro Sync: your working folder"));
    // Both are unchanged on the next click.
    assert_eq!(
        call(
            "working_folder_prepare",
            serde_json::json!({ "language": "en" })
        )["readme"]["outcome"],
        "unchanged"
    );
    assert_eq!(
        call(
            "guide_write",
            serde_json::json!({ "language": "fr", "text": "# Guide 2\n" })
        )["guide"]["outcome"],
        "unchanged"
    );

    // The prompt: the full path, or none when the guide was not written.
    let with_path = call(
        "guide_prompt",
        serde_json::json!({
            "language": "fr",
            "input": { "root": root.to_string_lossy(), "guideFile": "GUIDE-ASTRO-SYNC.fr.md" }
        }),
    );
    let with_path = with_path.as_str().unwrap();
    assert!(with_path.starts_with("Parle-moi en français.\n"));
    assert!(with_path.contains(&format!(
        "{}/GUIDE-ASTRO-SYNC.fr.md",
        root.to_string_lossy()
    )));
    let without = call(
        "guide_prompt",
        serde_json::json!({
            "language": "ja",
            "input": { "root": root.to_string_lossy(), "guideFile": null }
        }),
    );
    assert!(!without.as_str().unwrap().contains("GUIDE-ASTRO-SYNC"));

    // A guide that cannot be written is an error the window turns into a
    // state (the root is gone).
    std::fs::remove_dir_all(&root).unwrap();
    let failed = harness
        .invoke(
            "guide_write",
            serde_json::json!({ "language": "fr", "text": "# Guide\n" }),
        )
        .answer(Duration::from_secs(5));
    assert!(failed.is_err());
}

#[test]
fn the_working_folder_the_night_folder_and_its_detection() {
    let _serial = serialized();
    let harness = harness(fast());
    let _ = std::fs::remove_file(harness.config_dir.join("local-settings.json"));
    let root = unique_dir("night-commands-root");

    let prepared = harness
        .invoke(
            "working_folder_set",
            serde_json::json!({ "folder": root.to_string_lossy() }),
        )
        .answer(Duration::from_secs(5))
        .expect("an existing folder becomes the working folder");
    assert_eq!(prepared["assistantFile"]["outcome"], "created");
    assert!(root.join("ASSISTANT-IA.md").is_file());
    let state = harness
        .invoke("working_folder_get", serde_json::json!({}))
        .answer(Duration::from_secs(5))
        .unwrap();
    assert_eq!(state["chosen"], true);
    assert_eq!(state["root"], root.to_string_lossy().as_ref());

    let spec = serde_json::json!({
        "nightId": "7f3a2c91-commands", "serviceUrl": "https://service.example",
        "eveningDate": "2026-10-07", "label": "Col des Étoiles",
        "targets": [{"targetId": "t-m31", "designation": "M31", "ticked": true},
                    {"targetId": "t-ic1396", "designation": "IC 1396", "ticked": false}]
    });
    let planned = harness
        .invoke("night_folder_plan", serde_json::json!({ "spec": spec }))
        .answer(Duration::from_secs(5))
        .unwrap();
    assert_eq!(planned["applied"], false);
    assert!(
        std::fs::read_dir(&root).unwrap().count() == 1,
        "the plan writes nothing"
    );
    let made = harness
        .invoke("night_folder_prepare", serde_json::json!({ "spec": spec }))
        .answer(Duration::from_secs(5))
        .unwrap();
    let folder = std::path::PathBuf::from(made["folder"].as_str().unwrap());
    assert!(folder.join("M31/lights").is_dir());
    assert!(!folder.join("IC 1396").exists());
    // The folder made is remembered for the night: moved away later, the
    // step asks to find it again rather than making another one.
    let kept = std::fs::read_to_string(harness.config_dir.join("local-settings.json")).unwrap();
    assert!(kept.contains("7f3a2c91-commands"), "{kept}");

    let listed = harness
        .invoke("night_folders_list", serde_json::json!({}))
        .answer(Duration::from_secs(5))
        .unwrap();
    assert_eq!(listed[0]["nightId"], "7f3a2c91-commands");
    let located = harness
        .invoke(
            "night_locate",
            serde_json::json!({ "nightId": "7f3a2c91-commands" }),
        )
        .answer(Duration::from_secs(5))
        .unwrap();
    assert!(located.to_string().contains("found"), "{located}");

    let pending = harness.invoke(
        "night_detect",
        serde_json::json!({
            "folder": folder.to_string_lossy(),
            "input": {"eveningDate": "2026-10-07", "targets": [
                {"targetId": "t-m31", "designation": "M31", "ticked": true}]}
        }),
    );
    assert!(pending.held_the_caller_for < Duration::from_millis(150));
    let detection = pending.answer(Duration::from_secs(20)).unwrap();
    let keys: Vec<&str> = detection["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["key"].as_str().unwrap_or_default())
        .collect();
    // An empty target (no FITS frame, no final image) is the error.
    assert!(keys.contains(&"detection.targetEmpty"), "{keys:?}");
    drop(harness.app);
}

/// Two folders carry the night, the person follows one; the choice is kept and
/// every step then finds the night in that folder. A folder that does not
/// carry the night is refused, and nothing is kept.
#[test]
fn a_followed_copy_is_the_nights_folder_from_then_on() {
    let _serial = serialized();
    let harness = harness(fast());
    let _ = std::fs::remove_file(harness.config_dir.join("local-settings.json"));
    let root = unique_dir("night-commands-follow");
    harness
        .invoke(
            "working_folder_set",
            serde_json::json!({ "folder": root.to_string_lossy() }),
        )
        .answer(Duration::from_secs(5))
        .unwrap();
    let night_id = "7f3a2c91-follow";
    let spec = serde_json::json!({
        "nightId": night_id, "serviceUrl": "https://service.example",
        "eveningDate": "2026-10-07", "label": "Col des Étoiles",
        "targets": [{"targetId": "t-m31", "designation": "M31", "ticked": true}]
    });
    let made = harness
        .invoke("night_folder_prepare", serde_json::json!({ "spec": spec }))
        .answer(Duration::from_secs(5))
        .unwrap();
    let first = std::path::PathBuf::from(made["folder"].as_str().unwrap());
    let copy = root.join("copie");
    std::fs::create_dir(&copy).unwrap();
    for entry in std::fs::read_dir(&first).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            std::fs::copy(entry.path(), copy.join(entry.file_name())).unwrap();
        }
    }

    let locate = |harness: &crate::wake_tests::Harness| {
        harness
            .invoke("night_locate", serde_json::json!({ "nightId": night_id }))
            .answer(Duration::from_secs(5))
            .unwrap()
    };
    assert_eq!(locate(&harness)["state"], "duplicated");
    // Without a choice the step refuses: it never picks a copy by itself.
    let refused = harness
        .invoke("night_folder_prepare", serde_json::json!({ "spec": spec }))
        .answer(Duration::from_secs(5));
    assert!(refused.is_err(), "two copies, nothing chosen");

    let elsewhere = unique_dir("night-commands-follow-other");
    let not_a_copy = harness
        .invoke(
            "night_follow",
            serde_json::json!({ "nightId": night_id, "folder": elsewhere.to_string_lossy() }),
        )
        .answer(Duration::from_secs(5));
    assert!(not_a_copy.is_err(), "a folder without the night's identity");
    assert_eq!(locate(&harness)["state"], "duplicated");

    harness
        .invoke(
            "night_follow",
            serde_json::json!({ "nightId": night_id, "folder": copy.to_string_lossy() }),
        )
        .answer(Duration::from_secs(5))
        .expect("a copy that carries the night can be followed");
    let located = locate(&harness);
    assert_eq!(located["state"], "found");
    assert_eq!(located["folder"], copy.to_string_lossy().as_ref());
    // The step, run again, fills the followed copy and leaves the other alone.
    let again = harness
        .invoke("night_folder_prepare", serde_json::json!({ "spec": spec }))
        .answer(Duration::from_secs(5))
        .expect("the followed copy is prepared");
    assert_eq!(again["folder"], copy.to_string_lossy().as_ref());
    assert!(copy.join("M31/lights").is_dir());
    drop(harness.app);
}

#[test]
fn dependencies_pipelines_and_the_prompt() {
    let _serial = serialized();
    let harness = harness(fast());
    let rows = harness
        .invoke("tools_detect", serde_json::json!({}))
        .answer(Duration::from_secs(60))
        .unwrap();
    let pipeline = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "stacking-pipeline")
        .unwrap();
    // The environment is not made in the test's data: the row waits for it.
    assert_eq!(pipeline["state"], "missing");
    assert!(pipeline["blockedBy"]
        .as_array()
        .unwrap()
        .iter()
        .any(|b| b == "python-env" || b == "python"));

    let sections = harness
        .invoke("pipeline_sections", serde_json::json!({ "language": "fr" }))
        .answer(Duration::from_secs(60))
        .unwrap();
    assert_eq!(sections["ignored"], serde_json::json!([]));
    let sections = &sections["sections"];
    assert_eq!(sections[0]["id"], "raw");
    assert_eq!(sections[0]["trust"], "not_needed");
    assert_eq!(sections[0]["dependencies"][0]["id"], "siril");
    assert_eq!(
        sections[0]["dependencies"][0]["reason"],
        "Siril calibre, aligne et additionne vos poses\u{a0}: tous les rendus partent de cet empilement."
    );

    let list = harness
        .invoke("pipelines_list", serde_json::json!({ "language": "fr" }))
        .answer(Duration::from_secs(10))
        .unwrap();
    assert_eq!(list["pipelines"][0]["id"], "raw");
    assert_eq!(list["listedBy"], "manifests");

    let prompt = harness
        .invoke(
            "assistant_prompt",
            serde_json::json!({ "language": "ja", "input": {
                "root": "/tmp/Astro Sync", "nightFolder": "2026-10-07 · Col des Étoiles · 7f3a2c",
                "targets": ["M31"] } }),
        )
        .answer(Duration::from_secs(5))
        .unwrap();
    assert!(prompt
        .as_str()
        .unwrap()
        .starts_with("日本語で話してください。\n"));
    drop(harness.app);
}

/// The plans the window got back from the summary carry the capture metadata
/// Astro Sync proposes; « Envoyer » enqueues them as they are, and the queue
/// keeps them with the row (a resumed sending proposes the same).
#[test]
fn the_enqueued_images_keep_the_proposals_of_their_plans() {
    let _serial = serialized();
    let harness = harness(fast());
    let queue_path = harness.config_dir.join("upload-queue.sqlite");
    let _ = std::fs::remove_file(&queue_path);
    let dir = unique_dir("night-enqueue-proposals");
    let image = dir.join("M31_2026-10-07_raw.tif");
    std::fs::write(&image, b"0123").unwrap();

    let plan = serde_json::json!({
        "image": {
            "targetId": "t-m31", "path": image.to_string_lossy(),
            "sha256": "ab".repeat(32), "size": 4, "kind": "stacked", "render": "raw"
        },
        "decision": "new_capture",
        "metadata": {
            "acquisition": [{"filter": "Ha", "frameCount": 12, "exposureSeconds": 300.0, "gain": 100}],
            "calibration": "30 darks",
            "processing": {"software": "Astro Sync \u{b7} Siril 1.4.4"},
            "variantGeneratedTitle": {"en": "Natural render", "fr": "Rendu naturel"}
        }
    });
    let quota = serde_json::json!({
        "allowed": true, "toSendBytes": 4, "toSendFiles": 1, "newCaptures": 1, "findings": []
    });
    let answer = harness
        .invoke(
            "night_enqueue",
            serde_json::json!({ "sessionId": "night-1", "plans": [plan], "quota": quota }),
        )
        .answer(Duration::from_secs(10))
        .expect("the plans are enqueued");
    assert_eq!(answer["result"], "enqueued");
    assert_eq!(answer["outcome"]["inserted"], 1);
    // Nothing was online for these plans: nothing proposed after the fact.
    assert_eq!(answer["proposals"], serde_json::json!([]));
    assert!(answer["proposalsError"].is_null());

    let queue = astro_sync_core::upload_queue::UploadQueue::open(&queue_path).unwrap();
    let rows = queue.list().unwrap();
    assert_eq!(rows.len(), 1);
    let kept = rows[0]
        .target
        .metadata
        .as_ref()
        .expect("the proposals are kept");
    assert_eq!(kept.acquisition[0].frame_count, 12);
    let title = kept
        .variant_generated_title
        .as_ref()
        .expect("the generated title");
    assert_eq!(title["fr"], "Rendu naturel");
    assert_eq!(title["en"], "Natural render");
    assert_eq!(rows[0].target.session_id.as_deref(), Some("night-1"));
    drop(harness.app);
}

use super::*;
use crate::upload_queue::{EntryTarget, ResultKind};

/// A night's target folder with one render, `M 31_natural.png`, and its TIFF.
fn render_in(dir: &Path) -> PathBuf {
    let stacked = dir.join("night").join("M 31").join("stacked");
    fs::create_dir_all(&stacked).unwrap();
    let render = stacked.join("M 31_natural.png");
    fs::write(&render, b"the render as the stacking wrote it").unwrap();
    fs::write(stacked.join("M 31_natural.tif"), b"the full resolution").unwrap();
    render
}

/// What the script writes when the person saves: the image set's PNG, and
/// its TIFF when the render has a TIFF twin.
fn set(render: &Path, values: AdjustValues) -> AdjustState {
    let adjusted = adjusted_path(render).unwrap();
    fs::create_dir_all(adjusted.parent().unwrap()).unwrap();
    // Each image set its own bytes (its digest names it in the queue).
    fs::write(&adjusted, format!("the image set of {}", render.display())).unwrap();
    if twin_of(render).is_some() {
        fs::write(
            adjusted_tiff_path(render).unwrap(),
            format!("the image set of {}, in 16 bits", render.display()),
        )
        .unwrap();
    }
    write_record(render, values).unwrap()
}

fn values() -> AdjustValues {
    AdjustValues {
        black_point: 8.0,
        brightness: 12.0,
        colour: 1.7,
        ..AdjustValues::NEUTRAL
    }
}

fn candidate(path: &Path, sha256: &str) -> ImageCandidate {
    serde_json::from_value(serde_json::json!({
        "targetId": "m31",
        "path": path,
        "sha256": sha256,
        "size": 35,
        "kind": "stacked",
        "render": "natural",
        "twinSha256": "tiff-digest",
    }))
    .unwrap()
}

fn result(path: &Path, sha256: &str, variant_of: Option<&str>) -> ResultFile {
    ResultFile {
        path: path.to_path_buf(),
        sha256: sha256.to_string(),
        size: 35,
        kind: ResultKind::Stacked,
        target: EntryTarget {
            variant_of: variant_of.map(str::to_string),
            ..EntryTarget::default()
        },
    }
}

#[test]
fn the_image_set_is_written_beside_and_the_render_is_never_changed() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let before = fs::read(&render).unwrap();
    let state = set(&render, values());
    assert_eq!(fs::read(&render).unwrap(), before);
    assert_eq!(
        state.path,
        render
            .parent()
            .unwrap()
            .join("adjusted/M 31_natural_adjusted.png")
    );
    assert!(is_adjusted_image(&state.path));
    assert!(!is_adjusted_image(&render));
    assert_eq!(state.sha256, file_sha256(&state.path).unwrap());
}

#[test]
fn the_values_reopen_as_they_were_saved() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    set(&render, values());
    assert_eq!(state(&render).unwrap().values, values());
    assert_eq!(read_record(&render).unwrap().unwrap().values, values());
}

#[test]
fn back_to_the_original_render_removes_the_image_set_and_keeps_the_render() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let before = fs::read(&render).unwrap();
    let set_state = set(&render, values());
    revert(&render).unwrap();
    assert!(!set_state.path.exists());
    assert!(state(&render).is_none());
    assert_eq!(fs::read(&render).unwrap(), before);
    // Twice is not an error.
    revert(&render).unwrap();
}

#[test]
fn a_render_stacked_again_since_leaves_as_it_is() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    set(&render, values());
    fs::write(&render, b"a new render, made by a new stacking").unwrap();
    assert!(state(&render).is_none());
    assert!(sent_file(&render).is_none());
}

#[test]
fn only_a_png_render_of_stacked_can_be_set() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let tiff = render.with_extension("tif");
    assert!(is_render_png(&render));
    assert!(!is_render_png(&tiff));
    assert!(matches!(
        write_record(&tiff, values()),
        Err(AdjustError::NotARender(_))
    ));
    assert!(matches!(revert(&tiff), Err(AdjustError::NotARender(_))));
}

#[test]
fn the_check_counts_the_image_set_and_keeps_the_path_of_the_render() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let other = render.with_file_name("M 31_hoo.png");
    fs::write(&other, b"another render").unwrap();
    let state = set(&render, values());
    let out = substitute_candidates(vec![
        candidate(&render, "render-digest"),
        candidate(&other, "other-digest"),
    ]);
    assert_eq!(out[0].path, render);
    assert_eq!(out[0].sha256, state.sha256);
    assert_eq!(out[0].size, state.size);
    // Its twin is the image set's TIFF, never the render's.
    assert_eq!(
        out[0].twin_sha256.as_deref(),
        Some(state.tiff.as_ref().unwrap().sha256.as_str())
    );
    assert_ne!(out[0].twin_sha256.as_deref(), Some("tiff-digest"));
    assert_eq!(out[1].sha256, "other-digest");
    assert_eq!(out[1].twin_sha256.as_deref(), Some("tiff-digest"));
}

#[test]
fn the_queue_takes_the_bytes_of_the_image_set_and_its_variants_follow_it() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let other = render.with_file_name("M 31_hoo.png");
    fs::write(&other, b"another render").unwrap();
    let state = set(&render, values());
    let out = substitute_results(&[
        result(&render, "RENDER-DIGEST", None),
        result(&other, "other-digest", Some("sha256:render-digest")),
    ]);
    assert_eq!(out[0].path, state.path);
    assert_eq!(out[0].sha256, state.sha256);
    assert_eq!(
        out[1].target.variant_of.as_deref(),
        Some(format!("sha256:{}", state.sha256).as_str())
    );
    assert_eq!(out[1].path, other);
    // Nothing set: the files leave as they are.
    revert(&render).unwrap();
    let plain = substitute_results(&[result(&render, "render-digest", None)]);
    assert_eq!(plain[0].path, render);
}

#[test]
fn values_out_of_range_are_brought_back_and_neutral_values_write_nothing() {
    let wild = AdjustValues {
        black_point: 99.0,
        brightness: f64::NAN,
        colour: -2.0,
        smoothing: 140.0,
        stars: -300.0,
        spikes: 400.0,
        spikes_reach: f64::NAN,
    }
    .clamped();
    assert_eq!(wild.black_point, 20.0);
    assert_eq!(wild.brightness, 0.0);
    assert_eq!(wild.colour, 0.0);
    assert_eq!(wild.smoothing, 100.0);
    assert_eq!(wild.stars, -100.0);
    assert_eq!(wild.spikes, 100.0);
    assert_eq!(wild.spikes_reach, 50.0);
    assert!(AdjustValues::NEUTRAL.is_neutral());
    assert!(!values().is_neutral());
    assert!(!values().needs_stars());
}

#[test]
fn a_saved_setting_is_five_named_values_renamed_removed_exported() {
    let mut list = Vec::new();
    save_named(&mut list, "  Nébuleuse sombre ", values()).unwrap();
    assert_eq!(list[0].name, "Nébuleuse sombre");
    assert!(matches!(
        save_named(&mut list, "NÉBULEUSE SOMBRE", values()),
        Err(AdjustError::NameTaken(_))
    ));
    assert!(matches!(
        save_named(&mut list, "   ", values()),
        Err(AdjustError::EmptyName)
    ));
    save_named(&mut list, "Amas", AdjustValues::NEUTRAL).unwrap();
    rename_named(&mut list, "amas", "Amas ouvert").unwrap();
    assert_eq!(list[1].name, "Amas ouvert");
    assert!(matches!(
        rename_named(&mut list, "Amas ouvert", "nébuleuse sombre"),
        Err(AdjustError::NameTaken(_))
    ));
    let text = export_saved(&list);
    let mut other = vec![SavedAdjustment {
        name: "Amas ouvert".into(),
        values: AdjustValues::NEUTRAL,
    }];
    let outcome = import_saved(&mut other, &text).unwrap();
    assert_eq!(outcome.added, vec!["Nébuleuse sombre".to_string()]);
    assert_eq!(outcome.skipped, vec!["Amas ouvert".to_string()]);
    assert_eq!(other[1].values, values());
    assert!(matches!(
        import_saved(
            &mut other,
            "{\"format\":\"x\",\"version\":1,\"adjustments\":[]}"
        ),
        Err(AdjustError::NotAnExport(_))
    ));
    remove_named(&mut list, "Amas ouvert").unwrap();
    assert_eq!(list.len(), 1);
    assert!(matches!(
        remove_named(&mut list, "Amas ouvert"),
        Err(AdjustError::UnknownName(_))
    ));
    let long: String = "x".repeat(80);
    save_named(&mut list, &long, values()).unwrap();
    assert_eq!(list[1].name.chars().count(), NAME_MAX_CHARS);
}

#[test]
fn the_saved_settings_live_with_the_settings_of_the_application() {
    let mut settings = crate::local_settings::LocalSettings::default();
    save_named(&mut settings.saved_adjustments, "Amas", values()).unwrap();
    let text = serde_json::to_string(&settings).unwrap();
    let back: crate::local_settings::LocalSettings = serde_json::from_str(&text).unwrap();
    assert_eq!(back.saved_adjustments, settings.saved_adjustments);
    // A file of before, without them, reads as an empty list.
    let old: crate::local_settings::LocalSettings = serde_json::from_str("{}").unwrap();
    assert!(old.saved_adjustments.is_empty());
}

/* ------------------- Saved settings: the screen's rules ------------------- */

fn nebulae() -> AdjustValues {
    AdjustValues {
        black_point: 8.0,
        brightness: 12.0,
        colour: 1.7,
        ..AdjustValues::NEUTRAL
    }
}

fn clusters() -> AdjustValues {
    AdjustValues {
        smoothing: 20.0,
        stars: -40.0,
        ..AdjustValues::NEUTRAL
    }
}

#[test]
fn an_import_never_overwrites_a_name_already_taken_and_says_which() {
    let mut mine = Vec::new();
    save_named(&mut mine, "Nébuleuses en émission", clusters()).unwrap();
    let mut theirs = Vec::new();
    save_named(&mut theirs, "nébuleuses EN émission", nebulae()).unwrap();
    save_named(&mut theirs, "Amas, étoiles réduites", clusters()).unwrap();
    let outcome = import_saved(&mut mine, &export_saved(&theirs)).unwrap();
    assert_eq!(outcome.added, vec!["Amas, étoiles réduites".to_string()]);
    assert_eq!(outcome.skipped, vec!["nébuleuses EN émission".to_string()]);
    // Mine kept its own values.
    assert_eq!(mine[0].name, "Nébuleuses en émission");
    assert_eq!(mine[0].values, clusters());
    assert_eq!(mine.len(), 2);
}

#[test]
fn an_export_reads_back_identical() {
    let dir = tempfile_dir();
    let mut list = Vec::new();
    save_named(&mut list, "Nébuleuses en émission", nebulae()).unwrap();
    save_named(&mut list, "Amas, étoiles réduites", clusters()).unwrap();
    let file = dir.path().join("réglages.json");
    export_saved_to(&file, &list).unwrap();
    assert!(!file.with_extension("json.part").exists());
    let mut back = Vec::new();
    let outcome = import_saved_from(&mut back, &file).unwrap();
    assert_eq!(outcome.skipped, Vec::<String>::new());
    assert_eq!(back, list);
    assert_eq!(export_saved(&back), fs::read_to_string(&file).unwrap());
    // Not a file of settings: said, nothing added.
    let other = dir.path().join("autre.json");
    fs::write(&other, "{\"hello\": 1}").unwrap();
    assert!(matches!(
        import_saved_from(&mut back, &other),
        Err(AdjustError::NotAnExport(_))
    ));
    assert_eq!(back.len(), 2);
}

#[test]
fn renaming_or_removing_a_setting_leaves_the_images_set_with_it_unchanged() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let mut list = Vec::new();
    save_named(&mut list, "Nébuleuses en émission", nebulae()).unwrap();
    // Applied (the sliders moved), then the image saved with those values.
    let applied = list[0].values;
    let saved = set(&render, applied);
    let bytes = fs::read(&saved.path).unwrap();
    rename_named(
        &mut list,
        "Nébuleuses en émission",
        "Nébuleuses (bicolores)",
    )
    .unwrap();
    remove_named(&mut list, "Nébuleuses (bicolores)").unwrap();
    assert!(list.is_empty());
    let after = state(&render).unwrap();
    assert_eq!(after, saved);
    assert_eq!(after.values, nebulae());
    assert_eq!(fs::read(&saved.path).unwrap(), bytes);
}

#[test]
fn a_name_in_double_is_refused_whatever_its_case_and_spaces() {
    let mut list = Vec::new();
    save_named(&mut list, "Amas, étoiles réduites", clusters()).unwrap();
    for twin in ["amas, étoiles réduites", "  AMAS, ÉTOILES RÉDUITES  "] {
        assert!(matches!(
            save_named(&mut list, twin, nebulae()),
            Err(AdjustError::NameTaken(_))
        ));
    }
    // Renaming a setting to its own name in another case is no double.
    rename_named(
        &mut list,
        "Amas, étoiles réduites",
        "AMAS, étoiles réduites",
    )
    .unwrap();
    assert_eq!(list[0].name, "AMAS, étoiles réduites");
    assert_eq!(list.len(), 1);
}

/* ------------- An image set whose render is already online ------------- */

fn online_with(render_sha: &str, main: bool) -> NightOnline {
    let image = |id: &str, sha: &str, main: bool| {
        serde_json::json!({
            "id": id, "main": main, "sha256": sha, "byteSize": 35,
            "originalName": "M 31_natural.png", "source": "stacking",
            "createdAt": "2026-11-14T20:00:00Z", "skySolution": "missing"
        })
    };
    let images = if main {
        vec![image("img-main", render_sha, true)]
    } else {
        vec![
            image("img-main", "another-main", true),
            image("img-variant", render_sha, false),
        ]
    };
    serde_json::from_value(serde_json::json!({
        "sessionId": "night-1",
        "targets": [{
            "targetId": "m31", "name": "M 31", "planned": true,
            "captures": [{
                "id": "cap-1", "targetIds": ["m31"], "source": "stacking",
                "stage": "draft", "visibility": "private", "uploaded": true,
                "createdAt": "2026-11-14T20:00:00Z",
                "mainImage": images[0].clone(),
                "imageCount": images.len(),
                "images": images,
            }]
        }],
        "analysis": { "present": false, "stepCount": 0, "editedStepCount": 0 }
    }))
    .unwrap()
}

/// The space of the fixtures, with room, one capture of one, and `limit`
/// variants per capture.
fn storage(limit: u64) -> crate::precheck::SpaceStorage {
    let text = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/online/space_storage.json"),
    )
    .unwrap();
    let mut storage: crate::precheck::SpaceStorage = serde_json::from_str(&text).unwrap();
    storage.remaining_bytes = 1_000_000;
    storage.max_file_bytes = 1_000_000;
    storage.capture_count = 1;
    storage.capture_count_limit = 1;
    storage.variant_limit = limit;
    storage
}

fn plans_for(online: &NightOnline, render: &Path, render_sha: &str, limit: u64) -> Vec<ImagePlan> {
    let candidates = substitute_candidates(vec![candidate(render, render_sha)]);
    let mut plans = crate::precheck::plan_images_within(online, &candidates, limit, u64::MAX);
    place_replacements(online, &mut plans);
    plans
}

#[test]
fn an_image_set_takes_the_place_of_its_render_online_as_the_main_image() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let render_sha = file_sha256(&render).unwrap();
    let saved = set(&render, values());
    let online = online_with(&render_sha, true);
    // Whatever the space keeps of variants: a replacement is no variant.
    for limit in [0, 3] {
        let plans = plans_for(&online, &render, &render_sha, limit);
        assert_eq!(
            plans[0].decision,
            Decision::Replace {
                capture_id: "cap-1".into(),
                image_id: "img-main".into()
            }
        );
        assert!(plans[0].decision.sends());
        let metadata = crate::precheck::metadata_of(&plans[0], "night-1");
        assert_eq!(metadata.capture_id.as_deref(), Some("cap-1"));
        assert_eq!(metadata.placement.as_deref(), Some(PLACEMENT_REPLACE));
        assert_eq!(metadata.sha256, saved.sha256);
        let storage = storage(limit);
        let quota = crate::precheck::check_quotas(&storage, &online, &plans);
        assert!(quota.allowed, "{:?}", quota.findings);
        assert_eq!(quota.new_captures, 0);
    }
}

#[test]
fn an_image_set_whose_render_is_a_variant_online_is_set_aside_not_doubled() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let render_sha = file_sha256(&render).unwrap();
    set(&render, values());
    let plans = plans_for(&online_with(&render_sha, false), &render, &render_sha, 3);
    assert_eq!(
        plans[0].decision,
        Decision::NotSent {
            reason: NotSent::AdjustedVariantOnline
        }
    );
    assert!(!plans[0].decision.sends());
}

#[test]
fn an_image_set_whose_render_never_left_leaves_by_the_usual_rule() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let render_sha = file_sha256(&render).unwrap();
    set(&render, values());
    let plans = plans_for(
        &online_with("something-else", true),
        &render,
        &render_sha,
        3,
    );
    assert_eq!(
        plans[0].decision,
        Decision::Variant {
            capture_id: "cap-1".into(),
            replaces_sha256: None,
        }
    );
    assert_eq!(
        crate::precheck::metadata_of(&plans[0], "night-1").placement,
        None
    );
    // The image set itself online: already online, nothing replaced.
    let state = state(&render).unwrap();
    let plans = plans_for(&online_with(&state.sha256, true), &render, &render_sha, 3);
    assert!(matches!(plans[0].decision, Decision::AlreadyOnline { .. }));
}

#[test]
fn the_queue_keeps_the_replacement_and_the_upload_declares_it() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let render_sha = file_sha256(&render).unwrap();
    let saved = set(&render, values());
    let online = online_with(&render_sha, true);
    let plans = plans_for(&online, &render, &render_sha, 0);
    let queue = crate::upload_queue::UploadQueue::open(&dir.path().join("queue.sqlite")).unwrap();
    let storage = storage(0);
    let check = crate::precheck::check_quotas(&storage, &online, &plans);
    crate::precheck::enqueue_checked(&queue, "night-1", &plans, &check)
        .unwrap()
        .unwrap();
    let entry = queue.find_by_sha256(&saved.sha256).unwrap().unwrap();
    assert_eq!(entry.source_path, saved.path);
    assert_eq!(entry.target.variant_of.as_deref(), Some("cap-1"));
    assert_eq!(entry.target.placement.as_deref(), Some(PLACEMENT_REPLACE));
    let metadata = crate::uploader::build_metadata(
        &entry,
        &crate::uploader::UploadTarget {
            session_id: None,
            plan_id: None,
            target_ids: Vec::new(),
        },
    );
    assert_eq!(metadata.capture_id.as_deref(), Some("cap-1"));
    assert_eq!(metadata.placement.as_deref(), Some(PLACEMENT_REPLACE));
    // The upload's body carries it.
    let body = serde_json::to_value(&metadata).unwrap();
    assert_eq!(body["placement"], "replace");
}

#[test]
fn a_render_sent_through_this_queue_then_set_is_sent_again_in_its_place() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let render_sha = file_sha256(&render).unwrap();
    let queue = crate::upload_queue::UploadQueue::open(&dir.path().join("queue.sqlite")).unwrap();
    // « Images à envoyer »: the render leaves as the capture's main image.
    queue
        .enqueue_results(&[result(&render, &render_sha, None)])
        .unwrap();
    queue.mark_completed(&render_sha, "cap-1").unwrap();
    // Set afterwards, then sent again.
    let saved = set(&render, values());
    let outcome = queue
        .enqueue_results(&[result(&render, &render_sha, None)])
        .unwrap();
    assert_eq!(outcome.inserted, 1);
    let entry = queue.find_by_sha256(&saved.sha256).unwrap().unwrap();
    assert_eq!(entry.target.variant_of.as_deref(), Some("cap-1"));
    assert_eq!(entry.target.placement.as_deref(), Some(PLACEMENT_REPLACE));
    // A render sent as a variant is not the main image: no replacement.
    let other = render.with_file_name("M 31_hoo.png");
    fs::write(&other, b"another render").unwrap();
    let other_sha = file_sha256(&other).unwrap();
    queue
        .enqueue_results(&[result(&other, &other_sha, Some("cap-1"))])
        .unwrap();
    queue.mark_completed(&other_sha, "cap-1").unwrap();
    let other_set = set(&other, values());
    queue
        .enqueue_results(&[result(&other, &other_sha, None)])
        .unwrap();
    let entry = queue.find_by_sha256(&other_set.sha256).unwrap().unwrap();
    assert_eq!(entry.target.placement, None);
}

#[test]
fn the_enqueueing_names_the_image_set_that_leaves_for_a_render() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let render_sha = file_sha256(&render).unwrap();
    let other = render.with_file_name("M 31_hoo.png");
    fs::write(&other, b"another render").unwrap();
    let other_sha = file_sha256(&other).unwrap();
    let saved = set(&render, values());
    let queue = crate::upload_queue::UploadQueue::open(&dir.path().join("queue.sqlite")).unwrap();
    let outcome = queue
        .enqueue_results(&[
            result(&render, &render_sha, None),
            result(&other, &other_sha, None),
        ])
        .unwrap();
    // The window follows the file that leaves, not the render it asked for.
    assert_eq!(outcome.sent_instead.len(), 1);
    let instead = &outcome.sent_instead[0];
    assert_eq!(instead.render_sha256, render_sha);
    assert_eq!(instead.sha256, saved.sha256);
    assert_eq!(instead.path, saved.path);
    assert_eq!(instead.size, saved.size);
    let json = serde_json::to_value(&outcome).unwrap();
    assert_eq!(json["sentInstead"][0]["renderSha256"], render_sha.as_str());
    // A render without an image set is named nowhere.
    assert!(
        outcome
            .sent_instead
            .iter()
            .all(|item| item.render_sha256 != other_sha)
    );
}

#[test]
fn a_night_whose_render_is_online_and_set_since_has_an_image_to_send() {
    use crate::night_resume::{FolderState, LocalImage, Missing, ResumeLocal, resume_night};
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let render_sha = file_sha256(&render).unwrap();
    let online = online_with(&render_sha, true);
    let local = ResumeLocal {
        folder: FolderState::Found,
        targets: Vec::new(),
        logs: 0,
        images: vec![LocalImage {
            target_id: "m31".into(),
            path: render.clone(),
            sha256: render_sha.clone(),
            size: 35,
            kind: ResultKind::Stacked,
            twin_sha256: None,
        }],
        analysis_inputs: None,
    };
    let to_send = |online: &NightOnline| {
        resume_night(online, &local, &[], None, 0)
            .missing
            .iter()
            .find_map(|m| match m {
                Missing::ImagesToSend { count } => Some(*count),
                _ => None,
            })
            .unwrap_or(0)
    };
    assert_eq!(to_send(&online), 0, "the render online: nothing to send");
    set(&render, values());
    assert_eq!(to_send(&online), 1, "set since: its image set is to send");
    // Online as a variant: it cannot be replaced from here, not « to send ».
    assert_eq!(to_send(&online_with(&render_sha, false)), 0);
}

/* ---------------- The TIFF of an image set (full resolution) -------------- */

fn tiff_candidate(path: &Path, sha256: &str, png_sha256: &str) -> ImageCandidate {
    serde_json::from_value(serde_json::json!({
        "targetId": "m31",
        "path": path,
        "sha256": sha256,
        "size": 19,
        "kind": "stacked",
        "render": "natural",
        "twinSha256": png_sha256,
    }))
    .unwrap()
}

#[test]
fn a_render_with_a_tiff_is_set_with_its_tiff_beside_and_the_twin_is_never_changed() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let twin = twin_of(&render).unwrap();
    let before = fs::read(&twin).unwrap();
    let state = set(&render, values());
    let tiff = state.tiff.clone().unwrap();
    assert_eq!(
        tiff.path,
        render
            .parent()
            .unwrap()
            .join("adjusted/M 31_natural_adjusted.tif")
    );
    assert_eq!(tiff.sha256, file_sha256(&tiff.path).unwrap());
    assert_eq!(fs::read(&twin).unwrap(), before);
    assert!(is_adjusted_image(&tiff.path));
    // The record names it, and reads back the same state.
    let record = read_record(&render).unwrap().unwrap();
    assert_eq!(
        record.adjusted_tiff.as_deref(),
        Some("M 31_natural_adjusted.tif")
    );
    assert_eq!(record.version, RECORD_VERSION);
    assert_eq!(super::state(&render).unwrap(), state);
    // What the window reads: the TIFF under `tiff`.
    let json = serde_json::to_value(&state).unwrap();
    assert_eq!(json["tiff"]["sha256"], tiff.sha256.as_str());
    // A twin is not a render that can be set by itself.
    assert!(sent_file(&twin).is_none());
}

#[test]
fn a_render_without_a_tiff_has_an_image_set_without_one() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    fs::remove_file(twin_of(&render).unwrap()).unwrap();
    let state = set(&render, values());
    assert!(state.tiff.is_none());
    assert!(adjusted_tiff_path(&render).is_some_and(|p| !p.exists()));
    let json = serde_json::to_value(&state).unwrap();
    assert!(json.get("tiff").is_none());
    let record = fs::read_to_string(record_path(&render).unwrap()).unwrap();
    assert!(!record.contains("adjustedTiff"));
}

#[test]
fn the_image_set_needs_its_tiff_when_the_render_has_one() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let adjusted = adjusted_path(&render).unwrap();
    fs::create_dir_all(adjusted.parent().unwrap()).unwrap();
    fs::write(&adjusted, b"an image set without its TIFF").unwrap();
    assert!(matches!(
        write_record(&render, values()),
        Err(AdjustError::Io { .. })
    ));
    assert!(state(&render).is_none());
}

#[test]
fn the_joined_tiff_of_a_set_render_is_the_image_set_s_tiff() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let twin = twin_of(&render).unwrap();
    let render_sha = file_sha256(&render).unwrap();
    let twin_sha = file_sha256(&twin).unwrap();
    let state = set(&render, values());
    let tiff = state.tiff.clone().unwrap();
    let out = substitute_candidates(vec![
        candidate(&render, &render_sha),
        tiff_candidate(&twin, &twin_sha, &render_sha),
    ]);
    // The PNG and the TIFF that leave name each other: the same image.
    assert_eq!(out[0].sha256, state.sha256);
    assert_eq!(out[0].twin_sha256.as_deref(), Some(tiff.sha256.as_str()));
    assert_eq!(out[1].path, twin);
    assert_eq!(out[1].sha256, tiff.sha256);
    assert_eq!(out[1].size, tiff.size);
    assert_eq!(out[1].twin_sha256.as_deref(), Some(state.sha256.as_str()));
    assert!(out[1].is_full_resolution_twin());
    // « Joindre aussi les TIFF » counts the image set's TIFF.
    let joined = crate::night_steps::full_resolution_of(&out, true);
    assert_eq!((joined.count, joined.bytes), (1, tiff.size));
    // The queue takes its bytes, a variant of the image set.
    let files = substitute_results(&[
        result(&render, &render_sha, None),
        result(&twin, &twin_sha, Some(&format!("sha256:{render_sha}"))),
    ]);
    assert_eq!(files[1].path, tiff.path);
    assert_eq!(files[1].sha256, tiff.sha256);
    assert_eq!(files[1].size, tiff.size);
    assert_eq!(
        files[1].target.variant_of.as_deref(),
        Some(format!("sha256:{}", state.sha256).as_str())
    );
    assert_eq!(
        crate::upload_queue::result_kind_of(&files[1].path).unwrap(),
        ResultKind::Stacked
    );
}

#[test]
fn back_to_the_original_render_the_tiff_is_the_render_s_again() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let twin = twin_of(&render).unwrap();
    let render_sha = file_sha256(&render).unwrap();
    let twin_sha = file_sha256(&twin).unwrap();
    let state = set(&render, values());
    revert(&render).unwrap();
    assert!(!state.tiff.unwrap().path.exists());
    assert!(twin.exists());
    let out = substitute_candidates(vec![
        candidate(&render, &render_sha),
        tiff_candidate(&twin, &twin_sha, &render_sha),
    ]);
    assert_eq!(out[1].sha256, twin_sha);
    assert_eq!(out[1].twin_sha256.as_deref(), Some(render_sha.as_str()));
    let files = substitute_results(&[result(&twin, &twin_sha, None)]);
    assert_eq!(files[0].path, twin);
}

#[test]
fn a_tiff_written_again_since_makes_the_image_set_stale() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let twin = twin_of(&render).unwrap();
    set(&render, values());
    fs::write(&twin, b"a new full resolution, longer than before").unwrap();
    assert!(state(&render).is_none());
    assert!(sent_twin(&twin).is_none());
    // Its image set's TIFF gone: stale too (never the render's TIFF instead).
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let saved = set(&render, values());
    fs::remove_file(saved.tiff.unwrap().path).unwrap();
    assert!(state(&render).is_none());
}

#[test]
fn a_record_of_before_without_the_tiff_leaves_the_render_and_its_tiff_as_they_are() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let twin = twin_of(&render).unwrap();
    let render_sha = file_sha256(&render).unwrap();
    let twin_sha = file_sha256(&twin).unwrap();
    let adjusted = adjusted_path(&render).unwrap();
    fs::create_dir_all(adjusted.parent().unwrap()).unwrap();
    fs::write(&adjusted, b"an image set of before").unwrap();
    let original = fs::metadata(&render).unwrap();
    // The record as « Enregistrer l'image réglée » wrote it before the TIFF.
    let record = serde_json::json!({
        "version": 1,
        "values": {"blackPoint": 8.0, "brightness": 12.0, "colour": 1.7, "smoothing": 0.0, "stars": 0.0},
        "adjusted": "M 31_natural_adjusted.png",
        "adjustedSha256": file_sha256(&adjusted).unwrap(),
        "adjustedSize": fs::metadata(&adjusted).unwrap().len(),
        "originalSize": original.len(),
        "originalModified": modified_secs(&original),
        "originalSha256": render_sha,
    });
    fs::write(record_path(&render).unwrap(), record.to_string()).unwrap();
    // Readable (its values are there) ...
    assert_eq!(read_record(&render).unwrap().unwrap().values, values());
    // ... but stale: the PNG and the TIFF that would leave would not be the
    // same image.
    assert!(state(&render).is_none());
    let out = substitute_candidates(vec![
        candidate(&render, &render_sha),
        tiff_candidate(&twin, &twin_sha, &render_sha),
    ]);
    assert_eq!(out[0].sha256, render_sha);
    assert_eq!(out[1].sha256, twin_sha);
    // The same record for a render without a TIFF stays valid.
    fs::remove_file(&twin).unwrap();
    let state = state(&render).unwrap();
    assert_eq!(state.values, values());
    assert!(state.tiff.is_none());
}

#[test]
fn the_enqueueing_names_the_image_set_s_tiff_for_the_render_s_tiff() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let twin = twin_of(&render).unwrap();
    let render_sha = file_sha256(&render).unwrap();
    let twin_sha = file_sha256(&twin).unwrap();
    let saved = set(&render, values());
    let tiff = saved.tiff.clone().unwrap();
    let queue = crate::upload_queue::UploadQueue::open(&dir.path().join("queue.sqlite")).unwrap();
    let outcome = queue
        .enqueue_results(&[
            result(&render, &render_sha, None),
            result(&twin, &twin_sha, Some(&format!("sha256:{render_sha}"))),
        ])
        .unwrap();
    assert_eq!(outcome.inserted, 2);
    assert_eq!(outcome.sent_instead.len(), 2);
    let instead = outcome
        .sent_instead
        .iter()
        .find(|item| item.render_sha256 == twin_sha)
        .unwrap();
    assert_eq!(
        (&instead.sha256, &instead.path, instead.size),
        (&tiff.sha256, &tiff.path, tiff.size)
    );
    let entry = queue.find_by_sha256(&tiff.sha256).unwrap().unwrap();
    assert_eq!(entry.source_path, tiff.path);
    assert_eq!(
        entry.target.variant_of.as_deref(),
        Some(format!("sha256:{}", saved.sha256).as_str())
    );
    assert!(queue.find_by_sha256(&twin_sha).unwrap().is_none());
}

#[test]
fn a_render_online_then_set_replaces_its_main_image_and_its_tiff_follows_as_the_image_set_s() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let twin = twin_of(&render).unwrap();
    let render_sha = file_sha256(&render).unwrap();
    let twin_sha = file_sha256(&twin).unwrap();
    let saved = set(&render, values());
    let tiff = saved.tiff.clone().unwrap();
    let online = online_with(&render_sha, true);
    let candidates = substitute_candidates(vec![
        candidate(&render, &render_sha),
        tiff_candidate(&twin, &twin_sha, &render_sha),
    ]);
    let mut plans = crate::precheck::plan_images_within(&online, &candidates, 3, u64::MAX);
    place_replacements(&online, &mut plans);
    assert!(matches!(plans[0].decision, Decision::Replace { .. }));
    assert_eq!(
        plans[1].decision,
        Decision::Variant {
            capture_id: "cap-1".into(),
            replaces_sha256: None,
        }
    );
    assert_eq!(
        crate::precheck::metadata_of(&plans[1], "night-1").sha256,
        tiff.sha256
    );
}

#[test]
fn the_night_s_check_joins_the_image_set_s_tiff_only_when_the_tiff_are_joined() {
    use crate::night_resume::{FolderState, LocalImage, ResumeLocal};
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let twin = twin_of(&render).unwrap();
    let render_sha = file_sha256(&render).unwrap();
    let twin_sha = file_sha256(&twin).unwrap();
    let image = |path: &Path, sha: &str, twin: &str| LocalImage {
        target_id: "m31".into(),
        path: path.to_path_buf(),
        sha256: sha.to_string(),
        size: 35,
        kind: ResultKind::Stacked,
        twin_sha256: Some(twin.to_string()),
    };
    // What `night_steps::local_images` reads in the night's folder.
    let local = ResumeLocal {
        folder: FolderState::Found,
        targets: Vec::new(),
        logs: 0,
        images: vec![
            image(&render, &render_sha, &twin_sha),
            image(&twin, &twin_sha, &render_sha),
        ],
        analysis_inputs: None,
    };
    let saved = set(&render, values());
    let tiff = saved.tiff.clone().unwrap();
    // The command line and the window read the same check.
    let joined = crate::night_steps::precheck_candidates(&local, Vec::new(), &[], true);
    assert_eq!(joined.len(), 2);
    assert_eq!(joined[0].sha256, saved.sha256);
    assert_eq!(
        (joined[1].sha256.as_str(), joined[1].size),
        (tiff.sha256.as_str(), tiff.size)
    );
    let alone = crate::night_steps::precheck_candidates(&local, Vec::new(), &[], false);
    assert_eq!(alone.len(), 1);
    assert_eq!(alone[0].sha256, saved.sha256);
    // Brought back: the render's TIFF again.
    revert(&render).unwrap();
    let joined = crate::night_steps::precheck_candidates(&local, Vec::new(), &[], true);
    assert_eq!(joined[1].sha256, twin_sha);
}

/* ------ The TIFF of an image set in place of the render's TIFF online ----- */

/// M 31 online: the render as the main image, its TIFF (`tiff_sha`) as a
/// variant of the capture `capture`.
fn online_with_tiff(render_sha: &str, tiff_sha: &str, capture: &str) -> NightOnline {
    let image = |id: &str, sha: &str, main: bool, name: &str| {
        serde_json::json!({
            "id": id, "main": main, "sha256": sha, "byteSize": 35,
            "originalName": name, "source": "stacking",
            "createdAt": "2026-11-14T20:00:00Z", "skySolution": "missing"
        })
    };
    let images = vec![
        image("img-main", render_sha, true, "M 31_natural.png"),
        image("img-tiff", tiff_sha, false, "M 31_natural.tif"),
    ];
    serde_json::from_value(serde_json::json!({
        "sessionId": "night-1",
        "targets": [{
            "targetId": "m31", "name": "M 31", "planned": true,
            "captures": [{
                "id": capture, "targetIds": ["m31"], "source": "stacking",
                "stage": "draft", "visibility": "private", "uploaded": true,
                "createdAt": "2026-11-14T20:00:00Z",
                "mainImage": images[0].clone(),
                "imageCount": images.len(),
                "images": images,
            }]
        }],
        "analysis": { "present": false, "stepCount": 0, "editedStepCount": 0 }
    }))
    .unwrap()
}

fn tiff_plans(online: &NightOnline, render: &Path, limit: u64) -> Vec<ImagePlan> {
    let twin = twin_of(render).unwrap();
    let render_sha = file_sha256(render).unwrap();
    let twin_sha = file_sha256(&twin).unwrap();
    let candidates = substitute_candidates(vec![
        candidate(render, &render_sha),
        tiff_candidate(&twin, &twin_sha, &render_sha),
    ]);
    let mut plans = crate::precheck::plan_images_within(online, &candidates, limit, u64::MAX);
    place_replacements(online, &mut plans);
    plans
}

#[test]
fn a_render_and_its_tiff_online_then_set_leave_in_their_place_one_tiff_online() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let twin_sha = file_sha256(&twin_of(&render).unwrap()).unwrap();
    let render_sha = file_sha256(&render).unwrap();
    let saved = set(&render, values());
    let tiff = saved.tiff.clone().unwrap();
    let online = online_with_tiff(&render_sha, &twin_sha, "cap-1");
    // The space keeps one variant per capture: the TIFF online is it.
    let plans = tiff_plans(&online, &render, 1);
    assert!(matches!(plans[0].decision, Decision::Replace { .. }));
    assert_eq!(
        plans[1].decision,
        Decision::Variant {
            capture_id: "cap-1".into(),
            replaces_sha256: Some(twin_sha.clone()),
        }
    );
    let metadata = crate::precheck::metadata_of(&plans[1], "night-1");
    assert_eq!(metadata.sha256, tiff.sha256);
    assert_eq!(metadata.capture_id.as_deref(), Some("cap-1"));
    assert_eq!(metadata.replaces_sha256.as_deref(), Some(twin_sha.as_str()));
    assert_eq!(metadata.placement, None);
    // A replacement adds no variant: the limit of one is not passed.
    let quota = crate::precheck::check_quotas(&storage(1), &online, &plans);
    assert!(quota.allowed, "{:?}", quota.findings);
    // The window reads it under `replacesSha256`, and passes it back.
    let json = serde_json::to_value(&plans[1].decision).unwrap();
    assert_eq!(json["decision"], "variant");
    assert_eq!(json["replacesSha256"], twin_sha.as_str());
    let back: Decision = serde_json::from_value(json).unwrap();
    assert_eq!(back, plans[1].decision);
    // A plain variant says nothing of it (the window of before reads it).
    let plain = serde_json::to_value(Decision::Variant {
        capture_id: "cap-1".into(),
        replaces_sha256: None,
    })
    .unwrap();
    assert!(plain.get("replacesSha256").is_none());
    // The queue keeps it, and the upload declares it.
    let queue = crate::upload_queue::UploadQueue::open(&dir.path().join("queue.sqlite")).unwrap();
    crate::precheck::enqueue_checked(&queue, "night-1", &plans, &quota)
        .unwrap()
        .unwrap();
    let entry = queue.find_by_sha256(&tiff.sha256).unwrap().unwrap();
    assert_eq!(entry.target.variant_of.as_deref(), Some("cap-1"));
    assert_eq!(
        entry.target.replaces_sha256.as_deref(),
        Some(twin_sha.as_str())
    );
    let body = serde_json::to_value(crate::uploader::build_metadata(
        &entry,
        &crate::uploader::UploadTarget::default(),
    ))
    .unwrap();
    assert_eq!(body["replacesSha256"], twin_sha.as_str());
}

#[test]
fn keeping_the_tiff_online_adds_the_image_set_s_tiff_beside_it() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let twin_sha = file_sha256(&twin_of(&render).unwrap()).unwrap();
    let render_sha = file_sha256(&render).unwrap();
    set(&render, values());
    let online = online_with_tiff(&render_sha, &twin_sha, "cap-1");
    let mut plans = tiff_plans(&online, &render, 3);
    crate::precheck::keep_beside(&mut plans);
    assert_eq!(
        plans[1].decision,
        Decision::Variant {
            capture_id: "cap-1".into(),
            replaces_sha256: None,
        }
    );
    // The main image is still replaced: only the TIFF is kept beside.
    assert!(matches!(plans[0].decision, Decision::Replace { .. }));
    assert_eq!(
        crate::precheck::metadata_of(&plans[1], "night-1").replaces_sha256,
        None
    );
    // Kept beside at a limit of one variant: the limit says so.
    let quota = crate::precheck::check_quotas(&storage(1), &online, &plans);
    assert!(!quota.allowed);
}

#[test]
fn the_box_keeps_only_the_tiff_it_names_beside_the_one_online() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let twin_sha = file_sha256(&twin_of(&render).unwrap()).unwrap();
    let render_sha = file_sha256(&render).unwrap();
    let saved = set(&render, values());
    let tiff_sha = saved.tiff.clone().unwrap().sha256;
    let online = online_with_tiff(&render_sha, &twin_sha, "cap-1");
    // Unticked: nothing is named, the TIFF replaces the one online.
    let mut plans = tiff_plans(&online, &render, 1);
    crate::precheck::keep_beside_where(&mut plans, &|_| false);
    assert!(
        matches!(&plans[1].decision, Decision::Variant { replaces_sha256: Some(old), .. } if *old == twin_sha)
    );
    assert!(crate::precheck::check_quotas(&storage(1), &online, &plans).allowed);
    // Another image named: this one still replaces.
    crate::precheck::keep_beside_where(&mut plans, &|plan| plan.image.sha256 == "other");
    assert!(matches!(
        &plans[1].decision,
        Decision::Variant {
            replaces_sha256: Some(_),
            ..
        }
    ));
    // Ticked: its TIFF is named and stays beside, the limit counts it.
    crate::precheck::keep_beside_where(&mut plans, &|plan| plan.image.sha256 == tiff_sha);
    assert_eq!(
        plans[1].decision,
        Decision::Variant {
            capture_id: "cap-1".into(),
            replaces_sha256: None,
        }
    );
    assert!(matches!(plans[0].decision, Decision::Replace { .. }));
    let quota = crate::precheck::check_quotas(&storage(1), &online, &plans);
    assert!(!quota.allowed);
    assert_eq!(quota.findings[0].code(), "capture_variant_limit_reached");
    assert!(crate::precheck::check_quotas(&storage(3), &online, &plans).allowed);
}

#[test]
fn a_tiff_online_elsewhere_or_never_set_is_never_replaced() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let twin_sha = file_sha256(&twin_of(&render).unwrap()).unwrap();
    let render_sha = file_sha256(&render).unwrap();
    // Not set: the render's own TIFF is already online, nothing leaves.
    let online = online_with_tiff(&render_sha, &twin_sha, "cap-1");
    let plans = tiff_plans(&online, &render, 3);
    assert!(
        plans
            .iter()
            .all(|plan| matches!(plan.decision, Decision::AlreadyOnline { .. }))
    );
    // Set, the TIFF online being the main image of its capture: the image
    // set's TIFF never replaces a main image as a variant.
    set(&render, values());
    let swapped = online_with_tiff(&twin_sha, "another", "cap-1");
    let plans = tiff_plans(&swapped, &render, 3);
    assert!(plans.iter().all(|plan| !matches!(
        plan.decision,
        Decision::Variant {
            replaces_sha256: Some(_),
            ..
        }
    )));
}

#[test]
fn a_record_of_before_names_the_tiff_online_by_the_twin_s_own_digest() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let twin_sha = file_sha256(&twin_of(&render).unwrap()).unwrap();
    let render_sha = file_sha256(&render).unwrap();
    set(&render, values());
    assert_eq!(
        original_tiff_sha256(&render).as_deref(),
        Some(twin_sha.as_str())
    );
    // The record of the build before this field: the twin is read.
    let path = record_path(&render).unwrap();
    let mut record: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    record.as_object_mut().unwrap().remove("originalTiffSha256");
    fs::write(&path, record.to_string()).unwrap();
    assert!(
        read_record(&render)
            .unwrap()
            .unwrap()
            .original_tiff_sha256
            .is_none()
    );
    assert_eq!(
        original_tiff_sha256(&render).as_deref(),
        Some(twin_sha.as_str())
    );
    let online = online_with_tiff(&render_sha, &twin_sha, "cap-1");
    let plans = tiff_plans(&online, &render, 3);
    assert!(matches!(
        &plans[1].decision,
        Decision::Variant { replaces_sha256: Some(old), .. } if *old == twin_sha
    ));
}

#[test]
fn a_render_and_its_tiff_sent_through_this_queue_then_set_replace_both() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let twin = twin_of(&render).unwrap();
    let render_sha = file_sha256(&render).unwrap();
    let twin_sha = file_sha256(&twin).unwrap();
    let queue = crate::upload_queue::UploadQueue::open(&dir.path().join("queue.sqlite")).unwrap();
    // « Images à envoyer » with « Joindre aussi les TIFF »: the render and its
    // TIFF, a variant of the capture the render creates.
    let files = [
        result(&render, &render_sha, None),
        result(&twin, &twin_sha, Some(&format!("sha256:{render_sha}"))),
    ];
    queue.enqueue_results(&files).unwrap();
    queue.mark_completed(&render_sha, "cap-1").unwrap();
    queue.mark_completed(&twin_sha, "cap-1").unwrap();
    // Set the next day, then sent again.
    let saved = set(&render, values());
    let tiff = saved.tiff.clone().unwrap();
    let outcome = queue.enqueue_results(&files).unwrap();
    assert_eq!(outcome.inserted, 2);
    let png = queue.find_by_sha256(&saved.sha256).unwrap().unwrap();
    assert_eq!(png.target.placement.as_deref(), Some(PLACEMENT_REPLACE));
    let entry = queue.find_by_sha256(&tiff.sha256).unwrap().unwrap();
    assert_eq!(entry.source_path, tiff.path);
    assert_eq!(entry.target.variant_of.as_deref(), Some("cap-1"));
    assert_eq!(
        entry.target.replaces_sha256.as_deref(),
        Some(twin_sha.as_str())
    );
    assert_eq!(entry.target.placement, None);
    // A TIFF the night's check already sent somewhere is left as decided.
    let mut decided = [ResultFile {
        path: tiff.path.clone(),
        sha256: tiff.sha256.clone(),
        size: tiff.size,
        kind: ResultKind::Stacked,
        target: EntryTarget {
            variant_of: Some("cap-2".into()),
            ..EntryTarget::default()
        },
    }];
    replace_sent_renders(&mut decided, |_| Some(("cap-1".into(), false)));
    assert_eq!(decided[0].target.variant_of.as_deref(), Some("cap-2"));
    assert_eq!(decided[0].target.replaces_sha256, None);
}

fn tempfile_dir() -> crate::test_support::TempDir {
    crate::test_support::TempDir::new("image-adjust")
}

/* ------------------------------ Quarter turns ----------------------------- */

#[test]
fn a_turned_image_set_reopens_with_its_turns() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let adjusted = adjusted_path(&render).unwrap();
    fs::create_dir_all(adjusted.parent().unwrap()).unwrap();
    fs::write(&adjusted, b"turned").unwrap();
    fs::write(adjusted_tiff_path(&render).unwrap(), b"turned, in 16 bits").unwrap();
    let written = write_record_turned(&render, AdjustValues::NEUTRAL, 3).unwrap();
    assert_eq!(written.quarter_turns, 3);
    // A turn alone is a valid image set: it leaves in place of the render.
    let reopened = state(&render).unwrap();
    assert_eq!(reopened.quarter_turns, 3);
    assert!(reopened.values.is_neutral());
    assert_eq!(sent_file(&render).unwrap().sha256, written.sha256);
}

#[test]
fn the_turns_are_brought_into_a_quarter_turn_count() {
    assert_eq!(clamp_turns(0), 0);
    assert_eq!(clamp_turns(4), 0);
    assert_eq!(clamp_turns(7), 3);
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let written = set(&render, values());
    assert_eq!(written.quarter_turns, 0);
    let again = write_record_turned(&render, values(), 6).unwrap();
    assert_eq!(again.quarter_turns, 2);
}

#[test]
fn a_record_of_before_has_no_turn() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    set(&render, values());
    let path = record_path(&render).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    // The record of before the turns: no `quarterTurns` field at all.
    assert!(!text.contains("quarterTurns"));
    assert_eq!(state(&render).unwrap().quarter_turns, 0);
    let mut record: AdjustRecord = serde_json::from_str(&text).unwrap();
    record.quarter_turns = 1;
    fs::write(&path, serde_json::to_string(&record).unwrap()).unwrap();
    assert_eq!(state(&render).unwrap().quarter_turns, 1);
}

#[test]
fn a_saved_setting_never_carries_a_turn() {
    // The values only: the turn belongs to the image, not to a setting
    // applied to other images.
    let json = serde_json::to_value(values()).unwrap();
    assert!(json.get("quarterTurns").is_none());
    let saved = serde_json::to_value(SavedAdjustment {
        name: "Nebulae".to_string(),
        values: values(),
    })
    .unwrap();
    assert!(saved.to_string().find("quarterTurns").is_none());
}

fn with_spikes(strength: f64, reach: f64) -> AdjustValues {
    AdjustValues {
        spikes: strength,
        spikes_reach: reach,
        ..AdjustValues::NEUTRAL
    }
}

#[test]
fn the_spikes_are_neutral_at_zero_whatever_the_reach() {
    assert!(AdjustValues::NEUTRAL.is_neutral());
    assert_eq!(AdjustValues::NEUTRAL.spikes, 0.0);
    assert_eq!(AdjustValues::NEUTRAL.spikes_reach, 50.0);
    // The reach alone moves nothing: no spike, no change, and the same values.
    assert!(with_spikes(0.0, 90.0).is_neutral());
    assert_eq!(with_spikes(0.0, 90.0).clamped(), AdjustValues::NEUTRAL);
    assert!(!with_spikes(5.0, 50.0).is_neutral());
    assert_eq!(with_spikes(5.0, 10.0).clamped().spikes_reach, 10.0);
}

#[test]
fn the_spikes_need_the_star_layer() {
    assert!(with_spikes(40.0, 50.0).needs_stars());
    assert!(!with_spikes(0.0, 50.0).needs_stars());
    assert!(
        AdjustValues {
            stars: -20.0,
            ..AdjustValues::NEUTRAL
        }
        .needs_stars()
    );
}

#[test]
fn a_record_of_before_the_spikes_reads_back_without_them() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    set(&render, values());
    let path = record_path(&render).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    // Written with no `spikes` field at all, as the records of before are.
    let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();
    let held = json["values"].as_object_mut().unwrap();
    held.remove("spikes");
    held.remove("spikesReach");
    fs::write(&path, serde_json::to_string(&json).unwrap()).unwrap();
    let state = state(&render).expect("the record of before is still valid");
    assert_eq!(state.values.spikes, 0.0);
    assert_eq!(state.values.spikes_reach, 50.0);
    assert_eq!(state.values.colour, 1.7);
}

#[test]
fn the_spikes_are_written_into_the_record_and_into_a_saved_setting() {
    let dir = tempfile_dir();
    let render = render_in(dir.path());
    let chosen = AdjustValues {
        spikes: 60.0,
        spikes_reach: 40.0,
        ..values()
    };
    let written = set(&render, chosen);
    assert_eq!(written.values.spikes, 60.0);
    assert_eq!(written.values.spikes_reach, 40.0);
    // What a later reader (the portal's mention) finds in the record.
    let text = fs::read_to_string(record_path(&render).unwrap()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["values"]["spikes"], 60.0);
    assert_eq!(json["values"]["spikesReach"], 40.0);
    let again = state(&render).unwrap();
    assert_eq!(again.values, chosen);
    let mut list = Vec::new();
    save_named(&mut list, "Croix", chosen).unwrap();
    assert_eq!(list[0].values, chosen);
}

//! The resume of a night from what is online
//! (`tests/fixtures/online/`, invented) and what is here -- its state, the
//! step the wizard reopens at, the complements (a sky solution from the
//! local file of the same digest or from the original, the analysis that
//! replaces the one in place, the progression redone) -- and the original
//! fetched, resumed and checked. The fake service only.

#[path = "support/fake_service.rs"]
mod fake_service;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use astro_sync_core::gui_bridge::HttpPortalGuiTransport;
use astro_sync_core::night_online::NightOnline;
use astro_sync_core::night_resume::{
    AnalysisInputs, AnalysisPlan, FolderState, LocalImage, LocalTarget, Missing, NightLedger,
    NightStatus, ResumeLocal, SolveFrom, WizardStep, analysis_inputs, fetch_original, load_ledger,
    resume_night, save_ledger,
};
use astro_sync_core::service_wait::{WaitHooks, WakePolicy};
use astro_sync_core::upload_queue::{EntryTarget, QueueEntry, QueueState, ResultKind};
use fake_service::{FakeService, Reply, Wake, error_reply};

fn online(name: &str) -> NightOnline {
    serde_json::from_str(
        &fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/online")
                .join(name),
        )
        .unwrap(),
    )
    .unwrap()
}

fn target(id: &str, frames: u64) -> LocalTarget {
    LocalTarget {
        target_id: id.into(),
        has_folder: true,
        light_frames: frames,
    }
}

fn final_image(target: &str, sha: char) -> LocalImage {
    LocalImage {
        target_id: target.into(),
        path: PathBuf::from(format!("/n/{target}/final/{target}.tif")),
        sha256: sha.to_string().repeat(64),
        size: 10,
        kind: ResultKind::Final,
        twin_sha256: None,
    }
}

fn inputs(fingerprint: &str, latest: &str) -> Option<AnalysisInputs> {
    Some(AnalysisInputs {
        fingerprint: fingerprint.into(),
        files: 12,
        latest_modified: Some(latest.into()),
    })
}

fn here(images: Vec<LocalImage>) -> ResumeLocal {
    ResumeLocal {
        folder: FolderState::Found,
        targets: vec![
            target("ngc7000", 40),
            target("m31", 20),
            target("ic1396", 0),
        ],
        logs: 2,
        images,
        analysis_inputs: inputs("after", "2026-10-07T23:00:00Z"),
    }
}

#[test]
fn a_night_with_nothing_online_nor_here_is_not_started() {
    let night = online("night_bare.json");
    let empty = ResumeLocal {
        folder: FolderState::Missing,
        targets: vec![],
        logs: 0,
        images: vec![],
        analysis_inputs: None,
    };
    let resume = resume_night(&night, &empty, &[], None, 0);
    assert_eq!(resume.status, NightStatus::NotStarted);
    assert_eq!(resume.step, Some(WizardStep::Dossier));
    assert_eq!(resume.sentence(), "Not started");
    assert_eq!(resume.analysis, AnalysisPlan::Nothing);
}

#[test]
fn a_night_in_progress_says_what_is_missing_and_reopens_at_files() {
    let night = online("night_in_progress.json");
    // NGC 7000's final is the one online (same digest); M31 has only the
    // phone's image online; IC 1396 has nothing anywhere.
    let local = here(vec![final_image("ngc7000", 'a')]);
    let ledger = NightLedger {
        analysis_inputs: Some("before".into()),
        progression_frames: BTreeMap::new(),
    };
    let resume = resume_night(&night, &local, &[], Some(&ledger), 12);
    assert_eq!(resume.status, NightStatus::ToComplete);
    assert_eq!(resume.step, Some(WizardStep::Fichiers));
    assert_eq!(
        resume.sentence(),
        "To complete: final image of IC 1396 · logs added to the folder · sky annotations of 1 image"
    );
    assert_eq!(
        resume.analysis,
        AnalysisPlan::Replace {
            previous_sent_at: "2026-10-05T08:00:00.000Z".into()
        }
    );
    assert_eq!(
        resume.analysis.sentence().unwrap(),
        "The night's analysis will be recomputed and will replace the one of 5 Oct."
    );
    // The phone's image of M31 has no solution and no file here: from its
    // original.
    assert_eq!(
        resume.sky_solutions,
        [SolveFrom::Original {
            capture_id: "33333333-3333-4333-8333-333333333333".into(),
            image_id: "43333333-3333-4333-8333-333333333333".into(),
            sha256: "b".repeat(64),
            original_name: "IMG_0412.jpg".into(),
            byte_size: 9_000_000
        }]
    );
}

#[test]
fn a_sky_solution_is_completed_from_the_local_file_of_the_same_digest() {
    let night = online("night_in_progress.json");
    let mut copy = final_image("m31", 'b');
    copy.path = PathBuf::from("/n/M31/final/IMG_0412.jpg");
    let local = here(vec![final_image("ngc7000", 'a'), copy]);
    let resume = resume_night(&night, &local, &[], None, 12);
    assert!(matches!(
        &resume.sky_solutions[0],
        SolveFrom::Local { path, .. } if path == Path::new("/n/M31/final/IMG_0412.jpg")
    ));
    // Sent from another computer, no file dated after the analysis: it stays.
    let older = ResumeLocal {
        analysis_inputs: inputs("x", "2026-10-04T23:00:00Z"),
        ..local.clone()
    };
    assert_eq!(
        resume_night(&night, &older, &[], None, 12).analysis,
        AnalysisPlan::UpToDate
    );
    // A file dated after it: it is rebuilt and replaces it.
    assert!(matches!(
        resume_night(&night, &local, &[], None, 12).analysis,
        AnalysisPlan::Replace { .. }
    ));
}

#[test]
fn a_complete_night_has_no_step_and_a_changed_one_redoes_its_progression() {
    let mut night = online("night_in_progress.json");
    // Every image solved, an image online for each target.
    for target in &mut night.targets {
        for capture in &mut target.captures {
            for image in &mut capture.images {
                image.sky_solution = "present".into();
            }
        }
    }
    let ic = night.targets[0].captures[0].clone();
    night.targets[2].captures.push(ic);
    let local = here(vec![final_image("ngc7000", 'a')]);
    let ledger = NightLedger {
        analysis_inputs: Some("after".into()),
        progression_frames: BTreeMap::from([("ngc7000".to_string(), 40)]),
    };
    let resume = resume_night(&night, &local, &[], Some(&ledger), 12);
    assert_eq!(resume.status, NightStatus::Complete, "{:?}", resume.missing);
    assert_eq!(resume.step, None);
    assert_eq!(resume.sentence(), "Complete");

    let more_frames = NightLedger {
        progression_frames: BTreeMap::from([("ngc7000".to_string(), 32)]),
        ..ledger
    };
    let resume = resume_night(&night, &local, &[], Some(&more_frames), 12);
    assert_eq!(
        resume.missing,
        [Missing::Progression {
            target_id: "ngc7000".into()
        }]
    );
    assert_eq!(resume.step, Some(WizardStep::Traitements));
}

fn row(sha: char, state: QueueState, session: &str) -> QueueEntry {
    QueueEntry {
        id: 1,
        source_path: PathBuf::from("/n/IC 1396/final/ic1396.tif"),
        sha256: sha.to_string().repeat(64),
        size: 10,
        upload_id: None,
        offset: 0,
        state,
        capture_id: None,
        last_error: None,
        last_error_code: None,
        created_at: String::new(),
        updated_at: String::new(),
        kind: ResultKind::Final,
        target: EntryTarget {
            session_id: Some(session.into()),
            ..EntryTarget::default()
        },
    }
}

#[test]
fn images_to_send_and_the_queue_reopen_the_summary() {
    let mut night = online("night_in_progress.json");
    for target in &mut night.targets {
        for capture in &mut target.captures {
            for image in &mut capture.images {
                image.sky_solution = "present".into();
            }
        }
    }
    let ledger = NightLedger {
        analysis_inputs: Some("after".into()),
        progression_frames: BTreeMap::new(),
    };
    // IC 1396's final is here, not online yet; a different M31 final is a
    // variant only when the space keeps variants.
    let local = here(vec![
        final_image("ngc7000", 'a'),
        final_image("ic1396", 'd'),
        final_image("m31", 'c'),
    ]);
    let with_variants = resume_night(&night, &local, &[], Some(&ledger), 12);
    assert_eq!(with_variants.missing, [Missing::ImagesToSend { count: 2 }]);
    assert_eq!(with_variants.step, Some(WizardStep::Recapitulatif));
    let without = resume_night(&night, &local, &[], Some(&ledger), 0);
    assert_eq!(without.missing, [Missing::ImagesToSend { count: 1 }]);

    // Once enqueued (held by a full storage), the queue says it.
    let queue = [
        row('d', QueueState::Held, &night.session_id),
        row('e', QueueState::Queued, "another-night"),
    ];
    let resume = resume_night(&night, &local, &queue, Some(&ledger), 0);
    assert_eq!(resume.missing, [Missing::QueuePending { count: 1 }]);
}

#[test]
fn what_the_analysis_reads_changes_with_a_log_not_with_a_final_image() {
    let root = std::env::temp_dir().join(format!("astro-sync-inputs-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    for folder in ["logs", "M31/lights", "M31/final"] {
        fs::create_dir_all(root.join(folder)).unwrap();
    }
    fs::write(root.join("M31/lights/l-0001.fits"), b"frame").unwrap();
    let first = analysis_inputs(&root).unwrap();
    assert_eq!(first.files, 1);
    fs::write(root.join("M31/final/m31.tif"), b"final").unwrap();
    assert_eq!(analysis_inputs(&root).unwrap(), first);
    fs::write(root.join("logs/PHD2_GuideLog.txt"), b"log").unwrap();
    let second = analysis_inputs(&root).unwrap();
    assert_ne!(second.fingerprint, first.fingerprint);
    assert_eq!(second.files, 2);
    assert!(second.latest_modified.is_some());

    let ledgers = root.join("data/nights");
    assert_eq!(load_ledger(&ledgers, "night-1"), None);
    let ledger = NightLedger {
        analysis_inputs: Some(second.fingerprint.clone()),
        progression_frames: BTreeMap::from([("m31".to_string(), 1)]),
    };
    save_ledger(&ledgers, "night-1", &ledger).unwrap();
    assert_eq!(load_ledger(&ledgers, "night-1"), Some(ledger));
    let _ = fs::remove_dir_all(&root);
}

/// C4: the candidates of a night's folder carry the pipeline the stacker
/// recorded beside its renders, so that the natural render is the capture's
/// main image whatever the order of the file names.
#[test]
fn the_renders_of_a_folder_carry_the_pipeline_the_stacker_recorded() {
    let folder = std::env::temp_dir().join(format!("astro-sync-recorded-{}", std::process::id()));
    let _ = fs::remove_dir_all(&folder);
    let stacked = folder.join("IC 1805/stacked");
    fs::create_dir_all(&stacked).unwrap();
    fs::write(stacked.join("IC 1805_dualband-correction.tif"), "d").unwrap();
    fs::write(stacked.join("IC 1805_raw.tif"), "r").unwrap();
    fs::write(
        stacked.join("IC 1805_stacking.json"),
        r#"{"outputs":[{"pipeline":"dualband-correction","file":"IC 1805_dualband-correction.tif"},{"pipeline":"raw","file":"IC 1805_raw.tif"}]}"#,
    )
    .unwrap();
    let image = |name: &str, sha: char| LocalImage {
        target_id: "ic-1805".into(),
        path: stacked.join(name),
        sha256: sha.to_string().repeat(64),
        size: 1,
        kind: ResultKind::Stacked,
        twin_sha256: None,
    };
    let local = here(vec![
        image("IC 1805_dualband-correction.tif", 'd'),
        image("IC 1805_raw.tif", 'r'),
    ]);
    let renders: Vec<Option<String>> = local.candidates().into_iter().map(|c| c.render).collect();
    assert_eq!(
        renders,
        [
            Some("dualband-correction".to_string()),
            Some("raw".to_string())
        ]
    );
    // A final image processed elsewhere has no render.
    let finals = here(vec![final_image("m31", 'a')]);
    assert_eq!(finals.candidates()[0].render, None);
    let _ = fs::remove_dir_all(&folder);
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
fn the_original_is_fetched_resumed_from_its_part_and_checked() {
    use sha2::{Digest, Sha256};
    // The fake answers text: the bytes are kept ASCII so the round trip is
    // exact.
    let ascii: Vec<u8> = (0..200u8).map(|b| b % 90 + 32).collect();
    let ascii_sha = format!("{:x}", Sha256::digest(&ascii));
    let calls = Arc::new(AtomicUsize::new(0));
    let served = ascii.clone();
    let seen = Arc::clone(&calls);
    let service = FakeService::start(Wake::Instant, move |request| {
        if request.method == "GET" && request.path.starts_with("/api/v1/captures/cap-1/original") {
            seen.fetch_add(1, Ordering::SeqCst);
            assert!(request.path.contains("imageId=img-1"));
            let text = |from: usize| String::from_utf8(served[from..].to_vec()).unwrap();
            return match request.header("range") {
                Some(range) => Reply::Text(
                    206,
                    text(
                        range
                            .trim_start_matches("bytes=")
                            .trim_end_matches('-')
                            .parse()
                            .unwrap(),
                    ),
                ),
                None => Reply::Text(200, text(0)),
            };
        }
        error_reply(404, "not_found", "no such route")
    });
    let transport = HttpPortalGuiTransport::with_options(fast(), WaitHooks::silent()).unwrap();
    let dir = std::env::temp_dir().join(format!("astro-sync-originals-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let source = SolveFrom::Original {
        capture_id: "cap-1".into(),
        image_id: "img-1".into(),
        sha256: ascii_sha.clone(),
        original_name: "IMG_0412.JPG".into(),
        byte_size: ascii.len() as u64,
    };
    // A part already fetched: only the rest is asked for.
    fs::write(dir.join(format!("{ascii_sha}.part")), &ascii[..80]).unwrap();
    let path = fetch_original(&transport, &service.base_url(), "t", &source, &dir).unwrap();
    assert_eq!(path, dir.join(format!("{ascii_sha}.jpg")));
    assert_eq!(fs::read(&path).unwrap(), ascii);
    let ranged: Vec<_> = service
        .requests()
        .into_iter()
        .filter(|r| r.reached_app)
        .map(|r| r.header("range").map(str::to_string))
        .collect();
    assert_eq!(ranged, [Some("bytes=80-".to_string())]);
    // Already there: no request.
    fetch_original(&transport, &service.base_url(), "t", &source, &dir).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // A digest that does not match: refused, the part dropped.
    let wrong = SolveFrom::Original {
        capture_id: "cap-1".into(),
        image_id: "img-1".into(),
        sha256: "0".repeat(64),
        original_name: "x.tif".into(),
        byte_size: 1,
    };
    let error = fetch_original(&transport, &service.base_url(), "t", &wrong, &dir).unwrap_err();
    assert_eq!(error.code(), "portal_refused");
    assert!(!dir.join(format!("{}.part", "0".repeat(64))).exists());
    // A local source needs no download.
    let local = SolveFrom::Local {
        capture_id: "c".into(),
        image_id: "i".into(),
        sha256: "a".repeat(64),
        path: PathBuf::from("/n/M31/final/m31.tif"),
    };
    assert_eq!(
        fetch_original(&transport, &service.base_url(), "t", &local, &dir).unwrap(),
        PathBuf::from("/n/M31/final/m31.tif")
    );
    let _ = fs::remove_dir_all(&dir);
}

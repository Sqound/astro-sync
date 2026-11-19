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
    NightLeftOut, NightStatus, PlanOnline, ResumeLocal, SolveFrom, WizardStep, analysis_inputs,
    fetch_original, load_ledger, plans_online, resume_night, resume_night_with_plans, save_ledger,
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

/// « N images to send », one per target named.
fn to_send(targets: &[&str]) -> Missing {
    Missing::ImagesToSend {
        count: targets.len() as u64,
        targets: targets.iter().map(|t| (*t).to_string()).collect(),
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
    // IC 1396's final is here, not online yet: its target has no image
    // online. A different M31 final is a choice, never owed.
    let local = here(vec![
        final_image("ngc7000", 'a'),
        final_image("ic1396", 'd'),
        final_image("m31", 'c'),
    ]);
    let with_variants = resume_night(&night, &local, &[], Some(&ledger), 12);
    assert_eq!(with_variants.missing, [to_send(&["ic1396"])]);
    assert_eq!(with_variants.step, Some(WizardStep::Recapitulatif));
    let without = resume_night(&night, &local, &[], Some(&ledger), 0);
    assert_eq!(without.missing, [to_send(&["ic1396"])]);

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

/* ----------- Renders produced here are choices, not debts ----------- */

fn render(target: &str, sha: char, ext: &str, twin: Option<char>) -> LocalImage {
    LocalImage {
        target_id: target.into(),
        path: PathBuf::from(format!("/n/{target}/stacked/{target}_{sha}.{ext}")),
        sha256: sha.to_string().repeat(64),
        size: 10,
        kind: ResultKind::Stacked,
        twin_sha256: twin.map(|c| c.to_string().repeat(64)),
    }
}

/// A night with one planned target (NGC 7000) whose render `a` is online,
/// its sky annotated, its analysis up to date.
fn settled() -> (NightOnline, NightLedger) {
    let mut night = online("night_in_progress.json");
    night
        .targets
        .retain(|t| t.target_id.as_deref() == Some("ngc7000"));
    let ledger = NightLedger {
        analysis_inputs: Some("after".into()),
        progression_frames: BTreeMap::new(),
    };
    (night, ledger)
}

/// Several renders of one integration, in PNG and TIFF, only one sent.
fn many_renders() -> Vec<LocalImage> {
    vec![
        render("ngc7000", 'a', "png", Some('1')),
        render("ngc7000", '1', "tif", Some('a')),
        render("ngc7000", 'b', "png", Some('2')),
        render("ngc7000", '2', "tif", Some('b')),
        render("ngc7000", 'c', "png", None),
        render("ngc7000", 'd', "png", None),
    ]
}

#[test]
fn renders_not_sent_do_not_keep_a_night_to_complete() {
    let (night, ledger) = settled();
    let local = here(many_renders());
    for limit in [0, 12] {
        let resume = resume_night(&night, &local, &[], Some(&ledger), limit);
        assert_eq!(resume.missing, [], "variant limit {limit}");
        assert_eq!(resume.status, NightStatus::Complete);
    }
}

#[test]
fn a_target_with_no_image_online_still_has_an_image_to_send() {
    let mut night = online("night_in_progress.json");
    night
        .targets
        .retain(|t| t.target_id.as_deref() == Some("ic1396"));
    let ledger = NightLedger {
        analysis_inputs: Some("after".into()),
        progression_frames: BTreeMap::new(),
    };
    let local = here(
        many_renders()
            .into_iter()
            .map(|mut i| {
                i.target_id = "ic1396".into();
                i
            })
            .collect(),
    );
    let resume = resume_night(&night, &local, &[], Some(&ledger), 12);
    // One image is owed for the target, not one per render.
    assert_eq!(resume.missing, [to_send(&["ic1396"])]);
    assert_eq!(resume.status, NightStatus::ToComplete);
}

#[test]
fn a_ticked_render_still_in_the_queue_is_pending_not_to_send() {
    let mut night = online("night_in_progress.json");
    night
        .targets
        .retain(|t| t.target_id.as_deref() == Some("ic1396"));
    let ledger = NightLedger {
        analysis_inputs: Some("after".into()),
        progression_frames: BTreeMap::new(),
    };
    let local = here(vec![
        render("ic1396", 'e', "png", None),
        render("ic1396", 'f', "png", None),
    ]);
    let queue = [row('e', QueueState::Queued, &night.session_id)];
    let resume = resume_night(&night, &local, &queue, Some(&ledger), 12);
    assert_eq!(resume.missing, [Missing::QueuePending { count: 1 }]);
}

#[test]
fn a_failed_or_refused_sending_is_said() {
    let (night, ledger) = settled();
    let local = here(many_renders());
    let queue = [
        row('c', QueueState::Failed, &night.session_id),
        row('d', QueueState::Refused, &night.session_id),
        // Another night's, and one whose bytes are online meanwhile.
        row('b', QueueState::Failed, "another-night"),
        row('a', QueueState::Failed, &night.session_id),
    ];
    let resume = resume_night(&night, &local, &queue, Some(&ledger), 12);
    assert_eq!(resume.missing, [Missing::QueueFailed { count: 2 }]);
    assert_eq!(resume.sentence(), "To complete: 2 uploads not sent");
}

#[test]
fn sky_annotations_are_owed_for_images_online_only() {
    let (mut night, ledger) = settled();
    let local = here(many_renders());
    night.targets[0].captures[0].images[0].sky_solution = "missing".into();
    let resume = resume_night(&night, &local, &[], Some(&ledger), 12);
    // The renders never sent add nothing: one image online lacks it.
    assert_eq!(
        resume.missing,
        [Missing::SkyAnnotations {
            count: 1,
            targets: vec!["ngc7000".into()]
        }]
    );
}

/* ------ The image of a multi-night plan is its nights' image online ------ */

const PLAN: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const NIGHTS: [&str; 3] = [
    "bbbbbbbb-0000-4000-8000-000000000001",
    "bbbbbbbb-0000-4000-8000-000000000002",
    "bbbbbbbb-0000-4000-8000-000000000003",
];

/// The multi-night plan of IC 1396 as the service answers it
/// (`GET /v1/projects/{id}`), its image online or not: a capture of the plan,
/// which no night's `…/online` lists. Read for the night `session`.
fn plan_of_ic1396(combined: Option<&[&str]>, session: &str) -> Vec<PlanOnline> {
    plan_image_of_ic1396(
        combined.map(|nights| serde_json::json!(nights)),
        session,
        Some("2026-08-24T19:30:00.000Z"),
    )
}

/// The same plan, its image online naming `combined` (an array, or nothing),
/// made on 20 Aug, for the night `session` that began at `started_at`.
fn plan_image_of_ic1396(
    combined: Option<serde_json::Value>,
    session: &str,
    started_at: Option<&str>,
) -> Vec<PlanOnline> {
    let mut capture = serde_json::json!({
        "id": "cccccccc-0000-4000-8000-000000000001",
        "createdAt": "2026-08-20T10:00:00.000Z",
        "totalsByFilter": [],
        "render": "natural"
    });
    if let Some(nights) = combined {
        capture["combinedNightIds"] = nights;
    }
    let project = serde_json::json!({
        "id": PLAN,
        "target": { "targetId": "ic1396", "name": "IC 1396" },
        "integrations": [],
        "planCapture": capture,
    });
    plans_online(
        &astro_sync_core::multi_night::NightMultiNight {
            evening_date: None,
            projects: astro_sync_core::multi_night::project_of(&project, "ic1396")
                .into_iter()
                .collect(),
        },
        session,
        started_at,
    )
}

/// The plan of IC 1396 without its image online.
fn plan_of_ic1396_without_image(session: &str) -> Vec<PlanOnline> {
    let project = serde_json::json!({
        "id": PLAN,
        "target": { "targetId": "ic1396", "name": "IC 1396" },
        "integrations": [],
        "planCapture": null,
    });
    plans_online(
        &astro_sync_core::multi_night::NightMultiNight {
            evening_date: None,
            projects: astro_sync_core::multi_night::project_of(&project, "ic1396")
                .into_iter()
                .collect(),
        },
        session,
        None,
    )
}

/// One night of the plan: `targets` planned, nothing of IC 1396 online for
/// the night itself, its analysis in place.
fn plan_night(session: &str, targets: &[&str]) -> (NightOnline, NightLedger) {
    let mut night = online("night_in_progress.json");
    night.session_id = session.into();
    night
        .targets
        .retain(|t| targets.contains(&t.target_id.as_deref().unwrap_or_default()));
    let ledger = NightLedger {
        analysis_inputs: Some("after".into()),
        progression_frames: BTreeMap::new(),
    };
    (night, ledger)
}

#[test]
fn the_three_nights_of_a_plan_whose_image_is_online_are_complete() {
    // The image of the plan combined the three nights; each night kept its
    // own render in its folder, never sent.
    for session in NIGHTS {
        let plans = plan_of_ic1396(Some(&NIGHTS), session);
        let (night, ledger) = plan_night(session, &["ic1396"]);
        let local = here(vec![render("ic1396", 'e', "png", None)]);
        // Without the plan, the night's own render is asked for.
        let alone = resume_night(&night, &local, &[], Some(&ledger), 12);
        assert_eq!(alone.missing, [to_send(&["ic1396"])]);
        let resume = resume_night_with_plans(&night, &local, &[], Some(&ledger), 12, &plans);
        assert_eq!(resume.missing, [], "night {session}");
        assert_eq!(resume.status, NightStatus::Complete);
        assert_eq!(resume.step, None);
    }
}

#[test]
fn a_night_the_image_of_its_plan_did_not_combine_owes_no_image_either() {
    // The plan has its image online, made before this night joined it: what
    // is owed then is the plan's image again, never an image of the night.
    let plans = plan_of_ic1396(Some(&NIGHTS[..2]), NIGHTS[2]);
    let (night, ledger) = plan_night(NIGHTS[2], &["ic1396"]);
    let local = here(vec![render("ic1396", 'e', "png", None)]);
    let resume = resume_night_with_plans(&night, &local, &[], Some(&ledger), 12, &plans);
    assert_eq!(resume.status, NightStatus::Complete);
    assert_eq!(resume.missing, []);
}

/* -- A night the image of its plan does not hold: complete, and said so -- */

#[test]
fn the_nights_the_image_of_the_plan_combined_are_complete_without_reserve() {
    // The image combined the first two nights: they are complete, nothing
    // said beside.
    for session in &NIGHTS[..2] {
        let plans = plan_of_ic1396(Some(&NIGHTS[..2]), session);
        assert_eq!(plans[0].night_left_out, None);
        let (night, ledger) = plan_night(session, &["ic1396"]);
        let local = here(vec![render("ic1396", 'e', "png", None)]);
        let resume = resume_night_with_plans(&night, &local, &[], Some(&ledger), 12, &plans);
        assert_eq!(resume.status, NightStatus::Complete);
        assert_eq!(resume.plans_left_out, []);
        assert_eq!(resume.step, None);
        assert_eq!(resume.sentence(), "Complete");
    }
}

#[test]
fn a_night_the_image_of_the_plan_did_not_combine_is_said_left_out() {
    // Criterion (a): the nights the image combined do not name the third.
    let plans = plan_of_ic1396(Some(&NIGHTS[..2]), NIGHTS[2]);
    assert_eq!(plans[0].night_left_out, Some(NightLeftOut::NotCombined));
    let (night, ledger) = plan_night(NIGHTS[2], &["ic1396"]);
    let local = here(vec![render("ic1396", 'e', "png", None)]);
    let resume = resume_night_with_plans(&night, &local, &[], Some(&ledger), 12, &plans);
    // Nothing owed: not « To complete ».
    assert_eq!(resume.status, NightStatus::Complete);
    assert_eq!(resume.missing, []);
    assert_eq!(resume.plans_left_out.len(), 1);
    assert_eq!(resume.plans_left_out[0].project_id, PLAN);
    // The wizard reopens where the image of the plan is made again.
    assert_eq!(resume.step, Some(WizardStep::Traitements));
    assert_eq!(
        resume.sentence(),
        "Complete: the image of the plan of IC 1396 does not hold this night"
    );
}

#[test]
fn the_combined_nights_win_over_the_dates() {
    // A night that began after the image was made, but which the image
    // names (the clocks disagree): it holds the night.
    let plans = plan_image_of_ic1396(
        Some(serde_json::json!(NIGHTS)),
        NIGHTS[2],
        Some("2026-08-24T19:30:00.000Z"),
    );
    assert_eq!(plans[0].night_left_out, None);
    // And a night the image does not name, though it began before the
    // image was made: left out.
    let plans = plan_image_of_ic1396(
        Some(serde_json::json!(&NIGHTS[..1])),
        NIGHTS[1],
        Some("2026-08-12T19:30:00.000Z"),
    );
    assert_eq!(plans[0].night_left_out, Some(NightLeftOut::NotCombined));
}

#[test]
fn an_image_naming_no_night_and_older_than_the_night_may_not_hold_it() {
    // Criterion (b), when the image names no night: made on 20 Aug.
    for combined in [None, Some(serde_json::json!([]))] {
        // A night that began after: it may not hold it.
        let later = plan_image_of_ic1396(
            combined.clone(),
            NIGHTS[2],
            Some("2026-08-24T19:30:00.000Z"),
        );
        assert_eq!(later[0].night_left_out, Some(NightLeftOut::MadeBefore));
        let (night, ledger) = plan_night(NIGHTS[2], &["ic1396"]);
        let local = here(vec![render("ic1396", 'e', "png", None)]);
        let resume = resume_night_with_plans(&night, &local, &[], Some(&ledger), 12, &later);
        assert_eq!(resume.status, NightStatus::Complete);
        assert_eq!(resume.plans_left_out.len(), 1);
        assert_eq!(
            resume.sentence(),
            "Complete: the image of the plan of IC 1396 may not hold this night"
        );
        // A night that began before: nothing said.
        let earlier = plan_image_of_ic1396(
            combined.clone(),
            NIGHTS[0],
            Some("2026-08-09T19:30:00.000Z"),
        );
        assert_eq!(earlier[0].night_left_out, None);
        // The start of the night unknown: nothing said.
        let unknown = plan_image_of_ic1396(combined, NIGHTS[2], None);
        assert_eq!(unknown[0].night_left_out, None);
    }
}

#[test]
fn a_night_still_owing_an_image_stays_to_complete_with_its_plan_left_out() {
    // The third night owes M31 an image: « To complete » comes first, the
    // plan left out is kept beside.
    let plans = plan_of_ic1396(Some(&NIGHTS[..2]), NIGHTS[2]);
    let (mut night, ledger) = plan_night(NIGHTS[2], &["ic1396", "m31"]);
    for target in &mut night.targets {
        target.captures.clear();
    }
    let local = here(vec![
        render("ic1396", 'e', "png", None),
        render("m31", 'f', "png", None),
    ]);
    let resume = resume_night_with_plans(&night, &local, &[], Some(&ledger), 12, &plans);
    assert_eq!(resume.status, NightStatus::ToComplete);
    assert_eq!(resume.missing, [to_send(&["m31"])]);
    assert_eq!(resume.step, Some(WizardStep::Recapitulatif));
    assert_eq!(resume.plans_left_out.len(), 1);
}

#[test]
fn a_plan_target_with_nothing_here_owes_no_final_image() {
    let plans = plan_of_ic1396(Some(&NIGHTS), NIGHTS[0]);
    let (night, ledger) = plan_night(NIGHTS[0], &["ic1396"]);
    let local = here(vec![]);
    let alone = resume_night(&night, &local, &[], Some(&ledger), 12);
    assert!(matches!(alone.missing[..], [Missing::FinalImage { .. }]));
    let resume = resume_night_with_plans(&night, &local, &[], Some(&ledger), 12, &plans);
    assert_eq!(resume.missing, []);
}

#[test]
fn a_plan_without_its_image_online_leaves_the_night_to_complete() {
    let plans = plan_of_ic1396_without_image(NIGHTS[0]);
    assert!(!plans[0].image_online);
    assert_eq!(plans[0].night_left_out, None);
    let (night, ledger) = plan_night(NIGHTS[0], &["ic1396"]);
    let local = here(vec![render("ic1396", 'e', "png", None)]);
    let resume = resume_night_with_plans(&night, &local, &[], Some(&ledger), 12, &plans);
    assert_eq!(resume.missing, [to_send(&["ic1396"])]);
}

#[test]
fn a_second_target_of_a_plan_night_is_still_owed_and_named() {
    // A night of the plan that also shot NGC 7000 (its image online) and M31
    // (a render here, nothing online): only M31 is owed.
    let plans = plan_of_ic1396(Some(&NIGHTS[..2]), NIGHTS[1]);
    let (mut night, ledger) = plan_night(NIGHTS[1], &["ic1396", "m31"]);
    for target in &mut night.targets {
        target.captures.clear();
    }
    let local = here(vec![
        render("ic1396", 'e', "png", None),
        render("m31", 'f', "png", None),
    ]);
    let resume = resume_night_with_plans(&night, &local, &[], Some(&ledger), 12, &plans);
    assert_eq!(resume.missing, [to_send(&["m31"])]);
    assert_eq!(resume.status, NightStatus::ToComplete);
    assert_eq!(resume.step, Some(WizardStep::Recapitulatif));
}

#[test]
fn a_single_night_without_image_online_names_its_target() {
    let (night, ledger) = plan_night(NIGHTS[0], &["ic1396"]);
    let local = here(vec![render("ic1396", 'e', "png", None)]);
    // The plan of another target changes nothing.
    let mut other = plan_of_ic1396(Some(&NIGHTS), NIGHTS[0]);
    other[0].target_id = "m31".into();
    let resume = resume_night_with_plans(&night, &local, &[], Some(&ledger), 12, &other);
    assert_eq!(resume.missing, [to_send(&["ic1396"])]);
}

#[test]
fn a_short_night_with_a_render_here_and_nothing_online_stays_to_complete() {
    // Six minutes of poses, a render the stacker wrote all the same, no log:
    // the engine cannot tell the render is of no use, so it names the target
    // and the person decides.
    let (mut night, _) = plan_night(NIGHTS[0], &["ic1396"]);
    night.analysis.present = false;
    let local = ResumeLocal {
        folder: FolderState::Found,
        targets: vec![target("ic1396", 82)],
        logs: 0,
        images: vec![render("ic1396", 'e', "png", None)],
        analysis_inputs: None,
    };
    let resume = resume_night(&night, &local, &[], None, 12);
    assert_eq!(resume.missing, [to_send(&["ic1396"])]);
    assert_eq!(resume.status, NightStatus::ToComplete);
}

/// NGC 7000 online with its main image and three variants.
fn with_variants(solved: [&str; 4]) -> (NightOnline, NightLedger) {
    let (mut night, ledger) = settled();
    let capture = &mut night.targets[0].captures[0];
    let main = capture.images[0].clone();
    for (index, sha) in ['7', '8', '9'].into_iter().enumerate() {
        let mut variant = main.clone();
        variant.id = format!("variant-{index}");
        variant.main = false;
        variant.sha256 = sha.to_string().repeat(64);
        capture.images.push(variant);
    }
    capture.image_count = 4;
    for (image, state) in capture.images.iter_mut().zip(solved) {
        image.sky_solution = state.into();
    }
    capture.main_image = Some(capture.images[0].clone());
    (night, ledger)
}

#[test]
fn sky_annotations_are_counted_on_the_main_image_of_each_capture() {
    // « Online » says « 1 image »: one image lacks its annotations, not four.
    let (night, ledger) = with_variants(["missing"; 4]);
    let local = here(many_renders());
    let resume = resume_night(&night, &local, &[], Some(&ledger), 12);
    assert_eq!(
        resume.missing,
        [Missing::SkyAnnotations {
            count: 1,
            targets: vec!["ngc7000".into()]
        }]
    );
    assert_eq!(resume.sentence(), "To complete: sky annotations of 1 image");
    // The pass that solves it solves the variants too.
    assert_eq!(resume.sky_solutions.len(), 4);
}

#[test]
fn a_variant_without_annotations_does_not_keep_a_night_to_complete() {
    let (night, ledger) = with_variants(["present", "present", "missing", "present"]);
    let local = here(many_renders());
    let resume = resume_night(&night, &local, &[], Some(&ledger), 12);
    assert_eq!(resume.missing, []);
    assert_eq!(resume.status, NightStatus::Complete);
    assert_eq!(resume.sky_solutions.len(), 1);
}

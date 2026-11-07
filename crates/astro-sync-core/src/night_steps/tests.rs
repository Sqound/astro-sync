use super::*;
use crate::detection::{DetectOptions, DetectionInput, DetectionTarget, detect_night};
use crate::gui_bridge::Capabilities;
use crate::precheck::{CheckStorage, Decision, NotSent, UploadCheck};
use crate::sky_solution::{SkySolutionPut, SolveFailure, Solvers, WrittenSkySolution};
use crate::test_support::{TempDir, fits_bytes};
use crate::uploader::UploadMetadata;
use crate::working_folder::{NightFolderSpec, NightTargetSpec, prepare_night_folder};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::fs;

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// A night with M31: two final images (the newer proposed), a stacked
/// render, a light frame.
fn night(root: &Path) -> (PathBuf, NightDetection) {
    let spec = NightFolderSpec {
        night_id: "7f3a2c91-steps".to_string(),
        service_url: "https://service.example".to_string(),
        evening_date: "2026-10-07".to_string(),
        label: "Col des Étoiles".to_string(),
        targets: vec![NightTargetSpec {
            target_id: "t-m31".to_string(),
            designation: "M31".to_string(),
            ticked: true,
        }],
        follow: None,
    };
    let folder = prepare_night_folder(root, &[], &spec, "2026-10-08T09:00:00Z")
        .unwrap()
        .folder;
    fs::write(folder.join("M31/final/M31_old.jpg"), b"old final").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::write(folder.join("M31/final/M31.tif"), b"new final").unwrap();
    fs::create_dir_all(folder.join("M31/stacked")).unwrap();
    fs::write(
        folder.join("M31/stacked/M31_2026-10-07_raw.tif"),
        b"stacked",
    )
    .unwrap();
    fs::write(folder.join("M31/stacked/M31_2026-10-07_raw.png"), b"png").unwrap();
    fs::write(
        folder.join("M31/lights/Light_0001.fits"),
        fits_bytes(&[("IMAGETYP", "'LIGHT'"), ("OBJECT", "'M31'")], 0),
    )
    .unwrap();
    let input = DetectionInput {
        evening_date: Some("2026-10-07".to_string()),
        night_started_at: None,
        longitude: None,
        targets: vec![DetectionTarget {
            target_id: "t-m31".to_string(),
            designation: "M31".to_string(),
            aliases: Vec::new(),
            ticked: true,
            online_final: false,
            stacking_planned: false,
        }],
    };
    let detection = detect_night(&folder, &input, &DetectOptions::default()).unwrap();
    (folder, detection)
}

#[test]
fn the_local_images_are_the_finals_and_the_stacked_renders_in_both_formats_with_their_digests() {
    let root = TempDir::new("steps-local");
    let (folder, detection) = night(root.path());
    let images = local_images(&folder, &detection, None, None).unwrap();
    let seen: Vec<(String, ResultKind)> = images
        .iter()
        .map(|i| {
            (
                i.path.file_name().unwrap().to_string_lossy().into_owned(),
                i.kind,
            )
        })
        .collect();
    assert_eq!(
        seen,
        vec![
            ("M31.tif".to_string(), ResultKind::Final),
            ("M31_old.jpg".to_string(), ResultKind::Final),
            ("M31_2026-10-07_raw.png".to_string(), ResultKind::Stacked),
            ("M31_2026-10-07_raw.tif".to_string(), ResultKind::Stacked),
        ]
    );
    assert_eq!(images[0].sha256, sha(b"new final"));
    assert_eq!(images[0].target_id, "t-m31");
    // The person's final TIFF is no render: it has no twin and always leaves.
    assert!(images[0].twin_sha256.is_none() && !images[0].is_full_resolution_twin());
    // The render's PNG and TIFF name each other; the TIFF is the twin.
    assert_eq!(
        images[2].twin_sha256.as_deref(),
        Some(sha(b"stacked").as_str())
    );
    assert_eq!(images[3].twin_sha256.as_deref(), Some(sha(b"png").as_str()));
    assert!(images[3].is_full_resolution_twin() && !images[2].is_full_resolution_twin());

    let local = resume_local(
        FolderState::Found,
        Some(&folder),
        Some(&detection),
        None,
        None,
    )
    .unwrap();
    assert_eq!(local.images.len(), 4);
    assert!(local.analysis_inputs.is_some());
    let material = night_material(Some(&detection), None);
    assert!(material.light_frames && material.final_image && !material.logs);
    let missing = resume_local(FolderState::Missing, None, None, None, None).unwrap();
    assert!(missing.images.is_empty());
}

fn online(sha256: &str, solution: &str) -> NightOnline {
    serde_json::from_value(serde_json::json!({
        "sessionId": "s-1",
        "targets": [{"targetId": "t-m31", "name": "M31", "planned": true, "captures": [{
            "id": "c-m31", "targetIds": ["t-m31"], "source": "phone", "stage": "draft",
            "visibility": "private", "uploaded": true, "createdAt": "2026-10-08T01:00:00Z",
            "mainImage": null, "imageCount": 1,
            "images": [{"id": "img-1", "main": true, "sha256": sha256, "byteSize": 9,
                        "originalName": "M31.tif", "source": "phone", "createdAt": "2026-10-08T01:00:00Z",
                        "skySolution": solution}]
        }]}],
        "analysis": {"present": false, "stepCount": 0, "editedStepCount": 0}
    }))
    .unwrap()
}

fn capabilities() -> Capabilities {
    serde_json::from_value(serde_json::json!({
        "planner.multiple": true, "planner.mosaic": true, "pointing.solve": true,
        "sync.plan_limit": 0, "gallery.private": true, "portal.invitations": true,
        "capture.variant_limit": 0, "capture.count_limit": 3, "capture.public": true,
        "analysis.logs": true, "analysis.progression": true,
        "storage.max_file_bytes": 100, "storage.quota_bytes": 1000, "history.retention_days": 0
    }))
    .unwrap()
}

struct Service {
    checks: RefCell<Vec<UploadMetadata>>,
}

impl PrecheckTransport for Service {
    fn space_storage(&self, _: &str, _: &str) -> Result<SpaceStorage, GuiBridgeError> {
        Ok(SpaceStorage {
            used_bytes: 10,
            quota_bytes: 1000,
            remaining_bytes: 990,
            max_file_bytes: 100,
            capture_count: 1,
            capture_count_limit: 3,
            variant_limit: 0,
            capabilities: capabilities(),
        })
    }
    fn check_upload(
        &self,
        _: &str,
        _: &str,
        metadata: &UploadMetadata,
    ) -> Result<UploadCheck, GuiBridgeError> {
        self.checks.borrow_mut().push(metadata.clone());
        Ok(UploadCheck::Refused {
            code: "storage_quota_reached".to_string(),
            message: "full".to_string(),
            storage: CheckStorage {
                used_bytes: 10,
                quota_bytes: 1000,
                max_file_bytes: 100,
                capture_count: 1,
                capture_count_limit: 3,
                variant_limit: 0,
                variant_count: None,
            },
        })
    }
}

#[test]
fn the_precheck_gathers_the_decisions_the_service_and_the_limits() {
    let root = TempDir::new("steps-precheck");
    let (folder, detection) = night(root.path());
    let local = resume_local(
        FolderState::Found,
        Some(&folder),
        Some(&detection),
        None,
        None,
    )
    .unwrap();
    let service = Service {
        checks: RefCell::new(Vec::new()),
    };
    // M31's capture is online with another image, the space keeps no variant.
    let check = precheck_night(
        &service,
        "http://s.invalid",
        "t",
        &online(&"0".repeat(64), "missing"),
        &local.candidates(),
    )
    .unwrap();
    assert!(!check.several_renders_allowed);
    assert!(check.plans.iter().any(|p| p.decision
        == Decision::NotSent {
            reason: NotSent::VariantsNotAllowed
        }));
    assert!(check.quota.allowed, "nothing leaves, nothing exceeds");
    assert!(service.checks.borrow().is_empty());

    // Nothing online: the stacked render is a new capture; the service
    // refuses it, so the sending is not allowed.
    let mut bare = online("x", "missing");
    bare.targets[0].captures.clear();
    let check = precheck_night(
        &service,
        "http://s.invalid",
        "t",
        &bare,
        &local.candidates(),
    )
    .unwrap();
    assert_eq!(service.checks.borrow().len(), 1);
    assert!(!check.quota.allowed);
    assert_eq!(check.quota.findings[0].code(), "storage_quota_reached");
}

struct Solving {
    put: RefCell<Vec<SkySolutionPut>>,
    fetched: RefCell<u32>,
}

impl SkySolutionTransport for Solving {
    fn put_sky_solution(
        &self,
        _: &str,
        _: &str,
        _: &str,
        body: &SkySolutionPut,
    ) -> Result<WrittenSkySolution, GuiBridgeError> {
        self.put.borrow_mut().push(body.clone());
        Ok(WrittenSkySolution {
            sky_solution: serde_json::Value::Null,
            created: true,
            changed: true,
        })
    }
    fn missing_page(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
    ) -> Result<serde_json::Value, GuiBridgeError> {
        Ok(serde_json::json!({"captures": []}))
    }
}

impl OriginalTransport for Solving {
    fn original(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
        _: u64,
    ) -> Result<(u16, Vec<u8>), GuiBridgeError> {
        *self.fetched.borrow_mut() += 1;
        Ok((200, b"new final".to_vec()))
    }
}

#[test]
fn the_missing_solutions_of_a_night_are_solved_from_the_local_file_first() {
    let root = TempDir::new("steps-solve");
    let (folder, detection) = night(root.path());
    let local = resume_local(
        FolderState::Found,
        Some(&folder),
        Some(&detection),
        None,
        None,
    )
    .unwrap();
    let transport = Solving {
        put: RefCell::new(Vec::new()),
        fetched: RefCell::new(0),
    };
    let data = DataPaths::new(&root.path().join("data"));
    let solvers = Solvers::default();
    let setup = SolveSetup {
        solvers: &solvers,
        choice: crate::sky_solution::SolverChoice::Astap,
        originals_dir: &data.originals,
        work_root: &data.solve,
        timeout: std::time::Duration::from_secs(5),
        cancel: None,
        now: "2026-10-08T10:00:00Z",
    };
    let outcomes = solve_night(
        &transport,
        "http://s.invalid",
        "t",
        &online(&sha(b"new final"), "missing"),
        &local,
        &BTreeMap::new(),
        &setup,
    )
    .unwrap();
    assert_eq!(outcomes.len(), 1);
    match &outcomes[0] {
        SolveOutcome::Failed { failure, .. } => assert_eq!(*failure, SolveFailure::SolverMissing),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        *transport.fetched.borrow(),
        0,
        "the local file of the same digest is used"
    );
    // Solved already: nothing to do.
    assert!(
        solve_night(
            &transport,
            "http://s.invalid",
            "t",
            &online(&sha(b"new final"), "present"),
            &local,
            &BTreeMap::new(),
            &setup
        )
        .unwrap()
        .is_empty()
    );
}

#[test]
fn the_ledger_is_written_after_a_sending() {
    let root = TempDir::new("steps-ledger");
    let paths = DataPaths::new(root.path());
    let sent = SentRecord {
        analysis_fingerprint: Some("abc".to_string()),
        progression_frames: BTreeMap::from([("t-m31".to_string(), 42)]),
    };
    record_sending(&paths, "7f3a2c91-steps", &sent).unwrap();
    let ledger = record_sending(&paths, "7f3a2c91-steps", &SentRecord::default()).unwrap();
    assert_eq!(ledger.analysis_inputs.as_deref(), Some("abc"));
    assert_eq!(ledger.progression_frames["t-m31"], 42);
    assert!(paths.nights.join("7f3a2c91-steps.json").is_file());
}

#[test]
fn a_night_of_frames_only_or_of_a_final_image_only_is_treated() {
    use crate::precheck::{Treatment, Unavailable, treatments};
    let tools = crate::precheck::ToolsPresent {
        stacking_pipeline: true,
        astap: true,
        siril: true,
    };
    let state = |material: NightMaterial, treatment: Treatment| {
        treatments(&capabilities(), &tools, &material)
            .into_iter()
            .find(|s| s.treatment == treatment)
            .unwrap()
    };
    let frames_only = NightMaterial {
        light_frames: true,
        device_images: false,
        logs: false,
        final_image: false,
    };
    assert!(state(frames_only, Treatment::Stacking).available);
    assert!(state(frames_only, Treatment::Progression).available);
    let final_only = NightMaterial {
        light_frames: false,
        device_images: false,
        logs: false,
        final_image: true,
    };
    assert!(state(final_only, Treatment::FinalImage).available);
    assert!(state(final_only, Treatment::SkyAnnotations).available);
    // The image a device stacked is material for the stacking and the sky
    // annotations after it, never for the progression (which reads poses).
    let image_only = NightMaterial {
        device_images: true,
        ..NightMaterial::default()
    };
    assert!(state(image_only, Treatment::Stacking).available);
    assert!(state(image_only, Treatment::SkyAnnotations).available);
    assert_eq!(
        state(image_only, Treatment::Progression).unavailable,
        Some(Unavailable::NothingToTreat)
    );
    let nothing = NightMaterial::default();
    assert_eq!(
        state(nothing, Treatment::FinalImage).unavailable,
        Some(Unavailable::NothingToTreat)
    );
    assert_eq!(
        state(nothing, Treatment::Stacking).unavailable,
        Some(Unavailable::NothingToTreat)
    );
}

#[test]
fn the_masters_of_a_night_leave_as_lines_of_its_multi_night_plan() {
    use crate::multi_night::tests::{FakeService, capabilities, heart, integration, write_master};
    use crate::multi_night::{IntegrationsSent, MultiNightClosed, project_of};
    let dir = TempDir::new("steps-integrations");
    let spec = NightFolderSpec {
        night_id: "n-12".to_string(),
        service_url: "https://service.example".to_string(),
        evening_date: "2026-08-12".to_string(),
        label: "Cœur UV/IR".to_string(),
        targets: vec![
            NightTargetSpec {
                target_id: "ic-1805".to_string(),
                designation: "IC 1805".to_string(),
                ticked: true,
            },
            NightTargetSpec {
                target_id: "m-31".to_string(),
                designation: "M31".to_string(),
                ticked: true,
            },
        ],
        follow: None,
    };
    let folder = prepare_night_folder(dir.path(), &[], &spec, "2026-08-13T09:00:00Z")
        .unwrap()
        .folder;
    write_master(
        &folder.join("IC 1805/stacked"),
        "UV-IR-cut_120s_g100",
        Some("UV/IR-cut"),
        120.0,
        120,
    );
    // M31 is in no multi-night plan: its master never leaves.
    write_master(&folder.join("M31/stacked"), "L_60s_g100", None, 60.0, 30);
    let night = crate::multi_night::NightMultiNight {
        evening_date: Some("2026-08-12".to_string()),
        projects: vec![project_of(&heart(), "ic-1805").unwrap()],
    };
    let computer = crate::multi_night::Computer {
        label: Some("iMac du bureau".into()),
        installation_id: Some("inst-1".into()),
    };
    let service = FakeService {
        put_answer: Some(Ok(serde_json::json!({ "integrations": [
            integration("n-12", "2026-08-12", "UV/IR-cut", 120.0, 120, "iMac du bureau")
        ]}))),
        ..FakeService::default()
    };
    let open = MultiNightGate::new(Some(&capabilities(true)), true);
    // The evening of the folder's name is not what is sent: the plan's is.
    let sent = send_night_integrations(
        &service,
        "u",
        "t",
        "n-12",
        &folder,
        &night,
        "2026-08-13",
        &computer,
        open,
    )
    .unwrap();
    assert!(matches!(sent[0], IntegrationsSent::Sent { lines: 1, .. }));
    let puts = service.puts.borrow();
    assert_eq!(puts.len(), 1);
    let line = &puts[0].1["integrations"][0];
    assert_eq!(line["nightId"], "n-12");
    assert_eq!(line["targetId"], "ic-1805");
    assert_eq!(line["eveningDate"], "2026-08-12");
    assert_eq!(line["filter"], "UV/IR-cut");
    assert_eq!(line["computerLabel"], "iMac du bureau");
    drop(puts);

    // The service never heard: nothing leaves, and it says so.
    let private = MultiNightGate::unknown();
    let quiet = FakeService::default();
    let sent = send_night_integrations(
        &quiet,
        "u",
        "t",
        "n-12",
        &folder,
        &night,
        "2026-08-12",
        &computer,
        private,
    )
    .unwrap();
    assert!(matches!(
        sent[0],
        IntegrationsSent::Closed {
            closed: MultiNightClosed::Unknown,
            ..
        }
    ));
    assert!(quiet.puts.borrow().is_empty());

    // The service refuses a night outside its window: said in clear.
    let refusing = FakeService {
        put_answer: Some(Err((
            "integration_outside_night".to_string(),
            "This integration is not dated by its night.".to_string(),
        ))),
        ..FakeService::default()
    };
    let sent = send_night_integrations(
        &refusing,
        "u",
        "t",
        "n-12",
        &folder,
        &night,
        "2026-08-12",
        &computer,
        open,
    )
    .unwrap();
    assert!(matches!(
        &sent[0],
        IntegrationsSent::Refused { code, .. } if code == "integration_outside_night"
    ));
}

/// A night of which only the image its device stacked is here (no master kept) still leaves its line of the
/// plan, read from the header of the image; a family a master covers is not sent twice.
#[test]
fn a_night_with_only_the_image_of_its_device_leaves_a_line_read_from_its_header() {
    use crate::multi_night::tests::{
        FakeService, capabilities, heart, integration, write_device_image, write_master,
    };
    use crate::multi_night::{IntegrationsSent, project_of};
    let dir = TempDir::new("steps-integrations-device");
    let spec = NightFolderSpec {
        night_id: "n-12".to_string(),
        service_url: "https://service.example".to_string(),
        evening_date: "2026-08-12".to_string(),
        label: "Cœur UV/IR".to_string(),
        targets: vec![NightTargetSpec {
            target_id: "ic-1805".to_string(),
            designation: "IC 1805".to_string(),
            ticked: true,
        }],
        follow: None,
    };
    let folder = prepare_night_folder(dir.path(), &[], &spec, "2026-08-13T09:00:00Z")
        .unwrap()
        .folder;
    write_device_image(&folder.join("IC 1805/device"), 120, "UV/IR-cut", 120.0);
    let night = crate::multi_night::NightMultiNight {
        evening_date: Some("2026-08-12".to_string()),
        projects: vec![project_of(&heart(), "ic-1805").unwrap()],
    };
    let computer = crate::multi_night::Computer::default();
    let service = FakeService {
        put_answer: Some(Ok(serde_json::json!({ "integrations": [
            integration("n-12", "2026-08-12", "UV/IR-cut", 120.0, 120, "iMac du bureau")
        ]}))),
        ..FakeService::default()
    };
    let open = MultiNightGate::new(Some(&capabilities(true)), true);
    let sent = send_night_integrations(
        &service,
        "u",
        "t",
        "n-12",
        &folder,
        &night,
        "2026-08-12",
        &computer,
        open,
    )
    .unwrap();
    assert!(matches!(sent[0], IntegrationsSent::Sent { lines: 1, .. }));
    let line = service.puts.borrow()[0].1["integrations"][0].clone();
    assert_eq!(line["filter"], "UV/IR-cut");
    assert_eq!(line["exposureSeconds"], 120.0);
    assert_eq!(line["frameCount"], 120);
    assert_eq!(line["sensor"], "Seestar S50 Pro");
    // The same night stacked here as well: the master's line goes, the device's image of the family is not a second one.
    write_master(
        &folder.join("IC 1805/stacked"),
        "UV-IR-cut_120s_g100",
        Some("UV/IR-cut"),
        120.0,
        118,
    );
    let again = FakeService {
        put_answer: Some(Ok(serde_json::json!({ "integrations": [] }))),
        ..FakeService::default()
    };
    send_night_integrations(
        &again,
        "u",
        "t",
        "n-12",
        &folder,
        &night,
        "2026-08-12",
        &computer,
        open,
    )
    .unwrap();
    let puts = again.puts.borrow();
    assert_eq!(puts[0].1["integrations"].as_array().unwrap().len(), 1);
    assert_eq!(puts[0].1["integrations"][0]["frameCount"], 118);
}

/// A night whose `final/` is empty and whose `stacked/` holds the render an
/// earlier stacking made, PNG and TIFF; the digest of its PNG.
fn night_with_a_stacked_leftover(root: &Path) -> (ResumeLocal, String) {
    let (folder, detection) = night(root);
    fs::remove_file(folder.join("M31/final/M31_old.jpg")).unwrap();
    fs::remove_file(folder.join("M31/final/M31.tif")).unwrap();
    let local = resume_local(
        FolderState::Found,
        Some(&folder),
        Some(&detection),
        None,
        None,
    )
    .unwrap();
    assert_eq!(local.images.len(), 2, "the render's PNG and its TIFF");
    assert_eq!(local.images[0].kind, ResultKind::Stacked);
    assert!(local.images[0].path.ends_with("M31_2026-10-07_raw.png"));
    let digest = local.images[0].sha256.clone();
    (local, digest)
}

fn render(target: &str, name: &str, fill: u8) -> ImageCandidate {
    ImageCandidate {
        render_rank: None,
        target_id: target.to_string(),
        path: PathBuf::from(format!("/n/M31/stacked/{name}")),
        sha256: format!("{fill:02x}").repeat(32),
        size: 5,
        kind: ResultKind::Stacked,
        render: Some("raw".to_string()),
        contrast: crate::stacking::Contrast::None,
        retouch: Default::default(),
        frame_count: None,
        twin_sha256: None,
    }
}

#[test]
fn a_stacked_output_of_an_earlier_run_is_replaced_by_the_new_stacking() {
    let root = TempDir::new("steps-restack");
    let (local, _) = night_with_a_stacked_leftover(root.path());

    // The summary, before the stacking runs: the target is named, the old
    // output is not a candidate (it is neither counted nor listed).
    let before = precheck_candidates(&local, Vec::new(), &["t-m31".to_string()], false);
    assert!(before.is_empty(), "{before:?}");

    // The sending: the new render is in `extra` (and in `stacked/` too, but
    // the folder's old and new outputs are both left out of the local list).
    let new = render("t-m31", "M31_2026-10-07_raw-2.tif", 0xab);
    let sending = precheck_candidates(&local, vec![new.clone()], &[], false);
    assert_eq!(sending, vec![new.clone()], "the new image is the one sent");

    // Another target's output is not touched.
    let other = precheck_candidates(&local, vec![render("t-ngc", "x.tif", 1)], &[], false);
    assert_eq!(other.len(), 2);
}

#[test]
fn a_stacked_output_online_is_kept_identical_never_a_final_image_sent_again() {
    let root = TempDir::new("steps-leftover");
    let (local, digest) = night_with_a_stacked_leftover(root.path());
    let service = Service {
        checks: RefCell::new(Vec::new()),
    };

    // Not stacked again, identical to the capture online: « already
    // online », nothing to send, the service is not even asked.
    let candidates = precheck_candidates(&local, Vec::new(), &[], false);
    let check = precheck_night(
        &service,
        "http://s.invalid",
        "t",
        &online(&digest, "missing"),
        &candidates,
    )
    .unwrap();
    assert_eq!(check.plans.len(), 1);
    assert_eq!(check.plans[0].image.kind, ResultKind::Stacked);
    assert!(matches!(
        check.plans[0].decision,
        Decision::AlreadyOnline { .. }
    ));
    assert_eq!(check.quota.to_send_bytes, 0);
    assert!(service.checks.borrow().is_empty());

    // Stacked again: nothing of the old output remains, so nothing counts
    // against the space.
    let candidates = precheck_candidates(&local, Vec::new(), &["t-m31".to_string()], false);
    let check = precheck_night(
        &service,
        "http://s.invalid",
        "t",
        &online(&digest, "missing"),
        &candidates,
    )
    .unwrap();
    assert!(check.plans.is_empty());
    assert_eq!(check.quota.to_send_bytes, 0);
}

/* ------------- The renders leave as PNG, the TIFF only when joined ------------- */

/// A night of M31 as a stacking leaves it: the person's final TIFF, a render
/// in both formats (`raw`), the linear TIFF alone and the small picture of a
/// plan; nothing online yet.
fn night_of_renders(root: &Path) -> (ResumeLocal, NightOnline) {
    let (folder, detection) = night(root);
    fs::remove_file(folder.join("M31/final/M31_old.jpg")).unwrap();
    let stacked = folder.join("M31/stacked");
    fs::write(stacked.join("M31_2026-10-07_raw.tif"), vec![b't'; 470]).unwrap();
    fs::write(stacked.join("M31_2026-10-07_raw.png"), vec![b'p'; 130]).unwrap();
    fs::write(stacked.join("M31_2026-10-07_linear.tif"), vec![b'l'; 200]).unwrap();
    fs::write(stacked.join("M31_plan.preview.png"), b"preview").unwrap();
    // What the stacker recorded: the natural render asked first, "No
    // processing" written last.
    fs::write(
        stacked.join("M31_2026-10-07_stacking.json"),
        serde_json::json!({"outputs": [
            {"pipeline": "raw", "file": "M31_2026-10-07_raw.tif"},
            {"pipeline": "raw", "file": "M31_2026-10-07_raw.png"},
            {"pipeline": "linear", "file": "M31_2026-10-07_linear.tif"}
        ]})
        .to_string(),
    )
    .unwrap();
    let local = resume_local(
        FolderState::Found,
        Some(&folder),
        Some(&detection),
        None,
        None,
    )
    .unwrap();
    let online: NightOnline = serde_json::from_value(serde_json::json!({
        "sessionId": "s-1",
        "targets": [{"targetId": "t-m31", "name": "M31", "planned": true, "captures": []}],
        "analysis": {"present": false, "stepCount": 0, "editedStepCount": 0}
    }))
    .unwrap();
    (local, online)
}

fn names_of(candidates: &[ImageCandidate]) -> Vec<String> {
    candidates
        .iter()
        .map(|c| c.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

fn space(remaining: u64) -> SpaceStorage {
    SpaceStorage {
        used_bytes: 1000 - remaining,
        quota_bytes: 1000,
        remaining_bytes: remaining,
        max_file_bytes: 1000,
        capture_count: 0,
        capture_count_limit: 10,
        variant_limit: 12,
        capabilities: capabilities(),
    }
}

#[test]
fn by_default_a_render_leaves_as_its_png_the_linear_as_its_tiff_and_final_as_it_is() {
    let root = TempDir::new("steps-png-only");
    let (local, online) = night_of_renders(root.path());
    let candidates = precheck_candidates(&local, Vec::new(), &[], false);
    assert_eq!(
        names_of(&candidates),
        [
            "M31.tif",
            "M31_2026-10-07_linear.tif",
            "M31_2026-10-07_raw.png"
        ],
        "the final image untouched, the linear TIFF, the PNG of the render; \
         never the render's TIFF nor the preview"
    );
    let plans = crate::precheck::plan_images(&online, &candidates, 12);
    let quota = crate::precheck::check_quotas(&space(1000), &online, &plans);
    // The final TIFF is replaced by the stacking (as before); the PNG and the
    // linear TIFF leave.
    assert_eq!(quota.to_send_bytes, 130 + 200);
    assert_eq!(quota.full_resolution_bytes, 0);
}

#[test]
fn joined_the_tiff_of_a_render_follows_its_png_as_an_image_of_the_same_capture() {
    let root = TempDir::new("steps-png-and-tiff");
    let (local, online) = night_of_renders(root.path());
    let candidates = precheck_candidates(&local, Vec::new(), &[], true);
    assert_eq!(
        names_of(&candidates),
        [
            "M31.tif",
            "M31_2026-10-07_linear.tif",
            "M31_2026-10-07_raw.png",
            "M31_2026-10-07_raw.tif"
        ]
    );
    let plans = crate::precheck::plan_images(&online, &candidates, 12);
    let decision = |name: &str| {
        plans
            .iter()
            .find(|p| p.image.path.ends_with(name))
            .map(|p| p.decision.clone())
            .unwrap()
    };
    // The PNG is the capture's image, its TIFF a variant of it.
    assert_eq!(decision("M31_2026-10-07_raw.png"), Decision::NewCapture);
    let png_sha = candidates[2].sha256.clone();
    assert_eq!(
        decision("M31_2026-10-07_raw.tif"),
        Decision::VariantOfNew {
            first_sha256: png_sha
        }
    );
    let quota = crate::precheck::check_quotas(&space(1000), &online, &plans);
    assert_eq!(quota.to_send_bytes, 130 + 200 + 470);
    assert_eq!(quota.full_resolution_bytes, 470);
    // What the box says of the TIFF, joined or not: one, 470 bytes.
    assert_eq!(
        full_resolution_of(&candidates, false),
        FullResolution {
            joined: false,
            count: 1,
            bytes: 470
        }
    );
}

#[test]
fn a_refusal_for_room_says_what_the_sending_weighs_without_the_tiff() {
    let root = TempDir::new("steps-room");
    let (local, online) = night_of_renders(root.path());
    // 500 bytes left: with the TIFF (800) it does not fit, without (330) it does.
    let with = crate::precheck::plan_images(
        &online,
        &precheck_candidates(&local, Vec::new(), &[], true),
        12,
    );
    let refused = crate::precheck::check_quotas(&space(500), &online, &with);
    assert!(!refused.allowed);
    assert!(matches!(
        refused.findings[0],
        crate::precheck::QuotaFinding::StorageQuota {
            to_send_bytes: 800,
            ..
        }
    ));
    assert_eq!(refused.to_send_bytes - refused.full_resolution_bytes, 330);
    let without = crate::precheck::plan_images(
        &online,
        &precheck_candidates(&local, Vec::new(), &[], false),
        12,
    );
    let allowed = crate::precheck::check_quotas(&space(500), &online, &without);
    assert!(allowed.allowed, "{:?}", allowed.findings);
    assert_eq!(allowed.to_send_bytes, 330);
}

#[test]
fn a_render_whose_tiff_is_online_is_online_and_its_tiff_is_never_missing() {
    let root = TempDir::new("steps-twin-online");
    let (local, _) = night_of_renders(root.path());
    let tiff = local
        .images
        .iter()
        .find(|i| i.path.ends_with("M31_2026-10-07_raw.tif"))
        .unwrap();
    assert!(tiff.is_full_resolution_twin());
    // A night sent before this rule left its TIFF online: the PNG of the same
    // render is not sent beside it.
    let before = online(&tiff.sha256, "present");
    let candidates = precheck_candidates(&local, Vec::new(), &[], false);
    let plans = crate::precheck::plan_images(&before, &candidates, 12);
    let png = plans
        .iter()
        .find(|p| p.image.path.ends_with("M31_2026-10-07_raw.png"))
        .unwrap();
    assert!(
        matches!(png.decision, Decision::AlreadyOnline { .. }),
        "{:?}",
        png.decision
    );
    // The resume never asks for the TIFF of a render: it is a choice.
    let resume = night_resume::resume_night(&before, &local, &[], None, 12);
    let to_send = resume.missing.iter().find_map(|m| match m {
        night_resume::Missing::ImagesToSend { count } => Some(*count),
        _ => None,
    });
    // The final TIFF and the linear one are here and not online.
    assert_eq!(to_send, Some(2));
}

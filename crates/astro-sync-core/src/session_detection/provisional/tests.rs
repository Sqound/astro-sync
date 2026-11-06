use super::*;
use crate::plan_image::plan_folder;
use crate::session_detection::{
    DetectedTarget, MultiNightGroup, MultiNightNight, PlaceCandidate, PlaceProposal, TargetSource,
    prepare_treatments,
};
use crate::test_support::TempDir;
use crate::upload_queue::{EntryTarget, ResultFile, ResultKind};
use std::cell::{Cell, RefCell};

const NOW: &str = "2026-10-24T03:50:00Z";

/// Two nights of M45 read from `source` (their frames written there), one
/// multi-night plan over both.
fn request(source: &Path) -> BulkCreateRequest {
    let mut sessions = Vec::new();
    for (id, date, names) in [
        ("s-07", "2026-10-07", vec!["a.fits", "b.fits"]),
        ("s-08", "2026-10-08", vec!["c.fits"]),
    ] {
        let lights = source.join(date).join("M45/lights");
        fs::create_dir_all(&lights).unwrap();
        let files: Vec<String> = names
            .iter()
            .map(|name| {
                let path = lights.join(name);
                fs::write(&path, format!("frame {name}")).unwrap();
                path.to_string_lossy().into_owned()
            })
            .collect();
        sessions.push(DetectedSession {
            id: id.into(),
            evening_date: date.into(),
            written_evening_date: date.into(),
            place: "place-1".into(),
            equipment: vec!["eq-1".into()],
            start: format!("{date}T21:00:00.000Z"),
            end: format!("{date}T23:00:00.000Z"),
            targets: vec![DetectedTarget {
                key: "m45".into(),
                target_id: Some("m45".into()),
                designation: "M45".into(),
                source: TargetSource::Object,
                start: format!("{date}T21:00:00.000Z"),
                end: format!("{date}T23:00:00.000Z"),
                frames: files.len(),
                exposure_seconds: 300.0,
                files,
                ..DetectedTarget::default()
            }],
            ..DetectedSession::default()
        });
    }
    let proposal = Proposal {
        places: vec![PlaceProposal {
            id: "place-1".into(),
            candidates: vec![PlaceCandidate {
                place_id: "maison".into(),
                name: "Col des Étoiles".into(),
                distance_km: 0.0,
            }],
            chosen: Some("maison".into()),
            longitude: Some(6.5),
            ..PlaceProposal::default()
        }],
        sessions,
        multi_night: vec![MultiNightGroup {
            id: "g-1".into(),
            target_key: "m45".into(),
            target_id: Some("m45".into()),
            designation: "M45".into(),
            nights: vec![
                MultiNightNight {
                    session_id: "s-07".into(),
                    evening_date: "2026-10-07".into(),
                    frames: 2,
                    seconds: 600.0,
                },
                MultiNightNight {
                    session_id: "s-08".into(),
                    evening_date: "2026-10-08".into(),
                    frames: 1,
                    seconds: 300.0,
                },
            ],
            goal_seconds: 900.0,
            frames: 3,
            ..MultiNightGroup::default()
        }],
        ..Proposal::default()
    };
    BulkCreateRequest {
        proposal,
        ticked: vec![
            "session.create:s-07".into(),
            "session.create:s-08".into(),
            "target.detected:s-07/m45".into(),
            "target.detected:s-08/m45".into(),
            "multiNight.create:g-1".into(),
        ],
        places: Vec::new(),
        answers: Vec::new(),
        shifts: Vec::new(),
    }
}

/// What the space answers: `sess-<ref>` for each night, `prj-<ref>` for the
/// plan; the same body again replays.
struct FakeSpace {
    calls: Cell<u32>,
    fail_next: Cell<bool>,
    bodies: RefCell<Vec<Value>>,
}

impl FakeSpace {
    fn new() -> Self {
        Self {
            calls: Cell::new(0),
            fail_next: Cell::new(false),
            bodies: RefCell::new(Vec::new()),
        }
    }
}

impl BulkCreateTransport for FakeSpace {
    fn create_bulk(&self, _: &str, _: &str, body: &Value) -> Result<Value, GuiBridgeError> {
        self.calls.set(self.calls.get() + 1);
        if self.fail_next.replace(false) {
            return Err(GuiBridgeError::Transport("cut".into()));
        }
        let replayed = self.bodies.borrow().iter().any(|b| b == body);
        self.bodies.borrow_mut().push(body.clone());
        let id = |reference: &str| format!("sess-{reference}");
        Ok(json!({
            "replayed": replayed,
            "places": [],
            "sessions": body["sessions"].as_array().unwrap().iter().map(|s| json!({
                "ref": s["ref"],
                "sessionId": id(s["ref"].as_str().unwrap()),
                "action": "created",
                "planAction": "created",
            })).collect::<Vec<_>>(),
            "multiNight": body["multiNight"].as_array().unwrap().iter().map(|m| json!({
                "ref": m["ref"],
                "projectId": format!("prj-{}", m["ref"].as_str().unwrap()),
                "action": "created",
                "sessionIds": m["sessionRefs"].as_array().unwrap().iter()
                    .map(|r| id(r.as_str().unwrap())).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        }))
    }

    fn personal_targets(&self, _: &str, _: &str) -> Result<Value, GuiBridgeError> {
        Ok(json!({ "targets": [] }))
    }

    fn create_personal_target(&self, _: &str, _: &str, _: &Value) -> Result<Value, GuiBridgeError> {
        Ok(json!({}))
    }
}

/// The import's step 3 on `root`: the provisional nights prepared, a result
/// of the stacking in each, the image of the plan in its folder.
fn treated(root: &Path, request: &BulkCreateRequest) -> BulkCreateOutcome {
    let provisional = provisional_outcome(request);
    prepare_treatments(
        &request.proposal,
        &provisional,
        &[],
        root,
        &[],
        "https://service.example",
        NOW,
    )
    .unwrap();
    for night in list_night_folders(root).unwrap() {
        let stacked = night.folder.join("M45/stacked");
        fs::create_dir_all(&stacked).unwrap();
        fs::write(stacked.join("M45_natural.tif"), b"tif").unwrap();
        fs::write(
            stacked.join("M45_natural.json"),
            json!({
                "nightId": night.night_id,
                "master": night.folder.join("M45/stacked/M45_master.fit"),
            })
            .to_string(),
        )
        .unwrap();
    }
    let group = &provisional.multi_night[0];
    let plan = plan_folder(
        root,
        &crate::multi_night::NightProject {
            project_id: group.plan_id.clone(),
            target_id: "m45".into(),
            name: "M45".into(),
            common_name: None,
            integrations: Vec::new(),
            total_seconds: 0.0,
            night_count: 2,
            plan_capture: None,
        },
    )
    .unwrap();
    fs::create_dir_all(plan.join("stacked")).unwrap();
    fs::write(plan.join("stacked/M45_plan_natural.tif"), b"plan").unwrap();
    let masters: Vec<PathBuf> = list_night_folders(root)
        .unwrap()
        .into_iter()
        .map(|n| n.folder.join("M45/stacked/M45_master.fit"))
        .collect();
    fs::write(
        plan.join("stacked/M45_plan.json"),
        json!({ "projectId": group.plan_id, "nights": group.session_ids, "masters": masters })
            .to_string(),
    )
    .unwrap();
    provisional
}

fn night_names(root: &Path) -> Vec<String> {
    list_night_folders(root)
        .unwrap()
        .into_iter()
        .map(|n| n.name)
        .collect()
}

#[test]
fn a_night_to_create_gets_a_provisional_identity_and_folder() {
    let dir = TempDir::new("provisional-naming");
    let source = dir.path().join("source");
    let root = dir.path().join("Astro Sync");
    fs::create_dir_all(&root).unwrap();
    let request = request(&source);
    let outcome = provisional_outcome(&request);
    // The same folder read again gives the same identifiers.
    assert_eq!(outcome, provisional_outcome(&request));
    let ids: Vec<&str> = outcome
        .sessions
        .iter()
        .map(|s| s.session_id.as_str())
        .collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.iter().all(|id| is_provisional(id) && id.len() == 18));
    assert_ne!(ids[0], ids[1]);
    assert!(outcome.sessions.iter().all(|s| s.action == "created"));
    assert_eq!(outcome.sessions[0].designations, vec!["M45"]);
    assert_eq!(outcome.multi_night.len(), 1);
    assert!(is_provisional(&outcome.multi_night[0].plan_id));
    assert_eq!(outcome.multi_night[0].session_ids, ids);

    let nights = prepare_treatments(
        &request.proposal,
        &outcome,
        &[],
        &root,
        &[],
        "https://service.example",
        NOW,
    )
    .unwrap();
    assert_eq!(nights.len(), 2);
    assert_eq!(
        night_names(&root),
        vec![
            format!("2026-10-07 · Col des Étoiles · {}", &ids[0][..12]),
            format!("2026-10-08 · Col des Étoiles · {}", &ids[1][..12]),
        ]
    );
    // The record carries the night's proposal and no session yet.
    let record: Value = serde_json::from_str(
        &fs::read_to_string(nights[0].night_folder.join(NIGHT_IDENTITY_FILE)).unwrap(),
    )
    .unwrap();
    assert_eq!(record["nightId"], ids[0]);
    assert_eq!(record["pending"]["pendingSessionId"], Value::Null);
    assert_eq!(record["pending"]["detectedId"], "s-07");
    assert_eq!(record["pending"]["place"], "Col des Étoiles");
    assert_eq!(record["pending"]["equipmentIds"], json!(["eq-1"]));
    assert_eq!(record["pending"]["targets"][0]["frames"], 2);
    // The frames are linked there, never moved.
    let link = nights[0].night_folder.join("M45/lights/a.fits");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(source.join("2026-10-07/M45/lights/a.fits").is_file());
    // Prepared again (a second run of the treatments): the same folders.
    prepare_treatments(
        &request.proposal,
        &outcome,
        &[],
        &root,
        &[],
        "https://service.example",
        NOW,
    )
    .unwrap();
    assert_eq!(night_names(&root).len(), 2);
}

#[test]
fn the_created_session_renames_the_folder_and_what_points_into_it() {
    let dir = TempDir::new("provisional-settle");
    let source = dir.path().join("source");
    let root = dir.path().join("Astro Sync");
    fs::create_dir_all(&root).unwrap();
    let request = request(&source);
    let provisional = treated(&root, &request);
    let first = list_night_folders(&root).unwrap().remove(0);
    let local_id = first.night_id.clone();
    // A row of the queue made there before the creation.
    let queue = UploadQueue::open(&dir.path().join("queue.sqlite")).unwrap();
    queue
        .enqueue_results(&[ResultFile {
            path: first.folder.join("M45/stacked/M45_natural.tif"),
            sha256: "f".repeat(64),
            size: 3,
            kind: ResultKind::Stacked,
            target: EntryTarget {
                session_id: Some(local_id.clone()),
                target_ids: vec!["m45".into()],
                ..EntryTarget::default()
            },
        }])
        .unwrap();

    let space = FakeSpace::new();
    let created = create_sessions(&space, "https://s", "t", &request).unwrap();
    let recorded = record_created(&root, &provisional, &created).unwrap();
    assert_eq!(
        recorded,
        Recorded {
            nights: 2,
            plans: 1
        }
    );
    // Written before anything is renamed: the session, and where the folder is.
    let record = read_night_identity(&first.folder).unwrap();
    let pending = record.pending.unwrap();
    assert_eq!(pending.pending_session_id.as_deref(), Some("sess-s-07"));
    assert_eq!(pending.folder.as_deref(), Some(first.folder.as_path()));

    let nights = settle_nights(&root, Some(&queue)).unwrap();
    assert_eq!(nights.len(), 2);
    let renamed = root.join("2026-10-07 · Col des Étoiles · sess-s");
    assert_eq!(nights[0].to, renamed);
    assert!(!first.folder.exists());
    let record = read_night_identity(&renamed).unwrap();
    assert_eq!(record.night_id, "sess-s-07");
    assert!(record.pending.is_none());
    // The application finds the night by its session, as any night.
    assert_eq!(
        crate::working_folder::locate_night(&root, &[], "sess-s-07").unwrap(),
        crate::working_folder::NightLocation::Found {
            folder: renamed.clone()
        }
    );
    // The links to the person's frames survive the rename.
    let link = renamed.join("M45/lights/a.fits");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_to_string(&link).unwrap(), "frame a.fits");
    // What Astro Sync wrote there names the session and the new folder.
    let kept: Value = serde_json::from_str(
        &fs::read_to_string(renamed.join("M45/stacked/M45_natural.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(kept["nightId"], "sess-s-07");
    assert_eq!(
        kept["master"],
        renamed
            .join("M45/stacked/M45_master.fit")
            .to_string_lossy()
            .as_ref()
    );
    // The row of the queue follows.
    let row = queue.list().unwrap().remove(0);
    assert_eq!(row.source_path, renamed.join("M45/stacked/M45_natural.tif"));
    assert_eq!(row.target.session_id.as_deref(), Some("sess-s-07"));

    // Then the plan.
    let plans = settle_plans(&root, &nights, Some(&queue)).unwrap();
    assert_eq!(plans.len(), 1);
    let plan = root.join("M45 · prj-g-");
    assert_eq!(plans[0].to, plan);
    let kept: Value =
        serde_json::from_str(&fs::read_to_string(plan.join("stacked/M45_plan.json")).unwrap())
            .unwrap();
    assert_eq!(kept["projectId"], "prj-g-1");
    assert_eq!(kept["nights"], json!(["sess-s-07", "sess-s-08"]));
    assert_eq!(
        kept["masters"][0],
        renamed
            .join("M45/stacked/M45_master.fit")
            .to_string_lossy()
            .as_ref()
    );
    // Settled again: nothing left to do.
    assert!(reconcile(&root, Some(&queue)).unwrap() == Reconciled::default());
}

#[test]
fn a_stop_between_the_creation_and_the_rename_is_finished_at_the_next_start() {
    let dir = TempDir::new("provisional-reconcile");
    let source = dir.path().join("source");
    let root = dir.path().join("Astro Sync");
    fs::create_dir_all(&root).unwrap();
    let request = request(&source);
    let provisional = treated(&root, &request);
    let space = FakeSpace::new();
    let created = create_sessions(&space, "https://s", "t", &request).unwrap();
    record_created(&root, &provisional, &created).unwrap();
    // The application stopped after renaming the first folder, before its
    // record: and before touching the second.
    let first = list_night_folders(&root).unwrap().remove(0);
    let moved = root.join("2026-10-07 · Col des Étoiles · sess-s");
    fs::rename(&first.folder, &moved).unwrap();

    let reconciled = reconcile(&root, None).unwrap();
    assert_eq!(reconciled.nights.len(), 2);
    assert_eq!(reconciled.plans.len(), 1);
    assert_eq!(
        night_names(&root),
        vec![
            "2026-10-07 · Col des Étoiles · sess-s".to_string(),
            "2026-10-08 · Col des Étoiles · sess-s".to_string(),
        ]
    );
    let record = read_night_identity(&moved).unwrap();
    assert_eq!(record.night_id, "sess-s-07");
    assert!(record.pending.is_none());
    let kept: Value = serde_json::from_str(
        &fs::read_to_string(moved.join("M45/stacked/M45_natural.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(kept["nightId"], "sess-s-07");
    assert!(
        kept["master"]
            .as_str()
            .unwrap()
            .starts_with(moved.to_string_lossy().as_ref())
    );
    assert_eq!(
        fs::read_to_string(moved.join("M45/lights/b.fits")).unwrap(),
        "frame b.fits"
    );

    // A night whose session was never created keeps its provisional folder.
    let other = TempDir::new("provisional-never-created");
    let root = other.path().join("Astro Sync");
    fs::create_dir_all(&root).unwrap();
    let request = self::request(&other.path().join("source"));
    treated(&root, &request);
    assert_eq!(reconcile(&root, None).unwrap(), Reconciled::default());
    assert!(
        list_night_folders(&root)
            .unwrap()
            .iter()
            .all(|n| is_provisional(&n.night_id))
    );
}

#[test]
fn abandoning_removes_the_provisional_folders_never_a_file_of_the_person_s() {
    let dir = TempDir::new("provisional-discard");
    let source = dir.path().join("source");
    let root = dir.path().join("Astro Sync");
    fs::create_dir_all(&root).unwrap();
    let request = request(&source);
    let provisional = treated(&root, &request);
    let second = list_night_folders(&root).unwrap().remove(1);
    // A file of the person's, put in the second night's folder.
    fs::write(second.folder.join("M45/lights/mine.fits"), b"mine").unwrap();
    let mut ids: Vec<String> = provisional
        .sessions
        .iter()
        .map(|s| s.session_id.clone())
        .collect();
    ids.push(provisional.multi_night[0].plan_id.clone());

    let discarded = discard_provisional(&root, &ids).unwrap();
    assert_eq!(
        discarded,
        Discarded {
            removed: 2,
            kept: 1
        }
    );
    // The first night and the plan are gone; the person's frames are not.
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    for (date, name) in [
        ("2026-10-07", "a.fits"),
        ("2026-10-07", "b.fits"),
        ("2026-10-08", "c.fits"),
    ] {
        assert!(source.join(date).join("M45/lights").join(name).is_file());
    }
    // The second keeps the person's file, and only it.
    assert_eq!(
        fs::read(second.folder.join("M45/lights/mine.fits")).unwrap(),
        b"mine"
    );
    assert!(!second.folder.join("M45/lights/c.fits").exists());
    assert!(!second.folder.join("M45/stacked").exists());
}

#[cfg(unix)]
#[test]
fn the_stages_run_in_order_and_a_second_try_skips_what_went_through() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new("provisional-stages");
    let source = dir.path().join("source");
    let root = dir.path().join("Astro Sync");
    fs::create_dir_all(&root).unwrap();
    let request = request(&source);
    let provisional = treated(&root, &request);
    let space = FakeSpace::new();
    let all = [
        ImportStage::Sessions,
        ImportStage::Nights,
        ImportStage::Plans,
    ];
    let mut progress = ImportProgress::default();
    let run = |progress: &mut ImportProgress| {
        let mut seen = Vec::new();
        let result = run_import_stages(
            &space,
            "https://s",
            "t",
            &request,
            &provisional,
            &root,
            None,
            &all,
            progress,
            &mut |stage| seen.push(stage),
        );
        (seen, result.err().map(|(stage, _)| stage))
    };

    // The service is cut: nothing went through.
    space.fail_next.set(true);
    let (seen, failed) = run(&mut progress);
    assert_eq!(seen, vec![ImportStage::Sessions]);
    assert_eq!(failed, Some(ImportStage::Sessions));
    assert_eq!(progress, ImportProgress::default());
    assert!(
        list_night_folders(&root)
            .unwrap()
            .iter()
            .all(|n| is_provisional(&n.night_id))
    );

    // The sessions are created; the folders cannot be renamed.
    fs::set_permissions(&root, fs::Permissions::from_mode(0o555)).unwrap();
    let (seen, failed) = run(&mut progress);
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(seen, vec![ImportStage::Sessions, ImportStage::Nights]);
    assert_eq!(failed, Some(ImportStage::Nights));
    assert!(progress.created.is_some());
    assert!(progress.nights.is_none());
    assert_eq!(space.calls.get(), 2);

    // The second try skips the creation (no third call), renames, then the plan.
    let (seen, failed) = run(&mut progress);
    assert_eq!(seen, vec![ImportStage::Nights, ImportStage::Plans]);
    assert_eq!(failed, None);
    assert_eq!(space.calls.get(), 2);
    assert_eq!(progress.nights.as_ref().unwrap().len(), 2);
    assert_eq!(progress.plans.as_ref().unwrap().len(), 1);
    assert!(root.join("M45 · prj-g-").is_dir());

    // Done: nothing runs again.
    let (seen, failed) = run(&mut progress);
    assert!(seen.is_empty());
    assert_eq!(failed, None);
}

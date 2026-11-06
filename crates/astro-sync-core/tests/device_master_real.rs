//! The whole way of a Seestar night on **real files**,
//! through the public API only -- "Detect sessions" names the three
//! `Stacked_*.fit`, the treatments link them in `device/`, the night's
//! detection finds the families and their defaults, the stacking renders the
//! image of the 30 s family from the device's image (no pose read) and stacks
//! the 36 poses of the cut 10 s family again, the device's image is measured.
//!
//! Ignored: it needs the night's folder (read only), the stacking
//! environment and Siril. Run it with
//!
//! ```text
//! ASTRO_SYNC_REAL_NIGHT=<dir with lights/ and stacked_seestar/> \
//! ASTRO_SYNC_PYTHON=<python of the stacking environment> \
//! ASTRO_SYNC_SIRIL=<siril-cli> ASTRO_SYNC_REAL_OUT=<empty folder> \
//! cargo test -p astro-sync-core --test device_master_real -- --ignored --nocapture
//! ```
//!
//! and it writes `summary.json` in `ASTRO_SYNC_REAL_OUT`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use astro_sync_core::catalogue_targets::CatalogueTarget;
use astro_sync_core::detection::{DetectOptions, DetectionInput, DetectionTarget, detect_night};
use astro_sync_core::device_master::DeviceUse;
use astro_sync_core::local_work::CancelFlag;
use astro_sync_core::master_quality::{self, Outcome};
use astro_sync_core::session_detection::{
    BulkCreateOutcome, CreatedSession, DetectOptions as SessionOptions, DetectRequest, UserData,
    detect_sessions, prepare_treatments,
};
use astro_sync_core::stacking::{
    PipelineChoice, PythonStacker, StackGate, plan_target_with_device, run_target_stacking,
};

fn env(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set")))
}

#[test]
#[ignore = "needs the real night, the stacking environment and Siril"]
fn a_real_seestar_night_from_detection_to_render() {
    let night = env("ASTRO_SYNC_REAL_NIGHT");
    let python = env("ASTRO_SYNC_PYTHON");
    let siril = env("ASTRO_SYNC_SIRIL");
    let out = env("ASTRO_SYNC_REAL_OUT");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stacking/astro_sync_stack.py");
    let quality_script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stacking/star_quality.py");
    std::fs::create_dir_all(&out).unwrap();
    let mut summary = serde_json::Map::new();

    // 1. "Detect sessions": the three images are named, not set aside.
    let catalogue = vec![CatalogueTarget {
        id: "t-m45".into(),
        designation: "M45".into(),
        name: Some("Pléiades".into()),
        aliases: vec!["M 45".into(), "Pleiades".into()],
        type_label: None,
        constellation_label: None,
        ra_hours: Some(3.79),
        dec_degrees: Some(24.1),
        magnitude: None,
        size_label: None,
        size_arcmin: Some(110.0),
        image_path: None,
    }];
    let request = DetectRequest {
        fits: vec![night.clone()],
        subfolders: true,
        ..DetectRequest::default()
    };
    let proposal = detect_sessions(
        &request,
        &catalogue,
        &UserData::default(),
        &SessionOptions::default(),
    )
    .unwrap();
    assert_eq!(proposal.sessions.len(), 1, "one night");
    let found = &proposal.sessions[0].targets[0];
    summary.insert("lights".into(), proposal.reading.lights.into());
    summary.insert(
        "deviceImages".into(),
        found
            .device_images
            .iter()
            .map(|p| {
                Path::new(p)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>()
            .into(),
    );
    summary.insert(
        "setAsideStacks".into(),
        proposal.set_aside.stacks.len().into(),
    );
    assert_eq!(found.device_images.len(), 3);
    assert!(proposal.set_aside.stacks.is_empty());

    // 2. "Choose the treatments": the poses and the images linked.
    let session = &proposal.sessions[0];
    let created = BulkCreateOutcome {
        sessions: vec![CreatedSession {
            detected_id: session.id.clone(),
            session_id: "3e7a91c0-0000-4000-8000-000000000001".into(),
            evening_date: session.evening_date.clone(),
            action: "created".into(),
            plan_action: "created".into(),
            plan_id: None,
            designations: vec!["M45".into()],
            frames: found.frames,
            shift_hours: 0,
            added_targets: Vec::new(),
        }],
        ..BulkCreateOutcome::default()
    };
    let nights = prepare_treatments(
        &proposal,
        &created,
        &[],
        &out.join("working"),
        &[],
        "http://127.0.0.1:9130",
        "2026-10-17T12:00:00.000Z",
    )
    .unwrap();
    let night_folder = nights[0].night_folder.clone();
    let target_folder = nights[0].targets[0].folder_name.clone();

    // 3. The night's detection: families, coverage, defaults.
    let input = DetectionInput {
        evening_date: Some(session.evening_date.clone()),
        night_started_at: None,
        longitude: Some(139.994),
        targets: vec![DetectionTarget {
            target_id: "t-m45".into(),
            designation: "M45".into(),
            aliases: Vec::new(),
            ticked: true,
            online_final: false,
            stacking_planned: true,
        }],
    };
    let detection = detect_night(&night_folder, &input, &DetectOptions::default()).unwrap();
    let device = detection.targets[0]
        .device
        .as_ref()
        .expect("device images found");
    summary.insert(
        "families".into(),
        serde_json::to_value(&device.families).unwrap(),
    );
    summary.insert(
        "lightFrames".into(),
        detection.targets[0].light_frames.into(),
    );
    assert_eq!(device.images.len(), 3);
    assert_eq!(device.families.len(), 2);
    let uses: Vec<DeviceUse> = device
        .families
        .iter()
        .filter_map(|f| f.use_file.clone())
        .map(|file| DeviceUse { file })
        .collect();
    assert_eq!(
        uses.len(),
        1,
        "only the 30 s family takes the device's image"
    );

    // 4. The stacking: the image rendered, the 36 poses stacked again.
    let plan = plan_target_with_device(
        &night_folder,
        &target_folder,
        &session.evening_date,
        &[PipelineChoice::new("raw")],
        12,
        None,
        &uses,
    );
    assert_eq!(plan.jobs.len(), 2);
    let device_job = plan
        .jobs
        .iter()
        .find(|j| j.device_master.is_some())
        .unwrap();
    assert_eq!(device_job.light_count(), 0);
    let stacker = PythonStacker {
        python: python.clone(),
        script,
        pipelines_dirs: Vec::new(),
        siril: Some(siril),
        starnet: None,
        graxpert: None,
        timeout: Duration::from_secs(3600),
    };
    let started = Instant::now();
    let outcome = run_target_stacking(
        &stacker,
        &plan,
        &night_folder,
        StackGate {
            tool_present: true,
            variant_limit: 12,
        },
        &|_| None,
        &CancelFlag::new(),
        None,
        &mut |_| {},
    )
    .unwrap();
    summary.insert("stackingSeconds".into(), started.elapsed().as_secs().into());
    summary.insert(
        "renders".into(),
        outcome
            .renders
            .iter()
            .map(|r| {
                serde_json::json!({
                    "file": r.path.file_name().unwrap().to_string_lossy(),
                    "family": r.family,
                    "bytes": r.size,
                })
            })
            .collect::<Vec<_>>()
            .into(),
    );
    summary.insert(
        "integrationsKept".into(),
        outcome
            .integrations
            .iter()
            .map(|m| m.family.clone())
            .collect::<Vec<_>>()
            .into(),
    );
    assert_eq!(outcome.renders.len(), 2);
    // The device's family keeps no master; the poses' family does.
    assert_eq!(outcome.integrations.len(), 1);

    // 5. "Measure this image": the device's image, where it is.
    let image = night_folder
        .join(&target_folder)
        .join("device")
        .join(&device.families[0].images[0]);
    let measured = master_quality::measure_for_target(
        &night_folder,
        &target_folder,
        &image,
        true,
        &python,
        &quality_script,
        None,
    )
    .unwrap();
    match &measured {
        Outcome::Measured { master, quality } => {
            summary.insert(
                "measured".into(),
                serde_json::json!({ "master": master, "quality": quality }),
            );
        }
        Outcome::Refused { code } => panic!("refused: {code}"),
    }
    std::fs::write(
        out.join("summary.json"),
        serde_json::to_string_pretty(&serde_json::Value::Object(summary)).unwrap(),
    )
    .unwrap();
}

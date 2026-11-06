//! Through the public API only: the "Folder" step creates a night
//! folder, the person drops frames in it (the invented headers of
//! `tests/fixtures/headers/`, one FITS light, one FITS dark, one XISF
//! flat), and the detection says what the sheet says. No real frame, place
//! or name.

use astro_sync_core::detection::{
    DetectOptions, DetectionInput, DetectionTarget, MinimumSource, detect_night, keys,
};
use astro_sync_core::working_folder::{
    NightFolderSpec, NightLocation, NightTargetSpec, locate_night, prepare_night_folder,
};
use std::fs;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/headers")
        .join(name)
}

fn temp_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "astro-sync-night-folder-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn a_night_prepared_then_filled_is_read_by_its_headers() {
    let root = temp_root("filled");
    let spec = NightFolderSpec {
        night_id: "7f3a2c91-0000-4000-8000-00000000000a".to_string(),
        service_url: "https://service.example".to_string(),
        evening_date: "2026-10-07".to_string(),
        label: "Col des Étoiles".to_string(),
        targets: vec![
            NightTargetSpec {
                target_id: "t-ngc7000".to_string(),
                designation: "NGC 7000".to_string(),
                ticked: true,
            },
            NightTargetSpec {
                target_id: "t-ic1396".to_string(),
                designation: "IC 1396".to_string(),
                ticked: true,
            },
        ],
        follow: None,
    };
    let outcome = prepare_night_folder(&root, &[], &spec, "2026-10-08T09:00:00Z").unwrap();
    let night = outcome.folder;
    assert_eq!(
        night.file_name().unwrap().to_string_lossy(),
        "2026-10-07 · Col des Étoiles · 7f3a2c"
    );

    // NINA dropped a NGC 7000 light in IC 1396's folder and the flat in the
    // lights; the dark is where it belongs.
    fs::copy(
        fixture("light_ngc7000_oiii.fits"),
        night.join("NGC 7000/lights/Light_0001.fits"),
    )
    .unwrap();
    fs::copy(
        fixture("light_ngc7000_oiii.fits"),
        night.join("IC 1396/lights/Light_0002.fits"),
    )
    .unwrap();
    fs::copy(
        fixture("flat_oiii.xisf"),
        night.join("NGC 7000/lights/Flat_0001.xisf"),
    )
    .unwrap();
    fs::copy(
        fixture("dark_300s_g100.fits"),
        night.join("calibration/darks/Dark_0001.fits"),
    )
    .unwrap();
    fs::write(night.join("NGC 7000/final/NGC 7000.tif"), b"tiff").unwrap();

    // Renamed by the person: still found by its identity.
    let renamed = root.join("Ma nuit");
    fs::rename(&night, &renamed).unwrap();
    assert_eq!(
        locate_night(&root, &[], &spec.night_id).unwrap(),
        NightLocation::Found {
            folder: renamed.clone()
        }
    );

    let input = DetectionInput {
        evening_date: Some("2026-10-07".to_string()),
        night_started_at: Some("2026-10-07T19:00:00Z".to_string()),
        longitude: None,
        targets: vec![
            DetectionTarget {
                target_id: "t-ngc7000".to_string(),
                designation: "NGC 7000".to_string(),
                aliases: vec!["North America Nebula".to_string()],
                ticked: true,
                online_final: false,
                stacking_planned: false,
            },
            DetectionTarget {
                target_id: "t-ic1396".to_string(),
                designation: "IC 1396".to_string(),
                aliases: Vec::new(),
                ticked: true,
                online_final: false,
                stacking_planned: false,
            },
        ],
    };
    let detection = detect_night(&renamed, &input, &DetectOptions::default()).unwrap();
    let lines = detection.sentences();
    let expected = [
        "Neither FITS frames nor a final image: put the frames in IC 1396/lights/ or a final image (TIFF, JPEG or PNG) in IC 1396/final/.",
        "1 flat filed in NGC 7000/lights/: its header says FLAT. To move into calibration/flats/.",
        "1 frame of NGC 7000 in IC 1396/lights/ (OBJECT header).",
        "No log: no steps, no guiding quality.",
    ];
    for line in expected {
        assert!(
            lines.iter().any(|l| l == line),
            "missing {line:?} in {lines:?}"
        );
    }
    assert_eq!(lines.len(), expected.len(), "{lines:?}");
    // The flat lies in the lights but is a flat of OIII: no "missing
    // flats"; the dark covers 300 s · gain 100: no "missing darks".
    assert!(
        detection
            .findings
            .iter()
            .all(|f| f.key != keys::FLATS_MISSING && f.key != keys::DARKS_MISSING)
    );
    assert_eq!(detection.targets[0].minimum, MinimumSource::FinalFile);
    assert_eq!(detection.targets[0].light_frames, 1);
    assert_eq!(detection.targets[1].minimum, MinimumSource::Missing);
    fs::remove_dir_all(&root).unwrap();
}

/// The stacking's scratch lives in the night folder, in the hidden
/// `.astro-sync-work/`, and nothing reads it as the night.
#[test]
fn the_hidden_work_folder_of_a_night_is_never_read_as_the_night() {
    use astro_sync_core::detection::night_folder_fingerprint;
    use astro_sync_core::scanner::{scan_folder, walk_folder};
    use astro_sync_core::working_folder::{WORK_DIR, stacking_scratch_root};

    let root = temp_root("work-dir");
    let spec = NightFolderSpec {
        night_id: "7f3a2c91-0000-4000-8000-00000000000b".to_string(),
        service_url: "https://service.example".to_string(),
        evening_date: "2026-10-07".to_string(),
        label: "Col des Étoiles".to_string(),
        targets: vec![NightTargetSpec {
            target_id: "t-ngc7000".to_string(),
            designation: "NGC 7000".to_string(),
            ticked: true,
        }],
        follow: None,
    };
    let night = prepare_night_folder(&root, &[], &spec, "2026-10-08T09:00:00Z")
        .unwrap()
        .folder;
    fs::copy(
        fixture("light_ngc7000_oiii.fits"),
        night.join("NGC 7000/lights/Light_0001.fits"),
    )
    .unwrap();
    let input = DetectionInput {
        evening_date: Some("2026-10-07".to_string()),
        night_started_at: Some("2026-10-07T19:00:00Z".to_string()),
        longitude: None,
        targets: vec![DetectionTarget {
            target_id: "t-ngc7000".to_string(),
            designation: "NGC 7000".to_string(),
            aliases: Vec::new(),
            ticked: true,
            online_final: false,
            stacking_planned: false,
        }],
    };
    let before_fingerprint = night_folder_fingerprint(&night).unwrap();
    let before_detection = detect_night(&night, &input, &DetectOptions::default()).unwrap();
    let listed = |night: &Path| -> Vec<(String, String)> {
        scan_folder(night)
            .unwrap()
            .files
            .into_iter()
            .map(|file| (file.relative_path, file.sha256))
            .collect()
    };
    let before_files = listed(&night);

    // A stacking in progress: FITS, a log and a frames list in the scratch.
    let scratch = stacking_scratch_root(&night).join("NGC 7000_2026-10-07");
    fs::create_dir_all(scratch.join("input/lights")).unwrap();
    fs::copy(
        fixture("light_ngc7000_oiii.fits"),
        scratch.join("input/lights/light_00001.fits"),
    )
    .unwrap();
    fs::write(scratch.join("siril.log"), b"log").unwrap();
    fs::write(
        stacking_scratch_root(&night).join("NGC 7000_2026-10-07.lights.txt"),
        b"x\n",
    )
    .unwrap();
    assert!(night.join(WORK_DIR).is_dir());

    assert_eq!(
        night_folder_fingerprint(&night).unwrap(),
        before_fingerprint,
        "same fingerprint"
    );
    let after_detection = detect_night(&night, &input, &DetectOptions::default()).unwrap();
    assert_eq!(after_detection.sentences(), before_detection.sentences());
    assert_eq!(
        after_detection.findings.len(),
        before_detection.findings.len()
    );
    let after_files = listed(&night);
    assert_eq!(after_files, before_files, "nothing of it in the package");
    assert!(
        walk_folder(&night, None)
            .unwrap()
            .files
            .iter()
            .all(|file| !file.relative_path.contains(WORK_DIR))
    );
    fs::remove_dir_all(&root).unwrap();
}

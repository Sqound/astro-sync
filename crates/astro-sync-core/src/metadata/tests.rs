//! The proposals on synthetic nights: invented targets (M31),
//! headers built here, no real frame, place or name.

use super::*;
use crate::analysis::fits::AcquisitionFamily;
use crate::detection::{
    DetectOptions, DetectionInput, DetectionTarget, detect_night, normalize_object,
};
use crate::night_online::NightOnline;
use crate::pipelines;
use crate::precheck::{ImageCandidate, NotSent};
use crate::test_support::{TempDir, fits_bytes};
use crate::working_folder::{NightFolderSpec, NightTargetSpec, prepare_night_folder};
use std::fs;
use std::path::PathBuf;

/// The generated title of the proposals in `language`.
fn title_of(proposals: &Proposals, language: &str) -> Option<String> {
    proposals
        .variant_generated_title
        .as_ref()
        .and_then(|texts| texts.get(language).cloned())
}

/// The generated description of the proposals in `language`.
fn description_of(proposals: &Proposals, language: &str) -> Option<String> {
    proposals
        .variant_generated_description
        .as_ref()
        .and_then(|texts| texts.get(language).cloned())
}

/// A generated text in English alone.
fn english(text: &str) -> GeneratedText {
    GeneratedText::from([("en".to_string(), text.to_string())])
}

const STARTED: &str = "2026-10-07T20:00:00Z";

fn family(
    filter: Option<&str>,
    exposure: Option<f64>,
    gain: Option<f64>,
    binning: Option<&str>,
    temperature: Option<i64>,
    frames: usize,
) -> AcquisitionFamily {
    AcquisitionFamily {
        key: "k".to_string(),
        filter: filter.map(str::to_string),
        exposure_seconds: exposure,
        gain,
        binning: binning.map(str::to_string),
        temperature_class_c: temperature,
        camera: None,
        frame_paths: (0..frames).map(|i| format!("f{i}.fits")).collect(),
    }
}

fn quality(arcsec: Option<f64>) -> ImageQuality {
    ImageQuality {
        fwhm_pixels: 3.8,
        fwhm_arcsec: arcsec,
        eccentricity: Some(0.31),
        star_count: 212,
        measured_on: "linear".into(),
        measured_at: "2026-10-07T23:40:00Z".into(),
        tool: QualityTool {
            name: "Astro Sync".into(),
            version: "0.1.0".into(),
        },
    }
}

/* ------------------------------ Acquisition ------------------------------ */

#[test]
fn acquisition_lines_merge_the_temperature_and_binning_of_one_filter_and_exposure() {
    let lines = acquisition_lines(&[
        family(
            Some("Ha"),
            Some(300.0),
            Some(100.0),
            Some("1x1"),
            Some(-10),
            8,
        ),
        family(
            Some("Ha"),
            Some(300.0),
            Some(100.0),
            Some("1x1"),
            Some(-9),
            4,
        ),
        family(
            Some("OIII"),
            Some(300.0),
            Some(100.0),
            Some("1x1"),
            Some(-10),
            20,
        ),
        family(Some("Ha"), Some(60.0), None, None, None, 3),
    ]);
    assert_eq!(lines.len(), 3);
    // The most numerous first; Ha 300 s gathers 8 + 4.
    assert_eq!(lines[0].filter.as_deref(), Some("OIII"));
    assert_eq!(lines[0].frame_count, 20);
    assert_eq!(lines[1].filter.as_deref(), Some("Ha"));
    assert_eq!(lines[1].frame_count, 12);
    assert_eq!(lines[1].gain, Some(100));
    assert_eq!(lines[2].exposure_seconds, 60.0);
    assert_eq!(lines[2].gain, None);
}

#[test]
fn a_frame_without_an_exposure_or_out_of_bounds_makes_no_line() {
    let lines = acquisition_lines(&[
        family(Some("L"), None, None, None, None, 5),
        family(Some("L"), Some(0.0), None, None, None, 5),
        family(Some("L"), Some(90_000.0), None, None, None, 5),
        family(Some("L"), Some(f64::NAN), None, None, None, 5),
        family(None, Some(30.0), None, None, None, 2),
    ]);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].filter, None);
    assert_eq!(lines[0].frame_count, 2);
}

#[test]
fn a_filter_name_longer_than_the_contract_allows_is_cut() {
    let long = "F".repeat(60);
    let lines = acquisition_lines(&[family(Some(&long), Some(10.0), None, None, None, 1)]);
    assert_eq!(lines[0].filter.as_deref().unwrap().chars().count(), 40);
}

#[test]
fn the_sensor_is_the_most_numerous_familys() {
    let sensor = sensor_of(&[
        family(Some("Ha"), Some(300.0), None, Some("1x1"), Some(-10), 30),
        family(Some("L"), Some(60.0), None, Some("2x2"), Some(5), 4),
    ])
    .unwrap();
    assert_eq!(sensor.temperature_c, Some(-10.0));
    assert_eq!(sensor.binning, Some(1));
    assert_eq!(
        sensor_of(&[family(None, Some(1.0), None, None, None, 1)]),
        None
    );
    let binned = sensor_of(&[family(None, Some(1.0), None, Some("2X2"), None, 1)]).unwrap();
    assert_eq!(binned.binning, Some(2));
    assert_eq!(binned.temperature_c, None);
}

/* ------------------------------ Calibration ------------------------------ */

#[test]
fn the_calibration_counts_the_folders_in_fixed_words() {
    let counts = CalibrationCounts {
        darks: 30,
        flats: 1,
        dark_flats: 30,
        biases: 0,
    };
    assert_eq!(
        calibration_text(&counts).as_deref(),
        Some("30 darks · 1 flat · 30 dark-flats")
    );
    assert_eq!(calibration_text(&CalibrationCounts::default()), None);
    let all = CalibrationCounts {
        darks: 1,
        flats: 2,
        dark_flats: 3,
        biases: 1,
    };
    assert_eq!(
        calibration_text(&all).as_deref(),
        Some("1 dark · 2 flats · 3 dark-flats · 1 bias")
    );
}

/* ------------------------------ What was left ---------------------------- */

#[test]
fn what_the_service_left_is_said_in_words() {
    let edited = SkippedField {
        field: "calibration".into(),
        reason: "edited".into(),
    };
    let deleted = SkippedField {
        field: "acquisition".into(),
        reason: "deleted".into(),
    };
    assert_eq!(skipped_sentence(&[]), None);
    assert_eq!(
        skipped_sentence(std::slice::from_ref(&edited)).as_deref(),
        Some("The values you entered in your Astro Portal space are kept.")
    );
    // A proposal the space has no right to keep is not told.
    let capability = SkippedField {
        field: "imageQuality".into(),
        reason: "capability".into(),
    };
    assert_eq!(skipped_sentence(std::slice::from_ref(&capability)), None);
    assert_eq!(
        visible_skipped(&[capability.clone(), edited.clone()]),
        std::slice::from_ref(&edited)
    );
    assert!(
        skipped_sentence(&[capability, edited.clone()])
            .unwrap()
            .contains("entered")
    );
    let both = skipped_sentence(&[edited, deleted]).unwrap();
    assert!(both.contains("The values you entered in your Astro Portal space are kept."));
    assert!(both.contains("deleted in your Astro Portal space are not put back"));
}

/* -------------------------------- The shape ------------------------------ */

#[test]
fn the_proposals_write_only_the_fields_a_device_may_propose() {
    let proposals = Proposals {
        acquisition: vec![AcquisitionLine {
            filter: Some("Ha".into()),
            frame_count: 12,
            exposure_seconds: 300.0,
            gain: Some(100),
        }],
        calibration: Some("30 darks".into()),
        sensor: Some(SensorProposal {
            temperature_c: Some(-10.0),
            binning: Some(1),
        }),
        processing: Some(ProcessingProposal {
            software: Some("Astro Sync · Siril 1.4.4".into()),
            steps: Some("raw · 12 lights".into()),
        }),
        variant_processing: Some(ProcessingProposal {
            software: Some("Astro Sync · Siril 1.4.4".into()),
            steps: Some("raw · contrast strong · 12 lights".into()),
        }),
        variant_generated_title: Some(english("Natural render")),
        variant_generated_description: Some(english("The sensor's colours.")),
        image_quality: Some(quality(Some(3.1))),
    };
    let json = serde_json::to_value(&proposals).unwrap();
    let mut keys: Vec<&str> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    // Never the notes, the equipment, the night, the place, a capture's own
    // title or description: those are the owner's (`web_session_required`).
    assert_eq!(
        keys,
        [
            "acquisition",
            "calibration",
            "imageQuality",
            "processing",
            "sensor",
            "variantGeneratedDescription",
            "variantGeneratedTitle",
            "variantProcessing"
        ]
    );
    assert_eq!(json["acquisition"][0]["frameCount"], 12);
    assert_eq!(json["acquisition"][0]["exposureSeconds"], 300.0);
    assert_eq!(
        serde_json::to_value(Proposals::default()).unwrap(),
        serde_json::json!({})
    );
}

#[test]
fn the_capture_part_and_the_image_part_split_the_proposals() {
    let proposals = Proposals {
        calibration: Some("30 darks".into()),
        variant_generated_title: Some(english("Natural render")),
        ..Proposals::default()
    };
    assert!(proposals.has_capture_part());
    assert!(proposals.has_image_part());
    assert_eq!(proposals.capture_part().variant_generated_title, None);
    // The steps of an image are its own: the capture's part never carries them.
    let with_steps = Proposals {
        variant_processing: Some(ProcessingProposal {
            software: None,
            steps: Some("raw".into()),
        }),
        ..Proposals::default()
    };
    assert!(!with_steps.has_capture_part());
    assert!(with_steps.has_image_part());
    assert_eq!(proposals.image_part().calibration, None);
    assert_eq!(
        title_of(&proposals.image_part(), "en").as_deref(),
        Some("Natural render")
    );
    assert!(!Proposals::default().has_capture_part());
}

/* ---------------------------- A night, in files -------------------------- */

struct Night {
    _root: TempDir,
    folder: PathBuf,
    detection: crate::detection::NightDetection,
}

fn target_id() -> String {
    format!("t-{}", normalize_object("M31"))
}

/// A night with M31: 8 Ha lights of 300 s, 4 OIII lights of 180 s, 3 darks,
/// 2 flats, a stacked render with its provenance.
fn night() -> Night {
    night_with(false)
}

/// [`night`], its frames saying their optics (a 3.76 µm pixel behind 250 mm:
/// 3.10 arc seconds a pixel) when `optics`.
fn night_with(optics: bool) -> Night {
    let root = TempDir::new("metadata-night");
    let spec = NightFolderSpec {
        night_id: "7f3a2c91-test".to_string(),
        service_url: "https://service.example".to_string(),
        evening_date: "2026-10-07".to_string(),
        label: "Col des Étoiles".to_string(),
        targets: vec![NightTargetSpec {
            target_id: target_id(),
            designation: "M31".to_string(),
            ticked: true,
        }],
        follow: None,
    };
    let folder = prepare_night_folder(root.path(), &[], &spec, STARTED)
        .unwrap()
        .folder;
    let write = |relative: &str, bytes: &[u8]| {
        let path = folder.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    };
    let frame = |kind: &str, filter: &str, exposure: &str, n: usize| {
        let cards: Vec<(String, String)> = vec![
            ("SIMPLE".into(), "T".into()),
            ("IMAGETYP".into(), format!("'{kind}'")),
            ("OBJECT".into(), "'M31'".into()),
            ("FILTER".into(), format!("'{filter}'")),
            ("EXPTIME".into(), exposure.into()),
            ("GAIN".into(), "100".into()),
            ("CCD-TEMP".into(), "-10.2".into()),
            ("XBINNING".into(), "1".into()),
            ("YBINNING".into(), "1".into()),
            ("DATE-OBS".into(), format!("'2026-10-07T21:{n:02}:00'")),
        ]
        .into_iter()
        .chain(optics.then(|| ("XPIXSZ".to_string(), "3.76".to_string())))
        .chain(optics.then(|| ("FOCALLEN".to_string(), "250".to_string())))
        .collect();
        let refs: Vec<(&str, &str)> = cards
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        fits_bytes(&refs, 64)
    };
    for n in 0..8 {
        write(
            &format!("M31/lights/ha_{n}.fits"),
            &frame("Light Frame", "Ha", "300", n),
        );
    }
    for n in 0..4 {
        write(
            &format!("M31/lights/oiii_{n}.fits"),
            &frame("Light Frame", "OIII", "180", 10 + n),
        );
    }
    for n in 0..3 {
        write(
            &format!("calibration/darks/dark_{n}.fits"),
            &frame("Dark Frame", "Ha", "300", 30 + n),
        );
    }
    for n in 0..2 {
        write(
            &format!("calibration/flats/flat_{n}.fits"),
            &frame("Flat Frame", "Ha", "2", 40 + n),
        );
    }
    write("M31/stacked/M31_2026-10-07_raw.tif", b"tiff-a");
    write(
        "M31/stacked/M31_2026-10-07_dualband-correction.tif",
        b"tiff-b",
    );
    write("M31/stacked/M31_2026-10-07_linear.tif", b"tiff-c");
    write(
        "M31/stacked/M31_2026-10-07_stacking.json",
        serde_json::json!({
            "format": 1,
            "frames": { "lights": 12, "darks": 3, "flats": 2, "dark_flats": 0, "biases": 0 },
            "stackedFrames": 11,
            "tools": { "siril": "siril 1.4.4", "starnet2": true },
            "outputs": [
                { "pipeline": "raw", "file": "M31_2026-10-07_raw.tif" },
                { "pipeline": "dualband-correction", "file": "M31_2026-10-07_dualband-correction.tif" },
                { "pipeline": "linear", "file": "M31_2026-10-07_linear.tif" }
            ]
        })
        .to_string()
        .as_bytes(),
    );
    let input = DetectionInput {
        evening_date: Some("2026-10-07".to_string()),
        night_started_at: Some(STARTED.to_string()),
        longitude: None,
        targets: vec![DetectionTarget {
            target_id: target_id(),
            designation: "M31".to_string(),
            aliases: Vec::new(),
            ticked: true,
            online_final: false,
            stacking_planned: false,
        }],
    };
    let detection = detect_night(&folder, &input, &DetectOptions::default()).unwrap();
    Night {
        _root: root,
        folder,
        detection,
    }
}

fn candidate(night: &Night, relative: &str, kind: ResultKind, sha: &str) -> ImageCandidate {
    ImageCandidate {
        render_rank: None,
        target_id: target_id(),
        path: night.folder.join(relative),
        sha256: sha.to_string(),
        size: 6,
        kind,
        render: None,
        contrast: crate::stacking::Contrast::None,
        retouch: Default::default(),
        frame_count: None,
        twin_sha256: None,
    }
}

fn plan(image: ImageCandidate, decision: Decision) -> ImagePlan {
    ImagePlan {
        image,
        decision,
        metadata: None,
        from_device: false,
    }
}

fn bare_online() -> NightOnline {
    serde_json::from_str(include_str!("../../tests/fixtures/online/night_bare.json")).unwrap()
}

#[test]
fn a_nights_files_give_the_acquisition_the_sensor_and_the_calibration() {
    let night = night();
    let list = pipelines::read_manifests(None, None, "fr");
    let ctx = ProposalContext {
        night_folder: &night.folder,
        detection: &night.detection,
        language: "fr",
        pipelines: Some(&list),
        image_quality: true,
    };
    let families = progression_images::target_families(&night.folder, "M31");
    let image = night.folder.join("M31/final/m31.tif");
    let proposals = proposals_for_image(&image, ResultKind::Final, None, &families, &ctx);

    assert_eq!(proposals.acquisition.len(), 2);
    assert_eq!(proposals.acquisition[0].filter.as_deref(), Some("Ha"));
    assert_eq!(proposals.acquisition[0].frame_count, 8);
    assert_eq!(proposals.acquisition[0].exposure_seconds, 300.0);
    assert_eq!(proposals.acquisition[0].gain, Some(100));
    assert_eq!(proposals.acquisition[1].filter.as_deref(), Some("OIII"));
    assert_eq!(proposals.acquisition[1].exposure_seconds, 180.0);
    assert_eq!(proposals.calibration.as_deref(), Some("3 darks · 2 flats"));
    let sensor = proposals.sensor.unwrap();
    assert_eq!(sensor.temperature_c, Some(-10.0));
    assert_eq!(sensor.binning, Some(1));
    // A final image was processed elsewhere: Astro Sync claims no processing
    // and names no rendering.
    assert_eq!(proposals.processing, None);
    assert_eq!(proposals.variant_generated_title, None);
}

#[test]
fn the_linear_image_without_processing_is_titled_in_the_language_of_the_app() {
    let night = night();
    let families = progression_images::target_families(&night.folder, "M31");
    let image = night.folder.join("M31/stacked/M31_2026-10-07_linear.tif");
    for (language, title) in [
        ("fr", "Sans traitement"),
        ("en", "No processing"),
        ("ja", "処理なし"),
        ("zh-Hant", "不作處理"),
    ] {
        // "No processing" is not a pipeline: its texts do not come from
        // the list, which can be missing altogether.
        for list in [None, Some(pipelines::read_manifests(None, None, language))] {
            let ctx = ProposalContext {
                night_folder: &night.folder,
                detection: &night.detection,
                language,
                pipelines: list.as_ref(),
                image_quality: true,
            };
            let proposals =
                proposals_for_image(&image, ResultKind::Stacked, Some("linear"), &families, &ctx);
            assert_eq!(
                title_of(&proposals, language).as_deref(),
                Some(title),
                "{language}"
            );
            assert!(description_of(&proposals, language).is_some_and(|d| !d.is_empty()));
            assert_eq!(
                proposals.processing.unwrap().steps.as_deref(),
                Some("linear · 11 lights · 3 darks · 2 flats")
            );
        }
    }
}

#[test]
fn a_stacked_image_gives_its_processing_and_the_rendering_in_the_language_of_the_app() {
    let night = night();
    let families = progression_images::target_families(&night.folder, "M31");
    let image = night.folder.join("M31/stacked/M31_2026-10-07_raw.tif");
    for (language, title) in [
        ("fr", "Rendu naturel"),
        ("en", "Natural render"),
        ("ja", "ナチュラルな仕上がり"),
    ] {
        let list = pipelines::read_manifests(None, None, language);
        let ctx = ProposalContext {
            night_folder: &night.folder,
            detection: &night.detection,
            language,
            pipelines: Some(&list),
            image_quality: true,
        };
        let proposals = proposals_for_image(&image, ResultKind::Stacked, None, &families, &ctx);
        assert_eq!(
            title_of(&proposals, language).as_deref(),
            Some(title),
            "{language}"
        );
        assert!(description_of(&proposals, language).is_some_and(|d| !d.is_empty()));
        let processing = proposals.processing.unwrap();
        assert_eq!(
            processing.software,
            Some(format!("Astro Sync {} · Siril 1.4.4", crate::version()))
        );
        assert_eq!(
            processing.steps.as_deref(),
            Some("raw · 11 lights · 3 darks · 2 flats")
        );
    }
    // The other render of the same stacking is found by its own file name.
    let list = pipelines::read_manifests(None, None, "fr");
    let ctx = ProposalContext {
        night_folder: &night.folder,
        detection: &night.detection,
        language: "fr",
        pipelines: Some(&list),
        image_quality: true,
    };
    let other = night
        .folder
        .join("M31/stacked/M31_2026-10-07_dualband-correction.tif");
    let proposals = proposals_for_image(&other, ResultKind::Stacked, None, &families, &ctx);
    // "Dual-band correction", the name of the shipped definition.
    assert_eq!(
        title_of(&proposals, "fr").as_deref(),
        Some("Correction bibande")
    );
    assert!(
        proposals
            .processing
            .unwrap()
            .steps
            .unwrap()
            .starts_with("dualband-correction")
    );
}

/// The renders started from the image a device stacked. Astro Sync
/// stacked no pose, so Siril is not named, the image is.
#[test]
fn an_image_rendered_from_the_image_of_a_device_says_so_in_its_processing() {
    let night = night();
    let families = progression_images::target_families(&night.folder, "M31");
    fs::write(
        night
            .folder
            .join("M31/stacked/M31_2026-10-09_stacking.json"),
        serde_json::json!({
            "format": 1,
            "frames": { "lights": 0, "darks": 0, "flats": 0, "dark_flats": 0, "biases": 0 },
            "stackedFrames": 430,
            "tools": { "siril": null, "starnet2": false },
            "quality": null,
            "outputs": [{ "pipeline": "raw", "file": "M31_2026-10-09_raw.tif" }],
            "deviceMaster": { "file": "Stacked_430.fit", "device": "Seestar S50 Pro", "stackedFrames": 430 }
        })
        .to_string(),
    )
    .unwrap();
    fs::write(
        night.folder.join("M31/stacked/M31_2026-10-09_raw.tif"),
        b"tiff-d",
    )
    .unwrap();
    let list = pipelines::read_manifests(None, None, "en");
    let ctx = ProposalContext {
        night_folder: &night.folder,
        detection: &night.detection,
        language: "en",
        pipelines: Some(&list),
        image_quality: true,
    };
    let image = night.folder.join("M31/stacked/M31_2026-10-09_raw.tif");
    let proposals = proposals_for_image(&image, ResultKind::Stacked, None, &families, &ctx);
    let processing = proposals.processing.unwrap();
    assert_eq!(
        processing.software,
        Some(format!("Astro Sync {}", crate::version()))
    );
    assert_eq!(
        processing.steps.as_deref(),
        Some("raw · Seestar S50 Pro stack · 430 lights")
    );
    // The quality is measured on request: the render gives none.
    assert_eq!(proposals.image_quality, None);
    // The window learns the origin from the provenance, plan by plan.
    assert!(recorded_device_master(&image));
    assert!(!recorded_device_master(
        &night.folder.join("M31/stacked/M31_2026-10-07_raw.tif")
    ));
}

#[test]
fn a_stacked_image_without_a_provenance_names_no_rendering() {
    let night = night();
    let families = progression_images::target_families(&night.folder, "M31");
    let list = pipelines::read_manifests(None, None, "fr");
    let ctx = ProposalContext {
        night_folder: &night.folder,
        detection: &night.detection,
        language: "fr",
        pipelines: Some(&list),
        image_quality: true,
    };
    let alone = night.folder.join("M31/stacked/unknown.tif");
    let proposals = proposals_for_image(&alone, ResultKind::Stacked, None, &families, &ctx);
    assert_eq!(proposals.variant_generated_title, None);
    assert_eq!(proposals.processing, None);
    assert!(!proposals.acquisition.is_empty());
}

#[test]
fn a_rendering_name_longer_than_the_contract_allows_is_cut_with_an_ellipsis() {
    let text = "é".repeat(100);
    let cut = clip(&text, 60);
    assert_eq!(cut.chars().count(), 60);
    assert!(cut.ends_with('…'));
    assert_eq!(clip("  court  ", 60), "court");
}

#[test]
fn the_decision_decides_what_each_image_carries() {
    let night = night();
    let list = pipelines::read_manifests(None, None, "fr");
    let ctx = ProposalContext {
        night_folder: &night.folder,
        detection: &night.detection,
        language: "fr",
        pipelines: Some(&list),
        image_quality: true,
    };
    let stacked = |sha: &str| {
        candidate(
            &night,
            "M31/stacked/M31_2026-10-07_raw.tif",
            ResultKind::Stacked,
            sha,
        )
    };
    let mut plans = vec![
        plan(stacked("a"), Decision::NewCapture),
        plan(
            stacked("b"),
            Decision::Variant {
                capture_id: "capture-1".into(),
            },
        ),
        plan(
            stacked("c"),
            Decision::VariantOfNew {
                first_sha256: "a".into(),
            },
        ),
        plan(
            stacked("d"),
            Decision::NotSent {
                reason: NotSent::OneRenderOnly,
            },
        ),
        plan(
            stacked("e"),
            Decision::AlreadyOnline {
                capture_id: "capture-9".into(),
                image_id: "image-9".into(),
            },
        ),
    ];
    attach_proposals(&mut plans, &bare_online(), &ctx);

    // A new capture carries everything.
    let new = plans[0].metadata.as_ref().unwrap();
    assert!(
        !new.acquisition.is_empty()
            && new.processing.is_some()
            && new.variant_generated_title.is_some()
    );
    // A variant of a capture online: texts and what the capture may lack,
    // never the processing (the main image's).
    let variant = plans[1].metadata.as_ref().unwrap();
    assert!(!variant.acquisition.is_empty() && variant.variant_generated_title.is_some());
    assert_eq!(variant.processing, None);
    // ... but it keeps the steps of its own render with it: the capture shows
    // them once it is the main image.
    assert!(variant.variant_processing.is_some());
    // A variant of the capture this sending creates: its texts and its steps only.
    let of_new = plans[2].metadata.as_ref().unwrap();
    assert_eq!(*of_new, of_new.image_part());
    assert!(of_new.variant_generated_title.is_some());
    assert!(of_new.variant_processing.is_some());
    assert_eq!(new.variant_processing, new.processing);
    // Not sent: nothing.
    assert_eq!(plans[3].metadata, None);
    // Online (not found among the images of the night): not the main image,
    // so no processing.
    let online = plans[4].metadata.as_ref().unwrap();
    assert_eq!(online.processing, None);
    assert!(!online.acquisition.is_empty());
}

#[test]
fn a_progression_image_proposes_nothing() {
    let night = night();
    let ctx = ProposalContext {
        night_folder: &night.folder,
        detection: &night.detection,
        language: "fr",
        pipelines: None,
        image_quality: true,
    };
    let mut plans = vec![plan(
        candidate(
            &night,
            "M31/final/progress.png",
            ResultKind::Progression,
            "p",
        ),
        Decision::NewCapture,
    )];
    attach_proposals(&mut plans, &bare_online(), &ctx);
    assert_eq!(plans[0].metadata, None);
}

/* ----------------------------- Image quality ----------------------------- */

/// The stacking's provenance of the night, with the `quality` the stacker
/// wrote (and the family of the master it kept), `mutate`d.
fn write_quality(night: &Night, mutate: impl FnOnce(&mut serde_json::Value)) {
    let path = night
        .folder
        .join("M31/stacked/M31_2026-10-07_stacking.json");
    let mut provenance: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    provenance["quality"] = serde_json::json!({
        "fwhmPixels": 3.8, "eccentricity": 0.312, "starCount": 212,
        "measuredOn": "linear", "measuredAt": "2026-10-07T23:40:00Z"
    });
    provenance["integration"] = serde_json::json!({ "family": "Ha_300s_g100" });
    mutate(&mut provenance);
    fs::write(path, provenance.to_string()).unwrap();
}

/// The image with the contrast of presentation says so in its processing
/// and in its title, the plain render of the same pipeline does not.
#[test]
fn an_image_with_a_contrast_says_so_in_its_processing_and_its_title() {
    let night = night_with(true);
    write_quality(&night, |provenance| {
        provenance["outputs"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "pipeline": "raw",
                "file": "M31_2026-10-07_raw_strong.tif",
                "contrast": "strong"
            }));
    });
    fs::write(
        night
            .folder
            .join("M31/stacked/M31_2026-10-07_raw_strong.tif"),
        b"tiff-d",
    )
    .unwrap();
    let plain = stacked_proposals(&night, "raw", false);
    let curved = stacked_proposals(&night, "raw_strong", false);
    let title = title_of(&plain, "fr").unwrap();
    let curved_title = title_of(&curved, "fr").unwrap();
    assert_eq!(
        recorded_contrast(
            &night
                .folder
                .join("M31/stacked/M31_2026-10-07_raw_strong.tif")
        ),
        crate::stacking::Contrast::Strong
    );
    assert_eq!(
        recorded_contrast(&night.folder.join("M31/stacked/M31_2026-10-07_raw.tif")),
        crate::stacking::Contrast::None
    );
    assert!(
        !plain
            .processing
            .unwrap()
            .steps
            .unwrap()
            .contains("contrast")
    );
    assert!(
        curved
            .processing
            .unwrap()
            .steps
            .unwrap()
            .starts_with("raw · contrast strong")
    );
    assert_eq!(curved_title, format!("{title} · Contraste 3,6"));
}

/// The image with a finish (stronger colours, softened stars) says so in its
/// processing and in its title, with the contrast's words when it has one.
#[test]
fn an_image_with_a_finish_says_so_in_its_processing_and_its_title() {
    use crate::stacking::{Colour, Retouch, SoftStars};
    let night = night_with(true);
    write_quality(&night, |provenance| {
        provenance["outputs"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "pipeline": "raw",
                "file": "M31_2026-10-07_raw_strong_colour-strong_stars-strong.tif",
                "contrast": "strong",
                "colour": 40,
                "softStars": 85
            }));
    });
    fs::write(
        night
            .folder
            .join("M31/stacked/M31_2026-10-07_raw_strong_colour-strong_stars-strong.tif"),
        b"tiff-e",
    )
    .unwrap();
    let plain = stacked_proposals(&night, "raw", false);
    let finished = stacked_proposals(&night, "raw_strong_colour-strong_stars-strong", false);
    let title = title_of(&plain, "fr").unwrap();
    let finished_title = title_of(&finished, "fr").unwrap();
    assert_eq!(
        recorded_retouch(
            &night
                .folder
                .join("M31/stacked/M31_2026-10-07_raw_strong_colour-strong_stars-strong.tif")
        ),
        Retouch {
            colour: Colour::Strong,
            soft_stars: SoftStars::Strong
        }
    );
    assert!(recorded_retouch(&night.folder.join("M31/stacked/M31_2026-10-07_raw.tif")).is_none());
    let steps = finished.processing.unwrap().steps.unwrap();
    assert!(
        steps.starts_with("raw · contrast strong · colour 40 · soft stars 85"),
        "{steps}"
    );
    assert!(!plain.processing.unwrap().steps.unwrap().contains("colour"));
    assert_eq!(
        finished_title,
        finish_title(
            &title,
            crate::stacking::Contrast::Strong,
            Retouch {
                colour: Colour::Strong,
                soft_stars: SoftStars::Strong
            },
            "fr"
        )
    );
    assert!(
        finished_title.starts_with(&format!("{title} · Contraste 3,6 · Couleurs")),
        "{finished_title}"
    );
    assert!(
        finished_title.chars().count() <= MAX_VARIANT_TITLE,
        "{finished_title}"
    );
}

/// Two pipelines of one target, each with a finish of its own (« Finition »
/// of its card): each variant's title and processing say that pipeline's
/// values, never the other's.
#[test]
fn each_pipeline_s_variant_says_its_own_finish() {
    use crate::stacking::{Colour, Contrast, Retouch, SoftStars};
    let night = night_with(true);
    let twins = [
        ("raw", "raw_strong_colour-40_stars-85", "strong", 40, 85),
        (
            "dualband-correction",
            "dualband-correction_presentation_stars-50",
            "presentation",
            0,
            50,
        ),
    ];
    write_quality(&night, |provenance| {
        for (pipeline, stem, contrast, colour, stars) in twins {
            let mut entry = serde_json::json!({
                "pipeline": pipeline,
                "file": format!("M31_2026-10-07_{stem}.tif"),
                "contrast": contrast,
                "softStars": stars
            });
            if colour > 0 {
                entry["colour"] = serde_json::json!(colour);
            }
            provenance["outputs"].as_array_mut().unwrap().push(entry);
        }
    });
    for (_, stem, ..) in twins {
        fs::write(
            night
                .folder
                .join(format!("M31/stacked/M31_2026-10-07_{stem}.tif")),
            format!("tiff-{stem}"),
        )
        .unwrap();
    }
    let raw = title_of(&stacked_proposals(&night, "raw", false), "fr").unwrap();
    let dualband = title_of(
        &stacked_proposals(&night, "dualband-correction", false),
        "fr",
    )
    .unwrap();
    let raw_twin = stacked_proposals(&night, twins[0].1, false);
    let dualband_twin = stacked_proposals(&night, twins[1].1, false);
    assert_eq!(
        title_of(&raw_twin, "fr").unwrap(),
        finish_title(
            &raw,
            Contrast::Strong,
            Retouch {
                colour: Colour::Strong,
                soft_stars: SoftStars::Strong
            },
            "fr"
        )
    );
    assert_eq!(
        title_of(&dualband_twin, "fr").unwrap(),
        finish_title(
            &dualband,
            Contrast::Presentation,
            Retouch {
                colour: Colour::None,
                soft_stars: SoftStars::Light
            },
            "fr"
        )
    );
    let steps = dualband_twin.processing.unwrap().steps.unwrap();
    assert!(
        steps.starts_with("dualband-correction · contrast presentation · soft stars 50"),
        "{steps}"
    );
    assert!(!steps.contains("colour"), "{steps}");
}

/// The title of a finish variant says every setting with its value, in the
/// language; a render without a finish keeps the contrast's title as it was.
#[test]
fn the_title_of_a_finish_variant_speaks_the_language() {
    use crate::stacking::{Colour, Contrast, Retouch, SoftStars};
    let strong = Retouch {
        colour: Colour::Strong,
        soft_stars: SoftStars::Strong,
    };
    let colour_only = Retouch {
        colour: Colour::Enhanced,
        soft_stars: SoftStars::None,
    };
    assert_eq!(
        finish_title("Raw", Contrast::Strong, Retouch::default(), "fr"),
        contrast_title("Raw", Contrast::Strong, "fr")
    );
    assert_eq!(
        finish_title("Raw", Contrast::None, Retouch::default(), "en"),
        "Raw"
    );
    for (language, expected) in [
        (
            "fr",
            "Raw · Contraste 3,6 · Couleurs +40\u{a0}% · Étoiles adoucies 85\u{a0}%",
        ),
        (
            "en",
            "Raw · Contrast 3.6 · Colours +40% · Softened stars 85%",
        ),
        (
            "de",
            "Raw · Kontrast 3,6 · Farben +40\u{a0}% · Weichere Sterne 85\u{a0}%",
        ),
        (
            "es",
            "Raw · Contraste 3,6 · Colores +40\u{a0}% · Estrellas suavizadas 85\u{a0}%",
        ),
        (
            "it",
            "Raw · Contrasto 3,6 · Colori +40% · Stelle attenuate 85%",
        ),
        (
            "pt-BR",
            "Raw · Contraste 3,6 · Cores +40% · Estrelas suavizadas 85%",
        ),
        (
            "ja",
            "Raw · コントラスト 3.6 · 色の強さ +40% · 星をやわらげる 85%",
        ),
        (
            "ru",
            "Raw · Контраст 3,6 · Цвета +40\u{a0}% · Смягчённые звёзды 85\u{a0}%",
        ),
        ("zh-Hans", "Raw · 对比度 3.6 · 色彩 +40% · 柔化星点 85%"),
        ("zh-Hant", "Raw · 對比度 3.6 · 色彩 +40% · 柔化星點 85%"),
        (
            "sv",
            "Raw · Contrast 3.6 · Colours +40% · Softened stars 85%",
        ),
    ] {
        let title = finish_title("Raw", Contrast::Strong, strong, language);
        assert_eq!(title, expected, "{language}");
        assert!(
            title.chars().count() <= MAX_VARIANT_TITLE,
            "{language}: {title}"
        );
    }
    // The service keeps 90 characters of a title: a long render name makes the word of the
    // stars shorter, then the values go.
    assert_eq!(
        finish_title("Correction bibande", Contrast::Strong, strong, "fr"),
        "Correction bibande · Contraste 3,6 · Couleurs +40\u{a0}% · Étoiles adoucies 85\u{a0}%"
    );
    assert_eq!(
        finish_title(&"x".repeat(35), Contrast::Strong, strong, "fr"),
        format!(
            "{} · Contraste 3,6 · Couleurs +40\u{a0}% · Étoiles 85\u{a0}%",
            "x".repeat(35)
        )
    );
    assert_eq!(
        finish_title(&"x".repeat(45), Contrast::Strong, strong, "fr"),
        format!("{} · Contraste 3,6 · Couleurs · Étoiles", "x".repeat(45))
    );
    for language in [
        "fr", "en", "de", "es", "it", "pt-BR", "ja", "ru", "zh-Hans", "zh-Hant",
    ] {
        let title = finish_title(&"x".repeat(80), Contrast::Strong, strong, language);
        assert!(
            title.chars().count() <= MAX_VARIANT_TITLE,
            "{language}: {title}"
        );
    }
    assert_eq!(
        finish_title("Raw", Contrast::None, colour_only, "fr"),
        "Raw · Couleurs +20\u{a0}%"
    );
}

/// The place of a render among those the stacking was asked for is the order
/// of the outputs the stacker recorded (the first asked is 0): "No processing"
/// is written last, an image with a curve has the rank of its pipeline, an
/// image the stacker did not write has none.
#[test]
fn a_render_knows_its_place_among_the_renders_asked_for() {
    let night = night_with(true);
    write_quality(&night, |provenance| {
        provenance["outputs"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "pipeline": "raw",
                "file": "M31_2026-10-07_raw_strong.tif",
                "contrast": "strong"
            }));
    });
    let rank = |name: &str| recorded_render_rank(&night.folder.join("M31/stacked").join(name));
    assert_eq!(rank("M31_2026-10-07_raw.tif"), Some(0));
    assert_eq!(rank("M31_2026-10-07_dualband-correction.tif"), Some(1));
    assert_eq!(rank("M31_2026-10-07_linear.tif"), Some(2));
    assert_eq!(rank("M31_2026-10-07_raw_strong.tif"), Some(0));
    assert_eq!(rank("someone-elses.tif"), None);
}

/// The word and the decimal sign of the title follow the language, English
/// for the others; « none » changes nothing.
#[test]
fn the_title_of_a_contrast_variant_speaks_the_language() {
    use crate::stacking::Contrast;
    assert_eq!(contrast_title("Raw", Contrast::None, "fr"), "Raw");
    // A gain of the slider keeps its decimals: 2.5 and 2.25.
    let gain = |value: f64| crate::stacking::Contrast::from_gain(value).unwrap();
    assert_eq!(
        contrast_title("Raw", gain(2.5), "fr"),
        "Raw · Contraste 2,5"
    );
    assert_eq!(
        contrast_title("Raw", gain(2.25), "en"),
        "Raw · Contrast 2.25"
    );
    for (language, expected) in [
        ("fr", "Raw · Contraste 2,0"),
        ("en", "Raw · Contrast 2.0"),
        ("de", "Raw · Kontrast 2,0"),
        ("es", "Raw · Contraste 2,0"),
        ("it", "Raw · Contrasto 2,0"),
        ("ja", "Raw · コントラスト 2.0"),
        ("pt-BR", "Raw · Contraste 2,0"),
        ("ru", "Raw · Контраст 2,0"),
        ("zh-Hans", "Raw · 对比度 2.0"),
        ("zh-Hant", "Raw · 對比度 2.0"),
        ("sv", "Raw · Contrast 2.0"),
    ] {
        assert_eq!(
            contrast_title("Raw", Contrast::Presentation, language),
            expected
        );
    }
    assert_eq!(
        contrast_title("Raw", Contrast::Strong, "en"),
        "Raw · Contrast 3.6"
    );
}

fn stacked_proposals(night: &Night, render: &str, with_right: bool) -> Proposals {
    let families = progression_images::target_families(&night.folder, "M31");
    let list = pipelines::read_manifests(None, None, "fr");
    let ctx = ProposalContext {
        night_folder: &night.folder,
        detection: &night.detection,
        language: "fr",
        pipelines: Some(&list),
        image_quality: with_right,
    };
    let image = night
        .folder
        .join(format!("M31/stacked/M31_2026-10-07_{render}.tif"));
    proposals_for_image(&image, ResultKind::Stacked, None, &families, &ctx)
}

#[test]
fn a_stacked_image_carries_the_quality_the_stacker_measured_in_arc_seconds() {
    let night = night_with(true);
    write_quality(&night, |_| {});
    // Every render of the stacking has the quality of its master.
    for render in ["raw", "dualband-correction", "linear"] {
        let quality = stacked_proposals(&night, render, true)
            .image_quality
            .unwrap();
        assert_eq!(quality.fwhm_pixels, 3.8);
        // 3.76 µm over 250 mm: 3.1022 arc seconds a pixel.
        let arcsec = quality.fwhm_arcsec.unwrap();
        assert!(
            (arcsec - 3.8 * 206.264_806 * 3.76 / 250.0).abs() < 0.01,
            "{arcsec}"
        );
        assert_eq!(quality.eccentricity, Some(0.31));
        assert_eq!(quality.star_count, 212);
        assert_eq!(quality.measured_on, "linear");
        assert_eq!(quality.measured_at, "2026-10-07T23:40:00Z");
        assert_eq!(quality.tool.name, "Astro Sync");
    }
    // The figures leave under their contract names, and with the image only.
    let proposals = stacked_proposals(&night, "raw", true);
    let json = serde_json::to_value(&proposals).unwrap();
    for key in [
        "fwhmPixels",
        "fwhmArcsec",
        "eccentricity",
        "starCount",
        "measuredOn",
        "measuredAt",
        "tool",
    ] {
        assert!(json["imageQuality"].get(key).is_some(), "{key}");
    }
    assert!(proposals.has_image_part() && proposals.image_part().image_quality.is_some());
    assert_eq!(proposals.capture_part().image_quality, None);
    assert!(proposals.without_processing().image_quality.is_some());
}

#[test]
fn without_analysis_logs_nothing_is_built() {
    let night = night_with(true);
    write_quality(&night, |_| {});
    let proposals = stacked_proposals(&night, "raw", false);
    assert_eq!(proposals.image_quality, None);
    // The rest of the image is proposed all the same.
    assert!(proposals.processing.is_some() && proposals.variant_generated_title.is_some());
}

#[test]
fn the_arc_seconds_wait_for_a_pixel_scale_and_the_figure_for_a_measure() {
    // Frames that say nothing of their optics: pixels only.
    let night = night();
    write_quality(&night, |_| {});
    let quality = stacked_proposals(&night, "raw", true)
        .image_quality
        .unwrap();
    assert_eq!((quality.fwhm_pixels, quality.fwhm_arcsec), (3.8, None));
    // No measure (fewer than twenty stars: `null`), a provenance
    // missing, or a figure measured elsewhere than on the linear
    // master: no quality at all.
    for change in [
        |p: &mut serde_json::Value| p["quality"] = serde_json::Value::Null,
        |p: &mut serde_json::Value| {
            p.as_object_mut().unwrap().remove("quality");
        },
        |p: &mut serde_json::Value| p["quality"]["measuredOn"] = "stretched".into(),
        |p: &mut serde_json::Value| p["quality"]["fwhmPixels"] = 0.0.into(),
        |p: &mut serde_json::Value| p["quality"]["fwhmPixels"] = 5000.0.into(),
    ] {
        let night = night_with(true);
        write_quality(&night, change);
        assert_eq!(stacked_proposals(&night, "raw", true).image_quality, None);
    }
    // An eccentricity outside 0..1 is left out, the width stays.
    let night = night_with(true);
    write_quality(&night, |p| p["quality"]["eccentricity"] = 1.4.into());
    let quality = stacked_proposals(&night, "raw", true)
        .image_quality
        .unwrap();
    assert_eq!(quality.eccentricity, None);
}

#[test]
fn only_the_images_the_stacker_made_have_a_quality() {
    // A processed image (a final image, even one lying in `stacked/`) is
    // never measured: its provenance is not read.
    let night = night_with(true);
    write_quality(&night, |_| {});
    let families = progression_images::target_families(&night.folder, "M31");
    let ctx = ProposalContext {
        night_folder: &night.folder,
        detection: &night.detection,
        language: "fr",
        pipelines: None,
        image_quality: true,
    };
    let own = night.folder.join("M31/stacked/M31_2026-10-07_raw.tif");
    assert_eq!(
        proposals_for_image(&own, ResultKind::Final, None, &families, &ctx).image_quality,
        None
    );
    let foreign = night.folder.join("M31/stacked/my-pixinsight-master.tif");
    assert_eq!(
        proposals_for_image(&foreign, ResultKind::Stacked, None, &families, &ctx).image_quality,
        None
    );
}

#[test]
fn the_decision_decides_who_carries_the_quality() {
    let night = night_with(true);
    write_quality(&night, |_| {});
    let list = pipelines::read_manifests(None, None, "fr");
    let ctx = ProposalContext {
        night_folder: &night.folder,
        detection: &night.detection,
        language: "fr",
        pipelines: Some(&list),
        image_quality: true,
    };
    let stacked = |sha: &str| {
        candidate(
            &night,
            "M31/stacked/M31_2026-10-07_raw.tif",
            ResultKind::Stacked,
            sha,
        )
    };
    let mut plans = vec![
        plan(stacked("a"), Decision::NewCapture),
        plan(
            stacked("b"),
            Decision::Variant {
                capture_id: "capture-1".into(),
            },
        ),
        plan(
            stacked("c"),
            Decision::VariantOfNew {
                first_sha256: "a".into(),
            },
        ),
        plan(
            stacked("d"),
            Decision::NotSent {
                reason: NotSent::OneRenderOnly,
            },
        ),
        plan(
            stacked("e"),
            Decision::AlreadyOnline {
                capture_id: "capture-9".into(),
                image_id: "image-9".into(),
            },
        ),
    ];
    attach_proposals(&mut plans, &bare_online(), &ctx);
    // A new capture, a variant (of a capture online or of the one this
    // sending creates), an image already online: the image's own quality
    // goes with each; the processing stays the main image's.
    for index in [0, 1, 2, 4] {
        let carried = plans[index].metadata.as_ref().unwrap();
        assert!(carried.image_quality.is_some(), "plan {index}");
        assert!(carried.has_image_part(), "plan {index}");
    }
    assert!(
        plans[2]
            .metadata
            .as_ref()
            .unwrap()
            .capture_part()
            .is_empty()
    );
    assert_eq!(plans[3].metadata, None);
}

/* ------------------- A master measured by hand ------------------- */

fn measured_master(night: &Night) {
    crate::master_quality::save(
        &night.folder,
        "M31",
        std::path::Path::new("M31_integration.xisf"),
        &MeasuredQuality {
            fwhm_pixels: 3.8,
            eccentricity: Some(0.31),
            star_count: 212,
            measured_on: Some("linear".into()),
            measured_at: Some("2026-10-14T09:00:00Z".into()),
        },
    )
    .unwrap();
}

fn final_proposals(night: &Night, with_right: bool) -> Proposals {
    let families = progression_images::target_families(&night.folder, "M31");
    let list = pipelines::read_manifests(None, None, "fr");
    let ctx = ProposalContext {
        night_folder: &night.folder,
        detection: &night.detection,
        language: "fr",
        pipelines: Some(&list),
        image_quality: with_right,
    };
    let image = night.folder.join("M31/final/m31.tif");
    proposals_for_image(&image, ResultKind::Final, None, &families, &ctx)
}

#[test]
fn a_final_image_carries_the_quality_of_the_master_a_person_measured() {
    let night = night_with(true);
    assert_eq!(final_proposals(&night, true).image_quality, None);
    measured_master(&night);
    let quality = final_proposals(&night, true).image_quality.unwrap();
    // The same contract as the quality of a stacking Astro Sync made.
    assert_eq!(quality.fwhm_pixels, 3.8);
    assert!((quality.fwhm_arcsec.unwrap() - 3.8 * 206.264_806 * 3.76 / 250.0).abs() < 0.01);
    assert_eq!(quality.eccentricity, Some(0.31));
    assert_eq!(quality.star_count, 212);
    assert_eq!(quality.measured_on, "linear");
    assert_eq!(quality.measured_at, "2026-10-14T09:00:00Z");
    assert_eq!(quality.tool.name, "Astro Sync");
    let proposals = final_proposals(&night, true);
    assert!(proposals.image_part().image_quality.is_some());
    assert_eq!(proposals.capture_part().image_quality, None);
    assert!(proposals.without_processing().image_quality.is_some());
    // Without the right, nothing is built, as for Astro Sync's own stacking.
    assert_eq!(final_proposals(&night, false).image_quality, None);
}

#[test]
fn a_master_measured_by_hand_never_replaces_the_measure_of_the_stacking_and_a_figure_out_of_contract_is_left_out()
 {
    let night = night_with(true);
    write_quality(&night, |_| {});
    crate::master_quality::save(
        &night.folder,
        "M31",
        std::path::Path::new("m.xisf"),
        &MeasuredQuality {
            fwhm_pixels: 9.0,
            star_count: 30,
            measured_on: Some("linear".into()),
            ..MeasuredQuality::default()
        },
    )
    .unwrap();
    // The stacked image keeps what the stacker measured on its own master.
    let stacked = stacked_proposals(&night, "raw", true)
        .image_quality
        .unwrap();
    assert_eq!(stacked.fwhm_pixels, 3.8);
    // A figure the contract would refuse is left out, never sent.
    let night = night_with(true);
    crate::master_quality::save(
        &night.folder,
        "M31",
        std::path::Path::new("m.xisf"),
        &MeasuredQuality {
            fwhm_pixels: 0.0,
            star_count: 30,
            measured_on: Some("linear".into()),
            ..MeasuredQuality::default()
        },
    )
    .unwrap();
    assert_eq!(final_proposals(&night, true).image_quality, None);
}

/// The texts of an image are written in the ten languages of the service
/// whatever the application speaks, English always there: the service shows
/// each reader its own (`capture-variant.schema.json` `generatedTitle`).
#[test]
fn the_texts_of_an_image_are_written_in_the_ten_languages_whatever_the_app_speaks() {
    use crate::stacking::{Colour, Contrast, Retouch, SoftStars};
    let finish = Retouch {
        colour: Colour::Strong,
        soft_stars: SoftStars::Strong,
    };
    for app in ["fr", "ja", "en"] {
        let list = pipelines::read_manifests(None, None, app);
        let (title, description) =
            generated_render_texts(Some(&list), "raw", Contrast::Strong, finish);
        let title = title.expect("a title");
        let description = description.expect("a description");
        assert_eq!(
            title.keys().map(String::as_str).collect::<Vec<_>>(),
            {
                let mut sorted = TEXT_LANGUAGES.to_vec();
                sorted.sort_unstable();
                sorted
            },
            "{app}"
        );
        assert_eq!(description.len(), TEXT_LANGUAGES.len(), "{app}");
        assert!(
            title["en"].starts_with("Natural render · Contrast 3.6 · Colours +40%"),
            "{}",
            title["en"]
        );
        assert!(
            title["fr"].starts_with("Rendu naturel · Contraste 3,6 · Couleurs +40\u{a0}%"),
            "{}",
            title["fr"]
        );
        assert!(title["ja"].starts_with("ナチュラルな仕上がり · コントラスト 3.6"));
        assert!(title["zh-Hant"].contains("對比度"));
        for text in title.values() {
            assert!(text.chars().count() <= MAX_VARIANT_TITLE, "{text}");
        }
    }
    // « No processing » has its own texts, without the list.
    let (title, _) =
        generated_render_texts(None, LINEAR_PIPELINE, Contrast::None, Retouch::default());
    let title = title.expect("a title");
    assert_eq!(title["en"], "No processing");
    assert_eq!(title["fr"], "Sans traitement");
    // A pipeline nobody knows, or no list: nothing to name the image with.
    let list = pipelines::read_manifests(None, None, "fr");
    assert_eq!(
        generated_render_texts(Some(&list), "unknown", Contrast::None, Retouch::default()),
        (None, None)
    );
    assert_eq!(
        generated_render_texts(None, "raw", Contrast::None, Retouch::default()),
        (None, None)
    );
}

/// The title of a finish says every setting in full up to the service's 90
/// characters -- it is not cut at the 60 the service kept before: the longest
/// shipped render, with the contrast and both finishes, in the ten languages.
#[test]
fn a_finish_title_is_kept_whole_up_to_ninety_characters() {
    use crate::stacking::{Colour, Contrast, Retouch, SoftStars};
    let strong = Retouch {
        colour: Colour::Strong,
        soft_stars: SoftStars::Strong,
    };
    let list = pipelines::read_manifests(None, None, "en");
    let mut longest = 0;
    for (id, _) in pipelines::SHIPPED {
        let (base, _) = generated_render_texts(Some(&list), id, Contrast::None, Retouch::default());
        let base = base.expect("a title");
        let finished = finished_title(&base, Contrast::Strong, strong);
        assert_eq!(finished.len(), TEXT_LANGUAGES.len(), "{id}");
        for (language, text) in &finished {
            let length = text.chars().count();
            assert!(length <= MAX_VARIANT_TITLE, "{id} {language}: {text}");
            // Whole: the three settings are there with their values, or the
            // shortened forms -- never an ellipsis cutting the end.
            assert!(!text.ends_with('…'), "{id} {language}: {text}");
            assert!(text.contains(&base[language]), "{id} {language}: {text}");
            longest = longest.max(length);
        }
        // The render alone keeps its own title.
        assert_eq!(
            finished_title(&base, Contrast::None, Retouch::default()),
            base
        );
    }
    assert!(
        longest > 60,
        "a title longer than 60 characters is sent whole: {longest}"
    );
}

//! The rules of the night folder on synthetic nights:
//! invented targets (NGC 7000, IC 1396, M31), headers built
//! here, no real frame, place or name.

use super::*;
use crate::test_support::{TempDir, fits_bytes};
use crate::working_folder::{NightFolderSpec, NightTargetSpec, prepare_night_folder};
use std::fs;

const STARTED: &str = "2026-10-07T20:00:00Z";

struct Night {
    _root: TempDir,
    folder: PathBuf,
}

impl Night {
    /// A night prepared by the "Folder" step with NGC 7000, IC 1396 and
    /// M31 ticked.
    fn new(label: &str) -> Self {
        let root = TempDir::new(label);
        let spec = NightFolderSpec {
            night_id: "7f3a2c91-test".to_string(),
            service_url: "https://service.example".to_string(),
            evening_date: "2026-10-07".to_string(),
            label: "Col des Étoiles".to_string(),
            targets: ["NGC 7000", "IC 1396", "M31"]
                .iter()
                .map(|designation| NightTargetSpec {
                    target_id: id_of(designation),
                    designation: (*designation).to_string(),
                    ticked: true,
                })
                .collect(),
            follow: None,
        };
        let outcome = prepare_night_folder(root.path(), &[], &spec, STARTED).unwrap();
        Self {
            _root: root,
            folder: outcome.folder,
        }
    }

    fn write(&self, relative: &str, bytes: &[u8]) {
        let path = self.folder.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    /// A frame with its first header.
    fn frame(&self, relative: &str, cards: &[(&str, &str)]) {
        self.write(relative, &fits_bytes(cards, 64));
    }

    fn detect(&self, input: &DetectionInput) -> NightDetection {
        detect_night(&self.folder, input, &DetectOptions::default()).unwrap()
    }
}

fn id_of(designation: &str) -> String {
    format!("t-{}", normalize_object(designation))
}

fn input() -> DetectionInput {
    DetectionInput {
        evening_date: Some("2026-10-07".to_string()),
        night_started_at: Some(STARTED.to_string()),
        longitude: None,
        targets: vec![
            target("NGC 7000", &["North America Nebula"]),
            target("IC 1396", &[]),
            target("M31", &["NGC 224", "Andromeda Galaxy"]),
        ],
    }
}

fn target(designation: &str, aliases: &[&str]) -> DetectionTarget {
    DetectionTarget {
        target_id: id_of(designation),
        designation: designation.to_string(),
        aliases: aliases.iter().map(|a| (*a).to_string()).collect(),
        ticked: true,
        online_final: false,
        stacking_planned: false,
    }
}

fn light(object: &str, filter: &str, exposure: &str, date: &str) -> Vec<(String, String)> {
    vec![
        ("SIMPLE".into(), "T".into()),
        ("IMAGETYP".into(), "'Light Frame'".into()),
        ("OBJECT".into(), format!("'{object}'")),
        ("FILTER".into(), format!("'{filter}'")),
        ("EXPTIME".into(), exposure.into()),
        ("GAIN".into(), "100".into()),
        ("DATE-OBS".into(), format!("'{date}'")),
    ]
}

fn cards(owned: &[(String, String)]) -> Vec<(&str, &str)> {
    owned
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect()
}

fn finding<'a>(detection: &'a NightDetection, key: &str) -> Vec<&'a Finding> {
    detection.findings.iter().filter(|f| f.key == key).collect()
}

#[test]
fn imagetyp_is_normalised_as_the_sheet_says() {
    assert_eq!(normalize_image_type("Light Frame"), Some(FrameType::Light));
    assert_eq!(normalize_image_type("LIGHT"), Some(FrameType::Light));
    assert_eq!(normalize_image_type("Dark Frame"), Some(FrameType::Dark));
    assert_eq!(normalize_image_type("Flat Field"), None);
    assert_eq!(normalize_image_type("FLAT"), Some(FrameType::Flat));
    assert_eq!(normalize_image_type("DarkFlat"), Some(FrameType::DarkFlat));
    assert_eq!(normalize_image_type("Flat Dark"), Some(FrameType::DarkFlat));
    assert_eq!(normalize_image_type("Bias Frame"), Some(FrameType::Bias));
    assert_eq!(normalize_image_type("Offset"), Some(FrameType::Bias));
    assert_eq!(normalize_image_type("Master Dark"), None);
}

#[test]
fn object_is_compared_by_alias_without_case_or_spaces() {
    let input = input();
    assert_eq!(match_target("M 31", &input), Some(2));
    assert_eq!(match_target("ngc224", &input), Some(2));
    assert_eq!(match_target("Andromeda  galaxy", &input), Some(2));
    assert_eq!(match_target("NGC7000", &input), Some(0));
    assert_eq!(match_target("M33", &input), None);
}

#[test]
fn the_minimum_per_target_and_the_final_folder() {
    let night = Night::new("detect-final");
    night.write("NGC 7000/final/old.jpg", b"jpeg");
    std::thread::sleep(std::time::Duration::from_millis(20));
    night.write("NGC 7000/final/new.tif", b"tiff");
    night.frame("IC 1396/final/stack.fits", &[("SIMPLE", "T")]);
    night.write("IC 1396/final/notes.xcf", b"gimp");

    let mut input = input();
    input.targets[2].online_final = true;
    let detection = night.detect(&input);

    let ngc = &detection.targets[0];
    assert_eq!(ngc.minimum, MinimumSource::FinalFile);
    assert_eq!(ngc.final_image.as_deref(), Some("NGC 7000/final/new.tif"));
    assert_eq!(ngc.other_final_images, vec!["NGC 7000/final/old.jpg"]);
    assert_eq!(detection.targets[1].minimum, MinimumSource::Missing);
    assert_eq!(detection.targets[2].minimum, MinimumSource::Online);

    let missing = finding(&detection, keys::TARGET_EMPTY);
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].severity, Severity::Error);
    assert!(finding(&detection, keys::FINAL_MISSING).is_empty());
    // The two others have a final image and no FITS: a warning each.
    let no_frames = finding(&detection, keys::FRAMES_MISSING);
    assert_eq!(no_frames.len(), 2);
    assert_eq!(no_frames[0].severity, Severity::Warning);
    assert_eq!(
        no_frames[0].sentence(),
        "No FITS frames: only the final image will be sent."
    );
    assert_eq!(
        missing[0].sentence(),
        "Neither FITS frames nor a final image: put the frames in IC 1396/lights/ or a final image (TIFF, JPEG or PNG) in IC 1396/final/."
    );
    let refused = finding(&detection, keys::FINAL_FRAME_REFUSED);
    assert_eq!(refused[0].files, vec!["IC 1396/final/stack.fits"]);
    assert_eq!(
        refused[0].sentence(),
        "The service does not take FITS: export the final image as TIFF, JPEG or PNG."
    );
    assert!(
        detection
            .outside_structure
            .contains(&"IC 1396/final/notes.xcf".to_string())
    );

    // The stacking produces the final image: the minimum is met.
    input.targets[1].stacking_planned = true;
    let stacked = night.detect(&input);
    assert_eq!(stacked.targets[1].minimum, MinimumSource::Stacking);
    assert!(finding(&stacked, keys::FINAL_MISSING).is_empty());
}

#[test]
fn flats_in_lights_say_where_they_go() {
    let night = Night::new("detect-misfiled");
    let flat = [
        ("IMAGETYP", "'FLAT'"),
        ("FILTER", "'L'"),
        ("EXPTIME", "1.5"),
    ];
    for index in 0..3 {
        night.frame(&format!("M31/lights/flat-{index}.fits"), &flat);
    }
    let detection = night.detect(&input());
    let misfiled = finding(&detection, keys::FRAMES_MISFILED);
    let texts: Vec<String> = misfiled.iter().map(|f| f.sentence()).collect();
    assert!(texts.contains(
        &"3 flats filed in M31/lights/: their header says FLAT. To move into calibration/flats/.".to_string()
    ), "{texts:?}");
    let flats = misfiled
        .iter()
        .find(|f| f.params["from"] == "M31/lights/")
        .unwrap();
    assert_eq!(flats.files.len(), 3);
    assert_eq!(flats.params["kind"], "flat");
    // A misfiled frame still counts as calibration: no second message.
    assert_eq!(detection.calibration.flats, 3);
}

/// In a calibration folder the folder is right. A
/// dark in `flats/` is read as a flat, with a warning to check, never a move.
#[test]
fn a_dark_in_the_flats_folder_is_read_as_a_flat_with_a_warning() {
    let night = Night::new("detect-dark-in-flats");
    night.frame(
        "calibration/flats/dark-1.fits",
        &[
            ("IMAGETYP", "'Dark Frame'"),
            ("EXPTIME", "120"),
            ("GAIN", "100"),
        ],
    );
    let detection = night.detect(&input());
    assert!(finding(&detection, keys::FRAMES_MISFILED).is_empty());
    assert_eq!(detection.calibration.flats, 1);
    assert_eq!(detection.calibration.darks, 0);
    let differs = finding(&detection, keys::CALIBRATION_HEADER_DIFFERS);
    assert_eq!(differs.len(), 1);
    assert_eq!(differs[0].severity, Severity::Warning);
    assert_eq!(differs[0].params["kind"], "flat");
    assert_eq!(differs[0].params["header"], "DARK");
    assert_eq!(
        differs[0].sentence(),
        "1 frame from calibration/flats/ has a DARK header: it is read as a flat. Check that this is not a filing mistake."
    );
}

/// The ASIAIR's « Light » mode wrote
/// `IMAGETYP = Light` and `OBJECT = Pi Pegasi` on 20 darks of
/// `calibration/darks/`. The folder is right: they are the night's darks,
/// 90 s gain 100, matched to the lights; one warning asks to check.
#[test]
fn darks_taken_in_light_mode_are_read_as_darks_with_a_warning() {
    let night = Night::new("detect-darks-as-lights");
    let date = "2026-10-07T22:00:00";
    for index in 0..90 {
        night.frame(
            &format!("IC 1396/lights/L{index:03}.fits"),
            &cards(&light("IC 1396", "Ha", "90", date)),
        );
    }
    for index in 0..20 {
        night.frame(
            &format!("calibration/darks/Light_Pi Pegasi_90.0s_{index:04}.fits"),
            &[
                ("SIMPLE", "T"),
                ("IMAGETYP", "'Light'"),
                ("OBJECT", "'Pi Pegasi'"),
                ("EXPTIME", "90"),
                ("GAIN", "100"),
            ],
        );
    }
    let detection = night.detect(&input());
    assert_eq!(detection.calibration.darks, 20);
    assert_eq!(detection.calibration_summary.darks.exposures, vec![90.0]);
    assert_eq!(detection.calibration_summary.darks.gains, vec![100.0]);
    assert!(
        finding(&detection, keys::DARKS_MISSING).is_empty(),
        "{:?}",
        detection.sentences()
    );
    assert!(finding(&detection, keys::FRAMES_MISFILED).is_empty());
    let differs = finding(&detection, keys::CALIBRATION_HEADER_DIFFERS);
    assert_eq!(differs.len(), 1);
    assert_eq!(differs[0].severity, Severity::Warning);
    assert_eq!(differs[0].target_id, None);
    assert_eq!(differs[0].params["count"], 20);
    assert_eq!(differs[0].params["folder"], "calibration/darks/");
    assert_eq!(differs[0].params["header"], "LIGHT");
    assert_eq!(differs[0].params["kind"], "dark");
    assert_eq!(differs[0].files.len(), 20);
    assert_eq!(
        differs[0].sentence(),
        "20 frames from calibration/darks/ have a LIGHT header: they are read as darks. Check that this is not a filing mistake."
    );
}

/// A flat (header FLAT) in `darks/` is counted as a dark.
#[test]
fn a_flat_in_the_darks_folder_is_counted_as_a_dark_with_a_warning() {
    let night = Night::new("detect-flat-in-darks");
    for index in 0..2 {
        night.frame(
            &format!("calibration/darks/flat-{index}.fits"),
            &[("IMAGETYP", "'FLAT'"), ("EXPTIME", "1.5"), ("GAIN", "100")],
        );
    }
    let detection = night.detect(&input());
    assert_eq!(detection.calibration.darks, 2);
    assert_eq!(detection.calibration.flats, 0);
    assert!(finding(&detection, keys::FRAMES_MISFILED).is_empty());
    let differs = finding(&detection, keys::CALIBRATION_HEADER_DIFFERS);
    assert_eq!(differs.len(), 1);
    assert_eq!(differs[0].params["count"], 2);
    assert_eq!(differs[0].params["header"], "FLAT");
    assert_eq!(differs[0].params["kind"], "dark");
}

/// The ASIAIR writes `IMAGETYP = Dark` on its dark-flats.
#[test]
fn an_asiair_dark_flat_in_its_folder_is_not_misfiled() {
    let night = Night::new("detect-dark-flat-folder");
    let dark = [("IMAGETYP", "'Dark'"), ("EXPTIME", "180"), ("GAIN", "100")];
    let dark_flat = [("IMAGETYP", "'Dark'"), ("EXPTIME", "1.6"), ("GAIN", "100")];
    let flat = [("IMAGETYP", "'Flat'"), ("EXPTIME", "1.61"), ("GAIN", "100")];
    for index in 0..4 {
        night.frame(&format!("calibration/darks/dark-{index}.fits"), &dark);
    }
    for index in 0..3 {
        night.frame(
            &format!("calibration/dark-flats/df-{index}.fits"),
            &dark_flat,
        );
        night.frame(&format!("calibration/flats/lot-1/flat-{index}.fits"), &flat);
    }
    let detection = night.detect(&input());
    assert!(
        finding(&detection, keys::FRAMES_MISFILED).is_empty(),
        "{:?}",
        detection.sentences()
    );
    assert_eq!(detection.calibration.darks, 4);
    assert_eq!(detection.calibration.dark_flats, 3);
    assert_eq!(detection.calibration.flats, 3);
    // The dark-flats' short exposure stays out of the darks' summary.
    assert_eq!(detection.calibration_summary.darks.exposures, vec![180.0]);
}

#[test]
fn a_short_dark_stays_a_dark_in_the_darks_folder() {
    let night = Night::new("detect-short-dark-in-darks");
    night.frame(
        "calibration/darks/short.fits",
        &[("IMAGETYP", "'Dark'"), ("EXPTIME", "1.6"), ("GAIN", "100")],
    );
    night.frame(
        "calibration/flats/flat.fits",
        &[("IMAGETYP", "'Flat'"), ("EXPTIME", "1.6"), ("GAIN", "100")],
    );
    let detection = night.detect(&input());
    assert!(finding(&detection, keys::FRAMES_MISFILED).is_empty());
    assert_eq!(detection.calibration.darks, 1);
    assert_eq!(detection.calibration.dark_flats, 0);
}

#[test]
fn a_dark_as_short_as_the_flats_outside_its_folders_is_sent_to_the_dark_flats() {
    let night = Night::new("detect-dark-flat-elsewhere");
    night.frame(
        "calibration/flats/flat.fits",
        &[("IMAGETYP", "'Flat'"), ("EXPTIME", "1.6"), ("GAIN", "100")],
    );
    night.frame(
        "M31/lights/short-dark.fits",
        &[("IMAGETYP", "'Dark'"), ("EXPTIME", "1.5"), ("GAIN", "100")],
    );
    night.frame(
        "M31/lights/long-dark.fits",
        &[("IMAGETYP", "'Dark'"), ("EXPTIME", "120"), ("GAIN", "100")],
    );
    let detection = night.detect(&input());
    let misfiled = finding(&detection, keys::FRAMES_MISFILED);
    let destination = |kind: &str| {
        misfiled
            .iter()
            .find(|f| f.params["kind"] == kind)
            .map(|f| f.params["to"].as_str().unwrap().to_string())
    };
    assert_eq!(
        destination("dark-flat").as_deref(),
        Some("calibration/dark-flats/")
    );
    assert_eq!(destination("dark").as_deref(), Some("calibration/darks/"));
    assert_eq!(detection.calibration.dark_flats, 1);
    assert_eq!(detection.calibration.darks, 1);
}

#[test]
fn a_targets_frames_in_another_targets_folder_are_said() {
    let night = Night::new("detect-other-target");
    for index in 0..2 {
        night.frame(
            &format!("IC 1396/lights/l-{index}.fits"),
            &cards(&light("NGC 7000", "Ha", "300", "2026-10-07T21:00:00")),
        );
    }
    night.frame(
        "IC 1396/lights/own.fits",
        &cards(&light("IC1396", "Ha", "300", "2026-10-07T21:10:00")),
    );
    let detection = night.detect(&input());
    let other = finding(&detection, keys::FRAMES_OTHER_TARGET);
    assert_eq!(other.len(), 1);
    assert_eq!(
        other[0].sentence(),
        "2 frames of NGC 7000 in IC 1396/lights/ (OBJECT header)."
    );
    assert_eq!(detection.targets[1].light_frames, 1);
}

#[test]
fn flats_and_darks_missing_are_warnings_per_filter_and_setting() {
    let night = Night::new("detect-calibration");
    night.frame(
        "IC 1396/lights/ha.fits",
        &cards(&light("IC 1396", "Ha", "300", "2026-10-07T21:00:00")),
    );
    night.frame(
        "IC 1396/lights/oiii.fits",
        &cards(&light("IC 1396", "OIII", "300", "2026-10-07T21:30:00")),
    );
    night.frame(
        "M31/lights/l.fits",
        &cards(&light("M 31", "L", "120", "2026-10-07T22:00:00")),
    );
    // Flats of Ha by header, flats of L by their subfolder only.
    night.frame(
        "calibration/flats/f-ha.fits",
        &[("IMAGETYP", "'FLAT'"), ("FILTER", "'Ha'")],
    );
    night.frame("calibration/flats/L/f-l.fits", &[("IMAGETYP", "'FLAT'")]);
    night.frame(
        "calibration/darks/d-300.fits",
        &[("IMAGETYP", "'DARK'"), ("EXPTIME", "300."), ("GAIN", "100")],
    );
    let detection = night.detect(&input());

    let flats: Vec<String> = finding(&detection, keys::FLATS_MISSING)
        .iter()
        .map(|f| f.sentence())
        .collect();
    assert_eq!(flats, vec!["No flat for the OIII filter (IC 1396)."]);
    let darks: Vec<String> = finding(&detection, keys::DARKS_MISSING)
        .iter()
        .map(|f| f.sentence())
        .collect();
    assert_eq!(
        darks,
        vec!["No dark of 120 s · gain 100 for M31: the dark in the folder is 300 s."]
    );
    assert!(
        detection
            .findings
            .iter()
            .filter(|f| f.key != keys::TARGET_EMPTY)
            .all(|f| matches!(f.severity, Severity::Info | Severity::Warning)),
        "only a target with nothing to send is an error"
    );
}

#[test]
fn logs_are_recognised_by_name_and_contents() {
    let night = Night::new("detect-logs");
    let detection = night.detect(&input());
    let none = finding(&detection, keys::NO_LOGS);
    assert_eq!(none.len(), 1);
    assert_eq!(none[0].severity, Severity::Info);
    assert_eq!(none[0].sentence(), "No log: no steps, no guiding quality.");

    night.write("logs/notes.txt", b"cloudy after 2 am");
    night.write(
        "logs/20261007-200000-3.0.0.9001.log",
        b"2026-10-07T20:00:01.1234|INFO|SequenceItem.cs|Run|210|Starting Category: Camera\n",
    );
    night.write(
        "logs/PHD2_GuideLog_2026-10-07_200000.txt",
        b"PHD2 version 2.6.13",
    );
    night.write(
        "logs/Autorun_Log_2026-10-07_200000.txt",
        b"2026/10/07 20:00:00 [AutoRun|Begin] invented\n",
    );
    let detection = night.detect(&input());
    assert!(finding(&detection, keys::NO_LOGS).is_empty());
    let kinds: Vec<(String, LogKind)> = detection
        .logs
        .iter()
        .map(|l| (l.relative_path.clone(), l.kind))
        .collect();
    assert_eq!(
        kinds,
        vec![
            (
                "logs/20261007-200000-3.0.0.9001.log".to_string(),
                LogKind::Nina
            ),
            (
                "logs/Autorun_Log_2026-10-07_200000.txt".to_string(),
                LogKind::Asiair
            ),
            (
                "logs/PHD2_GuideLog_2026-10-07_200000.txt".to_string(),
                LogKind::Phd2
            ),
            ("logs/notes.txt".to_string(), LogKind::Unknown),
        ]
    );
}

/// The `_CHN` file of the ASIAIR is not a journal to read.
#[test]
fn the_chinese_rendition_of_an_asiair_log_is_not_a_log_to_read() {
    let night = Night::new("detect-logs-chn");
    night.write(
        "logs/Autorun_Log_2026-10-07_221116.txt",
        b"Log enabled at 2026/10/07 22:11:16\n2026/10/07 22:11:16 [Autorun|Begin] Start\n",
    );
    night.write(
        "logs/Autorun_Log_2026-10-07_221116_CHN.txt",
        "日志开始于 2026/10/07 22:11:16\n2026/10/07 22:11:16 [Autorun|Begin] 开始\n".as_bytes(),
    );
    night.write(
        "logs/copie.txt",
        "日志开始于 2026/10/07 22:11:16\n2026/10/07 22:11:16 [Autorun|Begin] 开始\n".as_bytes(),
    );
    let detection = night.detect(&input());
    let kinds: Vec<(&str, LogKind)> = detection
        .logs
        .iter()
        .map(|log| (log.relative_path.as_str(), log.kind))
        .collect();
    assert!(kinds.contains(&("logs/Autorun_Log_2026-10-07_221116.txt", LogKind::Asiair)));
    assert!(kinds.contains(&(
        "logs/Autorun_Log_2026-10-07_221116_CHN.txt",
        LogKind::Unknown
    )));
    assert!(kinds.contains(&("logs/copie.txt", LogKind::Unknown)));
}

#[test]
fn each_log_found_carries_its_size_for_the_logs_tab() {
    let night = Night::new("detect-log-sizes");
    night.write(
        "logs/PHD2_GuideLog_2026-10-07_200000.txt",
        b"PHD2 version 2.6.13",
    );
    night.write("logs/notes.txt", b"cloudy");
    let detection = night.detect(&input());
    let sizes: Vec<(String, u64)> = detection
        .logs
        .iter()
        .map(|l| (l.relative_path.clone(), l.bytes))
        .collect();
    assert_eq!(
        sizes,
        vec![
            ("logs/PHD2_GuideLog_2026-10-07_200000.txt".to_string(), 19),
            ("logs/notes.txt".to_string(), 6),
        ]
    );
    let json = serde_json::to_value(&detection.logs[0]).unwrap();
    assert_eq!(json["bytes"], 19);
}

#[test]
fn lights_of_another_night_are_said_with_their_evening() {
    let night = Night::new("detect-other-night");
    for index in 0..2 {
        night.frame(
            &format!("NGC 7000/lights/old-{index}.fits"),
            &cards(&light("NGC 7000", "Ha", "300", "2026-10-05T22:00:00")),
        );
    }
    night.frame(
        "NGC 7000/lights/tonight.fits",
        &cards(&light("NGC 7000", "Ha", "300", "2026-10-08T03:00:00")),
    );
    let detection = night.detect(&input());
    let other = finding(&detection, keys::FRAMES_OTHER_NIGHT);
    assert_eq!(other.len(), 1);
    assert_eq!(other[0].params["date"], "2026-10-05");
    assert_eq!(
        other[0].sentence(),
        "2 frames outside the night (night of 5 Oct), left out of the integration; they stay counted in NGC 7000/lights/."
    );
    assert_eq!(other[0].severity, Severity::Warning);
    // Said, never dropped from the folder: the two old frames stay lights of
    // the target (the stacking leaves them out of the integration).
    assert_eq!(detection.targets[0].light_frames, 3);
}

/// A night of `evening` at `longitude` (no start: the start typed is
/// never read when the evening is known).
fn night_input(evening: &str, longitude: Option<f64>) -> DetectionInput {
    DetectionInput {
        evening_date: Some(evening.to_string()),
        night_started_at: None,
        longitude,
        targets: vec![target("NGC 7000", &[])],
    }
}

fn other_night_dates(
    night: &Night,
    input: &DetectionInput,
    dates: &[&str],
) -> (Vec<String>, usize) {
    for (index, date) in dates.iter().enumerate() {
        night.frame(
            &format!("NGC 7000/lights/f-{index}.fits"),
            &cards(&light("NGC 7000", "Ha", "300", date)),
        );
    }
    let detection = night.detect(input);
    let found = finding(&detection, keys::FRAMES_OTHER_NIGHT)
        .iter()
        .map(|f| f.params["date"].as_str().unwrap().to_string())
        .collect();
    (found, detection.targets[0].light_frames)
}

#[test]
fn the_window_runs_from_local_noon_to_local_noon() {
    // 2° E: local noon is 11:52 UTC (sheet § 2.2).
    let window = NightWindow::of_evening("2026-08-10", Some(2.0)).unwrap();
    let at = |iso: &str| crate::time_util::parse_iso8601_utc_to_unix_seconds(iso).unwrap();
    assert!(!window.contains(at("2026-08-10T11:51:59Z")));
    assert!(window.contains(at("2026-08-10T11:52:00Z")));
    assert!(window.contains(at("2026-08-11T03:01:00Z")));
    assert!(window.contains(at("2026-08-11T11:51:59Z")));
    assert!(!window.contains(at("2026-08-11T11:52:00Z")));
    assert_eq!(window.evening_of(at("2026-08-11T20:15:00Z")), "2026-08-11");
    // Without a longitude: UTC.
    let utc = NightWindow::of_evening("2026-08-10", None).unwrap();
    assert!(utc.contains(at("2026-08-10T12:00:00Z")));
    assert!(!utc.contains(at("2026-08-11T12:00:00Z")));
    // The start is read only when the evening is unknown.
    let from_start = NightWindow::of_input(&DetectionInput {
        evening_date: None,
        night_started_at: Some("2026-08-11T01:00:00Z".to_string()),
        longitude: Some(10.0),
        targets: Vec::new(),
    })
    .unwrap();
    assert_eq!(
        from_start,
        NightWindow::of_evening("2026-08-10", Some(10.0)).unwrap()
    );
}

#[test]
fn a_night_crossing_utc_midnight_stays_one_night() {
    // 21:48 -> 03:41 UTC: the frames of the 11th are the 10th's night.
    let night = Night::new("detect-utc-midnight");
    let (other, lights) = other_night_dates(
        &night,
        &night_input("2026-08-10", None),
        &[
            "2026-08-10T21:50:00",
            "2026-08-10T23:59:30",
            "2026-08-11T00:00:10",
            "2026-08-11T03:41:00",
        ],
    );
    assert!(other.is_empty(), "{other:?}");
    assert_eq!(lights, 4);
}

#[test]
fn a_night_crossing_local_midnight_only_stays_one_night() {
    // A site at 135° E: 20:00 -> 04:00 local is 11:00 -> 19:00 UTC, the same
    // UTC day; local noon is 03:00 UTC.
    let night = Night::new("detect-local-midnight");
    let (other, lights) = other_night_dates(
        &night,
        &night_input("2026-08-10", Some(135.0)),
        &[
            "2026-08-10T11:05:00",
            "2026-08-10T14:59:59",
            "2026-08-10T19:00:00",
        ],
    );
    assert!(other.is_empty(), "{other:?}");
    assert_eq!(lights, 3);
}

#[test]
fn a_frame_of_the_dusk_belongs_to_the_night() {
    let night = Night::new("detect-dusk");
    let (other, lights) = other_night_dates(
        &night,
        &night_input("2026-08-10", Some(2.0)),
        &["2026-08-10T19:30:00", "2026-08-10T21:48:00"],
    );
    assert!(other.is_empty(), "{other:?}");
    assert_eq!(lights, 2);
}

#[test]
fn a_real_other_night_is_said_with_its_own_evening() {
    let night = Night::new("detect-real-other");
    let (other, lights) = other_night_dates(
        &night,
        &night_input("2026-08-10", Some(2.0)),
        &[
            "2026-08-09T23:00:00", // the evening before
            "2026-08-10T22:30:00", // tonight
            "2026-08-11T22:30:00", // the evening after
            "2026-08-12T01:00:00", // the night after, past midnight
        ],
    );
    assert_eq!(other, vec!["2026-08-09", "2026-08-11"]);
    // Flagged frames stay lights of the folder: 1 in the window + 3 flagged.
    assert_eq!(lights, 4);
}

#[test]
fn the_heart_night_is_whole_and_a_frame_of_the_evening_before_is_set_aside() {
    // The night of the 10th August, its frames from 00:00 to 03:01 UTC on the
    // 11th (a site at 10° E: local noon 11:20 UTC), whatever start was typed;
    // one frame of the evening before, filed there by mistake, is said and
    // left out of the integration.
    let night = Night::new("detect-heart");
    let mut dates: Vec<String> = (0..90)
        .map(|index| {
            let seconds = index * 121;
            format!(
                "2026-08-11T{:02}:{:02}:{:02}",
                seconds / 3600,
                seconds % 3600 / 60,
                seconds % 60
            )
        })
        .collect();
    dates.push("2026-08-09T22:00:00".to_string());
    let dates: Vec<&str> = dates.iter().map(String::as_str).collect();
    let mut input = night_input("2026-08-10", Some(10.0));
    input.night_started_at = Some("2026-08-10T02:00:00Z".to_string());
    let (other, lights) = other_night_dates(&night, &input, &dates);
    assert_eq!(other, vec!["2026-08-09"]);
    assert_eq!(lights, 91);
    let detection = night.detect(&input);
    let found = finding(&detection, keys::FRAMES_OTHER_NIGHT);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Warning);
    assert_eq!(found[0].params["count"], 1);
    assert_eq!(
        found[0].files,
        vec!["NGC 7000/lights/f-90.fits".to_string()]
    );
    assert_eq!(
        found[0].sentence(),
        "1 frame outside the night (night of 9 Aug), left out of the integration; it stays counted in NGC 7000/lights/."
    );
    assert!(
        detection
            .findings
            .iter()
            .all(|f| f.severity != Severity::Error),
        "{:?}",
        detection.findings
    );
}

#[test]
fn frames_without_a_final_image_are_a_warning_not_an_error() {
    let night = Night::new("detect-frames-only");
    night.frame(
        "NGC 7000/lights/a.fits",
        &cards(&light("NGC 7000", "Ha", "300", "2026-10-07T21:00:00")),
    );
    let detection = night.detect(&input());
    let missing = finding(&detection, keys::FINAL_MISSING);
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].severity, Severity::Warning);
    assert_eq!(missing[0].target_id.as_deref(), Some("t-ngc7000"));
    assert_eq!(
        missing[0].sentence(),
        "No final image: only the FITS frames will be used to create one (stacking)."
    );
    // Ticking the stacking for it says nothing more: the frames make it.
    let mut stacked = input();
    stacked.targets[0].stacking_planned = true;
    let detection = night.detect(&stacked);
    assert!(finding(&detection, keys::FINAL_MISSING).is_empty());
    assert!(
        finding(&detection, keys::TARGET_EMPTY)
            .iter()
            .all(|f| f.target_id.as_deref() != Some("t-ngc7000"))
    );
}

#[test]
fn files_outside_the_structure_are_counted() {
    let night = Night::new("detect-outside");
    night.write("readme.txt", b"x");
    night.write("M31/preview.jpg", b"x");
    night.write("Desktop stuff/a.fits", b"x");
    night.write("calibration/stray.fits", b"x");
    night.write("M31/lights/thumb.png", b"x");
    // Astro Sync's own results are never "outside the rules".
    night.write("M31/stacked/M31_2026-10-07_raw.tif", b"x");
    night.write("M31/progression/milestone_01.jpg", b"x");
    let detection = night.detect(&input());
    let outside = finding(&detection, keys::FILES_OUTSIDE_STRUCTURE);
    assert_eq!(outside[0].files.len(), 5);
    assert!(
        outside[0]
            .files
            .iter()
            .all(|f| !f.contains("/stacked/") && !f.contains("/progression/"))
    );
    assert_eq!(
        outside[0].sentence(),
        "5 files outside the structure: see where to file them."
    );
}

#[test]
fn xisf_frames_are_read_like_fits() {
    let night = Night::new("detect-xisf");
    let xml = r#"<xisf version="1.0"><Image>
<FITSKeyword name="IMAGETYP" value="'Flat Frame'" comment=""/>
<FITSKeyword name="FILTER" value="'OIII'" comment=""/>
</Image></xisf>"#;
    night.write(
        "IC 1396/lights/flat.xisf",
        &crate::xisf::build_xisf_bytes(xml),
    );
    let detection = night.detect(&input());
    let misfiled = finding(&detection, keys::FRAMES_MISFILED);
    assert_eq!(misfiled.len(), 1);
    assert_eq!(misfiled[0].files, vec!["IC 1396/lights/flat.xisf"]);
    assert_eq!(detection.calibration.flats, 1);
}

#[cfg(unix)]
#[test]
fn linked_lights_are_read_and_a_missing_disk_is_said() {
    let night = Night::new("detect-links");
    let disk = TempDir::new("detect-links-disk");
    fs::write(
        disk.path().join("Light_0001.fits"),
        fits_bytes(&cards(&light("M31", "L", "120", "2026-10-07T22:00:00")), 64),
    )
    .unwrap();
    fs::remove_dir(night.folder.join("M31/lights")).unwrap();
    std::os::unix::fs::symlink(disk.path(), night.folder.join("M31/lights")).unwrap();
    std::os::unix::fs::symlink(
        night.folder.join("missing-disk/PHD2"),
        night.folder.join("logs/phd2"),
    )
    .unwrap();
    std::os::unix::fs::symlink(&night.folder, night.folder.join("IC 1396/lights/loop")).unwrap();

    let detection = night.detect(&input());
    assert_eq!(detection.targets[2].light_frames, 1);
    let missing = finding(&detection, keys::LINK_TARGET_MISSING);
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].sentence(), "Link to a missing disk: logs/phd2.");
    assert_eq!(finding(&detection, keys::LINK_LOOP).len(), 1);
}

#[test]
fn a_frame_without_imagetyp_takes_its_folders_type() {
    let night = Night::new("detect-no-imagetyp");
    night.frame(
        "M31/lights/l.fits",
        &[("OBJECT", "'M31'"), ("EXPTIME", "120"), ("FILTER", "'L'")],
    );
    night.frame("calibration/darks/d.fits", &[("EXPTIME", "120")]);
    let detection = night.detect(&input());
    assert_eq!(detection.targets[2].light_frames, 1);
    assert_eq!(detection.calibration.darks, 1);
    assert!(finding(&detection, keys::FRAMES_MISFILED).is_empty());
    assert!(finding(&detection, keys::DARKS_MISSING).is_empty());
}

#[test]
fn an_unticked_targets_folder_is_out_of_the_night_and_left_alone() {
    let night = Night::new("detect-out-of-night");
    night.write("M31/lights/notes.txt", b"x");
    let mut input = input();
    input.targets[2].ticked = false;
    let detection = night.detect(&input);
    let m31 = detection
        .targets
        .iter()
        .find(|t| t.designation == "M31")
        .unwrap();
    assert!(!m31.in_night);
    assert!(detection.outside_structure.is_empty());
    assert!(
        detection
            .findings
            .iter()
            .all(|f| f.target_id.as_deref() != Some(m31.target_id.as_str()))
    );
}

#[test]
fn the_fingerprint_changes_with_the_folder_and_only_then() {
    let night = Night::new("detect-fingerprint");
    let first = night_folder_fingerprint(&night.folder).unwrap();
    assert_eq!(night_folder_fingerprint(&night.folder).unwrap(), first);
    assert_eq!(night.detect(&input()).fingerprint, first);
    night.write("logs/new.txt", b"x");
    assert_ne!(night_folder_fingerprint(&night.folder).unwrap(), first);
}

#[test]
fn the_detection_stops_on_the_switch() {
    let night = Night::new("detect-cancel");
    night.frame("M31/lights/l.fits", &[("IMAGETYP", "'LIGHT'")]);
    let cancel = CancelFlag::new();
    cancel.cancel();
    let error = detect_night(
        &night.folder,
        &input(),
        &DetectOptions {
            cancel: Some(&cancel),
            progress: None,
        },
    )
    .unwrap_err();
    assert_eq!(error.code(), "cancelled");
}

#[test]
fn files_says_the_frames_per_filter_the_exposures_gains_and_sizes() {
    let night = Night::new("frame-summary");
    let date = "2026-10-07T22:00:00";
    for (n, filter) in [(1, "Ha"), (2, "Ha"), (3, "OIII")] {
        night.frame(
            &format!("NGC 7000/lights/L{n}.fits"),
            &cards(&light("NGC 7000", filter, "180", date)),
        );
    }
    night.frame(
        "NGC 7000/lights/L4.fits",
        &cards(&light("NGC 7000", "OIII", "120", date)),
    );
    night.write("NGC 7000/final/NGC7000_final.tif", &[0u8; 300]);
    for (n, exposure) in [(1, "180"), (2, "120")] {
        night.frame(
            &format!("calibration/darks/D{n}.fits"),
            &[
                ("SIMPLE", "T"),
                ("IMAGETYP", "'Dark Frame'"),
                ("EXPTIME", exposure),
                ("GAIN", "100"),
            ],
        );
    }
    night.frame(
        "calibration/flats/Ha/F1.fits",
        &[("SIMPLE", "T"), ("IMAGETYP", "'Flat Frame'")],
    );
    let detection = night.detect(&input());
    let ngc = &detection.targets[0];
    assert_eq!(ngc.final_bytes, Some(300));
    let summary = &ngc.light_summary;
    assert_eq!(
        summary.filters,
        vec![
            FilterCount {
                filter: Some("Ha".into()),
                frames: 2
            },
            FilterCount {
                filter: Some("OIII".into()),
                frames: 2
            },
        ]
    );
    assert_eq!(summary.exposures, vec![180.0, 120.0]);
    assert_eq!(summary.gains, vec![100.0]);
    assert!(summary.bytes > 0);
    let calibration = &detection.calibration_summary;
    assert_eq!(calibration.darks.exposures, vec![180.0, 120.0]);
    assert_eq!(
        calibration.darks.filters,
        vec![FilterCount {
            filter: None,
            frames: 2
        }]
    );
    // A flat without FILTER counts for its filter's subfolder.
    assert_eq!(
        calibration.flats.filters,
        vec![FilterCount {
            filter: Some("Ha".into()),
            frames: 1
        }]
    );
}

/* ---------------------- The image a device stacked ---------------------- */

/// The cards of a Seestar's `Stacked_*.fit` (a colour cube of 16 bits).
fn seestar_stack(poses: u32, exposure: &str) -> Vec<(String, String)> {
    [
        ("SIMPLE", "T".to_string()),
        ("BITPIX", "16".to_string()),
        ("NAXIS", "3".to_string()),
        ("NAXIS1", "2160".to_string()),
        ("NAXIS2", "3840".to_string()),
        ("NAXIS3", "3".to_string()),
        ("BZERO", "32768".to_string()),
        ("CREATOR", "'ZWO Seestar S50 Pro'".to_string()),
        ("STACKCNT", poses.to_string()),
        ("EXPTIME", exposure.to_string()),
        (
            "TOTALEXP",
            format!("{}", f64::from(poses) * exposure.parse::<f64>().unwrap()),
        ),
        ("FILTER", "'IRCUT'".to_string()),
        ("GAIN", "200".to_string()),
        ("OBJECT", "'M31'".to_string()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

/// A Seestar sub-frame (30 s unless said) of M31.
fn seestar_light(exposure: &str, second: u32) -> Vec<(String, String)> {
    let mut cards = light(
        "M31",
        "IRCUT",
        exposure,
        &format!("2026-10-07T21:00:{second:02}Z"),
    );
    cards.retain(|(key, _)| key != "GAIN");
    cards.push(("GAIN".into(), "200".into()));
    cards
}

#[test]
fn a_device_image_is_found_in_its_folder_and_covers_the_family_of_the_poses() {
    let night = Night::new("detect-device");
    for second in 0..3 {
        night.frame(
            &format!("M31/lights/IRCUT/Light_{second}.fit"),
            &cards(&seestar_light("30", second)),
        );
    }
    night.frame(
        "M31/device/Stacked_3_M31_30.0s_IRCUT.fit",
        &cards(&seestar_stack(3, "30")),
    );
    let detection = night.detect(&input());
    let m31 = &detection.targets[2];
    let device = m31.device.as_ref().expect("a device image is found");
    assert_eq!(device.images.len(), 1);
    let image = &device.images[0];
    assert_eq!(
        image.relative_path,
        "M31/device/Stacked_3_M31_30.0s_IRCUT.fit"
    );
    assert_eq!(image.info.poses, 3);
    assert_eq!(image.info.device, "Seestar S50 Pro");
    assert_eq!(device.families.len(), 1);
    let family = &device.families[0];
    assert_eq!(family.label, "IRCUT_30s_g200");
    assert_eq!(family.coverage, crate::device_master::Coverage::Covers);
    assert_eq!(family.default_source, crate::device_master::Source::Device);
    assert_eq!(family.lights, 3);
    // The poses are counted as they were, the image is not one of them, and
    // nothing of `device/` is "outside the structure".
    assert_eq!(m31.light_frames, 3);
    assert!(detection.outside_structure.is_empty());
    assert!(detection.targets[0].device.is_none());
}

#[test]
fn a_family_cut_in_two_images_is_split_and_restacked_when_its_poses_are_here() {
    let night = Night::new("detect-device-split");
    // 10 s poses here, 30 s poses not: two images of 10 s, one of 30 s.
    for second in 0..3 {
        night.frame(
            &format!("M31/lights/IRCUT/Light_{second}.fit"),
            &cards(&seestar_light("10", second)),
        );
    }
    night.frame("M31/device/Stacked_2.fit", &cards(&seestar_stack(2, "10")));
    night.frame("M31/device/Stacked_1x.fit", &cards(&seestar_stack(2, "10")));
    night.frame(
        "M31/device/Stacked_30.fit",
        &cards(&seestar_stack(30, "30")),
    );
    let detection = night.detect(&input());
    let device = detection.targets[2].device.as_ref().unwrap();
    assert_eq!(device.images.len(), 3);
    let by_label = |label: &str| {
        device
            .families
            .iter()
            .find(|f| f.label == label)
            .unwrap_or_else(|| panic!("no family {label}"))
    };
    let ten = by_label("IRCUT_10s_g200");
    assert_eq!(ten.coverage, crate::device_master::Coverage::Split);
    assert_eq!(ten.default_source, crate::device_master::Source::Restack);
    assert_eq!(ten.use_file, None);
    assert_eq!(ten.lights, 3);
    // The 30 s image has no poses here: it is the family's only source.
    let thirty = by_label("IRCUT_30s_g200");
    assert_eq!(thirty.coverage, crate::device_master::Coverage::Covers);
    assert_eq!(thirty.default_source, crate::device_master::Source::Device);
    assert_eq!(thirty.lights, 0);
    assert!(!thirty.restackable());
}

#[test]
fn a_target_with_only_the_device_image_has_something_to_stack() {
    let night = Night::new("detect-device-only");
    night.frame("M31/device/Stacked_8.fit", &cards(&seestar_stack(8, "10")));
    let detection = night.detect(&input());
    let m31 = &detection.targets[2];
    assert_eq!(m31.light_frames, 0);
    assert!(m31.device.is_some());
    // Not "neither a frame nor a final image": the image stands for the poses.
    assert!(
        finding(&detection, keys::TARGET_EMPTY)
            .iter()
            .all(|f| { f.target_id.as_deref() != Some(m31.target_id.as_str()) })
    );
}

/// With the image of a Seestar covering a family, the warning
/// does not promise a stacking of the FITS frames alone -- it says the device's
/// image is the source and how many other poses are stacked.
#[test]
fn a_device_image_makes_the_missing_final_image_say_it_is_the_source() {
    let night = Night::new("detect-device-final-missing");
    // 30 s poses covered by the Seestar's image, 10 s poses cut in two images
    // (restacked from the poses here).
    for second in 0..3 {
        night.frame(
            &format!("M31/lights/IRCUT/Light_30_{second}.fit"),
            &cards(&seestar_light("30", second)),
        );
    }
    for second in 10..12 {
        night.frame(
            &format!("M31/lights/IRCUT/Light_10_{second}.fit"),
            &cards(&seestar_light("10", second)),
        );
    }
    night.frame(
        "M31/device/Stacked_3_M31_30.0s_IRCUT.fit",
        &cards(&seestar_stack(3, "30")),
    );
    night.frame("M31/device/Stacked_a.fit", &cards(&seestar_stack(1, "10")));
    night.frame("M31/device/Stacked_b.fit", &cards(&seestar_stack(1, "10")));
    let detection = night.detect(&input());
    let m31 = detection.targets[2].target_id.clone();
    assert!(
        finding(&detection, keys::FINAL_MISSING)
            .iter()
            .all(|f| f.target_id.as_deref() != Some(m31.as_str())),
        "« only the FITS frames will be used » is false here"
    );
    let from_device = finding(&detection, keys::FINAL_FROM_DEVICE);
    assert_eq!(from_device.len(), 1);
    assert_eq!(from_device[0].target_id.as_deref(), Some(m31.as_str()));
    assert_eq!(from_device[0].params["restackFrames"], 2);
    assert!(
        from_device[0].sentence().contains("the device’s image")
            && from_device[0]
                .sentence()
                .contains("the 2 other FITS frames")
            && !from_device[0].sentence().contains("only the FITS frames"),
        "{}",
        from_device[0].sentence()
    );

    // Everything covered by the device's image: nothing else to stack.
    let night = Night::new("detect-device-final-covered");
    for second in 0..3 {
        night.frame(
            &format!("M31/lights/IRCUT/Light_{second}.fit"),
            &cards(&seestar_light("30", second)),
        );
    }
    night.frame(
        "M31/device/Stacked_3_M31_30.0s_IRCUT.fit",
        &cards(&seestar_stack(3, "30")),
    );
    let detection = night.detect(&input());
    let covered = finding(&detection, keys::FINAL_FROM_DEVICE);
    assert_eq!(covered.len(), 1);
    assert_eq!(covered[0].params["restackFrames"], 0);
    assert_eq!(
        covered[0].sentence(),
        "No final image: the device’s image will be used as the source."
    );
}

#[test]
fn the_jpg_of_the_device_is_never_a_master_and_a_thumbnail_is_not_noise() {
    let night = Night::new("detect-device-jpg");
    night.frame("M31/device/Stacked_8.fit", &cards(&seestar_stack(8, "10")));
    // Beside its own .fit, and its thumbnail: nothing to say.
    night.write("M31/device/Stacked_8.jpg", b"jpeg");
    night.write("M31/device/Stacked_8_thn.jpg", b"jpeg");
    // Alone: said.
    night.write("M31/device/Stacked_9.jpg", b"jpeg");
    let detection = night.detect(&input());
    let device = detection.targets[2].device.as_ref().unwrap();
    assert_eq!(device.images.len(), 1);
    assert_eq!(device.jpgs, vec!["M31/device/Stacked_9.jpg"]);
    assert!(detection.outside_structure.is_empty());
}

#[test]
fn only_the_jpg_is_left_when_the_fit_was_not_copied() {
    let night = Night::new("detect-device-jpg-alone");
    night.write("M31/device/Stacked_430.jpg", b"jpeg");
    let detection = night.detect(&input());
    let device = detection.targets[2].device.as_ref().unwrap();
    assert!(device.images.is_empty());
    assert!(device.families.is_empty());
    assert_eq!(device.jpgs, vec!["M31/device/Stacked_430.jpg"]);
    // Nothing to stack: the target is still empty.
    assert!(
        finding(&detection, keys::TARGET_EMPTY)
            .iter()
            .any(|f| f.target_id.as_deref() == Some(detection.targets[2].target_id.as_str()))
    );
}

#[test]
fn a_stack_of_another_writer_in_device_is_not_a_device_image() {
    let night = Night::new("detect-device-asiair");
    // A Live stack of the ASIAIR: named `Stacked…`, counted, no known device.
    night.frame(
        "M31/device/Stacked_40.fit",
        &[
            ("SIMPLE", "T"),
            ("BITPIX", "16"),
            ("NAXIS", "2"),
            ("STACKCNT", "40"),
            ("CREATOR", "'ZWO ASIAIR'"),
        ],
    );
    let detection = night.detect(&input());
    assert!(detection.targets[2].device.is_none());
    assert!(
        detection
            .outside_structure
            .contains(&"M31/device/Stacked_40.fit".to_string())
    );
}

/* ------------------------- Camera RAW files ------------------- */

use crate::raw_header::fixture::{Shot, write_raw};

impl Night {
    /// A camera RAW file (a Nikon's, 30 s at ISO 800 unless `shot` says
    /// otherwise).
    fn raw(&self, relative: &str, shot: &Shot) {
        write_raw(&self.folder.join(relative), shot);
    }
}

fn nikon(taken: &'static str) -> Shot {
    Shot::new(taken, 30, 800)
}

#[test]
fn a_night_of_camera_raw_files_is_read_like_a_night_of_fits() {
    let night = Night::new("detect-raw-night");
    for index in 0..3 {
        night.raw(
            &format!("M31/lights/DSC_{index:04}.NEF"),
            &nikon("2026:10:07 23:15:42"),
        );
    }
    for index in 0..2 {
        night.raw(
            &format!("calibration/darks/dark-{index}.NEF"),
            &nikon("2026:10:08 05:10:00"),
        );
    }
    let detection = night.detect(&input());
    let m31 = &detection.targets[2];
    assert_eq!(m31.light_frames, 3);
    let summary = &m31.light_summary;
    assert_eq!(summary.exposures, vec![30.0]);
    // The ISO is the gain.
    assert_eq!(summary.gains, vec![800.0]);
    assert_eq!(detection.calibration.darks, 2);
    // The darks fit the exposure and the ISO of the lights: nothing is missing.
    assert!(finding(&detection, keys::DARKS_MISSING).is_empty());
    // Nothing is « outside the structure ».
    assert!(detection.outside_structure.is_empty());
}

#[test]
fn raw_darks_of_another_iso_are_not_the_darks_of_the_lights() {
    let night = Night::new("detect-raw-iso");
    night.raw("M31/lights/a.NEF", &nikon("2026:10:07 23:15:42"));
    night.raw("M31/lights/b.NEF", &nikon("2026:10:07 23:16:42"));
    night.raw(
        "calibration/darks/d.NEF",
        &Shot::new("2026:10:08 05:10:00", 30, 1600),
    );
    let detection = night.detect(&input());
    assert_eq!(finding(&detection, keys::DARKS_MISSING).len(), 1);
}

#[test]
fn a_raw_dark_in_the_lights_is_misfiled_by_the_word_of_its_name() {
    let night = Night::new("detect-raw-misfiled");
    night.raw("M31/lights/Light_0001.NEF", &nikon("2026:10:07 23:15:42"));
    night.raw("M31/lights/Dark_0001.NEF", &nikon("2026:10:07 23:20:42"));
    night.raw("M31/lights/Dark_0002.NEF", &nikon("2026:10:07 23:21:42"));
    let detection = night.detect(&input());
    assert_eq!(detection.targets[2].light_frames, 1);
    let misfiled = finding(&detection, keys::FRAMES_MISFILED);
    assert_eq!(misfiled.len(), 1);
    assert_eq!(misfiled[0].params["count"], 2);
    assert_eq!(misfiled[0].params["kind"], "dark");
}

#[test]
fn a_camera_clock_without_a_zone_is_compared_with_the_wall_clock_of_the_night() {
    // 139.7° E: local noon is 02:41 UTC. The frames of a night shot from
    // 23:30 to 03:00 on the camera's clock are the night's; read as UTC, the
    // 03:00 one (next day) would be past the window's end and be dropped.
    let night = Night::new("detect-raw-wall-clock");
    let input = night_input("2026-10-07", Some(139.7));
    night.raw("NGC 7000/lights/a.NEF", &nikon("2026:10:07 23:30:00"));
    night.raw("NGC 7000/lights/b.NEF", &nikon("2026:10:08 03:00:00"));
    // After the next local noon: another night, said with its own evening.
    night.raw("NGC 7000/lights/c.NEF", &nikon("2026:10:08 21:00:00"));
    let detection = night.detect(&input);
    let other = finding(&detection, keys::FRAMES_OTHER_NIGHT);
    assert_eq!(other.len(), 1);
    assert_eq!(other[0].params["date"], "2026-10-08");
    assert_eq!(other[0].files, vec!["NGC 7000/lights/c.NEF".to_string()]);
}

#[test]
fn a_camera_clock_with_its_offset_is_compared_in_utc() {
    let night = Night::new("detect-raw-offset");
    let input = night_input("2026-10-07", Some(139.7));
    let at = |name: &str, taken: &'static str| {
        let mut shot = nikon(taken);
        shot.offset = Some("+09:00");
        night.raw(&format!("NGC 7000/lights/{name}.NEF"), &shot);
    };
    at("a", "2026:10:07 23:30:00");
    at("b", "2026:10:08 03:00:00");
    at("c", "2026:10:08 21:00:00");
    let detection = night.detect(&input);
    let other = finding(&detection, keys::FRAMES_OTHER_NIGHT);
    assert_eq!(other.len(), 1);
    assert_eq!(other[0].files, vec!["NGC 7000/lights/c.NEF".to_string()]);
}

#[test]
fn a_raw_file_the_reader_does_not_know_is_a_light_without_a_header() {
    // Not a TIFF, not a CR3, not a RAF: the same as a FITS file whose header
    // cannot be read (counted, no figure).
    let night = Night::new("detect-raw-unknown");
    night.write("M31/lights/odd.NEF", b"not a raw file");
    night.raw("M31/lights/ok.NEF", &nikon("2026:10:07 23:15:42"));
    let detection = night.detect(&input());
    assert_eq!(detection.targets[2].light_frames, 2);
    let summary = &detection.targets[2].light_summary;
    assert_eq!(summary.exposures, vec![30.0]);
}

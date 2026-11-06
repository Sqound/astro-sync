//! A night's logs and frame headers become the report v2 the service
//! accepts, and its sending replaces the analysis in place, never with an
//! empty one. Invented logs (`tests/fixtures/nina_*`,
//! `phd2_guide_log_short.txt`) and headers written here; no real frame, place
//! or name.

#[path = "support/fake_service.rs"]
mod fake_service;
#[path = "support/schema_check.rs"]
mod schema_check;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use astro_sync_core::analysis::AnalysisOptions;
use astro_sync_core::analysis::log_offset::ForcedLogOffsets;
use astro_sync_core::analysis::nina::{NinaOptions, parse_nina_log};
use astro_sync_core::analysis::report::{generate_report, render_report_json};
use astro_sync_core::analysis::sky_night::{NightSkyMeasure, SKY_METHOD_VERSION, Site};
use astro_sync_core::gui_bridge::{GuiBridgeError, HttpPortalGuiTransport};
use astro_sync_core::night_analysis::{
    NightAnalysis, SkyMeasureSending, analyze_night_detecting, ensure_not_empty,
    send_night_analysis,
};
use astro_sync_core::service_wait::{WaitHooks, WakePolicy};
use fake_service::{FakeService, Reply, Wake, error_reply};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn temp_night(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "astro-sync-report-v2-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn header(cards: &[(&str, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (keyword, value) in [("SIMPLE", "T"), ("BITPIX", "16"), ("NAXIS", "0")]
        .iter()
        .chain(cards.iter())
    {
        let mut card = format!("{keyword:<8}= {value}").into_bytes();
        card.resize(80, b' ');
        out.extend(card);
    }
    let mut end = b"END".to_vec();
    end.resize(80, b' ');
    out.extend(end);
    let len = out.len().div_ceil(2880) * 2880;
    out.resize(len, b' ');
    out
}

/// A night folder as the "Folder" step makes it, filled with the logs of
/// the fixtures and three lights of two targets, a light without `OBJECT`
/// (NINA's window names it) and a dark.
fn fixture_night(label: &str) -> PathBuf {
    let night = temp_night(label);
    for folder in [
        "logs",
        "calibration/darks",
        "NGC 7000/lights",
        "NGC 7000/final",
        "M31/lights",
    ] {
        fs::create_dir_all(night.join(folder)).unwrap();
    }
    fs::copy(
        fixture("nina_log_two_targets.txt"),
        night.join("logs/NINA-2026-10-07.log"),
    )
    .unwrap();
    fs::copy(
        fixture("phd2_guide_log_short.txt"),
        night.join("logs/PHD2_GuideLog_2030-03-01.txt"),
    )
    .unwrap();
    let light = |object: Option<&str>, filter: &str, exposure: &str, at: &str| {
        let mut cards = vec![
            ("IMAGETYP", "'LIGHT'".to_string()),
            ("FILTER", format!("'{filter}'")),
            ("EXPTIME", exposure.to_string()),
            ("GAIN", "100".to_string()),
            ("XBINNING", "1".to_string()),
            ("YBINNING", "1".to_string()),
            ("CCD-TEMP", "-10.0".to_string()),
            ("XPIXSZ", "3.76".to_string()),
            ("FOCALLEN", "400.0".to_string()),
            ("DATE-OBS", format!("'{at}'")),
        ];
        if let Some(object) = object {
            cards.push(("OBJECT", format!("'{object}'")));
        }
        let cards: Vec<(&str, &str)> = cards.iter().map(|(k, v)| (*k, v.as_str())).collect();
        header(&cards)
    };
    fs::write(
        night.join("NGC 7000/lights/ha-0001.fits"),
        light(Some("NGC 7000"), "Ha", "180.0", "2026-10-07T21:10:00"),
    )
    .unwrap();
    fs::write(
        night.join("NGC 7000/lights/ha-0002.fits"),
        light(Some("NGC7000"), "Ha", "180.0", "2026-10-07T21:13:30"),
    )
    .unwrap();
    // No OBJECT: the NINA window open at 21:16:40 (UTC, offset +00:00) is
    // NGC 7000.
    fs::write(
        night.join("NGC 7000/lights/oiii-0001.fits"),
        light(None, "OIII", "180.0", "2026-10-07T21:16:40"),
    )
    .unwrap();
    fs::write(
        night.join("M31/lights/l-0001.fits"),
        light(Some("M 31"), "L", "120.0", "2026-10-07T22:02:00"),
    )
    .unwrap();
    fs::write(
        night.join("calibration/darks/dark-0001.fits"),
        header(&[
            ("IMAGETYP", "'DARK'"),
            ("EXPTIME", "180.0"),
            ("GAIN", "100"),
        ]),
    )
    .unwrap();
    night
}

#[test]
fn the_nina_reader_on_two_fixtures() {
    let two = parse_nina_log(
        &fs::read_to_string(fixture("nina_log_two_targets.txt")).unwrap(),
        &NinaOptions::default(),
    )
    .unwrap();
    assert_eq!(two.exposures.len(), 4);
    assert_eq!(two.windows.len(), 2);
    assert_eq!(two.exposures[3].target.as_deref(), Some("M 31"));
    assert_eq!(
        two.exposures[3].filter, None,
        "« Filter: None » is no filter"
    );
    let types: Vec<&str> = two.events.iter().map(|e| e.event_type.as_str()).collect();
    assert_eq!(
        types,
        [
            "target-started",
            "autofocus",
            "autofocus-result",
            "dither",
            "target-finished",
            "target-started",
            "target-finished"
        ]
    );

    let flip = parse_nina_log(
        &fs::read_to_string(fixture("nina_log_flip_and_failed_focus.txt")).unwrap(),
        &NinaOptions {
            utc_offset: "+09:00".into(),
        },
    )
    .unwrap();
    assert_eq!(flip.exposures.len(), 2);
    assert_eq!(flip.exposures[0].timestamp_utc, "2026-10-07T15:11:00.000Z");
    assert_eq!(flip.exposures[0].gain, Some(120));
    let flips = flip
        .events
        .iter()
        .filter(|e| e.event_type == "meridian-flip")
        .count();
    assert_eq!(flips, 1, "the end of a flip is not a second flip");
    let result = flip
        .events
        .iter()
        .find(|e| e.event_type == "autofocus-result")
        .unwrap();
    assert_eq!(result.data["succeeded"], false);
    assert_eq!(
        flip.windows[0].finished_unix, None,
        "a window open to the end"
    );
}

#[test]
fn a_fixture_night_gives_the_report_v2_of_the_contract() {
    let night = fixture_night("contract");
    let report = generate_report(&night, &AnalysisOptions::default()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&render_report_json(&report)).unwrap();
    schema_check::assert_valid("session-analysis.schema.json", "report", &value);

    assert_eq!(value["reportVersion"], 3);
    assert_eq!(value["producer"], "astro-sync");
    // The raw samples are no longer sent: the trace of 10 s replaces them.
    assert!(value.get("guiding").is_none());
    assert_eq!(report.source.nina_files_parsed, 1);
    assert_eq!(report.source.phd2_files_parsed, 1);
    assert_eq!(report.source.fits_files_parsed, 5);
    assert_eq!(report.pixel_scale_arcsec, Some(1.94));

    // Steps: NGC 7000 Ha (2 frames, OBJECT written two ways), NGC 7000 OIII
    // (its target from NINA's window), M 31 L; the dark makes none.
    let steps: Vec<(&str, Option<&str>, u64)> = report
        .acquisition_steps
        .iter()
        .map(|s| (s.key.as_str(), s.target_hint.as_deref(), s.frame_count))
        .collect();
    assert_eq!(
        steps,
        [
            ("ngc-7000-ha-180s-g100", Some("NGC 7000"), 2),
            ("ngc-7000-oiii-180s-g100", Some("NGC 7000"), 1),
            ("m-31-l-120s-g100", Some("M 31"), 1),
        ]
    );
    assert_eq!(report.quality.len(), 4, "one row per light frame");
    assert!(report.quality.iter().all(|row| row.step_key.is_some()));

    // The PHD2 samples are in slices of two minutes; its dither and the
    // events of NINA are in the timeline.
    let trace = report
        .guiding_trace
        .as_ref()
        .expect("a PHD2 log gives a trace");
    assert!(!trace.points.is_empty());
    assert!(!report.guiding_series.is_empty());
    let total: u64 = report.guiding_series.iter().map(|s| s.samples).sum();
    assert_eq!(total, trace.samples);
    assert_eq!(trace.rms.samples, total);
    let types: Vec<&str> = report
        .timeline
        .events
        .iter()
        .map(|e| e.event_type.as_str())
        .collect();
    for expected in ["autofocus", "autofocus-result", "dither", "target-started"] {
        assert!(types.contains(&expected), "{expected} in {types:?}");
    }
    assert!(
        !render_report_json(&report).contains("Dithering"),
        "no raw log line leaves the computer"
    );
}

/// Rule: a file under `calibration/` is never a light frame, whatever its
/// header says. Darks taken as « Light » by a capture program (header
/// `IMAGETYP = Light`, `OBJECT` of the last target) do not count as the frames
/// of a target in the night's report.
#[test]
fn a_dark_with_a_light_header_under_calibration_is_not_a_frame_of_the_report() {
    let night = fixture_night("dark-as-light");
    for index in 1..=3 {
        fs::write(
            night.join(format!(
                "calibration/darks/Light_Pi Pegasi_180.0s_{index:04}.fits"
            )),
            header(&[
                ("IMAGETYP", "'Light'"),
                ("OBJECT", "'Pi Pegasi'"),
                ("EXPTIME", "180.0"),
                ("GAIN", "100"),
                ("DATE-OBS", "'2026-10-07T21:12:00'"),
            ]),
        )
        .unwrap();
    }
    let report = generate_report(&night, &AnalysisOptions::default()).unwrap();
    assert_eq!(
        report.quality.len(),
        4,
        "the lights only: {:?}",
        report.quality
    );
    assert!(
        report
            .quality
            .iter()
            .all(|row| !row.source_path.contains("calibration")),
        "no quality row from calibration/"
    );
    let targets: Vec<Option<&str>> = report
        .acquisition_steps
        .iter()
        .map(|step| step.target_hint.as_deref())
        .collect();
    assert!(
        !targets.contains(&Some("Pi Pegasi")),
        "no target of the dark's OBJECT: {targets:?}"
    );
    let frames: u64 = report.acquisition_steps.iter().map(|s| s.frame_count).sum();
    assert_eq!(frames, 4);
}

/// A night whose logs are on the clock of the computer that ran them
/// (UTC+2) and whose frames say UTC (`DATE-OBS`): eight frames of 90 s every
/// 100 s from 21:00:00 UTC, PHD2 guiding all along, an ASIAIR dither in each
/// gap.
fn night_with_logs_at_plus_two(label: &str) -> PathBuf {
    let night = temp_night(label);
    for folder in ["logs", "Sh2-190/lights"] {
        fs::create_dir_all(night.join(folder)).unwrap();
    }
    let clock = |seconds_after_21h_utc: u32, offset_hours: u32| {
        let total = (21 + offset_hours) * 3600 + seconds_after_21h_utc;
        format!(
            "{:02}:{:02}:{:02}",
            total / 3600,
            total % 3600 / 60,
            total % 60
        )
    };
    let mut phd2 = format!(
        "PHD2 version 2.6.13dev4, Log version 2.5. Log enabled at 2026-08-09 {}\nGuiding Begins at 2026-08-09 {}\nPixel scale = 1.85 arc-sec/px, Binning = 1, Focal length = 400 mm\nFrame,Time,mount,dx,dy,RARawDistance,DECRawDistance,RAGuideDistance,DECGuideDistance,RADuration,RADirection,DECDuration,DECDirection,XStep,YStep,StarMass,SNR,ErrorCode\n",
        clock(0, 2),
        clock(0, 2)
    );
    for frame in 1..=400_u32 {
        phd2.push_str(&format!(
            "{frame},{:.3},Mount,0.010,-0.020,0.100,-0.100,0.000,0.000,0,,0,,0,0,31000,13.0,0\n",
            f64::from(frame) * 2.0
        ));
    }
    phd2.push_str(&format!("Guiding Ends at 2026-08-09 {}\n", clock(801, 2)));
    fs::write(night.join("logs/PHD2_GuideLog_2026-08-09.txt"), phd2).unwrap();
    let mut asiair = String::new();
    for index in 0..8_u32 {
        asiair.push_str(&format!(
            "2026/08/09 {} [Guide] Dither\n",
            clock(index * 100 + 93, 2)
        ));
    }
    fs::write(night.join("logs/Autorun_Log_2026-08-09.txt"), asiair).unwrap();
    for index in 0..8_u32 {
        let at = format!("2026-08-09T{}", clock(index * 100, 0));
        fs::write(
            night.join(format!("Sh2-190/lights/light-{index:04}.fits")),
            header(&[
                ("IMAGETYP", "'LIGHT'"),
                ("OBJECT", "'Sh2-190'"),
                ("EXPTIME", "90.0"),
                ("GAIN", "100"),
                ("DATE-OBS", &format!("'{at}'")),
            ]),
        )
        .unwrap();
    }
    night
}

/// G1: the offset of the PHD2 and ASIAIR logs is found from the `DATE-OBS` of
/// the frames (the whole number of half hours that puts the most frames in
/// front of their guiding), said in the answer, and the person's own offset is
/// kept as it is.
#[test]
fn the_clock_of_the_logs_is_found_from_the_frames_and_said() {
    let night = night_with_logs_at_plus_two("log-clock");

    let found = analyze_night_detecting(&night, &ForcedLogOffsets::default(), None).unwrap();
    let clock = found.logs_utc_offset.as_ref().expect("the offset is said");
    assert!(clock.detected);
    assert_eq!(clock.phd2.as_deref(), Some("+02:00"));
    assert_eq!(clock.asiair.as_deref(), Some("+02:00"));
    assert_eq!(clock.nina, None, "no NINA log, no NINA clock");
    assert_eq!(clock.frames, 8);
    assert_eq!(
        clock.phd2_matched_frames,
        Some(8),
        "every frame has its guiding"
    );
    assert_eq!(found.report.quality.len(), 8);
    assert!(
        found
            .report
            .quality
            .iter()
            .all(|row| row.rms_total_arcsec.is_some()),
        "a guiding RMS for every frame"
    );
    let json: serde_json::Value = serde_json::to_value(&found).unwrap();
    assert_eq!(json["logsUtcOffset"]["detected"], true);
    assert_eq!(json["logsUtcOffset"]["phd2"], "+02:00");
    assert_eq!(json["logsUtcOffset"]["phd2MatchedFrames"], 8);

    // The old way: UTC, no frame has its guiding.
    let utc = analyze_night_with_options_at_utc(&night);
    assert!(utc.quality.iter().all(|row| row.rms_total_arcsec.is_none()));

    // Forced: the person's offsets are used, whatever the frames say.
    let forced = ForcedLogOffsets {
        phd2: Some("+00:00".into()),
        asiair: Some("+00:00".into()),
        nina: None,
    };
    let own = analyze_night_detecting(&night, &forced, None).unwrap();
    let clock = own.logs_utc_offset.as_ref().unwrap();
    assert!(!clock.detected, "nothing was found: the offsets are given");
    assert_eq!(clock.phd2.as_deref(), Some("+00:00"));
    assert_eq!(clock.phd2_matched_frames, Some(0));
    assert!(
        own.report
            .quality
            .iter()
            .all(|row| row.rms_total_arcsec.is_none())
    );

    // One forced, the other found.
    let half = ForcedLogOffsets {
        phd2: Some("+02:00".into()),
        ..ForcedLogOffsets::default()
    };
    let mixed = analyze_night_detecting(&night, &half, None).unwrap();
    let clock = mixed.logs_utc_offset.unwrap();
    assert!(clock.detected);
    assert_eq!(clock.phd2.as_deref(), Some("+02:00"));
    assert_eq!(clock.asiair.as_deref(), Some("+02:00"));
}

fn analyze_night_with_options_at_utc(
    night: &Path,
) -> astro_sync_core::analysis::report::SessionReport {
    generate_report(night, &AnalysisOptions::default()).unwrap()
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

const SESSION: &str = "0b9f3c1e-6d0a-4f5e-9a3b-2c1d0e9f8a7b";

const STORED_ANALYSIS: &str = r#"{"analysis":{"id":"6c0f3a8e-1b2d-4e5f-8a9b-0c1d2e3f4a5b","sessionId":"0b9f3c1e-6d0a-4f5e-9a3b-2c1d0e9f8a7b","reportVersion":2,"createdAt":"2026-10-08T09:12:00.000Z","originDeviceId":null,"deviceName":"Mac de Camille","report":{}},"replaced":true,"steps":{"proposed":3,"keptEdited":1}}"#;

const STORED_SKY: &str = r#"{"measure":{"placeId":"col-des-etoiles","night":"2026-10-07","sqm":21.04,"poseCount":4,"methodVersion":"astro-sync-sky/1","observedAt":"2026-10-07T23:00:00.000Z","sessionId":"0b9f3c1e-6d0a-4f5e-9a3b-2c1d0e9f8a7b","updatedAt":"2026-10-08T09:12:00.000Z"},"replaced":false}"#;

fn sky() -> NightSkyMeasure {
    NightSkyMeasure {
        night: "2026-10-07".into(),
        sqm: 21.04,
        pose_count: 4,
        method_version: SKY_METHOD_VERSION.into(),
        observed_at: "2026-10-07T23:00:00.000Z".into(),
        site: Site {
            latitude: 45.0,
            longitude: -30.0,
        },
    }
}

fn analysed(label: &str, with_sky: bool) -> NightAnalysis {
    let night = fixture_night(label);
    NightAnalysis {
        report: generate_report(&night, &AnalysisOptions::default()).unwrap(),
        sky_measure: with_sky.then(sky),
        sky_missing_reason: None,
        logs_utc_offset: None,
    }
}

#[test]
fn the_report_replaces_the_analysis_then_the_sky_value_goes_with_it() {
    let service = FakeService::start(Wake::Instant, |request| {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/api/v1/service") => fake_service::identity_reply(),
            ("POST", path) if path.ends_with("/analyses") => {
                Reply::Json(201, STORED_ANALYSIS.to_string())
            }
            ("POST", "/api/v1/place-sky-measures") => Reply::Json(200, STORED_SKY.to_string()),
            _ => error_reply(404, "not_found", "no such route"),
        }
    });
    let transport = HttpPortalGuiTransport::with_options(fast(), WaitHooks::silent()).unwrap();
    let night = analysed("send", true);
    let sent =
        send_night_analysis(&transport, &service.base_url(), "token", SESSION, &night).unwrap();
    let analysis = sent.analysis.unwrap();
    assert!(
        analysis.replaced,
        "one analysis per night: the new one replaces it"
    );
    assert_eq!(analysis.steps.kept_edited, 1);
    assert!(matches!(sent.sky_measure, SkyMeasureSending::Sent { .. }));

    let posts: Vec<_> = service
        .requests()
        .into_iter()
        .filter(|r| r.reached_app && r.method == "POST")
        .collect();
    assert_eq!(posts.len(), 2);
    assert_eq!(
        posts[0].path,
        format!("/api/v1/sessions/{SESSION}/analyses")
    );
    schema_check::assert_valid("session-analysis.schema.json", "report", &posts[0].json());
    assert_eq!(posts[1].path, "/api/v1/place-sky-measures");
    assert_eq!(posts[1].json()["sessionId"], SESSION);
}

#[test]
fn an_empty_night_sends_nothing_and_a_refused_sky_value_keeps_the_analysis() {
    let service = FakeService::start(Wake::Instant, |request| {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/api/v1/service") => fake_service::identity_reply(),
            ("POST", path) if path.ends_with("/analyses") => {
                Reply::Json(201, STORED_ANALYSIS.to_string())
            }
            ("POST", "/api/v1/place-sky-measures") => error_reply(
                422,
                "place_not_found",
                "No place of this account is within 500 m.",
            ),
            _ => error_reply(404, "not_found", "no such route"),
        }
    });
    let transport = HttpPortalGuiTransport::with_options(fast(), WaitHooks::silent()).unwrap();

    let empty_folder = temp_night("empty");
    let empty = NightAnalysis {
        report: generate_report(&empty_folder, &AnalysisOptions::default()).unwrap(),
        sky_measure: None,
        sky_missing_reason: Some("no_reference_stars".into()),
        logs_utc_offset: None,
    };
    let error =
        send_night_analysis(&transport, &service.base_url(), "token", SESSION, &empty).unwrap_err();
    assert!(matches!(&error, GuiBridgeError::Refused { code, .. } if code == "analysis_empty"));
    assert_eq!(error.code(), "analysis_empty");
    assert!(
        service.requests().iter().all(|r| r.method != "POST"),
        "an empty report never leaves the computer"
    );
    assert!(
        ensure_not_empty(&serde_json::json!({"reportVersion": 2, "timeline": {"events": []}}))
            .is_err()
    );

    let night = analysed("sky-refused", true);
    let sent =
        send_night_analysis(&transport, &service.base_url(), "token", SESSION, &night).unwrap();
    assert!(sent.analysis.is_some());
    assert_eq!(
        sent.sky_measure,
        SkyMeasureSending::Refused {
            code: "place_not_found".into()
        }
    );
}

#[test]
fn a_refused_report_stops_before_the_sky_value() {
    let service = FakeService::start(Wake::Instant, |request| {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/api/v1/service") => fake_service::identity_reply(),
            ("POST", path) if path.ends_with("/analyses") => error_reply(
                403,
                "analysis_not_entitled",
                "This account cannot upload local-analysis reports.",
            ),
            _ => error_reply(404, "not_found", "no such route"),
        }
    });
    let transport = HttpPortalGuiTransport::with_options(fast(), WaitHooks::silent()).unwrap();
    let night = analysed("refused", true);
    let error =
        send_night_analysis(&transport, &service.base_url(), "token", SESSION, &night).unwrap_err();
    assert_eq!(error.code(), "analysis_not_entitled");
    assert_eq!(service.app_hits("POST", "/api/v1/place-sky-measures"), 0);
}

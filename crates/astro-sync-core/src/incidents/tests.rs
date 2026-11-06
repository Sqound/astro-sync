use super::EvidenceSource as Source;
use super::detect::{
    Flip, Loss, Pose, exposure_change_incidents, flips_of_nina, group_guide_losses, group_spikes,
    group_timeouts, pause_incidents, temperature_drift_incidents, unique_ids,
};
use super::*;
use crate::analysis::log_offset::ForcedLogOffsets;
use crate::detection::NightWindow;
use crate::gui_bridge::GuiBridgeError;
use crate::test_support::{TempDir, fits_bytes};
use crate::time_util::parse_iso8601_utc_to_unix_seconds;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

/// 2026-08-14T21:30:00Z, the evening of Sadr's night.
fn base() -> f64 {
    parse_iso8601_utc_to_unix_seconds("2026-08-14T21:30:00Z").unwrap()
}

fn clock(unix: f64) -> String {
    iso_seconds(unix)
        .replace('-', "/")
        .replace('T', " ")
        .replace('Z', "")
}

fn iso_ms(unix: f64) -> String {
    crate::time_util::format_iso8601_utc_millis(unix)
}

/// A pose of the night: start (seconds after [`base`]), exposure, `CCD-TEMP`.
#[derive(Clone, Copy)]
struct P(f64, f64, Option<f64>);

struct Night {
    _dir: TempDir,
    folder: PathBuf,
}

impl Night {
    /// The poses of `Sadr/lights/`, and the logs of `logs/`.
    fn new(poses: &[P], asiair: Option<&str>, phd2: Option<&str>) -> Self {
        let dir = TempDir::new("incidents");
        let folder = dir
            .path()
            .join("2026-08-14 \u{b7} Sample site \u{b7} abc123");
        fs::create_dir_all(folder.join("Sadr/lights")).unwrap();
        fs::create_dir_all(folder.join("logs")).unwrap();
        for (index, pose) in poses.iter().enumerate() {
            write_pose(
                &folder.join("Sadr/lights"),
                index,
                base() + pose.0,
                pose.1,
                pose.2,
            );
        }
        if let Some(text) = asiair {
            fs::write(folder.join("logs/Autorun_Log_2026-08-14_233001.txt"), text).unwrap();
        }
        if let Some(text) = phd2 {
            fs::write(
                folder.join("logs/PHD2_GuideLog_2026-08-14_230510.txt"),
                text,
            )
            .unwrap();
        }
        Self { _dir: dir, folder }
    }

    fn input(&self) -> DetectInput {
        DetectInput {
            night_folder: self.folder.clone(),
            target_folder: Some("Sadr".to_string()),
            window: NightWindow::of_evening("2026-08-14", Some(7.45)),
            log_offsets: utc_logs(),
        }
    }
}

fn utc_logs() -> ForcedLogOffsets {
    ForcedLogOffsets {
        phd2: Some("+00:00".to_string()),
        asiair: Some("+00:00".to_string()),
        nina: Some("+00:00".to_string()),
    }
}

fn write_pose(dir: &std::path::Path, index: usize, start: f64, exposure: f64, temp: Option<f64>) {
    let date = format!("'{}'", iso_ms(start));
    let exposure = format!("{exposure:.1}");
    let mut cards = vec![
        ("IMAGETYP", "'LIGHT'"),
        ("OBJECT", "'Sadr'"),
        ("EXPTIME", exposure.as_str()),
        ("DATE-OBS", date.as_str()),
    ];
    let temp_text = temp.map(|t| format!("{t:.1}"));
    if let Some(text) = &temp_text {
        cards.push(("CCD-TEMP", text.as_str()));
        // A cooled camera (the ASIAIR writes the set point).
        cards.push(("SET-TEMP", "-10.0"));
    }
    fs::write(
        dir.join(format!("Light_Sadr_{index:04}.fit")),
        fits_bytes(&cards, 0),
    )
    .unwrap();
}

/// `count` poses of `exposure` seconds, one every `step` seconds from `from`.
fn run(from: f64, count: usize, exposure: f64, step: f64) -> Vec<P> {
    (0..count)
        .map(|i| P(from + step * i as f64, exposure, Some(-10.0)))
        .collect()
}

fn incident<'a>(detection: &'a Detection, code: &str) -> Vec<&'a DetectedIncident> {
    detection
        .incidents
        .iter()
        .filter(|i| i.code == code)
        .collect()
}

/// The payload of an incident as the schema of its type allows it:
/// `additionalProperties: false`, the required fields, the bounds.
fn assert_payload(incident: &DetectedIncident) {
    let payload = incident
        .payload
        .as_object()
        .expect("a payload is an object");
    let (required, optional): (&[&str], &[&str]) = match incident.event_type {
        IncidentKind::Pause => (&[], &["reason", "cause"]),
        IncidentKind::EquipmentIssue => (&["description"], &["equipmentId", "severity"]),
        IncidentKind::ExposureChange => (&["fromSeconds", "toSeconds"], &["description"]),
        IncidentKind::GuidingTimeouts => (&["count", "firstAt", "lastAt"], &["description"]),
        IncidentKind::SensorTemperatureDrift => (&["fromCelsius", "toCelsius"], &["description"]),
        IncidentKind::GuidingSpikes => (
            &["count", "maxArcSeconds", "firstAt", "lastAt"],
            &["description"],
        ),
        IncidentKind::Condition | IncidentKind::Note => (&[], &[]),
    };
    for field in required {
        assert!(
            payload.contains_key(*field),
            "{} lacks {field}",
            incident.id
        );
    }
    for key in payload.keys() {
        assert!(
            required.contains(&key.as_str()) || optional.contains(&key.as_str()),
            "{}: {key} is outside the schema",
            incident.id
        );
    }
    let length = |field: &str| {
        payload
            .get(field)
            .and_then(Value::as_str)
            .map(|t| t.chars().count())
    };
    assert!(length("description").is_none_or(|n| (1..=300).contains(&n)));
    assert!(length("reason").is_none_or(|n| (1..=200).contains(&n)));
    if let Some(severity) = payload.get("severity") {
        assert!(["minor", "blocking"].contains(&severity.as_str().unwrap()));
    }
    if let Some(cause) = payload.get("cause") {
        assert!(
            [
                "clouds",
                "wind",
                "dew",
                "guiding",
                "meridian_flip",
                "equipment",
                "fatigue",
                "other"
            ]
            .contains(&cause.as_str().unwrap())
        );
    }
    for field in ["count"] {
        if let Some(count) = payload.get(field) {
            assert!(count.as_u64().is_some_and(|c| c >= 1));
        }
    }
    for field in ["firstAt", "lastAt"] {
        if let Some(at) = payload.get(field) {
            let at = at.as_str().unwrap();
            assert!(parse_iso8601_utc_to_unix_seconds(at).is_some() && at.len() == 20);
        }
    }
    if let Some(seconds) = payload.get("fromSeconds") {
        assert!((0.001..=86_400.0).contains(&seconds.as_f64().unwrap()));
    }
}

/// The meridian flip of Sadr: pose 13 ends at 00:13:37 local, the flip runs
/// from 00:13:38 to 00:24:58, pose 14 starts at 00:25:24 (a gap of 11 min
/// 47 s), here in UTC.
fn sadr_flip_night() -> (Night, f64, f64) {
    let mut poses = run(0.0, 13, 180.0, 181.0);
    let gap_start = base() + 12.0 * 181.0 + 180.0;
    let next = gap_start + 707.0;
    poses.extend(run(next - base(), 8, 180.0, 181.0));
    let begin = gap_start + 1.0;
    let end = begin + 680.0;
    let asiair = [
        format!("{} [Guide] Stop Guiding", clock(begin)),
        format!(
            "{} [Meridian Flip|Begin] Wait 10min18s to Meridian Flip",
            clock(begin)
        ),
        format!("{} [Meridian Flip|End] Meridian Flip succeeded", clock(end)),
    ]
    .join("\n");
    (Night::new(&poses, Some(&asiair), None), begin, end)
}

// ---------------------------------------------------------------------------
// Detection: pauses
// ---------------------------------------------------------------------------

#[test]
fn a_flip_that_covers_a_gap_is_one_pause_with_its_two_instants_and_the_frames_around() {
    let (night, begin, end) = sadr_flip_night();
    let detection = detect_incidents(&night.input());
    let pauses: Vec<_> = detection
        .incidents
        .iter()
        .filter(|i| i.event_type == IncidentKind::Pause)
        .collect();
    assert_eq!(
        pauses.len(),
        1,
        "one pause, not a flip and a gap: {pauses:#?}"
    );
    let pause = pauses[0];
    assert_eq!(pause.code, "meridian_flip");
    assert_eq!(pause.start, iso_seconds(begin));
    assert_eq!(pause.end.as_deref(), Some(iso_seconds(end).as_str()));
    assert_eq!(pause.id, format!("pause:{}", iso_seconds(begin)));
    assert_eq!(pause.params["durationSeconds"], 680);
    assert_eq!(pause.params["afterPose"], 13);
    assert_eq!(pause.params["beforePose"], 14);
    assert_eq!(pause.params["gapSeconds"], 707);
    assert_eq!(pause.params["succeeded"], true);
    assert_eq!(pause.evidence.source, EvidenceSource::Asiair);
    assert_eq!(pause.evidence.code, "meridian_flip");
    assert_eq!(
        pause.payload,
        json!({ "reason": "Meridian flip", "cause": "meridian_flip" })
    );
    assert!(pause.description.contains("no frame between 13 and 14"));
    assert_payload(pause);
    assert_eq!(detection.poses, 21);
    assert_eq!(detection.logs.asiair, 1);
}

#[test]
fn a_gap_without_a_flip_is_a_pause_of_frames_and_a_short_gap_is_not() {
    // 20 poses, one every 181 s; a hole of 400 s after the 9th (a cloud), and
    // a hole of 100 s after the 15th (an autofocus): only the first counts.
    let mut poses = run(0.0, 9, 180.0, 181.0);
    let after_hole = 8.0 * 181.0 + 180.0 + 400.0;
    poses.extend(run(after_hole, 6, 180.0, 181.0));
    let after_short = after_hole + 5.0 * 181.0 + 180.0 + 100.0;
    poses.extend(run(after_short, 6, 180.0, 181.0));
    let night = Night::new(&poses, None, None);
    let detection = detect_incidents(&night.input());
    let gaps = incident(&detection, "pose_gap");
    assert_eq!(gaps.len(), 1, "{:#?}", detection.incidents);
    let gap = gaps[0];
    assert_eq!(gap.start, iso_seconds(base() + 8.0 * 181.0 + 180.0));
    assert_eq!(
        gap.end.as_deref(),
        Some(iso_seconds(base() + after_hole).as_str())
    );
    assert_eq!(gap.params["afterPose"], 9);
    assert_eq!(gap.params["beforePose"], 10);
    assert_eq!(gap.params["gapSeconds"], 400);
    assert_eq!(gap.evidence.source, EvidenceSource::Fits);
    assert_eq!(gap.evidence.code, "no_pose_between");
    assert!(
        gap.payload["reason"]
            .as_str()
            .unwrap()
            .contains("6 min 40 s")
    );
    assert_payload(gap);
    assert!(incident(&detection, "meridian_flip").is_empty());
}

fn pose(rank: usize, start: f64, exposure: f64) -> Pose {
    Pose {
        rank,
        start,
        exposure,
        temperature: Some(-10.0),
        set_temperature: Some(-10.0),
    }
}

#[test]
fn the_rhythm_of_the_night_decides_what_a_gap_is() {
    // Poses every 300 s with a 120 s gap each time: a 240 s gap is not three
    // times that rhythm, 400 s is.
    let mut poses = Vec::new();
    let mut at = 0.0;
    for rank in 1..=12 {
        poses.push(pose(rank, at, 180.0));
        at += 180.0 + 120.0;
    }
    let mut unproposed = Vec::new();
    assert!(pause_incidents(&poses, &[], &mut unproposed).is_empty());
    poses[8].start += 280.0; // a gap of 400 s before pose 9
    for p in &mut poses[9..] {
        p.start += 280.0;
    }
    let found = pause_incidents(&poses, &[], &mut unproposed);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].params["gapSeconds"], 400);
    // « Usual rhythm: one frame every ... »: the cycle of the frames (300 s),
    // not the median gap between them.
    assert_eq!(found[0].params["typicalSeconds"], 300);
    // Fewer than five poses: no rhythm, no gap.
    assert!(pause_incidents(&poses[..4], &[], &mut unproposed).is_empty());
}

#[test]
fn a_flip_with_no_gap_still_is_a_pause_and_one_with_nothing_to_close_it_is_told_apart() {
    let poses: Vec<Pose> = (0..10)
        .map(|i| pose(i + 1, i as f64 * 181.0, 180.0))
        .collect();
    let closed = Flip {
        begin: 500.0,
        end: Some(560.0),
        succeeded: Some(true),
        source: Source::Asiair,
    };
    let open = Flip {
        begin: 900.0,
        end: None,
        succeeded: None,
        source: Source::Nina,
    };
    let mut unproposed = Vec::new();
    let found = pause_incidents(&poses, &[closed, open], &mut unproposed);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].params["durationSeconds"], 60);
    assert!(found[0].params.get("afterPose").is_none());
    assert_eq!(unproposed.len(), 1);
    assert_eq!(unproposed[0].code, "meridian_flip_unbounded");
    // NINA writes several lines for one flip, and no end.
    let flips = flips_of_nina(&[100.0, 130.0, 160.0, 5000.0]);
    assert_eq!(flips.len(), 2);
    assert_eq!(flips[0].end, None);
}

#[test]
fn a_nina_flip_that_a_gap_covers_ends_with_the_gap() {
    let mut poses = run(0.0, 8, 180.0, 181.0);
    let hole_from = 7.0 * 181.0 + 180.0;
    poses.extend(run(hole_from + 600.0 - 0.0, 6, 180.0, 181.0));
    let flip = Flip {
        begin: hole_from + 30.0,
        end: None,
        succeeded: None,
        source: Source::Nina,
    };
    let rebased: Vec<Pose> = poses
        .iter()
        .enumerate()
        .map(|(i, p)| pose(i + 1, p.0, p.1))
        .collect();
    let mut unproposed = Vec::new();
    let found = pause_incidents(&rebased, &[flip], &mut unproposed);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].code, "meridian_flip");
    assert_eq!(found[0].evidence.source, EvidenceSource::Nina);
    assert_eq!(
        found[0].end.as_deref(),
        Some(iso_seconds(hole_from + 600.0).as_str())
    );
    assert!(unproposed.is_empty());
}

#[test]
fn the_instants_are_rounded_to_the_second_and_key_the_incident() {
    // The sixth pose starts at 21:45:05.600 and ends at 21:48:05.600: the
    // pause begins at :06. The next one starts at 21:54:46.400: it ends at :46.
    let mut poses: Vec<P> = (0..6)
        .map(|i| P(i as f64 * 181.0 + 0.6, 180.0, Some(-10.0)))
        .collect();
    poses.push(P(5.0 * 181.0 + 0.6 + 180.0 + 400.8, 180.0, Some(-10.0)));
    let night = Night::new(&poses, None, None);
    let detection = detect_incidents(&night.input());
    let gap = &incident(&detection, "pose_gap")[0];
    let expected_from = base() + 5.0 * 181.0 + 0.6 + 180.0;
    assert_eq!(gap.start, iso_seconds(expected_from));
    assert_eq!(gap.start, "2026-08-14T21:48:06Z");
    assert_eq!(gap.id, "pause:2026-08-14T21:48:06Z");
    assert_eq!(gap.end.as_deref(), Some("2026-08-14T21:54:46Z"));
    assert_eq!(gap.params["gapSeconds"], 401);
    // Half a second goes up.
    assert_eq!(iso_seconds(base() + 10.5), "2026-08-14T21:30:11Z");
    assert_eq!(iso_seconds(base() + 10.49), "2026-08-14T21:30:10Z");
}

// ---------------------------------------------------------------------------
// Detection: guide star lost
// ---------------------------------------------------------------------------

fn lost_night(lines: &[(f64, &str)]) -> Night {
    let poses = run(0.0, 40, 180.0, 181.0);
    let asiair: Vec<String> = lines
        .iter()
        .map(|(at, text)| format!("{} {text}", clock(base() + at)))
        .collect();
    Night::new(&poses, Some(&asiair.join("\n")), None)
}

#[test]
fn guide_star_losses_less_than_ten_minutes_apart_are_one_incident_and_the_rest_are_not() {
    let lost = "[Guide] Guide star lost";
    // Two losses a minute apart; then 25 minutes later eight, the seventh
    // followed by a star that was not found again.
    let mut lines: Vec<(f64, &str)> = vec![(600.0, lost), (660.0, lost)];
    for at in [2160.0, 2430.0, 2490.0, 2910.0, 2970.0, 3030.0, 3330.0] {
        lines.push((at, lost));
    }
    lines.push((3379.0, "[Guide] Select Guide Star failed, no star found"));
    lines.push((3420.0, lost));
    let night = lost_night(&lines);
    let detection = detect_incidents(&night.input());
    let losses = incident(&detection, "guide_star_lost");
    assert_eq!(losses.len(), 2, "{losses:#?}");
    assert_eq!(losses[0].params, json!({ "count": 2, "notFound": 0 }));
    assert_eq!(losses[0].description, "Guide star lost 2 times");
    assert_eq!(losses[0].start, iso_seconds(base() + 600.0));
    assert_eq!(
        losses[0].end.as_deref(),
        Some(iso_seconds(base() + 660.0).as_str())
    );
    // The « no star found » is the end of the seventh loss, not a loss of its own.
    assert_eq!(losses[1].params, json!({ "count": 8, "notFound": 1 }));
    assert_eq!(
        losses[1].description,
        "Guide star lost 8 times, once with no star found again"
    );
    assert_eq!(losses[1].event_type, IncidentKind::EquipmentIssue);
    assert_eq!(losses[1].evidence.source, EvidenceSource::Asiair);
    assert_eq!(losses[1].payload["severity"], "minor");
    assert_payload(losses[0]);
    assert_payload(losses[1]);
}

#[test]
fn a_no_star_found_far_from_any_loss_is_a_loss_and_a_single_loss_has_no_end() {
    let items = [
        (0.0, Loss::Lost),
        (1000.0, Loss::NoStarFound),
        (2000.0, Loss::NoStarFound),
        (2100.0, Loss::Lost),
    ];
    let found = group_guide_losses(Source::Asiair, &items);
    assert_eq!(found.len(), 3);
    assert_eq!(found[0].description, "Guide star lost once");
    assert!(found[0].end.is_none());
    // Alone, a star not found is a loss that no star followed.
    assert_eq!(found[1].params, json!({ "count": 1, "notFound": 1 }));
    // One that follows a loss by at most five minutes is that loss's end.
    assert_eq!(found[2].params, json!({ "count": 2, "notFound": 1 }));
}

fn phd2_log(start: f64, drops: &[(f64, usize)]) -> String {
    // A session from `start`, a good frame every 3 s for 60 s, then streaks
    // of dropped frames `(seconds after start, how many)`.
    let mut lines = vec![
        format!("Guiding Begins at {}", clock(base() + start)).replace('/', "-"),
        "Pixel scale = 2.00 arc-sec/px, Binning = 1".to_string(),
        "Frame,Time,mount,dx,dy,RARawDistance,DECRawDistance,RAGuideDistance,DECGuideDistance,RADuration,RADirection,DECDuration,DECDirection,XStep,YStep,StarMass,SNR,ErrorCode".to_string(),
    ];
    let mut frame = 1;
    for i in 0..20 {
        lines.push(format!(
            "{frame},{:.3},\"Mount\",0,0,0.100,0.100,0,0,0,,0,,,,1800,30,0",
            3.0 + 3.0 * f64::from(i)
        ));
        frame += 1;
    }
    for (from, count) in drops {
        for i in 0..*count {
            // PHD2 says « Star lost » or « No star found » of a dropped frame:
            // both are dropped frames.
            let message = if i % 2 == 0 {
                "0,0.00,3,\"Star lost - low mass\""
            } else {
                "478,15.36,7,\"No star found\""
            };
            lines.push(format!(
                "{frame},{:.3},\"DROP\",,,,,,,,,,,,,{message}",
                from + 3.0 * i as f64
            ));
            frame += 1;
        }
    }
    lines.join("\n")
}

#[test]
fn without_an_asiair_log_the_losses_come_from_the_phd2_dropped_frames_and_with_one_they_do_not() {
    // A streak of 12 dropped frames is a loss; one of 4 is a gust.
    let poses = run(0.0, 40, 180.0, 181.0);
    let phd2 = phd2_log(300.0, &[(100.0, 12), (400.0, 4)]);
    let night = Night::new(&poses, None, Some(&phd2));
    let detection = detect_incidents(&night.input());
    let losses = incident(&detection, "guide_star_lost");
    assert_eq!(losses.len(), 1, "{:#?}", detection.incidents);
    assert_eq!(losses[0].params, json!({ "count": 1, "notFound": 0 }));
    assert_eq!(losses[0].evidence.source, EvidenceSource::Phd2);
    assert_eq!(losses[0].start, iso_seconds(base() + 400.0));

    // The same PHD2 log beside an ASIAIR log that says nothing: the ASIAIR
    // log is the source, its silence is not filled with PHD2's dropped frames.
    let asiair = format!("{} [Guide] Dither", clock(base() + 1000.0));
    let night = Night::new(&poses, Some(&asiair), Some(&phd2));
    let detection = detect_incidents(&night.input());
    assert!(incident(&detection, "guide_star_lost").is_empty());
    assert_eq!(detection.logs.phd2, 1);
}

// ---------------------------------------------------------------------------
// Detection: timeouts, spikes, exposure, temperature
// ---------------------------------------------------------------------------

#[test]
fn the_settle_timeouts_make_one_event_per_group_of_sixty_minutes() {
    let timeouts = [0.0, 780.0, 1560.0, 4500.0, 15_000.0, 16_000.0];
    let items: Vec<(f64, ())> = timeouts.iter().map(|t| (base() + t, ())).collect();
    let found = group_timeouts(Source::Asiair, &items);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].params["count"], 4);
    assert_eq!(found[0].params["firstAt"], iso_seconds(base()));
    assert_eq!(found[0].params["lastAt"], iso_seconds(base() + 4500.0));
    assert_eq!(found[1].params["count"], 2);
    assert_eq!(found[1].evidence.code, "settle_timeout");
    for incident in &found {
        assert_payload(incident);
    }
}

#[test]
fn the_settle_timeouts_of_an_autorun_log_are_read_from_the_file() {
    let lines: Vec<String> = [100.0, 900.0, 1700.0]
        .iter()
        .map(|t| format!("{} [Guide] Settle Timeout", clock(base() + t)))
        .collect();
    let poses = run(0.0, 20, 180.0, 181.0);
    let night = Night::new(&poses, Some(&lines.join("\n")), None);
    let detection = detect_incidents(&night.input());
    let timeouts = incident(&detection, "guiding_timeouts");
    assert_eq!(timeouts.len(), 1);
    assert_eq!(timeouts[0].params["count"], 3);
    assert_payload(timeouts[0]);
}

#[test]
fn guiding_spikes_outside_a_dither_settle_are_counted_and_the_settle_s_own_error_is_not() {
    // Pixel scale 2 "/px: 6 px = 12 ". Four spikes at 201..210 s; a dither at
    // 300 s whose settling makes a big error of its own until 345 s.
    let mut lines = vec![
        format!("Guiding Begins at {}", clock(base())).replace('/', "-"),
        "Pixel scale = 2.00 arc-sec/px, Binning = 1".to_string(),
        "Frame,Time,mount,dx,dy,RARawDistance,DECRawDistance,RAGuideDistance,DECGuideDistance,RADuration,RADirection,DECDuration,DECDirection,XStep,YStep,StarMass,SNR,ErrorCode".to_string(),
    ];
    for i in 0..200_u32 {
        let at = 3.0 * f64::from(i);
        if i == 100 {
            lines.push("INFO: DITHER by 1.0, 1.0".to_string());
            lines.push("INFO: SETTLING STATE CHANGE, Settling started".to_string());
        }
        let spike = (200.0..=212.0).contains(&at);
        let in_settle = (300.0..=345.0).contains(&at);
        let raw = if spike || in_settle { 6.0 } else { 0.2 };
        lines.push(format!(
            "{},{at:.3},\"Mount\",0,0,{raw},0.100,0,0,0,,0,,,,1800,30,0",
            i + 1
        ));
        if i == 120 {
            lines.push("INFO: SETTLING STATE CHANGE, Settling complete".to_string());
        }
    }
    let phd2 = lines.join("\n");
    let poses = run(0.0, 40, 180.0, 181.0);
    let night = Night::new(&poses, None, Some(&phd2));
    let detection = detect_incidents(&night.input());
    let spikes = incident(&detection, "guiding_spikes");
    assert_eq!(
        spikes.len(),
        1,
        "{:#?} {:#?}",
        detection.incidents,
        detection.unproposed
    );
    // 201, 204, 207 and 210 s: four errors of hypot(12, 0.2) = 12.0 ".
    assert_eq!(spikes[0].params["count"], 4);
    assert_eq!(spikes[0].params["maxArcSeconds"], 12.0);
    assert_eq!(spikes[0].params["thresholdArcSeconds"], SPIKE_ARCSEC);
    assert_eq!(spikes[0].evidence.source, EvidenceSource::Phd2);
    assert_eq!(spikes[0].start, iso_seconds(base() + 201.0));
    assert_payload(spikes[0]);
    assert!(detection.unproposed.is_empty());
}

#[test]
fn a_big_guiding_error_between_two_poses_is_not_a_spike_of_an_image() {
    // The mount slews between the two poses (a re-centering): errors of 12 "
    // at 300..312 s, nothing a pose shows. Four real spikes during the first
    // pose, at 150..159 s.
    let mut lines = vec![
        format!("Guiding Begins at {}", clock(base())).replace('/', "-"),
        "Pixel scale = 2.00 arc-sec/px, Binning = 1".to_string(),
        "Frame,Time,mount,dx,dy,RARawDistance,DECRawDistance,RAGuideDistance,DECGuideDistance,RADuration,RADirection,DECDuration,DECDirection,XStep,YStep,StarMass,SNR,ErrorCode".to_string(),
    ];
    for i in 0..260_u32 {
        let at = 3.0 * f64::from(i);
        let big = (150.0..=159.0).contains(&at) || (300.0..=312.0).contains(&at);
        lines.push(format!(
            "{},{at:.3},\"Mount\",0,0,{},0.100,0,0,0,,0,,,,1800,30,0",
            i + 1,
            if big { 6.0 } else { 0.2 }
        ));
    }
    let poses = [P(0.0, 180.0, None), P(600.0, 180.0, None)];
    let night = Night::new(
        &poses,
        None,
        Some(&lines.join(
            "
",
        )),
    );
    let detection = detect_incidents(&night.input());
    let spikes = incident(&detection, "guiding_spikes");
    assert_eq!(spikes.len(), 1, "{:#?}", detection.incidents);
    assert_eq!(spikes[0].params["count"], 4);
    assert_eq!(spikes[0].params["lastAt"], iso_seconds(base() + 159.0));
}

#[test]
fn a_group_of_fewer_than_three_spikes_is_told_apart_not_proposed() {
    let mut unproposed = Vec::new();
    let found = group_spikes(&[(0.0, 9.0), (10.0, 12.0)], &mut unproposed);
    assert!(found.is_empty());
    assert_eq!(unproposed[0].code, "guiding_spikes");
    assert_eq!(unproposed[0].count, 2);
    let found = group_spikes(
        &[(0.0, 9.0), (10.0, 12.04), (20.0, 8.0), (9000.0, 30.0)],
        &mut unproposed,
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].params["maxArcSeconds"], 12.0);
    assert_eq!(unproposed[1].count, 1);
}

#[test]
fn a_change_of_exposure_is_a_change_of_run_not_a_test_frame_and_an_alternating_night_is_not_proposed()
 {
    let mut unproposed = Vec::new();
    let mut poses = Vec::new();
    let mut rank = 0;
    let mut at = 0.0;
    let mut add = |poses: &mut Vec<Pose>, count: usize, exposure: f64| {
        for _ in 0..count {
            rank += 1;
            poses.push(pose(rank, at, exposure));
            at += exposure + 1.0;
        }
    };
    add(&mut poses, 20, 10.0);
    add(&mut poses, 1, 2.0); // a test frame
    add(&mut poses, 5, 10.0);
    add(&mut poses, 15, 30.0);
    let found = exposure_change_incidents(&poses, &mut unproposed);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].params["fromSeconds"], 10.0);
    assert_eq!(found[0].params["toSeconds"], 30.0);
    assert_eq!(found[0].params["atPose"], 27);
    assert_eq!(found[0].evidence.source, EvidenceSource::Fits);
    assert_payload(&found[0]);
    assert!(unproposed.is_empty());

    // Six runs of three, alternating 10 s and 30 s: told, not proposed.
    let mut alternating = Vec::new();
    let mut rank = 0;
    for run in 0..7 {
        for _ in 0..3 {
            rank += 1;
            alternating.push(pose(
                rank,
                f64::from(rank as u32) * 40.0,
                if run % 2 == 0 { 10.0 } else { 30.0 },
            ));
        }
    }
    let found = exposure_change_incidents(&alternating, &mut unproposed);
    assert!(found.is_empty());
    assert_eq!(unproposed[0].code, "exposure_change");
    assert_eq!(unproposed[0].count, 6);
}

#[test]
fn the_sensor_temperature_drift_is_read_from_the_headers_and_a_steady_sensor_says_nothing() {
    let mut poses: Vec<P> = run(0.0, 10, 180.0, 181.0);
    let night = Night::new(&poses, None, None);
    let detection = detect_incidents(&night.input());
    assert!(incident(&detection, "sensor_temperature_drift").is_empty());

    for (i, p) in poses.iter_mut().enumerate() {
        p.2 = Some(-10.0 + i as f64 * 0.5);
    }
    let night = Night::new(&poses, None, None);
    let detection = detect_incidents(&night.input());
    let drift = incident(&detection, "sensor_temperature_drift");
    assert_eq!(drift.len(), 1);
    assert_eq!(
        drift[0].params,
        json!({ "fromCelsius": -10.0, "toCelsius": -5.5 })
    );
    assert_eq!(drift[0].evidence.source, EvidenceSource::Fits);
    assert_eq!(
        drift[0].end.as_deref(),
        Some(iso_seconds(base() + 9.0 * 181.0).as_str())
    );
    assert_payload(drift[0]);

    // 2.9 degrees is under the threshold.
    let small: Vec<Pose> = (0..5)
        .map(|i| Pose {
            temperature: Some(-10.0 + f64::from(i) * 0.725),
            ..pose(i as usize + 1, f64::from(i) * 181.0, 180.0)
        })
        .collect();
    assert!(temperature_drift_incidents(&small).is_empty());
}

// ---------------------------------------------------------------------------
// Detection: scope, identity, read only
// ---------------------------------------------------------------------------

#[test]
fn only_what_falls_in_the_window_and_in_the_span_of_the_target_counts() {
    let lost = "[Guide] Guide star lost";
    // Before the first pose (the set-up), during the night, long after the
    // last pose; and a pose of another night, beyond the window.
    let mut poses = run(0.0, 10, 180.0, 181.0);
    poses.push(P(2.0 * 86_400.0, 180.0, Some(-10.0)));
    let lines = [
        format!("{} {lost}", clock(base() - 1500.0)),
        format!("{} {lost}", clock(base() + 700.0)),
        format!("{} {lost}", clock(base() + 9.0 * 181.0 + 180.0 + 3000.0)),
    ]
    .join("\n");
    let night = Night::new(&poses, Some(&lines), None);
    let detection = detect_incidents(&night.input());
    assert_eq!(detection.poses, 10, "the pose of another night is left out");
    let losses = incident(&detection, "guide_star_lost");
    assert_eq!(losses.len(), 1);
    assert_eq!(losses[0].start, iso_seconds(base() + 700.0));
}

#[test]
fn a_target_is_read_by_its_own_poses_and_the_rank_counts_them_alone() {
    let mut poses = run(0.0, 12, 180.0, 181.0);
    let gap_from = 11.0 * 181.0 + 180.0;
    poses.extend(run(gap_from + 500.0, 6, 180.0, 181.0));
    let night = Night::new(&poses, None, None);
    // Another target's poses in the same night, in the hole.
    fs::create_dir_all(night.folder.join("M27/lights")).unwrap();
    for i in 0..4_u32 {
        write_pose(
            &night.folder.join("M27/lights"),
            i as usize,
            base() + gap_from + 60.0 + f64::from(i) * 100.0,
            90.0,
            Some(-10.0),
        );
    }
    let detection = detect_incidents(&night.input());
    assert_eq!(detection.poses, 18);
    let gap = &incident(&detection, "pose_gap")[0];
    assert_eq!(gap.params["afterPose"], 12);
    assert_eq!(gap.params["beforePose"], 13);
    let mut all = night.input();
    all.target_folder = None;
    let everything = detect_incidents(&all);
    assert_eq!(everything.poses, 22);
    assert!(
        incident(&everything, "pose_gap").is_empty(),
        "the other target's poses fill the hole"
    );
}

#[test]
fn the_clock_of_the_logs_is_found_from_the_poses_and_the_ids_are_stable() {
    // The logs are in local time, UTC+2; nothing is forced.
    let begin = base() + 12.0 * 181.0 + 180.0 + 1.0;
    let end = begin + 680.0;
    let mut poses = run(0.0, 13, 180.0, 181.0);
    let gap_start = base() + 12.0 * 181.0 + 180.0;
    poses.extend(run(gap_start + 707.0 - base(), 8, 180.0, 181.0));
    let local = |t: f64| clock(t + 7200.0);
    let mut lines: Vec<String> = poses
        .iter()
        .take(13)
        .map(|p| format!("{} [Guide] Dither", local(base() + p.0 + p.1 + 1.0)))
        .collect();
    lines.push(format!("{} [Meridian Flip|Begin] Wait", local(begin)));
    lines.push(format!(
        "{} [Meridian Flip|End] Meridian Flip succeeded",
        local(end)
    ));
    let night = Night::new(&poses, Some(&lines.join("\n")), None);
    let mut input = night.input();
    input.log_offsets = ForcedLogOffsets::default();
    let first = detect_incidents(&input);
    let offsets = first.logs_utc_offset.as_ref().expect("the clocks are told");
    assert!(offsets.detected);
    assert_eq!(offsets.asiair.as_deref(), Some("+02:00"));
    assert_eq!(first.incidents.len(), 1);
    assert_eq!(first.incidents[0].start, iso_seconds(begin));
    // The same night read again gives the same keys.
    let again = detect_incidents(&input);
    assert_eq!(first.incidents, again.incidents);
}

#[test]
fn two_incidents_of_one_genre_in_one_second_keep_distinct_keys() {
    let make = |id: &str| DetectedIncident {
        id: id.to_string(),
        event_type: IncidentKind::EquipmentIssue,
        start: "2026-08-14T22:00:00Z".to_string(),
        end: None,
        code: "guide_star_lost".to_string(),
        params: json!({}),
        description: "x".to_string(),
        evidence: Evidence {
            source: EvidenceSource::Asiair,
            code: "x".to_string(),
            params: json!({}),
        },
        payload: json!({}),
    };
    let ids: Vec<String> = unique_ids(vec![
        make("equipment_issue:2026-08-14T22:00:00Z"),
        make("equipment_issue:2026-08-14T22:00:00Z"),
        make("equipment_issue:2026-08-14T22:00:00Z"),
    ])
    .into_iter()
    .map(|i| i.id)
    .collect();
    assert_eq!(
        ids,
        [
            "equipment_issue:2026-08-14T22:00:00Z",
            "equipment_issue:2026-08-14T22:00:00Z#2",
            "equipment_issue:2026-08-14T22:00:00Z#3"
        ]
    );
}

fn files_of(root: &std::path::Path) -> Vec<(String, Vec<u8>, std::time::SystemTime)> {
    let mut files: Vec<_> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            (
                e.path()
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                fs::read(e.path()).unwrap(),
                fs::metadata(e.path()).unwrap().modified().unwrap(),
            )
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

#[test]
fn reading_a_night_changes_no_file() {
    let (night, _, _) = sadr_flip_night();
    let before = files_of(&night.folder);
    let detection = detect_incidents(&night.input());
    assert!(!detection.incidents.is_empty());
    assert_eq!(before, files_of(&night.folder));
}

#[test]
fn a_folder_that_cannot_be_read_is_a_problem_not_a_panic() {
    let detection = detect_incidents(&DetectInput {
        night_folder: PathBuf::from("/nonexistent/astro-sync-incidents"),
        target_folder: None,
        window: None,
        log_offsets: ForcedLogOffsets::default(),
    });
    assert!(detection.incidents.is_empty());
    assert_eq!(detection.problems.len(), 1);
    assert_eq!(detection.problems[0].code, "analysis_failed");
}

// ---------------------------------------------------------------------------
// The event identifiers
// ---------------------------------------------------------------------------

#[test]
fn an_event_id_is_deterministic_per_session_and_key_and_in_its_own_key_space() {
    let key = "pause:2026-08-14T22:13:38Z";
    let id = event_id_for("4d2b7e10-8a6c-4f53-9b1d-7e0a3c5f2b84", key);
    assert_eq!(
        id,
        event_id_for("4d2b7e10-8a6c-4f53-9b1d-7e0a3c5f2b84", key)
    );
    assert_ne!(
        id,
        event_id_for("00000000-8a6c-4f53-9b1d-7e0a3c5f2b84", key)
    );
    assert_ne!(
        id,
        event_id_for(
            "4d2b7e10-8a6c-4f53-9b1d-7e0a3c5f2b84",
            "pause:2026-08-14T22:13:39Z"
        )
    );
    // `^[0-9A-HJKMNP-TV-Z]{26}$`
    assert_eq!(id.len(), 26);
    assert!(id.chars().all(
        |c| matches!(c, '0'..='9' | 'A'..='H' | 'J' | 'K' | 'M' | 'N' | 'P'..='T' | 'V'..='Z')
    ));
    // A generated ULID starts with `0` for as long as the clock reads a
    // year before 10889: this space starts with `7` and never meets it.
    assert!(id.starts_with(EVENT_ID_PREFIX));
    // A valid ULID: 128 bits, the first character at most `7`.
    assert!(id.chars().next().unwrap() <= '7');
}

// ---------------------------------------------------------------------------
// The declared signals
// ---------------------------------------------------------------------------

fn stored(id: &str, kind: &str, at: &str, payload: Value) -> Value {
    json!({
        "eventId": id,
        "sessionId": "4d2b7e10-8a6c-4f53-9b1d-7e0a3c5f2b84",
        "type": kind,
        "recordedAt": at,
        "receivedAt": "2026-08-15T07:00:00.000Z",
        "originDeviceId": null,
        "timing": "observed",
        "source": "declared",
        "payload": payload,
    })
}

/// The service, as the list sees it.
#[derive(Default)]
struct FakeService {
    calls: RefCell<Vec<String>>,
    pages: Vec<Value>,
    since_asked: RefCell<Vec<Option<String>>>,
    stored: RefCell<HashSet<String>>,
    rejects: HashMap<String, (String, String)>,
    appended: RefCell<Vec<Vec<Value>>>,
    revision: Option<u64>,
    put_refusal: Option<(String, String)>,
    puts: RefCell<Vec<(u64, Value)>>,
    fail_append_from_batch: Option<usize>,
    unreachable: bool,
}

impl IncidentsTransport for FakeService {
    fn events_page(
        &self,
        _: &str,
        _: &str,
        _: &str,
        since: Option<&str>,
    ) -> Result<Value, GuiBridgeError> {
        let index = self.since_asked.borrow().len();
        self.since_asked
            .borrow_mut()
            .push(since.map(str::to_string));
        self.calls.borrow_mut().push("page".to_string());
        Ok(self.pages[index.min(self.pages.len() - 1)].clone())
    }

    fn append_events(
        &self,
        _: &str,
        _: &str,
        _: &str,
        events: &[Value],
    ) -> Result<Value, GuiBridgeError> {
        self.calls.borrow_mut().push("append".to_string());
        let batch = self.appended.borrow().len();
        if self
            .fail_append_from_batch
            .is_some_and(|from| batch >= from)
        {
            return Err(GuiBridgeError::Transport("down".to_string()));
        }
        self.appended.borrow_mut().push(events.to_vec());
        let (mut accepted, mut duplicate, mut rejected) = (Vec::new(), Vec::new(), Vec::new());
        for event in events {
            let id = event["eventId"].as_str().unwrap().to_string();
            if let Some((code, message)) = self.rejects.get(&id) {
                rejected.push(json!({ "eventId": id, "code": code, "message": message }));
            } else if self.stored.borrow_mut().insert(id.clone()) {
                accepted.push(id);
            } else {
                duplicate.push(id);
            }
        }
        Ok(json!({ "accepted": accepted, "duplicate": duplicate, "rejected": rejected }))
    }

    fn rework_revision(&self, _: &str, _: &str, _: &str) -> Result<u64, GuiBridgeError> {
        self.calls.borrow_mut().push("revision".to_string());
        if self.unreachable {
            return Err(GuiBridgeError::Transport("down".to_string()));
        }
        Ok(self.revision.unwrap_or(0))
    }

    fn put_rework(
        &self,
        _: &str,
        _: &str,
        _: &str,
        revision: u64,
        body: &Value,
    ) -> Result<Value, GuiBridgeError> {
        self.calls.borrow_mut().push("put".to_string());
        self.puts.borrow_mut().push((revision, body.clone()));
        match &self.put_refusal {
            Some((code, message)) => Err(GuiBridgeError::Refused {
                code: code.clone(),
                message: message.clone(),
            }),
            None => Ok(json!({})),
        }
    }
}

const SESSION: &str = "4d2b7e10-8a6c-4f53-9b1d-7e0a3c5f2b84";

#[test]
fn the_declared_signals_are_read_as_the_service_gives_them_a_pause_with_its_resume() {
    // A signal a rework replaced: hidden by the service, and not read even
    // if one slips through.
    let mut old = stored(
        "E6",
        "condition",
        "2026-08-14T23:02:00.000Z",
        json!({ "sky": "clear" }),
    );
    old["rework"] = json!({ "added": false, "targetId": null, "replacedBy": "E7" });
    let mut new = stored(
        "E7",
        "condition",
        "2026-08-14T23:02:00.000Z",
        json!({ "sky": "hazy" }),
    );
    new["rework"] = json!({ "added": true, "targetId": null, "replaces": "E6" });
    new["timing"] = json!("reconstructed");
    let mut detected = stored(
        "E8",
        "guiding_timeouts",
        "2026-08-14T23:44:00.000Z",
        json!({ "count": 4, "firstAt": "2026-08-14T23:44:00Z", "lastAt": "2026-08-14T23:59:00Z" }),
    );
    detected["source"] = json!("astro-sync");
    let page = json!({
        "sessionId": SESSION,
        "events": [
            stored("E1", "target_started", "2026-08-14T21:31:00.000Z", json!({ "targetId": "sadr" })),
            stored("E2", "pause", "2026-08-14T22:14:00.000Z", json!({ "reason": "Flip" })),
            stored("E3", "resume", "2026-08-14T22:26:00.000Z", json!({})),
            stored("E4", "note", "2026-08-14T23:00:00.000Z", json!({ "text": "Dew", "targetId": "sadr" })),
            stored("E5", "a_type_of_a_newer_service", "2026-08-14T23:01:00.000Z", json!({ "x": 1 })),
            old,
            new,
            detected,
            stored("E9", "auto_time_confirmed", "2026-08-14T23:50:00.000Z", json!({ "targetId": "sadr" })),
        ],
        "hasMore": false,
        "nextSince": null,
    });
    let service = FakeService {
        pages: vec![page],
        ..FakeService::default()
    };
    let read = read_declared_events(&service, "https://s.example.test", "t", SESSION).unwrap();
    assert!(!read.truncated);
    let ids: Vec<&str> = read.events.iter().map(|e| e.event_id.as_str()).collect();
    assert_eq!(ids, ["E2", "E4", "E7", "E8"], "{ids:?}");
    let pause = &read.events[0];
    assert_eq!(pause.event_type, IncidentKind::Pause);
    assert_eq!(pause.until.as_deref(), Some("2026-08-14T22:26:00.000Z"));
    assert_eq!(pause.resume_event_id.as_deref(), Some("E3"));
    assert_eq!(pause.source.as_deref(), Some("declared"));
    assert_eq!(pause.timing.as_deref(), Some("observed"));
    assert_eq!(read.events[1].target_id.as_deref(), Some("sadr"));
    let replacement = &read.events[2];
    assert_eq!(
        replacement.rework.as_ref().unwrap().replaces.as_deref(),
        Some("E6")
    );
    assert_eq!(replacement.timing.as_deref(), Some("reconstructed"));
    assert_eq!(replacement.payload["sky"], "hazy");
    assert!(read.events[3].by_astro_sync());
}

#[test]
fn the_pages_are_followed_with_the_inclusive_cursor_and_a_stuck_cursor_says_the_read_is_short() {
    let one = stored(
        "A",
        "note",
        "2026-08-14T22:00:00.000Z",
        json!({ "text": "a" }),
    );
    let two = stored(
        "B",
        "note",
        "2026-08-14T22:10:00.000Z",
        json!({ "text": "b" }),
    );
    let pages = vec![
        json!({ "events": [one.clone()], "hasMore": true, "nextSince": "2026-08-15T07:00:00.000Z" }),
        // Inclusive: the last row comes again, then the new one.
        json!({ "events": [one.clone(), two.clone()], "hasMore": false, "nextSince": null }),
    ];
    let service = FakeService {
        pages,
        ..FakeService::default()
    };
    let read = read_declared_events(&service, "s", "t", SESSION).unwrap();
    assert_eq!(read.events.len(), 2);
    assert!(!read.truncated);
    assert_eq!(
        *service.since_asked.borrow(),
        [None, Some("2026-08-15T07:00:00.000Z".to_string())]
    );

    // A page that only repeats itself while saying there is more.
    let stuck = FakeService {
        pages: vec![
            json!({ "events": [one], "hasMore": true, "nextSince": "2026-08-15T07:00:00.000Z" }),
        ],
        ..FakeService::default()
    };
    let read = read_declared_events(&stuck, "s", "t", SESSION).unwrap();
    assert_eq!(read.events.len(), 1);
    assert!(read.truncated);
}

fn incident_of(start: &str, end: Option<&str>, kind: IncidentKind) -> DetectedIncident {
    DetectedIncident {
        id: format!("{}:{start}", kind.as_str()),
        event_type: kind,
        start: start.to_string(),
        end: end.map(str::to_string),
        code: "x".to_string(),
        params: json!({}),
        description: "detected".to_string(),
        evidence: Evidence {
            source: EvidenceSource::Fits,
            code: "x".to_string(),
            params: json!({}),
        },
        payload: json!({ "reason": "detected" }),
    }
}

#[test]
fn the_list_merges_by_time_pairs_the_close_ones_and_does_not_propose_what_is_already_added() {
    let declared = declared_from_events(&[
        stored(
            "P1",
            "pause",
            "2026-08-14T22:14:00.000Z",
            json!({ "reason": "Flip" }),
        ),
        stored("R1", "resume", "2026-08-14T22:26:00.000Z", json!({})),
        stored(
            "N1",
            "note",
            "2026-08-14T22:14:00.000Z",
            json!({ "text": "Dew" }),
        ),
    ]);
    let flip = incident_of(
        "2026-08-14T22:13:38Z",
        Some("2026-08-14T22:24:58Z"),
        IncidentKind::Pause,
    );
    let far = incident_of("2026-08-15T02:00:00Z", None, IncidentKind::EquipmentIssue);
    let already = incident_of("2026-08-15T03:00:00Z", None, IncidentKind::EquipmentIssue);
    // Astro Sync wrote `already` in an earlier sending.
    let mut written = stored(
        &event_id_for(SESSION, &already.id),
        "equipment_issue",
        &already.start,
        json!({ "description": "x" }),
    );
    written["source"] = json!("astro-sync");
    let mut declared = declared;
    declared.extend(declared_from_events(&[written]));

    let rows = merge_rows(SESSION, &declared, &[flip, far, already]);
    let order: Vec<(&str, RowOrigin)> = rows.iter().map(|r| (r.id.as_str(), r.origin)).collect();
    assert_eq!(
        order,
        [
            ("pause:2026-08-14T22:13:38Z", RowOrigin::Detected),
            ("N1", RowOrigin::Declared),
            ("P1", RowOrigin::Declared),
            ("equipment_issue:2026-08-15T02:00:00Z", RowOrigin::Detected),
            (
                &*event_id_for(SESSION, "equipment_issue:2026-08-15T03:00:00Z"),
                RowOrigin::Detected
            ),
        ]
    );
    // The pause pair points at each other; the note is another kind.
    assert_eq!(rows[0].near.as_deref(), Some("P1"));
    assert_eq!(rows[1].near, None);
    assert_eq!(rows[2].near.as_deref(), Some("pause:2026-08-14T22:13:38Z"));
    assert_eq!(rows[3].near, None);
    // The line already in the session is the stored one, not proposed twice.
    assert!(rows[4].already_added);
    assert_eq!(
        rows.iter()
            .filter(|r| r.text.as_deref() == Some("detected"))
            .count(),
        2
    );
}

// ---------------------------------------------------------------------------
// The writing
// ---------------------------------------------------------------------------

fn pause_add(key: &str, at: &str, until: &str) -> AddedIncident {
    AddedIncident {
        key: key.to_string(),
        event_type: IncidentKind::Pause,
        at: at.to_string(),
        until: Some(until.to_string()),
        payload: json!({ "reason": "Meridian flip", "cause": "meridian_flip" }),
    }
}

fn issue_add(key: &str, at: &str) -> AddedIncident {
    AddedIncident {
        key: key.to_string(),
        event_type: IncidentKind::EquipmentIssue,
        at: at.to_string(),
        until: None,
        payload: json!({ "description": "Guide star lost 2 times", "severity": "minor" }),
    }
}

fn replacement(event_id: &str) -> Replacement {
    Replacement {
        event_id: event_id.to_string(),
        event_type: IncidentKind::Pause,
        at: "2026-08-14T22:13:38Z".to_string(),
        until: Some("2026-08-14T22:24:58Z".to_string()),
        payload: json!({ "reason": "Flip \u{b7} detected: Meridian flip" }),
    }
}

#[test]
fn the_additions_go_first_then_the_replacements_with_the_revision_read_just_before() {
    let service = FakeService {
        revision: Some(4),
        ..FakeService::default()
    };
    let request = Apply {
        session_id: SESSION.to_string(),
        adds: vec![
            pause_add(
                "pause:2026-08-14T22:13:38Z",
                "2026-08-14T22:13:38Z",
                "2026-08-14T22:24:58Z",
            ),
            issue_add(
                "equipment_issue:2026-08-15T01:57:36Z",
                "2026-08-15T01:57:36Z",
            ),
        ],
        replacements: vec![replacement("DECLARED1")],
    };
    let outcome = apply(&service, "https://s.example.test", "t", &request).unwrap();
    assert_eq!(*service.calls.borrow(), ["append", "revision", "put"]);

    // The pause is the pair pause + resume, each dated at its own instant,
    // with identifiers of this module's key space.
    let sent = &service.appended.borrow()[0];
    assert_eq!(sent.len(), 3);
    let pause_id = event_id_for(SESSION, "pause:2026-08-14T22:13:38Z");
    assert_eq!(sent[0]["eventId"], pause_id);
    assert_eq!(sent[0]["type"], "pause");
    assert_eq!(sent[0]["recordedAt"], "2026-08-14T22:13:38Z");
    assert_eq!(sent[1]["type"], "resume");
    assert_eq!(sent[1]["recordedAt"], "2026-08-14T22:24:58Z");
    assert_ne!(sent[1]["eventId"], pause_id);
    assert!(sent[1]["eventId"].as_str().unwrap().starts_with('7'));
    assert_eq!(sent[1]["payload"], json!({}));
    assert_eq!(sent[2]["type"], "equipment_issue");
    for event in sent {
        // Nothing but the four members of the envelope.
        let keys: HashSet<&str> = event
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            HashSet::from(["eventId", "type", "recordedAt", "payload"])
        );
    }

    // The rework: the revision as read, all the replacements at once.
    let puts = service.puts.borrow();
    assert_eq!(puts.len(), 1);
    assert_eq!(puts[0].0, 4);
    assert_eq!(
        puts[0].1,
        json!({ "eventReplacements": [{
            "eventId": "DECLARED1",
            "event": {
                "type": "pause",
                "at": "2026-08-14T22:13:38Z",
                "until": "2026-08-14T22:24:58Z",
                "payload": { "reason": "Flip \u{b7} detected: Meridian flip" }
            }
        }] })
    );
    assert_eq!(outcome.added.len(), 2);
    assert_eq!(outcome.added[0].event_id, pause_id);
    assert!(outcome.added[0].resume_event_id.is_some());
    assert!(outcome.added[1].resume_event_id.is_none());
    assert_eq!(outcome.replaced, ["DECLARED1"]);
    assert!(outcome.replacements_refused.is_none());
    assert!(outcome.duplicate.is_empty() && outcome.rejected.is_empty());
}

#[test]
fn sending_the_same_list_again_writes_nothing_twice() {
    let service = FakeService::default();
    let request = Apply {
        session_id: SESSION.to_string(),
        adds: vec![
            pause_add(
                "pause:2026-08-14T22:13:38Z",
                "2026-08-14T22:13:38Z",
                "2026-08-14T22:24:58Z",
            ),
            issue_add(
                "equipment_issue:2026-08-15T01:57:36Z",
                "2026-08-15T01:57:36Z",
            ),
        ],
        replacements: Vec::new(),
    };
    let first = apply(&service, "s", "t", &request).unwrap();
    assert_eq!(first.added.len(), 2);
    let second = apply(&service, "s", "t", &request).unwrap();
    assert!(second.added.is_empty());
    assert_eq!(
        second.duplicate,
        [
            "pause:2026-08-14T22:13:38Z",
            "equipment_issue:2026-08-15T01:57:36Z"
        ]
    );
    // The same events, the same identifiers, both times; one stored copy.
    let sent = service.appended.borrow();
    assert_eq!(sent[0], sent[1]);
    assert_eq!(service.stored.borrow().len(), 3);
    // No replacement asked: the rework is not touched.
    assert!(
        !service
            .calls
            .borrow()
            .iter()
            .any(|c| c == "put" || c == "revision")
    );
    // Another session writes other events for the same key.
    assert_ne!(
        event_id_for(SESSION, "pause:2026-08-14T22:13:38Z"),
        event_id_for("another-session", "pause:2026-08-14T22:13:38Z")
    );
}

#[test]
fn an_addition_the_service_refuses_is_told_alone_and_the_others_are_kept() {
    let bad = issue_add(
        "equipment_issue:2026-08-15T01:57:36Z",
        "2026-08-15T01:57:36Z",
    );
    let good = pause_add(
        "pause:2026-08-14T22:13:38Z",
        "2026-08-14T22:13:38Z",
        "2026-08-14T22:24:58Z",
    );
    let mut rejects = HashMap::new();
    rejects.insert(
        event_id_for(SESSION, &bad.key),
        (
            "invalid_payload".to_string(),
            "description too long".to_string(),
        ),
    );
    let service = FakeService {
        rejects,
        ..FakeService::default()
    };
    let outcome = apply(
        &service,
        "s",
        "t",
        &Apply {
            session_id: SESSION.to_string(),
            adds: vec![bad.clone(), good],
            replacements: Vec::new(),
        },
    )
    .unwrap();
    assert_eq!(outcome.added.len(), 1);
    assert_eq!(outcome.added[0].key, "pause:2026-08-14T22:13:38Z");
    assert_eq!(
        outcome.rejected,
        [Rejection {
            key: bad.key,
            code: "invalid_payload".to_string(),
            message: "description too long".to_string(),
        }]
    );
}

#[test]
fn a_refused_rework_is_an_answer_never_an_error_and_the_additions_stay_told() {
    for (code, message) in [
        ("session_rework_conflict", "The session changed"),
        ("night_not_finished", "The night is not finished"),
        ("event_not_replaceable", "Not one of the four"),
    ] {
        let service = FakeService {
            revision: Some(2),
            put_refusal: Some((code.to_string(), message.to_string())),
            ..FakeService::default()
        };
        let outcome = apply(
            &service,
            "s",
            "t",
            &Apply {
                session_id: SESSION.to_string(),
                adds: vec![issue_add(
                    "equipment_issue:2026-08-15T01:57:36Z",
                    "2026-08-15T01:57:36Z",
                )],
                replacements: vec![replacement("M1"), replacement("M2")],
            },
        )
        .unwrap();
        assert_eq!(outcome.added.len(), 1, "{code}");
        assert!(outcome.replaced.is_empty(), "{code}: all or nothing");
        assert_eq!(
            outcome.replacements_refused,
            Some(ReplacementsRefused {
                code: code.to_string(),
                message: message.to_string(),
            })
        );
        // One call for both replacements.
        assert_eq!(service.puts.borrow().len(), 1);
    }
    // The revision cannot be read: the same answer, and no rework sent.
    let service = FakeService {
        unreachable: true,
        ..FakeService::default()
    };
    let outcome = apply(
        &service,
        "s",
        "t",
        &Apply {
            session_id: SESSION.to_string(),
            adds: vec![issue_add("k", "2026-08-15T01:57:36Z")],
            replacements: vec![replacement("M1")],
        },
    )
    .unwrap();
    assert_eq!(outcome.added.len(), 1);
    assert_eq!(outcome.replacements_refused.unwrap().code, "unreachable");
    assert!(service.puts.borrow().is_empty());
}

#[test]
fn a_wrong_request_is_refused_before_anything_is_sent() {
    let service = FakeService::default();
    let session = SESSION.to_string();
    let wrong = |adds: Vec<AddedIncident>, replacements: Vec<Replacement>| {
        apply(
            &service,
            "s",
            "t",
            &Apply {
                session_id: session.clone(),
                adds,
                replacements,
            },
        )
    };
    // An `until` on something that is not a pause.
    let mut issue = issue_add("k", "2026-08-15T01:57:36Z");
    issue.until = Some("2026-08-15T02:00:00Z".to_string());
    assert!(matches!(
        wrong(vec![issue], vec![]),
        Err(ApplyError::Invalid(_))
    ));
    // An end before the start.
    assert!(matches!(
        wrong(
            vec![pause_add(
                "k",
                "2026-08-15T02:00:00Z",
                "2026-08-15T01:00:00Z"
            )],
            vec![]
        ),
        Err(ApplyError::Invalid(_))
    ));
    // An instant that is not one.
    assert!(matches!(
        wrong(vec![issue_add("k", "yesterday evening")], vec![]),
        Err(ApplyError::Invalid(_))
    ));
    // More replacements than a rework takes.
    let many: Vec<Replacement> = (0..=MAX_REPLACEMENTS)
        .map(|i| replacement(&format!("D{i}")))
        .collect();
    assert!(matches!(wrong(vec![], many), Err(ApplyError::Invalid(_))));
    // A kind a rework does not replace.
    let mut spikes = replacement("M1");
    spikes.event_type = IncidentKind::GuidingSpikes;
    spikes.until = None;
    assert!(matches!(
        wrong(vec![], vec![spikes]),
        Err(ApplyError::Invalid(_))
    ));
    assert!(service.calls.borrow().is_empty(), "nothing was sent");
}

#[test]
fn the_additions_go_in_batches_of_five_hundred_and_a_pause_is_never_cut_from_its_resume() {
    let service = FakeService::default();
    // 300 pauses are 600 events: a batch of 250 pauses (500 events), then 50.
    let adds: Vec<AddedIncident> = (0..300_u32)
        .map(|i| {
            let at = iso_seconds(base() + f64::from(i) * 60.0);
            let until = iso_seconds(base() + f64::from(i) * 60.0 + 30.0);
            pause_add(&format!("pause:{at}"), &at, &until)
        })
        .collect();
    let outcome = apply(
        &service,
        "s",
        "t",
        &Apply {
            session_id: SESSION.to_string(),
            adds,
            replacements: Vec::new(),
        },
    )
    .unwrap();
    assert_eq!(outcome.added.len(), 300);
    let batches = service.appended.borrow();
    assert_eq!(batches.len(), 2);
    assert!(batches.iter().all(|b| b.len() <= MAX_ADDS_PER_BATCH));
    for batch in batches.iter() {
        let types: Vec<&str> = batch.iter().map(|e| e["type"].as_str().unwrap()).collect();
        assert_eq!(types.first(), Some(&"pause"));
        assert_eq!(types.last(), Some(&"resume"));
    }
}

#[test]
fn a_cut_between_two_batches_gives_what_was_answered_and_a_second_sending_finishes() {
    let service = FakeService {
        fail_append_from_batch: Some(1),
        ..FakeService::default()
    };
    let adds: Vec<AddedIncident> = (0..300_u32)
        .map(|i| {
            let at = iso_seconds(base() + f64::from(i) * 60.0);
            let until = iso_seconds(base() + f64::from(i) * 60.0 + 30.0);
            pause_add(&format!("pause:{at}"), &at, &until)
        })
        .collect();
    let request = Apply {
        session_id: SESSION.to_string(),
        adds,
        replacements: vec![replacement("M1")],
    };
    let Err(ApplyError::Service { partial, .. }) = apply(&service, "s", "t", &request) else {
        panic!("the second batch could not be sent");
    };
    assert_eq!(partial.added.len(), 250);
    assert!(
        !service.calls.borrow().iter().any(|c| c == "put"),
        "no rework after a cut"
    );
    // The service is back: the same list, the first batch is a duplicate.
    let back = FakeService {
        stored: RefCell::new(service.stored.borrow().clone()),
        revision: Some(1),
        ..FakeService::default()
    };
    let done = apply(&back, "s", "t", &request).unwrap();
    assert_eq!(done.duplicate.len(), 250);
    assert_eq!(done.added.len(), 50);
    assert_eq!(done.replaced, ["M1"]);
}

// ---------------------------------------------------------------------------
// The text of a replaced signal
// ---------------------------------------------------------------------------

#[test]
fn the_text_of_the_person_stays_in_front_of_the_detected_one() {
    assert_eq!(
        merged_text("Nuage de passage", "Meridian flip", 200),
        "Nuage de passage \u{b7} detected: Meridian flip"
    );
    assert_eq!(merged_text("  ", "Meridian flip", 200), "Meridian flip");
    assert_eq!(merged_text("Mine", "", 200), "Mine");
    assert_eq!(merged_text("", "", 200), "");
    assert_eq!(
        merged_text_with("Mine", "Detected", " \u{b7} detected: ", 200),
        "Mine \u{b7} detected: Detected"
    );
}

#[test]
fn a_text_too_long_is_cut_on_a_character_never_in_a_code_point() {
    let user = "\u{e9}\u{1f31f}".repeat(10); // 20 characters, 40 bytes
    let detected = "\u{4e2d}".repeat(50);
    let merged = merged_text(&user, &detected, 30);
    assert_eq!(merged.chars().count(), 30);
    assert!(merged.ends_with('\u{2026}'));
    // The person's text is whole at the head.
    assert!(merged.starts_with(&user));
    // Every cut length is valid UTF-8 and within the limit.
    for max in 0..60 {
        let text = merged_text(&user, &detected, max);
        assert!(text.chars().count() <= max, "{max}");
    }
    // Under the limit: untouched, no ellipsis.
    let exact = merged_text("a", "b", 2 + DETECTED_SEPARATOR.chars().count());
    assert!(!exact.ends_with('\u{2026}'));
    // A person's text alone longer than the limit is cut too.
    assert_eq!(merged_text(&"x".repeat(250), "y", 200).chars().count(), 200);
}

#[test]
fn the_replacing_payload_keeps_the_person_s_text_first_and_what_only_they_said() {
    let detected = json!({ "reason": "Meridian flip", "cause": "meridian_flip" });
    let merged = merged_payload(
        IncidentKind::Pause,
        &json!({ "reason": "Flip", "cause": "equipment" }),
        &detected,
    );
    assert_eq!(
        merged,
        json!({ "reason": "Flip \u{b7} detected: Meridian flip", "cause": "meridian_flip" })
    );
    let issue = merged_payload(
        IncidentKind::EquipmentIssue,
        &json!({ "description": "Mount stalled", "equipmentId": "eq-1", "severity": "blocking" }),
        &json!({ "description": "Guide star lost 2 times", "severity": "minor" }),
    );
    assert_eq!(issue["equipmentId"], "eq-1");
    assert_eq!(issue["severity"], "minor");
    assert!(
        issue["description"]
            .as_str()
            .unwrap()
            .starts_with("Mount stalled")
    );
    // 200 characters for the reason of a pause.
    let long = merged_payload(
        IncidentKind::Pause,
        &json!({ "reason": "r".repeat(190) }),
        &json!({ "reason": "d".repeat(190) }),
    );
    assert_eq!(long["reason"].as_str().unwrap().chars().count(), 200);
}

#[test]
fn the_kinds_are_the_registry_s_and_a_stranger_is_none() {
    for kind in IncidentKind::ALL {
        assert_eq!(IncidentKind::parse(kind.as_str()), Some(kind));
    }
    assert_eq!(IncidentKind::parse("resume"), None);
    assert_eq!(IncidentKind::parse("target_started"), None);
    assert!(IncidentKind::Pause.replaceable() && !IncidentKind::GuidingSpikes.replaceable());
    assert_eq!(
        serde_json::to_value(IncidentKind::SensorTemperatureDrift).unwrap(),
        json!("sensor_temperature_drift")
    );
}

#[test]
fn a_detected_incident_becomes_the_addition_that_sends_its_exact_payload() {
    let (night, begin, end) = sadr_flip_night();
    let detection = detect_incidents(&night.input());
    let added = detection.incidents[0].as_added();
    assert_eq!(added.key, detection.incidents[0].id);
    assert_eq!(added.at, iso_seconds(begin));
    assert_eq!(added.until.as_deref(), Some(iso_seconds(end).as_str()));
    assert_eq!(added.payload, detection.incidents[0].payload);
    // Only a pause has an `until`.
    let lost = lost_night(&[(600.0, "[Guide] Guide star lost")]);
    let found = detect_incidents(&lost.input());
    assert_eq!(found.incidents[0].as_added().until, None);
}

// ---------------------------------------------------------------------------
// The real nights: the timelines of the Seestar night of M 45 and of four
// Askar 50P nights, tuned there. The fixture holds the instant, exposure and
// sensor temperature of each pose, never a pixel.
// ---------------------------------------------------------------------------

/// The poses of a night of the fixture; a cooled camera writes its set point.
fn real_poses(key: &str) -> Vec<Pose> {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/real_nights.json")).unwrap();
    let night = &fixture[key];
    let first = parse_iso8601_utc_to_unix_seconds(night["firstPoseUtc"].as_str().unwrap()).unwrap();
    let set_temperature = night["setTemperature"].as_f64();
    night["poses"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(index, row)| Pose {
            rank: index + 1,
            start: first + row[0].as_f64().unwrap(),
            exposure: row[1].as_f64().unwrap(),
            temperature: row[2].as_f64(),
            set_temperature,
        })
        .collect()
}

/// `(after, before, seconds)` of each pause of frames the poses give.
fn pauses(poses: &[Pose], flips: &[Flip]) -> Vec<(u64, u64, u64)> {
    let mut unproposed = Vec::new();
    pause_incidents(poses, flips, &mut unproposed)
        .iter()
        .filter(|p| p.code == "pose_gap")
        .map(|p| {
            (
                p.params["afterPose"].as_u64().unwrap(),
                p.params["beforePose"].as_u64().unwrap(),
                p.params["gapSeconds"].as_u64().unwrap(),
            )
        })
        .collect()
}

#[test]
fn the_repeated_recentring_of_the_seestar_night_is_no_pause_and_its_two_long_stops_are() {
    let poses = real_poses("seestar_m45");
    assert_eq!(poses.len(), 466);
    // Three gaps of 3 min 21 s, 3 min 41 s and 3 min 39 s (the same step of
    // the sequence, repeated) were pauses; 54 min and 36 min are.
    assert_eq!(pauses(&poses, &[]), vec![(28, 29, 3256), (270, 271, 2139)]);
}

#[test]
fn a_three_minute_gap_the_night_does_not_repeat_is_a_pause_on_a_night_of_longer_frames() {
    // Eleven minutes of the Askar night of Sadr (the flip, seen without its
    // log): still a pause of frames; the dither (65 s) and its rhythm are not.
    let poses = real_poses("sadr_2026_08_14");
    assert_eq!(pauses(&poses, &[]), vec![(13, 14, 707)]);
    // Mu Cephei 19/20 August: the long gaps of five and three and a half
    // minutes and the flip are all told, as before.
    let poses = real_poses("mu_cephei_2026_08_19");
    assert_eq!(
        pauses(&poses, &[]),
        vec![(52, 53, 298), (56, 57, 773), (99, 100, 209)]
    );
}

#[test]
fn an_autofocus_of_about_three_minutes_that_comes_back_is_no_pause() {
    // Voile 12/13 August: the 835 s flip is a pause; the two gaps of 182 and
    // 176 s (an autofocus, a restart of the plan), the same step twice or
    // more in the night, are not any more.
    let poses = real_poses("voile_2026_08_12");
    assert_eq!(pauses(&poses, &[]), vec![(12, 13, 835)]);
    // SH2-190 12 August (120 s frames): the autofocus of 182 s on a night of
    // 65 s dithers is not a pause either.
    let poses = real_poses("sh2_190_2026_08_12");
    assert_eq!(pauses(&poses, &[]), vec![]);
}

#[test]
fn a_gap_that_repeats_is_routine_only_up_to_ten_frame_cycles() {
    // 40 poses of 30 s, one every 33 s; three gaps of 200 s (six cycles:
    // routine), then three of 400 s (twelve cycles: three interruptions).
    let mut poses = Vec::new();
    let mut at = 0.0;
    for rank in 1..=40 {
        poses.push(pose(rank, at, 30.0));
        at += 33.0;
        if [10, 20, 30].contains(&rank) {
            at += 200.0;
        }
    }
    assert_eq!(pauses(&poses, &[]), vec![]);
    let mut at = 0.0;
    for (index, p) in poses.iter_mut().enumerate() {
        p.start = at;
        at += 33.0;
        if [10, 20, 30].contains(&(index + 1)) {
            at += 400.0;
        }
    }
    assert_eq!(
        pauses(&poses, &[]),
        vec![(10, 11, 403), (20, 21, 403), (30, 31, 403)]
    );
    // Two repeats of 200 s are not enough to be routine.
    let mut at = 0.0;
    for (index, p) in poses.iter_mut().enumerate() {
        p.start = at;
        at += 33.0;
        if [10, 20].contains(&(index + 1)) {
            at += 200.0;
        }
    }
    assert_eq!(pauses(&poses, &[]), vec![(10, 11, 203), (20, 21, 203)]);
}

#[test]
fn the_sensor_that_follows_the_air_is_no_drift_and_a_cooler_that_lets_go_is() {
    // The Seestar (no set point; 23.8 C then 17.2 C over the night).
    let poses = real_poses("seestar_m45");
    assert!(poses.iter().all(|p| p.set_temperature.is_none()));
    assert!(temperature_drift_incidents(&poses).is_empty());
    // The cooled nights hold their set point.
    for key in [
        "sadr_2026_08_14",
        "voile_2026_08_12",
        "mu_cephei_2026_08_19",
    ] {
        assert!(
            temperature_drift_incidents(&real_poses(key)).is_empty(),
            "{key}"
        );
    }
    // The same Sadr night, the last quarter with the cooler off: -10 C going
    // up to -4 C: from the last pose still within 3 degrees of the set point
    // (the 100th) to the warmest.
    let mut poses = real_poses("sadr_2026_08_14");
    for (i, p) in poses.iter_mut().enumerate().skip(86) {
        p.temperature = Some(-10.0 + (i - 85) as f64 * 0.21);
    }
    let drift = temperature_drift_incidents(&poses);
    assert_eq!(drift.len(), 1);
    assert_eq!(
        drift[0].params,
        json!({ "fromCelsius": -10.0, "toCelsius": -4.1 })
    );
    assert_eq!(drift[0].start, iso_seconds(poses[99].start));
    assert_eq!(
        drift[0].end.as_deref(),
        Some(iso_seconds(poses[113].start).as_str())
    );
    // Without a set point in the headers, a sensor that held its temperature
    // all the night is cooled all the same; one that follows the air is not.
    let mut cooled = poses.clone();
    cooled.iter_mut().for_each(|p| p.set_temperature = None);
    assert_eq!(temperature_drift_incidents(&cooled).len(), 1);
    let mut air = real_poses("seestar_m45");
    air.iter_mut().for_each(|p| p.set_temperature = None);
    assert!(temperature_drift_incidents(&air).is_empty());
}

/// The poses of a camera RAW night are read like FITS ones, so a hole in
/// the rhythm is a pause and a change of the exposure is told.
#[test]
fn the_poses_of_a_camera_raw_night_make_the_same_incidents() {
    use crate::raw_header::fixture::{Shot, write_raw};
    let night = Night::new(&[], None, None);
    let lights = night.folder.join("Sadr/lights");
    // 20 poses of 180 s, one every 181 s, with a hole of 400 s after the 9th
    // (a cloud); the camera says its zone, so its clock is UTC.
    let mut at = 0.0;
    for index in 0..20 {
        if index == 9 {
            at += 400.0;
        }
        let taken = crate::time_util::format_iso8601_utc_millis(base() + at)
            .replace('-', ":")
            .replace('T', " ")[..19]
            .to_string();
        let mut shot = Shot::new(&taken, 180, 800);
        shot.offset = Some("+00:00");
        // The exposure changes for the last poses.
        if index >= 17 {
            shot.exposure = (120, 1);
        }
        write_raw(&lights.join(format!("DSC_{index:04}.NEF")), &shot);
        at += 181.0;
    }
    let detection = detect_incidents(&night.input());
    assert_eq!(detection.poses, 20);
    let gaps = incident(&detection, "pose_gap");
    assert_eq!(gaps.len(), 1, "{:#?}", detection.incidents);
    // The hole of 400 s and the second the camera loses between two poses.
    assert_eq!(gaps[0].params["gapSeconds"], 401);
    assert_eq!(gaps[0].params["afterPose"], 9);
    assert!(
        detection
            .incidents
            .iter()
            .any(|i| i.event_type == IncidentKind::ExposureChange),
        "{:#?}",
        detection.incidents
    );
}

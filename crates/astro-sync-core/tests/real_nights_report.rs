//! The report v3 on real nights, when this computer holds them: the nights
//! of the working folder `ASTRO_SYNC_REAL_NIGHTS` names (an Astro Sync
//! working folder: one folder per night), read only, each night a folder
//! with a `logs/` holding a PHD2 and an ASIAIR log. Without the variable
//! (any other computer), the test says so and passes: no real log, place or
//! frame is committed.

#[path = "support/schema_check.rs"]
mod schema_check;

use std::path::PathBuf;

use astro_sync_core::analysis::log_offset::ForcedLogOffsets;
use astro_sync_core::analysis::report::render_report_json;
use astro_sync_core::night_analysis::analyze_night_detecting;

fn real_nights() -> Vec<PathBuf> {
    let Some(root) = std::env::var_os("ASTRO_SYNC_REAL_NIGHTS").map(PathBuf::from) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut nights: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            let Ok(logs) = std::fs::read_dir(path.join("logs")) else {
                return false;
            };
            let names: Vec<String> = logs
                .flatten()
                .map(|l| l.file_name().to_string_lossy().to_ascii_lowercase())
                .collect();
            names.iter().any(|n| n.starts_with("phd2_guidelog"))
                && names.iter().any(|n| n.starts_with("autorun_log"))
        })
        .collect();
    nights.sort();
    nights
}

/// `YYYY-MM-DDTHH:MM:SS.sssZ` in seconds since 1970 (the days by the civil
/// calendar's own count).
fn seconds_of(at: &str) -> f64 {
    let number = |range: std::ops::Range<usize>| at[range].parse::<i64>().unwrap();
    let (year, month, day) = (number(0..4), number(5..7), number(8..10));
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let clock: f64 = at[17..at.len() - 1].parse().unwrap();
    (days * 86_400 + number(11..13) * 3600 + number(14..16) * 60) as f64 + clock
}

#[test]
fn the_real_nights_give_a_light_report_v3_on_one_clock_with_one_dither_per_dither() {
    let nights = real_nights();
    if nights.is_empty() {
        eprintln!("no real night on this computer: nothing to read");
        return;
    }
    for (index, night) in nights.iter().enumerate() {
        // The folder's name holds the place: the messages name the night by
        // its rank only.
        let label = format!("night {}", index + 1);
        let analysis = analyze_night_detecting(night, &ForcedLogOffsets::default(), None)
            .unwrap_or_else(|e| panic!("{label}: {e}"));
        let report = analysis.report;
        let rendered = render_report_json(&report);
        let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        schema_check::assert_valid("session-analysis.schema.json", "report", &value);
        assert_eq!(report.report_version, 3, "{label}");
        // Lighter than the raw samples of the version 2 (1.8 to 3.8 MB).
        assert!(
            rendered.len() < 600_000,
            "{label}: {} bytes",
            rendered.len()
        );
        let trace = report.guiding_trace.as_ref().expect("a PHD2 log");
        // At most one point per 10 s.
        assert!(
            trace.points.windows(2).all(|w| w[1].t >= w[0].t + 10),
            "{label}"
        );
        // The PHD2 of an ASIAIR runs on the ASIAIR's clock.
        let clock = report.log_clock.as_ref().unwrap();
        assert_eq!(clock.phd2, clock.asiair, "{label}");
        // One dither per dither: no ASIAIR dither within 30 s of PHD2's.
        let dithers: Vec<(&str, f64)> = report
            .timeline
            .events
            .iter()
            .filter(|e| e.event_type == "dither")
            .map(|e| (e.source.as_str(), seconds_of(&e.timestamp_utc)))
            .collect();
        for (source, at) in &dithers {
            if *source == "asiair" {
                assert!(
                    !dithers
                        .iter()
                        .any(|(s, t)| *s == "phd2" && (t - at).abs() <= 30.0),
                    "{label}: a dither told twice"
                );
            }
        }
        let rms = trace.rms.total_arcsec.expect("a guided night");
        assert!((0.2..3.0).contains(&rms), "{label}: RMS {rms}");
        eprintln!(
            "{label}: {} bytes, {} points, {} markers, RMS {rms}\u{2033}, clock {:?} ({:?})",
            rendered.len(),
            trace.points.len(),
            trace.markers.len(),
            clock.phd2,
            clock.phd2_from
        );
    }
}

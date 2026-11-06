//! Fixture-backed tests for `analysis::report`: the session-analysis report,
//! generated over a folder built from the exact same committed synthetic
//! fixtures `analysis_fixtures.rs` already uses
//! (`tests/fixtures/phd2_guide_log_short.txt`,
//! `tests/fixtures/asiair_autorun_log_short.txt`), plus one synthetic FITS
//! header built the same mechanical way that file's own
//! `build_fits_header_bytes` does (duplicated here for the same reason: a
//! private `#[cfg(test)]` helper of a different compilation unit).
//!
//! Covers directly: deterministic rendering, no composite score anywhere in the
//! rendered JSON, and traceability of every quality/guiding row and timeline
//! boundary back to a source file, frame, or FITS keyword.

use astro_sync_core::analysis::AnalysisOptions;
use astro_sync_core::analysis::report::{generate_report, render_report_json};
use std::fs;
use std::sync::{Mutex, MutexGuard, PoisonError};

const PHD2_FIXTURE: &str = include_str!("fixtures/phd2_guide_log_short.txt");
const ASIAIR_FIXTURE: &str = include_str!("fixtures/asiair_autorun_log_short.txt");

/// This integration test binary has no access to `astro_sync_core`'s own
/// private `time_util::DETERMINISTIC_TIME_ENV_LOCK` (`time_util` is not a
/// public module -- see that item's own doc comment for why a shared lock
/// is needed at all), so it rolls its own local equivalent, on the same
/// doctrine as `ScratchDir` below duplicating `test_support::TempDir`.
/// Every test in this file calls [`generate_report`], which reads
/// `ASTRO_SYNC_DETERMINISTIC_TIME` (`time_util::now_iso8601_utc`); exactly
/// one of them ([`a_report_generated_from_the_committed_9_2_fixtures_is_deterministic`])
/// also sets/clears it. `std::env::set_var`/`remove_var` mutate
/// process-wide state and are unsound to race against a concurrent read or
/// write from another thread -- Rust's default test harness runs this
/// binary's `#[test]` functions concurrently -- so every test here takes
/// this lock for its entire critical section, not only the one that
/// mutates the variable.
static DETERMINISTIC_TIME_ENV_LOCK: Mutex<()> = Mutex::new(());

fn lock_deterministic_time_env_for_test() -> MutexGuard<'static, ()> {
    DETERMINISTIC_TIME_ENV_LOCK
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

fn build_fits_header_bytes(cards: &[(&str, &str)]) -> Vec<u8> {
    const CARD_LEN: usize = 80;
    const BLOCK_LEN: usize = 2880;
    let mut out = Vec::new();
    for (keyword, value) in cards {
        let card = format!("{keyword:<8}= {value}");
        let mut bytes = card.into_bytes();
        bytes.resize(CARD_LEN, b' ');
        out.extend_from_slice(&bytes);
    }
    let mut end_card = b"END".to_vec();
    end_card.resize(CARD_LEN, b' ');
    out.extend_from_slice(&end_card);
    let remainder = out.len() % BLOCK_LEN;
    if remainder != 0 {
        out.resize(out.len() + (BLOCK_LEN - remainder), b' ');
    }
    out
}

/// A fresh directory under the system temp dir, removed on drop -- this
/// integration test crate has no access to `astro_sync_core`'s own private
/// `test_support::TempDir`, so it rolls its own minimal equivalent.
struct ScratchDir(std::path::PathBuf);

impl ScratchDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "astro-sync-core-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_fixture_folder() -> ScratchDir {
    let dir = ScratchDir::new("analysis-report-fixtures");
    fs::write(dir.path().join("PHD2_GuideLog_short.txt"), PHD2_FIXTURE).unwrap();
    fs::write(dir.path().join("Autorun_Log_short.txt"), ASIAIR_FIXTURE).unwrap();
    fs::write(
        dir.path().join("frame-0001.fits"),
        build_fits_header_bytes(&[
            ("EXPTIME", "300.0"),
            ("FILTER", "'Ha      '"),
            ("GAIN", "100"),
            ("CCD-TEMP", "-10.0"),
            ("DATE-OBS", "'2030-03-01T22:00:00.000'"),
            ("XBINNING", "1"),
            ("YBINNING", "1"),
        ]),
    )
    .unwrap();
    dir
}

#[test]
fn a_report_generated_from_the_committed_9_2_fixtures_is_deterministic() {
    let _guard = lock_deterministic_time_env_for_test();
    let dir = write_fixture_folder();
    // SAFETY (test-only): `ASTRO_SYNC_DETERMINISTIC_TIME` is the crate's
    // own documented escape hatch for exactly this
    // (`time_util::DETERMINISTIC_TIME_ENV_VAR`). The lock acquired above
    // -- held by every test in this file, not only this one -- is what
    // rules out a concurrent read/write from another thread; Rust's
    // default test harness is not single-threaded.
    unsafe {
        std::env::set_var("ASTRO_SYNC_DETERMINISTIC_TIME", "2030-03-02T00:00:00Z");
    }
    let first = render_report_json(
        &generate_report(dir.path(), &AnalysisOptions::default())
            .expect("the committed fixtures are well-formed"),
    );
    let second = render_report_json(
        &generate_report(dir.path(), &AnalysisOptions::default())
            .expect("the committed fixtures are well-formed"),
    );
    unsafe {
        std::env::remove_var("ASTRO_SYNC_DETERMINISTIC_TIME");
    }
    assert_eq!(
        first, second,
        "two reports generated from byte-identical fixtures must be byte-identical"
    );
}

#[test]
fn a_report_generated_from_the_committed_9_2_fixtures_carries_the_expected_chronology() {
    let _guard = lock_deterministic_time_env_for_test();
    let dir = write_fixture_folder();
    let report = generate_report(dir.path(), &AnalysisOptions::default())
        .expect("the committed fixtures are well-formed");

    assert_eq!(report.report_version, 3);
    assert_eq!(report.source.phd2_files_parsed, 1);
    assert_eq!(report.source.asiair_files_parsed, 1);
    assert_eq!(report.source.fits_files_parsed, 1);

    // The PHD2 fixture's own "Guiding Begins at"/"Guiding Ends at" lines
    // (analysis_fixtures.rs asserts the same values against
    // `phd2::parse_phd2_guide_log` directly).
    let started = report
        .timeline
        .started_at
        .as_ref()
        .expect("the PHD2 fixture states a start");
    let ended = report
        .timeline
        .ended_at
        .as_ref()
        .expect("the PHD2 fixture states an end");
    assert_eq!(started.timestamp_utc, "2030-03-01T22:00:00.000Z");
    assert_eq!(ended.timestamp_utc, "2030-03-01T22:41:12.000Z");

    // The fixture's own event vocabulary (settling, a lost star, dithering,
    // an autofocus run, a meridian flip) all surface in the timeline.
    let event_types: Vec<&str> = report
        .timeline
        .events
        .iter()
        .map(|event| event.event_type.as_str())
        .collect();
    for expected in [
        "dither-settle",
        "star-lost",
        "dither",
        "settle-timeout",
        "autofocus",
        "autofocus-result",
        "meridian-flip",
    ] {
        assert!(
            event_types.contains(&expected),
            "expected a {expected:?} event, got {event_types:?}"
        );
    }
}

#[test]
fn every_value_in_the_report_traces_to_a_source_file_frame_or_fits_keyword() {
    let _guard = lock_deterministic_time_env_for_test();
    let dir = write_fixture_folder();
    let report = generate_report(dir.path(), &AnalysisOptions::default())
        .expect("the committed fixtures are well-formed");

    assert!(!report.quality.is_empty());
    for row in &report.quality {
        assert!(
            !row.source_path.is_empty(),
            "a quality row must trace to its source FITS file"
        );
    }

    // The guiding is the trace of the PHD2 log's own samples: every point
    // counts samples, none is invented.
    let trace = report.guiding_trace.as_ref().expect("the PHD2 fixture");
    assert!(!trace.points.is_empty());
    assert!(
        trace
            .points
            .iter()
            .all(|point| point.n > 0 && point.out <= point.n)
    );

    let started = report.timeline.started_at.as_ref().unwrap();
    let ended = report.timeline.ended_at.as_ref().unwrap();
    assert!(!started.evidence.is_empty());
    assert!(!ended.evidence.is_empty());

    for step in &report.acquisition_steps {
        assert!(
            step.frame_count >= 1,
            "an acquisition step counts the frames it summarizes"
        );
        assert!(
            report
                .quality
                .iter()
                .any(|row| row.step_key.as_deref() == Some(step.key.as_str())),
            "every step is the step of at least one quality row"
        );
    }
}

#[test]
fn the_rendered_report_never_carries_a_composite_score() {
    let _guard = lock_deterministic_time_env_for_test();
    const FORBIDDEN: [&str; 3] = ["score", "rating", "grade"];
    let dir = write_fixture_folder();
    let report = generate_report(dir.path(), &AnalysisOptions::default())
        .expect("the committed fixtures are well-formed");
    let rendered = render_report_json(&report);
    let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    assert_no_forbidden_keys(&value, &FORBIDDEN);
}

fn assert_no_forbidden_keys(value: &serde_json::Value, forbidden: &[&str]) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, nested) in map {
                let lower = key.to_ascii_lowercase();
                assert!(
                    !forbidden.iter().any(|f| lower.contains(f)),
                    "JSON key {key:?} looks like a composite score"
                );
                assert_no_forbidden_keys(nested, forbidden);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                assert_no_forbidden_keys(item, forbidden);
            }
        }
        _ => {}
    }
}

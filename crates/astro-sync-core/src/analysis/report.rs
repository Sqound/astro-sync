//! The night's report, version 3 (the service still reads the version 2
//! already sent): the shape
//! `POST /v1/sessions/{id}/analyses` accepts
//! (`contracts/schemas/api/session-analysis.schema.json`,
//! `$defs.report`). One analysis per night: the next sending replaces it,
//! and a report holding nothing is never sent ([`SessionReport::is_empty`];
//! the service refuses it too, `422 analysis_empty`).
//!
//! [`generate_report`] runs [`super::analyze_folder`] and reshapes its
//! [`super::AnalysisOutcome`]:
//!
//! - `acquisitionSteps[]`: the light frames grouped by target, filter,
//!   exposure and gain ([`super::acquisition_steps`]), the target hint being
//!   the header's `OBJECT` or the NINA target window of the frame; a night
//!   known only from NINA's log takes its steps from the frames NINA started;
//! - `quality[]`: one row per light frame, with its step (`stepKey`), the
//!   measures of « Mesure des poses » when the frame was measured (FWHM,
//!   eccentricity, stars, sky), and whether the stacking left it out
//!   (`rejected`, `rejectReason`: `unregistered`, or the measure's reason for
//!   a pose left out by « Écarter les poses signalées »); calibration frames
//!   are left out;
//! - `guidingTrace`: the night's guiding reduced to one point per 10 s with
//!   the corrections and the markers ([`super::guiding_trace`]) -- the raw
//!   samples of the version 2 (`guiding[]`) are no longer sent;
//!   `guidingSeries[]` the slices of two minutes ([`super::guiding_series`]);
//! - `timeline`: the night's start and end, each with its evidence, and the
//!   events of PHD2, ASIAIR and NINA, one dither per dither;
//! - `logClock`: the clocks the logs were read with and how they were known;
//! - `pixelScaleArcsec`: the median of the frames' scales from their headers.
//!
//! The raw logs never leave the computer: only these values do.
//!
//! **Evidence rule**: every value traces to a value a parser read; no field
//! named or containing `score`, `rating` or `grade` exists anywhere
//! (`report_never_produces_a_composite_score`, below, walks the rendered
//! JSON).

use super::acquisition_steps::{self, AcquisitionStep, StepFrame};
use super::guiding_series::{GuidingSlice, guiding_series};
use super::guiding_trace::{GuidingTrace, guiding_trace, one_dither_per_dither};
use super::log_offset::LogsUtcOffset;
use super::{AnalysisError, AnalysisOptions, AnalysisOutcome, GuidingRow, SessionEvent};
use crate::canonical_json::canonical_json;
use crate::pose_stats::record::{self as pose_record, NightPoseRecords};
use crate::time_util::{
    format_iso8601_utc_millis, now_iso8601_utc, parse_iso8601_utc_to_unix_seconds,
};
use serde::Serialize;
use std::path::Path;

/// `reportVersion` of the rendered JSON (the contract accepts 2 and 3).
pub const REPORT_VERSION: u32 = 3;

/// `producer`: who built the report.
pub const PRODUCER: &str = "astro-sync";

/// Exactly which parsed value a [`Timeline`] boundary came from.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineBoundary {
    pub timestamp_utc: String,
    pub evidence: String,
}

/// The night's own chronology and its events, in order.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timeline {
    pub started_at: Option<TimelineBoundary>,
    pub ended_at: Option<TimelineBoundary>,
    pub events: Vec<SessionEvent>,
}

/// How many files of each kind fed the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceCounts {
    pub phd2_files_parsed: usize,
    pub asiair_files_parsed: usize,
    pub fits_files_parsed: usize,
    pub nina_files_parsed: usize,
}

/// One measured light frame (`qualityRow` of the contract).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportQualityRow {
    pub timestamp_utc: String,
    pub step_key: Option<String>,
    pub filter: Option<String>,
    pub exposure_seconds: Option<f64>,
    pub gain: Option<f64>,
    pub iso: Option<f64>,
    pub binning: Option<String>,
    pub sensor_temp_c: Option<f64>,
    pub rms_total_arcsec: Option<f64>,
    pub rms_ra_arcsec: Option<f64>,
    pub rms_dec_arcsec: Option<f64>,
    pub guiding_sample_count: Option<u64>,
    pub sky_background_adu: Option<f64>,
    pub fwhm_pixels: Option<f64>,
    pub eccentricity: Option<f64>,
    /// Stars the measure found on the frame.
    pub star_count: Option<u64>,
    /// The stacking left the frame out.
    pub rejected: bool,
    /// Why: `unregistered` (the stacking could not align it), or the reason
    /// the measure gave a pose the person left out (`dawn`, `sky`, `clouds`,
    /// `trailed`, `wide`; `manual` without one).
    pub reject_reason: Option<String>,
    pub source_path: String,
}

/// The clocks the logs were read with (`+HH:MM`, a log's wall clock east of
/// UTC), and how the PHD2 one was known (`given`, `asiair`, `frames`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogClock {
    pub phd2: Option<String>,
    pub asiair: Option<String>,
    pub nina: Option<String>,
    pub detected: bool,
    pub phd2_from: Option<String>,
}

impl From<&LogsUtcOffset> for LogClock {
    fn from(offsets: &LogsUtcOffset) -> Self {
        Self {
            phd2: offsets.phd2.clone(),
            asiair: offsets.asiair.clone(),
            nina: offsets.nina.clone(),
            detected: offsets.detected,
            phd2_from: offsets.phd2_from.clone(),
        }
    }
}

/// The report v3. Serialize with [`render_report_json`] (canonical JSON)
/// before sending it or writing it to disk.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionReport {
    pub report_version: u32,
    pub producer: String,
    pub generated_at_utc: String,
    pub source: SourceCounts,
    pub pixel_scale_arcsec: Option<f64>,
    pub timeline: Timeline,
    pub acquisition_steps: Vec<AcquisitionStep>,
    pub quality: Vec<ReportQualityRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guiding_trace: Option<GuidingTrace>,
    pub guiding_series: Vec<GuidingSlice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_clock: Option<LogClock>,
}

impl SessionReport {
    /// Holds nothing a night could show: no step, no quality row, no guiding
    /// sample nor slice, no event (the service's `analysis_empty`). Such a
    /// report is never sent: the night's analysis in place is kept.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.acquisition_steps.is_empty()
            && self.quality.is_empty()
            && self.guiding_trace.is_none()
            && self.guiding_series.is_empty()
            && self.timeline.events.is_empty()
    }
}

/// A timestamp of the report in the contract's shape
/// (`YYYY-MM-DDTHH:MM:SS.sssZ`); a value that does not parse is kept as is.
fn normalized(timestamp: &str) -> String {
    parse_iso8601_utc_to_unix_seconds(timestamp)
        .filter(|t| t.is_finite())
        .map_or_else(|| timestamp.to_string(), format_iso8601_utc_millis)
}

struct Candidate {
    timestamp_utc: String,
    unix: f64,
    evidence: String,
}

fn pick(candidates: &[Candidate], latest: bool) -> Option<TimelineBoundary> {
    let best = if latest {
        candidates.iter().max_by(|a, b| a.unix.total_cmp(&b.unix))
    } else {
        candidates.iter().min_by(|a, b| a.unix.total_cmp(&b.unix))
    }?;
    Some(TimelineBoundary {
        timestamp_utc: best.timestamp_utc.clone(),
        evidence: best.evidence.clone(),
    })
}

fn candidate(timestamp: &str, evidence: String) -> Option<Candidate> {
    let unix = parse_iso8601_utc_to_unix_seconds(timestamp).filter(|t| t.is_finite())?;
    Some(Candidate {
        timestamp_utc: format_iso8601_utc_millis(unix),
        unix,
        evidence,
    })
}

/// The night's start and end: the PHD2 log's own « Guiding Begins at » /
/// « Guiding Ends at » when there is one, else the earliest and latest of
/// every guiding sample, light frame and event.
fn timeline_boundaries(
    outcome: &AnalysisOutcome,
    quality: &[ReportQualityRow],
) -> (Option<TimelineBoundary>, Option<TimelineBoundary>) {
    let mut starts = Vec::new();
    let mut ends = Vec::new();
    for bounds in &outcome.guiding_session_bounds {
        if let Some(at) = &bounds.started_at_utc {
            starts.extend(candidate(
                at,
                format!(
                    "PHD2 guiding session start (\"Guiding Begins at\") in {}",
                    bounds.source_file
                ),
            ));
        }
        if let Some(at) = &bounds.ended_at_utc {
            ends.extend(candidate(
                at,
                format!(
                    "PHD2 guiding session end (\"Guiding Ends at\") in {}",
                    bounds.source_file
                ),
            ));
        }
    }
    if !starts.is_empty() || !ends.is_empty() {
        return (pick(&starts, false), pick(&ends, true));
    }
    let mut all = Vec::new();
    for row in &outcome.guiding_rows {
        all.extend(candidate(
            &row.timestamp_utc,
            format!(
                "guiding sample ({}, frame {})",
                row.source,
                row.frame
                    .map_or_else(|| "unknown".to_string(), |frame| frame.to_string())
            ),
        ));
    }
    for row in quality {
        all.extend(candidate(
            &row.timestamp_utc,
            format!("quality row ({})", row.source_path),
        ));
    }
    for event in &outcome.events {
        all.extend(candidate(
            &event.timestamp_utc,
            format!("{} event ({})", event.event_type, event.source),
        ));
    }
    (pick(&all, false), pick(&all, true))
}

/// The target of a light frame: its `OBJECT`, else the NINA window holding
/// its `DATE-OBS`.
fn target_hint(outcome: &AnalysisOutcome, frame: &super::AnalysedFrame) -> Option<String> {
    if let Some(object) = frame
        .object
        .as_deref()
        .map(str::trim)
        .filter(|o| !o.is_empty())
    {
        return Some(object.to_string());
    }
    let at = parse_iso8601_utc_to_unix_seconds(frame.record.date_obs_utc.as_deref()?)?;
    outcome
        .nina_logs
        .iter()
        .find_map(|log| log.target_at(at))
        .map(str::to_string)
}

/// The guiding samples the night's statistics are made of: the steady ones of
/// the analysis ([`AnalysisOutcome::steady_guiding_rows`]), and, when the
/// light frames are known and the log shares instants with them, only those
/// taken while the sensor was imaging.
/// The guider runs long before the first exposure (slew, centring, plate
/// solving) and after the last: a mount that is not tracking yet is not a
/// guiding result.
fn steady_guiding(
    outcome: &AnalysisOutcome,
    lights: &[(&super::AnalysedFrame, &super::QualityRow)],
) -> Vec<GuidingRow> {
    let spans: Vec<(f64, f64)> = lights
        .iter()
        .filter_map(|(_, row)| {
            let start = parse_iso8601_utc_to_unix_seconds(&row.timestamp_utc)?;
            Some((start, start + row.exposure_seconds?))
        })
        .collect();
    let first = spans
        .iter()
        .map(|(from, _)| *from)
        .fold(f64::INFINITY, f64::min);
    let last = spans
        .iter()
        .map(|(_, to)| *to)
        .fold(f64::NEG_INFINITY, f64::max);
    let imaging: Vec<GuidingRow> = if spans.is_empty() {
        Vec::new()
    } else {
        outcome
            .steady_guiding_rows
            .iter()
            .filter(|row| {
                parse_iso8601_utc_to_unix_seconds(&row.timestamp_utc)
                    .is_some_and(|at| at >= first && at <= last)
            })
            .cloned()
            .collect()
    };
    // A log that shares no instant with the exposures (a clock that was not
    // read right, a log of another night) cannot be told apart from them:
    // all its steady samples stay rather than none.
    if imaging.is_empty() {
        outcome.steady_guiding_rows.clone()
    } else {
        imaging
    }
}

/// An arcsecond figure to the thousandth: the precision of the guiding.
fn thousandths(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

/// What the measure and the stacking say of a frame: its measures, and
/// whether (and why) the stacking left it out.
fn pose_facts<'a>(
    records: &'a NightPoseRecords,
    source_path: &str,
) -> (Option<&'a pose_record::PoseRecord>, bool, Option<String>) {
    let name = source_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(source_path);
    let measure = records.measures.get(name);
    if records.unregistered.contains(name) {
        return (measure, true, Some("unregistered".to_string()));
    }
    if records.left_out.contains(name) {
        let reason = measure
            .and_then(|m| m.flag.clone())
            .unwrap_or_else(|| "manual".to_string());
        return (measure, true, Some(reason));
    }
    (measure, false, None)
}

/// Reshapes an already-computed [`AnalysisOutcome`] into the report v3,
/// without the measures of the poses. Pure (no filesystem access).
#[must_use]
pub fn report_from_outcome(outcome: &AnalysisOutcome) -> SessionReport {
    report_from_outcome_with(outcome, &NightPoseRecords::default())
}

/// [`report_from_outcome`] with what the measure of the poses and the
/// stacking recorded beside the night ([`pose_record::load`]). Pure.
#[must_use]
pub fn report_from_outcome_with(
    outcome: &AnalysisOutcome,
    records: &NightPoseRecords,
) -> SessionReport {
    let lights: Vec<(&super::AnalysedFrame, &super::QualityRow)> = outcome
        .frames
        .iter()
        .zip(outcome.quality_rows.iter())
        .filter(|(frame, _)| frame.is_light)
        .collect();
    let step_frames: Vec<StepFrame> = lights
        .iter()
        .map(|(frame, _)| StepFrame {
            path: frame.path.clone(),
            target_hint: target_hint(outcome, frame),
            record: frame.record.clone(),
        })
        .collect();
    let (mut acquisition_steps, frame_keys) = acquisition_steps::steps_from_frames(&step_frames);
    if lights.is_empty() {
        let exposures: Vec<_> = outcome
            .nina_logs
            .iter()
            .flat_map(|log| log.exposures.iter().cloned())
            .collect();
        acquisition_steps = acquisition_steps::steps_from_nina(&exposures);
    }
    let quality: Vec<ReportQualityRow> = lights
        .iter()
        .zip(frame_keys)
        .map(|((_, row), step_key)| {
            let (measure, rejected, reject_reason) = pose_facts(records, &row.source_path);
            // The header's own figure first; the measure's when it has none.
            let measured = |own: Option<f64>, from: fn(&pose_record::PoseRecord) -> Option<f64>| {
                own.or_else(|| measure.and_then(from))
            };
            ReportQualityRow {
                timestamp_utc: normalized(&row.timestamp_utc),
                step_key,
                filter: row.filter.clone(),
                exposure_seconds: row.exposure_seconds,
                gain: row.gain,
                iso: row.iso,
                binning: row.binning.clone(),
                sensor_temp_c: row.sensor_temp_c,
                rms_total_arcsec: row.rms_total_arcsec.map(thousandths),
                rms_ra_arcsec: row.rms_ra_arcsec.map(thousandths),
                rms_dec_arcsec: row.rms_dec_arcsec.map(thousandths),
                guiding_sample_count: row.guiding_sample_count,
                sky_background_adu: measured(row.sky_background_adu, |m| m.sky_background),
                fwhm_pixels: measured(row.fwhm_pixels, |m| m.fwhm_pixels),
                eccentricity: measured(row.eccentricity, |m| m.eccentricity),
                star_count: measure.and_then(|m| m.star_count).map(u64::from),
                rejected,
                reject_reason,
                source_path: row.source_path.clone(),
            }
        })
        .collect();
    let scales: Vec<f64> = lights
        .iter()
        .filter_map(|(frame, _)| frame.pixel_scale_arcsec)
        .collect();
    let (started_at, ended_at) = timeline_boundaries(outcome, &quality);
    let events: Vec<SessionEvent> = one_dither_per_dither(&outcome.events)
        .iter()
        .map(|event| SessionEvent {
            timestamp_utc: normalized(&event.timestamp_utc),
            ..event.clone()
        })
        .collect();
    let steady = steady_guiding(outcome, &lights);
    let trace = guiding_trace(outcome, &events, &steady);
    SessionReport {
        report_version: REPORT_VERSION,
        producer: PRODUCER.to_string(),
        generated_at_utc: now_iso8601_utc(),
        source: SourceCounts {
            phd2_files_parsed: outcome.phd2_files_parsed,
            asiair_files_parsed: outcome.asiair_files_parsed,
            fits_files_parsed: outcome.fits_files_parsed,
            nina_files_parsed: outcome.nina_files_parsed,
        },
        pixel_scale_arcsec: acquisition_steps::night_pixel_scale(&scales),
        timeline: Timeline {
            started_at,
            ended_at,
            events,
        },
        acquisition_steps,
        quality,
        guiding_trace: trace,
        guiding_series: guiding_series(&steady),
        log_clock: None,
    }
}

/// Analyzes `root` (read-only) and renders the result as the report v2.
pub fn generate_report(
    root: &Path,
    options: &AnalysisOptions,
) -> Result<SessionReport, AnalysisError> {
    generate_report_with(root, options, None)
}

/// [`generate_report`], telling `progress` which file it reads
/// ([`super::analyze_folder_with`]).
pub fn generate_report_with(
    root: &Path,
    options: &AnalysisOptions,
    progress: Option<crate::local_work::ProgressSink<'_>>,
) -> Result<SessionReport, AnalysisError> {
    let outcome = super::analyze_folder_with(root, options, progress)?;
    Ok(report_from_outcome_with(&outcome, &pose_record::load(root)))
}

/// Renders a [`SessionReport`] as canonical JSON (sorted keys, no
/// insignificant whitespace): the exact bytes sent or written.
#[must_use]
pub fn render_report_json(report: &SessionReport) -> String {
    canonical_json(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    use crate::time_util::lock_deterministic_time_env_for_test;
    use std::fs;

    /// Same synthetic-header builder as `fits::tests`/`analysis::mod::tests`
    /// (duplicated locally: both of those copies are private to their own
    /// `#[cfg(test)]` module).
    fn build_fits_header(cards: &[(&str, &str)]) -> Vec<u8> {
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

    const PHD2_LOG: &str = "Guiding Begins at 2026-08-07 22:38:00\nPixel scale = 2.00 arc-sec/px\nFrame,Time,mount,dx,dy,RARawDistance,DECRawDistance,RAGuideDistance,DECGuideDistance,RADuration,RADirection,DECDuration,DECDirection,XStep,YStep,StarMass,SNR,ErrorCode\n1,1.000,Mount,0,0,0.100,0.100,0,0,0,,0,,0,0,30000,12,0\n2,3.000,Mount,0,0,0.200,0.100,0,0,0,,0,,0,0,30000,12,0\nINFO: SETTLING STATE CHANGE, Settling started\nINFO: SETTLING STATE CHANGE, Settling complete\nGuiding Ends at 2026-08-07 23:00:00\n";

    const ASIAIR_LOG: &str = "2026/08/07 22:39:00 [Guide] Dither\n2026/08/07 22:39:10 [Guide] Dither Settle\n2026/08/07 22:40:00 [AutoFocus|Begin] Run AF when temperature changed 2 degrees, 27.0\u{2103} changed to 24.0\u{2103}, exposure 3.0s Bin1, temperature 24.0\u{2103}\n2026/08/07 22:41:00 [AutoFocus|End] Auto focus succeeded\n";

    fn write_fixture_folder() -> TempDir {
        let dir = TempDir::new("analysis-report-folder");
        fs::write(dir.path().join("PHD2_GuideLog_2026-08-07.txt"), PHD2_LOG).unwrap();
        fs::write(dir.path().join("Autorun_Log_2026-08-07.txt"), ASIAIR_LOG).unwrap();
        fs::write(
            dir.path().join("frame-0001.fits"),
            build_fits_header(&[
                ("EXPTIME", "300.0"),
                ("FILTER", "'Ha      '"),
                ("GAIN", "100"),
                ("CCD-TEMP", "-10.0"),
                ("DATE-OBS", "'2026-08-07T22:38:00'"),
                ("XBINNING", "1"),
                ("YBINNING", "1"),
            ]),
        )
        .unwrap();
        dir
    }

    #[test]
    fn generates_a_report_v3_by_astro_sync() {
        let _guard = lock_deterministic_time_env_for_test();
        let dir = write_fixture_folder();
        let report = generate_report(dir.path(), &AnalysisOptions::default()).unwrap();
        assert_eq!(report.report_version, 3);
        assert_eq!(report.producer, "astro-sync");
        assert!(!report.is_empty());
    }

    #[test]
    fn combines_quality_guiding_and_events_from_the_underlying_outcome() {
        let _guard = lock_deterministic_time_env_for_test();
        let dir = write_fixture_folder();
        let report = generate_report(dir.path(), &AnalysisOptions::default()).unwrap();
        assert_eq!(report.quality.len(), 1);
        // The two samples (22:38:01, 22:38:03) are one point of 10 s.
        let trace = report.guiding_trace.as_ref().unwrap();
        assert_eq!(trace.samples, 2);
        assert_eq!(trace.points.len(), 1);
        assert_eq!(trace.scale_arcsec_per_px, Some(2.0));
        assert!(!report.timeline.events.is_empty());
        assert_eq!(report.acquisition_steps.len(), 1);
        assert_eq!(
            report.quality[0].step_key.as_deref(),
            Some(report.acquisition_steps[0].key.as_str())
        );
        assert_eq!(report.guiding_series.len(), 1);
        assert_eq!(report.guiding_series[0].samples, 2);
        assert_eq!(report.source.phd2_files_parsed, 1);
        assert_eq!(report.source.asiair_files_parsed, 1);
        assert_eq!(report.source.fits_files_parsed, 1);
    }

    /// A PHD2 log whose first section is the guider running before the
    /// exposure (the mount is still moving: 50 px), then the guiding of the
    /// exposure with a dither (settling, then a recovery that walks back).
    fn guiding_noise_log() -> String {
        let header = "Frame,Time,mount,dx,dy,RARawDistance,DECRawDistance,RAGuideDistance,DECGuideDistance,RADuration,RADirection,DECDuration,DECDirection,XStep,YStep,StarMass,SNR,ErrorCode";
        let row = |frame: u32, at: f64, ra: f64, dec: f64, error: u32| {
            format!("{frame},{at:.3},Mount,0,0,{ra},{dec},0,0,0,,0,,0,0,30000,12,{error}")
        };
        let mut lines = vec![
            "Guiding Begins at 2026-08-07 22:30:00".to_string(),
            "Pixel scale = 2.00 arc-sec/px".to_string(),
            header.to_string(),
            row(1, 1.0, 50.0, 40.0, 0),
            row(2, 4.0, 52.0, 41.0, 0),
            "Guiding Ends at 2026-08-07 22:37:00".to_string(),
            "Guiding Begins at 2026-08-07 22:37:30".to_string(),
            "Pixel scale = 2.00 arc-sec/px".to_string(),
            header.to_string(),
        ];
        // 22:38:00 -> 22:43:00 is the exposure. Calm frames of 0.5" each way.
        for frame in 1..=10_u32 {
            let sign = if frame % 2 == 0 { 1.0 } else { -1.0 };
            lines.push(row(
                frame,
                40.0 + f64::from(frame) * 3.0,
                0.25 * sign,
                0.25 * sign,
                0,
            ));
        }
        lines.push("INFO: DITHER by 3.000, -2.000, new lock pos = 10.0, 10.0".to_string());
        lines.push("INFO: SETTLING STATE CHANGE, Settling started".to_string());
        lines.push(row(11, 95.0, 4.0, -3.0, 0));
        lines.push("INFO: SETTLING STATE CHANGE, Settling failed".to_string());
        // Still walking back after the settle gave up: 8 px, then 1 px, then calm.
        lines.push(row(12, 98.0, 8.0, -6.0, 0));
        lines.push(row(13, 101.0, 0.4, -0.3, 0));
        lines.push(row(14, 104.0, 0.3, 0.2, 0));
        lines.push(row(15, 107.0, 0.2, -0.2, 7));
        lines.push("Guiding Ends at 2026-08-07 22:42:00".to_string());
        lines.join("\n") + "\n"
    }

    fn write_noise_folder() -> TempDir {
        let dir = TempDir::new("analysis-report-guiding-noise");
        fs::write(
            dir.path().join("PHD2_GuideLog_2026-08-07.txt"),
            guiding_noise_log(),
        )
        .unwrap();
        fs::write(
            dir.path().join("frame-0001.fits"),
            build_fits_header(&[
                ("EXPTIME", "300.0"),
                ("DATE-OBS", "'2026-08-07T22:38:00'"),
                ("XBINNING", "1"),
                ("YBINNING", "1"),
            ]),
        )
        .unwrap();
        dir
    }

    #[test]
    fn the_nights_guiding_leaves_out_the_runup_the_dither_and_its_recovery() {
        let _guard = lock_deterministic_time_env_for_test();
        let dir = write_noise_folder();
        let report = generate_report(dir.path(), &AnalysisOptions::default()).unwrap();
        // Every raw frame is in the trace; the dither's are out of the
        // statistics, never a spike.
        let trace = report.guiding_trace.as_ref().unwrap();
        assert_eq!(trace.samples, 17);
        let out: u32 = trace.points.iter().map(|p| p.out).sum();
        assert!(out >= 3, "the settle, the walk back and the error: {out}");
        assert_eq!(trace.rms.samples, 12);
        let counted: u64 = report
            .guiding_series
            .iter()
            .map(|slice| slice.samples)
            .sum();
        // 10 calm frames + the two calm ones of the recovery; not the 2 of the
        // run-up before the exposure, the settle, the 8 px frame walking back,
        // nor the one with an error code.
        assert_eq!(counted, 12);
        let worst = report
            .guiding_series
            .iter()
            .filter_map(|slice| slice.rms_total)
            .fold(0.0_f64, f64::max);
        assert!(
            worst < 1.5,
            "the dither and the run-up are counted: {worst}"
        );
    }

    #[test]
    fn a_log_that_shares_no_instant_with_the_exposures_is_not_emptied() {
        let _guard = lock_deterministic_time_env_for_test();
        let dir = write_noise_folder();
        // The frame is dated a day later than the log: a clock that was read
        // wrong must not wipe the night's guiding.
        fs::write(
            dir.path().join("frame-0001.fits"),
            build_fits_header(&[
                ("EXPTIME", "300.0"),
                ("DATE-OBS", "'2026-08-08T22:38:00'"),
                ("XBINNING", "1"),
                ("YBINNING", "1"),
            ]),
        )
        .unwrap();
        let report = generate_report(dir.path(), &AnalysisOptions::default()).unwrap();
        let counted: u64 = report
            .guiding_series
            .iter()
            .map(|slice| slice.samples)
            .sum();
        // The twelve steady samples stay (the two of the run-up are far-off jumps).
        assert_eq!(counted, 12);
    }

    #[test]
    fn without_light_frames_the_run_up_cannot_be_told_and_is_kept() {
        let _guard = lock_deterministic_time_env_for_test();
        let dir = write_noise_folder();
        fs::remove_file(dir.path().join("frame-0001.fits")).unwrap();
        let report = generate_report(dir.path(), &AnalysisOptions::default()).unwrap();
        // The run-up frames are 50 px against a night of 0.5": a jump of the
        // lock, left out by the spike limit all the same.
        let counted: u64 = report
            .guiding_series
            .iter()
            .map(|slice| slice.samples)
            .sum();
        assert_eq!(counted, 12);
    }

    #[test]
    fn the_report_says_which_file_it_reads_one_report_per_log_or_frame() {
        let _guard = lock_deterministic_time_env_for_test();
        let dir = write_fixture_folder();
        fs::write(dir.path().join("notes.png"), b"not read").unwrap();
        let seen = std::sync::Mutex::new(Vec::new());
        let sink = |progress: &crate::local_work::WorkProgress| {
            seen.lock().unwrap().push((
                progress.kind,
                progress.done,
                progress.total,
                progress.current.clone().unwrap_or_default(),
            ));
        };
        generate_report_with(dir.path(), &AnalysisOptions::default(), Some(&sink)).unwrap();
        let seen = seen.into_inner().unwrap();
        // Two logs and a frame; the image is not read, so not counted.
        assert_eq!(seen.len(), 3, "{seen:?}");
        assert!(
            seen.iter()
                .enumerate()
                .all(
                    |(i, (kind, done, total, _))| *kind == crate::local_work::WorkKind::Analysis
                        && *done == i as u64 + 1
                        && *total == 3
                )
        );
        assert!(seen.iter().any(|(_, _, _, file)| file == "frame-0001.fits"));
    }

    #[test]
    fn the_session_boundary_is_the_phd2_logs_own_begin_and_end() {
        let _guard = lock_deterministic_time_env_for_test();
        let dir = write_fixture_folder();
        let report = generate_report(dir.path(), &AnalysisOptions::default()).unwrap();
        let started = report.timeline.started_at.as_ref().unwrap();
        let ended = report.timeline.ended_at.as_ref().unwrap();
        assert_eq!(started.timestamp_utc, "2026-08-07T22:38:00.000Z");
        assert_eq!(ended.timestamp_utc, "2026-08-07T23:00:00.000Z");
        assert!(started.evidence.contains("Guiding Begins"));
        assert!(started.evidence.contains("PHD2_GuideLog_2026-08-07.txt"));
        assert!(ended.evidence.contains("Guiding Ends"));
    }

    #[test]
    fn falls_back_to_earliest_latest_timestamp_when_no_phd2_log_is_present() {
        let _guard = lock_deterministic_time_env_for_test();
        let dir = TempDir::new("analysis-report-fits-only");
        fs::write(
            dir.path().join("frame-0001.fits"),
            build_fits_header(&[("EXPTIME", "300.0"), ("DATE-OBS", "'2026-08-07T22:38:00'")]),
        )
        .unwrap();
        fs::write(
            dir.path().join("frame-0002.fits"),
            build_fits_header(&[("EXPTIME", "300.0"), ("DATE-OBS", "'2026-08-07T23:10:00'")]),
        )
        .unwrap();
        let report = generate_report(dir.path(), &AnalysisOptions::default()).unwrap();
        let started = report.timeline.started_at.as_ref().unwrap();
        let ended = report.timeline.ended_at.as_ref().unwrap();
        assert_eq!(started.timestamp_utc, "2026-08-07T22:38:00.000Z");
        assert_eq!(ended.timestamp_utc, "2026-08-07T23:10:00.000Z");
        assert!(started.evidence.contains("quality row"));
    }

    #[test]
    fn an_empty_folder_produces_a_report_with_no_boundary_and_no_data() {
        let _guard = lock_deterministic_time_env_for_test();
        let dir = TempDir::new("analysis-report-empty");
        let report = generate_report(dir.path(), &AnalysisOptions::default()).unwrap();
        assert!(report.timeline.started_at.is_none());
        assert!(report.timeline.ended_at.is_none());
        assert!(report.quality.is_empty());
        assert!(report.guiding_trace.is_none());
        assert!(report.timeline.events.is_empty());
        assert!(report.acquisition_steps.is_empty());
        assert!(report.is_empty(), "an empty night is never sent");
    }

    #[test]
    fn calibration_frames_make_neither_a_step_nor_a_quality_row() {
        let _guard = lock_deterministic_time_env_for_test();
        let dir = TempDir::new("analysis-report-calibration");
        fs::create_dir_all(dir.path().join("calibration/darks")).unwrap();
        fs::create_dir_all(dir.path().join("M31/lights")).unwrap();
        fs::write(
            dir.path().join("calibration/darks/dark-1.fits"),
            build_fits_header(&[("EXPTIME", "120.0"), ("GAIN", "100")]),
        )
        .unwrap();
        fs::write(
            dir.path().join("M31/lights/flat-in-lights.fits"),
            build_fits_header(&[("IMAGETYP", "'FLAT    '"), ("EXPTIME", "1.0")]),
        )
        .unwrap();
        fs::write(
            dir.path().join("M31/lights/light-1.fits"),
            build_fits_header(&[
                ("IMAGETYP", "'LIGHT   '"),
                ("OBJECT", "'M 31    '"),
                ("EXPTIME", "120.0"),
                ("GAIN", "100"),
                ("XPIXSZ", "3.76"),
                ("FOCALLEN", "400.0"),
                ("DATE-OBS", "'2026-10-07T21:10:00'"),
            ]),
        )
        .unwrap();
        let report = generate_report(dir.path(), &AnalysisOptions::default()).unwrap();
        assert_eq!(report.source.fits_files_parsed, 3);
        assert_eq!(report.quality.len(), 1);
        assert_eq!(report.acquisition_steps.len(), 1);
        assert_eq!(
            report.acquisition_steps[0].target_hint.as_deref(),
            Some("M 31")
        );
        assert_eq!(report.pixel_scale_arcsec, Some(1.94));
    }

    #[test]
    fn a_night_known_from_nina_alone_takes_its_steps_and_target_from_the_log() {
        let _guard = lock_deterministic_time_env_for_test();
        let dir = TempDir::new("analysis-report-nina-only");
        fs::create_dir_all(dir.path().join("logs")).unwrap();
        fs::write(
            dir.path().join("logs/nina.log"),
            "2026-10-07T21:05:00.0000|INFO|SequenceItem.cs|Run|50|Starting target IC 1396\n\
             2026-10-07T21:10:00.0000|INFO|CameraVM.cs|Capture|88|Starting Exposure - Exposure Time: 300s; Filter: OIII; Gain: 120;\n\
             2026-10-07T21:15:30.0000|INFO|CameraVM.cs|Capture|88|Starting Exposure - Exposure Time: 300s; Filter: OIII; Gain: 120;\n",
        )
        .unwrap();
        let report = generate_report(
            dir.path(),
            &AnalysisOptions {
                nina_utc_offset: "+09:00".into(),
                ..AnalysisOptions::default()
            },
        )
        .unwrap();
        assert_eq!(report.source.nina_files_parsed, 1);
        assert_eq!(report.acquisition_steps.len(), 1);
        let step = &report.acquisition_steps[0];
        assert_eq!(step.target_hint.as_deref(), Some("IC 1396"));
        assert_eq!(step.frame_count, 2);
        assert_eq!(step.gain, Some(120));
        assert_eq!(
            report.timeline.started_at.as_ref().unwrap().timestamp_utc,
            "2026-10-07T12:05:00.000Z"
        );
        assert!(report.quality.is_empty());
    }

    #[test]
    fn every_quality_row_traces_to_its_source_fits_file() {
        let _guard = lock_deterministic_time_env_for_test();
        let dir = write_fixture_folder();
        let report = generate_report(dir.path(), &AnalysisOptions::default()).unwrap();
        for row in &report.quality {
            assert!(!row.source_path.is_empty());
        }
    }

    #[test]
    fn the_raw_samples_are_no_longer_sent() {
        let _guard = lock_deterministic_time_env_for_test();
        let dir = write_fixture_folder();
        let report = generate_report(dir.path(), &AnalysisOptions::default()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&render_report_json(&report)).unwrap();
        assert!(value.get("guiding").is_none());
        assert!(value["guidingTrace"]["points"].is_array());
    }

    #[test]
    fn a_dither_phd2_and_the_asiair_both_tell_is_counted_once() {
        let _guard = lock_deterministic_time_env_for_test();
        let dir = write_fixture_folder();
        fs::write(
            dir.path().join("PHD2_GuideLog_2026-08-07.txt"),
            PHD2_LOG.replace(
                "INFO: SETTLING STATE CHANGE, Settling started",
                "INFO: DITHER by 3.000, -2.000, new lock pos = 10.0, 10.0\nINFO: SETTLING STATE CHANGE, Settling started",
            ),
        )
        .unwrap();
        // The ASIAIR's dither (22:39:00) is not PHD2's (22:38:03): both stay.
        let report = generate_report(dir.path(), &AnalysisOptions::default()).unwrap();
        let dithers = |r: &SessionReport| {
            r.timeline
                .events
                .iter()
                .filter(|e| e.event_type == "dither")
                .count()
        };
        assert_eq!(dithers(&report), 2);
        // Told at the same minute by both logs: one dither.
        fs::write(
            dir.path().join("Autorun_Log_2026-08-07.txt"),
            ASIAIR_LOG.replace("22:39:00 [Guide] Dither", "22:38:10 [Guide] Dither"),
        )
        .unwrap();
        let report = generate_report(dir.path(), &AnalysisOptions::default()).unwrap();
        assert_eq!(dithers(&report), 1);
        let dither = report
            .timeline
            .events
            .iter()
            .find(|e| e.event_type == "dither")
            .unwrap();
        assert_eq!(dither.source, "phd2");
    }

    #[test]
    fn the_measures_of_the_poses_and_the_stackings_rejects_reach_the_quality_rows() {
        let _guard = lock_deterministic_time_env_for_test();
        let dir = write_fixture_folder();
        fs::create_dir_all(dir.path().join(".astro-sync-work/pose-stats")).unwrap();
        fs::write(
            dir.path().join(".astro-sync-work/pose-stats/measures-T.json"),
            r#"{"format":1,"target":"T","poses":{"frame-0001.fits":{"fwhmPixels":2.6,"eccentricity":0.31,"starCount":80,"skyBackground":812.0,"flag":"clouds"}}}"#,
        )
        .unwrap();
        let report = generate_report(dir.path(), &AnalysisOptions::default()).unwrap();
        let row = &report.quality[0];
        assert_eq!(row.fwhm_pixels, Some(2.6));
        assert_eq!(row.eccentricity, Some(0.31));
        assert_eq!(row.star_count, Some(80));
        assert_eq!(row.sky_background_adu, Some(812.0));
        // Flagged but kept by the person: not rejected.
        assert!(!row.rejected);
        fs::write(
            dir.path()
                .join(".astro-sync-work/pose-stats/left-out-T.json"),
            r#"{"format":1,"target":"T","leftOut":["frame-0001.fits"]}"#,
        )
        .unwrap();
        let report = generate_report(dir.path(), &AnalysisOptions::default()).unwrap();
        assert!(report.quality[0].rejected);
        assert_eq!(report.quality[0].reject_reason.as_deref(), Some("clouds"));
    }

    #[test]
    fn rendering_the_same_report_twice_is_byte_identical() {
        let _guard = lock_deterministic_time_env_for_test();
        let dir = write_fixture_folder();
        // SAFETY (test-only): setting one process-wide env var read by
        // `now_iso8601_utc`. Rust's test harness is NOT single-threaded by
        // default -- the lock acquired above (see
        // `time_util::DETERMINISTIC_TIME_ENV_LOCK`'s own doc comment) is
        // what actually rules out a concurrent read/write from another
        // test's thread while this one is set.
        unsafe {
            std::env::set_var("ASTRO_SYNC_DETERMINISTIC_TIME", "2026-09-27T00:00:00Z");
        }
        let first =
            render_report_json(&generate_report(dir.path(), &AnalysisOptions::default()).unwrap());
        let second =
            render_report_json(&generate_report(dir.path(), &AnalysisOptions::default()).unwrap());
        unsafe {
            std::env::remove_var("ASTRO_SYNC_DETERMINISTIC_TIME");
        }
        assert_eq!(first, second);
        assert!(first.contains("\"reportVersion\":3"));
        assert!(!first.contains('\n'));
        assert!(!first.contains(": ")); // no insignificant whitespace after a colon
    }

    /// The evidence rule, checked against the actual rendered JSON (not the
    /// source, which legitimately discusses "score" in comments explaining
    /// why there isn't one): no JSON key anywhere in a rendered report is
    /// (or contains) `score`, `rating`, or `grade`, case-insensitively.
    #[test]
    fn report_never_produces_a_composite_score() {
        let _guard = lock_deterministic_time_env_for_test();
        const FORBIDDEN: [&str; 3] = ["score", "rating", "grade"];
        let dir = write_fixture_folder();
        let report = generate_report(dir.path(), &AnalysisOptions::default()).unwrap();
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
}

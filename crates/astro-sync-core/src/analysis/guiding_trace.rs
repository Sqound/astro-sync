//! The night's guiding, reduced (`guidingTrace` of the report v3): at most
//! one point per [`INTERVAL_SECONDS`] instead of every raw PHD2 sample (the
//! report v2's `guiding[]`, 1.8 to 3.8 MB a night; the trace of a night of
//! six hours is a few hundred kilobytes).
//!
//! - **Points.** The samples are cut into windows of [`INTERVAL_SECONDS`]
//!   aligned on UTC (`t` = seconds since `startUtc`, a window without a
//!   sample is not written). Each holds the mean and the peak (the value
//!   farthest from zero, signed) of the offsets in right ascension and
//!   declination, in arcseconds, the corrections sent over the window (the
//!   pulses' durations summed with their direction: east and north count
//!   positive, west and south negative; `null` when the log does not say the
//!   direction), the number of samples and how many of them stay out of the
//!   statistics (`out`: settling after a dither, the guider walking back,
//!   a lost star found again, an error code, a meridian flip). A window whose
//!   samples are all out is a settle, never a spike of the guiding.
//! - **Markers**: dithers (one per dither: PHD2's, else the ASIAIR's),
//!   settles (from the dither to « Settling complete » or « failed »), lost
//!   stars, pauses, meridian flips and focus runs, with their end when they
//!   last.
//! - **RMS** of the night over the samples that count, while the sensor was
//!   imaging (the same samples as `guidingSeries`).
//! - **Scale** of the guide camera (`Pixel scale = …`, median of the
//!   sections): a reader shows pixels by dividing.
//!
//! Every value is a mean, a peak, a sum or a root-mean-square of values a
//! parser read; no composite figure.

use serde::Serialize;
use std::collections::HashSet;

use super::{AnalysisOutcome, GuidingRow, SessionEvent};
use crate::time_util::{format_iso8601_utc_millis, parse_iso8601_utc_to_unix_seconds};

/// The width of a window, in seconds: at most one point every ten seconds.
pub const INTERVAL_SECONDS: f64 = 10.0;

/// Two dithers of PHD2 and the ASIAIR closer than this (seconds) are one.
pub const SAME_DITHER_SECONDS: f64 = 30.0;

/// Lost-star markers closer than this (seconds) are one loss.
const SAME_LOSS_SECONDS: f64 = 30.0;

/// One window of the trace.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TracePoint {
    /// Seconds from the trace's `startUtc` to the window's start.
    pub t: u64,
    /// Samples in the window.
    pub n: u32,
    /// Samples left out of the statistics.
    pub out: u32,
    /// Mean offsets, arcseconds.
    pub ra: f64,
    pub dec: f64,
    /// The offset farthest from zero, signed, arcseconds.
    pub ra_peak: f64,
    pub dec_peak: f64,
    /// Corrections sent, milliseconds: east and north positive.
    pub ra_pulse_ms: Option<i64>,
    pub dec_pulse_ms: Option<i64>,
}

/// A moment of the night drawn on the trace.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceMarker {
    /// `dither`, `settle`, `star-lost`, `pause`, `meridian-flip`, `autofocus`.
    #[serde(rename = "type")]
    pub kind: String,
    pub at: String,
    pub end: Option<String>,
}

/// The root-mean-square of the night, arcseconds, over the samples that count.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceRms {
    pub ra_arcsec: Option<f64>,
    pub dec_arcsec: Option<f64>,
    pub total_arcsec: Option<f64>,
    pub samples: u64,
}

/// `guidingTrace` of the report v3.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuidingTrace {
    pub interval_seconds: u32,
    pub start_utc: String,
    pub scale_arcsec_per_px: Option<f64>,
    /// Seconds with at least one guide sample (windows written × interval).
    pub guided_seconds: u64,
    pub samples: u64,
    pub rms: TraceRms,
    pub points: Vec<TracePoint>,
    pub markers: Vec<TraceMarker>,
}

fn round_to(value: f64, digits: i32) -> f64 {
    let factor = 10_f64.powi(digits);
    (value * factor).round() / factor
}

fn rms(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| {
        round_to(
            (values.iter().map(|v| v * v).sum::<f64>() / values.len() as f64).sqrt(),
            3,
        )
    })
}

fn peak(values: &[f64]) -> f64 {
    values.iter().copied().fold(
        0.0_f64,
        |kept, v| if v.abs() > kept.abs() { v } else { kept },
    )
}

/// The signed pulse of a row: its duration with its direction; `None` when
/// the row has a pulse whose direction the log did not write.
fn signed_pulse(duration: Option<f64>, direction: Option<i8>) -> Option<f64> {
    match (duration.filter(|d| d.is_finite() && *d > 0.0), direction) {
        (None, _) => Some(0.0),
        (Some(duration), Some(direction)) => Some(duration * f64::from(direction)),
        (Some(_), None) => None,
    }
}

/// The night's RMS over `steady` (the samples of `guidingSeries`).
#[must_use]
pub fn night_rms(steady: &[GuidingRow]) -> TraceRms {
    let ra: Vec<f64> = steady.iter().filter_map(|r| r.ra_offset_arcsec).collect();
    let dec: Vec<f64> = steady.iter().filter_map(|r| r.dec_offset_arcsec).collect();
    let (ra, dec) = (rms(&ra), rms(&dec));
    TraceRms {
        ra_arcsec: ra,
        dec_arcsec: dec,
        total_arcsec: match (ra, dec) {
            (Some(ra), Some(dec)) => Some(round_to(ra.hypot(dec), 3)),
            _ => None,
        },
        samples: steady.len() as u64,
    }
}

/// The median of the guide camera's scales the PHD2 sections declare.
fn guide_scale(outcome: &AnalysisOutcome) -> Option<f64> {
    let mut scales: Vec<f64> = outcome
        .guiding_session_bounds
        .iter()
        .filter_map(|b| b.pixel_scale_arcsec_per_px)
        .filter(|s| s.is_finite() && *s > 0.0)
        .collect();
    if scales.is_empty() {
        return None;
    }
    scales.sort_by(f64::total_cmp);
    let middle = scales.len() / 2;
    let median = if scales.len() % 2 == 1 {
        scales[middle]
    } else {
        (scales[middle - 1] + scales[middle]) / 2.0
    };
    Some(round_to(median, 3))
}

/// The points of the trace: `rows` (any order) cut into windows; a row whose
/// timestamp is not in `counted` is left out of the statistics.
#[must_use]
pub fn trace_points(rows: &[GuidingRow], counted: &HashSet<&str>) -> (f64, Vec<TracePoint>) {
    let mut samples: Vec<(f64, &GuidingRow)> = rows
        .iter()
        .filter_map(|row| {
            let at = parse_iso8601_utc_to_unix_seconds(&row.timestamp_utc)?;
            (at.is_finite() && row.ra_offset_arcsec.is_some() && row.dec_offset_arcsec.is_some())
                .then_some((at, row))
        })
        .collect();
    samples.sort_by(|a, b| a.0.total_cmp(&b.0));
    let Some(first) = samples.first().map(|(at, _)| *at) else {
        return (0.0, Vec::new());
    };
    let origin = (first / INTERVAL_SECONDS).floor() * INTERVAL_SECONDS;
    let mut points = Vec::new();
    let mut index = 0;
    while index < samples.len() {
        let slot = ((samples[index].0 - origin) / INTERVAL_SECONDS).floor();
        let end = origin + (slot + 1.0) * INTERVAL_SECONDS;
        let mut ra = Vec::new();
        let mut dec = Vec::new();
        let mut out = 0_u32;
        let mut ra_pulse = Some(0.0);
        let mut dec_pulse = Some(0.0);
        while index < samples.len() && samples[index].0 < end {
            let row = samples[index].1;
            ra.extend(row.ra_offset_arcsec);
            dec.extend(row.dec_offset_arcsec);
            if !counted.contains(row.timestamp_utc.as_str()) {
                out += 1;
            }
            ra_pulse = ra_pulse
                .zip(signed_pulse(row.ra_duration_ms, row.ra_direction))
                .map(|(a, b)| a + b);
            dec_pulse = dec_pulse
                .zip(signed_pulse(row.dec_duration_ms, row.dec_direction))
                .map(|(a, b)| a + b);
            index += 1;
        }
        let mean = |values: &[f64]| round_to(values.iter().sum::<f64>() / values.len() as f64, 2);
        points.push(TracePoint {
            t: (slot * INTERVAL_SECONDS) as u64,
            n: ra.len() as u32,
            out,
            ra: mean(&ra),
            dec: mean(&dec),
            ra_peak: round_to(peak(&ra), 2),
            dec_peak: round_to(peak(&dec), 2),
            ra_pulse_ms: ra_pulse.map(|v| v.round() as i64),
            dec_pulse_ms: dec_pulse.map(|v| v.round() as i64),
        });
    }
    (origin, points)
}

fn instant(event: &SessionEvent) -> Option<f64> {
    parse_iso8601_utc_to_unix_seconds(&event.timestamp_utc).filter(|t| t.is_finite())
}

/// The night's events with one dither per dither: an ASIAIR dither within
/// [`SAME_DITHER_SECONDS`] of a PHD2 dither is the same one, told twice
/// (PHD2's is kept: it has the guider's own instant). `events` sorted.
#[must_use]
pub fn one_dither_per_dither(events: &[SessionEvent]) -> Vec<SessionEvent> {
    let phd2: Vec<f64> = events
        .iter()
        .filter(|e| e.event_type == "dither" && e.source == "phd2")
        .filter_map(instant)
        .collect();
    events
        .iter()
        .filter(|event| {
            !(event.event_type == "dither"
                && event.source != "phd2"
                && instant(event)
                    .is_some_and(|at| phd2.iter().any(|p| (p - at).abs() <= SAME_DITHER_SECONDS)))
        })
        .cloned()
        .collect()
}

fn marker(kind: &str, at: f64, end: Option<f64>) -> TraceMarker {
    TraceMarker {
        kind: kind.to_string(),
        at: format_iso8601_utc_millis(at),
        end: end.filter(|e| *e > at).map(format_iso8601_utc_millis),
    }
}

/// The markers of the trace, from the night's events (one dither per dither
/// already) and the meridian flips' spans, in order.
#[must_use]
pub fn trace_markers(events: &[SessionEvent], flips: &[(f64, f64)]) -> Vec<TraceMarker> {
    let mut markers: Vec<(f64, TraceMarker)> = Vec::new();
    let timed: Vec<(f64, &SessionEvent)> = events
        .iter()
        .filter_map(|e| instant(e).map(|at| (at, e)))
        .collect();
    let next_after = |from: f64, test: &dyn Fn(&SessionEvent) -> bool| {
        timed
            .iter()
            .find(|(at, e)| *at >= from && test(e))
            .map(|(at, _)| *at)
    };
    let phd2_settles = timed
        .iter()
        .any(|(_, e)| e.event_type == "dither-settle" && e.source == "phd2");
    let settle_end = |e: &SessionEvent| {
        e.event_type == "dither-settle"
            && e.source == "phd2"
            && matches!(
                e.data.get("outcome").and_then(|o| o.as_str()),
                Some("complete" | "failed")
            )
    };
    let mut lost_last: Option<f64> = None;
    for (at, event) in &timed {
        let at = *at;
        match event.event_type.as_str() {
            "dither" => {
                markers.push((at, marker("dither", at, None)));
                // The settle runs from the dither to PHD2's verdict.
                if phd2_settles && event.source == "phd2" {
                    let end = next_after(at, &settle_end);
                    markers.push((at, marker("settle", at, end)));
                }
            }
            "dither-settle" if event.source == "phd2" => {
                // A settle PHD2 opened without a dither (guiding start).
                let started = event.data.get("outcome").and_then(|o| o.as_str()) == Some("started");
                let after_dither = timed.iter().any(|(t, e)| {
                    e.event_type == "dither" && e.source == "phd2" && (at - t).abs() <= 5.0
                });
                if started && !after_dither {
                    markers.push((at, marker("settle", at, next_after(at, &settle_end))));
                }
            }
            "settle" if !phd2_settles && event.source == "asiair" => {
                // The ASIAIR tells the end of a settle: it began at the dither.
                let begin = timed
                    .iter()
                    .rev()
                    .find(|(t, e)| *t <= at && e.event_type == "dither")
                    .map_or(at, |(t, _)| *t);
                markers.push((begin, marker("settle", begin, Some(at))));
            }
            "star-lost" => {
                if lost_last.is_none_or(|last| at - last > SAME_LOSS_SECONDS) {
                    markers.push((at, marker("star-lost", at, None)));
                }
                lost_last = Some(at);
            }
            "pause" => {
                let end = next_after(at, &|e: &SessionEvent| e.event_type == "resume");
                markers.push((at, marker("pause", at, end)));
            }
            "autofocus" => {
                let end = next_after(at, &|e: &SessionEvent| e.event_type == "autofocus-result");
                markers.push((at, marker("autofocus", at, end)));
            }
            _ => {}
        }
    }
    if flips.is_empty() {
        for (at, event) in &timed {
            if event.event_type == "meridian-flip" {
                markers.push((*at, marker("meridian-flip", *at, None)));
            }
        }
    } else {
        for (from, to) in flips {
            markers.push((*from, marker("meridian-flip", *from, Some(*to))));
        }
    }
    markers.sort_by(|a, b| a.0.total_cmp(&b.0));
    markers.into_iter().map(|(_, m)| m).collect()
}

/// The trace of the night, `None` without a guide sample. `events`: the
/// night's events with one dither per dither; `steady`: the samples of the
/// statistics while the sensor was imaging.
#[must_use]
pub fn guiding_trace(
    outcome: &AnalysisOutcome,
    events: &[SessionEvent],
    steady: &[GuidingRow],
) -> Option<GuidingTrace> {
    let counted: HashSet<&str> = outcome
        .steady_guiding_rows
        .iter()
        .map(|r| r.timestamp_utc.as_str())
        .collect();
    let (origin, points) = trace_points(&outcome.guiding_rows, &counted);
    if points.is_empty() {
        return None;
    }
    Some(GuidingTrace {
        interval_seconds: INTERVAL_SECONDS as u32,
        start_utc: format_iso8601_utc_millis(origin),
        scale_arcsec_per_px: guide_scale(outcome),
        guided_seconds: points.len() as u64 * INTERVAL_SECONDS as u64,
        samples: points.iter().map(|p| u64::from(p.n)).sum(),
        rms: night_rms(steady),
        points,
        markers: trace_markers(events, &outcome.meridian_flip_spans),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(at: &str, ra: f64, dec: f64, ra_ms: f64, ra_dir: Option<i8>) -> GuidingRow {
        GuidingRow {
            timestamp_utc: at.to_string(),
            source: "phd2".to_string(),
            ra_offset_arcsec: Some(ra),
            dec_offset_arcsec: Some(dec),
            ra_duration_ms: Some(ra_ms),
            dec_duration_ms: Some(0.0),
            ra_direction: ra_dir,
            ..GuidingRow::default()
        }
    }

    #[test]
    fn samples_become_one_point_per_ten_seconds_with_signed_corrections() {
        let rows = vec![
            row("2026-08-10T21:00:03.000Z", 0.4, -0.2, 120.0, Some(1)),
            row("2026-08-10T21:00:06.000Z", -0.8, 0.2, 50.0, Some(-1)),
            row("2026-08-10T21:00:09.000Z", 0.1, 0.0, 0.0, None),
            // 21:00:10 opens the next window; 21:00:30 the fourth (the third
            // holds nothing and is not written).
            row("2026-08-10T21:00:12.000Z", 6.0, 4.0, 300.0, Some(-1)),
            row("2026-08-10T21:00:31.000Z", 0.2, 0.2, 80.0, None),
        ];
        let counted: HashSet<&str> = [
            "2026-08-10T21:00:03.000Z",
            "2026-08-10T21:00:06.000Z",
            "2026-08-10T21:00:09.000Z",
            "2026-08-10T21:00:31.000Z",
        ]
        .into_iter()
        .collect();
        let (origin, points) = trace_points(&rows, &counted);
        assert_eq!(
            format_iso8601_utc_millis(origin),
            "2026-08-10T21:00:00.000Z"
        );
        assert_eq!(points.len(), 3);
        assert_eq!(points[0].t, 0);
        assert_eq!(points[0].n, 3);
        assert_eq!(points[0].out, 0);
        assert!((points[0].ra - (-0.1)).abs() < 1e-9);
        assert_eq!(points[0].ra_peak, -0.8);
        // 120 ms east, 50 ms west.
        assert_eq!(points[0].ra_pulse_ms, Some(70));
        assert_eq!(points[0].dec_pulse_ms, Some(0));
        // The settle after a dither: out of the statistics, not a spike.
        assert_eq!(points[1].t, 10);
        assert_eq!((points[1].n, points[1].out), (1, 1));
        assert_eq!(points[1].ra_pulse_ms, Some(-300));
        // A pulse whose direction the log did not write: unknown, not zero.
        assert_eq!(points[2].t, 30);
        assert_eq!(points[2].ra_pulse_ms, None);
    }

    fn event(kind: &str, at: &str, source: &str, data: serde_json::Value) -> SessionEvent {
        SessionEvent {
            event_type: kind.to_string(),
            timestamp_utc: at.to_string(),
            source: source.to_string(),
            data,
        }
    }

    #[test]
    fn a_dither_told_by_phd2_and_the_asiair_is_one_dither() {
        let events = vec![
            event("dither", "2026-08-10T21:00:00.000Z", "phd2", json!({})),
            event("dither", "2026-08-10T21:00:04.000Z", "asiair", json!({})),
            // An ASIAIR dither PHD2 did not log stays.
            event("dither", "2026-08-10T21:30:00.000Z", "asiair", json!({})),
        ];
        let kept = one_dither_per_dither(&events);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].source, "phd2");
        assert_eq!(kept[1].timestamp_utc, "2026-08-10T21:30:00.000Z");
    }

    #[test]
    fn markers_carry_the_settle_the_pause_the_focus_and_the_flip_with_their_end() {
        let events = vec![
            event("dither", "2026-08-10T21:00:00.000Z", "phd2", json!({})),
            event(
                "dither-settle",
                "2026-08-10T21:00:00.000Z",
                "phd2",
                json!({"outcome": "started"}),
            ),
            event(
                "dither-settle",
                "2026-08-10T21:00:25.000Z",
                "phd2",
                json!({"outcome": "complete"}),
            ),
            event("star-lost", "2026-08-10T22:00:00.000Z", "phd2", json!({})),
            event("star-lost", "2026-08-10T22:00:10.000Z", "phd2", json!({})),
            event("pause", "2026-08-10T23:00:00.000Z", "phd2", json!({})),
            event("resume", "2026-08-10T23:05:00.000Z", "phd2", json!({})),
            event("autofocus", "2026-08-10T23:10:00.000Z", "asiair", json!({})),
            event(
                "autofocus-result",
                "2026-08-10T23:12:00.000Z",
                "asiair",
                json!({"succeeded": true}),
            ),
        ];
        let flip_from = parse_iso8601_utc_to_unix_seconds("2026-08-11T00:00:00Z").unwrap();
        let markers = trace_markers(&events, &[(flip_from, flip_from + 300.0)]);
        let kinds: Vec<&str> = markers.iter().map(|m| m.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec![
                "dither",
                "settle",
                "star-lost",
                "pause",
                "autofocus",
                "meridian-flip"
            ]
        );
        assert_eq!(markers[1].end.as_deref(), Some("2026-08-10T21:00:25.000Z"));
        assert_eq!(markers[3].end.as_deref(), Some("2026-08-10T23:05:00.000Z"));
        assert_eq!(markers[4].end.as_deref(), Some("2026-08-10T23:12:00.000Z"));
        assert_eq!(markers[5].end.as_deref(), Some("2026-08-11T00:05:00.000Z"));
    }
}

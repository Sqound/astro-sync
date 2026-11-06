//! The NINA log reader.
//!
//! NINA writes one line per message, `date|level|file|member|line|text`,
//! the date in the computer's local time without a zone
//! (`2026-10-07T21:10:03.5120`): it is read with the fixed UTC offset of the
//! settings, like PHD2 and ASIAIR. Only the text is interpreted, by the
//! phrases below, case ignored; every other line is skipped without an error
//! (a NINA log is mostly about the equipment). The phrases are those of NINA
//! 3; the exact words of the installed version are checked on real logs by
//! the lot of real data, and a phrase this reader does not know only means
//! one fact fewer, never a wrong one.
//!
//! | Text of the message (case ignored)                                | What it gives                                    |
//! | ----------------------------------------------------------------- | ------------------------------------------------ |
//! | `Starting Exposure - Exposure Time: 180s; Filter: Ha; Gain: 100;` | a frame: exposure, filter, gain (target window)  |
//! | `Starting target <name>` / `Target: <name>` of a « Starting » line | the target window opens (`target-started`)       |
//! | `Finishing target <name>` / `Target finished`                      | the target window closes (`target-finished`)     |
//! | `Starting AutoFocus` / `AutoFocus … started`                      | a focus run launched (`autofocus`)               |
//! | `AutoFocus completed` / `… successful` / `AutoFocus failed`       | its result (`autofocus-result`, `succeeded`)     |
//! | `Meridian Flip` with `start`, `initiat` or `perform`               | a flip (`meridian-flip`)                         |
//! | `Dither` with `start` or `Dithering`                               | a dither (`dither`)                              |
//!
//! The raw log never leaves the computer: only these facts, in the
//! report v2 (`source: "nina"`).

use crate::time_util::{
    format_iso8601_utc_millis, local_datetime_to_unix_seconds, parse_fixed_offset_seconds,
};
use serde_json::json;
use std::fmt;

use super::SessionEvent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NinaError {
    InvalidUtcOffset(String),
}

impl fmt::Display for NinaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NinaError::InvalidUtcOffset(offset) => {
                write!(f, "invalid UTC offset {offset:?} (expected +HH:MM)")
            }
        }
    }
}

impl std::error::Error for NinaError {}

#[derive(Debug, Clone)]
pub struct NinaOptions {
    /// Fixed UTC offset of the computer that wrote the log (`+HH:MM`).
    pub utc_offset: String,
}

impl Default for NinaOptions {
    fn default() -> Self {
        Self {
            utc_offset: "+00:00".to_string(),
        }
    }
}

/// One frame NINA started, with what the message says of it.
#[derive(Debug, Clone, PartialEq)]
pub struct NinaExposure {
    pub timestamp_utc: String,
    pub unix_seconds: f64,
    /// The target whose window holds this frame, when one is open.
    pub target: Option<String>,
    pub filter: Option<String>,
    pub exposure_seconds: f64,
    pub gain: Option<i64>,
}

/// A target window: from `target-started` to `target-finished` (or the next
/// target, or the end of the log).
#[derive(Debug, Clone, PartialEq)]
pub struct NinaTargetWindow {
    pub target: String,
    pub started_unix: f64,
    pub finished_unix: Option<f64>,
}

impl NinaTargetWindow {
    /// Whether an instant falls in this window.
    #[must_use]
    pub fn holds(&self, unix_seconds: f64) -> bool {
        unix_seconds >= self.started_unix
            && self
                .finished_unix
                .is_none_or(|finished| unix_seconds <= finished)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct NinaLog {
    pub exposures: Vec<NinaExposure>,
    pub windows: Vec<NinaTargetWindow>,
    pub events: Vec<SessionEvent>,
}

impl NinaLog {
    /// The target NINA was imaging at `unix_seconds` (the last window that
    /// holds it), for a frame whose header names none.
    #[must_use]
    pub fn target_at(&self, unix_seconds: f64) -> Option<&str> {
        self.windows
            .iter()
            .rev()
            .find(|window| window.holds(unix_seconds))
            .map(|window| window.target.as_str())
    }
}

/// `2026-10-07T21:10:03.5120` read with the offset; `None` for anything else.
fn parse_nina_datetime(text: &str, offset_seconds: i64) -> Option<f64> {
    let (date, time) = text.trim().split_once('T')?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    let mut time_parts = time.split(':');
    let hour: u32 = time_parts.next()?.parse().ok()?;
    let minute: u32 = time_parts.next()?.parse().ok()?;
    let second: f64 = time_parts.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 {
        return None;
    }
    Some(local_datetime_to_unix_seconds(
        year,
        month,
        day,
        hour,
        minute,
        second,
        offset_seconds,
    ))
}

/// The value after `label` up to the next `;` (`Filter: Ha;` → `Ha`).
fn field_after<'a>(text: &'a str, lower: &str, label: &str) -> Option<&'a str> {
    let start = lower.find(label)? + label.len();
    let rest = text.get(start..)?;
    let value = rest.split(';').next()?.trim();
    (!value.is_empty()).then_some(value)
}

fn parse_seconds(value: &str) -> Option<f64> {
    let number: String = value
        .trim()
        .trim_end_matches(['s', 'S'])
        .trim()
        .replace(',', ".");
    number.parse::<f64>().ok().filter(|v| *v > 0.0)
}

fn parse_exposure(text: &str, lower: &str) -> Option<(f64, Option<String>, Option<i64>)> {
    if !lower.contains("starting exposure") {
        return None;
    }
    let exposure = parse_seconds(field_after(text, lower, "exposure time:")?)?;
    let filter = field_after(text, lower, "filter:")
        .map(str::to_string)
        .filter(|f| !f.eq_ignore_ascii_case("none"));
    let gain = field_after(text, lower, "gain:").and_then(|g| g.parse::<i64>().ok());
    Some((exposure, filter, gain))
}

/// The target a « starting target » message names.
fn target_started(text: &str, lower: &str) -> Option<String> {
    for label in ["starting target ", "starting target:", "target started:"] {
        if let Some(index) = lower.find(label) {
            let name = text
                .get(index + label.len()..)?
                .trim()
                .trim_matches(['\'', '"']);
            let name = name.split(';').next()?.trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    if lower.starts_with("starting")
        && let Some(name) = field_after(text, lower, "target:")
    {
        let name = name.split(',').next()?.trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    None
}

fn target_finished(lower: &str) -> bool {
    lower.contains("finishing target") || lower.contains("target finished")
}

/// Reads a NINA log (`contents`); lines that are not NINA's are skipped.
pub fn parse_nina_log(contents: &str, options: &NinaOptions) -> Result<NinaLog, NinaError> {
    let offset_seconds = parse_fixed_offset_seconds(&options.utc_offset)
        .ok_or_else(|| NinaError::InvalidUtcOffset(options.utc_offset.clone()))?;
    let mut log = NinaLog::default();
    let mut open: Option<NinaTargetWindow> = None;

    for raw in contents.lines() {
        let line = raw.trim_end_matches('\r');
        let fields: Vec<&str> = line.splitn(6, '|').collect();
        if fields.len() < 6 {
            continue;
        }
        let Some(unix) = parse_nina_datetime(fields[0], offset_seconds) else {
            continue;
        };
        let text = fields[5].trim();
        // ASCII only: the byte offsets of `lower` stay those of `text`.
        let lower = text.to_ascii_lowercase();
        let at = format_iso8601_utc_millis(unix);
        let event = |event_type: &str, data: serde_json::Value| SessionEvent {
            event_type: event_type.to_string(),
            timestamp_utc: at.clone(),
            source: "nina".to_string(),
            data,
        };

        if let Some((exposure_seconds, filter, gain)) = parse_exposure(text, &lower) {
            log.exposures.push(NinaExposure {
                timestamp_utc: at.clone(),
                unix_seconds: unix,
                target: open.as_ref().map(|window| window.target.clone()),
                filter,
                exposure_seconds,
                gain,
            });
            continue;
        }
        if let Some(target) = target_started(text, &lower) {
            if let Some(mut previous) = open.take() {
                previous.finished_unix = Some(unix);
                log.windows.push(previous);
            }
            log.events
                .push(event("target-started", json!({ "target": target })));
            open = Some(NinaTargetWindow {
                target,
                started_unix: unix,
                finished_unix: None,
            });
            continue;
        }
        if target_finished(&lower) {
            if let Some(mut window) = open.take() {
                window.finished_unix = Some(unix);
                log.events.push(event(
                    "target-finished",
                    json!({ "target": window.target.clone() }),
                ));
                log.windows.push(window);
            }
            continue;
        }
        if lower.contains("autofocus") || lower.contains("auto focus") {
            if lower.contains("fail") {
                log.events
                    .push(event("autofocus-result", json!({ "succeeded": false })));
            } else if lower.contains("complete")
                || lower.contains("success")
                || lower.contains("finished")
            {
                log.events
                    .push(event("autofocus-result", json!({ "succeeded": true })));
            } else if lower.contains("start") {
                log.events.push(event("autofocus", json!({})));
            }
            continue;
        }
        if lower.contains("meridian flip")
            && (lower.contains("start") || lower.contains("initiat") || lower.contains("perform"))
        {
            log.events.push(event("meridian-flip", json!({})));
            continue;
        }
        if lower.contains("dithering") || (lower.contains("dither") && lower.contains("start")) {
            log.events.push(event("dither", json!({})));
        }
    }
    if let Some(window) = open.take() {
        log.windows.push(window);
    }
    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "\
DATE|LEVEL|SOURCE|MEMBER|LINE|MESSAGE
2026-10-07T21:05:00.0000|INFO|SequenceItem.cs|Run|50|Starting target NGC 7000
2026-10-07T21:06:00.0000|INFO|AutoFocusVM.cs|StartAutoFocus|120|Starting AutoFocus
2026-10-07T21:09:00.0000|INFO|AutoFocusVM.cs|StartAutoFocus|210|AutoFocus completed
2026-10-07T21:10:00.0000|INFO|CameraVM.cs|Capture|88|Starting Exposure - Exposure Time: 180s; Filter: Ha; Gain: 100; Offset: 30; Binning: 1x1;
2026-10-07T21:13:05.0000|INFO|ImagingVM.cs|Dither|40|Dithering
2026-10-07T21:13:30.0000|INFO|CameraVM.cs|Capture|88|Starting Exposure - Exposure Time: 180s; Filter: Ha; Gain: 100; Offset: 30; Binning: 1x1;
2026-10-07T22:00:00.0000|INFO|MeridianFlipVM.cs|DoFlip|12|Meridian Flip - Initiating meridian flip
2026-10-07T22:30:00.0000|INFO|SequenceItem.cs|Run|60|Finishing target NGC 7000
2026-10-07T22:31:00.0000|DEBUG|FocuserVM.cs|Move|10|Moving focuser to position 12000
not a NINA line at all
";

    #[test]
    fn reads_frames_targets_and_events_with_the_offset() {
        let log = parse_nina_log(
            LOG,
            &NinaOptions {
                utc_offset: "+02:00".into(),
            },
        )
        .unwrap();
        assert_eq!(log.exposures.len(), 2);
        let first = &log.exposures[0];
        assert_eq!(first.timestamp_utc, "2026-10-07T19:10:00.000Z");
        assert_eq!(first.target.as_deref(), Some("NGC 7000"));
        assert_eq!(first.filter.as_deref(), Some("Ha"));
        assert_eq!(first.exposure_seconds, 180.0);
        assert_eq!(first.gain, Some(100));
        let types: Vec<&str> = log.events.iter().map(|e| e.event_type.as_str()).collect();
        assert_eq!(
            types,
            [
                "target-started",
                "autofocus",
                "autofocus-result",
                "dither",
                "meridian-flip",
                "target-finished"
            ]
        );
        assert!(log.events.iter().all(|e| e.source == "nina"));
        assert_eq!(log.windows.len(), 1);
        let inside = log.exposures[1].unix_seconds;
        assert_eq!(log.target_at(inside), Some("NGC 7000"));
        assert_eq!(log.target_at(inside + 7_200.0), None);
    }

    #[test]
    fn a_failed_focus_run_and_a_bad_offset() {
        let log = parse_nina_log(
            "2026-10-07T23:00:00.0000|WARNING|AutoFocusVM.cs|Run|9|AutoFocus failed: not enough stars\n",
            &NinaOptions::default(),
        )
        .unwrap();
        assert_eq!(log.events[0].data["succeeded"], false);
        assert!(matches!(
            parse_nina_log(
                "",
                &NinaOptions {
                    utc_offset: "UTC+9".into()
                }
            ),
            Err(NinaError::InvalidUtcOffset(_))
        ));
    }
}

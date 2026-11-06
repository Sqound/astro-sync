//! PHD2 guide-log parser.
//!
//! Targets the plain-text `PHD2_GuideLog_*.txt` format PHD2 itself writes
//! (not its JSON event-server protocol, which is a different interface to
//! the same underlying events). One log file can contain
//! several guiding sections (PHD2 appends a new `Guiding Begins at`/
//! `Guiding Ends at` block every time guiding is (re)started, e.g. after a
//! meridian flip or an app restart); each becomes one [`GuidingSession`].
//!
//! **Known v1 limitation, recorded here and in the parameters document:**
//! PHD2's inline `INFO:` marker lines (settling, star lost, pause/resume)
//! do not carry their own independently parseable timestamp column the way
//! guide-frame data rows do (a `Time` column of seconds elapsed since
//! `Guiding Begins at`). This parser timestamps each marker using the
//! elapsed time of the most recently parsed data row *before* it (falling
//! back to the session start if the marker appears before the first data
//! row), and keeps the bracketing frame numbers in the event so that
//! approximation is visible to a reader rather than presented as exact.

use crate::time_util::{
    format_iso8601_utc_millis, local_datetime_to_unix_seconds, parse_fixed_offset_seconds,
};
use std::fmt;

#[derive(Debug, PartialEq)]
pub enum Phd2Error {
    /// `options.utc_offset` is not a valid `"+HH:MM"`/`"-HH:MM"` string.
    InvalidUtcOffset(String),
    /// A `"Guiding Begins at "` / `"Guiding Ends at "` line's timestamp
    /// could not be parsed as `"YYYY-MM-DD HH:MM:SS"`.
    InvalidTimestamp(String),
}

impl fmt::Display for Phd2Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Phd2Error::InvalidUtcOffset(raw) => write!(f, "invalid UTC offset {raw:?}"),
            Phd2Error::InvalidTimestamp(raw) => write!(f, "invalid timestamp {raw:?}"),
        }
    }
}

impl std::error::Error for Phd2Error {}

#[derive(Debug, Clone)]
pub struct Phd2Options {
    /// Fixed UTC offset the log's local wall-clock timestamps are in
    /// (e.g. `"+09:00"`). PHD2 logs carry no timezone of their own.
    pub utc_offset: String,
}

impl Default for Phd2Options {
    fn default() -> Self {
        Self {
            utc_offset: "+00:00".to_string(),
        }
    }
}

/// Why a guide frame is left out of the guiding statistics (the RMS of a
/// night, of a slice, of a frame's exposure), the way PHD2 leaves it out of
/// its own: it does not describe how well the mount tracks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleExclusion {
    /// Taken while the guider settles after a dither (or at the start of a
    /// guiding section): the star is being walked to its new position, so
    /// the offset is the move, not an error.
    Settling,
    /// The first frame after a lost star: the guider has just found it again.
    Reacquired,
    /// PHD2 gave the frame a non-zero `ErrorCode`.
    ErrorCode,
}

/// One parsed guide-frame data row (a "pose" in the plan's wording).
/// `ra_offset_arcsec`/`dec_offset_arcsec` are `RARawDistance`/
/// `DECRawDistance` (guide-camera pixels) converted with the log's own
/// declared pixel scale -- the star's actual measured drift, not the
/// (possibly clamped) correction PHD2 issued.
#[derive(Debug, Clone, PartialEq)]
pub struct GuideSample {
    pub timestamp_utc: String,
    pub frame: Option<u32>,
    /// The raw `Time` column value: seconds elapsed since this session's
    /// `Guiding Begins at` -- the literal source figure `timestamp_utc` was
    /// derived from.
    pub elapsed_seconds: f64,
    pub ra_offset_arcsec: f64,
    pub dec_offset_arcsec: f64,
    pub ra_duration_ms: Option<f64>,
    pub dec_duration_ms: Option<f64>,
    /// The way the right-ascension pulse went (`RADirection`): `+1` east,
    /// `-1` west, `None` without a pulse or a direction PHD2 did not write.
    /// The sign of the raw offsets is not a safe datum from one PHD2 version
    /// to the next; the direction of the pulses is.
    pub ra_direction: Option<i8>,
    /// The way the declination pulse went (`DECDirection`): `+1` north,
    /// `-1` south.
    pub dec_direction: Option<i8>,
    pub star_mass: Option<f64>,
    pub snr: Option<f64>,
    /// `ErrorCode` as recorded (`0` on a clean frame). Kept on every
    /// sample rather than silently dropping non-zero rows, so no source
    /// value ever disappears between the parser and `metrics/guiding.csv`;
    /// which rows a report or RMS aggregate then chooses to use is a
    /// separate, documented decision (`metrics.md`), never a silent one.
    pub error_code: i64,
    /// Why the frame stays out of the guiding statistics; `None` for a frame
    /// that counts. The frame itself is kept (`metrics/guiding.csv`).
    pub exclusion: Option<SampleExclusion>,
    /// The ordinal (in this section) of the dither whose recovery this frame
    /// belongs to: taken within [`DITHER_RECOVERY_SECONDS`] after the dither.
    /// After the settle window the guider may still be walking back (a
    /// declination axis with a slow aggressiveness takes minutes), so the
    /// statistics leave such a frame out until the guider is first back
    /// within the noise of the night (`analysis`, which knows the night).
    pub dither_recovery: Option<u32>,
}

/// How long after a dither a frame may still be the guider walking back to
/// its lock position, in seconds.
pub const DITHER_RECOVERY_SECONDS: f64 = 240.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettleOutcome {
    Started,
    Complete,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SettleEvent {
    pub timestamp_utc: String,
    pub outcome: SettleOutcome,
    /// The data row immediately before this marker, if any -- see the
    /// module doc comment's note on inline-marker timestamp precision.
    pub preceding_frame: Option<u32>,
}

/// A guide frame PHD2 dropped (`"DROP"` row, `ErrorCode` other than 0, its
/// message `Star lost - …` or `No star found`), at its own instant: unlike
/// [`StarLostEvent`], which takes the instant of the last good frame, the
/// instant here is the row's own `Time`.
#[derive(Debug, Clone, PartialEq)]
pub struct DroppedFrame {
    pub timestamp_utc: String,
    pub frame: Option<u32>,
    /// The message PHD2 gives (`Star lost - mass changed`, `No star found`).
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StarLostEvent {
    pub timestamp_utc: String,
    pub status_text: Option<String>,
    pub preceding_frame: Option<u32>,
}

/// A dither the imaging software asked for (`INFO: DITHER by …`): counted in
/// the night's guiding summary.
#[derive(Debug, Clone, PartialEq)]
pub struct DitherEvent {
    pub timestamp_utc: String,
    pub preceding_frame: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseKind {
    /// An explicit `"Guiding paused"`/`"Guiding resumed"` marker inside one
    /// continuous `Guiding Begins`/`Guiding Ends` block.
    ExplicitMarker,
    /// A full stop/restart: this session's `Guiding Ends at` followed later
    /// in the same file by the next session's `Guiding Begins at`.
    SessionGap,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PauseEvent {
    pub paused_at_utc: String,
    pub resumed_at_utc: Option<String>,
    pub kind: PauseKind,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GuidingSession {
    pub started_at_utc: Option<String>,
    pub ended_at_utc: Option<String>,
    pub pixel_scale_arcsec_per_px: Option<f64>,
    pub samples: Vec<GuideSample>,
    pub settle_events: Vec<SettleEvent>,
    pub star_lost_events: Vec<StarLostEvent>,
    pub dither_events: Vec<DitherEvent>,
    /// Additive: the frames PHD2 dropped, one per `"DROP"` row.
    pub dropped_frames: Vec<DroppedFrame>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Phd2Log {
    pub sessions: Vec<GuidingSession>,
    /// Pauses spanning two sessions in this file ([`PauseKind::SessionGap`])
    /// live here rather than inside either session, since they belong to
    /// neither one alone.
    pub pauses: Vec<PauseEvent>,
}

fn parse_pixel_scale(line: &str) -> Option<f64> {
    let after = line.strip_prefix("Pixel scale = ")?;
    let number_text: String = after
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    number_text.parse().ok()
}

/// Parses `"YYYY-MM-DD HH:MM:SS"` (PHD2's own timestamp shape for
/// `Guiding Begins at`/`Guiding Ends at`) plus the caller's fixed UTC
/// offset into a Unix timestamp.
fn parse_phd2_datetime(text: &str, offset_seconds: i64) -> Option<f64> {
    let text = text.trim();
    let (date, time) = text.split_once(' ')?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    let mut time_parts = time.split(':');
    let hour: u32 = time_parts.next()?.parse().ok()?;
    let minute: u32 = time_parts.next()?.parse().ok()?;
    let second: f64 = time_parts.next()?.parse().ok()?;
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

struct SessionBuilder {
    started_at_unix: Option<f64>,
    started_at_utc: Option<String>,
    pixel_scale_arcsec_per_px: Option<f64>,
    samples: Vec<GuideSample>,
    settle_events: Vec<SettleEvent>,
    star_lost_events: Vec<StarLostEvent>,
    dither_events: Vec<DitherEvent>,
    dropped_frames: Vec<DroppedFrame>,
    last_frame: Option<u32>,
    last_timestamp_utc: Option<String>,
    /// Between a dither or a `Settling started` and the matching
    /// `Settling complete` / `Settling failed`.
    settling: bool,
    /// The last data line was a dropped frame.
    after_drop: bool,
    /// Dithers met in this section.
    dithers: u32,
    /// The latest dither whose recovery window is open: its ordinal, and the
    /// elapsed time it ends at (`None` until the first frame after the
    /// dither fixes the start).
    recovery: Option<(u32, Option<f64>)>,
}

impl SessionBuilder {
    fn new(started_at_unix: f64, started_at_utc: String, pixel_scale: Option<f64>) -> Self {
        Self {
            started_at_unix: Some(started_at_unix),
            started_at_utc: Some(started_at_utc.clone()),
            pixel_scale_arcsec_per_px: pixel_scale,
            samples: Vec::new(),
            settle_events: Vec::new(),
            star_lost_events: Vec::new(),
            dither_events: Vec::new(),
            dropped_frames: Vec::new(),
            last_frame: None,
            last_timestamp_utc: Some(started_at_utc),
            settling: false,
            after_drop: false,
            dithers: 0,
            recovery: None,
        }
    }

    fn marker_timestamp(&self) -> String {
        self.last_timestamp_utc.clone().unwrap_or_default()
    }

    fn finish(self, ended_at_utc: Option<String>) -> GuidingSession {
        GuidingSession {
            started_at_utc: self.started_at_utc,
            ended_at_utc,
            pixel_scale_arcsec_per_px: self.pixel_scale_arcsec_per_px,
            samples: self.samples,
            settle_events: self.settle_events,
            star_lost_events: self.star_lost_events,
            dither_events: self.dither_events,
            dropped_frames: self.dropped_frames,
        }
    }
}

/// Splits a PHD2 guide-frame CSV data row into its raw fields. Real PHD2
/// rows are never expected to carry an embedded comma inside a field
/// (the only text field, `mount`, is a bare device name), so a plain
/// `split(',')` is deliberately used instead of a full CSV parser.
fn split_data_row(line: &str) -> Option<Vec<&str>> {
    let first_comma = line.find(',')?;
    if !line[..first_comma].chars().all(|c| c.is_ascii_digit()) {
        return None; // not a "Frame,Time,..." row (frame numbers are integers)
    }
    Some(line.split(',').collect())
}

fn parse_f64_field(fields: &[&str], index: usize) -> Option<f64> {
    fields.get(index)?.trim().parse().ok()
}

/// `RADirection` / `DECDirection`: `E`/`East` and `N`/`North` are `+1`,
/// `W`/`West` and `S`/`South` `-1` (PHD2 wrote the full word in older
/// logs, the letter since); an empty field or any other text is `None`.
fn parse_direction(fields: &[&str], index: usize) -> Option<i8> {
    let text = fields.get(index)?.trim().trim_matches('"');
    match text.chars().next()?.to_ascii_uppercase() {
        'E' | 'N' => Some(1),
        'W' | 'S' => Some(-1),
        _ => None,
    }
}

/// `12,35.761,"DROP",,,…,"Star lost - mass changed"`: the frame, at its own
/// `Time`.
fn dropped_frame(line: &str, builder: &SessionBuilder) -> Option<DroppedFrame> {
    let fields: Vec<&str> = line.split(',').collect();
    if fields.get(2).map(|f| f.trim()) != Some("\"DROP\"") {
        return None;
    }
    let frame = fields.first()?.trim().parse::<u32>().ok()?;
    let elapsed: f64 = fields.get(1)?.trim().parse().ok()?;
    let timestamp = builder.started_at_unix? + elapsed;
    let message = fields
        .last()
        .map(|f| f.trim().trim_matches('"').to_string())
        .filter(|m| !m.is_empty() && m.chars().any(|c| c.is_alphabetic()));
    Some(DroppedFrame {
        timestamp_utc: format_iso8601_utc_millis(timestamp),
        frame: Some(frame),
        message,
    })
}

/// Parses a full PHD2 `PHD2_GuideLog_*.txt` file, possibly containing
/// several guiding sections.
pub fn parse_phd2_guide_log(source: &str, options: &Phd2Options) -> Result<Phd2Log, Phd2Error> {
    let offset_seconds = parse_fixed_offset_seconds(&options.utc_offset)
        .ok_or_else(|| Phd2Error::InvalidUtcOffset(options.utc_offset.clone()))?;

    let mut sessions = Vec::new();
    let mut pauses = Vec::new();
    let mut current: Option<SessionBuilder> = None;
    let mut pixel_scale: Option<f64> = None;
    let mut pending_gap_start: Option<String> = None;

    for raw_line in source.lines() {
        let line = raw_line.trim_end_matches('\r');

        if let Some(scale) = parse_pixel_scale(line) {
            pixel_scale = Some(scale);
            // The "Pixel scale = " summary line is written just after
            // "Guiding Begins at" in a real PHD2 log, i.e. *inside* the
            // session it describes -- update the in-progress session too,
            // not only the outer tracker used for the next one.
            if let Some(builder) = current.as_mut() {
                builder.pixel_scale_arcsec_per_px = Some(scale);
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("Guiding Begins at ") {
            let start_unix = parse_phd2_datetime(rest, offset_seconds)
                .ok_or_else(|| Phd2Error::InvalidTimestamp(rest.trim().to_string()))?;
            let started_at_utc = format_iso8601_utc_millis(start_unix);
            if let Some(gap_start) = pending_gap_start.take() {
                pauses.push(PauseEvent {
                    paused_at_utc: gap_start,
                    resumed_at_utc: Some(started_at_utc.clone()),
                    kind: PauseKind::SessionGap,
                });
            }
            if let Some(builder) = current.take() {
                sessions.push(builder.finish(None));
            }
            current = Some(SessionBuilder::new(start_unix, started_at_utc, pixel_scale));
            continue;
        }

        if let Some(rest) = line.strip_prefix("Guiding Ends at ") {
            let end_unix = parse_phd2_datetime(rest, offset_seconds)
                .ok_or_else(|| Phd2Error::InvalidTimestamp(rest.trim().to_string()))?;
            let ended_at_utc = format_iso8601_utc_millis(end_unix);
            if let Some(builder) = current.take() {
                sessions.push(builder.finish(Some(ended_at_utc.clone())));
            }
            pending_gap_start = Some(ended_at_utc);
            continue;
        }

        if line.contains("SETTLING STATE CHANGE") {
            let outcome = if line.contains("Settling started") {
                Some(SettleOutcome::Started)
            } else if line.contains("Settling complete") {
                Some(SettleOutcome::Complete)
            } else if line.contains("Settling failed") {
                Some(SettleOutcome::Failed)
            } else {
                None
            };
            if let (Some(outcome), Some(builder)) = (outcome, current.as_mut()) {
                builder.settling = outcome == SettleOutcome::Started;
                builder.settle_events.push(SettleEvent {
                    timestamp_utc: builder.marker_timestamp(),
                    outcome,
                    preceding_frame: builder.last_frame,
                });
            }
            continue;
        }

        if line.contains("INFO: DITHER by") {
            if let Some(builder) = current.as_mut() {
                // The star jumps to its new lock position at once, before
                // PHD2 reports the settling.
                builder.settling = true;
                builder.dithers += 1;
                builder.recovery = Some((builder.dithers, None));
                builder.dither_events.push(DitherEvent {
                    timestamp_utc: builder.marker_timestamp(),
                    preceding_frame: builder.last_frame,
                });
            }
            continue;
        }

        // Every dropped frame, whatever PHD2 says of it (`Star lost - …`,
        // `No star found`): its own instant. The marker below keeps its old
        // reading.
        if let Some(builder) = current.as_mut()
            && let Some(dropped) = dropped_frame(line, builder)
        {
            builder.dropped_frames.push(dropped);
            builder.after_drop = true;
        }

        if line.to_ascii_uppercase().contains("STAR LOST") {
            if let Some(builder) = current.as_mut() {
                let status_text = line
                    .split_once("STAR LOST")
                    .map(|(_, rest)| rest.trim_start_matches([':', ' ']).trim())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                builder.star_lost_events.push(StarLostEvent {
                    timestamp_utc: builder.marker_timestamp(),
                    status_text,
                    preceding_frame: builder.last_frame,
                });
            }
            continue;
        }

        if let Some(fields) = split_data_row(line) {
            let Some(builder) = current.as_mut() else {
                continue; // a data row outside any "Guiding Begins" block: ignore
            };
            let Some(frame) = fields.first().and_then(|f| f.trim().parse::<u32>().ok()) else {
                continue;
            };
            let Some(elapsed_seconds) = parse_f64_field(&fields, 1) else {
                continue;
            };
            let Some(ra_raw) = parse_f64_field(&fields, 5) else {
                continue;
            };
            let Some(dec_raw) = parse_f64_field(&fields, 6) else {
                continue;
            };
            let scale = builder.pixel_scale_arcsec_per_px.unwrap_or(1.0);
            let timestamp_unix = builder.started_at_unix.map(|start| start + elapsed_seconds);
            let timestamp_utc = timestamp_unix
                .map(format_iso8601_utc_millis)
                .unwrap_or_default();
            builder.last_frame = Some(frame);
            builder.last_timestamp_utc = Some(timestamp_utc.clone());
            let error_code = fields
                .get(17)
                .and_then(|f| f.trim().parse::<i64>().ok())
                .unwrap_or(0);
            let exclusion = if error_code != 0 {
                Some(SampleExclusion::ErrorCode)
            } else if builder.settling {
                Some(SampleExclusion::Settling)
            } else if builder.after_drop {
                Some(SampleExclusion::Reacquired)
            } else {
                None
            };
            builder.after_drop = false;
            let dither_recovery = match builder.recovery {
                Some((id, until)) => {
                    let until = until.unwrap_or(elapsed_seconds + DITHER_RECOVERY_SECONDS);
                    if elapsed_seconds <= until {
                        builder.recovery = Some((id, Some(until)));
                        Some(id)
                    } else {
                        builder.recovery = None;
                        None
                    }
                }
                None => None,
            };
            builder.samples.push(GuideSample {
                timestamp_utc,
                frame: Some(frame),
                elapsed_seconds,
                ra_offset_arcsec: ra_raw * scale,
                dec_offset_arcsec: dec_raw * scale,
                ra_duration_ms: parse_f64_field(&fields, 9),
                dec_duration_ms: parse_f64_field(&fields, 11),
                ra_direction: parse_direction(&fields, 10),
                dec_direction: parse_direction(&fields, 12),
                star_mass: parse_f64_field(&fields, 15),
                snr: parse_f64_field(&fields, 16),
                error_code,
                exclusion,
                dither_recovery,
            });
        }
    }

    if let Some(builder) = current.take() {
        sessions.push(builder.finish(None));
    }

    Ok(Phd2Log { sessions, pauses })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(offset: &str) -> Phd2Options {
        Phd2Options {
            utc_offset: offset.to_string(),
        }
    }

    #[test]
    fn parses_a_short_single_section_log() {
        let source = [
            "PHD2 version 2.6.13, Log version 2.5. Log enabled at 2026-08-07 22:37:00",
            "",
            "Guiding Begins at 2026-08-07 22:38:03",
            "",
            "Pixel scale = 2.00 arc-sec/px, Binning = 1, Focal length = 250 mm",
            "Frame,Time,mount,dx,dy,RARawDistance,DECRawDistance,RAGuideDistance,DECGuideDistance,RADuration,RADirection,DECDuration,DECDirection,XStep,YStep,StarMass,SNR,ErrorCode",
            "1,1.201,Mount,-0.043,0.297,-0.020,0.301,0.000,0.000,0,,0,,0,0,32059,12.7,0",
            "2,3.310,Mount,0.010,-0.150,0.400,-0.300,0.000,0.000,120,East,80,North,0,0,31000,11.9,0",
            "INFO: SETTLING STATE CHANGE, Settling started",
            "INFO: SETTLING STATE CHANGE, Settling complete",
            "3,6.410,Mount,0.020,0.050,0.100,0.050,0.000,0.000,0,,0,,0,0,30500,13.1,0",
            "INFO: STAR LOST: SNR too low",
            "Guiding Ends at 2026-08-07 23:41:12",
        ]
        .join("\n");

        let log = parse_phd2_guide_log(&source, &options("+09:00")).unwrap();
        assert_eq!(log.sessions.len(), 1);
        let session = &log.sessions[0];
        assert_eq!(session.pixel_scale_arcsec_per_px, Some(2.0));
        assert_eq!(session.samples.len(), 3);
        assert_eq!(
            session.started_at_utc.as_deref(),
            Some("2026-08-07T13:38:03.000Z")
        );
        assert_eq!(
            session.ended_at_utc.as_deref(),
            Some("2026-08-07T14:41:12.000Z")
        );

        let first = &session.samples[0];
        assert_eq!(first.frame, Some(1));
        assert_eq!(first.elapsed_seconds, 1.201);
        assert_eq!(first.ra_offset_arcsec, -0.040);
        assert_eq!(first.dec_offset_arcsec, 0.602);
        assert_eq!(first.error_code, 0);
        // 1.201s after 22:38:03 local (+09:00) = 13:38:04.201Z.
        assert_eq!(first.timestamp_utc, "2026-08-07T13:38:04.201Z");

        // The way the pulses went is kept: no pulse, then east and north.
        assert_eq!(first.ra_direction, None);
        assert_eq!(session.samples[1].ra_direction, Some(1));
        assert_eq!(session.samples[1].dec_direction, Some(1));
        assert_eq!(session.samples[1].ra_duration_ms, Some(120.0));

        assert_eq!(session.settle_events.len(), 2);
        assert_eq!(session.settle_events[0].outcome, SettleOutcome::Started);
        assert_eq!(session.settle_events[0].preceding_frame, Some(2));
        assert_eq!(session.settle_events[1].outcome, SettleOutcome::Complete);

        assert_eq!(session.star_lost_events.len(), 1);
        assert_eq!(session.star_lost_events[0].preceding_frame, Some(3));
        assert_eq!(
            session.star_lost_events[0].status_text.as_deref(),
            Some("SNR too low")
        );
    }

    #[test]
    fn a_dropped_frame_keeps_its_own_instant_unlike_the_star_lost_marker() {
        let source = [
            "Guiding Begins at 2026-08-14 23:05:13",
            "Pixel scale = 6.88 arc-sec/px",
            "Frame,Time,mount,dx,dy,RARawDistance,DECRawDistance,RAGuideDistance,DECGuideDistance,RADuration,RADirection,DECDuration,DECDirection,XStep,YStep,StarMass,SNR,ErrorCode",
            "8,25.608,\"Mount\",2.965,1.886,2.960,1.864,2.286,1.491,500,W,500,S,,,1962,31.24,0",
            "9,26.764,\"DROP\",,,,,,,,,,,,,509,15.71,6,\"Star lost - mass changed\"",
            "10,29.763,\"DROP\",,,,,,,,,,,,,0,0.00,3,\"Star lost - low mass\"",
            "11,32.763,\"DROP\",,,,,,,,,,,,,478,15.36,7,\"No star found\"",
        ]
        .join("\n");
        let log = parse_phd2_guide_log(&source, &options("+00:00")).unwrap();
        let session = &log.sessions[0];
        assert_eq!(session.samples.len(), 1);
        // `W` and `S`, the letters of PHD2 2.6: west and south.
        assert_eq!(session.samples[0].ra_direction, Some(-1));
        assert_eq!(session.samples[0].dec_direction, Some(-1));
        // A drop that is not a « Star lost » is a dropped frame too.
        assert_eq!(session.dropped_frames.len(), 3);
        assert_eq!(
            session.dropped_frames[2].message.as_deref(),
            Some("No star found")
        );
        assert_eq!(
            session.dropped_frames[0].timestamp_utc,
            "2026-08-14T23:05:39.764Z"
        );
        assert_eq!(session.dropped_frames[1].frame, Some(10));
        assert_eq!(
            session.dropped_frames[1].message.as_deref(),
            Some("Star lost - low mass")
        );
        // The marker of the reader that existed before is unchanged: both
        // rows carry the instant of the last good frame.
        assert_eq!(session.star_lost_events.len(), 2);
        assert_eq!(session.star_lost_events[0].preceding_frame, Some(8));
        assert_eq!(
            session.star_lost_events[0].timestamp_utc,
            session.star_lost_events[1].timestamp_utc
        );
    }

    const HEADER: &str = "Frame,Time,mount,dx,dy,RARawDistance,DECRawDistance,RAGuideDistance,DECGuideDistance,RADuration,RADirection,DECDuration,DECDirection,XStep,YStep,StarMass,SNR,ErrorCode";

    #[test]
    fn the_frames_of_a_settle_a_lost_star_or_an_error_stay_out_of_the_statistics() {
        let source = [
            "Guiding Begins at 2026-08-07 22:38:03",
            "Pixel scale = 2.00 arc-sec/px",
            HEADER,
            "1,1.000,Mount,0,0,0.1,0.1,0,0,0,,0,,0,0,30000,12,0",
            "INFO: DITHER by 3.000, -2.000, new lock pos = 10.0, 10.0",
            "INFO: SETTLING STATE CHANGE, Settling started",
            "2,4.000,Mount,0,0,3.0,-2.0,0,0,0,,0,,0,0,30000,12,0",
            "INFO: SETTLING STATE CHANGE, Settling complete",
            "3,7.000,Mount,0,0,1.5,-1.0,0,0,0,,0,,0,0,30000,12,0",
            "4,10.000,Mount,0,0,0.1,0.1,0,0,0,,0,,0,0,30000,12,7",
            "5,13.000,\"DROP\",,,,,,,,,,,,,0,0.00,3,\"Star lost - low mass\"",
            "6,16.000,Mount,0,0,9.0,9.0,0,0,0,,0,,0,0,30000,12,0",
            "7,19.000,Mount,0,0,0.1,0.1,0,0,0,,0,,0,0,30000,12,0",
        ]
        .join("\n");
        let log = parse_phd2_guide_log(&source, &options("+00:00")).unwrap();
        let samples = &log.sessions[0].samples;
        let exclusions: Vec<_> = samples.iter().map(|s| s.exclusion).collect();
        assert_eq!(
            exclusions,
            vec![
                None,
                Some(SampleExclusion::Settling),
                None,
                Some(SampleExclusion::ErrorCode),
                Some(SampleExclusion::Reacquired),
                None,
            ]
        );
        // The frames after the dither belong to its recovery; the one before
        // it does not.
        let recoveries: Vec<_> = samples.iter().map(|s| s.dither_recovery).collect();
        assert_eq!(
            recoveries,
            vec![None, Some(1), Some(1), Some(1), Some(1), Some(1)]
        );
    }

    #[test]
    fn a_dither_recovery_window_closes_after_its_duration() {
        let source = [
            "Guiding Begins at 2026-08-07 22:38:03",
            "Pixel scale = 2.00 arc-sec/px",
            HEADER,
            "INFO: DITHER by 3.000, -2.000, new lock pos = 10.0, 10.0",
            "1,1.000,Mount,0,0,3.0,-2.0,0,0,0,,0,,0,0,30000,12,0",
            "2,200.000,Mount,0,0,0.1,0.1,0,0,0,,0,,0,0,30000,12,0",
            "3,300.000,Mount,0,0,0.1,0.1,0,0,0,,0,,0,0,30000,12,0",
        ]
        .join("\n");
        let log = parse_phd2_guide_log(&source, &options("+00:00")).unwrap();
        let recoveries: Vec<_> = log.sessions[0]
            .samples
            .iter()
            .map(|s| s.dither_recovery)
            .collect();
        assert_eq!(recoveries, vec![Some(1), Some(1), None]);
    }

    #[test]
    fn a_settling_failed_marker_is_distinct_from_complete() {
        let source = [
            "Guiding Begins at 2026-08-07 22:38:03",
            "Pixel scale = 2.00 arc-sec/px",
            "Frame,Time,mount,dx,dy,RARawDistance,DECRawDistance,RAGuideDistance,DECGuideDistance,RADuration,RADirection,DECDuration,DECDirection,XStep,YStep,StarMass,SNR,ErrorCode",
            "1,1.000,Mount,0,0,0.1,0.1,0,0,0,,0,,0,0,30000,12,0",
            "INFO: SETTLING STATE CHANGE, Settling failed",
        ]
        .join("\n");
        let log = parse_phd2_guide_log(&source, &options("+00:00")).unwrap();
        assert_eq!(
            log.sessions[0].settle_events[0].outcome,
            SettleOutcome::Failed
        );
    }

    #[test]
    fn two_sections_in_one_file_produce_a_session_gap_pause() {
        let source = [
            "Guiding Begins at 2026-08-07 22:00:00",
            "Pixel scale = 2.00 arc-sec/px",
            "Frame,Time,mount,dx,dy,RARawDistance,DECRawDistance,RAGuideDistance,DECGuideDistance,RADuration,RADirection,DECDuration,DECDirection,XStep,YStep,StarMass,SNR,ErrorCode",
            "1,1.000,Mount,0,0,0.1,0.1,0,0,0,,0,,0,0,30000,12,0",
            "Guiding Ends at 2026-08-07 22:30:00",
            "Guiding Begins at 2026-08-07 22:45:00",
            "Pixel scale = 2.00 arc-sec/px",
            "1,1.000,Mount,0,0,0.1,0.1,0,0,0,,0,,0,0,30000,12,0",
            "Guiding Ends at 2026-08-07 23:00:00",
        ]
        .join("\n");
        let log = parse_phd2_guide_log(&source, &options("+00:00")).unwrap();
        assert_eq!(log.sessions.len(), 2);
        assert_eq!(log.pauses.len(), 1);
        assert_eq!(log.pauses[0].kind, PauseKind::SessionGap);
        assert_eq!(log.pauses[0].paused_at_utc, "2026-08-07T22:30:00.000Z");
        assert_eq!(
            log.pauses[0].resumed_at_utc.as_deref(),
            Some("2026-08-07T22:45:00.000Z")
        );
    }

    #[test]
    fn rows_with_a_nonzero_error_code_are_kept_not_dropped() {
        let source = [
            "Guiding Begins at 2026-08-07 22:00:00",
            "Pixel scale = 2.00 arc-sec/px",
            "Frame,Time,mount,dx,dy,RARawDistance,DECRawDistance,RAGuideDistance,DECGuideDistance,RADuration,RADirection,DECDuration,DECDirection,XStep,YStep,StarMass,SNR,ErrorCode",
            "1,1.000,Mount,0,0,0.1,0.1,0,0,0,,0,,0,0,30000,12,7",
        ]
        .join("\n");
        let log = parse_phd2_guide_log(&source, &options("+00:00")).unwrap();
        assert_eq!(log.sessions[0].samples.len(), 1);
        assert_eq!(log.sessions[0].samples[0].error_code, 7);
    }

    #[test]
    fn rejects_an_invalid_utc_offset() {
        let result =
            parse_phd2_guide_log("Guiding Begins at 2026-08-07 22:00:00", &options("bogus"));
        assert_eq!(
            result,
            Err(Phd2Error::InvalidUtcOffset("bogus".to_string()))
        );
    }

    #[test]
    fn an_empty_log_produces_no_sessions() {
        let log = parse_phd2_guide_log("", &options("+00:00")).unwrap();
        assert!(log.sessions.is_empty());
        assert!(log.pauses.is_empty());
    }

    #[test]
    fn a_data_row_before_any_guiding_begins_line_is_ignored() {
        let source = "1,1.000,Mount,0,0,0.1,0.1,0,0,0,,0,,0,0,30000,12,0";
        let log = parse_phd2_guide_log(source, &options("+00:00")).unwrap();
        assert!(log.sessions.is_empty());
    }
}

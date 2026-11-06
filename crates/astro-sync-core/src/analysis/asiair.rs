//! ASIAIR "Autorun" log parser.
//!
//! ASIAIR's own sequencer log (`Autorun_Log_*.txt`) is a different file,
//! in a different format, from the PHD2 guide log it drives underneath
//! (`crate::analysis::phd2`): every line starts with a `YYYY/MM/DD
//! HH:MM:SS` timestamp followed by a bracketed tag (`[Guide]`,
//! `[AutoFocus|Begin]`, `[Meridian Flip|End]`...) describing a system
//! event. It carries no per-frame guiding numbers of its own -- when
//! ASIAIR guides through PHD2 (the usual setup), the guiding series
//! itself always comes from `crate::analysis::phd2`; this module only
//! extracts the operational events that PHD2's own guide log does not
//! carry: dithering commands, settle/settle-timeout, autofocus runs (with
//! sensor temperature), and
//! meridian flips.
//!
//! The line patterns matched below were established from real ASIAIR
//! archives. They are matched here with plain string matching (no `regex` crate: every
//! pattern is a fixed substring or a `[0-9.]+` run, both trivial without
//! one), keeping this crate's dependency list unchanged.

use crate::time_util::{
    format_iso8601_utc_millis, local_datetime_to_unix_seconds, parse_fixed_offset_seconds,
};
use std::fmt;

#[derive(Debug, PartialEq)]
pub enum AsiairError {
    InvalidUtcOffset(String),
}

impl fmt::Display for AsiairError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AsiairError::InvalidUtcOffset(raw) => write!(f, "invalid UTC offset {raw:?}"),
        }
    }
}

impl std::error::Error for AsiairError {}

#[derive(Debug, Clone)]
pub struct AsiairOptions {
    pub utc_offset: String,
}

impl Default for AsiairOptions {
    fn default() -> Self {
        Self {
            utc_offset: "+00:00".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DitherEvent {
    pub timestamp_utc: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SettleEvent {
    pub timestamp_utc: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SettleTimeoutEvent {
    pub timestamp_utc: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AutofocusEvent {
    pub timestamp_utc: String,
    pub temperature_c: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AutofocusResultEvent {
    pub timestamp_utc: String,
    pub succeeded: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MeridianFlipEvent {
    pub timestamp_utc: String,
}

/// `[Meridian Flip|Begin]`: the sequencer stops guiding and tracking and
/// waits for the mount to reach the flip.
#[derive(Debug, Clone, PartialEq)]
pub struct MeridianFlipBeginEvent {
    pub timestamp_utc: String,
}

/// `[Meridian Flip|End]`, whatever its result (`meridian_flip_events` keeps
/// only the ones that succeeded).
#[derive(Debug, Clone, PartialEq)]
pub struct MeridianFlipEndEvent {
    pub timestamp_utc: String,
    pub succeeded: bool,
}

/// What a guiding line of the log says went wrong with the guide star.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuideStarLoss {
    /// `[Guide] Guide star lost`.
    Lost,
    /// `[Guide] Select Guide Star failed, no star found`: no star could be
    /// chosen again.
    NoStarFound,
}

/// A loss of the guide star.
#[derive(Debug, Clone, PartialEq)]
pub struct GuideStarLostEvent {
    pub timestamp_utc: String,
    pub kind: GuideStarLoss,
}

/// `Exposure 180.0s image 12#`: the start of a light frame's exposure, the
/// line the Autorun log writes for every pose (a test or plate-solving
/// exposure has no `image N#`). Its instant is the frame's `DATE-OBS` to a few
/// seconds: the anchor the clock of the log is found with.
#[derive(Debug, Clone, PartialEq)]
pub struct ExposureStartEvent {
    pub timestamp_utc: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AsiairAutorunLog {
    /// The start of every light exposure the log names.
    pub exposure_start_events: Vec<ExposureStartEvent>,
    pub dither_events: Vec<DitherEvent>,
    pub settle_events: Vec<SettleEvent>,
    pub settle_timeout_events: Vec<SettleTimeoutEvent>,
    pub autofocus_events: Vec<AutofocusEvent>,
    pub autofocus_result_events: Vec<AutofocusResultEvent>,
    pub meridian_flip_events: Vec<MeridianFlipEvent>,
    /// Additive: the start of each meridian flip.
    pub meridian_flip_begin_events: Vec<MeridianFlipBeginEvent>,
    /// Additive: the end of each meridian flip, succeeded or not.
    pub meridian_flip_end_events: Vec<MeridianFlipEndEvent>,
    /// Additive: the guide star lost, or not found again.
    pub guide_star_lost_events: Vec<GuideStarLostEvent>,
}

/// Parses the `"YYYY/MM/DD HH:MM:SS"` prefix every Autorun log line starts
/// with, returning the rest of the line (the bracketed message) alongside
/// the parsed UTC timestamp.
fn parse_line(line: &str, offset_seconds: i64) -> Option<(String, &str)> {
    if line.len() < 19 {
        return None;
    }
    if line.as_bytes().get(4) != Some(&b'/')
        || line.as_bytes().get(7) != Some(&b'/')
        || line.as_bytes().get(10) != Some(&b' ')
        || line.as_bytes().get(13) != Some(&b':')
        || line.as_bytes().get(16) != Some(&b':')
    {
        return None;
    }
    // Each `.parse()` below already rejects anything but ASCII digits in
    // its slice (a leading sign or surrounding whitespace would also fail
    // to parse as these unsigned-looking widths), so no separate
    // digit-by-digit check is needed on top of the separator check above.
    let year: i64 = line.get(0..4)?.parse().ok()?;
    let month: u32 = line.get(5..7)?.parse().ok()?;
    let day: u32 = line.get(8..10)?.parse().ok()?;
    let hour: u32 = line.get(11..13)?.parse().ok()?;
    let minute: u32 = line.get(14..16)?.parse().ok()?;
    let second: f64 = line.get(17..19)?.parse().ok()?;
    let timestamp =
        local_datetime_to_unix_seconds(year, month, day, hour, minute, second, offset_seconds);
    let rest = line.get(19..)?.trim_start();
    Some((format_iso8601_utc_millis(timestamp), rest))
}

/// Finds the first occurrence of `needle` that is immediately followed by a
/// number immediately followed by `unit`, and returns that number. ASIAIR's
/// autofocus line repeats the word `"temperature"` (once in a sentence
/// describing the trigger, once with the actual reading, e.g.
/// `"...temperature changed 2 degrees, 27.0℃ changed to 24.0℃, exposure
/// 3.0s Bin1, temperature 24.0℃"`) -- only the occurrence directly
/// attached to a `"<number>℃"` reading counts, as a backtracking regex
/// would.
fn number_immediately_followed_by(message: &str, needle: &str, unit: &str) -> Option<f64> {
    for (start, _) in message.match_indices(needle) {
        let after = &message[start + needle.len()..];
        let end = after
            .char_indices()
            .find(|(index, ch)| !(ch.is_ascii_digit() || *ch == '.' || (*index == 0 && *ch == '-')))
            .map_or(after.len(), |(index, _)| index);
        if end == 0 {
            continue;
        }
        if after[end..].starts_with(unit)
            && let Ok(value) = after[..end].parse()
        {
            return Some(value);
        }
    }
    None
}

/// Whether `message` is the line of a light exposure's start:
/// `Exposure 180.0s image 12#`.
fn is_light_exposure_start(message: &str) -> bool {
    let Some(rest) = message.trim().strip_prefix("Exposure ") else {
        return false;
    };
    let Some((seconds, tail)) = rest.split_once("s image ") else {
        return false;
    };
    seconds.parse::<f64>().is_ok()
        && tail
            .strip_suffix('#')
            .is_some_and(|number| !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()))
}

/// Parses an ASIAIR `Autorun_Log_*.txt` file (or a target-scoped excerpt of
/// one).
pub fn parse_asiair_autorun_log(
    source: &str,
    options: &AsiairOptions,
) -> Result<AsiairAutorunLog, AsiairError> {
    let offset_seconds = parse_fixed_offset_seconds(&options.utc_offset)
        .ok_or_else(|| AsiairError::InvalidUtcOffset(options.utc_offset.clone()))?;

    let mut log = AsiairAutorunLog::default();
    for raw_line in source.lines() {
        let line = raw_line.trim_end_matches('\r');
        let Some((timestamp_utc, message)) = parse_line(line, offset_seconds) else {
            continue;
        };

        if is_light_exposure_start(message) {
            log.exposure_start_events
                .push(ExposureStartEvent { timestamp_utc });
            continue;
        }
        if message.contains("[Guide] Dither") && !message.contains("Dither Settle") {
            log.dither_events.push(DitherEvent { timestamp_utc });
            continue;
        }
        if message.contains("Dither Settle") {
            log.settle_events.push(SettleEvent { timestamp_utc });
            continue;
        }
        if message.to_ascii_lowercase().contains("settle timeout") {
            log.settle_timeout_events
                .push(SettleTimeoutEvent { timestamp_utc });
            continue;
        }
        if message.contains("[AutoFocus|Begin]") {
            let temperature_c = number_immediately_followed_by(message, "temperature ", "\u{2103}");
            log.autofocus_events.push(AutofocusEvent {
                timestamp_utc,
                temperature_c,
            });
            continue;
        }
        if message.contains("[AutoFocus|End]") {
            let succeeded = message.to_ascii_lowercase().contains("succeeded");
            log.autofocus_result_events.push(AutofocusResultEvent {
                timestamp_utc,
                succeeded,
            });
            continue;
        }
        if message.contains("[Meridian Flip|Begin]") {
            log.meridian_flip_begin_events
                .push(MeridianFlipBeginEvent { timestamp_utc });
            continue;
        }
        if message.contains("[Meridian Flip|End]") {
            let succeeded = message.to_ascii_lowercase().contains("succeeded");
            log.meridian_flip_end_events.push(MeridianFlipEndEvent {
                timestamp_utc: timestamp_utc.clone(),
                succeeded,
            });
            if succeeded {
                log.meridian_flip_events
                    .push(MeridianFlipEvent { timestamp_utc });
            }
            continue;
        }
        let lower = message.to_ascii_lowercase();
        if lower.contains("[guide]") && lower.contains("no star found") {
            log.guide_star_lost_events.push(GuideStarLostEvent {
                timestamp_utc,
                kind: GuideStarLoss::NoStarFound,
            });
        } else if lower.contains("[guide]") && lower.contains("guide star lost") {
            log.guide_star_lost_events.push(GuideStarLostEvent {
                timestamp_utc,
                kind: GuideStarLoss::Lost,
            });
        }
    }
    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(offset: &str) -> AsiairOptions {
        AsiairOptions {
            utc_offset: offset.to_string(),
        }
    }

    #[test]
    fn the_start_of_every_light_exposure_is_an_anchor_a_test_exposure_is_not() {
        let source = [
            "2026/08/07 22:45:40 Exposure 5.0s",
            "2026/08/07 22:46:04 Shooting 120 light frames, exposure 180.0s Bin1",
            "2026/08/07 22:46:05 Exposure 180.0s image 1#",
            "2026/08/07 22:49:06 Exposure 180.0s image 2#",
            "2026/08/07 22:52:08 Exposure 180.0s image 12#",
            "2026/08/07 22:55:00 Exposure 180.0s image #",
            "2026/08/07 22:56:00 Exposure fast image 3#",
        ]
        .join("\n");
        let log = parse_asiair_autorun_log(&source, &options("+02:00")).unwrap();
        let starts: Vec<&str> = log
            .exposure_start_events
            .iter()
            .map(|e| e.timestamp_utc.as_str())
            .collect();
        assert_eq!(
            starts,
            [
                "2026-08-07T20:46:05.000Z",
                "2026-08-07T20:49:06.000Z",
                "2026-08-07T20:52:08.000Z"
            ]
        );
    }

    #[test]
    fn extracts_every_event_kind_from_a_short_excerpt() {
        let source = [
            "2026/08/19 22:10:00 [Guide] Dither",
            "2026/08/19 22:10:10 [Guide] Dither Settle",
            "2026/08/19 22:10:30 [Guide] Settle Timeout",
            "2026/08/19 22:11:00 [AutoFocus|Begin] Run AF when temperature changed 2 degrees, 27.0\u{2103} changed to 24.0\u{2103}, exposure 3.0s Bin1, temperature 24.0\u{2103}",
            "2026/08/19 22:12:00 [AutoFocus|End] Auto focus succeeded",
            "2026/08/19 22:13:00 [Meridian Flip|End] Meridian Flip succeeded",
            "2026/08/19 22:14:00 [AutoFocus|End] Auto focus failed",
        ]
        .join("\n");

        let log = parse_asiair_autorun_log(&source, &options("+09:00")).unwrap();
        assert_eq!(log.dither_events.len(), 1);
        assert_eq!(
            log.dither_events[0].timestamp_utc,
            "2026-08-19T13:10:00.000Z"
        );
        assert_eq!(log.settle_events.len(), 1);
        assert_eq!(log.settle_timeout_events.len(), 1);
        assert_eq!(log.autofocus_events.len(), 1);
        assert_eq!(log.autofocus_events[0].temperature_c, Some(24.0));
        assert_eq!(log.autofocus_result_events.len(), 2);
        assert!(log.autofocus_result_events[0].succeeded);
        assert!(!log.autofocus_result_events[1].succeeded);
        assert_eq!(log.meridian_flip_events.len(), 1);
    }

    #[test]
    fn a_flip_has_its_two_instants_and_a_failed_one_is_kept_apart() {
        let source = [
            "2026/08/15 00:13:38 [Meridian Flip|Begin] Wait 10min18s to Meridian Flip",
            "2026/08/15 00:24:58 [Meridian Flip|End] Meridian Flip succeeded",
            "2026/08/15 02:00:00 [Meridian Flip|Begin] Wait 1min to Meridian Flip",
            "2026/08/15 02:03:00 [Meridian Flip|End] Meridian Flip failed",
        ]
        .join("\n");
        let log = parse_asiair_autorun_log(&source, &options("+02:00")).unwrap();
        assert_eq!(log.meridian_flip_begin_events.len(), 2);
        assert_eq!(
            log.meridian_flip_begin_events[0].timestamp_utc,
            "2026-08-14T22:13:38.000Z"
        );
        assert_eq!(log.meridian_flip_end_events.len(), 2);
        assert!(log.meridian_flip_end_events[0].succeeded);
        assert!(!log.meridian_flip_end_events[1].succeeded);
        // The reader that existed before keeps the successful ones only.
        assert_eq!(log.meridian_flip_events.len(), 1);
    }

    #[test]
    fn a_lost_guide_star_and_a_star_not_found_again_are_two_kinds() {
        let source = [
            "2026/08/15 01:57:36 [Guide] Guide star lost",
            "2026/08/15 03:07:42 [Guide] Select Guide Star failed, no star found",
            "2026/08/15 03:08:00 Guide star lost without the tag",
        ]
        .join("\n");
        let log = parse_asiair_autorun_log(&source, &options("+00:00")).unwrap();
        assert_eq!(log.guide_star_lost_events.len(), 2);
        assert_eq!(log.guide_star_lost_events[0].kind, GuideStarLoss::Lost);
        assert_eq!(
            log.guide_star_lost_events[1].kind,
            GuideStarLoss::NoStarFound
        );
        assert_eq!(
            log.guide_star_lost_events[1].timestamp_utc,
            "2026-08-15T03:07:42.000Z"
        );
    }

    #[test]
    fn the_timestamp_is_converted_from_a_positive_offset_to_utc() {
        let source = "2026/08/19 22:10:00 [Guide] Dither";
        let log = parse_asiair_autorun_log(source, &options("+09:00")).unwrap();
        assert_eq!(
            log.dither_events[0].timestamp_utc,
            "2026-08-19T13:10:00.000Z"
        );
    }

    #[test]
    fn a_dither_settle_line_is_not_also_counted_as_a_dither() {
        let source = "2026/08/19 22:10:10 [Guide] Dither Settle";
        let log = parse_asiair_autorun_log(source, &options("+00:00")).unwrap();
        assert!(log.dither_events.is_empty());
        assert_eq!(log.settle_events.len(), 1);
    }

    #[test]
    fn lines_without_a_leading_timestamp_are_ignored() {
        let source = "not a log line\n[Guide] Dither";
        let log = parse_asiair_autorun_log(source, &options("+00:00")).unwrap();
        assert!(log.dither_events.is_empty());
    }

    #[test]
    fn rejects_an_invalid_utc_offset() {
        let result =
            parse_asiair_autorun_log("2026/08/19 22:10:00 [Guide] Dither", &options("nope"));
        assert_eq!(
            result,
            Err(AsiairError::InvalidUtcOffset("nope".to_string()))
        );
    }
}

//! The clock of the logs, found on its own (review B2, G1).
//!
//! PHD2, ASIAIR and NINA write the wall clock of the computer that ran them
//! (`UTC+2` in Paris in summer), while `DATE-OBS` of a frame is UTC. Read
//! with the wrong offset, the guiding samples fall next to the frames, not in
//! them: no RMS per frame, events two hours away. The offset is a whole
//! number of half hours (the offsets of the world's clocks, but the odd
//! quarter hours), so it is **found**: the shift that puts the most light
//! frames in front of their log is the one. The person's own offsets (the
//! options of the command line, the settings of the window) are kept as they
//! are and never second-guessed.

use serde::Serialize;

use super::{AnalysisError, AnalysisOptions, AnalysisOutcome, analyze_folder_with};
use crate::time_util::{parse_fixed_offset_seconds, parse_iso8601_utc_to_unix_seconds};
use std::path::Path;

/// Half an hour, in seconds: the step of the search.
const STEP_SECONDS: i64 = 1_800;
/// The search runs from -14 h to +14 h, the extremes of the world's clocks.
const MAX_STEPS: i64 = 28;

/// The offsets a person forces, by kind of log; `None` is « find it ».
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForcedLogOffsets {
    pub phd2: Option<String>,
    pub asiair: Option<String>,
    pub nina: Option<String>,
}

/// The clocks the logs of a night were read with, and how they were known.
/// A kind of log the folder does not hold has `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogsUtcOffset {
    /// `true` when at least one offset was found by Astro Sync (the others
    /// were given); `false` when every offset used is the person's.
    pub detected: bool,
    /// `+HH:MM`, the clock of the PHD2 logs.
    pub phd2: Option<String>,
    pub asiair: Option<String>,
    pub nina: Option<String>,
    /// Light frames (with a start and an exposure) the logs were compared to.
    pub frames: usize,
    /// How many of them have their PHD2 guiding samples at this offset.
    pub phd2_matched_frames: Option<usize>,
    /// How many have the start of an ASIAIR exposure at their `DATE-OBS` (an
    /// ASIAIR event in the gap that follows them, for a log without exposure
    /// lines).
    pub asiair_matched_frames: Option<usize>,
    /// How many have the start of a NINA exposure at their `DATE-OBS`.
    pub nina_matched_frames: Option<usize>,
    /// How the clock of the PHD2 logs was known: `given` (the person's),
    /// `asiair` (the ASIAIR's, found from its exposure lines: the PHD2 of an
    /// ASIAIR runs on the same clock), `frames` (found from the frames alone).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phd2_from: Option<String>,
}

/// The PHD2 clock found from the frames alone may be half an hour off the
/// true one when the guiding stopped before the last exposures: shifting the
/// log brings its samples in front of the uncovered frames. The ASIAIR's own
/// clock, anchored on its exposure lines, is the same computer's: it is taken
/// for PHD2 whenever it matches at least this share of what the best shift
/// matches.
pub const PHD2_FOLLOWS_ASIAIR_SHARE: f64 = 0.8;

/// `+HH:MM` of signed seconds east of UTC.
#[must_use]
pub fn format_offset(seconds: i64) -> String {
    let sign = if seconds < 0 { '-' } else { '+' };
    let minutes = seconds.abs() / 60;
    format!("{sign}{:02}:{:02}", minutes / 60, minutes % 60)
}

/// The span of a frame (its start and end, the next frame if any) in which an
/// instant of a log counts for it.
type WindowOf = dyn Fn(&Pose, Option<&Pose>) -> (f64, f64);

struct Pose {
    start: f64,
    end: f64,
}

/// What one kind of log gives: instants, in the clock of the log read as UTC.
struct Kind<'a> {
    times: Vec<f64>,
    /// The span of a frame in which an instant of this log counts for it.
    window: &'a WindowOf,
    forced: Option<&'a str>,
}

fn poses_of(outcome: &AnalysisOutcome) -> Vec<Pose> {
    let mut poses: Vec<Pose> = outcome
        .frames
        .iter()
        .filter(|frame| frame.is_light)
        .filter_map(|frame| {
            let start = parse_iso8601_utc_to_unix_seconds(frame.record.date_obs_utc.as_deref()?)?;
            let exposure = frame.record.exposure_seconds?;
            Some(Pose {
                start,
                end: start + exposure,
            })
        })
        .collect();
    poses.sort_by(|a, b| a.start.total_cmp(&b.start));
    poses
}

/// How many frames have an instant of the log in their window when the log's
/// clock is `shift` seconds ahead of UTC.
fn matched(times: &[f64], poses: &[Pose], window: &WindowOf, shift: i64) -> usize {
    let shift = shift as f64;
    poses
        .iter()
        .enumerate()
        .filter(|(index, pose)| {
            let (from, to) = window(pose, poses.get(index + 1));
            let (from, to) = (from + shift, to + shift);
            let first = times.partition_point(|t| *t < from);
            times.get(first).is_some_and(|t| *t <= to)
        })
        .count()
}

fn phd2_window(pose: &Pose, _next: Option<&Pose>) -> (f64, f64) {
    (pose.start, pose.end)
}

/// The ASIAIR writes no exposure: its dither, settle and autofocus lines
/// follow a frame, in the gap before the next one.
fn asiair_window(pose: &Pose, next: Option<&Pose>) -> (f64, f64) {
    let gap_end = next.map_or(pose.end + 300.0, |n| n.start.min(pose.end + 300.0));
    (pose.end - 2.0, gap_end + 2.0)
}

/// The line of NINA that starts an exposure is at its `DATE-OBS`.
fn nina_window(pose: &Pose, _next: Option<&Pose>) -> (f64, f64) {
    (pose.start - 30.0, pose.start + 60.0)
}

/// Which log the instants of [`match_log`] come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogClock {
    Phd2,
    Asiair,
    Nina,
}

/// How one log lines up with light frames: the shift of its clock east of
/// UTC (seconds, a whole number of half hours) and the frames it matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogMatch {
    pub shift_seconds: i64,
    /// Indices into the `frames` given to [`match_log`].
    pub frames: Vec<usize>,
}

/// The same search as [`resolve`], for one log alone against any frames
/// (`(start, end)` in Unix seconds, in any order): the shift that puts the
/// most frames in front of the log's instants (read as UTC), and which
/// frames those are; `within` keeps the search to the shifts of that range,
/// seconds, both ends included (one shift when the place's clock is known).
/// On a tie a whole hour wins over a half hour (most clocks of the world),
/// then the nearest to the middle of the range (to UTC without a range).
/// `None` when no shift matches any frame.
/// Used by `crate::session_detection` to tie a log to its nights and
/// targets.
#[must_use]
pub fn match_log(
    kind: LogClock,
    instants: &[f64],
    frames: &[(f64, f64)],
    within: Option<(i64, i64)>,
) -> Option<LogMatch> {
    let mut order: Vec<usize> = (0..frames.len()).collect();
    order.sort_by(|a, b| frames[*a].0.total_cmp(&frames[*b].0));
    let poses: Vec<Pose> = order
        .iter()
        .map(|i| Pose {
            start: frames[*i].0,
            end: frames[*i].1,
        })
        .collect();
    let mut times: Vec<f64> = instants.iter().copied().filter(|t| t.is_finite()).collect();
    times.sort_by(f64::total_cmp);
    let window: &WindowOf = match kind {
        LogClock::Phd2 => &phd2_window,
        LogClock::Asiair => &asiair_window,
        LogClock::Nina => &nina_window,
    };
    let (low, high) = within.unwrap_or((-MAX_STEPS * STEP_SECONDS, MAX_STEPS * STEP_SECONDS));
    let centre = (low + high) / 2;
    let mut shifts: Vec<i64> = (-MAX_STEPS..=MAX_STEPS)
        .map(|step| step * STEP_SECONDS)
        .chain([low, high])
        .filter(|shift| (low..=high).contains(shift))
        .collect();
    shifts.sort_unstable();
    shifts.dedup();
    // Whole hours first, then the half hours: a tie keeps the first.
    shifts.sort_by_key(|shift| (shift % 3600 != 0, (shift - centre).abs(), *shift < centre));
    let mut best: Option<(i64, usize)> = None;
    for shift in shifts {
        let count = matched(&times, &poses, window, shift);
        if count > 0 && best.is_none_or(|(_, kept)| count > kept) {
            best = Some((shift, count));
        }
    }
    let shift = best?.0;
    let shifted = shift as f64;
    let matched = poses
        .iter()
        .enumerate()
        .filter(|(index, pose)| {
            let (from, to) = window(pose, poses.get(index + 1));
            let first = times.partition_point(|t| *t < from + shifted);
            times.get(first).is_some_and(|t| *t <= to + shifted)
        })
        .map(|(index, _)| order[index])
        .collect();
    Some(LogMatch {
        shift_seconds: shift,
        frames: matched,
    })
}

/// The shift (a whole number of half hours) that matches the most frames; the
/// nearest to UTC when several do, and `None` when no shift matches any.
fn best_shift(times: &[f64], poses: &[Pose], window: &WindowOf) -> Option<(i64, usize)> {
    let mut best: Option<(i64, usize)> = None;
    // From the nearest to UTC outwards, the east before the west: a tie keeps
    // the first met.
    let mut steps = vec![0_i64];
    for step in 1..=MAX_STEPS {
        steps.push(step);
        steps.push(-step);
    }
    for step in steps {
        let count = matched(times, poses, window, step * STEP_SECONDS);
        if count > 0 && best.is_none_or(|(_, kept)| count > kept) {
            best = Some((step * STEP_SECONDS, count));
        }
    }
    best
}

/// What an analysis read at `+00:00` says of the clocks of its logs, with the
/// offsets the person forced (`forced`) taken as they are. Returns the offsets
/// to read the logs with, and the account of it (`None` when the folder holds
/// no PHD2, ASIAIR or NINA log).
#[must_use]
pub fn resolve(
    outcome: &AnalysisOutcome,
    forced: &ForcedLogOffsets,
) -> (AnalysisOptions, Option<LogsUtcOffset>) {
    let poses = poses_of(outcome);
    let mut options = AnalysisOptions::default();
    let mut account = LogsUtcOffset {
        frames: poses.len(),
        ..LogsUtcOffset::default()
    };

    let sorted = |mut times: Vec<f64>| {
        times.retain(|t| t.is_finite());
        times.sort_by(f64::total_cmp);
        times
    };
    let phd2 = (outcome.phd2_files_parsed > 0).then(|| Kind {
        times: sorted(
            outcome
                .guiding_rows
                .iter()
                .filter(|row| row.source == "phd2")
                .filter_map(|row| parse_iso8601_utc_to_unix_seconds(&row.timestamp_utc))
                .collect(),
        ),
        window: &phd2_window,
        forced: forced.phd2.as_deref(),
    });
    // The Autorun log writes the start of every light exposure
    // (`Exposure 180.0s image 12#`): at the frame's own `DATE-OBS`, it is the
    // anchor of the clock. Its dither, settle and autofocus lines are few on a
    // night without dither (three events in a night, which fit any offset by
    // chance); they are the anchor only for a log without exposure lines.
    let asiair_events = sorted(
        outcome
            .events
            .iter()
            .filter(|event| event.source == "asiair")
            .filter_map(|event| parse_iso8601_utc_to_unix_seconds(&event.timestamp_utc))
            .collect(),
    );
    let asiair_exposures = sorted(outcome.asiair_exposure_starts.clone());
    // Exposure lines that line up with no frame at any offset (a log of another
    // night) leave the events to try.
    let exposures_anchor = !asiair_exposures.is_empty()
        && best_shift(&asiair_exposures, &poses, &nina_window).is_some();
    let asiair = (outcome.asiair_files_parsed > 0).then(|| {
        if !exposures_anchor {
            Kind {
                times: asiair_events.clone(),
                window: &asiair_window,
                forced: forced.asiair.as_deref(),
            }
        } else {
            Kind {
                times: asiair_exposures.clone(),
                window: &nina_window,
                forced: forced.asiair.as_deref(),
            }
        }
    });
    let nina = (outcome.nina_files_parsed > 0).then(|| Kind {
        times: sorted(
            outcome
                .nina_logs
                .iter()
                .flat_map(|log| log.exposures.iter().map(|exposure| exposure.unix_seconds))
                .collect(),
        ),
        window: &nina_window,
        forced: forced.nina.as_deref(),
    });

    let mut decide = |kind: Option<Kind<'_>>,
                      given: Option<&str>,
                      follow: Option<i64>|
     -> (Option<String>, Option<usize>, String, Option<&'static str>) {
        let Some(kind) = kind else {
            return (None, None, given.unwrap_or("+00:00").to_string(), None);
        };
        if let Some(own) = kind.forced {
            // The log was read with the person's clock: its instants are UTC.
            let count = matched(&kind.times, &poses, kind.window, 0);
            return (
                Some(own.to_string()),
                Some(count),
                own.to_string(),
                Some("given"),
            );
        }
        match best_shift(&kind.times, &poses, kind.window) {
            Some((seconds, count)) => {
                account.detected = true;
                // Another log of the same computer whose clock is surer.
                if let Some(other) = follow.filter(|other| *other != seconds) {
                    let there = matched(&kind.times, &poses, kind.window, other);
                    if there as f64 >= count as f64 * PHD2_FOLLOWS_ASIAIR_SHARE && there > 0 {
                        let text = format_offset(other);
                        return (Some(text.clone()), Some(there), text, Some("asiair"));
                    }
                }
                let text = format_offset(seconds);
                (Some(text.clone()), Some(count), text, Some("frames"))
            }
            // Nothing matches at any offset (no frame, logs of another night):
            // UTC, as before, and not a word of detection.
            None => (None, None, "+00:00".to_string(), None),
        }
    };
    let (asiair_shown, asiair_count, asiair_used, _) =
        decide(asiair, forced.asiair.as_deref(), None);
    // The ASIAIR clock the PHD2 log may follow: the person's, or the one its
    // exposure lines anchor (never the few events of a log without them).
    let asiair_clock = if forced.asiair.is_some() || exposures_anchor {
        asiair_shown.as_deref().and_then(parse_fixed_offset_seconds)
    } else {
        None
    };
    let (phd2_shown, phd2_count, phd2_used, phd2_from) =
        decide(phd2, forced.phd2.as_deref(), asiair_clock);
    let (nina_shown, nina_count, nina_used, _) = decide(nina, forced.nina.as_deref(), None);
    options.phd2_utc_offset = phd2_used;
    options.asiair_utc_offset = asiair_used;
    options.nina_utc_offset = nina_used;
    account.phd2 = phd2_shown;
    account.asiair = asiair_shown;
    account.nina = nina_shown;
    account.phd2_matched_frames = phd2_count;
    account.asiair_matched_frames = asiair_count;
    account.nina_matched_frames = nina_count;
    account.phd2_from = phd2_from.map(str::to_string);

    let any_log =
        outcome.phd2_files_parsed + outcome.asiair_files_parsed + outcome.nina_files_parsed > 0;
    (options, any_log.then_some(account))
}

/// Reads the folder with the clocks of its logs, found unless forced: the
/// outcome and the account of the offsets used (`None` without a log).
pub fn analyze_folder_detecting(
    root: &Path,
    forced: &ForcedLogOffsets,
    progress: Option<crate::local_work::ProgressSink<'_>>,
) -> Result<(AnalysisOutcome, Option<LogsUtcOffset>), AnalysisError> {
    let base = AnalysisOptions {
        phd2_utc_offset: forced.phd2.clone().unwrap_or_else(|| "+00:00".into()),
        asiair_utc_offset: forced.asiair.clone().unwrap_or_else(|| "+00:00".into()),
        nina_utc_offset: forced.nina.clone().unwrap_or_else(|| "+00:00".into()),
    };
    let first = analyze_folder_with(root, &base, progress)?;
    let (resolved, account) = resolve(&first, forced);
    if resolved.phd2_utc_offset == base.phd2_utc_offset
        && resolved.asiair_utc_offset == base.asiair_utc_offset
        && resolved.nina_utc_offset == base.nina_utc_offset
    {
        return Ok((first, account));
    }
    Ok((analyze_folder_with(root, &resolved, None)?, account))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{TempDir, fits_bytes};

    /// Six lights of 180 s from 20:46:05 UTC, one every 181 s; the Autorun log
    /// of the same night written by the ASIAIR two hours east of UTC: one
    /// `Exposure 180.0s image N#` line per light, and the three events of a
    /// night without dither (an autofocus pair and a flip), one of which falls
    /// in the gap after a light when the clock is read half an hour wrong.
    fn night() -> TempDir {
        let dir = TempDir::new("asiair-clock");
        for index in 0..6_u32 {
            let seconds = 46 * 60 + 5 + 181 * index;
            let at = format!(
                "'2026-08-07T20:{:02}:{:02}.000'",
                seconds / 60 % 60,
                seconds % 60
            );
            std::fs::write(
                dir.path().join(format!("Light_{index:04}.fit")),
                fits_bytes(
                    &[
                        ("IMAGETYP", "'Light'"),
                        ("EXPTIME", "180.0"),
                        ("DATE-OBS", &at),
                        ("XBINNING", "1"),
                        ("YBINNING", "1"),
                    ],
                    0,
                ),
            )
            .unwrap();
        }
        let mut log = vec!["Log enabled at 2026/08/07 22:45:29".to_string()];
        log.push("2026/08/07 22:45:40 Exposure 5.0s".to_string());
        for index in 0..6_u32 {
            let seconds = 46 * 60 + 5 + 181 * index;
            log.push(format!(
                "2026/08/07 22:{:02}:{:02} Exposure 180.0s image {}#",
                seconds / 60 % 60,
                seconds % 60,
                index + 1
            ));
        }
        // 23:22:06 on the log's clock is 20:52:06 UTC half an hour too far
        // east: right after the second light.
        log.push("2026/08/07 23:22:06 [AutoFocus|Begin] Run AF when temperature changed 2 degrees, 24.0\u{2103} changed to 21.0\u{2103}, exposure 5.0s Bin1, temperature 21.0\u{2103}".to_string());
        log.push("2026/08/07 23:30:00 [AutoFocus|End] Auto focus succeeded".to_string());
        std::fs::write(
            dir.path().join("Autorun_Log_2026-08-07_224529.txt"),
            log.join("\n"),
        )
        .unwrap();
        dir
    }

    #[test]
    fn the_exposure_lines_of_the_log_find_its_clock_where_a_few_events_cannot() {
        let dir = night();
        let (_, account) =
            analyze_folder_detecting(dir.path(), &ForcedLogOffsets::default(), None).unwrap();
        let account = account.unwrap();
        assert_eq!(account.asiair.as_deref(), Some("+02:00"));
        assert_eq!(account.asiair_matched_frames, Some(6));
        assert_eq!(account.frames, 6);
    }

    #[test]
    fn a_log_without_exposure_lines_is_still_read_from_its_events() {
        let dir = night();
        let path = dir.path().join("Autorun_Log_2026-08-07_224529.txt");
        let kept: Vec<String> = std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .filter(|line| !line.contains("image "))
            .map(str::to_string)
            .collect();
        std::fs::write(&path, kept.join("\n")).unwrap();
        let (_, account) =
            analyze_folder_detecting(dir.path(), &ForcedLogOffsets::default(), None).unwrap();
        // The events alone are the old, fragile anchor: one frame of six
        // lines up, at a clock half an hour off. The account says how weak
        // that match is, as it always did.
        let account = account.unwrap();
        assert_eq!(account.asiair.as_deref(), Some("+02:30"));
        assert_eq!(account.asiair_matched_frames, Some(1));
    }

    /// Twenty lights of 180 s from 20:46:05 UTC; the Autorun log of the same
    /// ASIAIR two hours east of UTC with one exposure line per light; its
    /// PHD2 log on the same clock, whose guiding stops after twenty minutes
    /// (the guiding of the last forty minutes is missing).
    fn night_with_short_guiding() -> TempDir {
        let dir = TempDir::new("phd2-follows-asiair");
        let mut log = vec![
            "Log enabled at 2026/08/07 22:45:29".to_string(),
            "2026/08/07 22:45:30 [Autorun|Begin] Plan Start".to_string(),
        ];
        for index in 0..20_u32 {
            let seconds = 46 * 60 + 5 + 181 * index;
            let at = format!(
                "'2026-08-07T{:02}:{:02}:{:02}.000'",
                20 + seconds / 3600,
                seconds / 60 % 60,
                seconds % 60
            );
            std::fs::write(
                dir.path().join(format!("Light_{index:04}.fit")),
                fits_bytes(
                    &[
                        ("IMAGETYP", "'Light'"),
                        ("EXPTIME", "180.0"),
                        ("DATE-OBS", &at),
                        ("XBINNING", "1"),
                        ("YBINNING", "1"),
                    ],
                    0,
                ),
            )
            .unwrap();
            log.push(format!(
                "2026/08/07 {:02}:{:02}:{:02} Exposure 180.0s image {}#",
                22 + seconds / 3600,
                seconds / 60 % 60,
                seconds % 60,
                index + 1
            ));
        }
        std::fs::write(
            dir.path().join("Autorun_Log_2026-08-07_224529.txt"),
            log.join("\n"),
        )
        .unwrap();
        let mut guide = vec![
            "Guiding Begins at 2026-08-07 22:46:00".to_string(),
            "Pixel scale = 6.88 arc-sec/px".to_string(),
            "Frame,Time,mount,dx,dy,RARawDistance,DECRawDistance,RAGuideDistance,DECGuideDistance,RADuration,RADirection,DECDuration,DECDirection,XStep,YStep,StarMass,SNR,ErrorCode".to_string(),
        ];
        for frame in 1..=400_u32 {
            guide.push(format!(
                "{frame},{:.3},\"Mount\",0,0,0.05,-0.05,0,0,40,E,30,N,,,1000,20,0",
                f64::from(frame) * 3.0
            ));
        }
        guide.push("Guiding Ends at 2026-08-07 23:06:03".to_string());
        std::fs::write(
            dir.path().join("PHD2_GuideLog_2026-08-07_224600.txt"),
            guide.join("\n"),
        )
        .unwrap();
        dir
    }

    #[test]
    fn a_phd2_clock_half_an_hour_off_gives_way_to_the_asiair_clock() {
        let dir = night_with_short_guiding();
        let (_, account) =
            analyze_folder_detecting(dir.path(), &ForcedLogOffsets::default(), None).unwrap();
        let account = account.unwrap();
        assert_eq!(account.asiair.as_deref(), Some("+02:00"));
        // From the frames alone the log fits as well half an hour later
        // (+01:30, the slot of the missing guiding): the ASIAIR's clock wins.
        assert_eq!(account.phd2.as_deref(), Some("+02:00"));
        assert_eq!(account.phd2_from.as_deref(), Some("asiair"));
    }

    #[test]
    fn the_clock_the_person_gave_is_never_second_guessed() {
        let dir = night();
        let forced = ForcedLogOffsets {
            asiair: Some("+01:00".to_string()),
            ..ForcedLogOffsets::default()
        };
        let (_, account) = analyze_folder_detecting(dir.path(), &forced, None).unwrap();
        assert_eq!(account.unwrap().asiair.as_deref(), Some("+01:00"));
    }
}

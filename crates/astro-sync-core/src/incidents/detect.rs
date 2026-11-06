//! The detectors of the "Incident list": what the logs and the frame
//! headers of a night show, read only.
//!
//! The night is read through the analysis of the crate ([`analyze_folder_with`],
//! the clock of each log found from the frames, [`crate::analysis::log_offset`])
//! for the headers of the poses and the NINA log, and once more for the two
//! logs whose lines the analysis does not keep (ASIAIR Autorun: the start of a
//! flip, the guide star lost; PHD2: the frames dropped). A fact is **one
//! source**: when the night holds an ASIAIR Autorun log, its lines say the
//! guide star losses, the settle timeouts and the flips; the PHD2 log (and the
//! NINA log for the flips) only stand in when there is none, so that the same
//! loss is never counted twice.
//!
//! Only what falls in the night's window, and between the first pose of the
//! target and its last (plus [`LOG_SPAN_MARGIN_SECONDS`]), is an incident of
//! the target: the guiding set-up before the first frame is not.
//!
//! The thresholds are the named constants below, first tuned on the real night
//! of Sadr (14 to 15 August 2026), then on the Seestar night of M 45 and
//! eleven Askar 50P nights: a pause is judged against the rhythm of its own
//! night (the cycle of its frames, the gaps it repeats), a drift of
//! temperature against the set point of a cooled sensor.

use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::{DetectedIncident, Evidence, EvidenceSource, IncidentKind, iso_seconds};
use crate::analysis::asiair::{self, AsiairAutorunLog, GuideStarLoss};
use crate::analysis::log_offset::{ForcedLogOffsets, LogsUtcOffset, resolve};
use crate::analysis::phd2::{self, Phd2Log, SettleOutcome};
use crate::analysis::{
    AnalysisOptions, AnalysisOutcome, analyze_folder_with, is_asiair_translation,
    looks_like_asiair_autorun_log, looks_like_phd2_guide_log,
};
use crate::detection::NightWindow;
use crate::scanner::{FileClass, walk_folder};
use crate::time_util::parse_iso8601_utc_to_unix_seconds;

/// A gap between two poses is a pause when it is more than this many times
/// what the night does between two poses *as a routine* (see
/// [`ROUTINE_REPEAT_MIN`]: the dither, the autofocus, the re-centring that the
/// night repeats)...
pub const GAP_FACTOR: f64 = 3.0;
/// ...and at least this long (seconds): an autofocus or a dither is not a
/// pause...
pub const GAP_MIN_SECONDS: f64 = 180.0;
/// ...and at least this many frame cycles (the median time from the start of
/// one pose to the start of the next): a gap under one cycle has not even
/// cost a frame.
pub const GAP_MIN_CYCLES: f64 = 1.0;
/// A gap length that the night repeats this many times (within
/// [`ROUTINE_REPEAT_SPREAD`] of each other) is a step of the sequence, not an
/// interruption: three gaps of 3 min 20 s to 3 min 40 s on a Seestar night are
/// its re-centring, not three pauses.
pub const ROUTINE_REPEAT_MIN: usize = 3;
/// The longest to the shortest of a repeated gap length (a ratio).
pub const ROUTINE_REPEAT_SPREAD: f64 = 1.25;
/// A repeated gap longer than this many frame cycles is never routine: three
/// long interruptions of a night stay interruptions.
pub const ROUTINE_MAX_CYCLES: f64 = 10.0;
/// Under this many poses there is no « rhythm of the night »: no gap is a
/// pause.
pub const GAP_MIN_POSES: usize = 5;
/// A flip and a gap between poses are the same pause when they overlap, to
/// this many seconds near (the clocks of the log and of the camera).
pub const FLIP_GAP_SLACK_SECONDS: f64 = 120.0;
/// The guide star losses less than this many seconds apart are one incident
/// (ten minutes).
pub const GUIDE_LOSS_GROUP_SECONDS: f64 = 600.0;
/// An ASIAIR « no star found » this close after a « Guide star lost » is the
/// end of that loss (no star could be chosen again), not a loss of its own.
pub const NO_STAR_FOLLOWS_LOST_SECONDS: f64 = 300.0;
/// Two findings of `guiding_timeouts` or `guiding_spikes` more than this far
/// apart (60 minutes, `session-events.md`) make two events.
pub const GUIDING_GROUP_SECONDS: f64 = 3600.0;
/// PHD2 only (no ASIAIR log): the dropped frames less than this far apart are
/// one episode.
pub const PHD2_DROP_EPISODE_GAP_SECONDS: f64 = 30.0;
/// PHD2 only: an episode of at least this many dropped frames in a row is a
/// loss of the guide star (a handful of dropped frames is a gust or a cloud
/// edge, the real night has dozens of them).
pub const PHD2_LOSS_MIN_DROPS: usize = 10;
/// A guiding error (arcseconds, both axes) at or above this is a spike...
pub const SPIKE_ARCSEC: f64 = 8.0;
/// ...outside a dither settle: from its start, this many seconds (and until
/// 30 s after PHD2 says it ended, if later) the error is the dither's own.
pub const SETTLE_EXCLUSION_SECONDS: f64 = 120.0;
/// A group of fewer spikes is left out of the list (told in `unproposed`).
pub const SPIKES_MIN_COUNT: usize = 3;
/// The sensor temperature moved this many degrees from the value the cooler
/// holds (its set point, `SET-TEMP`; without one, the temperature the night
/// held).
pub const TEMPERATURE_DRIFT_CELSIUS: f64 = 3.0;
/// Without a set point in the headers: the sensor is cooled (regulated) when
/// at least [`TEMPERATURE_HELD_SHARE`] of the poses sit within this many
/// degrees of the median temperature (the cooled Askar nights: all of them
/// within 0.2). A sensor that follows the air (the Seestar went from 23.8 to
/// 17.2 C over a night: 26 % of its poses within 0.5 of the median) holds
/// nothing, has no cooler to fail, and is never a drift.
pub const TEMPERATURE_HOLD_BAND_CELSIUS: f64 = 0.5;
/// See [`TEMPERATURE_HOLD_BAND_CELSIUS`].
pub const TEMPERATURE_HELD_SHARE: f64 = 0.6;
/// A run of fewer poses of one exposure is a test frame, not a change of
/// exposure.
pub const EXPOSURE_RUN_MIN_POSES: usize = 3;
/// More changes of exposure than this in a night is a sequence that
/// alternates on purpose: none is proposed (told in `unproposed`).
pub const EXPOSURE_CHANGE_MAX: usize = 4;
/// What the logs say counts for the target from this many seconds before its
/// first pose to this many after its last.
pub const LOG_SPAN_MARGIN_SECONDS: f64 = 120.0;
/// The NINA lines of one flip are less than this far apart.
const NINA_FLIP_CLUSTER_SECONDS: f64 = 600.0;

/// What to read: the night's folder, one target, the window and the clocks.
#[derive(Debug, Clone)]
pub struct DetectInput {
    /// The night's folder (`<target>/lights/`, `logs/`, …).
    pub night_folder: PathBuf,
    /// The target's folder in the night (its poses are in `<target>/lights/`);
    /// `None`: every light of the night.
    pub target_folder: Option<String>,
    /// The night's window ([`NightWindow::of_evening`]); `None`: no limit.
    pub window: Option<NightWindow>,
    /// The clocks of the logs the person forces; none: found from the poses.
    pub log_offsets: ForcedLogOffsets,
}

/// How many logs of each kind were read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogsRead {
    pub asiair: usize,
    pub phd2: usize,
    pub nina: usize,
}

/// A reading that could not be done, told instead of an error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    /// `analysis_failed`: a log or the folder could not be read.
    pub code: String,
    pub detail: String,
}

/// Detected, and not proposed: a genre the engine chose not to list.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unproposed {
    /// `guiding_spikes` (groups under [`SPIKES_MIN_COUNT`]), `exposure_change`
    /// (more than [`EXPOSURE_CHANGE_MAX`]), `meridian_flip_unbounded` (a flip
    /// with no end and no gap between poses).
    pub code: String,
    pub count: usize,
    pub params: Value,
}

/// What a night's files show.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detection {
    /// By start, then key.
    pub incidents: Vec<DetectedIncident>,
    pub unproposed: Vec<Unproposed>,
    /// The poses of the target in the night's window.
    pub poses: usize,
    pub logs: LogsRead,
    /// The clocks the logs were read with (`None` without a log).
    pub logs_utc_offset: Option<LogsUtcOffset>,
    pub problems: Vec<Problem>,
}

/// One pose of the target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Pose {
    /// 1 = the first of the target in the night.
    pub rank: usize,
    pub start: f64,
    pub exposure: f64,
    pub temperature: Option<f64>,
    /// The set point of the cooler (`SET-TEMP`): only a cooled camera writes
    /// one.
    pub set_temperature: Option<f64>,
}

impl Pose {
    fn end(&self) -> f64 {
        self.start + self.exposure
    }
}

/// A meridian flip, from a log.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Flip {
    pub begin: f64,
    pub end: Option<f64>,
    pub succeeded: Option<bool>,
    pub source: EvidenceSource,
}

/// What was read of the night, before any judgement.
struct Night {
    poses: Vec<Pose>,
    asiair: Vec<AsiairAutorunLog>,
    phd2: Vec<Phd2Log>,
    nina_flips: Vec<f64>,
    nina_files: usize,
    offsets: Option<LogsUtcOffset>,
}

/// Reads a night and says its incidents. Nothing is written; a log that
/// cannot be read is a [`Problem`], not an error.
#[must_use]
pub fn detect_incidents(input: &DetectInput) -> Detection {
    let mut detection = Detection::default();
    let night = match read_night(input, &mut detection.problems) {
        Ok(night) => night,
        Err(detail) => {
            detection.problems.push(Problem {
                code: "analysis_failed".to_string(),
                detail,
            });
            return detection;
        }
    };
    detection.poses = night.poses.len();
    detection.logs = LogsRead {
        asiair: night.asiair.len(),
        phd2: night.phd2.len(),
        nina: night.nina_files,
    };
    detection.logs_utc_offset = night.offsets.clone();
    let scope = Scope::of(&night.poses, input.window);

    let mut found = Vec::new();
    found.extend(pause_incidents(
        &night.poses,
        &flips_of(&night, &scope),
        &mut detection.unproposed,
    ));
    found.extend(guide_loss_incidents(&night, &scope));
    found.extend(guiding_timeout_incidents(&night, &scope));
    found.extend(spike_incidents(
        &night.phd2,
        &night.poses,
        &scope,
        &mut detection.unproposed,
    ));
    found.extend(exposure_change_incidents(
        &night.poses,
        &mut detection.unproposed,
    ));
    found.extend(temperature_drift_incidents(&night.poses));

    found.sort_by(|a, b| a.start.cmp(&b.start).then(a.id.cmp(&b.id)));
    detection.incidents = unique_ids(found);
    detection
}

/// Where the logs count: the window, and the span of the target's poses.
#[derive(Debug, Clone, Copy)]
struct Scope {
    window: Option<NightWindow>,
    span: Option<(f64, f64)>,
}

impl Scope {
    fn of(poses: &[Pose], window: Option<NightWindow>) -> Self {
        let span = match (poses.first(), poses.last()) {
            (Some(first), Some(last)) => Some((
                first.start - LOG_SPAN_MARGIN_SECONDS,
                last.end() + LOG_SPAN_MARGIN_SECONDS,
            )),
            _ => None,
        };
        Self { window, span }
    }

    fn holds(&self, unix: f64) -> bool {
        self.window.is_none_or(|w| w.contains(unix))
            && self
                .span
                .is_none_or(|(from, to)| unix >= from && unix <= to)
    }
}

fn unix_of(text: &str) -> Option<f64> {
    parse_iso8601_utc_to_unix_seconds(text).filter(|t| t.is_finite())
}

fn read_night(input: &DetectInput, problems: &mut Vec<Problem>) -> Result<Night, String> {
    let forced = &input.log_offsets;
    let base = AnalysisOptions {
        phd2_utc_offset: forced.phd2.clone().unwrap_or_else(|| "+00:00".into()),
        asiair_utc_offset: forced.asiair.clone().unwrap_or_else(|| "+00:00".into()),
        nina_utc_offset: forced.nina.clone().unwrap_or_else(|| "+00:00".into()),
    };
    let first =
        analyze_folder_with(&input.night_folder, &base, None).map_err(|error| error.to_string())?;
    let (options, offsets) = resolve(&first, forced);
    let same = options.phd2_utc_offset == base.phd2_utc_offset
        && options.asiair_utc_offset == base.asiair_utc_offset
        && options.nina_utc_offset == base.nina_utc_offset;
    let outcome = if same {
        first
    } else {
        analyze_folder_with(&input.night_folder, &options, None)
            .map_err(|error| error.to_string())?
    };
    let poses = poses_of(&outcome, input.target_folder.as_deref(), input.window);
    let (asiair, phd2) = read_logs(&input.night_folder, &options, problems);
    let nina_flips = outcome
        .nina_logs
        .iter()
        .flat_map(|log| log.events.iter())
        .filter(|event| event.event_type == "meridian-flip")
        .filter_map(|event| unix_of(&event.timestamp_utc))
        .collect();
    Ok(Night {
        poses,
        asiair,
        phd2,
        nina_flips,
        nina_files: outcome.nina_files_parsed,
        offsets,
    })
}

/// The lights of the target inside the window, by time.
fn poses_of(
    outcome: &AnalysisOutcome,
    target_folder: Option<&str>,
    window: Option<NightWindow>,
) -> Vec<Pose> {
    let target = target_folder
        .map(|t| t.trim().trim_matches('/'))
        .filter(|t| !t.is_empty() && *t != ".");
    // (path, pose): the rank is given once the poses are in order.
    let mut found: Vec<(String, Pose)> = outcome
        .frames
        .iter()
        .filter(|frame| frame.is_light)
        .filter(|frame| {
            target.is_none_or(|t| {
                frame
                    .path
                    .strip_prefix(t)
                    .is_some_and(|rest| rest.starts_with('/'))
            })
        })
        .filter_map(|frame| {
            let start = unix_of(frame.record.date_obs_utc.as_deref()?)?;
            window.is_none_or(|w| w.contains(start)).then(|| {
                (
                    frame.path.clone(),
                    Pose {
                        rank: 0,
                        start,
                        exposure: frame.record.exposure_seconds.unwrap_or(0.0).max(0.0),
                        temperature: frame.record.sensor_temp_c,
                        set_temperature: frame.record.set_temp_c,
                    },
                )
            })
        })
        .collect();
    found.sort_by(|a, b| a.1.start.total_cmp(&b.1.start).then_with(|| a.0.cmp(&b.0)));
    found
        .into_iter()
        .enumerate()
        .map(|(index, (_, pose))| Pose {
            rank: index + 1,
            ..pose
        })
        .collect()
}

/// The ASIAIR Autorun logs and the PHD2 guide logs of the folder, read with
/// the clocks found. Read only.
fn read_logs(
    root: &Path,
    options: &AnalysisOptions,
    problems: &mut Vec<Problem>,
) -> (Vec<AsiairAutorunLog>, Vec<Phd2Log>) {
    let mut asiair_logs = Vec::new();
    let mut phd2_logs = Vec::new();
    let Ok(walk) = walk_folder(root, None) else {
        return (asiair_logs, phd2_logs);
    };
    for file in walk
        .files
        .iter()
        .filter(|f| matches!(f.class, FileClass::Log))
    {
        let Ok(text) = std::fs::read_to_string(&file.path) else {
            continue;
        };
        let name = file.relative_path.rsplit('/').next().unwrap_or_default();
        if looks_like_phd2_guide_log(&text) {
            match phd2::parse_phd2_guide_log(
                &text,
                &phd2::Phd2Options {
                    utc_offset: options.phd2_utc_offset.clone(),
                },
            ) {
                Ok(log) => phd2_logs.push(log),
                Err(error) => problems.push(Problem {
                    code: "analysis_failed".to_string(),
                    detail: format!("{}: {error}", file.relative_path),
                }),
            }
        } else if is_asiair_translation(name, &text) {
            // The Chinese rendition of a sibling log: no event of its own.
        } else if looks_like_asiair_autorun_log(&text) {
            match asiair::parse_asiair_autorun_log(
                &text,
                &asiair::AsiairOptions {
                    utc_offset: options.asiair_utc_offset.clone(),
                },
            ) {
                Ok(log) => asiair_logs.push(log),
                Err(error) => problems.push(Problem {
                    code: "analysis_failed".to_string(),
                    detail: format!("{}: {error}", file.relative_path),
                }),
            }
        }
    }
    (asiair_logs, phd2_logs)
}

// ---------------------------------------------------------------------------
// Building an incident
// ---------------------------------------------------------------------------

struct Draft {
    kind: IncidentKind,
    start: f64,
    end: Option<f64>,
    code: &'static str,
    params: Value,
    description: String,
    evidence: Evidence,
    payload: Value,
}

impl Draft {
    fn build(self) -> DetectedIncident {
        let start = iso_seconds(self.start);
        DetectedIncident {
            id: format!("{}:{start}", self.kind.as_str()),
            event_type: self.kind,
            start,
            end: self.end.map(iso_seconds),
            code: self.code.to_string(),
            params: self.params,
            description: truncate_chars(&self.description, 300),
            evidence: self.evidence,
            payload: self.payload,
        }
    }
}

fn truncate_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// Two incidents of one genre in the same second take `#2`, `#3`: the key
/// stays unique.
pub(crate) fn unique_ids(incidents: Vec<DetectedIncident>) -> Vec<DetectedIncident> {
    let mut seen: HashSet<String> = HashSet::new();
    incidents
        .into_iter()
        .map(|mut incident| {
            let base = incident.id.clone();
            let mut n = 1;
            while !seen.insert(incident.id.clone()) {
                n += 1;
                incident.id = format!("{base}#{n}");
            }
            incident
        })
        .collect()
}

/// `11 min 47 s`, `47 s`, `1 h 02 min`: neutral English for a description.
fn span_text(seconds: f64) -> String {
    let total = seconds.round().max(0.0) as i64;
    if total < 60 {
        format!("{total} s")
    } else if total < 3600 {
        let (minutes, rest) = (total / 60, total % 60);
        if rest == 0 {
            format!("{minutes} min")
        } else {
            format!("{minutes} min {rest} s")
        }
    } else {
        format!("{} h {:02} min", total / 3600, (total % 3600) / 60)
    }
}

fn times_text(count: usize) -> String {
    if count == 1 {
        "once".to_string()
    } else {
        format!("{count} times")
    }
}

fn round_to(value: f64, decimals: i32) -> f64 {
    let factor = 10_f64.powi(decimals);
    (value * factor).round() / factor
}

fn evidence(source: EvidenceSource, code: &str, params: Value) -> Evidence {
    Evidence {
        source,
        code: code.to_string(),
        params,
    }
}

/// The groups of instants (sorted) whose neighbours are at most `gap` apart.
fn chains<T: Copy>(items: &[(f64, T)], gap: f64) -> Vec<Vec<(f64, T)>> {
    let mut groups: Vec<Vec<(f64, T)>> = Vec::new();
    for item in items {
        match groups.last_mut() {
            Some(group) if group.last().is_some_and(|last| item.0 - last.0 <= gap) => {
                group.push(*item);
            }
            _ => groups.push(vec![*item]),
        }
    }
    groups
}

fn sorted<T: Copy>(mut items: Vec<(f64, T)>) -> Vec<(f64, T)> {
    items.sort_by(|a, b| a.0.total_cmp(&b.0));
    items
}

// ---------------------------------------------------------------------------
// Pauses: a flip, a gap between poses
// ---------------------------------------------------------------------------

/// The flips of the target: the ASIAIR log's when the night has one, else
/// NINA's.
pub(crate) fn flips_of_asiair(logs: &[AsiairAutorunLog]) -> Vec<Flip> {
    let mut flips = Vec::new();
    for log in logs {
        let begins = sorted(
            log.meridian_flip_begin_events
                .iter()
                .filter_map(|e| unix_of(&e.timestamp_utc).map(|t| (t, ())))
                .collect(),
        );
        let ends = sorted(
            log.meridian_flip_end_events
                .iter()
                .filter_map(|e| unix_of(&e.timestamp_utc).map(|t| (t, e.succeeded)))
                .collect(),
        );
        for (index, (begin, ())) in begins.iter().enumerate() {
            let next = begins.get(index + 1).map(|b| b.0);
            let end = ends
                .iter()
                .find(|(at, _)| *at >= *begin && next.is_none_or(|n| *at < n));
            flips.push(Flip {
                begin: *begin,
                end: end.map(|e| e.0),
                succeeded: end.map(|e| e.1),
                source: EvidenceSource::Asiair,
            });
        }
    }
    flips
}

/// NINA writes a few lines for one flip and no end: they make one flip
/// without an end.
pub(crate) fn flips_of_nina(instants: &[f64]) -> Vec<Flip> {
    let items = sorted(instants.iter().map(|t| (*t, ())).collect());
    chains(&items, NINA_FLIP_CLUSTER_SECONDS)
        .into_iter()
        .map(|group| Flip {
            begin: group[0].0,
            end: None,
            succeeded: None,
            source: EvidenceSource::Nina,
        })
        .collect()
}

fn flips_of(night: &Night, scope: &Scope) -> Vec<Flip> {
    let flips = if night.asiair.is_empty() {
        flips_of_nina(&night.nina_flips)
    } else {
        flips_of_asiair(&night.asiair)
    };
    flips.into_iter().filter(|f| scope.holds(f.begin)).collect()
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Gap {
    after: usize,
    before: usize,
    from: f64,
    to: f64,
}

impl Gap {
    fn seconds(&self) -> f64 {
        self.to - self.from
    }
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len() % 2 == 1 {
        values[middle]
    } else {
        (values[middle - 1] + values[middle]) / 2.0
    }
}

/// What a night's gaps are made of: the cycle of its frames and the longest
/// gap it repeats.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Rhythm {
    /// The median time from the start of a pose to the start of the next
    /// (exposure and overhead): « one frame every ... ».
    pub cycle: f64,
    /// The routine gap: the median gap, or the longest gap the night repeats
    /// ([`ROUTINE_REPEAT_MIN`]), whichever is longer.
    pub routine: f64,
}

/// The routine gap of a night: the longest length that [`ROUTINE_REPEAT_MIN`]
/// gaps or more share (within [`ROUTINE_REPEAT_SPREAD`]), at most
/// [`ROUTINE_MAX_CYCLES`] cycles, never under the median gap.
fn routine_gap(lengths: &[f64], median_gap: f64, cycle: f64) -> f64 {
    let mut longest_first: Vec<f64> = lengths.to_vec();
    longest_first.sort_by(|a, b| b.total_cmp(a));
    let cap = ROUTINE_MAX_CYCLES * cycle;
    for &length in &longest_first {
        if length > cap || length <= median_gap {
            continue;
        }
        let shared = longest_first
            .iter()
            .filter(|&&other| other <= length && other * ROUTINE_REPEAT_SPREAD >= length)
            .count();
        if shared >= ROUTINE_REPEAT_MIN {
            return length;
        }
    }
    median_gap
}

/// The gaps between two poses (the end of one to the start of the next) that
/// are pauses, and the rhythm of the night they are judged against.
fn long_gaps(poses: &[Pose]) -> (Vec<Gap>, Rhythm) {
    let none = Rhythm {
        cycle: 0.0,
        routine: 0.0,
    };
    if poses.len() < GAP_MIN_POSES {
        return (Vec::new(), none);
    }
    let gaps: Vec<Gap> = poses
        .windows(2)
        .map(|pair| Gap {
            after: pair[0].rank,
            before: pair[1].rank,
            from: pair[0].end(),
            to: pair[1].start,
        })
        .collect();
    let lengths: Vec<f64> = gaps.iter().map(|g| g.seconds().max(0.0)).collect();
    let mut cycles: Vec<f64> = poses
        .windows(2)
        .map(|pair| (pair[1].start - pair[0].start).max(0.0))
        .collect();
    let cycle = median(&mut cycles);
    let median_gap = median(&mut lengths.clone());
    let rhythm = Rhythm {
        cycle,
        routine: routine_gap(&lengths, median_gap, cycle),
    };
    let floor = GAP_MIN_SECONDS.max(GAP_MIN_CYCLES * cycle);
    let long = gaps
        .into_iter()
        .filter(|g| g.seconds() >= floor && g.seconds() > GAP_FACTOR * rhythm.routine)
        .collect();
    (long, rhythm)
}

fn overlaps(gap: &Gap, flip: &Flip) -> bool {
    let end = flip.end.unwrap_or(flip.begin);
    flip.begin <= gap.to + FLIP_GAP_SLACK_SECONDS && end >= gap.from - FLIP_GAP_SLACK_SECONDS
}

/// The pauses: each flip, with the gap of poses it covers when there is one
/// (one pause, not two), then the gaps no flip explains.
pub(crate) fn pause_incidents(
    poses: &[Pose],
    flips: &[Flip],
    unproposed: &mut Vec<Unproposed>,
) -> Vec<DetectedIncident> {
    let (gaps, rhythm) = long_gaps(poses);
    let mut used = vec![false; gaps.len()];
    let mut found = Vec::new();
    let mut unbounded = 0;
    for flip in flips {
        let covered = gaps
            .iter()
            .enumerate()
            .find(|(index, gap)| !used[*index] && overlaps(gap, flip))
            .map(|(index, _)| index);
        if let Some(index) = covered {
            used[index] = true;
        }
        let gap = covered.map(|index| gaps[index]);
        let Some(end) = flip.end.or(gap.map(|g| g.to)) else {
            unbounded += 1;
            continue;
        };
        found.push(flip_pause(flip, end, gap.as_ref()));
    }
    for (gap, _) in gaps.iter().zip(&used).filter(|(_, used)| !**used) {
        found.push(gap_pause(gap, rhythm.cycle));
    }
    if unbounded > 0 {
        unproposed.push(Unproposed {
            code: "meridian_flip_unbounded".to_string(),
            count: unbounded,
            params: json!({}),
        });
    }
    found
}

fn flip_pause(flip: &Flip, end: f64, gap: Option<&Gap>) -> DetectedIncident {
    let duration = (end - flip.begin).max(0.0);
    let mut params = json!({ "durationSeconds": duration.round() as i64 });
    if let Some(succeeded) = flip.succeeded {
        params["succeeded"] = json!(succeeded);
    }
    let mut description = format!("Meridian flip, {}", span_text(duration));
    if let Some(gap) = gap {
        params["afterPose"] = json!(gap.after);
        params["beforePose"] = json!(gap.before);
        params["gapSeconds"] = json!(gap.seconds().round() as i64);
        description.push_str(&format!(
            "; no frame between {} and {}",
            gap.after, gap.before
        ));
    }
    Draft {
        kind: IncidentKind::Pause,
        start: flip.begin,
        end: Some(end),
        code: "meridian_flip",
        params: params.clone(),
        description,
        evidence: evidence(flip.source, "meridian_flip", params),
        payload: json!({ "reason": "Meridian flip", "cause": "meridian_flip" }),
    }
    .build()
}

fn gap_pause(gap: &Gap, cycle: f64) -> DetectedIncident {
    let params = json!({
        "afterPose": gap.after,
        "beforePose": gap.before,
        "gapSeconds": gap.seconds().round() as i64,
        "typicalSeconds": cycle.round() as i64,
    });
    let description = format!(
        "No frame for {} between frames {} and {}",
        span_text(gap.seconds()),
        gap.after,
        gap.before
    );
    Draft {
        kind: IncidentKind::Pause,
        start: gap.from,
        end: Some(gap.to),
        code: "pose_gap",
        params: params.clone(),
        payload: json!({ "reason": truncate_chars(&description, 200) }),
        description,
        evidence: evidence(EvidenceSource::Fits, "no_pose_between", params),
    }
    .build()
}

// ---------------------------------------------------------------------------
// Guide star lost
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Loss {
    Lost,
    NoStarFound,
}

fn guide_losses(night: &Night, scope: &Scope) -> (EvidenceSource, Vec<(f64, Loss)>) {
    if !night.asiair.is_empty() {
        let items = night
            .asiair
            .iter()
            .flat_map(|log| log.guide_star_lost_events.iter())
            .filter_map(|e| {
                let kind = match e.kind {
                    GuideStarLoss::Lost => Loss::Lost,
                    GuideStarLoss::NoStarFound => Loss::NoStarFound,
                };
                unix_of(&e.timestamp_utc).map(|t| (t, kind))
            })
            .filter(|(t, _)| scope.holds(*t))
            .collect();
        return (EvidenceSource::Asiair, sorted(items));
    }
    let drops = sorted(
        night
            .phd2
            .iter()
            .flat_map(|log| log.sessions.iter())
            .flat_map(|session| session.dropped_frames.iter())
            .filter_map(|d| unix_of(&d.timestamp_utc).map(|t| (t, ())))
            .filter(|(t, _)| scope.holds(*t))
            .collect(),
    );
    let losses = chains(&drops, PHD2_DROP_EPISODE_GAP_SECONDS)
        .into_iter()
        .filter(|episode| episode.len() >= PHD2_LOSS_MIN_DROPS)
        .map(|episode| (episode[0].0, Loss::Lost))
        .collect();
    (EvidenceSource::Phd2, losses)
}

fn guide_loss_incidents(night: &Night, scope: &Scope) -> Vec<DetectedIncident> {
    let (source, losses) = guide_losses(night, scope);
    group_guide_losses(source, &losses)
}

/// The guide star losses (sorted), grouped: one `equipment_issue` of
/// `minor` severity per group.
pub(crate) fn group_guide_losses(
    source: EvidenceSource,
    losses: &[(f64, Loss)],
) -> Vec<DetectedIncident> {
    chains(losses, GUIDE_LOSS_GROUP_SECONDS)
        .into_iter()
        .map(|group| {
            let mut count = 0_usize;
            let mut not_found = 0_usize;
            let mut last_lost: Option<f64> = None;
            for (at, kind) in &group {
                match kind {
                    Loss::Lost => {
                        count += 1;
                        last_lost = Some(*at);
                    }
                    Loss::NoStarFound => {
                        not_found += 1;
                        let follows = last_lost
                            .is_some_and(|lost| *at - lost <= NO_STAR_FOLLOWS_LOST_SECONDS);
                        if !follows {
                            count += 1;
                        }
                    }
                }
            }
            let first = group[0].0;
            let last = group[group.len() - 1].0;
            let params = json!({ "count": count, "notFound": not_found });
            let mut description = format!("Guide star lost {}", times_text(count));
            if not_found > 0 {
                description.push_str(&format!(
                    ", {} with no star found again",
                    times_text(not_found)
                ));
            }
            Draft {
                kind: IncidentKind::EquipmentIssue,
                start: first,
                end: (iso_seconds(last) != iso_seconds(first)).then_some(last),
                code: "guide_star_lost",
                params: params.clone(),
                payload: json!({ "description": truncate_chars(&description, 300), "severity": "minor" }),
                description,
                evidence: evidence(source, "guide_star_lost", params),
            }
            .build()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Guiding timeouts and spikes
// ---------------------------------------------------------------------------

fn guiding_timeout_incidents(night: &Night, scope: &Scope) -> Vec<DetectedIncident> {
    let (source, items): (EvidenceSource, Vec<(f64, ())>) = if night.asiair.is_empty() {
        let items = night
            .phd2
            .iter()
            .flat_map(|log| log.sessions.iter())
            .flat_map(|session| session.settle_events.iter())
            .filter(|e| e.outcome == SettleOutcome::Failed)
            .filter_map(|e| unix_of(&e.timestamp_utc).map(|t| (t, ())))
            .collect();
        (EvidenceSource::Phd2, items)
    } else {
        let items = night
            .asiair
            .iter()
            .flat_map(|log| log.settle_timeout_events.iter())
            .filter_map(|e| unix_of(&e.timestamp_utc).map(|t| (t, ())))
            .collect();
        (EvidenceSource::Asiair, items)
    };
    let items = sorted(items.into_iter().filter(|(t, _)| scope.holds(*t)).collect());
    group_timeouts(source, &items)
}

/// The settle timeouts (sorted), one `guiding_timeouts` event per group of
/// [`GUIDING_GROUP_SECONDS`].
pub(crate) fn group_timeouts(source: EvidenceSource, items: &[(f64, ())]) -> Vec<DetectedIncident> {
    chains(items, GUIDING_GROUP_SECONDS)
        .into_iter()
        .map(|group| {
            let count = group.len();
            let (first, last) = (group[0].0, group[count - 1].0);
            let params = json!({
                "count": count,
                "firstAt": iso_seconds(first),
                "lastAt": iso_seconds(last),
            });
            let description = if count == 1 {
                "Guiding did not settle in time once".to_string()
            } else {
                format!("Guiding did not settle in time {count} times")
            };
            Draft {
                kind: IncidentKind::GuidingTimeouts,
                start: first,
                end: (iso_seconds(last) != iso_seconds(first)).then_some(last),
                code: "guiding_timeouts",
                payload: json!({
                    "count": count,
                    "firstAt": iso_seconds(first),
                    "lastAt": iso_seconds(last),
                    "description": description,
                }),
                params: params.clone(),
                description,
                evidence: evidence(source, "settle_timeout", json!({ "count": count })),
            }
            .build()
        })
        .collect()
}

/// Whether `at` falls in the exposure of one of the poses (sorted).
fn during_exposure(poses: &[Pose], at: f64) -> bool {
    let after = poses.partition_point(|p| p.start <= at);
    after > 0 && at <= poses[after - 1].end()
}

/// The spikes of PHD2's guiding error: from the samples taken while a pose
/// is exposing (what the guiding does between two poses does not touch an
/// image: the slew of a re-centering is not a spike), outside a dither
/// settle. Without any pose, the samples of the scope.
fn spike_incidents(
    logs: &[Phd2Log],
    poses: &[Pose],
    scope: &Scope,
    unproposed: &mut Vec<Unproposed>,
) -> Vec<DetectedIncident> {
    let mut spikes: Vec<(f64, f64)> = Vec::new();
    for session in logs.iter().flat_map(|log| log.sessions.iter()) {
        if session.pixel_scale_arcsec_per_px.is_none() {
            // Without the pixel scale the offsets are pixels, not arcseconds.
            continue;
        }
        let excluded = settle_windows(session);
        for sample in &session.samples {
            let Some(at) = unix_of(&sample.timestamp_utc) else {
                continue;
            };
            let counts = if poses.is_empty() {
                scope.holds(at)
            } else {
                during_exposure(poses, at)
            };
            if sample.error_code != 0
                || !counts
                || excluded.iter().any(|(from, to)| at >= *from && at <= *to)
            {
                continue;
            }
            let error = sample.ra_offset_arcsec.hypot(sample.dec_offset_arcsec);
            if error >= SPIKE_ARCSEC {
                spikes.push((at, error));
            }
        }
    }
    spikes.sort_by(|a, b| a.0.total_cmp(&b.0));
    group_spikes(&spikes, unproposed)
}

/// The spans of a PHD2 session where the error is a dither's own: the first
/// seconds of the session, and from each settling start or dither.
fn settle_windows(session: &phd2::GuidingSession) -> Vec<(f64, f64)> {
    let mut windows = Vec::new();
    if let Some(start) = session.started_at_utc.as_deref().and_then(unix_of) {
        windows.push((start, start + SETTLE_EXCLUSION_SECONDS));
    }
    let settles: Vec<(f64, SettleOutcome)> = session
        .settle_events
        .iter()
        .filter_map(|e| unix_of(&e.timestamp_utc).map(|t| (t, e.outcome)))
        .collect();
    for (index, (at, outcome)) in settles.iter().enumerate() {
        if *outcome != SettleOutcome::Started {
            continue;
        }
        let ended = settles[index + 1..]
            .iter()
            .find(|(_, o)| *o != SettleOutcome::Started)
            .map_or(*at, |(t, _)| *t);
        windows.push((*at, (*at + SETTLE_EXCLUSION_SECONDS).max(ended + 30.0)));
    }
    for dither in &session.dither_events {
        if let Some(at) = unix_of(&dither.timestamp_utc) {
            windows.push((at, at + SETTLE_EXCLUSION_SECONDS + 30.0));
        }
    }
    windows
}

/// The spikes `(instant, error in arcseconds)` (sorted), one event per group
/// of [`GUIDING_GROUP_SECONDS`]; a group under [`SPIKES_MIN_COUNT`] is told
/// in `unproposed`.
pub(crate) fn group_spikes(
    spikes: &[(f64, f64)],
    unproposed: &mut Vec<Unproposed>,
) -> Vec<DetectedIncident> {
    let mut found = Vec::new();
    let mut dropped = 0;
    for group in chains(spikes, GUIDING_GROUP_SECONDS) {
        if group.len() < SPIKES_MIN_COUNT {
            dropped += group.len();
            continue;
        }
        let count = group.len();
        let (first, last) = (group[0].0, group[count - 1].0);
        let max = round_to(group.iter().map(|s| s.1).fold(0.0, f64::max), 1);
        let params = json!({
            "count": count,
            "maxArcSeconds": max,
            "thresholdArcSeconds": SPIKE_ARCSEC,
            "firstAt": iso_seconds(first),
            "lastAt": iso_seconds(last),
        });
        let description =
            format!("Guiding error above {SPIKE_ARCSEC} arcseconds {count} times, up to {max}");
        found.push(
            Draft {
                kind: IncidentKind::GuidingSpikes,
                start: first,
                end: (iso_seconds(last) != iso_seconds(first)).then_some(last),
                code: "guiding_spikes",
                payload: json!({
                    "count": count,
                    "maxArcSeconds": max,
                    "firstAt": iso_seconds(first),
                    "lastAt": iso_seconds(last),
                    "description": description,
                }),
                params: params.clone(),
                description,
                evidence: evidence(EvidenceSource::Phd2, "guiding_spikes", params),
            }
            .build(),
        );
    }
    if dropped > 0 {
        unproposed.push(Unproposed {
            code: "guiding_spikes".to_string(),
            count: dropped,
            params: json!({ "minimumPerGroup": SPIKES_MIN_COUNT }),
        });
    }
    found
}

// ---------------------------------------------------------------------------
// From the headers: exposure, temperature
// ---------------------------------------------------------------------------

/// A run of poses of one exposure: the exposure, its first pose, how many.
type Run = (f64, Pose, usize);

/// A change of exposure from one run of poses to the next. A run shorter than
/// [`EXPOSURE_RUN_MIN_POSES`] is a test frame and is ignored; more than
/// [`EXPOSURE_CHANGE_MAX`] changes is a sequence that alternates on purpose,
/// told in `unproposed`.
pub(crate) fn exposure_change_incidents(
    poses: &[Pose],
    unproposed: &mut Vec<Unproposed>,
) -> Vec<DetectedIncident> {
    let mut runs: Vec<Run> = Vec::new();
    for pose in poses.iter().filter(|p| p.exposure > 0.0) {
        match runs.last_mut() {
            Some(run) if (run.0 - pose.exposure).abs() < 0.0005 => run.2 += 1,
            _ => runs.push((pose.exposure, *pose, 1)),
        }
    }
    runs.retain(|run| run.2 >= EXPOSURE_RUN_MIN_POSES);
    // Runs of the same exposure that a test frame separated are one run.
    let mut merged: Vec<Run> = Vec::new();
    for run in runs {
        match merged.last_mut() {
            Some(last) if (last.0 - run.0).abs() < 0.0005 => last.2 += run.2,
            _ => merged.push(run),
        }
    }
    let changes: Vec<(&Run, &Run)> = merged.iter().zip(merged.iter().skip(1)).collect();
    if changes.len() > EXPOSURE_CHANGE_MAX {
        unproposed.push(Unproposed {
            code: "exposure_change".to_string(),
            count: changes.len(),
            params: json!({}),
        });
        return Vec::new();
    }
    changes
        .into_iter()
        .map(|(from, to)| {
            let (from_seconds, to_seconds) = (round_to(from.0, 3), round_to(to.0, 3));
            let params = json!({
                "fromSeconds": from_seconds,
                "toSeconds": to_seconds,
                "atPose": to.1.rank,
            });
            let description = format!("Exposure changed from {from_seconds} s to {to_seconds} s");
            Draft {
                kind: IncidentKind::ExposureChange,
                start: to.1.start,
                end: None,
                code: "exposure_change",
                payload: json!({
                    "fromSeconds": from_seconds,
                    "toSeconds": to_seconds,
                    "description": description,
                }),
                params: params.clone(),
                description,
                evidence: evidence(EvidenceSource::Fits, "exposure_change", params),
            }
            .build()
        })
        .collect()
}

/// The temperature a cooled sensor is meant to hold: the median set point of
/// the poses (`SET-TEMP`), else the night's median temperature when the
/// sensor held it ([`TEMPERATURE_HELD_SHARE`] of the poses within
/// [`TEMPERATURE_HOLD_BAND_CELSIUS`]); `None` for a sensor that follows the
/// air.
fn held_temperature(poses: &[Pose]) -> Option<f64> {
    let mut set_points: Vec<f64> = poses
        .iter()
        .filter_map(|p| p.set_temperature.filter(|t| t.is_finite()))
        .collect();
    if !set_points.is_empty() {
        return Some(median(&mut set_points));
    }
    let temperatures: Vec<f64> = poses
        .iter()
        .filter_map(|p| p.temperature.filter(|t| t.is_finite()))
        .collect();
    if temperatures.is_empty() {
        return None;
    }
    let middle = median(&mut temperatures.clone());
    let held = temperatures
        .iter()
        .filter(|t| (**t - middle).abs() <= TEMPERATURE_HOLD_BAND_CELSIUS)
        .count();
    (held as f64 >= TEMPERATURE_HELD_SHARE * temperatures.len() as f64).then_some(middle)
}

/// The cooled sensor's temperature (`CCD-TEMP`) went [`TEMPERATURE_DRIFT_CELSIUS`]
/// or more away from the value it holds ([`held_temperature`]): from the last
/// pose that still held it to the pose that went furthest. A sensor that is
/// not cooled follows the air and says nothing.
pub(crate) fn temperature_drift_incidents(poses: &[Pose]) -> Vec<DetectedIncident> {
    let Some(held) = held_temperature(poses) else {
        return Vec::new();
    };
    let with_temperature: Vec<(&Pose, f64)> = poses
        .iter()
        .filter_map(|p| p.temperature.filter(|t| t.is_finite()).map(|t| (p, t)))
        .collect();
    let Some(&(farthest, to)) = with_temperature
        .iter()
        .max_by(|a, b| (a.1 - held).abs().total_cmp(&(b.1 - held).abs()))
    else {
        return Vec::new();
    };
    if (to - held).abs() < TEMPERATURE_DRIFT_CELSIUS {
        return Vec::new();
    }
    // The start: the last pose before the farthest one that was still near
    // the held value (or the first pose).
    let first = with_temperature
        .iter()
        .rev()
        .skip_while(|(p, _)| p.start >= farthest.start)
        .find(|(_, t)| (*t - held).abs() < TEMPERATURE_DRIFT_CELSIUS)
        .or_else(|| with_temperature.first())
        .map_or(*farthest, |(p, _)| **p);
    let (from, to) = (round_to(held, 1), round_to(to, 1));
    let params = json!({ "fromCelsius": from, "toCelsius": to });
    let description = format!("Sensor temperature moved from {from} C to {to} C");
    vec![
        Draft {
            kind: IncidentKind::SensorTemperatureDrift,
            start: first.start,
            end: Some(farthest.start),
            code: "sensor_temperature_drift",
            payload: json!({
                "fromCelsius": from,
                "toCelsius": to,
                "description": description,
            }),
            params: params.clone(),
            description,
            evidence: evidence(EvidenceSource::Fits, "sensor_temperature_drift", params),
        }
        .build(),
    ]
}

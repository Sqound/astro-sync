//! "Incident list": what a night's logs and
//! frame headers show of its incidents, merged with the signals already
//! declared in the session, and written back to the session once the person
//! has chosen. Nothing is written by the reading; nothing is invented: every
//! incident carries the evidence it comes from.
//!
//! - **Detection** ([`detect_incidents`]): from the night's folder (logs and
//!   frame headers, read only), the night's window and the clock of the logs.
//!   Pauses (a meridian flip, or a gap between two frames far longer than
//!   the night's rhythm), guide star losses, and the four further kinds the
//!   service accepts (`exposure_change`, `guiding_timeouts`,
//!   `sensor_temperature_drift`, `guiding_spikes`). The dawn, the cloud and the
//!   trailed frames come from the measure of the poses ([`crate::pose_stats`]),
//!   made by the window: not here.
//! - **Declared signals** ([`read_declared_events`], [`merge_rows`]): `GET
//!   /v1/sessions/{id}/events`, the effective events (what a rework replaced
//!   is not read); a pause and its `resume` make one interval.
//! - **Writing** ([`apply`]): two calls, in this order. First the additions,
//!   `POST /v1/sessions/{id}/events`, **idempotent**: the `eventId` is
//!   derived from the session and the detection key ([`event_id_for`]), so
//!   sending the list again after a cut writes nothing twice. Then the
//!   replacements, `PUT /v1/sessions/{id}/rework` (`eventReplacements`,
//!   `If-Match`), all or nothing; a refusal is an answer
//!   ([`ApplyOutcome::replacements_refused`]), never an error, and the
//!   additions already made stay told.
//!
//! **No text for the person in the core.** Every incident has a `code` and
//! `params` from which the window composes its own sentence, in the person's
//! language; its `description` is neutral English, what goes to the service
//! if the person does not edit it. The one fragment of words is the separator
//! of [`merged_text`] ("Your text · detected:
//! ..."); [`merged_text_with`] takes the one of the person's language.
//!
//! **The codes and params** (the window depends on them; `params` is a JSON
//! object, numbers in seconds and degrees Celsius, instants in ISO 8601 UTC):
//!
//! | `eventType`                | `code`              | `params`                                                    |
//! | -------------------------- | ------------------- | ----------------------------------------------------------- |
//! | `pause`                    | `meridian_flip`     | `durationSeconds`, `succeeded?`, and when a gap of frames covers it: `afterPose`, `beforePose`, `gapSeconds` |
//! | `pause`                    | `pose_gap`          | `afterPose`, `beforePose`, `gapSeconds`, `typicalSeconds`   |
//! | `equipment_issue`          | `guide_star_lost`   | `count`, `notFound`                                         |
//! | `guiding_timeouts`         | `guiding_timeouts`  | `count`, `firstAt`, `lastAt`                                |
//! | `exposure_change`          | `exposure_change`   | `fromSeconds`, `toSeconds`, `atPose`                        |
//! | `sensor_temperature_drift` | `sensor_temperature_drift` | `fromCelsius`, `toCelsius`                           |
//! | `guiding_spikes`           | `guiding_spikes`    | `count`, `maxArcSeconds`, `thresholdArcSeconds`, `firstAt`, `lastAt` |
//!
//! The thresholds are named constants of [`detect`], tuned on the real nights
//! (Sadr of 14 to 15 August 2026; then, the Seestar night
//! of M 45 and eleven Askar 50P nights). The pauses and the sensor
//! temperature are judged against what each night does by itself (its frame
//! cycle, the gaps it repeats, the cooler's set point), not against fixed
//! numbers alone.

mod apply;
mod declared;
mod detect;

#[cfg(test)]
mod tests;

pub use apply::{
    AddedEvent, AddedIncident, Apply, ApplyError, ApplyOutcome, DETECTED_SEPARATOR,
    EVENT_ID_PREFIX, MAX_ADDS_PER_BATCH, MAX_REPLACEMENTS, Rejection, Replacement,
    ReplacementsRefused, apply, event_id_for, merged_payload, merged_text, merged_text_with,
};
pub use declared::{
    DeclaredEvent, DeclaredEvents, IncidentsTransport, MergedRow, PAIR_PROXIMITY_SECONDS,
    ReworkMark, RowOrigin, declared_from_events, merge_rows, read_declared_events,
};
pub use detect::{
    DetectInput, Detection, EXPOSURE_CHANGE_MAX, EXPOSURE_RUN_MIN_POSES, FLIP_GAP_SLACK_SECONDS,
    GAP_FACTOR, GAP_MIN_CYCLES, GAP_MIN_POSES, GAP_MIN_SECONDS, GUIDE_LOSS_GROUP_SECONDS,
    GUIDING_GROUP_SECONDS, LOG_SPAN_MARGIN_SECONDS, LogsRead, NO_STAR_FOLLOWS_LOST_SECONDS,
    PHD2_DROP_EPISODE_GAP_SECONDS, PHD2_LOSS_MIN_DROPS, Problem, ROUTINE_MAX_CYCLES,
    ROUTINE_REPEAT_MIN, ROUTINE_REPEAT_SPREAD, SETTLE_EXCLUSION_SECONDS, SPIKE_ARCSEC,
    SPIKES_MIN_COUNT, TEMPERATURE_DRIFT_CELSIUS, TEMPERATURE_HELD_SHARE,
    TEMPERATURE_HOLD_BAND_CELSIUS, Unproposed, detect_incidents,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::time_util::civil_from_days;

/// The kinds of signal the list shows: the four of the app, and the four
/// further ones the service accepts (`session-events/*.schema.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncidentKind {
    /// "Pause": an interval (`pause` then `resume`).
    Pause,
    /// "Sky condition".
    Condition,
    /// "Equipment incident".
    EquipmentIssue,
    Note,
    ExposureChange,
    GuidingTimeouts,
    SensorTemperatureDrift,
    GuidingSpikes,
}

impl IncidentKind {
    pub const ALL: [Self; 8] = [
        Self::Pause,
        Self::Condition,
        Self::EquipmentIssue,
        Self::Note,
        Self::ExposureChange,
        Self::GuidingTimeouts,
        Self::SensorTemperatureDrift,
        Self::GuidingSpikes,
    ];

    /// The `type` of the event, as the registry spells it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Condition => "condition",
            Self::EquipmentIssue => "equipment_issue",
            Self::Note => "note",
            Self::ExposureChange => "exposure_change",
            Self::GuidingTimeouts => "guiding_timeouts",
            Self::SensorTemperatureDrift => "sensor_temperature_drift",
            Self::GuidingSpikes => "guiding_spikes",
        }
    }

    /// The kind of an event `type`; `None` for any other (a type the list
    /// does not show, or one a newer service stores).
    #[must_use]
    pub fn parse(event_type: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == event_type)
    }

    /// Whether a rework can replace a signal of this kind
    /// (`session-rework.schema.json`: `note`, `condition`, `equipment_issue`
    /// and `pause`).
    #[must_use]
    pub fn replaceable(self) -> bool {
        matches!(
            self,
            Self::Pause | Self::Condition | Self::EquipmentIssue | Self::Note
        )
    }
}

/// Where an incident's evidence is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSource {
    Asiair,
    Phd2,
    Nina,
    /// The headers of the frames.
    Fits,
}

/// The grey line under an incident: where it was read and what, as a code and
/// parameters for the window to say in the person's language.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub source: EvidenceSource,
    pub code: String,
    pub params: Value,
}

/// One incident found in the night's files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedIncident {
    /// The detection key, `genre:instant`: the genre is the event type, the
    /// instant its start in UTC to the second (`pause:2026-08-14T22:13:38Z`).
    /// Stable from one reading to the next; a second incident of the same
    /// genre and second takes `#2`, `#3`.
    pub id: String,
    pub event_type: IncidentKind,
    /// ISO 8601 UTC, to the second.
    pub start: String,
    /// The end of an interval (a pause) or of a group, ISO 8601 UTC.
    pub end: Option<String>,
    pub code: String,
    pub params: Value,
    /// Neutral English (at most 300 characters): what goes to the service
    /// when the person does not edit it.
    pub description: String,
    pub evidence: Evidence,
    /// The exact payload of the event type, ready to send and valid against
    /// its schema.
    pub payload: Value,
}

impl DetectedIncident {
    /// The addition of this incident to the session, as it is.
    #[must_use]
    pub fn as_added(&self) -> AddedIncident {
        AddedIncident {
            key: self.id.clone(),
            event_type: self.event_type,
            at: self.start.clone(),
            until: (self.event_type == IncidentKind::Pause)
                .then(|| self.end.clone())
                .flatten(),
            payload: self.payload.clone(),
        }
    }
}

/// `2026-08-14T22:13:38Z` of Unix seconds (rounded to the second).
pub(crate) fn iso_seconds(unix_seconds: f64) -> String {
    let whole = unix_seconds.round() as i64;
    let days = whole.div_euclid(86_400);
    let of_day = whole.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        of_day / 3600,
        (of_day % 3600) / 60,
        of_day % 60
    )
}

//! "Detect sessions": the one engine that reads a folder of FITS, groups its
//! frames into nights, sessions, targets and multi-night plans, and matches
//! them to what the person already has. It is called two ways, with the same
//! code: "Detect sessions" (any folder) and the detection sheet of the sending
//! of one session, bounded by that session ([`SessionScope`]: its date range
//! and its targets).
//!
//! **Nothing is written.** The engine reads headers (never a pixel) and the
//! logs, and answers a [`Proposal`]: one line per element, ticked or not,
//! with its reason, serialisable for the window and the command line. The
//! creation on the service is [`create::create_sessions`], after the person's
//! "Create…"; the only writing gesture on the disk is [`organise`], on an
//! explicit choice, never by default.
//!
//! The three steps, each a function of its own so a caller can read the
//! person's data from the service between them (it needs the date range):
//!
//! 1. [`read_locations`]: the three locations (FITS required, calibration
//!    and logs optional), sub-folders included, symbolic links followed
//!    (`crate::scanner::walk_folder`, loops reported), one record per
//!    physical file (a file reached by two paths is read once), with the
//!    progress (`WorkKind::Headers`) and the stop switch.
//! 2. [`group_frames`]: nights between two noons at the place's longitude
//!    (the rule of `crate::detection::NightWindow`, or the place's civil
//!    noon when its offset is known), targets (`OBJECT` in the catalogue,
//!    else the coordinates, else the folder's name, else "plate solving to
//!    do" on one frame), sessions (one night, one place), the rebuilt
//!    one-night plan, multi-night groups, calibration and logs attached, the
//!    suspect clock ([`clock`]).
//! 3. [`propose`]: places within [`PLACE_RADIUS_KM`], equipment, existing
//!    sessions and plans, the rights of the account, and the lines.
//!
//! [`detect_sessions`] runs the three for a caller that has the person's
//! data at hand.

pub mod band;
mod clock;
pub mod create;
mod group;
mod organise;
mod propose;
pub mod provisional;
mod read;
pub mod service;
pub mod treat;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::catalogue_targets::CatalogueTarget;
use crate::detection::FrameType;
use crate::local_work::{CancelFlag, ProgressSink};
use crate::scanner::{LinkIssue, ScanError};

pub use clock::{ClockFinding, ClockReason, ClockReasonCode, ClockSample, PlaceUsed};
pub use create::{
    BulkCreateOutcome, BulkCreateRequest, CreatedMultiNight, CreatedSession, LineAnswer,
    PlaceAnswer, SessionShift, SkippedElement, bulk_body, create_sessions,
};
pub use group::Grouping;
pub use organise::{
    InPlaceAs, NightNaming, OrganiseAction, OrganiseActionKind, OrganiseError, OrganiseFailure,
    OrganiseMode, OrganiseOptions, OrganiseOutcome, OrganisePlan, organise, plan_organise,
    plan_organise_named,
};
pub use propose::{RightsRefresh, propose};
pub use read::{Reading, read_locations};
pub use treat::{TreatError, TreatmentNight, TreatmentTarget, prepare_treatments};

/// Two places closer than this are the same place (10 km).
pub const PLACE_RADIUS_KM: f64 = 10.0;
/// Two nights of a multi-night group are this many days apart at most.
pub const MULTI_NIGHT_GAP_DAYS: i64 = 30;
/// A calibration set is attached to a night this many days away at most.
pub const CALIBRATION_MAX_DAYS: f64 = 30.0;

/* --------------------------------- Input --------------------------------- */

fn yes() -> bool {
    true
}

/// The three locations and how to read them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectRequest {
    /// The folders of FITS (lights): at least one.
    pub fits: Vec<PathBuf>,
    /// The folders of calibration frames (darks, flats, dark-flats, bias).
    #[serde(default)]
    pub calibration: Vec<PathBuf>,
    /// The folders of logs (N.I.N.A., ASIAIR, PHD2).
    #[serde(default)]
    pub logs: Vec<PathBuf>,
    /// "Also read the sub-folders" (ticked by default).
    #[serde(default = "yes")]
    pub subfolders: bool,
    /// The detection of the sending of one session: only its range counts.
    #[serde(default)]
    pub scope: Option<SessionScope>,
    /// The filter of every light of the run, given by the person: it stands
    /// over the `FILTER` cards and over what the folder names say.
    #[serde(default)]
    pub filter: Option<String>,
}

/// The limits of a declared session (the detection sheet of its sending).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionScope {
    /// Start and end of the session, ISO 8601 UTC.
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub targets: Vec<ScopeTarget>,
}

/// A target of the declared session.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeTarget {
    pub id: String,
    pub designation: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub ra_hours: Option<f64>,
    #[serde(default)]
    pub dec_degrees: Option<f64>,
}

/// What the person already has, read from the service by the caller
/// ([`service::read_user_data`]) or given as files on the command line: the
/// core makes no network call.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserData {
    #[serde(default)]
    pub places: Vec<KnownPlace>,
    /// "Default place": the place assumed for frames that name none.
    #[serde(default)]
    pub default_place_id: Option<String>,
    #[serde(default)]
    pub equipment: Vec<KnownEquipment>,
    #[serde(default)]
    pub sessions: Vec<KnownSession>,
    #[serde(default)]
    pub plans: Vec<KnownPlan>,
    #[serde(default)]
    pub account: AccountRights,
}

/// One of "My places".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownPlace {
    pub id: String,
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
    /// IANA name, carried as is (the core has no time-zone database).
    #[serde(default)]
    pub time_zone: Option<String>,
    /// The place's offset from UTC at the dates read, minutes east, when the
    /// caller knows it (the window computes it from `timeZone`): the night is
    /// then cut at civil noon, else at mean solar noon.
    #[serde(default)]
    pub utc_offset_minutes: Option<i32>,
}

/// One item of "My equipment".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownEquipment {
    pub id: String,
    pub name: String,
    /// `telescope`, `camera`, `mount`, … as the service types it.
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub manufacturer: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub focal_length_mm: Option<f64>,
}

/// A session the person already has.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownSession {
    pub id: String,
    #[serde(default)]
    pub plan_id: Option<String>,
    /// `YYYY-MM-DD`, the evening of the night.
    pub evening_date: String,
    #[serde(default)]
    pub place_id: Option<String>,
    #[serde(default)]
    pub targets: Vec<KnownSlot>,
}

/// A target of an existing session and, when known, its slot.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownSlot {
    #[serde(default)]
    pub target_id: Option<String>,
    #[serde(default)]
    pub designation: Option<String>,
    /// ISO 8601 UTC.
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub end: Option<String>,
    /// The filter the session declares for it (the person's own text).
    #[serde(default)]
    pub filter: Option<String>,
}

/// A plan the person already has.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownPlan {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub evening_date: String,
    /// `pending` (to execute), `executed`, `abandoned`, `archived`.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub target_ids: Vec<String>,
    #[serde(default)]
    pub designations: Vec<String>,
    /// The session the plan was run in, when it was.
    #[serde(default)]
    pub session_id: Option<String>,
}

impl KnownPlan {
    /// Planned and never executed.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        self.session_id.is_none() && matches!(self.status.as_str(), "pending" | "active" | "")
    }
}

/// The rights the proposal follows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountRights {
    /// "Advanced Planning" (`planner.mosaic`): multi-night plans.
    #[serde(default)]
    pub advanced_planning: bool,
    /// Several plans a night (`planner.multiple`): without it, "one plan per
    /// night".
    #[serde(default)]
    pub several_plans_per_night: bool,
}

/// Solves one frame: the centre `(ra, dec)` in degrees, or why it could not
/// be solved.
pub type FrameSolver<'a> =
    dyn Fn(&Path) -> Result<(f64, f64), crate::sky_solution::SolveFailure> + Sync + 'a;

/// What the reading has found so far ("Found so far"), counted while the
/// headers are read, before any grouping: an estimate the review then makes
/// exact. A night is the evening of a light at the longitude its header gives
/// (mean solar noon), a target its `OBJECT` or its position to a degree, a
/// place its site to a tenth of a degree, an instrument its camera and
/// telescope, a calibration set its type, exposure and gain; `to_solve` counts
/// the folders of lights without `OBJECT` nor coordinates (one frame each will
/// be plate-solved).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundSoFar {
    pub nights: usize,
    pub targets: usize,
    pub places: usize,
    pub instruments: usize,
    pub lights: usize,
    pub calibration_files: usize,
    pub calibration_sets: usize,
    /// `dark`, `flat`, `dark-flat`, `bias`, in that order, those met.
    pub calibration_kinds: Vec<String>,
    pub log_files: usize,
    /// `nina`, `asiair`, `phd2`, in that order, those met.
    pub log_kinds: Vec<String>,
    pub to_solve: usize,
}

/// Where the reading reports [`FoundSoFar`] (every few files, and at the end).
pub type FoundSink<'a> = dyn Fn(&FoundSoFar) + Sync + 'a;

/// What a run may use beside its input.
#[derive(Default, Clone, Copy)]
pub struct DetectOptions<'a> {
    pub cancel: Option<&'a CancelFlag>,
    pub progress: Option<ProgressSink<'a>>,
    /// A solver that already exists here (`crate::sky_solution`), asked for
    /// **one** frame of a group without a target: the centre `(ra, dec)` in
    /// degrees. `None`: the group is only marked "plate solving to do".
    pub solver: Option<&'a FrameSolver<'a>>,
    /// "Found so far": the counts while the headers are read.
    pub found: Option<&'a FoundSink<'a>>,
}

#[derive(Debug, Error)]
pub enum DetectError {
    #[error("the folder of FITS is required")]
    NoFitsFolder,
    #[error(transparent)]
    Scan(#[from] ScanError),
    #[error("the detection was stopped")]
    Cancelled,
    #[error("the session's range cannot be read: {0}")]
    BadScope(String),
}

impl DetectError {
    /// A stable code for the window.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            DetectError::NoFitsFolder => "no_fits_folder",
            DetectError::Scan(ScanError::NotADirectory(_)) => "folder_missing",
            DetectError::Scan(_) => "scan_failed",
            DetectError::Cancelled => "cancelled",
            DetectError::BadScope(_) => "bad_scope",
        }
    }
}

/* -------------------------------- Output --------------------------------- */

/// What the engine found and what it proposes: nothing is written.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    pub reading: ReadingSummary,
    pub places: Vec<PlaceProposal>,
    pub equipment: Vec<EquipmentProposal>,
    pub sessions: Vec<DetectedSession>,
    pub multi_night: Vec<MultiNightGroup>,
    pub calibration_sets: Vec<CalibrationSet>,
    pub logs: Vec<DetectedLogFile>,
    pub set_aside: SetAside,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scoped: Option<ScopedDetection>,
    /// One line per element, in the order of the review screen.
    pub lines: Vec<ProposalLine>,
    /// Remarks above the nights (the account's rights).
    pub notes: Vec<ProposalNote>,
    /// The rights the proposal was computed for. A proposal kept (a file, the
    /// review's state) outlives the offer: when the account's rights have
    /// changed since, [`Proposal::refresh_rights`] follows them.
    #[serde(default)]
    pub account: AccountRights,
}

/// A line about the files the detection does not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadNote {
    /// A warning (nothing readable in the folder) or an information.
    pub warning: bool,
    /// The words of the command line, in French.
    pub text: String,
}

impl Proposal {
    /// Follows the rights the account holds now: the multi-night plans open
    /// (ticked by default) or close, the lines and the remarks made again,
    /// without reading the folders. [`RightsRefresh::Stale`] when the new
    /// rights need the person's plans read again (a new detection).
    pub fn refresh_rights(&mut self, account: AccountRights) -> RightsRefresh {
        propose::refresh_rights(self, account)
    }

    /// Whether the folders hold no readable pose while they hold RAW files
    /// or videos: the person believes everything was read, and nothing will
    /// come out of it (a real empty folder says nothing more than before).
    #[must_use]
    pub fn nothing_readable(&self) -> bool {
        self.reading.lights == 0
            && self.sessions.is_empty()
            && (!self.set_aside.raw_files.is_empty() || !self.set_aside.videos.is_empty())
    }

    /// The command line's lines about RAW files and videos: the warning when
    /// nothing is readable, else one information per kind.
    #[must_use]
    pub fn unread_note_sentences(&self) -> Vec<UnreadNote> {
        let aside = &self.set_aside;
        let (raw, videos, captures) = (
            aside.raw_files.len(),
            aside.videos.len(),
            aside.video_captures.len(),
        );
        let raw_words = |n: usize| format!("{n} RAW file{}", if n > 1 { "s" } else { "" });
        let video_words = |n: usize| format!("{n} video{}", if n > 1 { "s" } else { "" });
        if self.nothing_readable() {
            // The videos are « not processed », the RAW files « not read »:
            // no way out is promised for either.
            let clauses: Vec<String> = [
                (raw > 0).then(|| {
                    format!(
                        "{} {} not read here",
                        raw_words(raw),
                        if raw > 1 { "are" } else { "is" }
                    )
                }),
                (videos > 0).then(|| {
                    format!(
                        "{} {} not processed here",
                        video_words(videos),
                        if videos > 1 { "are" } else { "is" }
                    )
                }),
            ]
            .into_iter()
            .flatten()
            .collect();
            return vec![UnreadNote {
                warning: true,
                text: format!(
                    "No readable frame in this folder. Astro Sync reads FITS and XISF frames: {}.",
                    clauses.join(", ")
                ),
            }];
        }
        let mut notes = Vec::new();
        if videos > 0 {
            notes.push(UnreadNote {
                warning: false,
                text: format!(
                    "{} not processed{}",
                    video_words(videos),
                    match captures {
                        0 => String::new(),
                        1 => ", with its capture file".to_string(),
                        n => format!(", with their {n} capture files"),
                    }
                ),
            });
        }
        if raw > 0 {
            notes.push(UnreadNote {
                warning: false,
                text: format!("{} not read", raw_words(raw)),
            });
        }
        notes
    }
}

/// How much was read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadingSummary {
    /// Frames and logs read (one per physical file).
    pub files_read: usize,
    pub lights: usize,
    pub calibration_frames: usize,
    pub log_files: usize,
    pub nights: usize,
    pub targets: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<LinkIssue>,
}

/// A place of the frames and its match in "My places".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaceProposal {
    pub id: String,
    /// From `SITELAT`/`SITELONG`; `None`: the frames name no place.
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    /// The known places within [`PLACE_RADIUS_KM`], the nearest first.
    pub candidates: Vec<PlaceCandidate>,
    /// The place ticked by default: the nearest candidate.
    pub chosen: Option<String>,
    pub action: PlaceAction,
    pub sessions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaceCandidate {
    pub place_id: String,
    pub name: String,
    pub distance_km: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlaceAction {
    /// One known place.
    #[default]
    Use,
    /// Two known places or more: the nearest ticked, the person may change.
    UseNearest,
    /// None within reach: "Choose a place" / "Create this place".
    Create,
    /// The frames name no place: the person chooses before creating.
    Choose,
}

/// The equipment the headers name and its match in "My equipment".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EquipmentProposal {
    pub id: String,
    pub telescop: Option<String>,
    pub instrume: Option<String>,
    pub focal_length_mm: Option<f64>,
    pub camera: Option<EquipmentMatch>,
    pub telescope: Option<EquipmentMatch>,
    pub mount: Option<EquipmentMatch>,
    /// The `FILTER` names the frames of this equipment carry, and the
    /// filters of "My equipment" they name: they are part of the equipment
    /// the night is created with (`equipment.filterIds`).
    #[serde(default)]
    pub filters: Vec<String>,
    #[serde(default)]
    pub filter_matches: Vec<EquipmentMatch>,
    pub sessions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EquipmentMatch {
    pub equipment_id: String,
    pub name: String,
    /// What matched: `name`, `focal-length`.
    pub by: String,
}

/// A session found: one night at one place.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedSession {
    pub id: String,
    /// `YYYY-MM-DD`, after the clock correction when one is proposed.
    pub evening_date: String,
    /// The evening the written dates give (differs only after a shift).
    pub written_evening_date: String,
    pub place: String,
    pub equipment: Vec<String>,
    /// First start and last end of a light, UTC, shift applied.
    pub start: String,
    pub end: String,
    pub targets: Vec<DetectedTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock: Option<ClockFinding>,
    /// The whole hours added to the written `DATE-OBS` (0: none).
    pub shift_hours: i32,
    pub calibration: Vec<AttachedCalibration>,
    /// What the night's poses lack in calibration:
    /// darks of their exposure, flats of their filter. Empty when nothing is
    /// missing.
    #[serde(default)]
    pub calibration_notes: Vec<CalibrationNote>,
    pub logs: Vec<AttachedLog>,
    pub existing: SessionMatch,
    /// Some target of the night has no `FILTER` in its frames and the
    /// session that exists declares none for it: nothing is supposed, the
    /// review asks the band of the night (one answer per night).
    #[serde(default)]
    pub filter_ask: bool,
    /// The band the names of the night say (folders, file names, a
    /// `SESSION.md` heading) when `filter_ask` holds: the review preselects
    /// it and says where it comes from; the person can change it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_guess: Option<band::BandGuess>,
}

/// A target of a session, and the frames of it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedTarget {
    /// Stable within the proposal: the catalogue id, `object:…`, `folder:…`
    /// or `unsolved:…`.
    pub key: String,
    pub target_id: Option<String>,
    pub designation: String,
    pub name: Option<String>,
    pub source: TargetSource,
    /// The `OBJECT` values written in the frames.
    pub objects: Vec<String>,
    /// Shift applied.
    pub start: String,
    pub end: String,
    pub frames: usize,
    pub exposure_seconds: f64,
    pub filters: Vec<String>,
    /// The filter the names around the lights say (`Dualband`, `UV/IR cut`,
    /// `RGB`), for lights whose headers name none: what a night created
    /// without asking the person records as the target's filter. Absent when
    /// the headers name a filter or nothing names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_hint: Option<String>,
    pub acquisition: Vec<AcquisitionRow>,
    pub instrument: String,
    /// The frame to plate-solve, when nothing else names the target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solve_frame: Option<String>,
    /// Why the solver, which exists here, gave nothing on that frame: the
    /// group stays "to solve", and the person is told why instead of
    /// nothing. Absent when no solver was asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solve_failure: Option<crate::sky_solution::SolveFailure>,
    /// The centre `(ra, dec)`, in degrees, the solver found on that frame
    /// when it matched no target of the catalogue: the position is shown
    /// instead of a target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solved_position: Option<(f64, f64)>,
    /// The images a known device stacked for this target and this night (the
    /// Seestar's `Stacked_*.fit`), absolute paths: not
    /// poses, offered as the master of the target. "Tidy the files"
    /// puts them in `<target>/device/`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub device_images: Vec<String>,
    /// Absolute paths, in the order taken.
    pub files: Vec<String>,
    /// The start of each file (same order), UTC, shift applied: the « UTC »
    /// column of the detection sheet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_times: Vec<String>,
    /// The `FILTER` of each file (same order), when any frame writes one:
    /// the filter level of the organised tree.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_filters: Vec<String>,
    pub existing: TargetMatch,
}

/// How the target was found.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "by")]
pub enum TargetSource {
    /// `OBJECT` names a target of the catalogue.
    Object,
    /// The coordinates fall on a target of the catalogue.
    Coordinates { distance_degrees: f64 },
    /// The folder's name names a target of the catalogue.
    Folder { folder: String },
    /// Solved on one frame by a solver of this computer.
    PlateSolved {
        frame: String,
        distance_degrees: f64,
    },
    /// `OBJECT` names no target of the catalogue: a new target to create.
    UnknownObject,
    /// Nothing names it: "plate solving to do" on one frame.
    #[default]
    ToSolve,
}

/// One line of the rebuilt plan: a filter, an exposure, a gain.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcquisitionRow {
    pub filter: Option<String>,
    pub exposure_seconds: Option<f64>,
    pub gain: Option<f64>,
    pub frames: usize,
    pub total_seconds: f64,
}

/// What a detected session meets among the person's sessions and plans.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum SessionMatch {
    /// Nothing that night: a session "Executed" and its one-night plan.
    #[default]
    Create,
    /// A session exists that night: the frames go to it.
    #[serde(rename_all = "camelCase")]
    Existing { session_id: String },
    /// A plan planned that night, never executed, with a target in common:
    /// reused, set to executed.
    #[serde(rename_all = "camelCase")]
    ReusePlan { plan_id: String, plan_name: String },
    /// A plan planned that night for another target: replaced by what is
    /// detected (ticked by default).
    #[serde(rename_all = "camelCase")]
    ReplacePlan { plan_id: String, plan_name: String },
    /// "One plan per night": a plan already done that night, none created.
    #[serde(rename_all = "camelCase")]
    PlanExists { plan_id: String, plan_name: String },
}

/// What a detected target meets in an existing session.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetMatch {
    /// Part of what will be created.
    #[default]
    New,
    /// Already in the existing session: the frames are attached to it.
    InSession,
    /// Not in the session, its slot free: added through the rework route.
    AddToSession,
    /// Not in the session, its slot overlaps another target's: asked.
    Overlap,
}

/// Calibration frames of one kind taken together.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalibrationSet {
    pub id: String,
    pub kind: Option<FrameType>,
    /// The type was decided by the folder's name, not the header.
    pub kind_from_folder: bool,
    pub folder: String,
    pub frames: usize,
    pub exposure_seconds: Option<f64>,
    pub gain: Option<f64>,
    pub temperature_c: Option<f64>,
    pub filter: Option<String>,
    pub instrument: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub files: Vec<String>,
}

/// A calibration set attached to a session, and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachedCalibration {
    pub set_id: String,
    pub kind: FrameType,
    /// Days between the set and the night's lights.
    pub days_away: f64,
    /// `exposure`, `gain`, `temperature`, `filter`, `date`: what agreed.
    pub matched_on: Vec<String>,
}

/// What a night's poses lack in calibration, said where the person reads the
/// night. A **warning** when calibration of the same
/// kind exists and does not fit the poses (the result will be worse, and the
/// person can act); an **information** when there is none at all
/// (that is sometimes a choice, and it is still said).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "code",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CalibrationNote {
    /// Darks exist and none has the exposure of the poses: "No 32 s dark
    /// for these frames (10 darks of 30 s not attached)".
    NoDark {
        exposure_seconds: f64,
        /// The darks found, whatever their exposure.
        count: usize,
        /// Their distinct exposures, ascending.
        found_seconds: Vec<f64>,
    },
    /// No dark at all, for poses of this exposure.
    NoDarkAtAll { exposure_seconds: f64 },
    /// Flats exist and none is for the poses' filter (`None`: the poses name
    /// none): "No flat for the V filter".
    NoFlat { filter: Option<String> },
    /// No flat at all, for poses of this filter.
    NoFlatAtAll { filter: Option<String> },
}

impl CalibrationNote {
    /// Whether the note is a warning (else an information).
    #[must_use]
    pub fn is_warning(&self) -> bool {
        matches!(
            self,
            CalibrationNote::NoDark { .. } | CalibrationNote::NoFlat { .. }
        )
    }

    /// The words of the command line (English; the window's dictionaries
    /// carry the ten languages).
    #[must_use]
    pub fn sentence(&self) -> String {
        let number = crate::detection::plain_number;
        match self {
            CalibrationNote::NoDark {
                exposure_seconds,
                count,
                found_seconds,
            } => format!(
                "No dark of {} s for these frames ({count} dark{} of {} s not attached).",
                number(*exposure_seconds),
                if *count > 1 { "s" } else { "" },
                found_seconds
                    .iter()
                    .map(|value| number(*value))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            CalibrationNote::NoDarkAtAll { exposure_seconds } => format!(
                "No dark at all for these {} s frames.",
                number(*exposure_seconds)
            ),
            CalibrationNote::NoFlat {
                filter: Some(filter),
            } => {
                format!("No flat for the {filter} filter.")
            }
            CalibrationNote::NoFlat { filter: None } => "No flat for these frames.".to_string(),
            CalibrationNote::NoFlatAtAll {
                filter: Some(filter),
            } => {
                format!("No flat for the {filter} filter.")
            }
            CalibrationNote::NoFlatAtAll { filter: None } => {
                "No flat for these frames.".to_string()
            }
        }
    }
}

/// A log read.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedLogFile {
    pub id: String,
    pub path: String,
    /// `phd2`, `asiair`, `nina`.
    pub kind: String,
    /// The clock of the log, found against the frames (`+02:00`).
    pub utc_offset: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub matched_frames: usize,
    pub sessions: Vec<String>,
}

/// A log attached to a session, and to which of its targets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachedLog {
    pub log_id: String,
    pub targets: Vec<String>,
    pub matched_frames: usize,
}

/// What was read and left out, listable.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetAside {
    /// Lights without a readable `DATE-OBS`.
    pub without_date: Vec<String>,
    /// Frames whose header cannot be read.
    pub unreadable: Vec<String>,
    /// Stacked images (Live stacks, masters): not frames of a night. The
    /// image of a known device that a target takes is not here: it is the
    /// target's [`DetectedTarget::device_images`].
    pub stacks: Vec<String>,
    /// Calibration frames of an unknown type, or attached to no night.
    pub calibration_unattached: Vec<String>,
    /// Logs tied to no night.
    pub logs_unattached: Vec<String>,
    /// ASIAIR's `_CHN` renditions and text files no reader knows.
    pub other_files: Vec<String>,
    /// Videos (AVI, SER…): not processed, not read.
    #[serde(default)]
    pub videos: Vec<String>,
    /// The text files a capture program wrote beside those videos (ZWO's
    /// `<video>.avi.txt`): they follow their video, and are not « other
    /// files ».
    #[serde(default)]
    pub video_captures: Vec<String>,
    /// Camera RAW files (CR2, NEF, ARW, DNG, RAF, PEF, CR3…): not read.
    #[serde(default)]
    pub raw_files: Vec<String>,
}

/// The detection of the sending of one session.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopedDetection {
    pub targets: Vec<ScopedTarget>,
    /// Frames of the range that match no target of the session ("Unassigned"), with what they seem to be.
    pub unassigned: Vec<UnassignedGroup>,
    /// Frames of the range without `OBJECT` nor coordinates.
    pub without_object: Vec<String>,
    /// Lights of the folders taken outside the session's range.
    pub other_nights: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopedTarget {
    pub id: String,
    pub designation: String,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnassignedGroup {
    pub key: String,
    pub designation: String,
    pub target_id: Option<String>,
    pub files: Vec<String>,
}

/// A multi-night plan proposed: one target, one instrument, nights at most
/// [`MULTI_NIGHT_GAP_DAYS`] apart.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiNightGroup {
    pub id: String,
    pub target_key: String,
    pub target_id: Option<String>,
    pub designation: String,
    pub instrument: String,
    pub nights: Vec<MultiNightNight>,
    /// The goal = the total acquired (editable afterwards in the app).
    pub goal_seconds: f64,
    pub frames: usize,
    /// The plan is born "Completed".
    pub status: String,
    /// Without "Advanced Planning": shown, locked.
    pub locked: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiNightNight {
    pub session_id: String,
    pub evening_date: String,
    pub frames: usize,
    pub seconds: f64,
}

/// One line of the review screen.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalLine {
    pub id: String,
    pub kind: LineKind,
    /// The element the line is about (`place-1`, `s-2026-08-17-1`, …).
    pub subject: String,
    pub checked: bool,
    /// Shown greyed with a padlock (the multi-night group of a free account).
    pub locked: bool,
    /// A question the person answers (two equal choices, nothing ticked).
    pub ask: bool,
    /// A stable key (`session.create`, `plan.replace`, …).
    pub reason: String,
    /// The line, in French, as the window shows it.
    pub text: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LineKind {
    Place,
    Equipment,
    #[default]
    Session,
    Target,
    MultiNight,
    Calibration,
    Log,
    Clock,
}

/// A remark above the nights.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalNote {
    pub key: String,
    pub text: String,
}

/// The three steps, for a caller that has the person's data at hand.
pub fn detect_sessions(
    request: &DetectRequest,
    catalogue: &[CatalogueTarget],
    user: &UserData,
    options: &DetectOptions<'_>,
) -> Result<Proposal, DetectError> {
    let reading = read_locations(request, options)?;
    let grouping = group_frames(&reading, request, catalogue, user, options)?;
    Ok(propose(&grouping, user))
}

/// Step 2: the frames read, grouped (see the module's documentation).
pub fn group_frames(
    reading: &Reading,
    request: &DetectRequest,
    catalogue: &[CatalogueTarget],
    user: &UserData,
    options: &DetectOptions<'_>,
) -> Result<Grouping, DetectError> {
    group::group(reading, request, catalogue, user, options)
}

/// Great-circle distance between two places, kilometres.
#[must_use]
pub fn distance_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dp = p2 - p1;
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * 6_371.0 * a.sqrt().min(1.0).asin()
}

#[cfg(test)]
mod tests;

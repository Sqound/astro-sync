//! The resume of a night:
//! the night's page **is the wizard resumed** at the step where something is
//! missing. [`resume_night`] reads
//!
//! - **online** -- `GET /v1/sessions/{id}/online` ([`crate::night_online`]):
//!   the night's targets, their captures and images (digest, sky solution),
//!   the analysis in place;
//! - **here** -- the night's folder found by its identity file, what its
//!   detection saw (targets, frames, logs), the digests of its final and
//!   stacked images, the upload queue's rows, and the record of the last
//!   sending from this computer ([`NightLedger`]);
//!
//! and gives the night's state ("Not started", "To complete" with what,
//! "Complete"), the step to reopen, and the complements: images online are
//! kept and never replaced, identical ones are not sent again
//! ([`crate::precheck::plan_images`] on [`ResumeLocal::candidates`]); a
//! missing sky solution is completed from the local file of the same digest,
//! else from the original fetched by `GET /v1/captures/{id}/original`
//! ([`fetch_original`], captures from the phone and the web included); the
//! analysis is rebuilt when logs or frames were added since it was sent
//! ("The night's analysis will be recomputed and will replace the one of 5 Oct.");
//! the progression is redone when a target's frame count changed.
//!
//! **A target of a multi-night plan** ([`PlanOnline`]): the image of the plan
//! is one image for the whole plan, a capture of the plan that belongs to no
//! night, so `…/online` of a night never lists it. A target whose plan has
//! its image online owes no image of its own for the night
//! ([`resume_night_with_plans`]). When that image leaves the night out (the
//! nights it combined do not name it, or, naming none, it was made before
//! the night began), the night is complete all the same and says so
//! ([`NightResume::plans_left_out`]).
//!
//! Nothing here writes in the person's tree: the ledger and the originals
//! live in Astro Sync's own data.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::gui_bridge::{GuiBridgeError, HttpPortalGuiTransport};
use crate::multi_night::NightMultiNight;
use crate::night_online::{NightOnline, OnlineCapture, OnlineImage};
use crate::precheck::ImageCandidate;
use crate::scanner::{self, ScanError};
use crate::service_wait::RequestFactory;
use crate::time_util::{parse_iso8601_utc_to_unix_seconds, system_time_to_iso8601_utc};
use crate::upload_queue::{QueueEntry, QueueState, ResultKind};

/* ------------------------------- Here ----------------------------------- */

/// A final or stacked image of the night's folder, with its digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalImage {
    pub target_id: String,
    pub path: PathBuf,
    pub sha256: String,
    pub size: u64,
    pub kind: ResultKind,
    /// The digest of the same render in its other format, for a render the
    /// stacker wrote as PNG and TIFF ([`crate::render_files`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub twin_sha256: Option<String>,
}

impl LocalImage {
    /// The TIFF of a render that also exists as a PNG: it leaves only when
    /// the person joins the TIFF, so it is never « to send » by itself.
    #[must_use]
    pub fn is_full_resolution_twin(&self) -> bool {
        self.twin_sha256.is_some() && crate::render_files::is_tiff(&self.path)
    }

    /// A TIFF the person chooses to send (the full-resolution twin of a PNG,
    /// or the linear file): never « missing », it leaves only with « Joindre
    /// aussi les TIFF ».
    #[must_use]
    pub fn is_optional_tiff(&self) -> bool {
        self.is_full_resolution_twin() || self.kind == ResultKind::Linear
    }
}

/// A target of the night as its folder holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalTarget {
    pub target_id: String,
    /// Its folder exists in the night's folder.
    pub has_folder: bool,
    pub light_frames: u64,
}

/// Where the night's folder is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderState {
    Found,
    /// Two folders carry the night (a copy): the page asks which to follow.
    Duplicated,
    /// Never created, or moved out of the root ("Find the folder again").
    Missing,
}

/// What the night's folder on this computer holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeLocal {
    pub folder: FolderState,
    pub targets: Vec<LocalTarget>,
    pub logs: u64,
    pub images: Vec<LocalImage>,
    /// [`analysis_inputs`] of the folder: what the analysis reads.
    pub analysis_inputs: Option<AnalysisInputs>,
}

impl ResumeLocal {
    /// The images that could leave, for [`crate::precheck::plan_images`].
    #[must_use]
    pub fn candidates(&self) -> Vec<ImageCandidate> {
        self.images
            .iter()
            .map(|image| ImageCandidate {
                target_id: image.target_id.clone(),
                path: image.path.clone(),
                sha256: image.sha256.clone(),
                size: image.size,
                kind: image.kind,
                // What the stacker recorded for its own renders: the natural
                // one is the main image of the capture (`precheck`).
                render: match image.kind {
                    ResultKind::Stacked => crate::metadata::recorded_pipeline(&image.path),
                    ResultKind::Linear => Some(crate::pipelines::LINEAR_PIPELINE.to_string()),
                    _ => None,
                },
                // The series it was stacked from: the main image comes from
                // the longest one (`precheck`).
                series_seconds: (image.kind == ResultKind::Stacked)
                    .then(|| crate::metadata::recorded_series_seconds(&image.path))
                    .flatten(),
                // Its series of poses: one capture per series.
                series: matches!(image.kind, ResultKind::Stacked | ResultKind::Linear)
                    .then(|| crate::metadata::recorded_series(&image.path))
                    .flatten(),
                // Where the stacker put the render among those asked for.
                render_rank: (image.kind == ResultKind::Stacked)
                    .then(|| crate::metadata::recorded_render_rank(&image.path))
                    .flatten(),
                contrast: if image.kind == ResultKind::Stacked {
                    crate::metadata::recorded_contrast(&image.path)
                } else {
                    Default::default()
                },
                retouch: if image.kind == ResultKind::Stacked {
                    crate::metadata::recorded_retouch(&image.path)
                } else {
                    Default::default()
                },
                frame_count: None,
                twin_sha256: image.twin_sha256.clone(),
            })
            .collect()
    }

    fn light_frames(&self) -> u64 {
        self.targets.iter().map(|t| t.light_frames).sum()
    }

    /// What the detection of the night's folder saw
    /// ([`crate::detection::detect_night`]), with the digests of its images
    /// (`images`, hashed by the caller through the digest cache) and what
    /// the analysis reads (`inputs`).
    #[must_use]
    pub fn from_detection(
        folder: FolderState,
        detection: Option<&crate::detection::NightDetection>,
        images: Vec<LocalImage>,
        inputs: Option<AnalysisInputs>,
    ) -> Self {
        Self {
            folder,
            targets: detection
                .map(|d| {
                    d.targets
                        .iter()
                        .map(|t| LocalTarget {
                            target_id: t.target_id.clone(),
                            has_folder: t.folder_name.is_some(),
                            light_frames: t.light_frames as u64,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            logs: detection.map_or(0, |d| d.logs.len() as u64),
            images,
            analysis_inputs: inputs,
        }
    }
}

/// The files the analysis reads -- the logs and the light frames -- as a
/// fingerprint (paths, sizes, dates; no file read) and the latest date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisInputs {
    pub fingerprint: String,
    pub files: u64,
    /// ISO 8601 UTC; `None` without any file.
    pub latest_modified: Option<String>,
}

/// Reads `night_folder`'s `logs/` and every `<target>/lights/` (links
/// followed), without reading a file.
pub fn analysis_inputs(night_folder: &Path) -> Result<AnalysisInputs, ScanError> {
    let walk = scanner::walk_folder(night_folder, None)?;
    let mut hasher = Sha256::new();
    let mut files = 0;
    let mut latest: Option<i64> = None;
    for file in &walk.files {
        let segments: Vec<&str> = file.relative_path.split('/').collect();
        let read_by_the_analysis =
            segments.first() == Some(&"logs") || (segments.len() >= 3 && segments[1] == "lights");
        if !read_by_the_analysis {
            continue;
        }
        files += 1;
        hasher.update(
            format!(
                "{}\0{}\0{}\n",
                file.relative_path, file.stamp.size, file.stamp.modified_ns
            )
            .as_bytes(),
        );
        latest = latest.max(Some(file.stamp.modified_ns));
    }
    Ok(AnalysisInputs {
        fingerprint: format!("{:x}", hasher.finalize()),
        files,
        latest_modified: latest.map(|ns| {
            system_time_to_iso8601_utc(
                std::time::UNIX_EPOCH + std::time::Duration::from_nanos(ns.max(0) as u64),
            )
        }),
    })
}

/* ----------------------------- The ledger ------------------------------- */

/// What this computer last sent for a night (Astro Sync's data,
/// `nights/<id>.json`): what the analysis read, and the frames of each
/// target the progression was made from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NightLedger {
    #[serde(default)]
    pub analysis_inputs: Option<String>,
    #[serde(default)]
    pub progression_frames: BTreeMap<String, u64>,
}

fn ledger_path(dir: &Path, night_id: &str) -> PathBuf {
    let safe: String = night_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    dir.join(format!("{safe}.json"))
}

/// The ledger of `night_id` in `dir`; none yet is the default.
#[must_use]
pub fn load_ledger(dir: &Path, night_id: &str) -> Option<NightLedger> {
    let text = fs::read_to_string(ledger_path(dir, night_id)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Keeps the ledger of `night_id` (whole or not at all).
pub fn save_ledger(dir: &Path, night_id: &str, ledger: &NightLedger) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let path = ledger_path(dir, night_id);
    let partial = path.with_extension("json.part");
    fs::write(
        &partial,
        serde_json::to_vec_pretty(ledger).map_err(std::io::Error::other)?,
    )?;
    fs::rename(&partial, &path)
}

/* ------------------------------ The state ------------------------------- */

/// The wizard's five steps, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WizardStep {
    Cibles,
    Dossier,
    Fichiers,
    Traitements,
    Recapitulatif,
}

/// "Not started", "To complete", "Complete".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NightStatus {
    NotStarted,
    ToComplete,
    Complete,
}

/// One thing the night still needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "key", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Missing {
    /// The night has no target.
    NoTarget,
    /// The night's folder is not found ("Find the folder again"), or two
    /// carry it.
    Folder { state: FolderState },
    /// A ticked target has no folder yet (created at the next "Folder").
    TargetFolder { target_id: String },
    /// "final image of IC 1396": nothing here nor online.
    FinalImage { target_id: String, name: String },
    /// "logs added to the folder": logs or frames added since the
    /// analysis was sent.
    LogsAdded,
    /// Logs here, no analysis online.
    Analysis,
    /// "sky annotations of 1 image": the main image of a capture online
    /// has no sky solution. `targets` names the targets of those captures.
    SkyAnnotations {
        count: u64,
        #[serde(default)]
        targets: Vec<String>,
    },
    /// A target's frames changed since its progression was made.
    Progression { target_id: String },
    /// Images here not online yet. `targets` names the targets they are
    /// of, one per image owed.
    ImagesToSend {
        count: u64,
        #[serde(default)]
        targets: Vec<String>,
    },
    /// Rows of the night still in the queue (waiting, or set aside).
    QueuePending { count: u64 },
    /// Rows of the night the person asked to send that failed or were
    /// refused.
    QueueFailed { count: u64 },
}

impl Missing {
    /// The step that completes it.
    #[must_use]
    pub fn step(&self) -> WizardStep {
        match self {
            Missing::NoTarget => WizardStep::Cibles,
            Missing::Folder { .. } | Missing::TargetFolder { .. } => WizardStep::Dossier,
            Missing::FinalImage { .. } | Missing::LogsAdded => WizardStep::Fichiers,
            Missing::Analysis | Missing::SkyAnnotations { .. } | Missing::Progression { .. } => {
                WizardStep::Traitements
            }
            Missing::ImagesToSend { .. }
            | Missing::QueuePending { .. }
            | Missing::QueueFailed { .. } => WizardStep::Recapitulatif,
        }
    }

    /// The words of the "Nights" list after "To complete" (English; the
    /// window's dictionaries carry the ten languages).
    #[must_use]
    pub fn sentence(&self) -> String {
        match self {
            Missing::NoTarget => "no target".to_string(),
            Missing::Folder {
                state: FolderState::Duplicated,
            } => "two folders for this night".to_string(),
            Missing::Folder { .. } => "folder not found".to_string(),
            Missing::TargetFolder { .. } => "target folder to create".to_string(),
            Missing::FinalImage { name, .. } => format!("final image of {name}"),
            Missing::LogsAdded => "logs added to the folder".to_string(),
            Missing::Analysis => "analysis of the logs".to_string(),
            Missing::SkyAnnotations { count, .. } => format!(
                "sky annotations of {count} image{}",
                if *count > 1 { "s" } else { "" }
            ),
            Missing::Progression { .. } => "progression to redo".to_string(),
            Missing::ImagesToSend { count, .. } => {
                format!("{count} image{} to send", if *count > 1 { "s" } else { "" })
            }
            Missing::QueuePending { count } => format!(
                "{count} upload{} waiting",
                if *count > 1 { "s" } else { "" }
            ),
            Missing::QueueFailed { count } => format!(
                "{count} upload{} not sent",
                if *count > 1 { "s" } else { "" }
            ),
        }
    }
}

/// What the analysis of the night becomes at the next sending.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AnalysisPlan {
    /// No log and no frame: nothing to analyse.
    Nothing,
    /// No analysis online: the first one is sent.
    Send,
    /// Logs or frames added since it was sent: rebuilt, it replaces the one
    /// of `previous_sent_at`.
    Replace { previous_sent_at: String },
    /// Nothing new: the analysis online stays.
    UpToDate,
}

impl AnalysisPlan {
    /// "The night's analysis will be recomputed and will replace the one of 5
    /// Oct." for a replacement.
    #[must_use]
    pub fn sentence(&self) -> Option<String> {
        match self {
            AnalysisPlan::Replace { previous_sent_at } => {
                let date = short_date(previous_sent_at);
                Some(format!(
                    "The night's analysis will be recomputed and will replace the one of {date}."
                ))
            }
            _ => None,
        }
    }
}

/// `2026-10-05T…` → "5 Oct" (the date of the instant, UTC).
fn short_date(iso: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut parts = iso.get(..10).unwrap_or_default().split('-');
    let (_, month, day) = (parts.next(), parts.next(), parts.next());
    match (
        month.and_then(|m| m.parse::<usize>().ok()),
        day.and_then(|d| d.parse::<u32>().ok()),
    ) {
        (Some(m @ 1..=12), Some(d)) => format!("{d} {}", MONTHS[m - 1]),
        _ => iso.to_string(),
    }
}

/// Where a missing sky solution is computed from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "from",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SolveFrom {
    /// The local file of the same digest (no download).
    Local {
        capture_id: String,
        image_id: String,
        sha256: String,
        path: PathBuf,
    },
    /// The original, fetched from the space (an image from the phone or the
    /// web, or one whose file is no longer here).
    Original {
        capture_id: String,
        image_id: String,
        sha256: String,
        original_name: String,
        byte_size: u64,
    },
}

/// The night resumed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NightResume {
    pub status: NightStatus,
    /// The step the wizard reopens at; `None` for a complete night, but
    /// « Traitements » when an image of its plan leaves it out.
    pub step: Option<WizardStep>,
    pub missing: Vec<Missing>,
    pub analysis: AnalysisPlan,
    pub sky_solutions: Vec<SolveFrom>,
    /// The multi-night plans whose image online stands for a target of the
    /// night and does not hold the night ([`PlanOnline::night_left_out`]).
    /// Nothing is owed for the night: a complete night says it, and its
    /// wizard reopens at « Traitements », where the image of the plan is
    /// made again.
    #[serde(default)]
    pub plans_left_out: Vec<PlanOnline>,
}

impl NightResume {
    /// "To complete: final image of IC 1396 · sky annotations of 1
    /// image" (for the command line).
    #[must_use]
    pub fn sentence(&self) -> String {
        match self.status {
            NightStatus::NotStarted => "Not started".to_string(),
            NightStatus::Complete if !self.plans_left_out.is_empty() => format!(
                "Complete: the image of the plan of {} {}hold this night",
                self.plans_left_out
                    .iter()
                    .map(|plan| plan.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                if self
                    .plans_left_out
                    .iter()
                    .any(|plan| plan.night_left_out == Some(NightLeftOut::NotCombined))
                {
                    "does not "
                } else {
                    "may not "
                }
            ),
            NightStatus::Complete => "Complete".to_string(),
            NightStatus::ToComplete => format!(
                "To complete: {}",
                self.missing
                    .iter()
                    .map(Missing::sentence)
                    .collect::<Vec<_>>()
                    .join(" · ")
            ),
        }
    }
}

fn later_than(a: &str, b: &str) -> bool {
    match (
        parse_iso8601_utc_to_unix_seconds(a),
        parse_iso8601_utc_to_unix_seconds(b),
    ) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// The multi-night plan a target of the night works for, and whether the
/// plan has its image online. That image is a capture of the plan: it
/// belongs to no night and no night's `…/online` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanOnline {
    pub project_id: String,
    /// The night's target that works for the plan.
    pub target_id: String,
    /// The plan's target as the plan names it.
    pub name: String,
    /// The plan has an image online (`planCapture`).
    pub image_online: bool,
    /// The image online does not hold this night, or may not: `None` when
    /// it holds it, or when nothing tells.
    #[serde(default)]
    pub night_left_out: Option<NightLeftOut>,
}

/// Why the image of a plan online leaves the night out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NightLeftOut {
    /// The nights the image combined (`planCapture.combinedNightIds`, what
    /// Astro Sync sends with it) do not name this night: the image does not
    /// hold it.
    NotCombined,
    /// The image names no night, and was made before the night began
    /// (`planCapture.createdAt`): it may not hold it.
    MadeBefore,
}

/// Whether the image of a plan online (`planCapture`) leaves the night
/// `session_id` out. The nights it combined say it when the image names
/// them; else an image made before the night began may not hold it.
#[must_use]
pub fn night_left_out(
    plan_capture: &serde_json::Value,
    session_id: &str,
    night_started_at: Option<&str>,
) -> Option<NightLeftOut> {
    let combined: Vec<&str> = plan_capture
        .get("combinedNightIds")
        .and_then(serde_json::Value::as_array)
        .map(|ids| ids.iter().filter_map(serde_json::Value::as_str).collect())
        .unwrap_or_default();
    if !combined.is_empty() {
        let held = combined
            .iter()
            .any(|id| id.eq_ignore_ascii_case(session_id));
        return (!held).then_some(NightLeftOut::NotCombined);
    }
    let made = plan_capture
        .get("createdAt")
        .and_then(serde_json::Value::as_str)?;
    later_than(night_started_at?, made).then_some(NightLeftOut::MadeBefore)
}

/// The plans of a night ([`crate::multi_night::night_projects`]) as the
/// resume reads them, for the night `session_id` that began at
/// `night_started_at` (ISO 8601 UTC, when known).
#[must_use]
pub fn plans_online(
    night: &NightMultiNight,
    session_id: &str,
    night_started_at: Option<&str>,
) -> Vec<PlanOnline> {
    night
        .projects
        .iter()
        .map(|project| PlanOnline {
            project_id: project.project_id.clone(),
            target_id: project.target_id.clone(),
            name: project.name.clone(),
            image_online: project.plan_capture.is_some(),
            night_left_out: project
                .plan_capture
                .as_ref()
                .and_then(|capture| night_left_out(capture, session_id, night_started_at)),
        })
        .collect()
}

/// [`resume_night_with_plans`] for a night whose targets work for no
/// multi-night plan.
#[must_use]
pub fn resume_night(
    online: &NightOnline,
    local: &ResumeLocal,
    queue: &[QueueEntry],
    ledger: Option<&NightLedger>,
    variant_limit: u64,
) -> NightResume {
    resume_night_with_plans(online, local, queue, ledger, variant_limit, &[])
}

/// The image of a capture the list shows and the annotations are owed for:
/// its main image.
fn shown_image(capture: &OnlineCapture) -> Option<&OnlineImage> {
    capture
        .images
        .iter()
        .find(|image| image.main)
        .or(capture.main_image.as_ref())
        .or(capture.images.first())
}

/// The night's state, its step and its complements (see the module
/// documentation). `queue` is every row of the upload queue (the night's are
/// those attributed to `online.session_id`); `variant_limit` is the
/// space's `capture.variant_limit`. Only what the person asked for keeps a
/// night "To complete": an image set here and not online, a queued, failed
/// or refused row, or a target with no image online at all. The other
/// renders of a folder are choices, never owed. `plans` are the multi-night
/// plans the night's targets work for: a target whose plan has its image
/// online owes no image for the night, whichever nights the plan combined.
///
/// The sky annotations are owed for the main image of each capture online
/// (the images « Online » counts); the variants that lack theirs are in
/// [`NightResume::sky_solutions`] and solved in the same pass, and never
/// keep a night "To complete" by themselves.
#[must_use]
pub fn resume_night_with_plans(
    online: &NightOnline,
    local: &ResumeLocal,
    queue: &[QueueEntry],
    ledger: Option<&NightLedger>,
    variant_limit: u64,
    plans: &[PlanOnline],
) -> NightResume {
    let mut missing = Vec::new();
    let plan_image_online = |target_id: &str| {
        plans
            .iter()
            .any(|plan| plan.image_online && plan.target_id == target_id)
    };
    let planned: Vec<_> = online
        .targets
        .iter()
        .filter(|t| t.planned && t.target_id.is_some())
        .collect();

    if planned.is_empty() {
        missing.push(Missing::NoTarget);
    }
    if local.folder != FolderState::Found {
        missing.push(Missing::Folder {
            state: local.folder,
        });
    } else {
        for target in &planned {
            let id = target.target_id.clone().unwrap_or_default();
            let has_folder = local
                .targets
                .iter()
                .any(|t| t.target_id == id && t.has_folder);
            if !has_folder {
                missing.push(Missing::TargetFolder { target_id: id });
            }
        }
    }

    // A planned target with nothing online and nothing here.
    for target in &planned {
        let id = target.target_id.as_deref().unwrap_or_default();
        let online_image = target.captures.iter().any(|c| c.main_image.is_some());
        let here = local.images.iter().any(|i| i.target_id == id);
        if !online_image && !here && !plan_image_online(id) {
            missing.push(Missing::FinalImage {
                target_id: id.to_string(),
                name: target.name.clone().unwrap_or_else(|| id.to_string()),
            });
        }
    }

    // The analysis.
    let inputs = local.analysis_inputs.as_ref();
    let has_material = local.logs > 0 || local.light_frames() > 0;
    let analysis = if !has_material {
        AnalysisPlan::Nothing
    } else if !online.analysis.present {
        AnalysisPlan::Send
    } else {
        let sent_at = online.analysis.sent_at.clone().unwrap_or_default();
        let changed = match (ledger.and_then(|l| l.analysis_inputs.as_deref()), inputs) {
            (Some(sent), Some(now)) => sent != now.fingerprint,
            // Sent from another computer: a file dated after the sending.
            (None, Some(now)) => now
                .latest_modified
                .as_deref()
                .is_some_and(|latest| later_than(latest, &sent_at)),
            _ => false,
        };
        if changed {
            AnalysisPlan::Replace {
                previous_sent_at: sent_at,
            }
        } else {
            AnalysisPlan::UpToDate
        }
    };
    match &analysis {
        AnalysisPlan::Replace { .. } => missing.push(Missing::LogsAdded),
        AnalysisPlan::Send if local.logs > 0 => missing.push(Missing::Analysis),
        _ => {}
    }

    // Sky solutions missing, from the local file of the same digest or the
    // original.
    let mut sky_solutions = Vec::new();
    for (capture, image) in online.images() {
        if image.has_sky_solution() {
            continue;
        }
        sky_solutions.push(solve_from(capture.id.as_str(), image, local));
    }
    // Owed: the main image of each capture (a capture listed under several
    // targets counts once).
    let mut unsolved: Vec<&OnlineCapture> = Vec::new();
    for capture in online.targets.iter().flat_map(|t| t.captures.iter()) {
        if unsolved.iter().any(|seen| seen.id == capture.id) {
            continue;
        }
        if shown_image(capture).is_some_and(|image| !image.has_sky_solution()) {
            unsolved.push(capture);
        }
    }
    if !unsolved.is_empty() {
        let mut targets: Vec<String> = Vec::new();
        for id in unsolved.iter().filter_map(|c| c.target_ids.first()) {
            if !targets.contains(id) {
                targets.push(id.clone());
            }
        }
        missing.push(Missing::SkyAnnotations {
            count: unsolved.len() as u64,
            targets,
        });
    }

    // Progression: a target whose frames changed since it was made.
    if let Some(ledger) = ledger {
        for target in &local.targets {
            if let Some(&made_from) = ledger.progression_frames.get(&target.target_id)
                && made_from != target.light_frames
            {
                missing.push(Missing::Progression {
                    target_id: target.target_id.clone(),
                });
            }
        }
    }

    // Images here that would leave (identical ones never do; a different
    // one only as a variant, when the space keeps variants). The TIFF of a
    // render that leaves as PNG is a choice of the person, never missing;
    // a render online in its other format is online.
    // A render set in « Régler l'image » leaves as its image set: it is to
    // send while the image set is not online, even when the render is (the
    // image set then takes the place of the main image online; a render
    // online as a variant cannot be replaced from here, `precheck`).
    let replacing = |image: &LocalImage| {
        crate::image_adjust::sent_file(&image.path).map(|set| {
            let original = crate::image_adjust::original_sha256(&image.path);
            let online_as = original.as_deref().and_then(|o| online.image_with(o));
            (
                set.sha256,
                set.tiff.map(|tiff| tiff.sha256),
                online_as.map(|(_, online_image)| online_image.main),
            )
        })
    };
    let in_queue = |sha: &str| queue.iter().any(|row| row.sha256.eq_ignore_ascii_case(sha));
    // An image set in « Régler l'image » is something the person asked for.
    let replacements: Vec<&str> = local
        .images
        .iter()
        .filter(|image| !image.is_optional_tiff())
        .filter_map(|image| replacing(image).map(|r| (&image.target_id, r)))
        .filter(|(target_id, (set, set_tiff, original_online))| {
            online.image_with(set).is_none()
                && set_tiff
                    .as_deref()
                    .is_none_or(|tiff| online.image_with(tiff).is_none())
                && *original_online != Some(false)
                && (*original_online == Some(true)
                    || variant_limit > 0
                    || online.capture_of(target_id).is_none())
                && !in_queue(set)
        })
        .map(|(target_id, _)| target_id.as_str())
        .collect();
    // Every other render here is a choice (the stacker writes several per
    // integration, and the person sends the ones they ticked, which then
    // sit in the queue or online). What is owed is one image for a target
    // that has none online, nothing in the queue and no multi-night plan
    // whose image is online.
    let mut bare_targets: Vec<&str> = Vec::new();
    for image in local
        .images
        .iter()
        .filter(|image| !image.is_optional_tiff() && replacing(image).is_none())
    {
        let id = image.target_id.as_str();
        if bare_targets.contains(&id) {
            continue;
        }
        let has_online = online.target(id).is_some_and(|target| {
            target
                .captures
                .iter()
                .any(|c| c.main_image.is_some() || !c.images.is_empty())
        });
        let touched = local.images.iter().filter(|i| i.target_id == id).any(|i| {
            in_queue(&i.sha256)
                || online.image_with(&i.sha256).is_some()
                || i.twin_sha256
                    .as_deref()
                    .is_some_and(|twin| in_queue(twin) || online.image_with(twin).is_some())
        });
        // The image of its multi-night plan is the target's image online.
        if !has_online && !touched && !plan_image_online(id) {
            bare_targets.push(id);
        }
    }
    let owed: Vec<String> = replacements
        .iter()
        .chain(bare_targets.iter())
        .map(|id| (*id).to_string())
        .collect();
    if !owed.is_empty() {
        missing.push(Missing::ImagesToSend {
            count: owed.len() as u64,
            targets: owed,
        });
    }
    let pending = queue
        .iter()
        .filter(|row| row.target.session_id.as_deref() == Some(online.session_id.as_str()))
        .filter(|row| {
            matches!(
                row.state,
                QueueState::Queued | QueueState::Uploading | QueueState::Held
            )
        })
        .count() as u64;
    if pending > 0 {
        missing.push(Missing::QueuePending { count: pending });
    }
    // Rows the person asked to send that did not leave: failed, or refused
    // by the service (unless the same bytes are online meanwhile).
    let failed = queue
        .iter()
        .filter(|row| row.target.session_id.as_deref() == Some(online.session_id.as_str()))
        .filter(|row| matches!(row.state, QueueState::Failed | QueueState::Refused))
        .filter(|row| online.image_with(&row.sha256).is_none())
        .count() as u64;
    if failed > 0 {
        missing.push(Missing::QueueFailed { count: failed });
    }

    let nothing_here = local.images.is_empty() && !has_material;
    let status = if online.is_bare() && nothing_here {
        NightStatus::NotStarted
    } else if missing.is_empty() {
        NightStatus::Complete
    } else {
        NightStatus::ToComplete
    };
    // The images of plans that stand for a target of the night without
    // holding it: owed by no one, said all the same.
    let mut plans_left_out: Vec<PlanOnline> = Vec::new();
    for plan in plans
        .iter()
        .filter(|plan| plan.image_online && plan.night_left_out.is_some())
    {
        if !plans_left_out
            .iter()
            .any(|seen| seen.project_id == plan.project_id)
        {
            plans_left_out.push(plan.clone());
        }
    }
    let step = match status {
        // The image of the plan is made again in « Traitements ».
        NightStatus::Complete if !plans_left_out.is_empty() => Some(WizardStep::Traitements),
        NightStatus::Complete => None,
        NightStatus::NotStarted => Some(
            missing
                .iter()
                .map(Missing::step)
                .min()
                .unwrap_or(WizardStep::Cibles)
                .min(WizardStep::Dossier),
        ),
        NightStatus::ToComplete => missing.iter().map(Missing::step).min(),
    };
    NightResume {
        status,
        step,
        missing,
        analysis,
        sky_solutions,
        plans_left_out,
    }
}

fn solve_from(capture_id: &str, image: &OnlineImage, local: &ResumeLocal) -> SolveFrom {
    match local
        .images
        .iter()
        .find(|l| l.sha256.eq_ignore_ascii_case(&image.sha256))
    {
        Some(here) => SolveFrom::Local {
            capture_id: capture_id.to_string(),
            image_id: image.id.clone(),
            sha256: image.sha256.clone(),
            path: here.path.clone(),
        },
        None => SolveFrom::Original {
            capture_id: capture_id.to_string(),
            image_id: image.id.clone(),
            sha256: image.sha256.clone(),
            original_name: image.original_name.clone(),
            byte_size: image.byte_size,
        },
    }
}

/* ---------------------------- The original ------------------------------ */

/// `GET /v1/captures/{id}/original?imageId=`: the original of an image, to
/// a device of the space; from byte `from` (`Range`) when a part was
/// already fetched. Answers the status (`200` whole, `206` the rest) and
/// the bytes.
pub trait OriginalTransport {
    fn original(
        &self,
        base_url: &str,
        token: &str,
        capture_id: &str,
        image_id: &str,
        from: u64,
    ) -> Result<(u16, Vec<u8>), GuiBridgeError>;
}

impl OriginalTransport for HttpPortalGuiTransport {
    /// A read: retried freely while the service wakes.
    fn original(
        &self,
        base_url: &str,
        token: &str,
        capture_id: &str,
        image_id: &str,
        from: u64,
    ) -> Result<(u16, Vec<u8>), GuiBridgeError> {
        let url = format!(
            "{}/api/v1/captures/{capture_id}/original",
            base_url.trim_end_matches('/')
        );
        let token = token.to_owned();
        let image_id = image_id.to_owned();
        let build: RequestFactory = Arc::new(move |client| {
            let request = client
                .get(url.clone())
                .bearer_auth(&token)
                .query(&[("imageId", image_id.as_str())]);
            if from > 0 {
                request.header("Range", format!("bytes={from}-"))
            } else {
                request
            }
        });
        self.read_bytes(&build)
    }
}

/// Fetches the original of `source` (a [`SolveFrom::Original`]) into `dir`
/// (Astro Sync's data, never the night's folder), resuming a part already
/// fetched, and checks its SHA-256. Returns the file. A local source is
/// returned as it is.
pub fn fetch_original<T: OriginalTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
    source: &SolveFrom,
    dir: &Path,
) -> Result<PathBuf, GuiBridgeError> {
    let (capture_id, image_id, sha256, original_name) = match source {
        SolveFrom::Local { path, .. } => return Ok(path.clone()),
        SolveFrom::Original {
            capture_id,
            image_id,
            sha256,
            original_name,
            ..
        } => (capture_id, image_id, sha256, original_name),
    };
    let io = |error: std::io::Error| GuiBridgeError::Io {
        path: dir.to_path_buf(),
        source: error,
    };
    fs::create_dir_all(dir).map_err(io)?;
    let extension = Path::new(original_name)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .filter(|e| e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or_else(|| "bin".to_string());
    let done = dir.join(format!("{sha256}.{extension}"));
    if done.exists() {
        return Ok(done);
    }
    let part = dir.join(format!("{sha256}.part"));
    let from = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    let (status, bytes) = transport.original(base_url, token, capture_id, image_id, from)?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(status == 206)
        .truncate(status != 206)
        .open(&part)
        .map_err(io)?;
    file.write_all(&bytes).map_err(io)?;
    drop(file);
    let digest = format!("{:x}", Sha256::digest(fs::read(&part).map_err(io)?));
    if !digest.eq_ignore_ascii_case(sha256) {
        let _ = fs::remove_file(&part);
        return Err(GuiBridgeError::Refused {
            code: "original_mismatch".to_string(),
            message: "The original fetched does not have the digest the space announced."
                .to_string(),
        });
    }
    fs::rename(&part, &done).map_err(io)?;
    Ok(done)
}

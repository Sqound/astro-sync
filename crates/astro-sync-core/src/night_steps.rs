//! The steps of a night, put together once for the window and the command
//! line (`README.md` § "CLI / GUI parity": every step of the app
//! is a command of `astro-sync`). Each function here composes the modules of
//! the crate and holds no rule of its own:
//!
//! - [`DataPaths`]: where Astro Sync keeps its own work (never in the night's
//!   folder): the downloaded tools, the originals fetched, the solvers' work,
//!   the progression, the stacking's scratch, the nights' ledgers, the
//!   person's pipelines.
//! - [`local_images`] and [`resume_local`]: the final and stacked images of a
//!   night's folder with their SHA-256 (from the digest cache), and what the
//!   resume reads there.
//! - [`night_material`]: what the night gives each treatment.
//! - [`precheck_night`]: the "Summary"'s check -- each image's
//!   decision ([`crate::precheck::plan_images`]), the service's answer before
//!   any byte, the four limits -- in one answer.
//! - [`solve_night`]: the sky solutions still missing for the night's images
//!   online, solved here (local file of the same digest, else the original)
//!   and sent.
//! - [`record_sending`]: the ledger of the night, written after a sending
//!   that went through.
//! - [`send_night_integrations`]: after the sending of a
//!   night of a multi-night plan, the lines of its masters, one `PUT` per
//!   plan, under the plan's right; a refusal is kept, never an error.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::detection::NightDetection;
use crate::digest_cache::DigestCache;
use crate::gui_bridge::GuiBridgeError;
use crate::local_work::CancelFlag;
use crate::multi_night::{
    Computer, IntegrationsSent, MultiNightGate, MultiNightTransport, NightMultiNight,
    integration_lines, night_sources, send_integrations, target_device_images, target_masters,
};
use crate::night_online::NightOnline;
use crate::night_resume::{
    self, AnalysisInputs, FolderState, LocalImage, NightLedger, OriginalTransport, ResumeLocal,
    analysis_inputs,
};
use crate::precheck::{
    ImageCandidate, ImagePlan, NightMaterial, PrecheckTransport, QuotaCheck, SpaceStorage,
    check_quotas, check_with_service, plan_images_within, rebind_progressions,
    several_renders_allowed,
};
use crate::render_files;
use crate::scanner::{self, ScanError};
use crate::sky_solution::{
    SkySolutionTransport, SolveHints, SolveOutcome, SolveSetup, solve_and_send,
};
use crate::upload_queue::ResultKind;
use crate::working_folder::{FINAL_DIR, STACKED_DIR};

/// Astro Sync's own folders under its data folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataPaths {
    pub data_dir: PathBuf,
    /// `tools/<id>/`: the downloads of "Dependencies".
    pub tools: PathBuf,
    /// The originals fetched from the space to solve them.
    pub originals: PathBuf,
    /// The solvers' work.
    pub solve: PathBuf,
    /// `progression/<night>/<target>/<family>/`.
    pub progression: PathBuf,
    /// `nights/<id>.json`: what this computer last sent for a night.
    pub nights: PathBuf,
    /// The pipelines the person added.
    pub pipelines: PathBuf,
}

impl DataPaths {
    #[must_use]
    pub fn new(data_dir: &Path) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
            tools: data_dir.join("tools"),
            originals: data_dir.join("originals"),
            solve: data_dir.join("solve"),
            progression: data_dir.join("progression"),
            nights: data_dir.join("nights"),
            pipelines: data_dir.join("pipelines"),
        }
    }
}

fn is_final_image(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "tif" | "tiff" | "jpg" | "jpeg" | "png"
            )
        })
}

/// The final images (`<target>/final/`, the proposed one first) and the
/// renders of `<target>/stacked/` of the night's ticked targets, each with
/// its SHA-256 (the cache spares the files already read). A render written as
/// PNG and TIFF gives both, each naming the other (`twin_sha256`): which of
/// the two leaves is the check's to decide ([`precheck_candidates`]).
pub fn local_images(
    night_folder: &Path,
    detection: &NightDetection,
    cache: Option<&DigestCache>,
    cancel: Option<&CancelFlag>,
) -> Result<Vec<LocalImage>, ScanError> {
    let walk = scanner::walk_folder(night_folder, cancel)?;
    let mut images = Vec::new();
    for target in detection.targets.iter().filter(|t| t.in_night) {
        let Some(folder) = &target.folder_name else {
            continue;
        };
        let mut finals: Vec<&scanner::WalkedFile> = walk
            .files
            .iter()
            .filter(|f| {
                f.relative_path
                    .strip_prefix(&format!("{folder}/{FINAL_DIR}/"))
                    .is_some_and(|rest| !rest.contains('/') && is_final_image(rest))
            })
            .collect();
        let proposed = target.final_image.as_deref();
        finals.sort_by_key(|f| {
            let name = f.relative_path.rsplit('/').next().unwrap_or_default();
            let rank = if proposed.is_some_and(|p| p == name || f.relative_path.ends_with(p)) {
                0
            } else {
                target
                    .other_final_images
                    .iter()
                    .position(|o| o == name || f.relative_path.ends_with(o.as_str()))
                    .map_or(usize::MAX, |p| p + 1)
            };
            (rank, f.relative_path.clone())
        });
        // `<target>/final/` is the person's own: it leaves as it is,
        // whatever its format.
        for file in finals {
            images.push(LocalImage {
                target_id: target.target_id.clone(),
                path: file.path.clone(),
                sha256: scanner::digest_of(file, cache, cancel)?,
                size: file.stamp.size,
                kind: ResultKind::Final,
                twin_sha256: None,
            });
        }
        let stacked: Vec<&scanner::WalkedFile> = walk
            .files
            .iter()
            .filter(|f| {
                f.relative_path
                    .strip_prefix(&format!("{folder}/{STACKED_DIR}/"))
                    .is_some_and(|rest| !rest.contains('/'))
            })
            .collect();
        for (file, twin) in render_files::pair_render_files(stacked, |f| f.path.as_path()) {
            let sha256 = scanner::digest_of(file, cache, cancel)?;
            let twin = match twin {
                Some(tiff) => Some((tiff, scanner::digest_of(tiff, cache, cancel)?)),
                None => None,
            };
            images.push(LocalImage {
                target_id: target.target_id.clone(),
                path: file.path.clone(),
                sha256: sha256.clone(),
                size: file.stamp.size,
                kind: ResultKind::Stacked,
                twin_sha256: twin.as_ref().map(|(_, digest)| digest.clone()),
            });
            if let Some((tiff, digest)) = twin {
                images.push(LocalImage {
                    target_id: target.target_id.clone(),
                    path: tiff.path.clone(),
                    sha256: digest,
                    size: tiff.stamp.size,
                    kind: ResultKind::Stacked,
                    twin_sha256: Some(sha256),
                });
            }
        }
    }
    Ok(images)
}

/// The images the "Summary" checks: those the night's folder holds
/// ([`ResumeLocal::candidates`]) and `extra` (the renders and progression
/// images this sending just made).
///
/// **The TIFF of a render that also leaves as a PNG is kept only when
/// `include_tiff`** (the box « Joindre aussi les TIFF » of the sending,
/// [`crate::render_files`]): the gallery shows the PNG, and the TIFF only
/// serves the download in full resolution, so it is the person's choice,
/// never the default. A render that exists only as a TIFF (the linear one)
/// and the person's own `final/` image are never concerned.
///
/// A stacked image an earlier stacking left in `<target>/stacked/` is **never
/// a final image to send again**: when this sending stacks the target again
/// -- `restack` names it before the stacking runs, or `extra` already holds
/// its new renders -- the new stacking replaces what the folder held, so the
/// old output is dropped (it would otherwise be counted, or sent, besides the
/// new one: the new render is in `stacked/` too once written, and in
/// `extra`). When the target is not stacked again, the old output stays a
/// candidate **as a stacked image**: identical to one online it is « already
/// online », else it leaves with its kind.
#[must_use]
pub fn precheck_candidates(
    local: &ResumeLocal,
    extra: Vec<ImageCandidate>,
    restack: &[String],
    include_tiff: bool,
) -> Vec<ImageCandidate> {
    let replaced = |target: &str| {
        restack.iter().any(|t| t == target)
            || extra
                .iter()
                .any(|e| e.kind == ResultKind::Stacked && e.target_id == target)
    };
    let mut candidates: Vec<ImageCandidate> = local
        .candidates()
        .into_iter()
        .filter(|c| !(c.kind == ResultKind::Stacked && replaced(&c.target_id)))
        .collect();
    candidates.extend(extra);
    candidates.retain(|c| render_files::leaves(c.is_full_resolution_twin(), include_tiff));
    candidates
}

/// The TIFF of the renders a sending could join (« Joindre aussi les TIFF
/// (pleine résolution, ≈ 47 Mo chacun) »): how many there are and what they
/// weigh, whether they are joined or not.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FullResolution {
    /// The box is ticked: they are among the candidates.
    pub joined: bool,
    pub count: u64,
    pub bytes: u64,
}

/// [`FullResolution`] of a sending: the TIFF twins among the candidates taken
/// with every TIFF (`precheck_candidates(.., true)`).
#[must_use]
pub fn full_resolution_of(with_tiff: &[ImageCandidate], joined: bool) -> FullResolution {
    let twins = with_tiff.iter().filter(|c| c.is_full_resolution_twin());
    FullResolution {
        joined,
        count: twins.clone().count() as u64,
        bytes: twins.map(|c| c.size).sum(),
    }
}

/// What the resume reads in the night's folder (or `Missing` without one).
pub fn resume_local(
    folder: FolderState,
    night_folder: Option<&Path>,
    detection: Option<&NightDetection>,
    cache: Option<&DigestCache>,
    cancel: Option<&CancelFlag>,
) -> Result<ResumeLocal, ScanError> {
    let (images, inputs): (Vec<LocalImage>, Option<AnalysisInputs>) =
        match (night_folder, detection) {
            (Some(night_folder), Some(detection)) if folder == FolderState::Found => (
                local_images(night_folder, detection, cache, cancel)?,
                Some(analysis_inputs(night_folder)?),
            ),
            _ => (Vec::new(), None),
        };
    Ok(ResumeLocal::from_detection(
        folder, detection, images, inputs,
    ))
}

/// What the night gives the treatments: frames, logs, a final image here or
/// online.
#[must_use]
pub fn night_material(
    detection: Option<&NightDetection>,
    online: Option<&NightOnline>,
) -> NightMaterial {
    let online_final = online.is_some_and(|o| {
        o.targets
            .iter()
            .any(|t| t.captures.iter().any(|c| c.main_image.is_some()))
    });
    NightMaterial {
        light_frames: detection.is_some_and(|d| d.targets.iter().any(|t| t.light_frames > 0)),
        device_images: detection.is_some_and(|d| {
            d.targets.iter().any(|t| {
                t.device
                    .as_ref()
                    .is_some_and(|device| !device.images.is_empty())
            })
        }),
        logs: detection.is_some_and(|d| !d.logs.is_empty()),
        final_image: online_final
            || detection.is_some_and(|d| d.targets.iter().any(|t| t.final_image.is_some())),
    }
}

/// The "Summary": what each image becomes, what the service said,
/// the four limits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NightPrecheck {
    pub storage: SpaceStorage,
    pub plans: Vec<ImagePlan>,
    pub quota: QuotaCheck,
    /// Several renders of the stacking may be ticked (each a variant).
    pub several_renders_allowed: bool,
    /// The TIFF of the renders this sending could join (set by the caller,
    /// [`full_resolution_of`]).
    #[serde(default)]
    pub full_resolution: FullResolution,
}

/// The check before a sending (see [`NightPrecheck`]): nothing is sent.
pub fn precheck_night<T: PrecheckTransport>(
    transport: &T,
    portal_url: &str,
    token: &str,
    online: &NightOnline,
    candidates: &[ImageCandidate],
) -> Result<NightPrecheck, GuiBridgeError> {
    let storage = transport.space_storage(portal_url, token)?;
    let mut plans = plan_images_within(
        online,
        candidates,
        storage.variant_limit,
        storage.max_file_bytes,
    );
    let refused = check_with_service(transport, portal_url, token, &online.session_id, &mut plans)?;
    rebind_progressions(&mut plans);
    let mut quota = check_quotas(&storage, online, &plans);
    quota.findings.extend(refused);
    quota.allowed = quota.findings.is_empty();
    Ok(NightPrecheck {
        several_renders_allowed: several_renders_allowed(&storage),
        full_resolution: FullResolution::default(),
        storage,
        plans,
        quota,
    })
}

/// Solves and sends every sky solution the night's images online still
/// lack: from the local file of the same digest, else from the original
/// fetched (`hints` by target id: the target's position). One outcome per
/// image; a resolution that finds nothing does not stop the others.
pub fn solve_night<T: SkySolutionTransport + OriginalTransport>(
    transport: &T,
    portal_url: &str,
    token: &str,
    online: &NightOnline,
    local: &ResumeLocal,
    hints: &BTreeMap<String, SolveHints>,
    setup: &SolveSetup<'_>,
) -> Result<Vec<SolveOutcome>, GuiBridgeError> {
    let resume = night_resume::resume_night(online, local, &[], None, 0);
    let mut outcomes = Vec::new();
    for source in &resume.sky_solutions {
        if setup.cancel.is_some_and(CancelFlag::is_cancelled) {
            break;
        }
        let capture_id = match source {
            night_resume::SolveFrom::Local { capture_id, .. }
            | night_resume::SolveFrom::Original { capture_id, .. } => capture_id,
        };
        let hint = online
            .images()
            .find(|(c, _)| &c.id == capture_id)
            .and_then(|(c, _)| c.target_ids.iter().find_map(|t| hints.get(t)))
            .copied()
            .unwrap_or_default();
        outcomes.push(solve_and_send(
            transport, portal_url, token, source, hint, setup,
        )?);
    }
    Ok(outcomes)
}

/// What a sending that went through changes in the night's ledger.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SentRecord {
    /// The analysis was sent from these inputs ([`AnalysisInputs`]).
    pub analysis_fingerprint: Option<String>,
    /// The progression of a target was made from this many light frames.
    pub progression_frames: BTreeMap<String, u64>,
}

/// Writes the ledger of `night_id` after a sending that went through.
pub fn record_sending(
    paths: &DataPaths,
    night_id: &str,
    sent: &SentRecord,
) -> std::io::Result<NightLedger> {
    let mut ledger = night_resume::load_ledger(&paths.nights, night_id).unwrap_or_default();
    if let Some(fingerprint) = &sent.analysis_fingerprint {
        ledger.analysis_inputs = Some(fingerprint.clone());
    }
    for (target, frames) in &sent.progression_frames {
        ledger.progression_frames.insert(target.clone(), *frames);
    }
    night_resume::save_ledger(&paths.nights, night_id, &ledger)?;
    Ok(ledger)
}

/// The lines of the night's masters for each multi-night plan its targets
/// work for (`night`: [`crate::multi_night::night_projects`]), sent under
/// `gate`. The evening is the plan's (`night.evening_date`), else
/// `evening_date`. One outcome per plan; a refusal of the service is one of
/// them, the others are still sent.
#[allow(clippy::too_many_arguments)]
pub fn send_night_integrations<T: MultiNightTransport>(
    transport: &T,
    portal_url: &str,
    token: &str,
    night_id: &str,
    night_folder: &Path,
    night: &NightMultiNight,
    evening_date: &str,
    computer: &Computer,
    gate: MultiNightGate,
) -> Result<Vec<IntegrationsSent>, GuiBridgeError> {
    let evening = night.evening_date.as_deref().unwrap_or(evening_date);
    let mut outcomes = Vec::new();
    for project in &night.projects {
        // The masters kept, and the image of the device for a family no master covers.
        let masters = night_sources(
            &target_masters(night_folder, &project.target_id),
            &target_device_images(night_folder, &project.target_id),
        );
        let lines = integration_lines(night_id, &project.target_id, evening, &masters, computer);
        outcomes.push(send_integrations(
            transport, portal_url, token, project, &lines, gate,
        )?);
    }
    Ok(outcomes)
}

#[cfg(test)]
mod tests;

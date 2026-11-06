//! Nothing in the space before the « Récapitulatif »: the treatments of
//! the import run on **provisional** nights and
//! plans, and the sessions, the multi-night plans, the incidents and the
//! images are written in one gesture at the end.
//!
//! - [`provisional_outcome`]: what « Créer … » would make of the review, with
//!   a provisional identifier (`local-<hash>`, [`PROVISIONAL_PREFIX`]) for each
//!   session and plan to create, computed here without the service. The
//!   treatments ([`super::prepare_treatments`]) name each night's folder after
//!   it (`AAAA-MM-JJ · <lieu> · local-<hash>`) and write the night's proposal
//!   in its record ([`PendingNight`], `pendingSessionId: null`); the image of a
//!   plan is made in a plan folder of the same kind.
//! - Once the space created them ([`run_import_stages`], stage
//!   [`ImportStage::Sessions`]): [`record_created`] writes in each record the
//!   session (the plan) created and where the folder is, **before** anything is
//!   renamed; then [`settle_nights`] and [`settle_plans`] rename the folders to
//!   the names the application uses (`… · <id court>`), rewrite the
//!   references to them (the JSON Astro Sync wrote there and in the plan
//!   folders, the links it made pointing inside, the rows of the queue) and
//!   the record itself, last.
//! - [`reconcile`], at each start of the application: a rename the last run
//!   left half done (the session created, the folder not renamed yet) is
//!   finished from the records alone.
//! - [`discard_provisional`], « Abandonner l'import »: the provisional folders
//!   are removed -- their links, Astro Sync's results --, never a file of the
//!   person's (a file left there keeps its folder).
//!
//! The person's files never move: the folders hold links to them (absolute),
//! which a rename of the folder leaves whole.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::gui_bridge::GuiBridgeError;
use crate::plan_image::{
    PLAN_IDENTITY_FILE, PendingPlan, plan_folder_name, read_plan_identity, write_plan_identity,
};
use crate::upload_queue::{UploadQueue, UploadQueueError};
use crate::working_folder::{
    NIGHT_IDENTITY_FILE, PROGRESSION_DIR, PROVISIONAL_PREFIX, PendingNight, PendingTarget,
    STACKED_DIR, TARGET_IDENTITY_FILE, WORK_DIR, WorkingFolderError, is_provisional,
    list_night_folders, night_folder_name, read_night_identity, rename_night_folder,
    write_night_identity,
};

use super::create::{BulkCreateTransport, bulk_body, create_sessions, outcome_of};
use super::{BulkCreateOutcome, BulkCreateRequest, DetectedSession, Proposal, SessionMatch};

/* ------------------------------ The identifiers --------------------------- */

fn hashed(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("{PROVISIONAL_PREFIX}{}", &hex[..12])
}

/// The folder the frames of a night share (their nearest common folder).
fn frames_folder(session: &DetectedSession) -> String {
    let mut common: Option<PathBuf> = None;
    for file in session.targets.iter().flat_map(|t| t.files.iter()) {
        let parent = Path::new(file).parent().unwrap_or(Path::new(""));
        common = Some(match common {
            None => parent.to_path_buf(),
            Some(held) => held
                .ancestors()
                .find(|a| parent.starts_with(a))
                .map_or_else(PathBuf::new, Path::to_path_buf),
        });
    }
    common.map_or_else(String::new, |c| c.to_string_lossy().into_owned())
}

/// The provisional identifier of a night to create: a short hash of the
/// folder its frames share and of its evening, the same for the same folder
/// read again (a second import of it finds its folder again).
#[must_use]
pub fn provisional_session_id(session: &DetectedSession) -> String {
    hashed(&format!(
        "night|{}|{}",
        frames_folder(session),
        session.evening_date
    ))
}

/// The provisional identifier of a multi-night plan to create: its target and
/// its nights.
#[must_use]
pub fn provisional_plan_id(proposal: &Proposal, group_id: &str) -> String {
    let Some(group) = proposal.multi_night.iter().find(|g| g.id == group_id) else {
        return hashed(&format!("plan|{group_id}"));
    };
    let nights: Vec<String> = group
        .nights
        .iter()
        .map(|night| {
            proposal
                .sessions
                .iter()
                .find(|s| s.id == night.session_id)
                .map_or_else(|| night.session_id.clone(), provisional_session_id)
        })
        .collect();
    hashed(&format!("plan|{}|{}", group.target_key, nights.join("|")))
}

/* --------------------------- The outcome, locally -------------------------- */

/// What « Créer … » would make of the review, computed here: the body the
/// service would receive ([`bulk_body`]) read back as if the service had
/// answered, a session to create taking its provisional identifier, a session
/// that exists keeping its own, a plan to create a provisional one. Nothing is
/// asked of the service, nothing is written.
#[must_use]
pub fn provisional_outcome(request: &BulkCreateRequest) -> BulkCreateOutcome {
    let proposal = &request.proposal;
    let built = bulk_body(request);
    let mut ids: BTreeMap<String, String> = BTreeMap::new();
    let mut sessions = Vec::new();
    for sent in built.body["sessions"].as_array().into_iter().flatten() {
        let reference = sent["ref"].as_str().unwrap_or_default();
        let Some(detected) = proposal.sessions.iter().find(|s| s.id == reference) else {
            continue;
        };
        let existing = sent["sessionId"].as_str().map(str::to_string);
        let session_id = existing
            .clone()
            .unwrap_or_else(|| provisional_session_id(detected));
        ids.insert(reference.to_string(), session_id.clone());
        let (plan_action, plan_id) = match &detected.existing {
            SessionMatch::Create => ("created", None),
            SessionMatch::ReusePlan { plan_id, .. } => ("reused", Some(plan_id.clone())),
            SessionMatch::ReplacePlan { plan_id, .. } => ("replaced", Some(plan_id.clone())),
            _ => ("none", None),
        };
        let added: Vec<Value> = sent["targets"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t["designation"].as_str().map(|d| json!(d)))
            .collect();
        sessions.push(json!({
            "ref": reference,
            "sessionId": session_id,
            "action": if existing.is_some() { "updated" } else { "created" },
            "planAction": plan_action,
            "planId": plan_id,
            "addedTargets": if existing.is_some() { added } else { Vec::new() },
        }));
    }
    let multi: Vec<Value> = built.body["multiNight"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|sent| {
            let reference = sent["ref"].as_str().unwrap_or_default();
            let nights: Vec<String> = sent["sessionRefs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|r| ids.get(r.as_str()?).cloned())
                .collect();
            json!({
                "ref": reference,
                "action": "created",
                "projectId": provisional_plan_id(proposal, reference),
                "sessionIds": nights,
            })
        })
        .collect();
    let places: Vec<Value> = built.body["places"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|place| {
            let reference = place["ref"].as_str().unwrap_or_default();
            json!({
                "ref": reference,
                "placeId": hashed(&format!("place|{reference}")),
                "created": true,
            })
        })
        .collect();
    let answer = json!({
        "places": places,
        "sessions": sessions,
        "multiNight": multi,
        "replayed": false,
    });
    outcome_of(request, &built, &answer)
}

/// The night's proposal written in the record of its provisional folder.
#[must_use]
pub fn pending_night(proposal: &Proposal, session: &DetectedSession, place: &str) -> PendingNight {
    let calibration: BTreeSet<String> = session
        .calibration
        .iter()
        .filter_map(|attached| {
            proposal
                .calibration_sets
                .iter()
                .find(|set| set.id == attached.set_id)
                .map(|set| set.folder.clone())
        })
        .collect();
    let logs: BTreeSet<String> = session
        .logs
        .iter()
        .filter_map(|attached| {
            proposal
                .logs
                .iter()
                .find(|log| log.id == attached.log_id)
                .map(|log| log.path.clone())
        })
        .collect();
    PendingNight {
        pending_session_id: None,
        folder: None,
        detected_id: session.id.clone(),
        evening_date: session.evening_date.clone(),
        place: place.to_string(),
        targets: session
            .targets
            .iter()
            .map(|t| PendingTarget {
                target_id: t.target_id.clone(),
                designation: t.designation.clone(),
                frames: t.frames,
            })
            .collect(),
        equipment_ids: session.equipment.clone(),
        calibration: calibration.into_iter().collect(),
        logs: logs.into_iter().collect(),
    }
}

/* --------------------------------- Errors --------------------------------- */

#[derive(Debug, thiserror::Error)]
pub enum ProvisionalError {
    #[error(transparent)]
    WorkingFolder(#[from] WorkingFolderError),
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Queue(#[from] UploadQueueError),
}

impl ProvisionalError {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            ProvisionalError::WorkingFolder(error) => error.code(),
            ProvisionalError::Io { .. } => "io_error",
            ProvisionalError::Queue(_) => "queue_error",
        }
    }
}

fn io_at(path: &Path) -> impl FnOnce(std::io::Error) -> ProvisionalError + '_ {
    move |source| ProvisionalError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/* ------------------------- After the space created them -------------------- */

/// The plan folders of the working folder (found by their identity).
fn plan_folders(root: &Path) -> Vec<PathBuf> {
    fs::read_dir(root)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.is_dir() && path.join(PLAN_IDENTITY_FILE).is_file())
                .collect()
        })
        .unwrap_or_default()
}

/// The provisional night folder carrying `night_id`, directly under `root`.
fn provisional_folder(root: &Path, night_id: &str) -> Option<PathBuf> {
    list_night_folders(root)
        .ok()?
        .into_iter()
        .find(|entry| entry.night_id == night_id)
        .map(|entry| entry.folder)
}

/// What [`record_created`] wrote.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recorded {
    pub nights: usize,
    pub plans: usize,
}

/// The space created the sessions and plans of `provisional` as `created`:
/// each provisional record under `root` learns its session (its plan) and
/// where its folder is, before any folder is renamed. A night the space did
/// not create keeps its provisional record.
pub fn record_created(
    root: &Path,
    provisional: &BulkCreateOutcome,
    created: &BulkCreateOutcome,
) -> Result<Recorded, ProvisionalError> {
    let mut recorded = Recorded::default();
    for local in provisional
        .sessions
        .iter()
        .filter(|s| is_provisional(&s.session_id))
    {
        let Some(real) = created
            .sessions
            .iter()
            .find(|s| s.detected_id == local.detected_id)
        else {
            continue;
        };
        let Some(folder) = provisional_folder(root, &local.session_id) else {
            continue;
        };
        let Some(mut identity) = read_night_identity(&folder) else {
            continue;
        };
        let Some(pending) = identity.pending.as_mut() else {
            continue;
        };
        if pending.pending_session_id.as_deref() == Some(real.session_id.as_str()) {
            continue;
        }
        pending.pending_session_id = Some(real.session_id.clone());
        pending.folder = Some(folder.clone());
        write_night_identity(&folder, &identity)?;
        recorded.nights += 1;
    }
    for local in &provisional.multi_night {
        let Some(real) = created
            .multi_night
            .iter()
            .find(|m| m.group_id == local.group_id)
        else {
            continue;
        };
        let Some(folder) = plan_folders(root).into_iter().find(|folder| {
            read_plan_identity(folder).is_some_and(|i| i.project_id == local.plan_id)
        }) else {
            continue;
        };
        let Some(mut identity) = read_plan_identity(&folder) else {
            continue;
        };
        let pending = identity.pending.get_or_insert_with(|| PendingPlan {
            pending_project_id: None,
            folder: None,
            group_id: local.group_id.clone(),
        });
        if pending.pending_project_id.as_deref() == Some(real.plan_id.as_str()) {
            continue;
        }
        pending.pending_project_id = Some(real.plan_id.clone());
        pending.folder = Some(folder.clone());
        write_plan_identity(&folder, &identity).map_err(io_at(&folder))?;
        recorded.plans += 1;
    }
    Ok(recorded)
}

/// A provisional folder renamed after what the space created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settled {
    /// The provisional identifier.
    pub from_id: String,
    /// The session (the plan) the space created.
    pub to_id: String,
    pub from: PathBuf,
    pub to: PathBuf,
}

/// Every string of `value` (keys too) with the old folder's path at its start
/// replaced by the new one, and the provisional identifiers by the created
/// ones. Returns whether something changed.
fn rewrite_value(value: &mut Value, from: &str, to: &str, ids: &[(String, String)]) -> bool {
    let swap = |text: &str| -> Option<String> {
        let mut out = text.to_string();
        if !from.is_empty()
            && (out == from
                || out.starts_with(&format!("{from}/"))
                || out.starts_with(&format!("{from}\\")))
        {
            out = format!("{to}{}", &out[from.len()..]);
        }
        for (old, new) in ids {
            if out.contains(old.as_str()) {
                out = out.replace(old.as_str(), new);
            }
        }
        (out != text).then_some(out)
    };
    match value {
        Value::String(text) => match swap(text) {
            Some(new) => {
                *text = new;
                true
            }
            None => false,
        },
        Value::Array(items) => items.iter_mut().fold(false, |changed, item| {
            rewrite_value(item, from, to, ids) || changed
        }),
        Value::Object(map) => {
            let entries: Vec<(String, Value)> = std::mem::take(map).into_iter().collect();
            let mut changed = false;
            for (key, mut item) in entries {
                changed |= rewrite_value(&mut item, from, to, ids);
                let key = match swap(&key) {
                    Some(new) => {
                        changed = true;
                        new
                    }
                    None => key,
                };
                map.insert(key, item);
            }
            changed
        }
        _ => false,
    }
}

/// The references Astro Sync made to `from` under `folder` (the JSON it wrote,
/// the links it made pointing inside it), now to `to`; links are never
/// followed. Returns how many files changed.
fn rewrite_references(
    folder: &Path,
    from: &Path,
    to: &Path,
    ids: &[(String, String)],
) -> Result<usize, ProvisionalError> {
    let mut changed = 0;
    let Ok(entries) = fs::read_dir(folder) else {
        return Ok(0);
    };
    let from_text = from.to_string_lossy().into_owned();
    let to_text = to.to_string_lossy().into_owned();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let Ok(kind) = fs::symlink_metadata(&path).map(|m| m.file_type()) else {
            continue;
        };
        if kind.is_symlink() {
            let Ok(target) = fs::read_link(&path) else {
                continue;
            };
            if let Ok(rest) = target.strip_prefix(from) {
                fs::remove_file(&path).map_err(io_at(&path))?;
                make_link(&to.join(rest), &path).map_err(io_at(&path))?;
                changed += 1;
            }
        } else if kind.is_dir() {
            changed += rewrite_references(&path, from, to, ids)?;
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(mut value) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if rewrite_value(&mut value, &from_text, &to_text, ids) {
                let beside = path.with_extension("json.tmp");
                fs::write(
                    &beside,
                    serde_json::to_string_pretty(&value).unwrap_or_default() + "\n",
                )
                .map_err(io_at(&beside))?;
                fs::rename(&beside, &path).map_err(io_at(&path))?;
                changed += 1;
            }
        }
    }
    Ok(changed)
}

#[cfg(unix)]
fn make_link(source: &Path, target: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, target)
}

#[cfg(windows)]
fn make_link(source: &Path, target: &Path) -> std::io::Result<()> {
    if source.is_dir() {
        std::os::windows::fs::symlink_dir(source, target)
    } else {
        std::os::windows::fs::symlink_file(source, target)
    }
}

/// Renames every night folder under `root` whose record names the session
/// created ([`record_created`]) to the name of that session, rewrites what
/// points into it (its JSON and links, the plan folders', the queue's rows)
/// and its record last. Run again after a stop: a folder renamed already is
/// not renamed twice, a reference rewritten already is left.
pub fn settle_nights(
    root: &Path,
    queue: Option<&UploadQueue>,
) -> Result<Vec<Settled>, ProvisionalError> {
    let mut settled = Vec::new();
    for entry in list_night_folders(root)? {
        let Some(mut identity) = read_night_identity(&entry.folder) else {
            continue;
        };
        let Some(pending) = identity.pending.clone() else {
            continue;
        };
        let Some(session_id) = pending.pending_session_id.clone() else {
            continue;
        };
        let from = pending
            .folder
            .clone()
            .unwrap_or_else(|| entry.folder.clone());
        let wanted = night_folder_name(&pending.evening_date, &pending.place, &session_id)?;
        let to = rename_night_folder(&entry.folder, &wanted)?;
        let ids = vec![(identity.night_id.clone(), session_id.clone())];
        // Rewritten again after a stop: what was rewritten already is left.
        rewrite_references(&to, &from, &to, &ids)?;
        for plan in plan_folders(root) {
            rewrite_references(&plan, &from, &to, &ids)?;
        }
        if let Some(queue) = queue {
            queue.rebase_provisional(&from, &to, &ids)?;
        }
        settled.push(Settled {
            from_id: identity.night_id.clone(),
            to_id: session_id.clone(),
            from,
            to: to.clone(),
        });
        identity.night_id = session_id;
        identity.pending = None;
        write_night_identity(&to, &identity)?;
    }
    Ok(settled)
}

/// Renames every plan folder under `root` whose identity names the plan
/// created to the name of that plan, rewrites what it holds (the provisional
/// identifiers of the plan, of its nights) and the queue's rows, and its
/// identity last. Run after [`settle_nights`].
pub fn settle_plans(
    root: &Path,
    nights: &[Settled],
    queue: Option<&UploadQueue>,
) -> Result<Vec<Settled>, ProvisionalError> {
    let mut settled = Vec::new();
    for folder in plan_folders(root) {
        let Some(mut identity) = read_plan_identity(&folder) else {
            continue;
        };
        let Some(pending) = identity.pending.clone() else {
            continue;
        };
        let Some(project_id) = pending.pending_project_id.clone() else {
            continue;
        };
        let from = pending.folder.clone().unwrap_or_else(|| folder.clone());
        let wanted = plan_folder_name(&identity.name, &project_id);
        let to = rename_night_folder(&folder, &wanted)?;
        let mut ids = vec![(identity.project_id.clone(), project_id.clone())];
        ids.extend(
            nights
                .iter()
                .map(|night| (night.from_id.clone(), night.to_id.clone())),
        );
        rewrite_references(&to, &from, &to, &ids)?;
        if let Some(queue) = queue {
            queue.rebase_provisional(&from, &to, &ids)?;
        }
        settled.push(Settled {
            from_id: identity.project_id.clone(),
            to_id: project_id.clone(),
            from,
            to: to.clone(),
        });
        identity.project_id = project_id;
        identity.pending = None;
        write_plan_identity(&to, &identity).map_err(io_at(&to))?;
    }
    Ok(settled)
}

/// What [`reconcile`] finished.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reconciled {
    pub nights: Vec<Settled>,
    pub plans: Vec<Settled>,
}

/// At each start of the application: the renames a stop left half done (the
/// session created and written in the record, the folder not renamed or its
/// record not rewritten) are finished from the records alone. A night whose
/// session was never created keeps its provisional folder.
pub fn reconcile(root: &Path, queue: Option<&UploadQueue>) -> Result<Reconciled, ProvisionalError> {
    let nights = settle_nights(root, queue)?;
    let plans = settle_plans(root, &nights, queue)?;
    Ok(Reconciled { nights, plans })
}

/* ---------------------------- « Abandonner l'import » ---------------------- */

/// What [`discard_provisional`] removed and kept.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Discarded {
    /// The provisional folders removed whole.
    pub removed: usize,
    /// The files of the person's found in a provisional folder: kept, with the
    /// folders that hold them.
    pub kept: usize,
}

/// Whether `path` (a file in a provisional folder) is Astro Sync's own: its
/// identity files, its results (`stacked/`, `progression/`), its scratch.
fn made_by_astro_sync(folder: &Path, path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    if matches!(
        name.as_deref(),
        Some(NIGHT_IDENTITY_FILE | TARGET_IDENTITY_FILE | PLAN_IDENTITY_FILE)
    ) {
        return true;
    }
    if folder.join(PLAN_IDENTITY_FILE).is_file() {
        // A plan folder holds nothing but the images of the plan.
        return true;
    }
    path.strip_prefix(folder).is_ok_and(|relative| {
        relative.components().any(|part| {
            let part = part.as_os_str().to_string_lossy();
            part == STACKED_DIR || part == PROGRESSION_DIR || part == WORK_DIR
        })
    })
}

/// Removes what Astro Sync made under `dir` (links, its own files) and the
/// folders left empty; counts the files kept.
fn empty_out(folder: &Path, dir: &Path, kept: &mut usize) -> Result<(), ProvisionalError> {
    let entries = fs::read_dir(dir).map_err(io_at(dir))?;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let kind = fs::symlink_metadata(&path)
            .map_err(io_at(&path))?
            .file_type();
        if kind.is_symlink() {
            // The link only: never what it points to.
            fs::remove_file(&path).map_err(io_at(&path))?;
        } else if kind.is_dir() {
            empty_out(folder, &path, kept)?;
            let _ = fs::remove_dir(&path);
        } else if made_by_astro_sync(folder, &path) {
            fs::remove_file(&path).map_err(io_at(&path))?;
        } else {
            *kept += 1;
        }
    }
    Ok(())
}

/// « Abandonner l'import », after the person confirmed: the provisional
/// folders of `ids` (nights and plans whose session or plan was never
/// created) are removed under `root`. Only links and Astro Sync's own files
/// are removed: a file of the person's keeps its folder.
pub fn discard_provisional(root: &Path, ids: &[String]) -> Result<Discarded, ProvisionalError> {
    let wanted = |id: &str| is_provisional(id) && ids.iter().any(|i| i == id);
    let mut folders: Vec<PathBuf> = list_night_folders(root)?
        .into_iter()
        .filter(|entry| wanted(&entry.night_id))
        .filter(|entry| {
            read_night_identity(&entry.folder)
                .and_then(|i| i.pending)
                .is_some_and(|p| p.pending_session_id.is_none())
        })
        .map(|entry| entry.folder)
        .collect();
    folders.extend(plan_folders(root).into_iter().filter(|folder| {
        read_plan_identity(folder).is_some_and(|identity| {
            wanted(&identity.project_id)
                && identity
                    .pending
                    .as_ref()
                    .is_none_or(|p| p.pending_project_id.is_none())
        })
    }));
    let mut discarded = Discarded::default();
    for folder in folders {
        let mut kept = 0;
        empty_out(&folder, &folder, &mut kept)?;
        if kept == 0 && fs::remove_dir(&folder).is_ok() {
            discarded.removed += 1;
        }
        discarded.kept += kept;
    }
    Ok(discarded)
}

/* ----------------------------- The stage runner ---------------------------- */

/// The stages of the creation, in their order: the sessions and plans
/// written by the service's one call, then the night folders, then the plan
/// folders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportStage {
    Sessions,
    Nights,
    Plans,
}

/// What the stages did so far: a second run skips the stages done.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportProgress {
    pub created: Option<BulkCreateOutcome>,
    pub nights: Option<Vec<Settled>>,
    pub plans: Option<Vec<Settled>>,
}

/// Why a stage stopped: the ones before it stay done.
#[derive(Debug, thiserror::Error)]
pub enum StageError {
    #[error(transparent)]
    Service(#[from] GuiBridgeError),
    #[error(transparent)]
    Local(#[from] ProvisionalError),
}

/// Runs `stages`, in their order, skipping each one `progress` holds done; at
/// the first failure, stops and says which stage failed (what went through
/// stays, in `progress`). The service's call replays on a second try (its key
/// derives from the body): the sessions are never created twice.
// The call's parts are the stage's own (one per thing it needs).
#[allow(clippy::too_many_arguments)]
pub fn run_import_stages<T: BulkCreateTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
    request: &BulkCreateRequest,
    provisional: &BulkCreateOutcome,
    root: &Path,
    queue: Option<&UploadQueue>,
    stages: &[ImportStage],
    progress: &mut ImportProgress,
    on_stage: &mut dyn FnMut(ImportStage),
) -> Result<(), (ImportStage, StageError)> {
    for stage in [
        ImportStage::Sessions,
        ImportStage::Nights,
        ImportStage::Plans,
    ] {
        if !stages.contains(&stage) {
            continue;
        }
        let done = match stage {
            ImportStage::Sessions => progress.created.is_some(),
            ImportStage::Nights => progress.nights.is_some(),
            ImportStage::Plans => progress.plans.is_some(),
        };
        if done {
            continue;
        }
        on_stage(stage);
        match stage {
            ImportStage::Sessions => {
                let created = create_sessions(transport, base_url, token, request)
                    .map_err(|e| (stage, e.into()))?;
                record_created(root, provisional, &created).map_err(|e| (stage, e.into()))?;
                progress.created = Some(created);
            }
            ImportStage::Nights => {
                progress.nights = Some(settle_nights(root, queue).map_err(|e| (stage, e.into()))?);
            }
            ImportStage::Plans => {
                let nights = progress.nights.clone().unwrap_or_default();
                progress.plans =
                    Some(settle_plans(root, &nights, queue).map_err(|e| (stage, e.into()))?);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

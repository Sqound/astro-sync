//! "Processing files": what Astro
//! Sync's pipelines and scripts produce in the working folder, measured and,
//! when the person asks, cleaned -- and nothing else.
//!
//! **The rule.** What the person copied, moved or linked into the working
//! folder (`lights/`, `calibration/`, `logs/`, `final/`, `device/`, anything
//! no pipeline wrote) is **never** deleted, and is never read as a thing to
//! clean: it is measured only, as "Your files" ([`ProcessingScan::user_bytes`]).
//! A symbolic link is never followed (it weighs nothing here, its target is
//! the person's frame) and is never deleted as a result.
//!
//! **What is produced, hence cleanable** -- four categories, found by the
//! identity files Astro Sync wrote ([`crate::working_folder::NIGHT_IDENTITY_FILE`],
//! [`crate::working_folder::TARGET_IDENTITY_FILE`],
//! [`crate::plan_image::PLAN_IDENTITY_FILE`]), never by a name:
//!
//! - **masters** ([`CategoryKind::Masters`]): `<night>/<target>/stacked/<family>_integration.fit`,
//!   the linear master `integration_master.py` keeps; the next stacking makes
//!   it again;
//! - **night results** ([`CategoryKind::NightResults`]): the other files the
//!   stacking writes in `<night>/<target>/stacked/` (renders, their pictures,
//!   `<name>_stacking.json`);
//! - **plan renders** ([`CategoryKind::PlanRenders`]): the files
//!   `plan_stack.py` writes in `<plan>/stacked/`;
//! - **temporaries** ([`CategoryKind::Temp`]): `<night>/.astro-sync-work/`,
//!   `<plan>/.astro-sync-work/` and the scratch an older build left in
//!   Astro Sync's data (`stacking-work/`).
//!
//! A result is **sent** when every TIFF of its `stacked/` folder is a
//! `completed` row of the upload queue ([`sent_files`]); anything else --
//! no row, a failed or held one, no TIFF at all -- counts as **not sent**
//! (prudence: a copy nowhere else).
//!
//! The Python environment (`stacking-env/`, in Astro Sync's data) is measured
//! apart and is never cleaned: it is reinstalled, not emptied
//! ([`remove_python_environment`]).
//!
//! **What this module deletes**, besides what the other modules already
//! remove: only regular files of those four places, each checked before it
//! goes (not a link; its parent resolves inside the working folder). Inside a
//! scratch folder the links Astro Sync made to the lights are unlinked with
//! the scratch -- the link only, never its target.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::folder_files::{GUIDE_FILE_STEM, README_FILE_NAME};
use crate::plan_image::PLAN_IDENTITY_FILE;
use crate::python_env;
use crate::stacking::{FIELD_SUFFIX, INTEGRATION_SUFFIX};
use crate::upload_queue::UploadQueue;
use crate::working_folder::{
    ASSISTANT_FILE_NAME, NIGHT_IDENTITY_FILE, PROGRESSION_DIR, STACKED_DIR, TARGET_IDENTITY_FILE,
    WORK_DIR,
};

/// Where an older build kept the stacking's scratch, in Astro Sync's data.
pub const LEGACY_SCRATCH_DIR: &str = "stacking-work";

/// What the stacking and the plan image write in a `stacked/` folder, by
/// extension; any other file there is the person's.
const PRODUCED_EXTENSIONS: [&str; 9] = [
    "tif", "tiff", "png", "jpg", "jpeg", "json", "fit", "fits", "part",
];

/// The categories of the section, in their display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CategoryKind {
    Masters,
    NightResults,
    PlanRenders,
    Temp,
}

/// What the person can ask to clean (the boxes of the confirmation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CleanKind {
    Masters,
    SentResults,
    UnsentResults,
    Temp,
}

/// One category of the scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryScan {
    pub kind: CategoryKind,
    pub bytes: u64,
    /// Nights (plans for the plan renders) that hold something of it.
    pub count: u64,
    /// For the results only: what is already in the space, and what is not.
    pub sent_bytes: u64,
    pub unsent_bytes: u64,
    pub sent_count: u64,
    pub unsent_count: u64,
}

/// What `processing_files_scan` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingScan {
    /// The working folder is there. Otherwise every size is zero and
    /// meaningless (an external disk unplugged), the interface says « — ».
    pub reachable: bool,
    pub root: PathBuf,
    pub computed_at: String,
    /// The weight of the whole working folder, the person's files included.
    pub folder_bytes: u64,
    /// What the person put there (copied or moved files; links weigh
    /// nothing): never touched.
    pub user_bytes: u64,
    /// `stacking-env/`, in Astro Sync's data.
    pub python_env_bytes: u64,
    pub categories: Vec<CategoryScan>,
}

/// A file that could not be deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanFailure {
    pub path: PathBuf,
    /// `not_a_plain_file`, `outside_working_folder` or `io_error`.
    pub code: &'static str,
}

/// What `processing_files_clean` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanOutcome {
    pub freed_bytes: u64,
    pub failed: Vec<CleanFailure>,
    /// The result files that are gone, for the callers that keep references
    /// to them (the upload queue).
    #[serde(skip)]
    pub removed: Vec<PathBuf>,
}

#[derive(Debug, Error)]
pub enum CleanError {
    #[error("the working folder is not reachable: {0}")]
    Unreachable(PathBuf),
}

/* ------------------------------- Inventory ------------------------------- */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owner {
    Night,
    Plan,
}

struct Item {
    path: PathBuf,
    bytes: u64,
}

/// One `stacked/` folder of a night's target or of a plan.
struct StackedSet {
    owner: Owner,
    /// The night's (or the plan's) folder.
    home: PathBuf,
    masters: Vec<Item>,
    results: Vec<Item>,
    sent: bool,
}

struct Inventory {
    sets: Vec<StackedSet>,
    /// Scratch folders: (path, inside the working folder).
    scratch: Vec<(PathBuf, bool)>,
    /// What Astro Sync wrote that is neither cleanable nor the person's:
    /// the identity files and the progression images.
    other_produced_bytes: u64,
}

fn real_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir())
}

fn plain_file_len(path: &Path) -> Option<u64> {
    fs::symlink_metadata(path)
        .ok()
        .filter(|meta| meta.is_file())
        .map(|meta| meta.len())
}

/// The weight of the regular files under `path`; links are not followed and
/// weigh nothing.
fn tree_bytes(path: &Path) -> u64 {
    let mut total = 0;
    let mut pending = vec![path.to_path_buf()];
    while let Some(folder) = pending.pop() {
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                total += entry.metadata().map_or(0, |meta| meta.len());
            }
        }
    }
    total
}

fn is_result_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "tif" | "tiff"))
}

fn is_produced(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| PRODUCED_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

fn is_master(name: &str) -> bool {
    let name = name.to_lowercase();
    // The master of the whole field a stacking keeps when the field turns is
    // one too: the next stacking makes it again.
    name.ends_with(INTEGRATION_SUFFIX) || name.ends_with(FIELD_SUFFIX)
}

/// The paths of the queue's finished rows, as written and resolved, whatever
/// the space they were sent to: the working folder is the same for all.
#[must_use]
pub fn sent_files(queue: &UploadQueue) -> HashSet<PathBuf> {
    let mut set = HashSet::new();
    for entry in queue.completed_everywhere().unwrap_or_default() {
        if let Ok(resolved) = fs::canonicalize(&entry.source_path) {
            set.insert(resolved);
        }
        set.insert(entry.source_path);
    }
    set
}

fn is_sent(sent: &HashSet<PathBuf>, path: &Path) -> bool {
    sent.contains(path) || fs::canonicalize(path).is_ok_and(|resolved| sent.contains(&resolved))
}

fn read_stacked(owner: Owner, home: &Path, stacked: &Path, sent: &HashSet<PathBuf>) -> StackedSet {
    let mut set = StackedSet {
        owner,
        home: home.to_path_buf(),
        masters: Vec::new(),
        results: Vec::new(),
        sent: false,
    };
    let Ok(entries) = fs::read_dir(stacked) else {
        return set;
    };
    for entry in entries.filter_map(Result::ok) {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        // A link or a folder is the person's: never listed.
        if !kind.is_file() {
            continue;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let bytes = entry.metadata().map_or(0, |meta| meta.len());
        if is_master(&name) {
            set.masters.push(Item { path, bytes });
        } else if is_produced(&path) {
            set.results.push(Item { path, bytes });
        }
    }
    let images: Vec<&Item> = set
        .results
        .iter()
        .filter(|item| is_result_image(&item.path))
        .collect();
    set.sent = !images.is_empty() && images.iter().all(|item| is_sent(sent, &item.path));
    set
}

fn inventory(root: &Path, data_dir: &Path, sent: &HashSet<PathBuf>) -> Inventory {
    let mut found = Inventory {
        sets: Vec::new(),
        scratch: Vec::new(),
        other_produced_bytes: 0,
    };
    let legacy = data_dir.join(LEGACY_SCRATCH_DIR);
    if real_dir(&legacy) {
        found.scratch.push((legacy, false));
    }
    let Ok(entries) = fs::read_dir(root) else {
        return found;
    };
    for entry in entries.filter_map(Result::ok) {
        let home = entry.path();
        // The files Astro Sync keeps at the root for the person and an AI
        // assistant are its own, small, and not cleanable.
        if entry.file_type().is_ok_and(|kind| kind.is_file()) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ASSISTANT_FILE_NAME
                || name == README_FILE_NAME
                || (name.starts_with(GUIDE_FILE_STEM) && name.ends_with(".md"))
            {
                found.other_produced_bytes += entry.metadata().map_or(0, |meta| meta.len());
            }
            continue;
        }
        // A link to a folder is the person's own: neither read nor followed.
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let night = plain_file_len(&home.join(NIGHT_IDENTITY_FILE));
        let plan = plain_file_len(&home.join(PLAN_IDENTITY_FILE));
        if night.is_none() && plan.is_none() {
            continue;
        }
        found.other_produced_bytes += night.unwrap_or(0) + plan.unwrap_or(0);
        let work = home.join(WORK_DIR);
        if real_dir(&work) {
            found.scratch.push((work, true));
        }
        if plan.is_some() {
            let stacked = home.join(STACKED_DIR);
            if real_dir(&stacked) {
                found
                    .sets
                    .push(read_stacked(Owner::Plan, &home, &stacked, sent));
            }
        }
        if night.is_some() {
            let Ok(targets) = fs::read_dir(&home) else {
                continue;
            };
            for target in targets.filter_map(Result::ok) {
                let folder = target.path();
                if !target.file_type().is_ok_and(|kind| kind.is_dir()) {
                    continue;
                }
                let Some(identity) = plain_file_len(&folder.join(TARGET_IDENTITY_FILE)) else {
                    continue;
                };
                found.other_produced_bytes += identity;
                let progression = folder.join(PROGRESSION_DIR);
                if real_dir(&progression) {
                    found.other_produced_bytes += tree_bytes(&progression);
                }
                let stacked = folder.join(STACKED_DIR);
                if real_dir(&stacked) {
                    found
                        .sets
                        .push(read_stacked(Owner::Night, &home, &stacked, sent));
                }
            }
        }
    }
    found
}

/* --------------------------------- The scan -------------------------------- */

fn sum(items: &[Item]) -> u64 {
    items.iter().map(|item| item.bytes).sum()
}

fn empty_category(kind: CategoryKind) -> CategoryScan {
    CategoryScan {
        kind,
        bytes: 0,
        count: 0,
        sent_bytes: 0,
        unsent_bytes: 0,
        sent_count: 0,
        unsent_count: 0,
    }
}

fn results_category(kind: CategoryKind, owner: Owner, sets: &[StackedSet]) -> CategoryScan {
    let mut category = empty_category(kind);
    let mut homes = HashSet::new();
    let mut sent_homes = HashSet::new();
    let mut unsent_homes = HashSet::new();
    for set in sets
        .iter()
        .filter(|set| set.owner == owner && !set.results.is_empty())
    {
        let bytes = sum(&set.results);
        category.bytes += bytes;
        homes.insert(&set.home);
        if set.sent {
            category.sent_bytes += bytes;
            sent_homes.insert(&set.home);
        } else {
            category.unsent_bytes += bytes;
            unsent_homes.insert(&set.home);
        }
    }
    category.count = homes.len() as u64;
    category.sent_count = sent_homes.len() as u64;
    category.unsent_count = unsent_homes.len() as u64;
    category
}

/// Measures the produced files of the working folder (see the module's
/// documentation). `sent` comes from [`sent_files`]. Never an error: a
/// folder that is not there is `reachable: false`.
#[must_use]
pub fn scan(root: &Path, data_dir: &Path, sent: &HashSet<PathBuf>) -> ProcessingScan {
    let python_env_bytes = {
        let env = python_env::env_dir(data_dir);
        if real_dir(&env) { tree_bytes(&env) } else { 0 }
    };
    let reachable = fs::metadata(root).is_ok_and(|meta| meta.is_dir());
    let mut scan = ProcessingScan {
        reachable,
        root: root.to_path_buf(),
        computed_at: crate::now_iso8601_utc(),
        folder_bytes: 0,
        user_bytes: 0,
        python_env_bytes,
        categories: Vec::new(),
    };
    // The legacy scratch lives in the data folder: still a temporary to say.
    let found = inventory(if reachable { root } else { Path::new("") }, data_dir, sent);
    let mut masters = empty_category(CategoryKind::Masters);
    let mut temp = empty_category(CategoryKind::Temp);
    let mut scratch_in_root = 0;
    for (path, in_root) in &found.scratch {
        let bytes = tree_bytes(path);
        temp.bytes += bytes;
        temp.count += 1;
        if *in_root {
            scratch_in_root += bytes;
        }
    }
    let mut master_homes = HashSet::new();
    for set in found.sets.iter().filter(|set| !set.masters.is_empty()) {
        masters.bytes += sum(&set.masters);
        master_homes.insert(&set.home);
    }
    masters.count = master_homes.len() as u64;
    let night_results = results_category(CategoryKind::NightResults, Owner::Night, &found.sets);
    let plan_renders = results_category(CategoryKind::PlanRenders, Owner::Plan, &found.sets);
    if reachable {
        scan.folder_bytes = tree_bytes(root);
        let produced = masters.bytes
            + night_results.bytes
            + plan_renders.bytes
            + scratch_in_root
            + found.other_produced_bytes;
        scan.user_bytes = scan.folder_bytes.saturating_sub(produced);
    }
    scan.categories = vec![masters, night_results, plan_renders, temp];
    scan
}

/* --------------------------------- The clean ------------------------------- */

/// Deletes `path` if it is a regular file (not a link) whose folder resolves
/// inside `canonical_root`; the size freed.
fn remove_plain_file(path: &Path, canonical_root: &Path) -> Result<u64, &'static str> {
    let meta = fs::symlink_metadata(path).map_err(|_| "io_error")?;
    if !meta.is_file() {
        return Err("not_a_plain_file");
    }
    let parent = path.parent().ok_or("outside_working_folder")?;
    let resolved = fs::canonicalize(parent).map_err(|_| "io_error")?;
    if !resolved.starts_with(canonical_root) {
        return Err("outside_working_folder");
    }
    fs::remove_file(path).map_err(|_| "io_error")?;
    Ok(meta.len())
}

/// Removes a folder of Astro Sync's own work, entry by entry: a regular file
/// is deleted, a link is unlinked (its target is never read, followed or
/// deleted), a folder is emptied then removed. The size of the regular files.
fn remove_tree(folder: &Path, freed: &mut u64) -> std::io::Result<()> {
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            remove_tree(&path, freed)?;
        } else if kind.is_file() {
            let bytes = entry.metadata().map_or(0, |meta| meta.len());
            fs::remove_file(&path)?;
            *freed += bytes;
        } else {
            // A link (to a file or a folder): the link only.
            fs::remove_file(&path).or_else(|_| fs::remove_dir(&path))?;
        }
    }
    fs::remove_dir(folder)
}

/// Removes a scratch folder after checking it is a real folder (not a link)
/// that resolves under `canonical_base`.
fn remove_scratch_tree(path: &Path, canonical_base: &Path) -> Result<u64, &'static str> {
    if !real_dir(path) {
        return Err("not_a_plain_file");
    }
    let resolved = fs::canonicalize(path).map_err(|_| "io_error")?;
    if !resolved.starts_with(canonical_base) {
        return Err("outside_working_folder");
    }
    let mut freed = 0;
    remove_tree(path, &mut freed).map_err(|_| "io_error")?;
    Ok(freed)
}

/// Deletes what `kinds` name -- and only that (see the module's
/// documentation). Refusing while a stacking runs is the caller's: this
/// function cannot know. `sent` is read again by the caller just before.
pub fn clean(
    root: &Path,
    data_dir: &Path,
    sent: &HashSet<PathBuf>,
    kinds: &[CleanKind],
) -> Result<CleanOutcome, CleanError> {
    if !fs::metadata(root).is_ok_and(|meta| meta.is_dir()) {
        return Err(CleanError::Unreachable(root.to_path_buf()));
    }
    let canonical_root =
        fs::canonicalize(root).map_err(|_| CleanError::Unreachable(root.into()))?;
    let canonical_data = fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    let wants = |kind: CleanKind| kinds.contains(&kind);
    let found = inventory(root, data_dir, sent);
    let mut outcome = CleanOutcome {
        freed_bytes: 0,
        failed: Vec::new(),
        removed: Vec::new(),
    };
    let mut delete = |item: &Item| match remove_plain_file(&item.path, &canonical_root) {
        Ok(bytes) => {
            outcome.freed_bytes += bytes;
            outcome.removed.push(item.path.clone());
        }
        Err(code) => outcome.failed.push(CleanFailure {
            path: item.path.clone(),
            code,
        }),
    };
    for set in &found.sets {
        if wants(CleanKind::Masters) {
            set.masters.iter().for_each(&mut delete);
        }
        let wanted = if set.sent {
            wants(CleanKind::SentResults)
        } else {
            wants(CleanKind::UnsentResults)
        };
        if wanted {
            set.results.iter().for_each(&mut delete);
        }
    }
    if wants(CleanKind::Temp) {
        for (path, in_root) in &found.scratch {
            let base = if *in_root {
                &canonical_root
            } else {
                &canonical_data
            };
            match remove_scratch_tree(path, base) {
                Ok(bytes) => outcome.freed_bytes += bytes,
                Err(code) => outcome.failed.push(CleanFailure {
                    path: path.clone(),
                    code,
                }),
            }
        }
    }
    Ok(outcome)
}

/// "Reinstall…": removes the Python environment (`stacking-env/`) so the
/// installation makes it again from nothing. The size freed; nothing when
/// there is none.
pub fn remove_python_environment(data_dir: &Path) -> std::io::Result<u64> {
    let env = python_env::env_dir(data_dir);
    if !real_dir(&env) {
        return Ok(0);
    }
    let mut freed = 0;
    remove_tree(&env, &mut freed)?;
    Ok(freed)
}

#[cfg(all(test, unix))]
mod tests;

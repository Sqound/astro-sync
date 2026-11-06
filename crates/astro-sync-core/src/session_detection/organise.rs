//! "Tidy the files": the frames of
//! the detected sessions copied, moved or linked into an organised tree,
//! **only on an explicit choice**: "Leave in place" is the default, and
//! the one deliberate exception to Astro Sync's rule that it never touches a
//! source file.
//!
//! The tree: `<night>/<target>/lights/[<filter>/]` (`<night>` is
//! `YYYY-MM-DD · <place> · <id court>`, the working folder's rule), the image a
//! known device stacked in `<night>/<target>/device/` (the Seestar's
//! `Stacked_*.fit`; copied, moved or linked like the
//! poses), the night's calibration
//! in `<night>/calibration/<darks|flats|dark-flats|bias>/` (a set shared
//! by several nights is copied into each; the flats of a filter in
//! `calibration/flats/<filter>/`), its logs in `<night>/logs/`. The
//! filter level exists only when `FILTER` is written (the ASI533MC's FITS
//! have none).
//!
//! [`plan_organise`] is the dry run: it describes every action before any is
//! taken. [`organise`] runs a plan, file by file: a copy is written beside
//! its place then renamed (never half a file), an existing file is never
//! overwritten; "Move" copies, checks the copy's SHA-256 against the
//! source's, and only then removes the source, stopping at the first check
//! that fails (that file and every later one stay where they are). On
//! Windows without developer mode a symbolic link cannot be made: the plan
//! says so (`windows_no_link`), and a run that links makes hard links when the
//! frames are on the destination's drive, else refuses before touching
//! anything (`LinkAcrossVolumes`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::detection::FrameType;
use crate::local_work::{CancelFlag, ProgressSink, WorkKind, WorkProgress};
use crate::scanner::file_sha256;
use crate::working_folder::{night_folder_name, target_folder_name};

use super::Proposal;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OrganiseMode {
    /// Astro Sync reads the files where they are (the default).
    #[default]
    LeaveInPlace,
    Copy,
    /// Copy, check by SHA-256, then remove the source.
    Move,
    /// A symbolic link to the source (macOS, Linux; Windows only in
    /// developer mode).
    Symlink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OrganiseActionKind {
    Copy,
    Move,
    Link,
    /// The same file is already at its place: nothing to do.
    AlreadyThere,
}

/// How a file already at its place is there (the window says "links already
/// in place" rather than a bare "0 links created").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InPlaceAs {
    /// A symbolic link to the source (the treatments link every frame into
    /// the working folder).
    Link,
    /// A file of its own (copied or moved there earlier).
    File,
}

/// One file of the plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganiseAction {
    pub source: String,
    pub destination: String,
    pub kind: OrganiseActionKind,
    pub bytes: u64,
    /// For `AlreadyThere` only: how the file is there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_place_as: Option<InPlaceAs>,
}

/// A file left where it is, and why (`destination-exists`,
/// `same-name`, `checksum-differs`, `io`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganiseFailure {
    pub source: String,
    pub destination: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// What a run will do, before it does anything.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganisePlan {
    pub mode: OrganiseMode,
    pub destination: String,
    pub actions: Vec<OrganiseAction>,
    /// Files that will not be touched (a different file at their place, two
    /// sources of the same name).
    pub conflicts: Vec<OrganiseFailure>,
    pub files: usize,
    pub bytes: u64,
    /// Windows: a symbolic link needs developer mode.
    pub windows_no_link: bool,
    /// The free space of the destination's disk, bytes ("412 GB free on
    /// this disk"); `None` when it cannot be read.
    #[serde(default)]
    pub free_bytes: Option<u64>,
    /// The summary shown before acting, in French.
    pub summary: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganiseOutcome {
    pub done: usize,
    /// Copies whose SHA-256 was checked against the source ("Move").
    pub verified: usize,
    pub removed_sources: usize,
    pub already_there: usize,
    pub failed: Vec<OrganiseFailure>,
    /// The run stopped (a check failed, or "Stop the tidying").
    pub stopped: bool,
}

#[derive(Debug, Error)]
pub enum OrganiseError {
    #[error("a symbolic link cannot be made here (Windows without developer mode)")]
    SymlinkUnavailable,
    /// Windows without developer mode: the links fall back to hard links,
    /// which cannot cross from one drive to another.
    #[error(
        "the frames are on {source_volume} and the destination on {destination_volume}: without developer mode Windows links files on one drive only"
    )]
    LinkAcrossVolumes {
        source_volume: String,
        destination_volume: String,
    },
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Default, Clone, Copy)]
pub struct OrganiseOptions<'a> {
    pub cancel: Option<&'a CancelFlag>,
    pub progress: Option<ProgressSink<'a>>,
}

/// `calibration/<darks|flats|dark-flats|bias>/`: the `bias/` spelling (the
/// readers of a night take it beside `biases/`).
fn calibration_folder(kind: FrameType) -> &'static str {
    match kind {
        FrameType::Bias => crate::working_folder::BIAS_ALIAS,
        other => other.calibration_folder().unwrap_or("lights"),
    }
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn clean(component: &str) -> String {
    let cleaned = target_folder_name(component);
    if cleaned.trim().is_empty() {
        "_".to_string()
    } else {
        cleaned
    }
}

/// The free space of the disk that holds `path` (or its first existing
/// parent: the destination is made by the run), bytes (`stacking::free_bytes`,
/// `None` where it cannot be read).
#[must_use]
pub fn free_bytes(path: &Path) -> Option<u64> {
    let existing = path.ancestors().find(|p| p.exists())?;
    crate::stacking::free_bytes(existing)
}

/// What names a night's folder, `YYYY-MM-DD · <place> · <id court>`
/// (`working_folder::night_folder_name`, the rule of the working folder): the
/// place, and the session on the service once created.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NightNaming {
    #[serde(default)]
    pub place: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    /// The night's folder itself, when it already exists (the working
    /// folder's night of the session, for the treatments).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<PathBuf>,
}

/// The folder of one night: its place (given, else the place chosen in the
/// proposal) and the short id of its session on the service (else of the
/// detected session, by its digest).
fn night_folder(
    proposal: &Proposal,
    session: &super::DetectedSession,
    naming: Option<&NightNaming>,
) -> String {
    let place = naming
        .and_then(|n| n.place.clone())
        .or_else(|| {
            let place = proposal.places.iter().find(|p| p.id == session.place)?;
            let chosen = place.chosen.as_ref()?;
            place
                .candidates
                .iter()
                .find(|c| &c.place_id == chosen)
                .map(|c| c.name.clone())
        })
        .unwrap_or_default();
    let id = naming
        .and_then(|n| n.session_id.clone())
        .unwrap_or_else(|| {
            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(session.id.as_bytes());
            digest.iter().take(3).map(|b| format!("{b:02x}")).collect()
        });
    night_folder_name(&session.evening_date, &place, &id)
        .unwrap_or_else(|_| session.evening_date.clone())
}

/// The dry run: every file of the sessions kept (`sessions`; empty: all)
/// with its place in the tree under `destination`. Nothing is written.
pub fn plan_organise(
    proposal: &Proposal,
    destination: &Path,
    mode: OrganiseMode,
    sessions: &[String],
) -> Result<OrganisePlan, OrganiseError> {
    plan_organise_named(proposal, destination, mode, sessions, &BTreeMap::new())
}

/// [`plan_organise`] with the names of the nights' folders (`names`, by
/// detected session id): the result page gives the sessions created.
pub fn plan_organise_named(
    proposal: &Proposal,
    destination: &Path,
    mode: OrganiseMode,
    sessions: &[String],
    names: &BTreeMap<String, NightNaming>,
) -> Result<OrganisePlan, OrganiseError> {
    let mut plan = OrganisePlan {
        mode,
        destination: destination.display().to_string(),
        windows_no_link: cfg!(windows) && mode == OrganiseMode::Symlink,
        ..OrganisePlan::default()
    };
    if mode == OrganiseMode::LeaveInPlace {
        plan.summary = "The files stay where they are: Astro Sync reads them in place.".into();
        return Ok(plan);
    }
    let kept: Vec<_> = proposal
        .sessions
        .iter()
        .filter(|s| sessions.is_empty() || sessions.contains(&s.id))
        .collect();
    // Every (source, destination), in a stable order.
    let mut wanted: Vec<(String, PathBuf)> = Vec::new();
    for session in &kept {
        let naming = names.get(&session.id);
        let night_dir = match naming.and_then(|n| n.folder.clone()) {
            Some(folder) => folder,
            None => destination.join(night_folder(proposal, session, naming)),
        };
        for target in &session.targets {
            let lights = night_dir.join(clean(&target.designation)).join("lights");
            for (index, file) in target.files.iter().enumerate() {
                let dir = match target.file_filters.get(index).filter(|f| !f.is_empty()) {
                    Some(filter) => lights.join(clean(filter)),
                    None => lights.clone(),
                };
                wanted.push((file.clone(), dir.join(file_name(file))));
            }
            // What the device stacked itself: its own folder of the target,
            // no filter level (the file's name says the filter).
            let device = night_dir
                .join(clean(&target.designation))
                .join(crate::working_folder::DEVICE_DIR);
            for file in &target.device_images {
                wanted.push((file.clone(), device.join(file_name(file))));
            }
        }
        for attached in &session.calibration {
            let Some(set) = proposal
                .calibration_sets
                .iter()
                .find(|s| s.id == attached.set_id)
            else {
                continue;
            };
            let mut dir = night_dir
                .join("calibration")
                .join(calibration_folder(attached.kind));
            // The flats of a filter stay together, one folder per filter
            // (like the lights): the stacking of a filter takes its own.
            if attached.kind == FrameType::Flat
                && let Some(filter) = set.filter.as_deref().filter(|f| !f.trim().is_empty())
            {
                dir = dir.join(clean(filter));
            }
            for file in &set.files {
                wanted.push((file.clone(), dir.join(file_name(file))));
            }
        }
        for attached in &session.logs {
            if let Some(log) = proposal.logs.iter().find(|l| l.id == attached.log_id) {
                wanted.push((
                    log.path.clone(),
                    night_dir.join("logs").join(file_name(&log.path)),
                ));
            }
        }
    }

    let mut taken: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut seen: BTreeSet<(String, PathBuf)> = BTreeSet::new();
    for (source, target) in wanted {
        if !seen.insert((source.clone(), target.clone())) {
            continue;
        }
        let conflict = |reason: &str| OrganiseFailure {
            source: source.clone(),
            destination: target.display().to_string(),
            reason: reason.to_string(),
            detail: None,
        };
        if let Some(other) = taken.get(&target)
            && other != &source
        {
            plan.conflicts.push(conflict("same-name"));
            continue;
        }
        taken.insert(target.clone(), source.clone());
        let bytes = fs::metadata(&source).map(|m| m.len()).unwrap_or(0);
        let kind = match fs::symlink_metadata(&target) {
            Ok(_) if same_file(&source, &target) => OrganiseActionKind::AlreadyThere,
            Ok(_) => {
                plan.conflicts.push(conflict("destination-exists"));
                continue;
            }
            Err(_) => match mode {
                OrganiseMode::Copy => OrganiseActionKind::Copy,
                OrganiseMode::Move => OrganiseActionKind::Move,
                OrganiseMode::Symlink => OrganiseActionKind::Link,
                OrganiseMode::LeaveInPlace => unreachable!("returned above"),
            },
        };
        plan.bytes += bytes;
        let in_place_as = (kind == OrganiseActionKind::AlreadyThere).then(|| {
            if fs::symlink_metadata(&target).is_ok_and(|m| m.file_type().is_symlink()) {
                InPlaceAs::Link
            } else {
                InPlaceAs::File
            }
        });
        plan.actions.push(OrganiseAction {
            source,
            destination: target.display().to_string(),
            kind,
            bytes,
            in_place_as,
        });
    }
    plan.files = plan.actions.len();
    plan.free_bytes = free_bytes(destination);
    let gigabytes = plan.bytes as f64 / 1e9;
    let size = format!("{gigabytes:.1} GB");
    plan.summary = match mode {
        OrganiseMode::Copy => format!(
            "Copy {} files ({size}) to {}; the originals are not touched.",
            plan.files, plan.destination
        ),
        OrganiseMode::Move => format!(
            "Move {} files ({size}) to {}: each file is copied, checked by its digest (SHA-256), and only then deleted from the source.",
            plan.files, plan.destination
        ),
        OrganiseMode::Symlink => format!(
            "Create {} links to the originals in {}; the originals are not touched.",
            plan.files, plan.destination
        ),
        OrganiseMode::LeaveInPlace => unreachable!("returned above"),
    };
    if !plan.conflicts.is_empty() {
        plan.summary.push_str(&format!(
            " {} files stay in place (another file already has their name).",
            plan.conflicts.len()
        ));
    }
    Ok(plan)
}

/// The same file: a link to it, or the same size and SHA-256.
fn same_file(source: &str, target: &Path) -> bool {
    if let (Ok(a), Ok(b)) = (fs::canonicalize(source), fs::canonicalize(target))
        && a == b
    {
        return true;
    }
    let (Ok(a), Ok(b)) = (fs::metadata(source), fs::metadata(target)) else {
        return false;
    };
    a.len() == b.len()
        && matches!(
            (file_sha256(Path::new(source)), file_sha256(target)),
            (Ok(x), Ok(y)) if x == y
        )
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> OrganiseError + '_ {
    move |source| OrganiseError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(unix)]
fn make_link(source: &Path, target: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, target)
}

/// Windows: a symbolic link when the person may make one (developer mode,
/// administrator), else a hard link (same drive only): no extra disk, and as
/// read-only for Astro Sync as a symbolic link, which never writes into the
/// frame either.
#[cfg(windows)]
fn make_link(source: &Path, target: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(source, target).or_else(|_| fs::hard_link(source, target))
}

/// The drive of a path (`C:`, `\\server\share`), read from its prefix.
#[cfg(windows)]
fn volume_label(path: &Path) -> Option<String> {
    use std::path::{Component, Prefix};
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return None;
    };
    Some(match prefix.kind() {
        Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
            format!("{}:", char::from(letter).to_ascii_uppercase())
        }
        _ => prefix.as_os_str().to_string_lossy().to_uppercase(),
    })
}

#[cfg(windows)]
fn volume_of(path: &Path) -> String {
    volume_label(path)
        .or_else(|| fs::canonicalize(path).ok().and_then(|p| volume_label(&p)))
        .unwrap_or_default()
}

/// Windows, before a run that links: a symbolic link when allowed; else hard
/// links, tried on a small file, and the frames must be on the destination's
/// drive (`LinkAcrossVolumes`; never a silent copy, it would double the disk
/// use).
#[cfg(windows)]
fn probe_links(dir: &Path, plan: &OrganisePlan) -> Result<(), OrganiseError> {
    let probe = dir.join(".astro-sync-link-probe");
    let _ = fs::remove_file(&probe);
    if std::os::windows::fs::symlink_file(dir, &probe).is_ok() {
        let _ = fs::remove_file(&probe);
        return Ok(());
    }
    let probe_source = dir.join(".astro-sync-link-probe-source");
    fs::write(&probe_source, b"").map_err(io(&probe_source))?;
    let hard = fs::hard_link(&probe_source, &probe);
    let _ = fs::remove_file(&probe);
    let _ = fs::remove_file(&probe_source);
    hard.map_err(|_| OrganiseError::SymlinkUnavailable)?;
    let destination_volume = volume_of(dir);
    let mut seen = BTreeSet::new();
    for action in &plan.actions {
        if action.kind != OrganiseActionKind::Link {
            continue;
        }
        let Some(parent) = Path::new(&action.source).parent() else {
            continue;
        };
        if !seen.insert(parent) {
            continue;
        }
        let source_volume = volume_of(parent);
        if source_volume != destination_volume {
            return Err(OrganiseError::LinkAcrossVolumes {
                source_volume,
                destination_volume,
            });
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn probe_links(dir: &Path, _plan: &OrganisePlan) -> Result<(), OrganiseError> {
    let probe = dir.join(".astro-sync-link-probe");
    let _ = fs::remove_file(&probe);
    if make_link(dir, &probe).is_err() {
        return Err(OrganiseError::SymlinkUnavailable);
    }
    let _ = fs::remove_file(&probe);
    Ok(())
}

/// Copies beside the place, then renames: never half a file at its place.
fn copy_whole(source: &Path, target: &Path) -> std::io::Result<()> {
    let partial = target.with_extension(format!(
        "{}.partial",
        target.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    fs::copy(source, &partial)?;
    fs::rename(&partial, target)
}

/// Runs a plan, file by file (see the module's documentation).
pub fn organise(
    plan: &OrganisePlan,
    options: &OrganiseOptions<'_>,
) -> Result<OrganiseOutcome, OrganiseError> {
    let mut outcome = OrganiseOutcome::default();
    if plan.mode == OrganiseMode::LeaveInPlace {
        return Ok(outcome);
    }
    if plan.mode == OrganiseMode::Symlink {
        // A probe, before anything: Windows refuses symbolic links without
        // developer mode (hard links then, on one drive).
        let dir = Path::new(&plan.destination);
        fs::create_dir_all(dir).map_err(io(dir))?;
        probe_links(dir, plan)?;
    }
    // A moved source is removed once all its copies are checked (shared
    // calibration goes to several nights).
    let mut remaining: BTreeMap<&str, usize> = BTreeMap::new();
    for action in &plan.actions {
        if action.kind == OrganiseActionKind::Move {
            *remaining.entry(action.source.as_str()).or_default() += 1;
        }
    }
    let mut digests: BTreeMap<&str, String> = BTreeMap::new();
    let total = plan.actions.len() as u64;
    let mut bytes_done = 0;
    for (index, action) in plan.actions.iter().enumerate() {
        if options.cancel.is_some_and(CancelFlag::is_cancelled) {
            outcome.stopped = true;
            break;
        }
        if let Some(progress) = options.progress {
            progress(&WorkProgress {
                kind: WorkKind::Organise,
                done: index as u64,
                total,
                bytes_done,
                bytes_total: plan.bytes,
                current: Some(action.destination.clone()),
            });
        }
        bytes_done += action.bytes;
        let source = Path::new(&action.source);
        let target = Path::new(&action.destination);
        let failure = |reason: &str, detail: Option<String>| OrganiseFailure {
            source: action.source.clone(),
            destination: action.destination.clone(),
            reason: reason.to_string(),
            detail,
        };
        if action.kind == OrganiseActionKind::AlreadyThere {
            outcome.already_there += 1;
            continue;
        }
        if fs::symlink_metadata(target).is_ok() {
            outcome.failed.push(failure("destination-exists", None));
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(io(parent))?;
        }
        let done = match action.kind {
            OrganiseActionKind::Copy => copy_whole(source, target),
            OrganiseActionKind::Link => make_link(
                &fs::canonicalize(source).unwrap_or(source.to_path_buf()),
                target,
            ),
            OrganiseActionKind::Move => {
                let expected = match digests.get(action.source.as_str()) {
                    Some(d) => d.clone(),
                    None => match file_sha256(source) {
                        Ok(d) => {
                            digests.insert(action.source.as_str(), d.clone());
                            d
                        }
                        Err(e) => {
                            outcome.failed.push(failure("io", Some(e.to_string())));
                            outcome.stopped = true;
                            break;
                        }
                    },
                };
                if let Err(e) = copy_whole(source, target) {
                    outcome.failed.push(failure("io", Some(e.to_string())));
                    outcome.stopped = true;
                    break;
                }
                match file_sha256(target) {
                    Ok(copied) if copied == expected => outcome.verified += 1,
                    other => {
                        // The copy is not the file: it is removed, the
                        // source stays, and the run stops here.
                        let _ = fs::remove_file(target);
                        outcome.failed.push(failure(
                            "checksum-differs",
                            other.err().map(|e| e.to_string()),
                        ));
                        outcome.stopped = true;
                        break;
                    }
                }
                let left = remaining
                    .get_mut(action.source.as_str())
                    .expect("counted above");
                *left -= 1;
                if *left == 0 {
                    fs::remove_file(source).map_err(io(source))?;
                    outcome.removed_sources += 1;
                }
                Ok(())
            }
            OrganiseActionKind::AlreadyThere => unreachable!("skipped above"),
        };
        match done {
            Ok(()) => outcome.done += 1,
            Err(e) => outcome.failed.push(failure("io", Some(e.to_string()))),
        }
    }
    Ok(outcome)
}

#[cfg(all(test, windows))]
mod windows_links {
    use super::*;
    use crate::test_support::TempDir;

    fn link_action(source: &Path, destination: &Path) -> OrganiseAction {
        OrganiseAction {
            source: source.display().to_string(),
            destination: destination.display().to_string(),
            kind: OrganiseActionKind::Link,
            bytes: 5,
        }
    }

    fn symlinks_allowed(dir: &Path) -> bool {
        let probe = dir.join("symlink-allowed-probe");
        let ok = std::os::windows::fs::symlink_file(dir, &probe).is_ok();
        let _ = fs::remove_file(&probe);
        ok
    }

    #[test]
    fn a_volume_is_read_from_the_prefix_of_the_path() {
        assert_eq!(volume_label(Path::new(r"c:\a\b")).as_deref(), Some("C:"));
        assert_eq!(volume_label(Path::new(r"\\?\D:\a")).as_deref(), Some("D:"));
        assert_eq!(volume_label(Path::new(r"a\b")), None);
    }

    #[test]
    fn the_probe_links_a_small_file_through_a_hard_link_on_one_drive() {
        let tree = TempDir::new("windows-probe");
        let dir = tree.path().join("work");
        fs::create_dir_all(&dir).unwrap();
        let plan = OrganisePlan {
            mode: OrganiseMode::Symlink,
            destination: dir.display().to_string(),
            ..OrganisePlan::default()
        };
        probe_links(&dir, &plan).expect("a symbolic link or a hard link is possible");
        assert!(!dir.join(".astro-sync-link-probe").exists());
        assert!(!dir.join(".astro-sync-link-probe-source").exists());
    }

    #[test]
    fn frames_on_another_drive_stop_the_run_when_only_hard_links_are_possible() {
        let tree = TempDir::new("windows-volumes");
        let dir = tree.path().join("work");
        fs::create_dir_all(&dir).unwrap();
        if symlinks_allowed(&dir) {
            return; // developer mode: nothing to refuse
        }
        let other = if volume_of(&dir) == "Z:" {
            r"Y:\"
        } else {
            r"Z:\"
        };
        let plan = OrganisePlan {
            mode: OrganiseMode::Symlink,
            destination: dir.display().to_string(),
            actions: vec![link_action(
                &Path::new(other).join("frames").join("l0.fit"),
                &dir.join("night").join("l0.fit"),
            )],
            ..OrganisePlan::default()
        };
        assert!(matches!(
            organise(&plan, &OrganiseOptions::default()),
            Err(OrganiseError::LinkAcrossVolumes { .. })
        ));
        assert!(!dir.join("night").exists(), "nothing is touched");
    }

    #[test]
    fn organising_in_link_mode_links_the_frame_without_writing_into_it() {
        let tree = TempDir::new("windows-link");
        let source = tree.path().join("frames").join("l0.fit");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"frame").unwrap();
        let before = fs::metadata(&source).unwrap().modified().unwrap();
        let destination = tree.path().join("work");
        let target = destination.join("night").join("lights").join("l0.fit");
        let plan = OrganisePlan {
            mode: OrganiseMode::Symlink,
            destination: destination.display().to_string(),
            actions: vec![link_action(&source, &target)],
            ..OrganisePlan::default()
        };
        let outcome = organise(&plan, &OrganiseOptions::default()).unwrap();
        assert_eq!((outcome.done, outcome.failed.len()), (1, 0));
        assert_eq!(fs::read(&target).unwrap(), b"frame");
        assert_eq!(fs::metadata(&source).unwrap().modified().unwrap(), before);
        if !symlinks_allowed(&destination) {
            // A hard link: a plain file that is the frame itself (a write
            // through the source shows in the link; the run never makes one).
            assert!(!fs::symlink_metadata(&target).unwrap().is_symlink());
            fs::write(&source, b"frame, again").unwrap();
            assert_eq!(fs::read(&target).unwrap(), b"frame, again");
        }
    }
}

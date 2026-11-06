//! The working folder of Astro Sync: one imposed root per system
//! (`Documents/Astro Sync/`), one folder per night named
//! `YYYY-MM-DD · <plan, or place outside a plan> · <id court>`, created at
//! the wizard's "Folder" step with `logs/`, `calibration/{darks,flats,
//! dark-flats,biases}/` and, for the ticked targets only, `<target>/lights/`
//! and `<target>/final/`. The folder names are English in every language.
//!
//! **Identity, not names.** A night folder carries `.astro-sync-night.json`
//! and each target folder `.astro-sync-target.json` (hidden on Windows):
//! Astro Sync finds a night by reading the root's direct subfolders' identity
//! files, never by name, so a renaming breaks nothing. Two folders carrying
//! the same night (a copy) are said, and the person chooses which one to
//! follow; a folder moved out of the root is found again by "Find the
//! folder again", which checks the chosen folder carries the same night.
//!
//! **What this module writes** -- and the whole of what Astro Sync writes in
//! the person's tree besides `ASSISTANT-IA.md` and the stacked images
//! (`crate`'s module doc): empty folders, and its two identity files,
//! created only when absent (`create_new`). It never moves, renames,
//! replaces or deletes a file or a folder. A target unticked later keeps its
//! folder, said "outside the night".
//!
//! **Provisional nights**. The import of sessions writes nothing to the space
//! before its summary step: its treatments run on night folders of their own,
//! carrying a provisional identifier ([`PROVISIONAL_PREFIX`],
//! `YYYY-MM-DD · <place> · local-<hash>`) and the detection's proposal for
//! the night ([`PendingNight`], `pendingSessionId: null`). Once the session
//! is created, Astro Sync renames that folder (its own, made of links and of
//! its results) to the session's name and rewrites its record
//! ([`write_night_identity`], atomic): the only folder it renames, the only
//! record it rewrites (`crate::session_detection::provisional`).

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::nfc::{same_name, to_nfc};

/// The root's folder name, inside the documents folder of the system.
pub const ROOT_FOLDER_NAME: &str = "Astro Sync";
/// The identity file of a night folder.
pub const NIGHT_IDENTITY_FILE: &str = ".astro-sync-night.json";
/// The identity file of a target folder.
pub const TARGET_IDENTITY_FILE: &str = ".astro-sync-target.json";
/// The hidden folder of a night folder where Astro Sync keeps its temporary
/// work (the stacking's scratch): created for a job and removed at its end,
/// ignored by every reading of the night (the scanner skips
/// dot-directories), never sent.
pub const WORK_DIR: &str = ".astro-sync-work";
/// `<night>/.astro-sync-work/stacking/`: one scratch folder per stacking.
pub const STACKING_SCRATCH_DIR: &str = "stacking";
/// `<night>/.astro-sync-work/progression/`: the scratch of the progression
/// images, one folder per target and family.
pub const PROGRESSION_SCRATCH_DIR: &str = "progression";
/// `<night>/.astro-sync-work/solve/`: the scratch of the plate solving, one
/// folder per image solved.
pub const SOLVE_SCRATCH_DIR: &str = "solve";
/// The file for an AI assistant, at the root (written by
/// `crate::assistant_prompt`).
pub const ASSISTANT_FILE_NAME: &str = "ASSISTANT-IA.md";
/// The version of both identity files.
pub const IDENTITY_FORMAT: u32 = 1;

pub const LOGS_DIR: &str = "logs";
pub const CALIBRATION_DIR: &str = "calibration";
pub const LIGHTS_DIR: &str = "lights";
pub const FINAL_DIR: &str = "final";
/// Where the image a device already stacked (the Seestar's `Stacked_*.fit`)
/// lies in a target's folder; never created in advance
/// ("Tidy the files" makes it when it has such an image to put there).
pub const DEVICE_DIR: &str = "device";
/// Where Astro Sync writes a target's stacked image; never created in
/// advance.
pub const STACKED_DIR: &str = "stacked";
/// Where Astro Sync writes a target's progression images, the JPEG of each
/// milestone (`milestone_01.jpg`…): a result of Astro Sync like `stacked/`,
/// small, kept after the sending; never created in advance.
pub const PROGRESSION_DIR: &str = "progression";

/// Where the stackings of `night_folder` keep their scratch: on the disk of
/// the night, never in Astro Sync's data.
#[must_use]
pub fn stacking_scratch_root(night_folder: &Path) -> PathBuf {
    night_folder.join(WORK_DIR).join(STACKING_SCRATCH_DIR)
}

/// Where the progression images of `night_folder` are made.
#[must_use]
pub fn progression_scratch_root(night_folder: &Path) -> PathBuf {
    night_folder.join(WORK_DIR).join(PROGRESSION_SCRATCH_DIR)
}

/// Where the plate solving of an image of `night_folder` works.
#[must_use]
pub fn solve_scratch_root(night_folder: &Path) -> PathBuf {
    night_folder.join(WORK_DIR).join(SOLVE_SCRATCH_DIR)
}

/// The night folder `path` lies in: its nearest ancestor holding the
/// night's identity file ([`NIGHT_IDENTITY_FILE`]), or `None`.
#[must_use]
pub fn night_folder_of(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .skip(1)
        .find(|folder| folder.join(NIGHT_IDENTITY_FILE).is_file())
        .map(Path::to_path_buf)
}

/// Removes `scratch` (a folder of Astro Sync's own work) and then each of
/// its parents up to and including `<night>/.astro-sync-work/` while they
/// are empty. Does nothing unless `scratch` lies inside a
/// `.astro-sync-work/` folder: it is the only thing Astro Sync removes in
/// the person's tree.
pub fn remove_scratch(scratch: &Path) {
    let Some(work) = scratch
        .ancestors()
        .find(|folder| folder.file_name().is_some_and(|name| name == WORK_DIR))
    else {
        return;
    };
    let _ = std::fs::remove_dir_all(scratch);
    for parent in scratch.ancestors().skip(1) {
        if !parent.starts_with(work) || std::fs::remove_dir(parent).is_err() {
            break;
        }
    }
}

/// The calibration kinds and their folder, in the order of the sheet.
pub const CALIBRATION_KINDS: [&str; 4] = ["darks", "flats", "dark-flats", "biases"];
/// The other name a folder of biases may carry: `bias/`, the one "Tidy
/// the files" writes. Every reader takes
/// both names.
pub const BIAS_ALIAS: &str = "bias";

/// The middle part of a night's name is cut at this many characters.
pub const MIDDLE_PART_MAX_CHARS: usize = 60;
/// The short identifier: the first characters of the night's identifier.
pub const SHORT_ID_CHARS: usize = 6;
/// The prefix of a provisional identifier: a night, or a multi-night plan,
/// the import prepared on this computer before the space created it
/// (`local-<hash>`).
pub const PROVISIONAL_PREFIX: &str = "local-";

/// Whether `id` is a provisional identifier ([`PROVISIONAL_PREFIX`]).
#[must_use]
pub fn is_provisional(id: &str) -> bool {
    id.trim().starts_with(PROVISIONAL_PREFIX)
}

/// The short identifier a folder's name ends with: the first
/// [`SHORT_ID_CHARS`] characters of the identifier, or `local-` and the first
/// [`SHORT_ID_CHARS`] characters of its hash for a provisional one.
#[must_use]
pub fn short_id(id: &str) -> String {
    let id = id.trim();
    match id.strip_prefix(PROVISIONAL_PREFIX) {
        Some(hash) => format!(
            "{PROVISIONAL_PREFIX}{}",
            hash.chars().take(SHORT_ID_CHARS).collect::<String>()
        ),
        None => id.chars().take(SHORT_ID_CHARS).collect(),
    }
}
/// The separator of a night's name (U+00B7, allowed on every system).
pub const NAME_SEPARATOR: &str = " · ";

const WINDOWS_RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

#[derive(Debug, Error)]
pub enum WorkingFolderError {
    #[error("io error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("no documents folder could be found for the working folder")]
    NoDocumentsFolder,
    #[error("the working folder must be an existing folder: {0}")]
    RootNotADirectory(PathBuf),
    #[error("the evening date must read AAAA-MM-JJ, got {0:?}")]
    BadEveningDate(String),
    #[error("the night identifier must not be empty")]
    EmptyNightId,
    /// Two folders carry this night: the person chooses which to follow.
    #[error("two folders carry the night {night_id}")]
    DuplicateNight {
        night_id: String,
        folders: Vec<PathBuf>,
    },
    /// "Find the folder again": the chosen folder carries no night.
    #[error("the folder {0} is not a night folder of Astro Sync")]
    NoNightIdentity(PathBuf),
    /// "Find the folder again": the chosen folder carries another night.
    #[error("the folder {folder} carries the night {found}, not {expected}")]
    OtherNight {
        folder: PathBuf,
        expected: String,
        found: String,
    },
}

impl WorkingFolderError {
    /// The stable code an interface looks up as `errors.<code>`.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            WorkingFolderError::Io { .. } => "io_error",
            WorkingFolderError::NoDocumentsFolder => "no_documents_folder",
            WorkingFolderError::RootNotADirectory(_) => "folder_not_found",
            WorkingFolderError::BadEveningDate(_) => "bad_evening_date",
            WorkingFolderError::EmptyNightId => "empty_session_id",
            WorkingFolderError::DuplicateNight { .. } => "night_folder_duplicated",
            WorkingFolderError::NoNightIdentity(_) => "night_folder_without_identity",
            WorkingFolderError::OtherNight { .. } => "night_folder_other_night",
        }
    }
}

fn io_at(path: &Path) -> impl FnOnce(io::Error) -> WorkingFolderError + '_ {
    move |source| WorkingFolderError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/* ------------------------------- The root ------------------------------- */

/// The system the default root is computed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostSystem {
    MacOs,
    Windows,
    Linux,
}

impl HostSystem {
    #[must_use]
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            HostSystem::Windows
        } else if cfg!(target_os = "macos") {
            HostSystem::MacOs
        } else {
            HostSystem::Linux
        }
    }
}

/// What the default root depends on, gathered once so the rule is a pure
/// function a test can run for every system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootEnvironment {
    pub system: HostSystem,
    pub home: Option<PathBuf>,
    /// Linux: the `XDG_DOCUMENTS_DIR` environment variable, when set.
    pub xdg_documents_dir: Option<String>,
    /// Linux: the contents of `user-dirs.dirs` (what `xdg-user-dir
    /// DOCUMENTS` reads).
    pub xdg_user_dirs: Option<String>,
    /// Windows: the known folder `FOLDERID_Documents` (possibly redirected
    /// to OneDrive).
    pub windows_documents: Option<PathBuf>,
}

impl RootEnvironment {
    /// The environment of this computer.
    #[must_use]
    pub fn current() -> Self {
        let system = HostSystem::current();
        let home = std::env::var_os(if system == HostSystem::Windows {
            "USERPROFILE"
        } else {
            "HOME"
        })
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
        let xdg_user_dirs = if system == HostSystem::Linux {
            let config = std::env::var_os("XDG_CONFIG_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .or_else(|| home.as_ref().map(|home| home.join(".config")));
            config.and_then(|config| fs::read_to_string(config.join("user-dirs.dirs")).ok())
        } else {
            None
        };
        Self {
            system,
            home,
            xdg_documents_dir: std::env::var("XDG_DOCUMENTS_DIR")
                .ok()
                .filter(|value| !value.is_empty()),
            xdg_user_dirs,
            windows_documents: windows_documents_folder(),
        }
    }
}

/// The default root for `env`: `~/Documents/Astro Sync/` on macOS, the
/// known folder Documents of the profile on Windows, `$XDG_DOCUMENTS_DIR`
/// (else `~/Documents`) on Linux. `None` when not even a home is known.
#[must_use]
pub fn default_root(env: &RootEnvironment) -> Option<PathBuf> {
    let documents = match env.system {
        HostSystem::MacOs => env.home.as_ref().map(|home| home.join("Documents")),
        HostSystem::Windows => env
            .windows_documents
            .clone()
            .or_else(|| env.home.as_ref().map(|home| home.join("Documents"))),
        HostSystem::Linux => linux_documents(env),
    }?;
    Some(documents.join(ROOT_FOLDER_NAME))
}

fn linux_documents(env: &RootEnvironment) -> Option<PathBuf> {
    let expand = |raw: &str| -> Option<PathBuf> {
        let raw = raw.trim().trim_matches('"');
        let path = match (raw.strip_prefix("$HOME"), env.home.as_ref()) {
            (Some(rest), Some(home)) => home.join(rest.trim_start_matches('/')),
            (Some(_), None) => return None,
            (None, _) => PathBuf::from(raw),
        };
        // The XDG rule: a documents folder equal to the home is « disabled ».
        (path.is_absolute() && Some(&path) != env.home.as_ref()).then_some(path)
    };
    if let Some(found) = env.xdg_documents_dir.as_deref().and_then(expand) {
        return Some(found);
    }
    if let Some(found) = env.xdg_user_dirs.as_deref().and_then(|contents| {
        contents
            .lines()
            .map(str::trim)
            .filter(|line| !line.starts_with('#'))
            .find_map(|line| line.strip_prefix("XDG_DOCUMENTS_DIR="))
            .and_then(expand)
    }) {
        return Some(found);
    }
    env.home.as_ref().map(|home| home.join("Documents"))
}

#[cfg(windows)]
fn windows_documents_folder() -> Option<PathBuf> {
    use std::ffi::{OsString, c_void};
    use std::os::windows::ffi::OsStringExt;

    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }
    // FOLDERID_Documents {FDD39AD0-238F-46AF-ADB4-6C85480369C7}.
    const FOLDERID_DOCUMENTS: Guid = Guid {
        data1: 0xFDD3_9AD0,
        data2: 0x238F,
        data3: 0x46AF,
        data4: [0xAD, 0xB4, 0x6C, 0x85, 0x48, 0x03, 0x69, 0xC7],
    };
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHGetKnownFolderPath(
            rfid: *const Guid,
            flags: u32,
            token: *mut c_void,
            path: *mut *mut u16,
        ) -> i32;
    }
    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoTaskMemFree(pv: *mut c_void);
    }

    let mut raw: *mut u16 = std::ptr::null_mut();
    // SAFETY: the documented call; `raw` receives a buffer the system
    // allocates, freed below with `CoTaskMemFree` whatever the result.
    let result =
        unsafe { SHGetKnownFolderPath(&FOLDERID_DOCUMENTS, 0, std::ptr::null_mut(), &mut raw) };
    let path = if result == 0 && !raw.is_null() {
        let mut length = 0;
        // SAFETY: the buffer is a NUL-terminated UTF-16 string.
        while unsafe { *raw.add(length) } != 0 {
            length += 1;
        }
        // SAFETY: `length` units were just read.
        let wide = unsafe { std::slice::from_raw_parts(raw, length) };
        Some(PathBuf::from(OsString::from_wide(wide)))
    } else {
        None
    };
    if !raw.is_null() {
        // SAFETY: allocated by `SHGetKnownFolderPath`.
        unsafe { CoTaskMemFree(raw.cast()) };
    }
    path
}

#[cfg(not(windows))]
fn windows_documents_folder() -> Option<PathBuf> {
    None
}

/// Creates the root if it does not exist yet (at the first signed-in
/// launch). Returns whether it was created. Nothing else is written here.
pub fn ensure_root(root: &Path) -> Result<bool, WorkingFolderError> {
    if root.is_dir() {
        return Ok(false);
    }
    fs::create_dir_all(root).map_err(io_at(root))?;
    Ok(true)
}

/// "Change folder" (Settings › General): the new root must be an
/// existing folder. Astro Sync then looks for the nights there; it moves
/// nothing from the old root.
pub fn check_new_root(root: &Path) -> Result<PathBuf, WorkingFolderError> {
    if !root.is_dir() {
        return Err(WorkingFolderError::RootNotADirectory(root.to_path_buf()));
    }
    Ok(root.to_path_buf())
}

/* -------------------------------- Names -------------------------------- */

/// One folder name made safe on every system (sheet § 2): NFC; `/ \ : * ?
/// " < > |` and control characters replaced by « - »; spaces and dots
/// removed at both ends; a Windows reserved name (`CON`, `PRN`, `AUX`,
/// `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`, with or without an extension)
/// suffixed with « - ». An empty result is « - ».
#[must_use]
pub fn clean_component(raw: &str) -> String {
    let replaced: String = to_nfc(raw)
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '-'
            } else {
                c
            }
        })
        .collect();
    let trimmed = replaced.trim_matches(|c: char| c == ' ' || c == '.');
    if trimmed.is_empty() {
        return "-".to_string();
    }
    let stem_end = trimmed.find('.').unwrap_or(trimmed.len());
    let stem = &trimmed[..stem_end];
    if WINDOWS_RESERVED
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
    {
        return format!("{stem}-{}", &trimmed[stem_end..]);
    }
    trimmed.to_string()
}

fn is_evening_date(date: &str) -> bool {
    let bytes = date.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
}

/// The name of a night's folder: `YYYY-MM-DD · <label> · <id court>`, the
/// label (the plan, or the place outside a plan) cleaned and cut at
/// [`MIDDLE_PART_MAX_CHARS`] characters, the short identifier the first
/// [`SHORT_ID_CHARS`] characters of the night's identifier. Without a label:
/// `YYYY-MM-DD · <id court>`.
pub fn night_folder_name(
    evening_date: &str,
    label: &str,
    night_id: &str,
) -> Result<String, WorkingFolderError> {
    if !is_evening_date(evening_date) {
        return Err(WorkingFolderError::BadEveningDate(evening_date.to_string()));
    }
    let short = short_id(night_id);
    if short.is_empty() {
        return Err(WorkingFolderError::EmptyNightId);
    }
    let short = clean_component(&short);
    let label = to_nfc(label.trim());
    if label.is_empty() {
        return Ok(format!("{evening_date}{NAME_SEPARATOR}{short}"));
    }
    let cut: String = clean_component(&label)
        .chars()
        .take(MIDDLE_PART_MAX_CHARS)
        .collect();
    let middle = cut.trim_end_matches([' ', '.']);
    let middle = if middle.is_empty() { "-" } else { middle };
    Ok(format!(
        "{evening_date}{NAME_SEPARATOR}{middle}{NAME_SEPARATOR}{short}"
    ))
}

/// The folder name of a target: its designation as the app writes it
/// (« M31 », « NGC 7000 »), cleaned by the same rules.
#[must_use]
pub fn target_folder_name(designation: &str) -> String {
    clean_component(designation)
}

/// The evening date of a night (`YYYY-MM-DD`, the "Night of 7 Oct" of the
/// portal) from its start: the date of the start in local solar time (the
/// place's longitude, never a time zone: the place rule), moved back by
/// twelve hours so a start after midnight still belongs to the evening
/// before. Without a longitude, UTC. `None` for an unreadable start.
#[must_use]
pub fn evening_date_of(started_at_utc: &str, longitude: Option<f64>) -> Option<String> {
    let seconds = crate::time_util::parse_iso8601_utc_to_unix_seconds(started_at_utc)?;
    let solar = seconds + longitude.unwrap_or(0.0) / 15.0 * 3600.0 - 12.0 * 3600.0;
    let days = (solar / 86_400.0).floor() as i64;
    let (year, month, day) = crate::time_util::civil_from_days(days);
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

/* ------------------------------- Identity ------------------------------- */

/// `.astro-sync-night.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NightIdentity {
    pub format: u32,
    pub night_id: String,
    pub service_url: String,
    pub created_at: String,
    /// A night the import prepared before its session exists: `night_id`
    /// is provisional, this is what the detection proposed for it.
    /// Absent from every other night.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<PendingNight>,
}

/// What the detection proposed for a night the import prepared before its
/// session exists: the night, its targets, its equipment, its calibration and
/// its logs, and the session once created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingNight {
    /// The session the space created for this night: `null` until the
    /// « Récapitulatif » of the import creates it; set before the folder is
    /// renamed, so that a start of the application finishes a rename the last
    /// one left half done.
    pub pending_session_id: Option<String>,
    /// The folder as it was named when the session was created (set with
    /// `pending_session_id`): the references to it are rewritten from there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<PathBuf>,
    /// The proposal's id of the night (`DetectedSession::id`).
    pub detected_id: String,
    pub evening_date: String,
    /// The place's name the review chose (may be empty).
    pub place: String,
    pub targets: Vec<PendingTarget>,
    /// « Mon matériel » ids the night's frames matched.
    pub equipment_ids: Vec<String>,
    /// The folders of the calibration sets attached to the night.
    pub calibration: Vec<String>,
    /// The log files attached to the night.
    pub logs: Vec<String>,
}

/// A target of a provisional night.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingTarget {
    pub target_id: Option<String>,
    pub designation: String,
    pub frames: usize,
}

/// Rewrites a night's record (a provisional night, once its session is
/// created): beside it, then renamed over it, so that the record is always
/// whole; hidden on Windows.
pub fn write_night_identity(
    folder: &Path,
    identity: &NightIdentity,
) -> Result<(), WorkingFolderError> {
    let path = folder.join(NIGHT_IDENTITY_FILE);
    let beside = folder.join(format!("{NIGHT_IDENTITY_FILE}.tmp"));
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_ATTRIBUTE_HIDDEN.
        options.attributes(0x2);
    }
    let mut file = options.open(&beside).map_err(io_at(&beside))?;
    file.write_all(identity_json(identity).as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(io_at(&beside))?;
    drop(file);
    fs::rename(&beside, &path).map_err(io_at(&path))
}

/// Renames the night folder `from` (a provisional one) to `wanted` under
/// its parent, or to `wanted (2)`… when that name is taken; nothing is
/// done when it is named so already. Returns where it is.
pub fn rename_night_folder(from: &Path, wanted: &str) -> Result<PathBuf, WorkingFolderError> {
    let parent = from.parent().unwrap_or(Path::new("."));
    let current = from.file_name().map(|n| n.to_string_lossy().into_owned());
    if current
        .as_deref()
        .is_some_and(|name| same_name(name, wanted))
    {
        return Ok(from.to_path_buf());
    }
    let name = free_name(parent, wanted, |_| false);
    let to = parent.join(name);
    fs::rename(from, &to).map_err(io_at(from))?;
    Ok(to)
}

/// `.astro-sync-target.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetIdentity {
    pub format: u32,
    pub target_id: String,
    /// The filter the lights were taken with, when their headers do not name
    /// one (an ASIAIR writes none): what the detection read from the folder
    /// names, or the person said. The stacking, the progression and the
    /// plans take it where a frame names no filter; the frames themselves are
    /// never touched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
}

/// The night a folder carries, if its identity file is there and readable.
#[must_use]
pub fn read_night_identity(folder: &Path) -> Option<NightIdentity> {
    let raw = fs::read_to_string(folder.join(NIGHT_IDENTITY_FILE)).ok()?;
    serde_json::from_str::<NightIdentity>(&raw)
        .ok()
        .filter(|identity| !identity.night_id.trim().is_empty())
}

/// The target a folder carries, if its identity file is there and readable.
#[must_use]
pub fn read_target_identity(folder: &Path) -> Option<TargetIdentity> {
    let raw = fs::read_to_string(folder.join(TARGET_IDENTITY_FILE)).ok()?;
    serde_json::from_str::<TargetIdentity>(&raw)
        .ok()
        .filter(|identity| !identity.target_id.trim().is_empty())
}

/// The filter a target folder records for lights whose headers name none
/// ([`TargetIdentity::filter`]).
#[must_use]
pub fn read_target_filter(folder: &Path) -> Option<String> {
    read_target_identity(folder)
        .and_then(|identity| identity.filter)
        .map(|filter| filter.trim().to_string())
        .filter(|filter| !filter.is_empty())
}

/// Records the filter of a target folder in its identity file (Astro Sync's
/// own file, never a frame): `Ok(true)` when it changed. A folder without an
/// identity is left as it is.
pub fn set_target_filter(folder: &Path, filter: &str) -> std::io::Result<bool> {
    let filter = filter.trim();
    let Some(mut identity) = read_target_identity(folder) else {
        return Ok(false);
    };
    if filter.is_empty() || identity.filter.as_deref() == Some(filter) {
        return Ok(false);
    }
    identity.filter = Some(filter.to_string());
    fs::write(folder.join(TARGET_IDENTITY_FILE), identity_json(&identity))?;
    Ok(true)
}

/// Writes an identity file only if none is there (`create_new`: an
/// existing file is never replaced); hidden on Windows. Returns whether it
/// was written.
fn write_identity_file(path: &Path, json: &str) -> Result<bool, WorkingFolderError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_ATTRIBUTE_HIDDEN.
        options.attributes(0x2);
    }
    match options.open(path) {
        Ok(mut file) => {
            file.write_all(json.as_bytes()).map_err(io_at(path))?;
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(io_at(path)(error)),
    }
}

/// A night folder found under the root by its identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NightFolderEntry {
    pub night_id: String,
    pub folder: PathBuf,
    /// The folder's current name (whatever the person renamed it to).
    pub name: String,
}

/// Every night folder directly under `root` (links to folders followed),
/// found by identity, sorted by name. A missing root is an empty list.
pub fn list_night_folders(root: &Path) -> Result<Vec<NightFolderEntry>, WorkingFolderError> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io_at(root)(error)),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.map_err(io_at(root))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if let Some(identity) = read_night_identity(&path) {
            found.push(NightFolderEntry {
                night_id: identity.night_id,
                name: entry.file_name().to_string_lossy().into_owned(),
                folder: path,
            });
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(found)
}

/// Where a night's folder is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum NightLocation {
    /// One folder carries the night.
    Found { folder: PathBuf },
    /// Several folders carry it (a copy): the page says so and asks which
    /// one to follow.
    Duplicated { folders: Vec<PathBuf> },
    /// No folder carries it: never created (the "Folder" step creates
    /// it), or moved out of the root ("Find the folder again").
    Missing,
}

/// Finds the folder of `night_id`: among the root's direct subfolders, and
/// among `remembered` folders (those found again by "Find the
/// folder again") that still carry it.
pub fn locate_night(
    root: &Path,
    remembered: &[PathBuf],
    night_id: &str,
) -> Result<NightLocation, WorkingFolderError> {
    let mut folders: Vec<PathBuf> = list_night_folders(root)?
        .into_iter()
        .filter(|entry| entry.night_id == night_id)
        .map(|entry| entry.folder)
        .collect();
    for folder in remembered {
        let carries = read_night_identity(folder).is_some_and(|i| i.night_id == night_id);
        let known = folders.iter().any(|f| same_folder(f, folder));
        if carries && !known {
            folders.push(folder.clone());
        }
    }
    Ok(match folders.len() {
        0 => NightLocation::Missing,
        1 => NightLocation::Found {
            folder: folders.remove(0),
        },
        _ => NightLocation::Duplicated { folders },
    })
}

/// Two folders carry the night and the person chose to follow one of them:
/// the night is then found in that one. Any other location is kept as it is
/// (a choice whose folder is no longer one of the copies is forgotten).
#[must_use]
pub fn apply_followed(location: NightLocation, followed: Option<&Path>) -> NightLocation {
    match (location, followed) {
        (NightLocation::Duplicated { folders }, Some(chosen))
            if folders.iter().any(|f| same_folder(f, chosen)) =>
        {
            NightLocation::Found {
                folder: chosen.to_path_buf(),
            }
        }
        (other, _) => other,
    }
}

fn same_folder(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// "Find the folder again": the chosen folder must carry `night_id`.
pub fn check_found_again(folder: &Path, night_id: &str) -> Result<PathBuf, WorkingFolderError> {
    match read_night_identity(folder) {
        None => Err(WorkingFolderError::NoNightIdentity(folder.to_path_buf())),
        Some(identity) if identity.night_id != night_id => Err(WorkingFolderError::OtherNight {
            folder: folder.to_path_buf(),
            expected: night_id.to_string(),
            found: identity.night_id,
        }),
        Some(_) => Ok(folder.to_path_buf()),
    }
}

/* ---------------------------- The "Folder" step ----------------------- */

/// A target of the night, as the "Targets" step left it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NightTargetSpec {
    pub target_id: String,
    /// « M31 », « NGC 7000 », or a personal target's name.
    pub designation: String,
    /// Ticked at "Targets": only a ticked target gets its folder.
    pub ticked: bool,
}

/// What the "Folder" step needs to know about a night.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NightFolderSpec {
    pub night_id: String,
    pub service_url: String,
    /// `YYYY-MM-DD`, the evening of the night ([`evening_date_of`]).
    pub evening_date: String,
    /// The plan's name, or the place's outside a plan; may be empty.
    pub label: String,
    pub targets: Vec<NightTargetSpec>,
    /// When two folders carry the night: the one the person chose to follow.
    #[serde(default)]
    pub follow: Option<PathBuf>,
}

/// The state of a target's folder after (or before) the "Folder" step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetFolderState {
    /// Created by this step (or to be, in a plan).
    Created,
    /// Already there, kept as it is.
    Existing,
    /// The folder of a target no longer ticked: kept, "outside the night".
    OutOfNight,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetFolder {
    pub target_id: String,
    pub designation: String,
    /// The folder's name in the night folder.
    pub folder_name: String,
    pub state: TargetFolderState,
}

/// What the "Folder" step does (or did) to the disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NightFolderOutcome {
    /// The night's folder.
    pub folder: PathBuf,
    /// The night's folder did not exist and is (to be) created.
    pub created: bool,
    /// Forward-slash paths, relative to the night folder, of every folder
    /// and identity file (to be) created, in creation order.
    pub created_paths: Vec<String>,
    pub targets: Vec<TargetFolder>,
    /// Applied to the disk (`false` for a dry run).
    pub applied: bool,
}

/// One step of the plan: a folder or an identity file.
#[derive(Debug, Clone)]
enum Creation {
    Folder(PathBuf),
    Identity(PathBuf, String),
}

/// Computes what [`prepare_night_folder`] would create, writing nothing
/// (`--dry-run` of the command line, and the preview of the step).
pub fn plan_night_folder(
    root: &Path,
    remembered: &[PathBuf],
    spec: &NightFolderSpec,
    now_utc: &str,
) -> Result<NightFolderOutcome, WorkingFolderError> {
    plan(root, remembered, spec, now_utc).map(|(outcome, _)| outcome)
}

/// The "Folder" step: creates the night's folder if no folder carries
/// the night yet (named by [`night_folder_name`]), then whatever is missing
/// of `logs/`, `calibration/{darks,flats,dark-flats,biases}/` and, for each
/// ticked target, `<target>/lights/` and `<target>/final/` with the identity
/// files. Run again at each opening of the step: a target ticked since gets
/// its folder; nothing existing is touched.
pub fn prepare_night_folder(
    root: &Path,
    remembered: &[PathBuf],
    spec: &NightFolderSpec,
    now_utc: &str,
) -> Result<NightFolderOutcome, WorkingFolderError> {
    let (mut outcome, creations) = plan(root, remembered, spec, now_utc)?;
    for creation in creations {
        match creation {
            Creation::Folder(path) => {
                if !path.is_dir() {
                    fs::create_dir_all(&path).map_err(io_at(&path))?;
                }
            }
            Creation::Identity(path, json) => {
                write_identity_file(&path, &json)?;
            }
        }
    }
    outcome.applied = true;
    Ok(outcome)
}

fn plan(
    root: &Path,
    remembered: &[PathBuf],
    spec: &NightFolderSpec,
    now_utc: &str,
) -> Result<(NightFolderOutcome, Vec<Creation>), WorkingFolderError> {
    if spec.night_id.trim().is_empty() {
        return Err(WorkingFolderError::EmptyNightId);
    }
    let name = night_folder_name(&spec.evening_date, &spec.label, &spec.night_id)?;
    let mut creations = Vec::new();
    let mut created_paths = Vec::new();

    let (folder, created) = match locate_night(root, remembered, &spec.night_id)? {
        NightLocation::Found { folder } => (folder, false),
        NightLocation::Duplicated { folders } => match &spec.follow {
            Some(chosen) if folders.iter().any(|f| same_folder(f, chosen)) => {
                (chosen.clone(), false)
            }
            _ => {
                return Err(WorkingFolderError::DuplicateNight {
                    night_id: spec.night_id.clone(),
                    folders,
                });
            }
        },
        NightLocation::Missing => {
            let folder = root.join(free_name(root, &name, |path| {
                // A folder of that name with no identity (made by hand for
                // this night) is taken; one carrying another night is not.
                read_night_identity(path).is_none()
            }));
            let exists = folder.is_dir();
            if !exists {
                creations.push(Creation::Folder(folder.clone()));
            }
            (folder, !exists)
        }
    };

    let identity = NightIdentity {
        format: IDENTITY_FORMAT,
        night_id: spec.night_id.clone(),
        service_url: spec.service_url.clone(),
        created_at: now_utc.to_string(),
        pending: None,
    };
    if read_night_identity(&folder).is_none() && !folder.join(NIGHT_IDENTITY_FILE).exists() {
        creations.push(Creation::Identity(
            folder.join(NIGHT_IDENTITY_FILE),
            identity_json(&identity),
        ));
        created_paths.push(NIGHT_IDENTITY_FILE.to_string());
    }

    let mut want_folder = |relative: &str, creations: &mut Vec<Creation>| {
        let path = folder.join(relative);
        if !path.is_dir() {
            creations.push(Creation::Folder(path));
            created_paths.push(format!("{relative}/"));
        }
    };
    want_folder(LOGS_DIR, &mut creations);
    want_folder(CALIBRATION_DIR, &mut creations);
    for kind in CALIBRATION_KINDS {
        want_folder(&format!("{CALIBRATION_DIR}/{kind}"), &mut creations);
    }

    // The target folders already there, by identity.
    let existing = target_folders_by_identity(&folder);
    let mut targets = Vec::new();
    let mut claimed: Vec<String> = Vec::new();
    for target in spec.targets.iter().filter(|t| t.ticked) {
        let found = existing
            .iter()
            .find(|(id, _)| *id == target.target_id)
            .map(|(_, name)| name.clone());
        let (folder_name, state) = match found {
            Some(name) => (name, TargetFolderState::Existing),
            None => {
                let wanted = target_folder_name(&target.designation);
                let name = free_name_among(&folder, &wanted, &claimed, |path| {
                    read_target_identity(path).is_none()
                        && !is_reserved_night_entry(
                            &path.file_name().unwrap_or_default().to_string_lossy(),
                        )
                });
                (name, TargetFolderState::Created)
            }
        };
        claimed.push(folder_name.clone());
        let target_dir = folder.join(&folder_name);
        let mut new_here = false;
        for sub in ["", LIGHTS_DIR, FINAL_DIR] {
            let path = if sub.is_empty() {
                target_dir.clone()
            } else {
                target_dir.join(sub)
            };
            if !path.is_dir() {
                creations.push(Creation::Folder(path));
                created_paths.push(if sub.is_empty() {
                    format!("{folder_name}/")
                } else {
                    format!("{folder_name}/{sub}/")
                });
                new_here = true;
            }
        }
        if read_target_identity(&target_dir).is_none()
            && !target_dir.join(TARGET_IDENTITY_FILE).exists()
        {
            creations.push(Creation::Identity(
                target_dir.join(TARGET_IDENTITY_FILE),
                identity_json(&TargetIdentity {
                    format: IDENTITY_FORMAT,
                    target_id: target.target_id.clone(),
                    filter: None,
                }),
            ));
            created_paths.push(format!("{folder_name}/{TARGET_IDENTITY_FILE}"));
        }
        let state = if state == TargetFolderState::Existing || !new_here {
            TargetFolderState::Existing
        } else {
            state
        };
        targets.push(TargetFolder {
            target_id: target.target_id.clone(),
            designation: target.designation.clone(),
            folder_name,
            state,
        });
    }
    // The folders of targets no longer ticked (or no longer of the night):
    // kept, "outside the night".
    for (target_id, name) in &existing {
        if targets.iter().any(|t| &t.target_id == target_id) {
            continue;
        }
        let designation = spec
            .targets
            .iter()
            .find(|t| &t.target_id == target_id)
            .map_or_else(|| name.clone(), |t| t.designation.clone());
        targets.push(TargetFolder {
            target_id: target_id.clone(),
            designation,
            folder_name: name.clone(),
            state: TargetFolderState::OutOfNight,
        });
    }

    Ok((
        NightFolderOutcome {
            folder,
            created,
            created_paths,
            targets,
            applied: false,
        },
        creations,
    ))
}

fn identity_json<T: Serialize>(value: &T) -> String {
    let mut json = serde_json::to_string_pretty(value).expect("identity always serializes");
    json.push('\n');
    json
}

/// `logs`, `calibration`, `stacked`…: names a target folder never takes.
fn is_reserved_night_entry(name: &str) -> bool {
    [LOGS_DIR, CALIBRATION_DIR]
        .iter()
        .any(|reserved| same_name(reserved, name))
}

/// Target folders of a night folder, by identity: (target id, folder name).
#[must_use]
pub fn target_folders_by_identity(night_folder: &Path) -> Vec<(String, String)> {
    let Ok(entries) = fs::read_dir(night_folder) else {
        return Vec::new();
    };
    let mut found: Vec<(String, String)> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            read_target_identity(&entry.path()).map(|identity| {
                (
                    identity.target_id,
                    entry.file_name().to_string_lossy().into_owned(),
                )
            })
        })
        .collect();
    found.sort_by(|a, b| a.1.cmp(&b.1));
    found
}

/// `wanted`, or `wanted (2)`, `wanted (3)`… : the first name under `parent`
/// that is free, or taken by a folder `may_take` accepts (compared in NFC,
/// ignoring case).
fn free_name(parent: &Path, wanted: &str, may_take: impl Fn(&Path) -> bool) -> String {
    free_name_among(parent, wanted, &[], may_take)
}

fn free_name_among(
    parent: &Path,
    wanted: &str,
    claimed: &[String],
    may_take: impl Fn(&Path) -> bool,
) -> String {
    let existing: Vec<String> = fs::read_dir(parent)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    for attempt in 1.. {
        let candidate = if attempt == 1 {
            wanted.to_string()
        } else {
            format!("{wanted} ({attempt})")
        };
        if claimed.iter().any(|name| same_name(name, &candidate)) {
            continue;
        }
        match existing.iter().find(|name| same_name(name, &candidate)) {
            None => return candidate,
            Some(name) => {
                let path = parent.join(name);
                if path.is_dir() && may_take(&path) {
                    // The existing spelling (NFD, other case) is kept.
                    return name.clone();
                }
            }
        }
    }
    unreachable!("an unbounded range always yields a free name")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    fn spec(targets: &[(&str, &str, bool)]) -> NightFolderSpec {
        NightFolderSpec {
            night_id: "7f3a2c91-0000-4000-8000-000000000001".to_string(),
            service_url: "https://service.example".to_string(),
            evening_date: "2026-10-07".to_string(),
            label: "Col des Étoiles".to_string(),
            targets: targets
                .iter()
                .map(|(id, designation, ticked)| NightTargetSpec {
                    target_id: (*id).to_string(),
                    designation: (*designation).to_string(),
                    ticked: *ticked,
                })
                .collect(),
            follow: None,
        }
    }

    const NOW: &str = "2026-10-08T09:00:00Z";

    #[test]
    fn the_default_root_per_system() {
        let mac = RootEnvironment {
            system: HostSystem::MacOs,
            home: Some(PathBuf::from("/maison/camille")),
            xdg_documents_dir: None,
            xdg_user_dirs: None,
            windows_documents: None,
        };
        assert_eq!(
            default_root(&mac),
            Some(PathBuf::from("/maison/camille/Documents/Astro Sync"))
        );
        let windows = RootEnvironment {
            system: HostSystem::Windows,
            windows_documents: Some(PathBuf::from("/onedrive/Documents")),
            ..mac.clone()
        };
        assert_eq!(
            default_root(&windows),
            Some(PathBuf::from("/onedrive/Documents/Astro Sync"))
        );
        let linux = RootEnvironment {
            system: HostSystem::Linux,
            home: Some(PathBuf::from("/home/camille")),
            xdg_user_dirs: Some(
                "# written by xdg-user-dirs-update\nXDG_DOCUMENTS_DIR=\"$HOME/Dokumente\"\n"
                    .to_string(),
            ),
            ..mac.clone()
        };
        assert_eq!(
            default_root(&linux),
            Some(PathBuf::from("/home/camille/Dokumente/Astro Sync"))
        );
        let linux_env = RootEnvironment {
            xdg_documents_dir: Some("/data/docs".to_string()),
            ..linux.clone()
        };
        assert_eq!(
            default_root(&linux_env),
            Some(PathBuf::from("/data/docs/Astro Sync"))
        );
        let linux_bare = RootEnvironment {
            xdg_user_dirs: Some("XDG_DOCUMENTS_DIR=\"$HOME/\"\n".to_string()),
            ..linux
        };
        assert_eq!(
            default_root(&linux_bare),
            Some(PathBuf::from("/home/camille/Documents/Astro Sync"))
        );
        let nobody = RootEnvironment { home: None, ..mac };
        assert_eq!(default_root(&nobody), None);
    }

    #[test]
    fn a_night_folder_name_follows_the_sheet() {
        assert_eq!(
            night_folder_name("2026-10-07", "Col des Étoiles", "7f3a2c91-aaaa").unwrap(),
            "2026-10-07 · Col des Étoiles · 7f3a2c"
        );
        // Forbidden characters, ends trimmed, NFD composed.
        assert_eq!(
            night_folder_name(
                "2026-10-07",
                "  .M31: test/a*b?\"c<d>e|f\u{7}.  ",
                "abcdef12"
            )
            .unwrap(),
            "2026-10-07 · M31- test-a-b--c-d-e-f- · abcdef"
        );
        assert_eq!(
            night_folder_name("2026-10-07", "Col des E\u{301}toiles", "abcdef").unwrap(),
            "2026-10-07 · Col des Étoiles · abcdef"
        );
        // The middle part cut at sixty characters.
        let long = "Plan très long ".repeat(10);
        let name = night_folder_name("2026-10-07", &long, "abcdef").unwrap();
        let middle = name.split(" · ").nth(1).unwrap();
        assert!(middle.chars().count() <= MIDDLE_PART_MAX_CHARS);
        assert!(!middle.ends_with(' '));
        // No label: date and short identifier only.
        assert_eq!(
            night_folder_name("2026-10-07", "  ", "abc").unwrap(),
            "2026-10-07 · abc"
        );
        assert!(night_folder_name("7 oct.", "x", "abc").is_err());
        assert!(night_folder_name("2026-10-07", "x", " ").is_err());
    }

    #[test]
    fn windows_reserved_names_are_suffixed() {
        assert_eq!(clean_component("CON"), "CON-");
        assert_eq!(clean_component("com1"), "com1-");
        assert_eq!(clean_component("LPT9.txt"), "LPT9-.txt");
        assert_eq!(clean_component("CONSOLE"), "CONSOLE");
        assert_eq!(clean_component("..."), "-");
        assert_eq!(target_folder_name("NGC 7000"), "NGC 7000");
    }

    #[test]
    fn the_evening_date_comes_from_the_start_and_the_longitude() {
        // 45° N 30° W: 23:30 UTC is 21:30 solar time the same evening.
        assert_eq!(
            evening_date_of("2026-10-07T23:30:00Z", Some(-30.0)).as_deref(),
            Some("2026-10-07")
        );
        // After midnight solar time: still the evening before.
        assert_eq!(
            evening_date_of("2026-10-08T03:00:00Z", Some(-30.0)).as_deref(),
            Some("2026-10-07")
        );
        // 135° E: 12:00 UTC is 21:00 solar time.
        assert_eq!(
            evening_date_of("2026-10-07T12:00:00Z", Some(135.0)).as_deref(),
            Some("2026-10-07")
        );
        assert_eq!(evening_date_of("not a date", None), None);
    }

    #[test]
    fn the_dossier_step_creates_the_ticked_targets_only() {
        let root = TempDir::new("wf-create");
        let night = spec(&[
            ("t-ngc7000", "NGC 7000", true),
            ("t-ic1396", "IC 1396", true),
            ("t-m31", "M31", false),
        ]);
        let preview = plan_night_folder(root.path(), &[], &night, NOW).unwrap();
        assert!(!preview.applied);
        assert!(
            !root
                .path()
                .join("2026-10-07 · Col des Étoiles · 7f3a2c")
                .exists()
        );

        let outcome = prepare_night_folder(root.path(), &[], &night, NOW).unwrap();
        assert!(outcome.created && outcome.applied);
        assert_eq!(outcome.created_paths, preview.created_paths);
        let folder = root.path().join("2026-10-07 · Col des Étoiles · 7f3a2c");
        assert_eq!(outcome.folder, folder);
        for dir in [
            "logs",
            "calibration/darks",
            "calibration/flats",
            "calibration/dark-flats",
            "calibration/biases",
            "NGC 7000/lights",
            "NGC 7000/final",
            "IC 1396/lights",
            "IC 1396/final",
        ] {
            assert!(folder.join(dir).is_dir(), "{dir}");
        }
        assert!(!folder.join("M31").exists());
        assert!(!folder.join("NGC 7000/stacked").exists());
        let identity = read_night_identity(&folder).unwrap();
        assert_eq!(identity.night_id, night.night_id);
        assert_eq!(identity.created_at, NOW);
        assert_eq!(
            read_target_identity(&folder.join("IC 1396"))
                .unwrap()
                .target_id,
            "t-ic1396"
        );

        // M31 ticked later: its folder at the next opening, the rest kept.
        let mut later = night.clone();
        later.targets[2].ticked = true;
        later.targets[1].ticked = false;
        let second = prepare_night_folder(root.path(), &[], &later, NOW).unwrap();
        assert!(!second.created);
        assert_eq!(
            second.created_paths,
            vec![
                "M31/",
                "M31/lights/",
                "M31/final/",
                "M31/.astro-sync-target.json"
            ]
        );
        let state = |id: &str| {
            second
                .targets
                .iter()
                .find(|t| t.target_id == id)
                .unwrap()
                .state
        };
        assert_eq!(state("t-m31"), TargetFolderState::Created);
        assert_eq!(state("t-ngc7000"), TargetFolderState::Existing);
        assert_eq!(state("t-ic1396"), TargetFolderState::OutOfNight);
        assert!(folder.join("IC 1396/lights").is_dir(), "never deleted");
    }

    #[test]
    fn a_renamed_night_and_target_are_found_by_identity() {
        let root = TempDir::new("wf-rename");
        let night = spec(&[("t-m31", "M31", true)]);
        let first = prepare_night_folder(root.path(), &[], &night, NOW).unwrap();
        let renamed = root.path().join("ma nuit à moi");
        fs::rename(&first.folder, &renamed).unwrap();
        fs::rename(renamed.join("M31"), renamed.join("Andromède")).unwrap();

        assert_eq!(
            locate_night(root.path(), &[], &night.night_id).unwrap(),
            NightLocation::Found {
                folder: renamed.clone()
            }
        );
        let again = prepare_night_folder(root.path(), &[], &night, NOW).unwrap();
        assert_eq!(again.folder, renamed);
        assert!(again.created_paths.is_empty());
        assert_eq!(again.targets[0].folder_name, "Andromède");
        assert!(!renamed.join("M31").exists(), "no second folder for M31");
    }

    #[test]
    fn two_folders_with_one_night_are_said_and_one_is_followed() {
        let root = TempDir::new("wf-duplicate");
        let night = spec(&[("t-m31", "M31", true)]);
        let first = prepare_night_folder(root.path(), &[], &night, NOW).unwrap();
        let copy = root.path().join("copie");
        fs::create_dir(&copy).unwrap();
        fs::copy(
            first.folder.join(NIGHT_IDENTITY_FILE),
            copy.join(NIGHT_IDENTITY_FILE),
        )
        .unwrap();

        let location = locate_night(root.path(), &[], &night.night_id).unwrap();
        let NightLocation::Duplicated { folders } = location else {
            panic!("expected a duplicate, got {location:?}");
        };
        assert_eq!(folders.len(), 2);
        let error = prepare_night_folder(root.path(), &[], &night, NOW).unwrap_err();
        assert_eq!(error.code(), "night_folder_duplicated");

        let mut follow = night.clone();
        follow.follow = Some(copy.clone());
        let outcome = prepare_night_folder(root.path(), &[], &follow, NOW).unwrap();
        assert_eq!(outcome.folder, copy);
        assert!(copy.join("M31/lights").is_dir());
    }

    /// "Folder" is run again at each opening of the step: the second run
    /// creates nothing, and a file dropped between the two is left exactly
    /// as it was.
    #[test]
    fn a_second_opening_of_the_step_creates_nothing_and_keeps_a_dropped_file() {
        let root = TempDir::new("wf-twice");
        let night = spec(&[("t-ngc7000", "NGC 7000", true), ("t-m31", "M31", true)]);
        let first = prepare_night_folder(root.path(), &[], &night, NOW).unwrap();
        assert!(first.created);
        assert!(!first.created_paths.is_empty());

        let dropped = first.folder.join("NGC 7000/lights/light_0001.fits");
        fs::write(&dropped, b"SIMPLE  =                    T").unwrap();
        let before = fs::read(&dropped).unwrap();

        let second =
            prepare_night_folder(root.path(), &[], &night, "2026-10-08T10:00:00Z").unwrap();
        assert!(!second.created, "the night's folder was already there");
        assert!(
            second.created_paths.is_empty(),
            "{:?}",
            second.created_paths
        );
        assert_eq!(second.folder, first.folder);
        assert_eq!(
            second.targets.iter().map(|t| t.state).collect::<Vec<_>>(),
            vec![TargetFolderState::Existing, TargetFolderState::Existing]
        );
        assert_eq!(fs::read(&dropped).unwrap(), before);
        assert_eq!(
            read_night_identity(&second.folder).unwrap().created_at,
            NOW,
            "the identity file keeps its first date"
        );
    }

    #[test]
    fn a_followed_copy_makes_a_duplicated_night_found() {
        let a = PathBuf::from("/data/a");
        let b = PathBuf::from("/data/b");
        let duplicated = NightLocation::Duplicated {
            folders: vec![a.clone(), b.clone()],
        };
        assert_eq!(
            apply_followed(duplicated.clone(), Some(&b)),
            NightLocation::Found { folder: b }
        );
        // Nothing followed, or a folder that is no longer a copy: still two.
        assert_eq!(apply_followed(duplicated.clone(), None), duplicated);
        assert_eq!(
            apply_followed(duplicated.clone(), Some(Path::new("/data/c"))),
            duplicated
        );
        assert_eq!(
            apply_followed(NightLocation::Missing, Some(&a)),
            NightLocation::Missing
        );
    }

    #[test]
    fn a_night_moved_out_is_found_again_by_its_identity() {
        let root = TempDir::new("wf-moved-root");
        let elsewhere = TempDir::new("wf-moved-elsewhere");
        let night = spec(&[]);
        let first = prepare_night_folder(root.path(), &[], &night, NOW).unwrap();
        let moved = elsewhere.path().join("nuit");
        fs::rename(&first.folder, &moved).unwrap();
        assert_eq!(
            locate_night(root.path(), &[], &night.night_id).unwrap(),
            NightLocation::Missing
        );

        assert_eq!(check_found_again(&moved, &night.night_id).unwrap(), moved);
        assert_eq!(
            check_found_again(elsewhere.path(), &night.night_id)
                .unwrap_err()
                .code(),
            "night_folder_without_identity"
        );
        assert_eq!(
            check_found_again(&moved, "another-night")
                .unwrap_err()
                .code(),
            "night_folder_other_night"
        );
        assert_eq!(
            locate_night(root.path(), std::slice::from_ref(&moved), &night.night_id).unwrap(),
            NightLocation::Found { folder: moved }
        );
    }

    #[test]
    fn a_folder_of_the_name_made_by_hand_is_taken_and_its_files_kept() {
        let root = TempDir::new("wf-by-hand");
        let night = spec(&[("t-m31", "M31", true)]);
        // Made by hand in NFD and in lower case, with a frame in it.
        let by_hand = root
            .path()
            .join("2026-10-07 · Col des E\u{301}toiles · 7f3a2c");
        fs::create_dir_all(by_hand.join("m31/lights")).unwrap();
        fs::write(by_hand.join("m31/lights/a.fits"), b"frame").unwrap();
        let outcome = prepare_night_folder(root.path(), &[], &night, NOW).unwrap();
        assert!(!outcome.created);
        assert_eq!(outcome.targets[0].folder_name, "m31");
        assert_eq!(
            fs::read(by_hand.join("m31/lights/a.fits")).unwrap(),
            b"frame"
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_name_taken_by_another_night_gets_a_number() {
        let root = TempDir::new("wf-other-night");
        let mut other = spec(&[]);
        other.night_id = "7f3a2cff-other".to_string();
        prepare_night_folder(root.path(), &[], &other, NOW).unwrap();
        let night = spec(&[]);
        let outcome = prepare_night_folder(root.path(), &[], &night, NOW).unwrap();
        assert_eq!(
            outcome.folder.file_name().unwrap().to_string_lossy(),
            "2026-10-07 · Col des Étoiles · 7f3a2c (2)"
        );
    }

    #[test]
    fn an_identity_file_is_never_replaced() {
        let root = TempDir::new("wf-identity-kept");
        let night = spec(&[("t-m31", "M31", true)]);
        let first = prepare_night_folder(root.path(), &[], &night, NOW).unwrap();
        let path = first.folder.join(NIGHT_IDENTITY_FILE);
        let before = fs::read(&path).unwrap();
        prepare_night_folder(root.path(), &[], &night, "2030-01-01T00:00:00Z").unwrap();
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(!write_identity_file(&path, "{}").unwrap());
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn the_root_is_created_once_and_a_new_root_must_exist() {
        let parent = TempDir::new("wf-root");
        let root = parent.path().join("Documents/Astro Sync");
        assert!(ensure_root(&root).unwrap());
        assert!(!ensure_root(&root).unwrap());
        assert_eq!(check_new_root(&root).unwrap(), root);
        assert_eq!(
            check_new_root(&parent.path().join("absent"))
                .unwrap_err()
                .code(),
            "folder_not_found"
        );
    }
}

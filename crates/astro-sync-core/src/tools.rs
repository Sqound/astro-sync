//! The external tools Astro Sync uses, as Settings › "Dependencies" shows
//! them.
//!
//! **One source**: the dependency registry of the repository,
//! `stacking/dependencies/registry.json`, embedded
//! in Astro Sync at build time ([`registry`]). It names every tool a
//! treatment or a stacking pipeline may use (Siril, StarNet2, the stacking's
//! Python environment, ASTAP, astrometry.net), with its detection rule
//! (`check`), its minimum version, its downloads by system (URL **and**
//! SHA-256), its manual page and its licence. Adding a tool is a pull
//! request on that file; nothing here lists a tool of its own. Three rows
//! are added beside the registry's because they are not programs: ASTAP's
//! star database, the Tycho-2 list of the measured sky background, and the
//! stacking pipeline (its Python environment and Siril).
//!
//! - **Detection** ([`detect_tools`]): the registry's `check` (the command,
//!   asked for its version only; the version read; the minimum) gives
//!   present, missing or outdated. An entry without `check` (ASTAP and
//!   astrometry.net today: their rule is not verified yet) is found by Astro
//!   Sync's own rule -- the program at its usual places -- and is said
//!   present without a version. The places looked at: Astro Sync's own
//!   tools folder, each system's usual installation folders, then `PATH`
//!   (with `/opt/homebrew/bin` and `/usr/local/bin`, which a macOS window
//!   does not inherit).
//! - **Download** ([`download_tool`]): the registry's file for this system,
//!   fetched into Astro Sync's settings folder (`tools/<id>/`, never in the
//!   installer), resumed from what is already there, refused unless its
//!   SHA-256 is the registry's. A system without an entry leaves the manual
//!   page (a neutral "To install", not a failure); a network, digest or
//!   unpacking failure is a failure.
//! - **Install** ([`install_download`]): a ZIP (ASTAP's
//!   command-line solver, its D20 star database) is unpacked into
//!   `tools/<id>/` through a staging folder, without any administrator
//!   right, then deleted. The detection looks there first. ASTAP's star
//!   database is the registry entry `astap-database`, shown as its own row.
//! - **The stacking's Python environment** (`python-env`) is
//!   not a program to download: it is made on this computer from a Python in
//!   the registry's range ([`crate::python_env`]); its row and the row
//!   "Stacking pipeline" say whether it is there, unfinished or made
//!   for other pins, and whether it can be installed (a Python was found).
//! - Nothing is ever bundled, and nothing is run but the version query.

use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::analysis::siril;
use crate::external_process;
use crate::local_work::CancelFlag;
use crate::precheck::ToolsPresent;
use crate::python_env::{self, StackingResources};
use crate::sky_solution::{SolverTool, Solvers};

/* ------------------------------- The registry ---------------------------- */

/// The registry as the repository holds it, embedded at build time.
pub const REGISTRY_JSON: &str = include_str!("../../../stacking/dependencies/registry.json");

/// `kind` of an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DependencyKind {
    Tool,
    PythonPackages,
}

/// An entry's detection rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    /// One program name, or several tried in order.
    #[serde(deserialize_with = "one_or_many")]
    pub command: Vec<String>,
    pub args: Vec<String>,
    pub version_pattern: String,
}

fn one_or_many<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(deserializer)? {
        OneOrMany::One(one) => vec![one],
        OneOrMany::Many(many) => many,
    })
}

/// One file to download for one system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Download {
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub bytes: Option<u64>,
}

/// One entry of the registry (`pipelines/SCHEMA.md`, « The dependency
/// registry »).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryEntry {
    pub id: String,
    pub name: BTreeMap<String, String>,
    pub kind: DependencyKind,
    #[serde(default)]
    pub min_version: Option<String>,
    #[serde(default)]
    pub check: Option<Check>,
    #[serde(default)]
    pub downloads: BTreeMap<String, Download>,
    /// The size of what the person downloads, by system, when `downloads`
    /// gives none: the manual archive of the `page` (a `tool`), or the
    /// pinned packages (`python-packages`): "Download (90 MB)".
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sizes: BTreeMap<String, u64>,
    #[serde(default)]
    pub requirements: Option<String>,
    pub page: String,
    pub licence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
    pub format: u32,
    pub dependencies: Vec<RegistryEntry>,
}

impl Registry {
    #[must_use]
    pub fn entry(&self, id: &str) -> Option<&RegistryEntry> {
        self.dependencies.iter().find(|e| e.id == id)
    }
}

/// The registry Astro Sync was built with. It ships with the app and is
/// checked by the tests: reading it cannot fail at run time.
#[must_use]
pub fn registry() -> Registry {
    parse_registry(REGISTRY_JSON).expect("the embedded dependency registry is valid")
}

/// Reads a registry, refusing it whole when it breaks a rule of the format
/// (a URL without its digest, a digest that is not 64 lower-case
/// hexadecimal characters, an unknown system, a `check` without minimum).
pub fn parse_registry(text: &str) -> Result<Registry, String> {
    let registry: Registry = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if registry.format != 1 {
        return Err(format!("unknown registry format {}", registry.format));
    }
    let mut seen = Vec::new();
    for entry in &registry.dependencies {
        if seen.contains(&entry.id) {
            return Err(format!("dependency {:?} is listed twice", entry.id));
        }
        seen.push(entry.id.clone());
        if !entry.name.contains_key("en") {
            return Err(format!("dependency {:?} has no English name", entry.id));
        }
        if entry.kind == DependencyKind::Tool
            && entry.check.is_some() != entry.min_version.is_some()
        {
            return Err(format!(
                "dependency {:?}: minVersion and check go together",
                entry.id
            ));
        }
        for (system, bytes) in &entry.sizes {
            if !SYSTEMS.contains(&system.as_str()) || *bytes == 0 {
                return Err(format!(
                    "dependency {:?}: a size needs a known system and a positive number",
                    entry.id
                ));
            }
        }
        for (system, download) in &entry.downloads {
            if !SYSTEMS.contains(&system.as_str()) {
                return Err(format!(
                    "dependency {:?}: unknown system {system:?}",
                    entry.id
                ));
            }
            let hex = download.sha256.len() == 64
                && download
                    .sha256
                    .chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
            if !download.url.starts_with("https://") || !hex {
                return Err(format!(
                    "dependency {:?}: a download needs an https URL and its SHA-256",
                    entry.id
                ));
            }
        }
    }
    Ok(registry)
}

/// The systems a download may be given for.
pub const SYSTEMS: [&str; 4] = ["macos-arm64", "macos-x64", "windows-x64", "linux-x64"];

/// This computer's key in `downloads`.
#[must_use]
pub fn this_system() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("macos-arm64"),
        ("macos", "x86_64") => Some("macos-x64"),
        ("windows", "x86_64") => Some("windows-x64"),
        ("linux", "x86_64") => Some("linux-x64"),
        _ => None,
    }
}

/// A text of the registry in `language`: the exact code, then the language
/// without its region, then English (`pipelines/SCHEMA.md`).
#[must_use]
pub fn localized(texts: &BTreeMap<String, String>, language: &str) -> String {
    let base = language.split(['-', '_']).next().unwrap_or(language);
    texts
        .get(language)
        .or_else(|| texts.get(base))
        .or_else(|| texts.get("en"))
        .cloned()
        .unwrap_or_default()
}

/* --------------------------------- Versions ------------------------------ */

/// The version `pattern` reads in `output`. The registry's patterns have one
/// form -- an optional `(?i)`, a word (its last character may be optional,
/// `starnet2?`), `[^\d\n]*`, then the group `(\d+(?:\.\d+)+)` -- which is
/// read here without a regular-expression library; any other pattern takes
/// the first dotted number of the output.
#[must_use]
pub fn read_version(pattern: &str, output: &str) -> Option<String> {
    const GROUP: &str = r"(\d+(?:\.\d+)+)";
    const GAP: &str = r"[^\d\n]*";
    let (insensitive, rest) = match pattern.strip_prefix("(?i)") {
        Some(rest) => (true, rest),
        None => (false, pattern),
    };
    let word = rest
        .strip_suffix(GROUP)
        .map(|w| w.strip_suffix(GAP).unwrap_or(w));
    let Some(word) = word.filter(|w| {
        w.chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '?')
    }) else {
        return first_dotted_number(output);
    };
    let mut words = vec![word.replace('?', "")];
    if let Some(stem) = word.strip_suffix('?') {
        words.push(stem[..stem.len().saturating_sub(1)].to_string());
    }
    for line in output.lines() {
        let haystack = if insensitive {
            line.to_lowercase()
        } else {
            line.to_string()
        };
        for word in &words {
            let needle = if insensitive {
                word.to_lowercase()
            } else {
                word.clone()
            };
            let mut from = 0;
            while let Some(at) = haystack[from..].find(&needle) {
                let after = &line[from + at + needle.len()..];
                let start = after.find(|c: char| c.is_ascii_digit());
                if let Some(version) = start.and_then(|s| dotted_number_at(&after[s..])) {
                    return Some(version);
                }
                from += at + needle.len();
            }
        }
    }
    None
}

fn dotted_number_at(text: &str) -> Option<String> {
    let token: String = text
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let token = token.trim_end_matches('.');
    (token.contains('.') && !token.contains("..")).then(|| token.to_string())
}

fn first_dotted_number(output: &str) -> Option<String> {
    let mut rest = output;
    while let Some(start) = rest.find(|c: char| c.is_ascii_digit()) {
        if let Some(version) = dotted_number_at(&rest[start..]) {
            return Some(version);
        }
        let skip = rest[start..]
            .find(|c: char| !c.is_ascii_digit())
            .map_or(rest.len(), |n| start + n);
        rest = &rest[skip..];
    }
    None
}

/// Whether `version` (dotted numbers) is at least `minimum`.
#[must_use]
pub fn at_least(version: &str, minimum: &str) -> bool {
    let numbers = |text: &str| -> Vec<u64> {
        text.split('.')
            .map(|part| {
                part.chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0)
            })
            .collect()
    };
    let (mut v, mut m) = (numbers(version), numbers(minimum));
    let len = v.len().max(m.len());
    v.resize(len, 0);
    m.resize(len, 0);
    v >= m
}

/* --------------------------------- Detection ----------------------------- */

/// A row's state (badges "Present · 1.4.0", « Absent », "Download"…).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolState {
    Present,
    Missing,
    /// A version was read, below the minimum.
    Outdated,
    /// The registry gives no rule Astro Sync can apply here.
    NotChecked,
    /// Its file is in Astro Sync's tools folder; the person installs it.
    Downloaded,
}

/// One row of "Dependencies".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolRow {
    /// The registry's id, or `astap-database`, `tycho-2`, `stacking-pipeline`.
    pub id: String,
    /// The name per language (the registry's).
    pub name: BTreeMap<String, String>,
    /// `tool`, `python-packages`, `data`, `pipeline`.
    pub kind: String,
    /// Installed by "Install all"; the others by their own "Install"
    /// ("Optional").
    pub required: bool,
    pub state: ToolState,
    pub version: Option<String>,
    pub min_version: Option<String>,
    /// Where it was found.
    pub path: Option<PathBuf>,
    pub page: Option<String>,
    pub licence: Option<String>,
    /// The automatic download for this system, if the registry has one.
    pub download: Option<Download>,
    /// What the person downloads for this system, in bytes: the automatic
    /// download's size, else the registry's `sizes`
    /// ("Download (90 MB)").
    #[serde(default)]
    pub download_bytes: Option<u64>,
    /// Bytes of an interrupted download kept ("Resume the
    /// download").
    pub partial_bytes: Option<u64>,
    /// Siril: whether its version is one the progression was checked
    /// against (`siril_versions.json`, a rule apart from the stacking's
    /// minimum).
    pub progression_compatible: Option<bool>,
    /// The Python environment (and the stacking pipeline): an installation
    /// that did not finish ("Resume the installation").
    #[serde(default)]
    pub interrupted: bool,
    /// Astro Sync can install it here without a download of the registry
    /// (the Python environment, when a Python in the range was found).
    #[serde(default)]
    pub installable: bool,
    /// What it waits for (`python`: no Python in the range; `siril`; the
    /// environment, `python-env`), in this order.
    #[serde(default)]
    pub blocked_by: Vec<String>,
}

/// Where to look.
#[derive(Debug, Clone)]
pub struct DetectContext {
    /// Astro Sync's settings folder: `tools/<id>/` holds the downloads.
    pub data_dir: PathBuf,
    /// The Tycho-2 list (`reference-stars.bin`).
    pub reference_stars: PathBuf,
    /// Folders searched for a program, in order (`PATH` and the usual ones
    /// by default; the tests give their own).
    pub search_dirs: Vec<PathBuf>,
    /// Programs given by full path for an id (each system's usual
    /// installation places; the tests give their own).
    pub known_paths: BTreeMap<String, Vec<PathBuf>>,
    /// How long a version query may take.
    pub probe_timeout: Duration,
    /// The stacking's scripts beside the app, when they are there.
    pub stacking: Option<StackingResources>,
}

impl DetectContext {
    /// This computer: `PATH` and the usual folders.
    #[must_use]
    pub fn system(data_dir: &Path, reference_stars: &Path) -> Self {
        let mut search_dirs: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect())
            .unwrap_or_default();
        for extra in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
            let extra = PathBuf::from(extra);
            if !search_dirs.contains(&extra) {
                search_dirs.push(extra);
            }
        }
        let mut known_paths = BTreeMap::new();
        known_paths.insert(
            "siril".to_string(),
            siril::discovery::standard_install_locations(),
        );
        known_paths.insert("astap".to_string(), astap_install_locations());
        known_paths.insert("graxpert".to_string(), graxpert_install_locations(data_dir));
        known_paths.insert(
            "python".to_string(),
            python_env::python_install_locations(python_env::python_range()),
        );
        Self {
            data_dir: data_dir.to_path_buf(),
            reference_stars: reference_stars.to_path_buf(),
            search_dirs,
            known_paths,
            probe_timeout: Duration::from_secs(15),
            stacking: None,
        }
    }

    /// With the stacking's scripts in `dir` (`<resources>/stacking`).
    #[must_use]
    pub fn with_stacking(mut self, dir: Option<&Path>) -> Self {
        self.stacking = dir.and_then(StackingResources::find);
        self
    }

    fn tools_dir(&self, id: &str) -> PathBuf {
        self.data_dir.join("tools").join(id)
    }
}

fn astap_install_locations() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        vec![
            PathBuf::from("/Applications/ASTAP.app/Contents/MacOS/astap"),
            PathBuf::from("/usr/local/opt/astap/astap"),
        ]
    }
    #[cfg(target_os = "windows")]
    {
        astap_windows_places(std::env::var_os("LOCALAPPDATA"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        vec![
            PathBuf::from("/opt/astap/astap_cli"),
            PathBuf::from("/opt/astap/astap"),
        ]
    }
}

/// ASTAP's folder when it was installed without administrator rights, as
/// its installer then proposes: `%LOCALAPPDATA%\Programs\astap`.
#[cfg(any(target_os = "windows", test))]
fn astap_user_folder(local_app_data: Option<std::ffi::OsString>) -> Option<PathBuf> {
    local_app_data
        .filter(|local| !local.is_empty())
        .map(|local| PathBuf::from(local).join("Programs").join("astap"))
}

/// ASTAP's usual places on Windows: the installer's folder, then the user's
/// one (`local_app_data` is `%LOCALAPPDATA%`), the command line program first.
#[cfg(any(target_os = "windows", test))]
fn astap_windows_places(local_app_data: Option<std::ffi::OsString>) -> Vec<PathBuf> {
    let mut folders = vec![PathBuf::from(r"C:\Program Files\astap")];
    folders.extend(astap_user_folder(local_app_data));
    folders
        .iter()
        .flat_map(|folder| [folder.join("astap_cli.exe"), folder.join("astap.exe")])
        .collect()
}

/// Where GraXpert is when it was installed as its page says: the
/// application of the macOS image, the Windows installer's folder, and the
/// Linux archive unpacked by Astro Sync into `tools/graxpert/GraXpert-linux/`
/// (its program is one folder down, which the generic `tools/<id>/<name>`
/// does not see). `PATH` (a `graxpert` installed with `pip`) is looked at
/// after these.
fn graxpert_install_locations(data_dir: &Path) -> Vec<PathBuf> {
    let unpacked = data_dir
        .join("tools")
        .join("graxpert")
        .join("GraXpert-linux")
        .join("GraXpert");
    #[cfg(target_os = "macos")]
    {
        let mut places = vec![PathBuf::from(
            "/Applications/GraXpert.app/Contents/MacOS/GraXpert",
        )];
        if let Some(home) = std::env::var_os("HOME") {
            places.push(
                PathBuf::from(home).join("Applications/GraXpert.app/Contents/MacOS/GraXpert"),
            );
        }
        places.push(unpacked);
        places
    }
    #[cfg(target_os = "windows")]
    {
        vec![
            PathBuf::from(r"C:\Program Files\GraXpert\GraXpert.exe"),
            unpacked.with_extension("exe"),
        ]
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        vec![unpacked, PathBuf::from("/opt/GraXpert-linux/GraXpert")]
    }
}

/// The program names Astro Sync's own rule looks for, for an entry without
/// `check`.
fn own_rule_names(id: &str) -> Vec<String> {
    let names: &[&str] = match id {
        "astap" => &["astap_cli", "astap"],
        "astrometry-net" => &["solve-field"],
        _ => &[],
    };
    names
        .iter()
        .map(|name| format!("{name}{}", std::env::consts::EXE_SUFFIX))
        .collect()
}

fn candidates(context: &DetectContext, id: &str, names: &[String]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut push = |path: PathBuf| {
        if path.is_file() && !found.contains(&path) {
            found.push(path);
        }
    };
    for name in names {
        push(context.tools_dir(id).join(name));
    }
    for path in context.known_paths.get(id).into_iter().flatten() {
        push(path.clone());
    }
    for dir in &context.search_dirs {
        for name in names {
            push(dir.join(name));
        }
    }
    found
}

fn probe(path: &Path, args: &[String], timeout: Duration) -> Option<String> {
    let mut command = Command::new(path);
    command.args(args);
    let end = external_process::run(command, timeout, None).ok()?;
    Some(format!("{}\n{}", end.stdout, end.stderr))
}

fn partial_of(context: &DetectContext, id: &str, download: Option<&Download>) -> Option<u64> {
    let download = download?;
    let part = context
        .tools_dir(id)
        .join(format!("{}.part", file_name_of(&download.url)));
    fs::metadata(part).ok().map(|m| m.len()).filter(|&n| n > 0)
}

/// Which ids "Install all" covers (the « required » tools: Siril,
/// ASTAP and its star database).
fn required(id: &str) -> bool {
    matches!(id, "siril" | "astap" | "astap-database")
}

/// The size of what the person downloads for `entry` on this system: the
/// automatic download's, else the registry's `sizes`.
#[must_use]
pub fn download_bytes(entry: &RegistryEntry) -> Option<u64> {
    let system = this_system()?;
    entry
        .downloads
        .get(system)
        .and_then(|download| download.bytes)
        .or_else(|| entry.sizes.get(system).copied())
}

fn registry_row(context: &DetectContext, entry: &RegistryEntry) -> ToolRow {
    let download = this_system().and_then(|system| entry.downloads.get(system).cloned());
    let mut row = ToolRow {
        id: entry.id.clone(),
        name: entry.name.clone(),
        kind: match entry.kind {
            DependencyKind::Tool => "tool".to_string(),
            DependencyKind::PythonPackages => "python-packages".to_string(),
        },
        required: required(&entry.id),
        state: ToolState::Missing,
        version: None,
        min_version: entry.min_version.clone(),
        path: None,
        page: Some(entry.page.clone()),
        licence: Some(entry.licence.clone()),
        partial_bytes: partial_of(context, &entry.id, download.as_ref()),
        download_bytes: download_bytes(entry),
        download,
        progression_compatible: None,
        interrupted: false,
        installable: false,
        blocked_by: Vec::new(),
    };
    if entry.kind == DependencyKind::PythonPackages {
        let status = python_env::environment_status(&context.data_dir, context.stacking.as_ref());
        row.state = status.state;
        row.version = status.python_version;
        row.path = status.path;
        row.interrupted = status.interrupted;
        if row.state != ToolState::Present {
            let python = python_env::find_python(context, python_env::python_range());
            row.installable = python.is_some() && context.stacking.is_some();
            if python.is_none() {
                row.blocked_by.push("python".to_string());
            }
        }
        return row;
    }
    match &entry.check {
        Some(check) => {
            let names: Vec<String> = check
                .command
                .iter()
                .map(|name| format!("{name}{}", std::env::consts::EXE_SUFFIX))
                .collect();
            for candidate in candidates(context, &entry.id, &names) {
                let Some(output) = probe(&candidate, &check.args, context.probe_timeout) else {
                    continue;
                };
                let Some(version) = read_version(&check.version_pattern, &output) else {
                    continue;
                };
                let enough = entry
                    .min_version
                    .as_deref()
                    .is_none_or(|minimum| at_least(&version, minimum));
                row.state = if enough {
                    ToolState::Present
                } else {
                    ToolState::Outdated
                };
                row.path = Some(candidate);
                row.version = Some(version);
                break;
            }
        }
        None => {
            if let Some(found) = candidates(context, &entry.id, &own_rule_names(&entry.id))
                .into_iter()
                .next()
            {
                row.state = ToolState::Present;
                row.path = Some(found);
            }
        }
    }
    if row.state == ToolState::Missing && downloaded_file(context, &row).is_some() {
        row.state = ToolState::Downloaded;
    }
    if entry.id == "siril" {
        row.progression_compatible = row.version.as_deref().map(siril::is_compatible);
    }
    row
}

fn downloaded_file(context: &DetectContext, row: &ToolRow) -> Option<PathBuf> {
    let url = &row.download.as_ref()?.url;
    // An archive is unpacked as soon as it is checked (`install_download`):
    // one left behind is an unpacking to retry, not a file for the person.
    if is_archive(url) {
        return None;
    }
    let file = context.tools_dir(&row.id).join(file_name_of(url));
    file.is_file().then_some(file)
}

/// Whether `name` is a file of ASTAP's star database (`d50_0101.1476`,
/// `v50_1101.290`, `w08_0101.001`…).
fn is_star_database_file(name: &str) -> bool {
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return false;
    };
    let shape = stem.len() == 8
        && stem.as_bytes()[0].is_ascii_lowercase()
        && stem[1..3].chars().all(|c| c.is_ascii_digit())
        && &stem[3..4] == "_"
        && stem[4..].chars().all(|c| c.is_ascii_digit());
    shape && matches!(extension, "1476" | "290" | "001")
}

fn star_database_folder(context: &DetectContext, astap: Option<&Path>) -> Option<PathBuf> {
    let mut folders = vec![context.tools_dir("astap-database")];
    if let Some(astap) = astap.and_then(Path::parent) {
        folders.push(astap.to_path_buf());
    }
    folders.extend([
        PathBuf::from("/usr/local/opt/astap"),
        PathBuf::from("/opt/astap"),
        PathBuf::from(r"C:\Program Files\astap"),
    ]);
    // The user's folder is a usual place: only for a context that has the
    // usual places (`DetectContext::system`), so that the tests, which give
    // their own, never see this computer's ASTAP.
    #[cfg(target_os = "windows")]
    if context.known_paths.contains_key("astap") {
        folders.extend(astap_user_folder(std::env::var_os("LOCALAPPDATA")));
    }
    folders.into_iter().find(|folder| {
        fs::read_dir(folder).is_ok_and(|entries| {
            entries
                .filter_map(Result::ok)
                .any(|entry| is_star_database_file(&entry.file_name().to_string_lossy()))
        })
    })
}

/// The registry entry of ASTAP's star database (the row `astap-database`).
const DATABASE_ID: &str = "astap-database";

fn names(fr: &str, en: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("fr".to_string(), fr.to_string()),
        ("en".to_string(), en.to_string()),
    ])
}

/// Every row of "Dependencies", in the registry's order, then the star
/// database after ASTAP, the Tycho-2 list and the stacking pipeline.
#[must_use]
pub fn detect_tools(context: &DetectContext) -> Vec<ToolRow> {
    let registry = registry();
    let mut rows: Vec<ToolRow> = Vec::new();
    for entry in &registry.dependencies {
        // ASTAP's star database is a row of its own, made below from this
        // entry (its download, page and licence).
        if entry.id == DATABASE_ID {
            continue;
        }
        let row = registry_row(context, entry);
        let astap_path = (entry.id == "astap").then(|| row.path.clone()).flatten();
        let is_astap = entry.id == "astap";
        rows.push(row);
        if is_astap {
            let folder = star_database_folder(context, astap_path.as_deref());
            let database = registry.entry(DATABASE_ID);
            let download =
                this_system().and_then(|system| database?.downloads.get(system).cloned());
            rows.push(ToolRow {
                id: "astap-database".to_string(),
                name: names(
                    "Base d\u{2019}étoiles d\u{2019}ASTAP",
                    "ASTAP star database",
                ),
                kind: "data".to_string(),
                required: true,
                state: if folder.is_some() {
                    ToolState::Present
                } else {
                    ToolState::Missing
                },
                version: None,
                min_version: None,
                path: folder,
                page: Some(database.map_or(&entry.page, |d| &d.page).clone()),
                licence: Some(database.map_or(&entry.licence, |d| &d.licence).clone()),
                partial_bytes: partial_of(context, DATABASE_ID, download.as_ref()),
                download_bytes: download.as_ref().and_then(|d| d.bytes),
                download,
                progression_compatible: None,
                interrupted: false,
                installable: false,
                blocked_by: Vec::new(),
            });
        }
    }
    let catalogue = &crate::analysis::reference_catalogue::TYCHO2;
    rows.push(ToolRow {
        id: "tycho-2".to_string(),
        name: names("Liste d\u{2019}étoiles Tycho-2", "Tycho-2 star list"),
        kind: "data".to_string(),
        required: false,
        state: if context.reference_stars.is_file() {
            ToolState::Present
        } else {
            ToolState::Missing
        },
        version: None,
        min_version: None,
        path: context
            .reference_stars
            .is_file()
            .then(|| context.reference_stars.clone()),
        page: Some("https://cdsarc.cds.unistra.fr/viz-bin/cat/I/259".to_string()),
        licence: Some(catalogue.licence.to_string()),
        download: None,
        download_bytes: None,
        partial_bytes: None,
        progression_compatible: None,
        interrupted: false,
        installable: false,
        blocked_by: Vec::new(),
    });
    rows.push(stacking_pipeline_row(&rows));
    rows
}

/// "Stacking pipeline": the stacking can run when its Python
/// environment is present and Siril (the registry's minimum) too. Its
/// "Install" makes the environment; Siril has its own row.
fn stacking_pipeline_row(rows: &[ToolRow]) -> ToolRow {
    let env = row(rows, "python-env");
    let siril_ok = present(rows, "siril");
    let env_state = env.map_or(ToolState::Missing, |r| r.state);
    let mut blocked_by = env.map(|r| r.blocked_by.clone()).unwrap_or_default();
    if !siril_ok {
        blocked_by.push("siril".to_string());
    }
    if env_state != ToolState::Present && !blocked_by.iter().any(|b| b == "python") {
        blocked_by.push("python-env".to_string());
    }
    ToolRow {
        id: "stacking-pipeline".to_string(),
        name: names("Pipeline d\u{2019}empilement", "Stacking pipeline"),
        kind: "pipeline".to_string(),
        required: false,
        state: match env_state {
            ToolState::Present if siril_ok => ToolState::Present,
            ToolState::Present => ToolState::Missing,
            other => other,
        },
        version: env.and_then(|r| r.version.clone()),
        min_version: None,
        path: env.and_then(|r| r.path.clone()),
        page: env.and_then(|r| r.page.clone()),
        licence: Some("MIT".to_string()),
        download: None,
        download_bytes: None,
        partial_bytes: None,
        progression_compatible: None,
        interrupted: env.is_some_and(|r| r.interrupted),
        installable: env.is_some_and(|r| r.installable),
        blocked_by,
    }
}

fn row<'a>(rows: &'a [ToolRow], id: &str) -> Option<&'a ToolRow> {
    rows.iter().find(|r| r.id == id)
}

fn present(rows: &[ToolRow], id: &str) -> bool {
    row(rows, id).is_some_and(|r| r.state == ToolState::Present)
}

/// What the treatments read ([`crate::precheck::treatments`]): ASTAP with
/// its database, a Siril the progression knows, the stacking pipeline.
#[must_use]
pub fn tools_present(rows: &[ToolRow]) -> ToolsPresent {
    ToolsPresent {
        stacking_pipeline: present(rows, "stacking-pipeline"),
        astap: present(rows, "astap") && present(rows, "astap-database"),
        siril: present(rows, "siril")
            && row(rows, "siril").is_some_and(|r| r.progression_compatible == Some(true)),
    }
}

/// The solvers for [`crate::sky_solution`].
#[must_use]
pub fn solvers(rows: &[ToolRow]) -> Solvers {
    let tool = |id: &str| {
        row(rows, id)
            .filter(|r| r.state == ToolState::Present)
            .and_then(|r| {
                Some(SolverTool {
                    path: r.path.clone()?,
                    version: r.version.clone(),
                    database: None,
                })
            })
    };
    let mut astap = tool("astap").filter(|_| present(rows, "astap-database"));
    if let Some(astap) = astap.as_mut() {
        astap.database = row(rows, "astap-database").and_then(|r| r.path.clone());
    }
    Solvers {
        astap,
        astrometry_net: tool("astrometry-net"),
    }
}

/// The Siril the progression runs: a present one whose version it knows.
#[must_use]
pub fn siril_for_progression(rows: &[ToolRow]) -> Option<(PathBuf, String)> {
    row(rows, "siril")
        .filter(|r| r.state == ToolState::Present && r.progression_compatible == Some(true))
        .and_then(|r| Some((r.path.clone()?, r.version.clone()?)))
}

/* --------------------------------- Download ------------------------------ */

/// The last segment of a URL's path, made safe as a file name.
#[must_use]
pub fn file_name_of(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let last = path.rsplit('/').next().unwrap_or_default();
    let clean: String = last
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .collect();
    if clean.is_empty() || clean.starts_with('.') {
        "download.bin".to_string()
    } else {
        clean
    }
}

/// Fetches a URL from byte `from`: answers where its bytes start (`from`
/// when the server resumed, `0` when it sends the whole file again), the
/// total size when known, and the bytes.
pub trait ToolFetcher {
    fn fetch(&self, url: &str, from: u64) -> Result<Fetched, String>;
}

pub struct Fetched {
    pub start: u64,
    pub total: Option<u64>,
    pub body: Box<dyn Read + Send>,
}

/// HTTPS, with `Range` to resume.
pub struct HttpToolFetcher {
    client: reqwest::blocking::Client,
}

impl HttpToolFetcher {
    pub fn new() -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(None)
            // The official host hands the file to a mirror: follow that, but
            // only to another https address (the digest decides the rest).
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 10 {
                    attempt.error("too many redirections")
                } else if attempt.url().scheme() == "https" {
                    attempt.follow()
                } else {
                    attempt.error("a redirection to an address that is not https")
                }
            }))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { client })
    }
}

impl ToolFetcher for HttpToolFetcher {
    fn fetch(&self, url: &str, from: u64) -> Result<Fetched, String> {
        let mut request = self.client.get(url);
        if from > 0 {
            request = request.header("Range", format!("bytes={from}-"));
        }
        let response = request
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .map_err(|e| e.to_string())?;
        let resumed = response.status().as_u16() == 206;
        let length = response.content_length();
        let start = if resumed { from } else { 0 };
        Ok(Fetched {
            start,
            total: length.map(|l| l + start),
            body: Box::new(response),
        })
    }
}

/// How far a download is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub bytes: u64,
    pub total: Option<u64>,
}

/// Why a download stopped ("The connection was interrupted at 184 MB of
/// 410 MB." "What is already downloaded is kept.").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum DownloadFailure {
    /// The registry has no file for this system: the manual page only.
    NoAutomaticDownload,
    /// The connection stopped: what came is kept, the next try resumes.
    Interrupted {
        kept_bytes: u64,
        total: Option<u64>,
        detail: String,
    },
    Cancelled {
        kept_bytes: u64,
    },
    /// The file is not the registry's: deleted, never used.
    DigestMismatch,
    /// The file is the registry's but could not be unpacked into
    /// `tools/<id>/` (a real failure: the file is kept, the next try
    /// unpacks it again without fetching it).
    Extraction {
        detail: String,
    },
    Io {
        detail: String,
    },
}

/// A download finished and checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Downloaded {
    pub path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
    /// Already there with the right digest: nothing fetched.
    pub already_there: bool,
}

fn sha256_of(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Downloads `download` into `<data>/tools/<id>/`, resuming a `.part` left
/// by an earlier try, and keeps it only when its SHA-256 is the registry's.
pub fn download_tool(
    data_dir: &Path,
    id: &str,
    download: Option<&Download>,
    fetcher: &dyn ToolFetcher,
    cancel: Option<&CancelFlag>,
    progress: &mut dyn FnMut(DownloadProgress),
) -> Result<Downloaded, DownloadFailure> {
    let download = download.ok_or(DownloadFailure::NoAutomaticDownload)?;
    let io = |error: std::io::Error| DownloadFailure::Io {
        detail: error.to_string(),
    };
    let dir = data_dir.join("tools").join(id);
    fs::create_dir_all(&dir).map_err(io)?;
    let name = file_name_of(&download.url);
    let done = dir.join(&name);
    if done.is_file() && sha256_of(&done).map_err(io)? == download.sha256 {
        let bytes = fs::metadata(&done).map_err(io)?.len();
        return Ok(Downloaded {
            path: done,
            sha256: download.sha256.clone(),
            bytes,
            already_there: true,
        });
    }
    let part = dir.join(format!("{name}.part"));
    let from = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    let fetched =
        fetcher
            .fetch(&download.url, from)
            .map_err(|detail| DownloadFailure::Interrupted {
                kept_bytes: from,
                total: download.bytes,
                detail,
            })?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(fetched.start == from && from > 0)
        .truncate(!(fetched.start == from && from > 0))
        .open(&part)
        .map_err(io)?;
    let total = fetched.total.or(download.bytes);
    let mut bytes = fetched.start.min(from);
    let mut body = fetched.body;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        if cancel.is_some_and(CancelFlag::is_cancelled) {
            return Err(DownloadFailure::Cancelled { kept_bytes: bytes });
        }
        let n = match body.read(&mut buffer) {
            Ok(n) => n,
            Err(error) => {
                return Err(DownloadFailure::Interrupted {
                    kept_bytes: bytes,
                    total,
                    detail: error.to_string(),
                });
            }
        };
        if n == 0 {
            break;
        }
        file.write_all(&buffer[..n]).map_err(io)?;
        bytes += n as u64;
        progress(DownloadProgress { bytes, total });
    }
    drop(file);
    if total.is_some_and(|total| bytes < total) {
        return Err(DownloadFailure::Interrupted {
            kept_bytes: bytes,
            total,
            detail: "the connection closed early".to_string(),
        });
    }
    if sha256_of(&part).map_err(io)? != download.sha256 {
        let _ = fs::remove_file(&part);
        return Err(DownloadFailure::DigestMismatch);
    }
    fs::rename(&part, &done).map_err(io)?;
    Ok(Downloaded {
        path: done,
        sha256: download.sha256.clone(),
        bytes,
        already_there: false,
    })
}

/* --------------------------------- Install ------------------------------- */

/// Whether a registry URL names a ZIP, which is unpacked into
/// `tools/<id>/` once its digest is checked ([`install_download`]).
#[must_use]
pub fn is_archive(url: &str) -> bool {
    file_name_of(url).to_ascii_lowercase().ends_with(".zip")
}

/// Unpacks `archive` into `destination` through a staging folder, so that a
/// stop in the middle never leaves half a star database where detection
/// looks. Refuses an entry that leaves the folder or is a link.
fn unpack_zip(archive: &Path, destination: &Path) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let stage = destination.join(".installing");
    let _ = fs::remove_dir_all(&stage);
    fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    let result = (|| -> Result<(), String> {
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
            let relative = entry
                .enclosed_name()
                .ok_or_else(|| format!("unsafe path in the archive: {:?}", entry.name()))?;
            if relative
                .components()
                .next()
                .is_some_and(|c| c.as_os_str() == "__MACOSX")
            {
                continue;
            }
            if entry.is_symlink() {
                return Err(format!("a link in the archive: {:?}", entry.name()));
            }
            let target = stage.join(&relative);
            if entry.is_dir() {
                fs::create_dir_all(&target).map_err(|e| e.to_string())?;
                continue;
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut out = fs::File::create(&target).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
            drop(out);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = entry.unix_mode().map_or(0o644, |m| m & 0o755) | 0o600;
                fs::set_permissions(&target, fs::Permissions::from_mode(mode))
                    .map_err(|e| e.to_string())?;
            }
        }
        // Move what was unpacked into place (the archive's top-level names).
        for item in fs::read_dir(&stage).map_err(|e| e.to_string())? {
            let item = item.map_err(|e| e.to_string())?;
            let target = destination.join(item.file_name());
            if target.is_dir() {
                let _ = fs::remove_dir_all(&target);
            } else {
                let _ = fs::remove_file(&target);
            }
            fs::rename(item.path(), &target).map_err(|e| e.to_string())?;
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(&stage);
    result
}

/// macOS, for a program that came through Astro Sync's own download: that
/// download carries no quarantine mark (only a browser or the Finder puts
/// one, and Gatekeeper reads only that mark), so nothing asks the person to
/// open it. The mark is removed all the same, in case one is there, and a
/// program Apple silicon would refuse for want of a signature is given an
/// ad hoc one (what ASTAP's page asks the person to do by hand with
/// `codesign`). Never fails the installation: a program that still does not
/// start is told by the detection, which is asked for its version.
#[cfg(target_os = "macos")]
fn prepare_for_macos(folder: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = Command::new("/usr/bin/xattr")
        .args(["-dr", "com.apple.quarantine"])
        .arg(folder)
        .output();
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let executable =
            fs::metadata(&path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0);
        if !executable {
            continue;
        }
        let signed = Command::new("/usr/bin/codesign")
            .arg("--verify")
            .arg(&path)
            .output()
            .is_ok_and(|o| o.status.success());
        if !signed {
            let _ = Command::new("/usr/bin/codesign")
                .args(["--force", "-s", "-"])
                .arg(&path)
                .output();
        }
    }
}

/// Puts a checked download where Astro Sync looks for it: a ZIP is
/// unpacked into `<data>/tools/<id>/` (no administrator right, nothing
/// outside the settings folder) and then deleted; any other file stays where
/// it was fetched, for the person to install. The answer's `path` is the
/// folder for an archive.
pub fn install_download(
    data_dir: &Path,
    id: &str,
    downloaded: Downloaded,
) -> Result<Downloaded, DownloadFailure> {
    let is_zip = downloaded
        .path
        .file_name()
        .is_some_and(|name| is_archive(&name.to_string_lossy()));
    if !is_zip {
        return Ok(downloaded);
    }
    let destination = data_dir.join("tools").join(id);
    unpack_zip(&downloaded.path, &destination)
        .map_err(|detail| DownloadFailure::Extraction { detail })?;
    #[cfg(target_os = "macos")]
    prepare_for_macos(&destination);
    let _ = fs::remove_file(&downloaded.path);
    Ok(Downloaded {
        path: destination,
        ..downloaded
    })
}

/// [`download_tool`] then [`install_download`]: what "Install" and the
/// command line do.
pub fn download_and_install(
    data_dir: &Path,
    id: &str,
    download: Option<&Download>,
    fetcher: &dyn ToolFetcher,
    cancel: Option<&CancelFlag>,
    progress: &mut dyn FnMut(DownloadProgress),
) -> Result<Downloaded, DownloadFailure> {
    let downloaded = download_tool(data_dir, id, download, fetcher, cancel, progress)?;
    install_download(data_dir, id, downloaded)
}

/// "Install all": the required rows that are missing and have a
/// download for this system.
#[must_use]
pub fn to_install(rows: &[ToolRow]) -> Vec<&ToolRow> {
    rows.iter()
        .filter(|r| r.required && matches!(r.state, ToolState::Missing | ToolState::Outdated))
        .filter(|r| r.download.is_some())
        .collect()
}

/// The id of the row a download is shown on: ASTAP's star database is part
/// of the row "ASTAP and its indexes".
#[must_use]
pub fn row_of(id: &str) -> &str {
    if id == DATABASE_ID { "astap" } else { id }
}

/// What "Install" on the row `id` fetches, in order: for ASTAP the
/// program and then the star database, each only when missing and with a
/// download for this system; for any other row, itself.
#[must_use]
pub fn install_plan<'a>(rows: &'a [ToolRow], id: &str) -> Vec<&'a ToolRow> {
    let wanted = |r: &&ToolRow| {
        r.download.is_some() && matches!(r.state, ToolState::Missing | ToolState::Outdated)
    };
    if id == "astap" {
        return rows
            .iter()
            .filter(|r| r.id == "astap" || r.id == DATABASE_ID)
            .filter(wanted)
            .collect();
    }
    rows.iter().filter(|r| r.id == id).collect()
}

#[cfg(test)]
mod tests;

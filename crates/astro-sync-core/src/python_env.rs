//! The stacking's Python environment, as a tool of "Dependencies"
//! (heavy dependencies installed and checked, never bundled; the registry's
//! entry `python-env`).
//!
//! - **The scripts** ship with Astro Sync, beside the app
//!   ([`StackingResources`]: `astro_sync_stack.py`, `check_environment.py`,
//!   `requirements.txt` pinned by version and SHA-256, the shipped
//!   pipelines). They are light; the packages they import are not.
//! - **Detection** ([`environment_status`]): the environment lives in Astro
//!   Sync's data (`stacking-env/`). It is present when its interpreter is
//!   there and the mark written after a successful check
//!   (`astro-sync-env.json`) names the `requirements.txt` of this Astro Sync
//!   (its SHA-256): a newer Astro Sync with other pins says « outdated ». An
//!   environment without its mark is an installation that did not finish
//!   ("Resume the installation"). Nothing is run to say so: the full check
//!   is [`verify_environment`] ("Check again").
//! - **A Python in the range** ([`find_python`]): the registry's
//!   `minVersion` of `python-env` (`>=3.12,<3.14`), the newest found among
//!   `python3.N` and `python3` in the usual places (`PATH`, Homebrew,
//!   python.org's installations). None: the manual page (python.org).
//! - **Installation** ([`install_environment`]): `python -m venv`, then
//!   `pip install --require-hashes --only-binary=:all: -r requirements.txt`
//!   (every file checked against its digest, no build from source), then
//!   `check_environment.py --json`; each step reports its progress, a stop
//!   kills it, and a second try resumes: an existing environment is kept,
//!   pip finds what is already installed.
//! - Every run of the scripts is `python -B -E -s -X utf8`: no bytecode
//!   written beside the scripts (they may be inside a signed application),
//!   no `PYTHON*` variable of the session, no user site, UTF-8 on the pipes
//!   and files whatever the system's code page.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::external_process;
use crate::local_work::CancelFlag;
use crate::tools::{self, DetectContext, ToolState};

/// The environment's folder in Astro Sync's data.
pub const ENV_DIR: &str = "stacking-env";

/// The mark written in the environment after a successful check.
pub const MARKER: &str = "astro-sync-env.json";

/// The flags of every run of the stacking's scripts (see the module
/// documentation). `-X utf8` is a command-line option, so `-E` does not
/// ignore it: without it Python on Windows writes a pipe in the system
/// code page (cp1252, cp932...), which the reading side cannot decode as
/// UTF-8 and which cannot hold every character of a path or a target name.
pub const PYTHON_FLAGS: [&str; 5] = ["-B", "-E", "-s", "-X", "utf8"];

/// The stacking's scripts beside the app (`<resources>/stacking/`), or the
/// repository's `stacking/` for the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackingResources {
    pub dir: PathBuf,
}

impl StackingResources {
    /// The folder, when it holds the three files the environment needs.
    #[must_use]
    pub fn find(dir: &Path) -> Option<Self> {
        let resources = Self {
            dir: dir.to_path_buf(),
        };
        (resources.script().is_file()
            && resources.check_script().is_file()
            && resources.requirements().is_file())
        .then_some(resources)
    }

    #[must_use]
    pub fn script(&self) -> PathBuf {
        self.dir.join("astro_sync_stack.py")
    }

    #[must_use]
    pub fn check_script(&self) -> PathBuf {
        self.dir.join("check_environment.py")
    }

    #[must_use]
    pub fn requirements(&self) -> PathBuf {
        self.dir.join("requirements.txt")
    }

    /// The number of pinned packages (`name==version` lines).
    #[must_use]
    pub fn package_count(&self) -> u32 {
        fs::read_to_string(self.requirements())
            .unwrap_or_default()
            .lines()
            .filter(|line| {
                let line = line.trim_start();
                !line.starts_with('#') && line.contains("==")
            })
            .count() as u32
    }

    fn requirements_sha256(&self) -> Option<String> {
        fs::read(self.requirements())
            .ok()
            .map(|bytes| format!("{:x}", Sha256::digest(&bytes)))
    }
}

/// `<data>/stacking-env`.
#[must_use]
pub fn env_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(ENV_DIR)
}

/// The environment's interpreter.
#[must_use]
pub fn env_python(data_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        env_dir(data_dir).join("Scripts").join("python.exe")
    } else {
        env_dir(data_dir).join("bin").join("python3")
    }
}

/* --------------------------------- The range ----------------------------- */

/// The Python versions the pinned packages support (`>=3.12,<3.14`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PythonRange {
    pub min: (u32, u32),
    pub below: (u32, u32),
}

impl PythonRange {
    #[must_use]
    pub fn contains(&self, version: &str) -> bool {
        let mut parts = version.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
        let major = parts.next().unwrap_or(0);
        let minor = parts.next().unwrap_or(0);
        self.min <= (major, minor) && (major, minor) < self.below
    }

    /// "3.12 to 3.13": the first and last minor versions accepted.
    #[must_use]
    pub fn first_and_last(&self) -> (String, String) {
        let last = if self.below.1 == 0 {
            (self.below.0 - 1, 99)
        } else {
            (self.below.0, self.below.1 - 1)
        };
        (
            format!("{}.{}", self.min.0, self.min.1),
            format!("{}.{}", last.0, last.1),
        )
    }
}

fn major_minor(text: &str) -> Option<(u32, u32)> {
    let mut parts = text.trim().split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

/// Reads `>=3.12,<3.14`.
#[must_use]
pub fn parse_range(text: &str) -> Option<PythonRange> {
    let mut min = None;
    let mut below = None;
    for part in text.split(',') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix(">=") {
            min = major_minor(rest);
        } else if let Some(rest) = part.strip_prefix('<') {
            below = major_minor(rest);
        }
    }
    Some(PythonRange {
        min: min?,
        below: below?,
    })
}

/// The range of the registry's `python-env` (it ships with Astro Sync).
#[must_use]
pub fn python_range() -> PythonRange {
    tools::registry()
        .entry("python-env")
        .and_then(|entry| entry.min_version.as_deref())
        .and_then(parse_range)
        .unwrap_or(PythonRange {
            min: (3, 12),
            below: (3, 14),
        })
}

/* ------------------------------ Finding Python --------------------------- */

/// A Python in the range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundPython {
    pub path: PathBuf,
    pub version: String,
}

const PYTHON_PATTERN: &str = r"(?i)python[^\d\n]*(\d+(?:\.\d+)+)";

fn python_names(range: PythonRange) -> Vec<String> {
    let mut names = Vec::new();
    if range.min.0 == range.below.0 {
        for minor in (range.min.1..range.below.1).rev() {
            names.push(format!("python{}.{minor}", range.min.0));
        }
    }
    names.push("python3".to_string());
    names.push("python".to_string());
    names
        .into_iter()
        .map(|name| format!("{name}{}", std::env::consts::EXE_SUFFIX))
        .collect()
}

/// python.org's installations, by system.
#[must_use]
pub fn python_install_locations(range: PythonRange) -> Vec<PathBuf> {
    let mut places = Vec::new();
    if range.min.0 != range.below.0 {
        return places;
    }
    for minor in (range.min.1..range.below.1).rev() {
        let version = format!("{}.{minor}", range.min.0);
        if cfg!(target_os = "macos") {
            places.push(PathBuf::from(format!(
                "/Library/Frameworks/Python.framework/Versions/{version}/bin/python3"
            )));
            places.push(PathBuf::from(format!(
                "/opt/homebrew/opt/python@{version}/bin/python{version}"
            )));
        } else if cfg!(windows) {
            let folder = format!("Python{}{minor}", range.min.0);
            if let Some(local) = std::env::var_os("LOCALAPPDATA") {
                places.push(
                    PathBuf::from(local)
                        .join("Programs")
                        .join("Python")
                        .join(&folder)
                        .join("python.exe"),
                );
            }
            places.push(PathBuf::from(format!(
                r"C:\Program Files\{folder}\python.exe"
            )));
        }
    }
    places
}

/// The newest Python in `range` among the places of `context` (its
/// `search_dirs`, then `known_paths["python"]`), never the environment's
/// own interpreter.
#[must_use]
pub fn find_python(context: &DetectContext, range: PythonRange) -> Option<FoundPython> {
    let names = python_names(range);
    let own = env_dir(&context.data_dir);
    let mut candidates: Vec<PathBuf> = Vec::new();
    for dir in &context.search_dirs {
        for name in &names {
            candidates.push(dir.join(name));
        }
    }
    candidates.extend(
        context
            .known_paths
            .get("python")
            .into_iter()
            .flatten()
            .cloned(),
    );
    let mut best: Option<FoundPython> = None;
    let mut seen: Vec<PathBuf> = Vec::new();
    for candidate in candidates {
        if !candidate.is_file() || candidate.starts_with(&own) || seen.contains(&candidate) {
            continue;
        }
        seen.push(candidate.clone());
        let mut command = Command::new(&candidate);
        command.arg("--version");
        let Ok(end) = external_process::run(command, context.probe_timeout, None) else {
            continue;
        };
        let output = format!("{}\n{}", end.stdout, end.stderr);
        let Some(version) = tools::read_version(PYTHON_PATTERN, &output) else {
            continue;
        };
        if !range.contains(&version) {
            continue;
        }
        if best
            .as_ref()
            .is_none_or(|b| !tools::at_least(&b.version, &version))
        {
            best = Some(FoundPython {
                path: candidate,
                version,
            });
        }
    }
    best
}

/* --------------------------------- Detection ----------------------------- */

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Marker {
    format: u32,
    python_version: String,
    requirements_sha256: String,
}

/// The environment as "Dependencies" shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvStatus {
    /// `present`, `missing` or `outdated` (other pins than this Astro
    /// Sync's).
    pub state: ToolState,
    /// The environment's Python, when it was checked.
    pub python_version: Option<String>,
    /// An installation that did not finish ("Resume the installation").
    pub interrupted: bool,
    pub path: Option<PathBuf>,
}

/// The version of the Python an environment was made with: `version` (or
/// `version_info`, as uv writes it) of its `pyvenv.cfg`; `None` when it
/// cannot be read.
fn made_with(data_dir: &Path) -> Option<String> {
    let text = fs::read_to_string(env_dir(data_dir).join("pyvenv.cfg")).ok()?;
    text.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        matches!(key.trim(), "version" | "version_info").then(|| value.trim().to_string())
    })
}

fn read_marker(data_dir: &Path) -> Option<Marker> {
    let text = fs::read_to_string(env_dir(data_dir).join(MARKER)).ok()?;
    serde_json::from_str(&text).ok()
}

/// See the module documentation. Runs nothing.
#[must_use]
pub fn environment_status(data_dir: &Path, resources: Option<&StackingResources>) -> EnvStatus {
    let dir = env_dir(data_dir);
    let python = env_python(data_dir);
    let marker = read_marker(data_dir);
    let mut status = EnvStatus {
        state: ToolState::Missing,
        python_version: None,
        interrupted: dir.exists(),
        path: None,
    };
    let (true, Some(marker)) = (python.is_file(), marker) else {
        return status;
    };
    status.interrupted = false;
    status.path = Some(dir);
    status.python_version = Some(marker.python_version.clone());
    let current = resources.and_then(StackingResources::requirements_sha256);
    // A Python the pins no longer support (the range of a newer Astro Sync)
    // makes the environment outdated even with the same pins.
    let out_of_range = !python_range().contains(&marker.python_version);
    status.state = match current {
        Some(current) if current != marker.requirements_sha256 => ToolState::Outdated,
        _ if out_of_range => ToolState::Outdated,
        _ => ToolState::Present,
    };
    status
}

/* ------------------------------- Verification ---------------------------- */

/// What `check_environment.py --json` says of the environment (Siril and
/// StarNet2 are rows of their own).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvCheck {
    pub ok: bool,
    pub python_version: String,
    /// The packages that are missing, at another version or that failed
    /// their call.
    pub problems: Vec<String>,
}

/// Why the environment could not be made or checked (said in words by the
/// window).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum EnvFailure {
    /// No Python in the range on this computer: python.org.
    NoPython {
        first: String,
        last: String,
    },
    /// The scripts are not beside the app.
    NoResources,
    /// `python -m venv` failed.
    Venv {
        detail: String,
    },
    /// `pip install` failed (no connection, a digest refused…): what is
    /// installed is kept, the next try resumes.
    Packages {
        detail: String,
    },
    /// The check found a problem.
    Check {
        detail: String,
        problems: Vec<String>,
    },
    Cancelled,
}

fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(4);
    lines[start..].join("\n").chars().take(600).collect()
}

/// The command running `script` of the resources with the environment's
/// Python.
#[must_use]
pub fn script_command(python: &Path, script: &Path) -> Command {
    let mut command = Command::new(python);
    command.args(PYTHON_FLAGS).arg(script);
    // The scripts stamp the TIFF files they write with « Astro Sync <version> »
    // (`software_tag.py`), never the library's own name.
    command.env(SOFTWARE_VERSION_VARIABLE, crate::version());
    command
}

/// The variable that tells the stacking scripts which version of Astro Sync
/// runs them (`stacking/software_tag.py`).
pub const SOFTWARE_VERSION_VARIABLE: &str = "ASTRO_SYNC_VERSION";

/// Reads the answer of `check_environment.py --json`.
#[must_use]
pub fn read_check(stdout: &str) -> Option<EnvCheck> {
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
    let python = value.get("python")?;
    let python_version = python.get("version")?.as_str()?.to_string();
    let python_ok = python.get("ok").and_then(serde_json::Value::as_bool) == Some(true);
    let mut problems = Vec::new();
    if !python_ok {
        problems.push(format!("Python {python_version}"));
    }
    for package in value
        .get("packages")
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
    {
        if package.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
            problems.push(
                package
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("?")
                    .to_string(),
            );
        }
    }
    Some(EnvCheck {
        ok: problems.is_empty(),
        python_version,
        problems,
    })
}

/// "Check again": `check_environment.py --json` in the
/// environment; the mark is written when it is fine, removed otherwise.
pub fn verify_environment(
    data_dir: &Path,
    resources: &StackingResources,
    timeout: Duration,
) -> Result<EnvCheck, EnvFailure> {
    let python = env_python(data_dir);
    if !python.is_file() {
        return Err(EnvFailure::Check {
            detail: "no environment".to_string(),
            problems: Vec::new(),
        });
    }
    let mut command = script_command(&python, &resources.check_script());
    command.arg("--json");
    let end = external_process::run(command, timeout, None).map_err(|e| EnvFailure::Check {
        detail: e.to_string(),
        problems: Vec::new(),
    })?;
    let marker = env_dir(data_dir).join(MARKER);
    let Some(check) = read_check(&end.stdout) else {
        let _ = fs::remove_file(&marker);
        return Err(EnvFailure::Check {
            detail: tail(&format!("{}\n{}", end.stdout, end.stderr)),
            problems: Vec::new(),
        });
    };
    if !check.ok {
        let _ = fs::remove_file(&marker);
        return Err(EnvFailure::Check {
            detail: check.problems.join(", "),
            problems: check.problems,
        });
    }
    let mark = Marker {
        format: 1,
        python_version: check.python_version.clone(),
        requirements_sha256: resources.requirements_sha256().unwrap_or_default(),
    };
    fs::write(
        &marker,
        serde_json::to_string_pretty(&mark).expect("the mark serializes"),
    )
    .map_err(|e| EnvFailure::Check {
        detail: e.to_string(),
        problems: Vec::new(),
    })?;
    Ok(check)
}

/* ------------------------------- Installation ---------------------------- */

/// A step of the installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvStep {
    /// A Python in the range is looked for.
    Python,
    /// `python -m venv`.
    Environment,
    /// `pip install`.
    Packages,
    /// `check_environment.py`.
    Check,
}

/// How far the installation is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvProgress {
    pub step: EnvStep,
    pub index: u32,
    pub total: u32,
    /// Packages seen by pip (collected or already there) of `packages_total`.
    pub packages_done: u32,
    pub packages_total: u32,
}

/// How long each step may take.
#[derive(Debug, Clone, Copy)]
pub struct EnvTimeouts {
    pub venv: Duration,
    pub packages: Duration,
    pub check: Duration,
}

impl Default for EnvTimeouts {
    fn default() -> Self {
        Self {
            venv: Duration::from_secs(5 * 60),
            packages: Duration::from_secs(45 * 60),
            check: Duration::from_secs(5 * 60),
        }
    }
}

/// The pip arguments: pinned and checked, binary wheels only.
#[must_use]
pub fn pip_args(requirements: &Path) -> Vec<String> {
    [
        "-m",
        "pip",
        "install",
        "--require-hashes",
        "--only-binary=:all:",
        "--disable-pip-version-check",
        "--no-input",
        "--progress-bar",
        "off",
        "-r",
    ]
    .iter()
    .map(ToString::to_string)
    .chain([requirements.display().to_string()])
    .collect()
}

/// Creates (or resumes) the environment and checks it (see the module
/// documentation).
pub fn install_environment(
    context: &DetectContext,
    resources: Option<&StackingResources>,
    timeouts: EnvTimeouts,
    cancel: &CancelFlag,
    progress: &mut dyn FnMut(EnvProgress),
) -> Result<EnvCheck, EnvFailure> {
    let resources = resources.ok_or(EnvFailure::NoResources)?;
    let packages_total = resources.package_count();
    let mut report = |step: EnvStep, index: u32, packages_done: u32| {
        progress(EnvProgress {
            step,
            index,
            total: 4,
            packages_done,
            packages_total,
        });
    };
    let python = env_python(&context.data_dir);
    report(EnvStep::Python, 1, 0);
    // An environment made with a Python the pins no longer support (the range
    // of a newer Astro Sync) cannot take them: it is made again.
    if python.is_file()
        && made_with(&context.data_dir).is_some_and(|version| !python_range().contains(&version))
    {
        fs::remove_dir_all(env_dir(&context.data_dir)).map_err(|e| EnvFailure::Venv {
            detail: e.to_string(),
        })?;
    }
    if !python.is_file() {
        let range = python_range();
        let Some(found) = find_python(context, range) else {
            let (first, last) = range.first_and_last();
            return Err(EnvFailure::NoPython { first, last });
        };
        report(EnvStep::Environment, 2, 0);
        fs::create_dir_all(&context.data_dir).map_err(|e| EnvFailure::Venv {
            detail: e.to_string(),
        })?;
        let mut command = Command::new(&found.path);
        command.args(["-m", "venv"]).arg(env_dir(&context.data_dir));
        let end = external_process::run(command, timeouts.venv, Some(cancel)).map_err(|e| {
            EnvFailure::Venv {
                detail: e.to_string(),
            }
        })?;
        if end.cancelled {
            return Err(EnvFailure::Cancelled);
        }
        if !end.succeeded() || !python.is_file() {
            return Err(EnvFailure::Venv {
                detail: tail(&format!("{}\n{}", end.stdout, end.stderr)),
            });
        }
    } else {
        report(EnvStep::Environment, 2, 0);
    }
    // A mark of an older installation is not the truth any more.
    let _ = fs::remove_file(env_dir(&context.data_dir).join(MARKER));

    report(EnvStep::Packages, 3, 0);
    let mut command = Command::new(&python);
    command.args(pip_args(&resources.requirements()));
    let mut done = 0;
    let end =
        external_process::run_streaming(command, timeouts.packages, Some(cancel), &mut |line| {
            let line = line.trim_start();
            if line.starts_with("Collecting ") || line.starts_with("Requirement already satisfied")
            {
                done = (done + 1).min(packages_total.max(1));
                progress(EnvProgress {
                    step: EnvStep::Packages,
                    index: 3,
                    total: 4,
                    packages_done: done,
                    packages_total,
                });
            }
        })
        .map_err(|e| EnvFailure::Packages {
            detail: e.to_string(),
        })?;
    if end.cancelled {
        return Err(EnvFailure::Cancelled);
    }
    if !end.succeeded() {
        return Err(EnvFailure::Packages {
            detail: tail(&format!("{}\n{}", end.stdout, end.stderr)),
        });
    }

    progress(EnvProgress {
        step: EnvStep::Check,
        index: 4,
        total: 4,
        packages_done: packages_total,
        packages_total,
    });
    verify_environment(&context.data_dir, resources, timeouts.check)
}

#[cfg(all(test, unix))]
mod tests;

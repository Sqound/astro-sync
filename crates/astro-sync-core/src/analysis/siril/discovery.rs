//! Siril discovery (point 1): standard install locations per OS, a `PATH`
//! search for `siril`/`siril-cli`, an optional manually-selected path, and
//! identity/version verification by actually running
//! `<candidate> --version` and matching the output against
//! [`super::versions::compatible_versions`].
//!
//! Running the candidate is the only way to *verify identity* rather than
//! merely guess from a file's location -- a path that happens to exist is
//! not evidence it is Siril, let alone a compatible version of it. This is
//! also exactly the seam the plan requires stay testable without a real
//! Siril install: `detect` never knows or cares whether the executable at a
//! given path is the genuine application or a test fixture script that
//! mimics `--version` (see this module's tests).

use std::path::{Path, PathBuf};
use std::process::Command;

use super::versions::{compatible_versions, extract_version, is_compatible};

/// The two executable names every OS's package or the project itself might
/// install Siril's headless entry point under.
const BINARY_NAMES: [&str; 2] = ["siril", "siril-cli"];

/// Non-`PATH` install locations worth probing directly, per OS -- task
/// 9.4's own list: "macOS `/Applications/Siril.app/Contents/MacOS/siril`,
/// Linux `siril`/`siril-cli` dans le PATH, Windows `Program Files`". Linux
/// has no dedicated entry here because it has no single conventional
/// install path outside `PATH` (a distro package, Flatpak, or AppImage all
/// end up on `PATH` differently) -- [`find_on_path`] already covers it.
#[must_use]
pub fn standard_install_locations() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        vec![PathBuf::from(
            "/Applications/Siril.app/Contents/MacOS/siril",
        )]
    }
    #[cfg(target_os = "windows")]
    {
        // `siril-cli.exe` first: `siril.exe` is the window program, which may not
        // write to a pipe (to be checked), and the installer puts neither on `PATH`.
        vec![
            PathBuf::from(r"C:\Program Files\SiriL\bin\siril-cli.exe"),
            PathBuf::from(r"C:\Program Files\SiriL\bin\siril.exe"),
            PathBuf::from(r"C:\Program Files\SiriL\siril.exe"),
        ]
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Vec::new()
    }
}

/// Searches every directory on `PATH` for `siril` or `siril-cli`, in `PATH`
/// order, both names per directory. Dependency-free: `std::env::split_paths`
/// already knows the platform's `PATH` separator (`:` / `;`).
#[must_use]
pub fn find_on_path() -> Vec<PathBuf> {
    let Some(path_var) = std::env::var_os("PATH") else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for dir in std::env::split_paths(&path_var) {
        for name in BINARY_NAMES {
            let candidate = dir.join(name);
            if candidate.is_file() {
                found.push(candidate);
            }
        }
    }
    found
}

/// The full, ordered list of candidates [`detect`] tries: a manually
/// selected path first ("manual selection possible"), then the OS's
/// standard locations, then `PATH`.
#[must_use]
pub fn candidate_locations(manual_path: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(manual) = manual_path {
        candidates.push(manual.to_path_buf());
    }
    candidates.extend(standard_install_locations());
    candidates.extend(find_on_path());
    candidates
}

/// The outcome of trying to run a candidate's `--version`: either it ran
/// and printed something [`extract_version`] could read, or it did not run
/// at all (missing, not executable, or produced no recognizable version) --
/// the two cases stay indistinguishable to every caller
/// (a wrong path and a wrong binary both simply mean "Siril not found
/// here"), so this stays private to this module.
fn run_version_probe(candidate: &Path) -> Option<String> {
    let mut command = Command::new(candidate);
    crate::external_process::hide_console(&mut command);
    let output = command.arg("--version").output().ok()?;
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    extract_version(&combined)
}

/// Why [`detect`] did or did not consider Siril available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SirilStatus {
    /// A candidate ran and reported a version on the whitelist.
    Compatible,
    /// A candidate ran and reported a version, but it is not on the
    /// whitelist -- still identified as Siril, just not a version this
    /// crate has verified the progression orchestration against.
    IncompatibleVersion,
    /// No candidate could be run and produce a recognizable version --
    /// covers "nothing at any candidate path", "path exists but is not
    /// executable", and "ran but printed nothing `extract_version`
    /// recognizes".
    NotFound,
}

/// The full result of a detection pass, serializable end to end so the CLI
/// (`siril detect`) and the GUI's Progression screen render exactly the
/// same shape.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SirilDetection {
    pub status: SirilStatus,
    /// The candidate path that actually ran and answered, if any --
    /// `None` only for [`SirilStatus::NotFound`].
    pub path: Option<String>,
    /// The version string [`super::versions::extract_version`] read back,
    /// if any.
    pub version: Option<String>,
    /// The whitelist this detection was checked against, echoed back so a
    /// GUI/CLI caller never has to load `siril_versions.json` itself just
    /// to explain *why* a version was flagged incompatible.
    pub compatible_versions: Vec<String>,
}

/// Tries every candidate in order (see [`candidate_locations`]) and returns
/// the first one that actually runs and reports a version, verified against
/// the whitelist. Trying every candidate rather than stopping at the first
/// *existing* path is what makes this "verify identity", not just "check a
/// file is there": a stale or unrelated executable at a standard location
/// that fails to run falls through to the next candidate instead of
/// reporting `NotFound` prematurely.
#[must_use]
pub fn detect(candidates: &[PathBuf]) -> SirilDetection {
    for candidate in candidates {
        if let Some(version) = run_version_probe(candidate) {
            let status = if is_compatible(&version) {
                SirilStatus::Compatible
            } else {
                SirilStatus::IncompatibleVersion
            };
            return SirilDetection {
                status,
                path: Some(candidate.display().to_string()),
                version: Some(version),
                compatible_versions: compatible_versions(),
            };
        }
    }
    SirilDetection {
        status: SirilStatus::NotFound,
        path: None,
        version: None,
        compatible_versions: compatible_versions(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    /// Writes an executable shell script at `path` that answers
    /// `--version` with `banner` on stdout and exits `0`, and refuses
    /// (exit `1`, no output) any other argument -- the "faux binaire" task
    /// 9.4 explicitly calls for so discovery is fully testable without a
    /// real Siril install.
    #[cfg(unix)]
    fn write_fake_siril(path: &Path, banner: &str) {
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo \"{banner}\"\n  exit 0\nfi\nexit 1\n"
        );
        fs::write(path, script).unwrap();
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn detects_a_compatible_fake_binary() {
        let dir = TempDir::new("siril-discovery-compatible");
        let fake = dir.path().join("fake-siril.sh");
        write_fake_siril(&fake, "Siril 1.2.1");

        let detection = detect(std::slice::from_ref(&fake));
        assert_eq!(detection.status, SirilStatus::Compatible);
        assert_eq!(detection.path.as_deref(), Some(fake.to_str().unwrap()));
        assert_eq!(detection.version.as_deref(), Some("1.2.1"));
        assert!(!detection.compatible_versions.is_empty());
    }

    #[test]
    #[cfg(unix)]
    fn detects_an_incompatible_fake_binary_version() {
        let dir = TempDir::new("siril-discovery-incompatible");
        let fake = dir.path().join("fake-siril.sh");
        write_fake_siril(&fake, "Siril 0.9.9");

        let detection = detect(&[fake]);
        assert_eq!(detection.status, SirilStatus::IncompatibleVersion);
        assert_eq!(detection.version.as_deref(), Some("0.9.9"));
    }

    #[test]
    fn reports_not_found_for_a_nonexistent_binary_path() {
        let dir = TempDir::new("siril-discovery-absent");
        let missing = dir.path().join("does-not-exist");

        let detection = detect(&[missing]);
        assert_eq!(detection.status, SirilStatus::NotFound);
        assert_eq!(detection.path, None);
        assert_eq!(detection.version, None);
    }

    #[test]
    #[cfg(unix)]
    fn falls_through_a_failing_candidate_to_the_next_one() {
        let dir = TempDir::new("siril-discovery-fallthrough");
        let broken = dir.path().join("broken.sh");
        fs::write(&broken, "#!/bin/sh\nexit 1\n").unwrap();
        let mut perms = fs::metadata(&broken).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&broken, perms).unwrap();
        let good = dir.path().join("good.sh");
        write_fake_siril(&good, "Siril 1.4.4");

        let detection = detect(&[broken, good.clone()]);
        assert_eq!(detection.status, SirilStatus::Compatible);
        assert_eq!(detection.path.as_deref(), Some(good.to_str().unwrap()));
    }

    #[test]
    fn candidate_locations_puts_the_manual_path_first() {
        let manual = PathBuf::from("/tmp/manual-siril");
        let candidates = candidate_locations(Some(&manual));
        assert_eq!(candidates.first(), Some(&manual));
    }

    #[test]
    fn candidate_locations_with_no_manual_path_starts_with_standard_locations() {
        let candidates = candidate_locations(None);
        assert_eq!(
            candidates.len() - find_on_path().len(),
            standard_install_locations().len()
        );
    }
}

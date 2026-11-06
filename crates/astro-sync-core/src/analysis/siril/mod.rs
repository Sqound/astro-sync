//! Siril orchestration: discovery of a compatible Siril install
//! ([`discovery`]), deterministic script generation for a cumulative-stack
//! progression ([`script`]), and running that script to produce a recorded
//! [`progression::ProgressionRecord`] ([`progression::run_progression`]).
//!
//! Every rule below is part of the Siril integration of Astro
//! Sync, and none of it is optional:
//!
//! - Siril is a **separately installed** application: this crate never
//!   downloads or bundles it. [`discovery::detect`] only ever looks for an
//!   already-installed binary and verifies its identity and version by
//!   actually running it.
//! - The compatible-version whitelist (`SirilDetection`'s own
//!   `compatible_versions` field) is **data, not code**: it lives in
//!   `siril_versions.json`, next to this module, loaded via `include_str!`
//!   (see [`versions`]).
//! - Registration and calibration run **once** and are reused for every
//!   milestone; milestone 1 is **always a single exposure, never stacked**;
//!   later milestones stack a growing, cumulative prefix of **one**
//!   homogeneous acquisition family (`crate::analysis::fits::AcquisitionFamily`).
//!   See [`script::generate_script`].
//! - **Any Siril failure marks the progression unavailable and never
//!   blocks session synchronization.** [`progression::run_progression`]
//!   never panics and never returns an `Err`; a missing binary, an
//!   incompatible version, or a failed run all become
//!   [`progression::ProgressionOutcome::Unavailable`], a plain value the
//!   caller can simply skip past.
//! - The generated script is written into an isolated working directory,
//!   never inside the scanned source folder, and is preserved on disk
//!   (viewable before and after the run) regardless of outcome.

pub mod discovery;
pub mod progression;
pub mod script;
pub mod versions;

use std::path::Path;

use crate::analysis::fits::AcquisitionFamily;

pub use discovery::{SirilDetection, SirilStatus, candidate_locations, detect};
pub use progression::{
    DisplayScale, MilestoneOutcome, ProgressionOutcome, ProgressionRecord, run_progression,
};
pub use script::{GeneratedScript, Milestone, ProgressionSettings, generate_script};
pub use versions::{compatible_versions, extract_version, is_compatible};

/// Combines [`detect`] and [`run_progression`] behind the one policy every
/// caller (the CLI's `progression` subcommand, the GUI's Progression
/// screen) needs: only actually run Siril when a *compatible* version was
/// found. A missing or incompatible Siril becomes
/// [`ProgressionOutcome::Unavailable`] immediately, with no script written
/// at all -- there is no point generating one for a Siril install this
/// crate has not verified [`script::generate_script`]'s command set
/// against. Kept here rather than duplicated in both the CLI and the GUI
/// command layers, so this policy is decided in exactly one, tested place.
#[must_use]
pub fn detect_and_run_progression(
    source_root: &Path,
    working_dir: &Path,
    family: &AcquisitionFamily,
    manual_siril_path: Option<&Path>,
    settings: &ProgressionSettings,
) -> ProgressionOutcome {
    let candidates = candidate_locations(manual_siril_path);
    let detection = detect(&candidates);
    match (&detection.status, &detection.path, &detection.version) {
        (SirilStatus::Compatible, Some(path), Some(version)) => run_progression(
            source_root,
            working_dir,
            family,
            Path::new(path),
            version,
            settings,
        ),
        _ => ProgressionOutcome::Unavailable {
            reason: unavailable_detection_reason(&detection),
            script: String::new(),
            script_path: String::new(),
        },
    }
}

/// Renders why [`detect_and_run_progression`] refused to even try running
/// Siril, from a [`SirilDetection`] whose status is not
/// [`SirilStatus::Compatible`].
fn unavailable_detection_reason(detection: &SirilDetection) -> String {
    match detection.status {
        SirilStatus::NotFound => {
            "Siril cannot be found (no compatible binary detected).".to_string()
        }
        SirilStatus::IncompatibleVersion => format!(
            "detected Siril version ({}) is not in the allowlist ({})",
            detection.version.as_deref().unwrap_or("?"),
            detection.compatible_versions.join(", ")
        ),
        SirilStatus::Compatible => {
            unreachable!("Compatible is matched separately by the caller")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    fn sample_family() -> AcquisitionFamily {
        AcquisitionFamily {
            key: "Ha_300s_g100_1x1_t-10C".to_string(),
            filter: Some("Ha".to_string()),
            exposure_seconds: Some(300.0),
            gain: Some(100.0),
            binning: Some("1x1".to_string()),
            temperature_class_c: Some(-10),
            camera: None,
            frame_paths: vec!["frame-0001.fits".to_string()],
        }
    }

    /// Uses a fake binary that answers `--version` with an *incompatible*
    /// version, rather than a nonexistent path: `detect` returns on the
    /// first candidate that runs at all, compatible or not, so this stays
    /// deterministic even on a machine that happens to have a real,
    /// compatible Siril installed on `PATH` (as the development machine
    /// this crate was validated on does) --
    /// a nonexistent manual path would otherwise fall through to that real
    /// install and make this test's outcome depend on the host.
    #[test]
    #[cfg(unix)]
    fn refuses_to_run_when_the_only_candidate_is_an_incompatible_version() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let source = TempDir::new("siril-detect-and-run-source");
        let working = TempDir::new("siril-detect-and-run-working");
        let fake = source.path().join("fake-siril.sh");
        fs::write(
            &fake,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"Siril 0.9.9\"; exit 0; fi\nexit 1\n",
        )
        .unwrap();
        let mut perms = fs::metadata(&fake).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&fake, perms).unwrap();

        let outcome = detect_and_run_progression(
            source.path(),
            working.path(),
            &sample_family(),
            Some(&fake),
            &ProgressionSettings::default(),
        );

        match outcome {
            ProgressionOutcome::Unavailable {
                reason,
                script,
                script_path,
            } => {
                assert!(reason.contains("0.9.9"));
                // No script is even generated when detection itself
                // refuses -- unlike `run_progression`'s own failure paths,
                // which always preserve the script it already wrote.
                assert!(script.is_empty());
                assert!(script_path.is_empty());
            }
            ProgressionOutcome::Available(_) => {
                panic!("0.9.9 is not on the whitelist; must be Unavailable")
            }
        }
    }
}

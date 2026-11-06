//! Progression orchestration (points 3-4): writes the script
//! [`super::script::generate_script`] renders into an isolated working
//! directory (never inside the scanned source folder), runs Siril against
//! it, and turns the outcome into a [`ProgressionOutcome`] that is *never*
//! an error the caller has to handle specially -- a missing binary, an
//! incompatible run, a nonzero exit code, or a milestone Siril did not
//! produce all collapse into [`ProgressionOutcome::Unavailable`] with a
//! human-readable reason. That is the whole of the failure policy:
//! "any Siril failure marks the progression unavailable and never blocks
//! session synchronization". The generated script itself is always written
//! to disk and always returned, in both outcomes, so it stays inspectable
//! (in the GUI's Progression screen) before and after the run even when the
//! run itself failed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::analysis::fits::{self, AcquisitionFamily};
use crate::paths::is_within;
use crate::time_util::now_iso8601_utc;

use super::script::{ProgressionSettings, generate_script, milestone_file_name};

/// The pixel dimensions of the *final* milestone's stacked output, read
/// back from its own FITS header (`NAXIS1`/`NAXIS2` -- a header field, not
/// pixel data, so this stays inside the same "header only" v1 scope
/// `analysis::fits` already commits to) and recorded once so every earlier
/// milestone can be displayed at the same scale (point 3: "display scale
/// of the milestone images derived from the final stack").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayScale {
    pub width_pixels: i64,
    pub height_pixels: i64,
}

/// One milestone's recorded outcome: the plan (`script::Milestone`) plus
/// where its output actually landed on disk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MilestoneOutcome {
    pub index: usize,
    pub frame_count: usize,
    pub label: String,
    /// Absolute path to this milestone's output file inside the working
    /// directory.
    pub output_path: String,
}

/// Everything recorded with each progression -- the exact Siril version, the
/// exact script, the settings, the acquisition family, every milestone, and the
/// final stack's display scale -- serialized verbatim to `progression.json`
/// next to the milestone images.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressionRecord {
    pub siril_version: String,
    pub script: String,
    pub script_path: String,
    pub settings: ProgressionSettings,
    pub acquisition_family: AcquisitionFamily,
    pub milestones: Vec<MilestoneOutcome>,
    pub final_stack_display_scale: DisplayScale,
    pub generated_at: String,
}

/// The result of attempting a progression: available with the full record,
/// or unavailable with a human-readable reason -- never an `Err`, per the
/// failure policy this module implements. The script text and its on-disk
/// path are present either way: [`super::script::generate_script`] runs
/// (and its output is written to disk) before Siril is ever invoked, so
/// both are already known regardless of what Siril does afterwards.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum ProgressionOutcome {
    /// Boxed: `ProgressionRecord` carries a whole `AcquisitionFamily` plus
    /// every milestone, far larger than the `Unavailable` variant, and
    /// `clippy::large_enum_variant` is right that leaving it unboxed would
    /// force every `ProgressionOutcome` value (including the common
    /// `Unavailable` case) to reserve that much space.
    Available(Box<ProgressionRecord>),
    // `rename_all` on the enum itself only renames variant *tags*
    // ("Available" -> "available"); a struct variant's own fields need
    // their own `rename_all` to be camelCase too (confirmed the hard way:
    // without this, `scriptPath` serialized as `script_path` here while
    // `Available`'s fields -- flattened from `ProgressionRecord`, which
    // carries its own `rename_all` -- were correctly camelCase).
    #[serde(rename_all = "camelCase")]
    Unavailable {
        reason: String,
        script: String,
        script_path: String,
    },
}

const SCRIPT_FILE_NAME: &str = "progression.ssf";
const RECORD_FILE_NAME: &str = "progression.json";

/// Writes the script for `family` into `working_dir` (created if needed),
/// runs `siril_path -s <script>` against it, verifies every milestone's
/// output file exists, reads the final milestone's display scale, and
/// writes `progression.json` next to the images -- or returns
/// [`ProgressionOutcome::Unavailable`] at the first problem, with no panic
/// and no `Err` a caller would have to specially unwrap.
///
/// `source_root` is never written to by this function: every filesystem
/// write below targets `working_dir` alone, and the one read outside it
/// (`fits::parse_primary_header` on the final milestone's own output) also
/// stays inside `working_dir`. `working_dir` is refused outright if it
/// would resolve inside `source_root`, mirroring
/// `package::produce_package`'s own "never inside the source" refusal.
#[must_use]
pub fn run_progression(
    source_root: &Path,
    working_dir: &Path,
    family: &AcquisitionFamily,
    siril_path: &Path,
    siril_version: &str,
    settings: &ProgressionSettings,
) -> ProgressionOutcome {
    if family.frame_paths.is_empty() {
        // Nothing to build a progression from. A caller should not
        // normally reach this (a family only exists because at least one
        // frame was grouped into it), but refusing cleanly here is simpler
        // than special-casing an empty family inside `generate_script`.
        return unavailable_without_script("the acquisition family holds no frame".to_string());
    }
    if is_within(source_root, working_dir).unwrap_or(true) {
        return unavailable_without_script(
            "the working directory must never sit inside the source folder".to_string(),
        );
    }

    let generated = generate_script(source_root, working_dir, family, siril_version, settings);

    if let Err(error) = fs::create_dir_all(working_dir) {
        return unavailable_without_script(format!(
            "could not create the working directory {}: {error}",
            working_dir.display()
        ));
    }
    let script_path = working_dir.join(SCRIPT_FILE_NAME);
    if let Err(error) = fs::write(&script_path, &generated.text) {
        return unavailable_without_script(format!(
            "could not write the script {}: {error}",
            script_path.display()
        ));
    }
    let script_path_string = script_path.display().to_string();

    let unavailable = |reason: String| ProgressionOutcome::Unavailable {
        reason,
        script: generated.text.clone(),
        script_path: script_path_string.clone(),
    };

    let mut command = Command::new(siril_path);
    crate::external_process::hide_console(&mut command);
    // Siril's own configuration, never the person's.
    let config = match crate::external_process::siril_config_in(working_dir) {
        Ok(config) => config,
        Err(error) => {
            return unavailable(format!(
                "could not write the Siril configuration in {}: {error}",
                working_dir.display()
            ));
        }
    };
    let output = match command
        .arg("-i")
        .arg(config)
        .arg("-s")
        .arg(&script_path)
        .current_dir(working_dir)
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            return unavailable(format!(
                "Siril could not be run ({}): {error}",
                siril_path.display()
            ));
        }
    };
    if !output.status.success() {
        // Verified against a real Siril 1.4.4 install: Siril's own
        // diagnostic messages ("There are not enough stars in reference
        // image to perform alignment", "Script execution failed.", ...)
        // are printed as `log:` lines on *stdout*, not stderr -- stderr
        // only ever carried an unrelated local CFITSIO version-mismatch
        // warning in that run. Reading stderr alone would have produced a
        // reason with none of the actually useful information, so both
        // streams are combined here, keeping only the tail (Siril's own
        // stdout can run to hundreds of lines for a multi-image run) where
        // the actual failure line lives.
        let combined = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return unavailable(format!(
            "Siril ended with an error (code {:?}): {}",
            output.status.code(),
            tail_lines(&combined, 12)
        ));
    }

    let mut milestone_outcomes = Vec::with_capacity(generated.milestones.len());
    for milestone in &generated.milestones {
        let output_path = working_dir.join(milestone_file_name(milestone));
        if !output_path.is_file() {
            return unavailable(format!(
                "milestone {} produced none of the expected output files ({})",
                milestone.label,
                output_path.display()
            ));
        }
        milestone_outcomes.push(MilestoneOutcome {
            index: milestone.index,
            frame_count: milestone.frame_count,
            label: milestone.label.clone(),
            output_path: output_path.display().to_string(),
        });
    }

    let Some(final_milestone_path) = milestone_outcomes
        .last()
        .map(|milestone| PathBuf::from(&milestone.output_path))
    else {
        return unavailable("no milestone was generated".to_string());
    };
    let final_stack_display_scale = match read_display_scale(&final_milestone_path) {
        Ok(scale) => scale,
        Err(reason) => return unavailable(reason),
    };

    let record = ProgressionRecord {
        siril_version: siril_version.to_string(),
        script: generated.text,
        script_path: script_path_string,
        settings: settings.clone(),
        acquisition_family: family.clone(),
        milestones: milestone_outcomes,
        final_stack_display_scale,
        generated_at: now_iso8601_utc(),
    };

    let record_path = working_dir.join(RECORD_FILE_NAME);
    let record_json =
        serde_json::to_string_pretty(&record).expect("ProgressionRecord always serializes") + "\n";
    if let Err(error) = fs::write(&record_path, record_json) {
        return ProgressionOutcome::Unavailable {
            reason: format!("could not write {}: {error}", record_path.display()),
            script: record.script,
            script_path: record.script_path,
        };
    }

    ProgressionOutcome::Available(Box::new(record))
}

/// The last `count` non-empty lines of `text`, trimmed and rejoined with
/// `" | "` -- keeps a Siril failure reason readable when the process
/// printed a long, verbose log rather than a single line.
fn tail_lines(text: &str, count: usize) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let start = lines.len().saturating_sub(count);
    lines[start..].join(" | ")
}

fn unavailable_without_script(reason: String) -> ProgressionOutcome {
    ProgressionOutcome::Unavailable {
        reason,
        script: String::new(),
        script_path: String::new(),
    }
}

fn read_display_scale(path: &Path) -> Result<DisplayScale, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let header = fits::parse_primary_header(&bytes)
        .map_err(|error| format!("invalid FITS header in {}: {error}", path.display()))?;
    let width = header
        .get_i64("NAXIS1")
        .ok_or_else(|| format!("NAXIS1 missing from {}", path.display()))?;
    let height = header
        .get_i64("NAXIS2")
        .ok_or_else(|| format!("NAXIS2 missing from {}", path.display()))?;
    Ok(DisplayScale {
        width_pixels: width,
        height_pixels: height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn tail_lines_keeps_only_the_last_non_empty_lines() {
        let text = "a\n\nb\nc\nd\n\n";
        assert_eq!(tail_lines(text, 2), "c | d");
        assert_eq!(tail_lines(text, 10), "a | b | c | d");
        assert_eq!(tail_lines("", 5), "");
    }

    /// Regression test for a real bug this crate shipped briefly: `#[serde(
    /// rename_all = "camelCase")]` on the *enum* only renames variant tags
    /// ("Unavailable" -> "unavailable"), not a struct variant's own field
    /// names -- confirmed by actually inspecting the CLI's `--json` output
    /// against a real `progression` run, where `scriptPath` came out as
    /// `script_path` until the `Unavailable` variant got its own
    /// `#[serde(rename_all = "camelCase")]`. This checks the exact rendered
    /// JSON keys directly so a future refactor that drops that attribute
    /// again fails a test instead of only being caught by chance.
    #[test]
    fn unavailable_serializes_every_field_as_camel_case() {
        let outcome = ProgressionOutcome::Unavailable {
            reason: "test".to_string(),
            script: "requires 1.2.1\n".to_string(),
            script_path: "/tmp/progression.ssf".to_string(),
        };
        let value = serde_json::to_value(&outcome).unwrap();
        let object = value.as_object().unwrap();
        for expected_key in ["status", "reason", "script", "scriptPath"] {
            assert!(
                object.contains_key(expected_key),
                "missing key {expected_key:?} in {object:?}"
            );
        }
        assert!(
            !object.contains_key("script_path"),
            "scriptPath must not also appear as snake_case: {object:?}"
        );
        assert_eq!(value["status"], "unavailable");
    }

    fn sample_family(frame_count: usize) -> AcquisitionFamily {
        AcquisitionFamily {
            key: "Ha_300s_g100_1x1_t-10C".to_string(),
            filter: Some("Ha".to_string()),
            exposure_seconds: Some(300.0),
            gain: Some(100.0),
            binning: Some("1x1".to_string()),
            temperature_class_c: Some(-10),
            camera: None,
            frame_paths: (1..=frame_count)
                .map(|i| format!("frame-{i:04}.fits"))
                .collect(),
        }
    }

    /// A fake Siril binary:
    /// answers `--version` with `banner`, and for `-s <script>` reads back
    /// every `# ASTRO-SYNC-MILESTONE-OUTPUT: <name>` marker
    /// (`script::MILESTONE_OUTPUT_MARKER`) the script embeds and writes a
    /// minimal, valid, `width`x`height` FITS header (padded to a whole
    /// 2880-byte block, exactly like the real format requires) at that
    /// name inside its own current directory -- which `run_progression`
    /// sets to `working_dir`, so this never touches the source folder.
    #[cfg(unix)]
    fn write_fake_siril_success(path: &Path, banner: &str, width: i64, height: i64) {
        let script = [
            "#!/bin/sh".to_string(),
            "if [ \"$1\" = \"--version\" ]; then".to_string(),
            format!("  echo \"{banner}\""),
            "  exit 0".to_string(),
            "fi".to_string(),
            "script=''; config=''".to_string(),
            "while [ $# -gt 0 ]; do case \"$1\" in -s) script=\"$2\"; shift;; -i) config=\"$2\"; shift;; esac; shift; done".to_string(),
            // Siril is given a configuration of its own.
            "[ -f \"$config\" ] || { echo 'no -i configuration' >&2; exit 3; }".to_string(),
            "if [ -n \"$script\" ]; then".to_string(),
            "  grep '^# ASTRO-SYNC-MILESTONE-OUTPUT: ' \"$script\" \\".to_string(),
            "    | sed 's/^# ASTRO-SYNC-MILESTONE-OUTPUT: //' \\".to_string(),
            "    | while IFS= read -r name; do".to_string(),
            "    printf '%-80s' \"SIMPLE  = T\" > \"$name\"".to_string(),
            "    printf '%-80s' \"NAXIS   = 2\" >> \"$name\"".to_string(),
            format!("    printf '%-80s' \"NAXIS1  = {width}\" >> \"$name\""),
            format!("    printf '%-80s' \"NAXIS2  = {height}\" >> \"$name\""),
            "    printf '%-80s' \"END\" >> \"$name\"".to_string(),
            "    size=$(wc -c < \"$name\" | tr -d ' ')".to_string(),
            "    pad=$(( (2880 - size % 2880) % 2880 ))".to_string(),
            "    if [ \"$pad\" -gt 0 ]; then".to_string(),
            "      head -c \"$pad\" /dev/zero | tr '\\0' ' ' >> \"$name\"".to_string(),
            "    fi".to_string(),
            "  done".to_string(),
            "  exit 0".to_string(),
            "fi".to_string(),
            "exit 1".to_string(),
        ]
        .join("\n")
            + "\n";
        fs::write(path, script).unwrap();
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn a_successful_run_produces_every_milestone_and_the_final_display_scale() {
        let source = TempDir::new("progression-success-source");
        let working = TempDir::new("progression-success-working");
        // `working` must not be *inside* `source`; two independent temp
        // dirs already guarantee that.
        let fake_siril = source.path().join("fake-siril.sh");
        write_fake_siril_success(&fake_siril, "Siril 1.2.1", 1024, 768);

        let family = sample_family(3);
        let outcome = run_progression(
            source.path(),
            working.path(),
            &family,
            &fake_siril,
            "1.2.1",
            &ProgressionSettings::default(),
        );

        let ProgressionOutcome::Available(record) = outcome else {
            panic!("expected an available progression, got {outcome:?}");
        };
        assert_eq!(record.siril_version, "1.2.1");
        assert_eq!(record.acquisition_family, family);
        assert!(record.script.contains("requires 1.2.1"));
        assert!(Path::new(&record.script_path).is_file());
        assert_eq!(
            record.settings,
            ProgressionSettings::default(),
            "the exact settings used must be recorded"
        );

        // Milestone 1 is a single exposure, with no stacking.
        assert_eq!(record.milestones[0].frame_count, 1);
        // The family has 3 frames; the milestone plan always ends at the
        // family's total.
        assert_eq!(record.milestones.last().unwrap().frame_count, 3);

        // The final stack's own dimensions, read back from its FITS
        // header, are what the fake binary wrote for every output file.
        assert_eq!(
            record.final_stack_display_scale,
            DisplayScale {
                width_pixels: 1024,
                height_pixels: 768,
            }
        );

        assert!(
            working.path().join("progression.json").is_file(),
            "the record must be written to disk next to the images"
        );
    }

    #[test]
    fn a_nonexistent_siril_path_is_unavailable_but_never_an_error() {
        let source = TempDir::new("progression-missing-binary-source");
        let working = TempDir::new("progression-missing-binary-working");
        let missing_binary = source.path().join("does-not-exist");

        let family = sample_family(2);
        let outcome = run_progression(
            source.path(),
            working.path(),
            &family,
            &missing_binary,
            "1.2.1",
            &ProgressionSettings::default(),
        );

        let ProgressionOutcome::Unavailable {
            reason,
            script,
            script_path,
        } = outcome
        else {
            panic!("expected an unavailable progression");
        };
        assert!(!reason.is_empty());
        // The script is still generated and preserved on disk, viewable
        // regardless of whether Siril itself could run (point 2).
        assert!(!script.is_empty());
        assert!(Path::new(&script_path).is_file());
    }

    /// Point 4's own verification bullet: a progression failure must
    /// never block session synchronization. This does not call any
    /// synchronization code directly (that lives in `package`/`uploader`,
    /// untouched by this module) -- it demonstrates the actual guarantee
    /// those callers rely on: `run_progression` never panics and never
    /// returns an `Err`, so a caller's own package-production/upload code
    /// path that runs before or after it is completely unaffected.
    #[test]
    fn a_siril_failure_never_panics_and_returns_a_plain_value() {
        let source = TempDir::new("progression-never-blocks-source");
        let working = TempDir::new("progression-never-blocks-working");
        let missing_binary = PathBuf::from("/definitely/not/a/real/path/siril");

        let outcome = std::panic::catch_unwind(|| {
            run_progression(
                source.path(),
                working.path(),
                &sample_family(1),
                &missing_binary,
                "1.2.1",
                &ProgressionSettings::default(),
            )
        });
        assert!(
            outcome.is_ok(),
            "run_progression must never panic, so a caller can always continue"
        );
        assert!(matches!(
            outcome.unwrap(),
            ProgressionOutcome::Unavailable { .. }
        ));
    }

    #[test]
    fn refuses_a_working_directory_inside_the_source_folder() {
        let source = TempDir::new("progression-nested-source");
        let nested_working = source.path().join("siril-work");
        fs::create_dir(&nested_working).unwrap();

        let outcome = run_progression(
            source.path(),
            &nested_working,
            &sample_family(1),
            Path::new("/does/not/matter"),
            "1.2.1",
            &ProgressionSettings::default(),
        );
        assert!(matches!(outcome, ProgressionOutcome::Unavailable { .. }));
    }

    /// The read-only invariant (point 2: "the source folders are only
    /// inputs".3's own read-only test, reused here). The fake binary in
    /// this test never reads its arguments at all (`exit 1`
    /// unconditionally), so this specifically pins down that
    /// `run_progression`'s own Rust code -- independently of whatever a
    /// real Siril process would or would not do -- never opens a source
    /// file for writing, and never copies, moves, renames, or deletes one.
    #[test]
    #[cfg(unix)]
    fn progression_never_modifies_a_source_file() {
        let source = TempDir::new("progression-read-only-source");
        let working = TempDir::new("progression-read-only-working");
        fs::write(source.path().join("frame-0001.fits"), vec![7_u8; 128]).unwrap();
        let unresponsive_binary = source.path().join("unresponsive.sh");
        fs::write(&unresponsive_binary, "#!/bin/sh\nexit 1\n").unwrap();
        let mut perms = fs::metadata(&unresponsive_binary).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&unresponsive_binary, perms).unwrap();

        let before = fs::metadata(source.path().join("frame-0001.fits")).unwrap();
        let before_bytes = fs::read(source.path().join("frame-0001.fits")).unwrap();

        let _ = run_progression(
            source.path(),
            working.path(),
            &sample_family(1),
            &unresponsive_binary,
            "1.2.1",
            &ProgressionSettings::default(),
        );

        let after = fs::metadata(source.path().join("frame-0001.fits")).unwrap();
        let after_bytes = fs::read(source.path().join("frame-0001.fits")).unwrap();
        assert_eq!(before.len(), after.len());
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());
        assert_eq!(before_bytes, after_bytes);
    }
}

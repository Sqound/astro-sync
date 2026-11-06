//! Milestone planning and Siril script rendering (points 2-3). Everything
//! here is pure and filesystem-free -- no file is written, no process is
//! spawned -- precisely so the exact, deterministic script text can be
//! golden-tested on its own; [`super::progression::run_progression`] is
//! what writes the returned text into the isolated working directory and
//! actually runs Siril against it.
//!
//! The generated script's exact Siril command set (`convert`, `calibrate`,
//! `register`, `select`/`unselect`, `stack`) and their default naming
//! conventions (`calibrate`'s `pp_` prefix, `register`'s `r_` prefix,
//! `convert`'s `<basename>_00001`-style frame numbering) follow Siril's own
//! published command reference (siril.readthedocs.io, "Commands"). This has
//! not been exercised against a real Siril install -- only against the
//! fake-binary fixtures -- so the exact flag spelling should be re-verified
//! end to end on a real installation before this is relied on for
//! production stacking.

use std::path::Path;

use crate::analysis::fits::AcquisitionFamily;

/// One stacking milestone: milestone 1 is always a single exposure with no
/// stacking (point 3); every later milestone stacks a larger, cumulative
/// prefix of the same acquisition family's registered frames.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Milestone {
    /// 1-based milestone number.
    pub index: usize,
    /// How many of the family's frames this milestone's output covers --
    /// always `1` for `index == 1`.
    pub frame_count: usize,
    /// The script's own output name for this milestone (`"milestone_01"`),
    /// reused as the `stack`/`save` target and later as the asset name
    /// uploaded under `progressions/` (point 5).
    pub label: String,
}

/// Deterministically builds the milestone plan for a family of
/// `total_frames` frames: `1` frame (milestone 1, no stacking), then a
/// strictly increasing sequence of cumulative frame counts at roughly
/// 25 %/50 %/75 %/100 % of the family, always ending exactly at
/// `total_frames` and never repeating a count already reached. The plan
/// depends only on `total_frames`, so the same family always produces the
/// same milestones -- what makes [`generate_script`]'s output
/// deterministic. `total_frames == 0` (no frames at all: nothing to build a
/// progression from) returns no milestones.
#[must_use]
pub fn build_milestones(total_frames: usize) -> Vec<Milestone> {
    if total_frames == 0 {
        return Vec::new();
    }
    let mut counts = vec![1_usize];
    if total_frames > 1 {
        for fraction in [0.25, 0.5, 0.75, 1.0] {
            let count = ((total_frames as f64) * fraction).round() as usize;
            let count = count.clamp(1, total_frames);
            if count > *counts.last().expect("counts is never empty") {
                counts.push(count);
            }
        }
        if *counts.last().expect("counts is never empty") != total_frames {
            counts.push(total_frames);
        }
    }
    counts
        .into_iter()
        .enumerate()
        .map(|(position, frame_count)| Milestone {
            index: position + 1,
            frame_count,
            label: format!("milestone_{:02}", position + 1),
        })
        .collect()
}

/// The registration/calibration/stacking settings recorded with every
/// progression (point 3: "settings"). Fixed for v1 -- there is no per-run
/// tuning screen yet -- but kept as a named, serializable struct rather
/// than being baked into the script text unnamed, so a future settings UI
/// has a well-defined place to plug into without changing
/// [`super::ProgressionRecord`]'s shape.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressionSettings {
    /// Siril's `register` command variant used for the one-time
    /// registration pass; `"default"` means no extra flag is appended.
    pub registration: String,
    /// Siril's `stack` rejection clause, e.g. `"rej 3 3"`.
    pub rejection: String,
    /// Siril's `stack -norm=` value.
    pub normalization: String,
}

impl Default for ProgressionSettings {
    fn default() -> Self {
        Self {
            registration: "default".to_string(),
            rejection: "rej 3 3".to_string(),
            normalization: "addscale".to_string(),
        }
    }
}

/// Siril's own naming convention for the sequences this script builds
/// (`convert`'s basename, then `calibrate`'s default `pp_` prefix, then
/// `register`'s default `r_` prefix) -- the script is written *for* Siril,
/// so it follows Siril's own defaults rather than inventing new names.
///
/// The trailing underscore on every constant below except
/// [`CONVERT_BASENAME`] is not decorative: verified against a real Siril
/// 1.4.4 install, `convert
/// <basename>` writes its sequence file as `<basename>_.seq` (not
/// `<basename>.seq`), and while `calibrate` tolerates being given the bare
/// basename, `register` does not -- it refused with "file cannot be
/// opened: pp_progression.seq" until the exact, underscore-suffixed name
/// (`"pp_progression_"`) was passed instead. Every later reference to a
/// sequence (`calibrate`, `register`, `select`/`unselect`, `stack`) uses
/// the underscore-suffixed form here for that reason; only [`CONVERT_BASENAME`]
/// itself (the argument `convert` is actually called with) stays bare.
const CONVERT_BASENAME: &str = "progression";
const CONVERTED_SEQUENCE: &str = "progression_";
const CALIBRATED_SEQUENCE: &str = "pp_progression_";
const REGISTERED_SEQUENCE: &str = "r_pp_progression_";

/// Every generated script embeds one `# ASTRO-SYNC-MILESTONE-OUTPUT: <file>`
/// comment per milestone, naming the exact file that milestone's commands
/// produce. Siril ignores `#` comments, so this changes nothing about how
/// Siril reads the script; it exists so both this crate's own
/// post-run verification (`super::progression::run_progression`) and a test
/// fixture's fake binary can discover
/// "which output files does this script promise" from the script text
/// itself, rather than duplicating the milestone-to-filename mapping in a
/// second place that could drift out of sync with it.
pub const MILESTONE_OUTPUT_MARKER: &str = "# ASTRO-SYNC-MILESTONE-OUTPUT: ";

/// The file extension every milestone output is saved under.
pub const MILESTONE_FILE_EXTENSION: &str = "fit";

/// The exact output file name [`generate_script`] promises for `milestone`
/// (and the exact one [`MILESTONE_OUTPUT_MARKER`] embeds next to it).
#[must_use]
pub fn milestone_file_name(milestone: &Milestone) -> String {
    format!("{}.{MILESTONE_FILE_EXTENSION}", milestone.label)
}

/// Everything [`generate_script`] computed: the exact script text (also the
/// value [`super::ProgressionRecord`] records verbatim) and the milestone
/// plan it encodes.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedScript {
    pub text: String,
    pub milestones: Vec<Milestone>,
}

/// Renders the exact Siril script for one progression, from `source_root`
/// (read-only input) into `working_dir` (isolated, disposable): converts
/// every frame of `family` once, calibrates and registers the resulting
/// sequence once, then emits a milestone-1 `load`/`save` (a single
/// exposure, no stacking) and one `stack` per later milestone over a
/// growing, explicitly `select`ed prefix of the registered sequence --
/// "registration and calibration performed once, reused for cumulative
/// milestones" (point 3). `family.frame_paths` is never read from disk here
/// (this function is pure); its length alone drives the milestone plan, and
/// its `key` is only ever echoed into comments.
#[must_use]
pub fn generate_script(
    source_root: &Path,
    working_dir: &Path,
    family: &AcquisitionFamily,
    siril_version: &str,
    settings: &ProgressionSettings,
) -> GeneratedScript {
    let milestones = build_milestones(family.frame_paths.len());
    let total = family.frame_paths.len();
    let mut lines: Vec<String> = Vec::new();

    lines.push("# Astro Sync -- Siril script generated automatically.".to_string());
    lines.push("# Do not edit by hand: regenerated for each new progression.".to_string());
    lines.push(format!("# Acquisition family : {}", family.key));
    lines.push(format!("# Number of frames     : {total}"));
    lines.push(String::new());
    lines.push(format!("requires {siril_version}"));
    lines.push(String::new());
    lines.push("# The source folder is only ever an input: Siril reads the original".to_string());
    lines.push(
        "# frames there, then converts into the isolated working directory below;".to_string(),
    );
    lines.push("# nothing is ever written to the source folder.".to_string());
    lines.push(format!("cd \"{}\"", source_root.display()));
    // Unlike `cd`'s own argument (confirmed against a real Siril 1.4.4
    // install to accept a quoted, space-safe directory), `convert`'s
    // `-out=` value is *not* quote-aware: a literal `-out="..."` leaves the
    // quote characters in the path and silently writes a stray `"` entry
    // instead of using `working_dir`. Left unquoted here as a result, which
    // is only verified safe for a path with no spaces -- a working
    // directory whose path contains a space is a known, unverified edge
    // case for this one line.
    lines.push(format!(
        "convert {CONVERT_BASENAME} -out={}",
        working_dir.display()
    ));
    lines.push(String::new());
    lines.push(format!("cd \"{}\"", working_dir.display()));
    lines.push(format!("calibrate {CONVERTED_SEQUENCE}"));
    let registration_suffix = if settings.registration == "default" {
        String::new()
    } else {
        format!(" {}", settings.registration)
    };
    lines.push(format!(
        "register {CALIBRATED_SEQUENCE}{registration_suffix}"
    ));
    // The registered frames are already aligned: their registration data would
    // only make Siril 1.4.4 stop in OpenCV (`gemm`) at the first stack.
    lines.push(format!("seqclean {REGISTERED_SEQUENCE} -reg"));

    let milestone_count = milestones.len();
    for milestone in &milestones {
        lines.push(String::new());
        lines.push(format!(
            "# Milestone {}/{milestone_count}: {} frame(s) of the family {}.",
            milestone.index, milestone.frame_count, family.key
        ));
        if milestone.index == 1 {
            lines.push(format!("load {REGISTERED_SEQUENCE}00001"));
            lines.push(format!("save {}", milestone.label));
        } else {
            // `select` and `unselect` count from 1, inclusive (a `0` is
            // « invalid arguments » on a real Siril 1.4.4).
            lines.push(format!("unselect {REGISTERED_SEQUENCE} 1 {total}"));
            lines.push(format!(
                "select {REGISTERED_SEQUENCE} 1 {}",
                milestone.frame_count
            ));
            lines.push(format!(
                "stack {REGISTERED_SEQUENCE} {} -filter-included -norm={} -out={}",
                settings.rejection, settings.normalization, milestone.label
            ));
        }
        lines.push(format!(
            "{MILESTONE_OUTPUT_MARKER}{}",
            milestone_file_name(milestone)
        ));
    }
    lines.push(String::new());

    GeneratedScript {
        text: lines.join("\n") + "\n",
        milestones,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn build_milestones_is_empty_for_zero_frames() {
        assert_eq!(build_milestones(0), Vec::new());
    }

    #[test]
    fn build_milestones_is_a_single_unstacked_milestone_for_one_frame() {
        let milestones = build_milestones(1);
        assert_eq!(milestones.len(), 1);
        assert_eq!(milestones[0].index, 1);
        assert_eq!(milestones[0].frame_count, 1);
        assert_eq!(milestones[0].label, "milestone_01");
    }

    #[test]
    fn build_milestones_always_starts_at_one_and_ends_at_the_total() {
        for total in [2, 5, 7, 20, 137] {
            let milestones = build_milestones(total);
            assert_eq!(milestones[0].frame_count, 1, "total={total}");
            assert_eq!(
                milestones.last().unwrap().frame_count,
                total,
                "total={total}"
            );
            // Strictly increasing, contiguous 1-based indices.
            for (position, milestone) in milestones.iter().enumerate() {
                assert_eq!(milestone.index, position + 1, "total={total}");
                assert_eq!(milestone.label, format!("milestone_{:02}", position + 1));
                if position > 0 {
                    assert!(
                        milestone.frame_count > milestones[position - 1].frame_count,
                        "total={total}"
                    );
                }
            }
        }
    }

    #[test]
    fn milestone_file_name_appends_the_fit_extension() {
        let milestone = Milestone {
            index: 1,
            frame_count: 1,
            label: "milestone_01".to_string(),
        };
        assert_eq!(milestone_file_name(&milestone), "milestone_01.fit");
    }

    /// Golden fixture (point 7: "deterministic generated script
    /// (golden)"): a fixed, hand-checked expected script for a 2-frame
    /// family. Any change to `generate_script`'s output shape must update
    /// this test deliberately rather than silently.
    #[test]
    fn generate_script_matches_the_golden_fixture_for_a_two_frame_family() {
        let family = sample_family(2);
        let settings = ProgressionSettings::default();

        let generated = generate_script(
            Path::new("/tmp/src"),
            Path::new("/tmp/work"),
            &family,
            "1.2.1",
            &settings,
        );

        let expected_lines: Vec<&str> = vec![
            "# Astro Sync -- Siril script generated automatically.",
            "# Do not edit by hand: regenerated for each new progression.",
            "# Acquisition family : Ha_300s_g100_1x1_t-10C",
            "# Number of frames     : 2",
            "",
            "requires 1.2.1",
            "",
            "# The source folder is only ever an input: Siril reads the original",
            "# frames there, then converts into the isolated working directory below;",
            "# nothing is ever written to the source folder.",
            "cd \"/tmp/src\"",
            "convert progression -out=/tmp/work",
            "",
            "cd \"/tmp/work\"",
            "calibrate progression_",
            "register pp_progression_",
            "seqclean r_pp_progression_ -reg",
            "",
            "# Milestone 1/2: 1 frame(s) of the family Ha_300s_g100_1x1_t-10C.",
            "load r_pp_progression_00001",
            "save milestone_01",
            "# ASTRO-SYNC-MILESTONE-OUTPUT: milestone_01.fit",
            "",
            "# Milestone 2/2: 2 frame(s) of the family Ha_300s_g100_1x1_t-10C.",
            "unselect r_pp_progression_ 1 2",
            "select r_pp_progression_ 1 2",
            "stack r_pp_progression_ rej 3 3 -filter-included -norm=addscale -out=milestone_02",
            "# ASTRO-SYNC-MILESTONE-OUTPUT: milestone_02.fit",
            "",
        ];
        let expected = expected_lines.join("\n") + "\n";

        assert_eq!(generated.text, expected);
        assert_eq!(generated.milestones.len(), 2);
        assert_eq!(generated.milestones[0].frame_count, 1);
        assert_eq!(generated.milestones[1].frame_count, 2);
    }

    #[test]
    fn generate_script_is_deterministic_for_identical_inputs() {
        let family = sample_family(5);
        let settings = ProgressionSettings::default();
        let first = generate_script(
            Path::new("/tmp/src"),
            Path::new("/tmp/work"),
            &family,
            "1.4.4",
            &settings,
        );
        let second = generate_script(
            Path::new("/tmp/src"),
            Path::new("/tmp/work"),
            &family,
            "1.4.4",
            &settings,
        );
        assert_eq!(first.text, second.text);
        assert_eq!(first.milestones, second.milestones);
    }

    #[test]
    fn generate_script_never_writes_a_command_targeting_the_source_directory() {
        // Structural check standing in for "the script never instructs
        // Siril to write into the source folder": after the initial
        // `convert ... -out="<working_dir>"` step, `cd` only ever switches
        // into `working_dir`, never back into `source_root`.
        let family = sample_family(3);
        let generated = generate_script(
            Path::new("/tmp/src"),
            Path::new("/tmp/work"),
            &family,
            "1.2.1",
            &ProgressionSettings::default(),
        );
        let cd_lines: Vec<&str> = generated
            .text
            .lines()
            .filter(|line| line.starts_with("cd "))
            .collect();
        assert_eq!(cd_lines, vec!["cd \"/tmp/src\"", "cd \"/tmp/work\""]);
    }
}

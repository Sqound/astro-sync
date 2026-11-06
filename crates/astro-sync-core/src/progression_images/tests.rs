use super::*;
use crate::test_support::{TempDir, fits_bytes};

fn frame(kind: &str, filter: &str, exposure: &str, minute: u32) -> Vec<u8> {
    fits_bytes(
        &[
            ("IMAGETYP", &format!("'{kind}'")),
            ("OBJECT", "'M31'"),
            ("FILTER", &format!("'{filter}'")),
            ("EXPTIME", exposure),
            ("GAIN", "100"),
            ("DATE-OBS", &format!("'2026-10-07T22:{minute:02}:00'")),
        ],
        16,
    )
}

/// No room is ever short in these tests, unless one says so.
fn plenty(_: &Path) -> Option<u64> {
    None
}

fn inputs(has_darks: bool, has_flats: bool, colour: bool) -> ScriptInputs<'static> {
    ScriptInputs {
        siril_version: "1.4.4",
        has_darks,
        has_flats,
        colour,
        xtrans: false,
    }
}

/// A night: M31 with four Ha lights, two OIII lights and a flat filed in
/// lights/; darks of 300 s and of 60 s; Ha flats in a sub-folder.
fn night(dir: &Path) -> PathBuf {
    let night = dir.join("2026-10-07 · Col des Étoiles · 7f3a2c");
    let write = |relative: &str, bytes: Vec<u8>| {
        let path = night.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    };
    for minute in [4, 1, 3, 2] {
        write(
            &format!("M31/lights/Ha_{minute}.fits"),
            frame("LIGHT", "Ha", "300", minute),
        );
    }
    for minute in [10, 11] {
        write(
            &format!("M31/lights/OIII_{minute}.fits"),
            frame("LIGHT", "OIII", "300", minute),
        );
    }
    write("M31/lights/Flat_1.fits", frame("FLAT", "Ha", "1", 30));
    write(
        "calibration/darks/Dark_300_1.fits",
        frame("DARK", "Ha", "300", 40),
    );
    write(
        "calibration/darks/Dark_300_2.fits",
        frame("DARK", "Ha", "300", 41),
    );
    write(
        "calibration/darks/Dark_60.fits",
        frame("DARK", "Ha", "60", 42),
    );
    let flat = fits_bytes(&[("IMAGETYP", "'FLAT'"), ("EXPTIME", "1")], 16);
    write("calibration/flats/Ha/Flat_1.fits", flat.clone());
    write("calibration/flats/Ha/Flat_2.fits", flat);
    write(
        "calibration/flats/OIII/Flat_1.fits",
        fits_bytes(&[("IMAGETYP", "'FLAT'")], 16),
    );
    night
}

fn every_file(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files: Vec<_> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| (e.path().to_path_buf(), fs::read(e.path()).unwrap()))
        .collect();
    files.sort();
    files
}

#[test]
fn a_targets_families_come_from_its_lights() {
    let dir = TempDir::new("progression-families");
    let night = night(dir.path());
    let families = target_families(&night, "M31");
    let keys: Vec<(&str, usize)> = families
        .iter()
        .map(|f| (f.filter.as_deref().unwrap_or(""), f.frame_paths.len()))
        .collect();
    assert_eq!(keys, vec![("Ha", 4), ("OIII", 2)]);
    // In the order they were taken.
    assert!(families[0].frame_paths[0].ends_with("Ha_1.fits"));
    assert!(families[0].frame_paths[3].ends_with("Ha_4.fits"));
}

fn four_frame_family() -> AcquisitionFamily {
    AcquisitionFamily {
        key: "Ha_300s_g100_unknown_tunknown".to_string(),
        filter: Some("Ha".to_string()),
        exposure_seconds: Some(300.0),
        gain: Some(100.0),
        binning: None,
        temperature_class_c: None,
        camera: None,
        frame_paths: (1..=4)
            .map(|i| format!("/night/M31/lights/{i}.fits"))
            .collect(),
    }
}

fn registered(indices: &[usize]) -> RegisteredSequence {
    RegisteredSequence {
        indices: indices.to_vec(),
    }
}

#[test]
fn the_script_has_fixed_settings_and_stays_in_its_scratch() {
    let family = four_frame_family();
    let work = Path::new("/night/.astro-sync-work/progression/M31/Ha");
    let (first, name) = registration_script(work, &family, inputs(true, true, false));
    assert_eq!(name, "r_pp_light_");
    let milestones = build_milestones(4);
    let second = milestones_script(
        work,
        &name,
        &registered(&[1, 2, 3, 4]),
        &milestones,
        "1.4.4",
        false,
    );
    let script = format!("{first}{second}");
    let lines: Vec<&str> = script.lines().collect();
    for expected in [
        "requires 1.4.4",
        "stack dark_ rej 3 3 -nonorm -out=master_dark",
        "stack flat_ rej 3 3 -norm=mul -out=master_flat",
        "convert light -out=../process",
        "calibrate light_ -dark=master_dark -flat=master_flat",
        "register pp_light_",
        "seqclean r_pp_light_ -reg",
        "load r_pp_light_00001",
        "stack r_pp_light_ rej 3 3 -filter-included -norm=addscale -out=milestone_02",
        "autostretch -linked -3.29 0.10",
        "savejpg milestone_04 90",
        "# ASTRO-SYNC-MILESTONE-OUTPUT: milestone_04.jpg",
    ] {
        assert!(lines.contains(&expected), "{expected:?} in\n{script}");
    }
    assert!(
        lines
            .iter()
            .filter(|l| l.starts_with("cd "))
            .all(|l| l.contains("/night/.astro-sync-work/progression/"))
    );
    assert!(
        lines
            .iter()
            .filter(|l| l.contains("-out="))
            .all(|l| !l.contains("-out=/"))
    );
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.starts_with("autostretch"))
            .count(),
        milestones.len()
    );
    // A mono sensor: nothing is debayered.
    assert!(!script.contains("debayer") && !script.contains("-cfa"));
    assert!(!script.contains("-rgb_equal"));

    let (bare, bare_name) = registration_script(work, &family, inputs(false, false, false));
    assert!(!bare.contains("calibrate"));
    assert!(bare.contains("register light_\n"));
    assert_eq!(bare_name, "r_light_");
    // The same input, the same script.
    assert_eq!(
        bare,
        registration_script(work, &family, inputs(false, false, false)).0
    );
}

/// A colour sensor's frames are debayered -- at the calibration when
/// there is one, else when converted -- and stacked with their channels
/// equalised; the display is the same linked stretch.
#[test]
fn a_colour_sensors_frames_are_debayered_once() {
    let family = four_frame_family();
    let work = Path::new("/night/.astro-sync-work/progression/IC_1805/f");
    let (calibrated, name) = registration_script(work, &family, inputs(true, true, true));
    let lines: Vec<&str> = calibrated.lines().collect();
    assert!(
        lines.contains(&"convert light -out=../process"),
        "{calibrated}"
    );
    assert!(
        lines.contains(
            &"calibrate light_ -dark=master_dark -flat=master_flat -cfa -equalize_cfa -debayer"
        ),
        "{calibrated}"
    );
    assert_eq!(calibrated.matches("-debayer").count(), 1);
    // The master flat stays a CFA image: only the lights are debayered.
    assert!(lines.contains(&"stack flat_ rej 3 3 -norm=mul -out=master_flat"));

    let (bare, _) = registration_script(work, &family, inputs(false, false, true));
    assert!(
        bare.lines()
            .any(|l| l == "convert light -debayer -out=../process"),
        "{bare}"
    );
    assert_eq!(bare.matches("-debayer").count(), 1);

    let milestones = milestones_script(
        work,
        &name,
        &registered(&[1, 2, 3, 4]),
        &build_milestones(4),
        "1.4.4",
        true,
    );
    assert!(
        milestones.lines().any(|l| l
            == "stack r_pp_light_ rej 3 3 -filter-included -norm=addscale -rgb_equal -out=milestone_04"),
        "{milestones}"
    );
    // The first milestone, a single colour frame no stack equalised, levels
    // its channels; the stacks keep the ratios `-rgb_equal` gave them.
    let stretches: Vec<&str> = milestones
        .lines()
        .filter(|l| l.starts_with("autostretch"))
        .collect();
    assert_eq!(
        stretches,
        [
            "autostretch -3.29 0.10",
            "autostretch -linked -3.29 0.10",
            "autostretch -linked -3.29 0.10",
            "autostretch -linked -3.29 0.10",
        ]
    );
}

#[test]
fn a_bayer_pattern_in_the_header_makes_a_colour_job() {
    let dir = TempDir::new("progression-colour");
    let night = dir.path().join("night");
    let lights = night.join("IC 1805/lights");
    fs::create_dir_all(&lights).unwrap();
    let colour = fits_bytes(
        &[
            ("IMAGETYP", "'Light Frame'"),
            ("BAYERPAT", "'RGGB'"),
            ("EXPTIME", "90"),
            ("GAIN", "100"),
            ("DATE-OBS", "'2026-08-11T02:02:03'"),
        ],
        16,
    );
    fs::write(lights.join("Light_0001.fit"), &colour).unwrap();
    let family = target_families(&night, "IC 1805").remove(0);
    let job = ProgressionJob::new(&night, "IC 1805", family);
    assert!(job.colour);
    assert_eq!(
        job.work_dir,
        night
            .join(".astro-sync-work/progression/IC_1805")
            .join(&job.family.key)
    );
    assert_eq!(job.output_dir, night.join("IC 1805/progression"));

    let mono = target_families(&night_with_mono(dir.path()), "M31").remove(0);
    assert!(!ProgressionJob::new(dir.path(), "M31", mono).colour);
}

fn night_with_mono(dir: &Path) -> PathBuf {
    night(dir)
}

/// WEB.17b.2 (TD2): Siril's `select` and `unselect` count from 1 (a `0`
/// is « invalid arguments » on a real Siril 1.4.4), and the sequence only
/// holds the frames that registered.
#[test]
fn the_selections_count_from_one_over_the_registered_frames_only() {
    let work = Path::new("/night/.astro-sync-work/progression/M31/Ha");
    // 112 frames taken, 108 registered (the real night's case).
    let milestones = build_milestones(108);
    let indices: Vec<usize> = (1..=112)
        .filter(|i| ![19, 20, 21, 77].contains(i))
        .collect();
    assert_eq!(indices.len(), 108);
    let script = milestones_script(
        work,
        "r_pp_light_",
        &registered(&indices),
        &milestones,
        "1.4.4",
        false,
    );
    let lines: Vec<&str> = script.lines().collect();
    assert!(lines.contains(&"unselect r_pp_light_ 1 108"), "{script}");
    assert!(lines.contains(&"select r_pp_light_ 1 108"), "{script}");
    for line in &lines {
        if line.starts_with("select ") || line.starts_with("unselect ") {
            let numbers: Vec<usize> = line
                .split_whitespace()
                .skip(2)
                .map(|n| n.parse().unwrap())
                .collect();
            assert!(numbers[0] >= 1 && numbers[1] <= 108, "{line}");
        }
    }
    assert!(!script.contains(" 0 "), "no 0-based range:\n{script}");
    // The registration data is cleared before the first stack.
    let clean = lines.iter().position(|l| *l == "seqclean r_pp_light_ -reg");
    let stack = lines.iter().position(|l| l.starts_with("stack "));
    assert!(clean.is_some() && clean < stack, "{script}");

    // The first frame failed to register: milestone 1 loads the first
    // frame that did.
    let script = milestones_script(
        work,
        "r_light_",
        &registered(&[2, 3, 5]),
        &build_milestones(3),
        "1.4.4",
        false,
    );
    assert!(script.contains("load r_light_00002\n"), "{script}");
    assert!(!script.contains("r_light_00001"), "{script}");
}

#[test]
fn the_registered_sequence_is_read_from_its_seq_file() {
    let dir = TempDir::new("progression-seq");
    fs::write(
        dir.path().join("r_light_.seq"),
        "#Siril sequence file.\nS 'r_light_' 1 3 3 5 -1 6 0 0 0\nL 1\nI 2 1\nI 3 1\nI 5 0\n",
    )
    .unwrap();
    let sequence = RegisteredSequence::read(dir.path(), "r_light_").unwrap();
    assert_eq!(sequence.indices, vec![2, 3, 5]);
    assert!(
        RegisteredSequence::read(dir.path(), "r_missing_")
            .unwrap_err()
            .contains("r_missing_")
    );
    fs::write(dir.path().join("r_empty_.seq"), "S 'r_empty_' 1 0 0\n").unwrap();
    assert!(RegisteredSequence::read(dir.path(), "r_empty_").is_err());
}

#[test]
fn the_gate_the_tool_and_the_room_come_first() {
    let dir = TempDir::new("progression-gate");
    let night = night(dir.path());
    let before = every_file(&night);
    let family = target_families(&night, "M31").remove(0);
    let job = ProgressionJob::new(&night, "M31", family);
    let siril = Some((Path::new("/nonexistent/siril"), "1.4.4"));
    let five = Duration::from_secs(5);
    assert_eq!(
        run_progression_images(&job, false, siril, &plenty, five, None).unwrap_err(),
        ProgressionFailure::NotInSubscription
    );
    assert_eq!(
        run_progression_images(&job, true, None, &plenty, five, None).unwrap_err(),
        ProgressionFailure::SirilMissing
    );
    // Four frames need about 4 × 210 MB on the disk of the night.
    let short = |folder: &Path| {
        assert_eq!(folder, night.as_path(), "measured on the night's disk");
        Some(1_000_000)
    };
    assert_eq!(
        run_progression_images(&job, true, siril, &short, five, None).unwrap_err(),
        ProgressionFailure::NotEnoughSpace {
            needed: 4 * BYTES_PER_LIGHT,
            available: 1_000_000,
            folder: night.display().to_string(),
        }
    );
    assert!(!job.work_dir.exists());
    assert_eq!(every_file(&night), before);
    assert!(!night.join(".astro-sync-work").exists());
}

/// A Siril that registers every frame the first script converts, except
/// frame `dropped` (0: none), writes the registered sequence's `.seq`, and
/// refuses what the real one refuses: a `select` or `unselect` range that
/// starts before 1 or ends after the registered frames.
#[cfg(unix)]
fn fake_siril_body(dropped: usize) -> String {
    format!(
        r#"script=''; config=''
while [ $# -gt 0 ]; do case "$1" in -s) script="$2"; shift;; -i) config="$2"; shift;; esac; shift; done
[ -f "$config" ] || {{ echo 'no -i configuration: the persons own would be rewritten' >&2; exit 3; }}
out=$(grep '^cd ' "$script" | tail -1 | sed 's/^cd "\(.*\)"$/\1/')
reg=$(grep '^register ' "$script" | sed 's/^register //')
if [ -n "$reg" ]; then
  n=$(grep '^# Frames: ' "$script" | sed 's/^# Frames: //')
  {{ echo "S 'r_$reg' 1 $n $n 5 -1 6 0 0 0"; i=1
    while [ $i -le $n ]; do [ "$i" != "{dropped}" ] && echo "I $i 1"; i=$((i+1)); done; }} > "$out/r_$reg.seq"
fi
grep -E '^(un)?select ' "$script" | while read cmd name from to; do
  count=$(grep -c '^I ' "$out/$name.seq")
  if [ "$from" -lt 1 ] || [ "$to" -gt "$count" ]; then echo "log: The second argument must be between 1 and the number of images."; exit 1; fi
done || exit 1
grep '^# ASTRO-SYNC-MILESTONE-OUTPUT: ' "$script" | sed 's/^# ASTRO-SYNC-MILESTONE-OUTPUT: //' | while read f; do printf 'jpeg %s' "$f" > "$out/$f"; done"#
    )
}

/// Every file under `root` but the progression's JPEGs.
fn without_milestones(files: Vec<(PathBuf, Vec<u8>)>, root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    files
        .into_iter()
        .filter(|(path, _)| !path.starts_with(root.join("M31/progression")))
        .collect()
}

/// The progression says its parts, its milestone and what Siril prints,
/// whatever the scratch.
#[cfg(unix)]
#[test]
fn the_progression_says_its_parts_its_milestone_and_what_siril_prints() {
    use crate::external_process::fake_tool;
    let dir = TempDir::new("progression-events");
    let night = night(dir.path());
    let body = format!(
        "printf 'log: Registering the frames...\\n\\033[A\\033[2KT\\rprogress: 62.50%%\\n'\nsleep 0.7\n{}",
        fake_siril_body(0)
    );
    let siril = fake_tool(dir.path(), "siril-cli", &body);
    let family = target_families(&night, "M31").remove(0);
    let job = ProgressionJob::new(&night, "M31", family);
    let mut events = Vec::new();
    run_progression_images_with(
        &job,
        true,
        Some((&siril, "1.4.4")),
        &plenty,
        Duration::from_secs(30),
        None,
        &mut |event| events.push(event.clone()),
    )
    .unwrap();
    let stages: Vec<ProgressionStage> = events
        .iter()
        .filter_map(|event| match event {
            ProgressionEvent::Stage { stage } => Some(*stage),
            _ => None,
        })
        .collect();
    assert_eq!(
        stages,
        vec![ProgressionStage::Registration, ProgressionStage::Milestones]
    );
    assert!(events.contains(&ProgressionEvent::Milestone { index: 1, total: 4 }));
    assert_eq!(
        events
            .iter()
            .filter_map(|event| match event {
                ProgressionEvent::Milestone { index, .. } => Some(*index),
                _ => None,
            })
            .next_back(),
        Some(4),
        "every milestone is made: the last one is the fourth"
    );
    assert!(events.iter().any(|event| matches!(
        event,
        ProgressionEvent::Progress { percent: Some(p), line: Some(line), .. }
            if (*p - 62.5).abs() < 1e-9 && line == "Registering the frames..."
    )));
}

/// The work is done on the night's disk and goes when the run ends; only
/// the milestones' JPEGs stay, in `<target>/progression/`.
#[cfg(unix)]
#[test]
fn siril_works_on_the_nights_disk_and_only_the_jpegs_stay() {
    use crate::external_process::fake_tool;
    let dir = TempDir::new("progression-run");
    let night = night(dir.path());
    let before = every_file(&night);
    let siril = fake_tool(dir.path(), "siril-cli", &fake_siril_body(0));
    let family = target_families(&night, "M31").remove(0);
    let job = ProgressionJob::new(&night, "M31", family);
    let measured = std::cell::RefCell::new(Vec::new());
    let free = |folder: &Path| {
        measured.borrow_mut().push(folder.to_path_buf());
        Some(u64::MAX)
    };
    let made = run_progression_images(
        &job,
        true,
        Some((&siril, "1.4.4")),
        &free,
        Duration::from_secs(30),
        None,
    )
    .unwrap();
    assert_eq!(*measured.borrow(), vec![night.clone()]);
    let counts: Vec<usize> = made.images.iter().map(|i| i.frame_count).collect();
    assert_eq!(counts, vec![1, 2, 3, 4]);
    assert!(made.calibrated_with_darks && made.calibrated_with_flats);
    assert!(!made.colour);
    // The script linked the frames and the calibration in the scratch.
    assert!(made.script.contains("/.astro-sync-work/progression/M31/"));
    let kept = night.join("M31/progression");
    assert!(
        made.images
            .iter()
            .all(|i| i.path.parent() == Some(kept.as_path()))
    );
    let names: Vec<String> = fs::read_dir(&kept)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    assert_eq!(
        names,
        [
            "milestone_01.jpg",
            "milestone_02.jpg",
            "milestone_03.jpg",
            "milestone_04.jpg",
            "progression.json"
        ]
    );
    // Only the JPEGs stay: no scratch, no hidden folder, nothing else changed.
    assert!(!night.join(".astro-sync-work").exists());
    assert_eq!(without_milestones(every_file(&night), &night), before);

    let candidates = progression_candidates(&made, "t-m31");
    assert_eq!(candidates.len(), 4);
    assert!(
        candidates
            .iter()
            .all(|c| c.kind == ResultKind::Progression && c.render.is_none())
    );
    assert_eq!(
        candidates.iter().map(|c| c.frame_count).collect::<Vec<_>>(),
        vec![Some(1), Some(2), Some(3), Some(4)]
    );
    assert_eq!(
        crate::upload_queue::result_kind_of(&candidates[0].path),
        Ok(ResultKind::Progression)
    );

    // Again: the new set replaces the old one; a file of the person's own
    // in `progression/` stays.
    fs::write(kept.join("notes.txt"), "mine").unwrap();
    fs::write(kept.join("milestone_09.jpg"), "an older run").unwrap();
    let again = run_progression_images(
        &job,
        true,
        Some((&siril, "1.4.4")),
        &plenty,
        Duration::from_secs(30),
        None,
    )
    .unwrap();
    assert_eq!(again.images.len(), 4);
    assert!(!kept.join("milestone_09.jpg").exists());
    assert_eq!(fs::read_to_string(kept.join("notes.txt")).unwrap(), "mine");
    fs::remove_file(kept.join("notes.txt")).unwrap();

    // A failure, then a run with no image: the scratch goes all the same.
    let failing = fake_tool(
        dir.path(),
        "siril-failing",
        "echo 'log: Not enough stars'\nexit 1",
    );
    match run_progression_images(
        &job,
        true,
        Some((&failing, "1.4.4")),
        &plenty,
        Duration::from_secs(30),
        None,
    )
    .unwrap_err()
    {
        ProgressionFailure::Failed { detail } => assert!(detail.contains("Not enough stars")),
        other => panic!("{other:?}"),
    }
    assert!(!night.join(".astro-sync-work").exists());
    let silent = fake_tool(dir.path(), "siril-silent", "exit 0");
    assert_eq!(
        run_progression_images(
            &job,
            true,
            Some((&silent, "1.4.4")),
            &plenty,
            Duration::from_secs(30),
            None
        )
        .unwrap_err(),
        ProgressionFailure::Failed {
            detail: "the registered sequence r_pp_light_ is missing (No such file or directory (os error 2))"
                .to_string()
        }
    );
    // Registered, but no image written: the JPEGs of the last good run stay.
    let body = fake_siril_body(0);
    let no_image = fake_tool(
        dir.path(),
        "siril-no-image",
        body.split("grep '^# ASTRO-SYNC-MILESTONE").next().unwrap(),
    );
    assert_eq!(
        run_progression_images(
            &job,
            true,
            Some((&no_image, "1.4.4")),
            &plenty,
            Duration::from_secs(30),
            None
        )
        .unwrap_err(),
        ProgressionFailure::MissingImage {
            file: "milestone_01.jpg".to_string()
        }
    );
    // The four JPEGs of the last good run and the record of their poses.
    assert_eq!(fs::read_dir(&kept).unwrap().count(), 5);
    assert!(!night.join(".astro-sync-work").exists());
    // The frames themselves were never touched.
    assert_eq!(without_milestones(every_file(&night), &night), before);
}

/// A stop (here: a Siril that never ends, and the stop switch thrown)
/// removes the scratch too.
#[cfg(unix)]
#[test]
fn a_stopped_progression_leaves_no_scratch() {
    use crate::external_process::fake_tool;
    let dir = TempDir::new("progression-stop");
    let night = night(dir.path());
    let sleeping = fake_tool(dir.path(), "siril-sleeps", "sleep 30");
    let family = target_families(&night, "M31").remove(0);
    let job = ProgressionJob::new(&night, "M31", family);
    let cancel = CancelFlag::default();
    let thrown = cancel.clone();
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        thrown.cancel();
    });
    assert_eq!(
        run_progression_images(
            &job,
            true,
            Some((&sleeping, "1.4.4")),
            &plenty,
            Duration::from_secs(30),
            Some(&cancel),
        )
        .unwrap_err(),
        ProgressionFailure::Cancelled
    );
    stopper.join().unwrap();
    assert!(!night.join(".astro-sync-work").exists());
    assert!(!night.join("M31/progression").exists());
}

/// WEB.17b.2 (TD2): a frame that fails to register (here the second of
/// four) leaves the sequence shorter; the milestones follow what registered
/// and every range stays inside it.
#[cfg(unix)]
#[test]
fn a_frame_that_fails_to_register_does_not_break_the_progression() {
    use crate::external_process::fake_tool;
    let dir = TempDir::new("progression-dropped");
    let night = night(dir.path());
    let siril = fake_tool(dir.path(), "siril-drops-two", &fake_siril_body(2));
    let family = target_families(&night, "M31").remove(0);
    let job = ProgressionJob::new(&night, "M31", family);
    let made = run_progression_images(
        &job,
        true,
        Some((&siril, "1.4.4")),
        &plenty,
        Duration::from_secs(30),
        None,
    )
    .unwrap();
    assert_eq!(made.frames_total, 4);
    assert_eq!(made.frames_registered, 3);
    let counts: Vec<usize> = made.images.iter().map(|i| i.frame_count).collect();
    assert_eq!(counts, vec![1, 2, 3]);
    assert!(
        made.script.contains("select r_pp_light_ 1 3\n"),
        "{}",
        made.script
    );
    assert!(!made.script.contains(" 0 "), "{}", made.script);
    // The old, 0-based script is what the fake (like Siril) refuses (the
    // scratch went with the run: a sequence of three is written again).
    let process = dir.path().join("process");
    fs::create_dir_all(&process).unwrap();
    fs::write(process.join("r_pp_light_.seq"), "I 1 1\nI 3 1\nI 4 1\n").unwrap();
    let old = dir.path().join("old.ssf");
    fs::write(
        &old,
        format!("cd \"{}\"\nunselect r_pp_light_ 0 3\n", process.display()),
    )
    .unwrap();
    let refused = std::process::Command::new(&siril)
        .arg("-s")
        .arg(&old)
        .output()
        .unwrap();
    assert!(!refused.status.success());
}

/// The progression of a real night with the real Siril: run by hand with
/// `cargo test -p astro-sync-core real_siril -- --ignored --nocapture`,
/// `ASTRO_SYNC_REAL_NIGHT` (a night folder, its frames are only read),
/// `ASTRO_SYNC_REAL_TARGET` (its target folder) and `ASTRO_SYNC_SIRIL`
/// (`siril-cli`). The JPEGs stay in `<target>/progression/`; nothing else
/// is left on the disk.
#[test]
#[ignore = "needs a real night and Siril: ASTRO_SYNC_REAL_NIGHT, ASTRO_SYNC_REAL_TARGET, ASTRO_SYNC_SIRIL"]
fn real_siril_makes_colour_milestones_on_the_nights_disk() {
    let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"));
    let night = PathBuf::from(var("ASTRO_SYNC_REAL_NIGHT"));
    let target = var("ASTRO_SYNC_REAL_TARGET");
    let siril = PathBuf::from(var("ASTRO_SYNC_SIRIL"));
    let version = std::env::var("ASTRO_SYNC_SIRIL_VERSION").unwrap_or_else(|_| "1.4.4".into());
    let family = target_families(&night, &target).remove(0);
    let job = ProgressionJob::new(&night, &target, family);
    let made = run_progression_images(
        &job,
        true,
        Some((&siril, &version)),
        &crate::stacking::free_bytes,
        Duration::from_secs(3600),
        None,
    )
    .unwrap_or_else(|failure| panic!("{failure:?}"));
    println!("{}", made.script);
    for image in &made.images {
        println!(
            "milestone {} — {} frame(s): {} ({} bytes)",
            image.index,
            image.frame_count,
            image.path.display(),
            image.size
        );
    }
    assert!(made.colour, "the night's first frame has a BAYERPAT");
    assert!(made.script.contains("-debayer"));
    assert!(!night.join(".astro-sync-work").exists());
    assert!(
        made.images
            .iter()
            .all(|i| i.path.starts_with(&job.output_dir))
    );
}

/// Siril's `calibrate -debayer` leaves a Fujifilm X-Trans mosaic as it is, so
/// such a night is debayered at every conversion and calibrated as colour
/// frames.
#[test]
fn an_x_trans_night_is_debayered_when_every_frame_is_converted() {
    let family = AcquisitionFamily {
        key: "nofilter_30s_g500".to_string(),
        filter: None,
        exposure_seconds: Some(30.0),
        gain: Some(500.0),
        binning: None,
        temperature_class_c: None,
        camera: None,
        frame_paths: vec!["a.RAF".to_string(), "b.RAF".to_string()],
    };
    let work = Path::new("/w");
    let x_trans = ScriptInputs {
        xtrans: true,
        ..inputs(true, true, true)
    };
    let (script, prefix) = registration_script(work, &family, x_trans);
    assert!(
        script.contains("convert dark -debayer -out=../process"),
        "{script}"
    );
    assert!(
        script.contains("convert flat -debayer -out=../process"),
        "{script}"
    );
    assert!(
        script.contains("convert light -debayer -out=../process"),
        "{script}"
    );
    let calibrate = script
        .lines()
        .find(|line| line.starts_with("calibrate light_"))
        .unwrap();
    assert_eq!(
        calibrate,
        "calibrate light_ -dark=master_dark -flat=master_flat"
    );
    assert_eq!(prefix, "r_pp_light_");
    // A Bayer night keeps its recipe.
    let (script, _) = registration_script(work, &family, inputs(true, true, true));
    assert!(script.contains("convert dark -out=../process"), "{script}");
    assert!(script.contains("-cfa -equalize_cfa -debayer"), "{script}");
}

#[test]
fn the_record_of_a_progression_gives_its_milestones_their_poses_for_a_later_sending() {
    let dir = TempDir::new("progression-record");
    let output = dir.path().join("M31/progression");
    fs::create_dir_all(&output).unwrap();
    let mut images = Vec::new();
    for (index, frames) in [(1usize, 1usize), (2, 12), (3, 24)] {
        let path = output.join(format!("milestone_{index:02}.jpg"));
        fs::write(&path, format!("jpeg {index}")).unwrap();
        images.push(ProgressionImage {
            index,
            frame_count: frames,
            path,
            sha256: String::new(),
            size: 0,
        });
    }
    write_record(&output, &images).unwrap();
    // A file of the person in the folder is not a milestone, and the record
    // names nothing that is not there.
    fs::write(output.join("mine.jpg"), "mine").unwrap();
    fs::remove_file(output.join("milestone_02.jpg")).unwrap();

    let found = recorded_candidates(dir.path(), "M31", "m31");
    assert_eq!(
        found.iter().map(|c| c.frame_count).collect::<Vec<_>>(),
        vec![Some(1), Some(24)]
    );
    assert!(found.iter().all(|c| c.kind == ResultKind::Progression));
    assert!(
        found
            .iter()
            .all(|c| c.target_id == "m31" && c.sha256.len() == 64)
    );
    // No record, no milestone to send.
    assert!(recorded_candidates(dir.path(), "NGC 7000", "ngc7000").is_empty());
}

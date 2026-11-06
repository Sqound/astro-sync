use super::*;
#[cfg(unix)]
use crate::external_process::fake_tool;
use crate::test_support::TempDir;
use std::fs;
use std::path::{Path, PathBuf};

const MEASURED: &str = r#"{"status": "measured", "quality": {"fwhmPixels": 3.8, "eccentricity": 0.31, "starCount": 212, "measuredOn": "linear", "measuredAt": "2026-10-14T09:00:00Z"}}"#;

fn measured() -> MeasuredQuality {
    MeasuredQuality {
        fwhm_pixels: 3.8,
        eccentricity: Some(0.31),
        star_count: 212,
        measured_on: Some("linear".into()),
        measured_at: Some("2026-10-14T09:00:00Z".into()),
    }
}

#[test]
fn the_answer_of_the_script_is_its_last_line() {
    let answer = parse_answer(&format!("a warning on stdout\n{MEASURED}\n\n")).unwrap();
    assert_eq!(answer, Answer::Measured(measured()));
    let refused = parse_answer(
        r#"{"status": "refused", "code": "stretched", "message": "the median of the image sits at 25% of its range"}"#,
    )
    .unwrap();
    assert_eq!(
        refused,
        Answer::Refused {
            code: "stretched".into(),
            message: "the median of the image sits at 25% of its range".into()
        }
    );
    // A code the window has no sentence for is « unreadable », never an
    // unknown word on the screen.
    let strange = parse_answer(r#"{"status": "refused", "code": "weird"}"#).unwrap();
    assert!(matches!(strange, Answer::Refused { code, .. } if code == "unreadable"));
    for nothing in [
        "",
        "not json",
        r#"{"status": "maybe"}"#,
        r#"{"status": "measured"}"#,
    ] {
        assert_eq!(parse_answer(nothing), None, "{nothing}");
    }
}

#[test]
fn only_one_path_component_names_a_target_folder() {
    for good in ["M31", "NGC 7000", "IC 1396", "Cœur"] {
        assert!(is_folder_name(good), "{good}");
    }
    for bad in ["", ".", "..", "../x", "a/b", "a\\b", ".hidden", "a\0b"] {
        assert!(!is_folder_name(bad), "{bad:?}");
    }
}

#[test]
fn a_measure_is_kept_in_a_hidden_file_of_the_target_and_forgotten() {
    let dir = TempDir::new("master-quality-record");
    let night = dir.path();
    fs::create_dir_all(night.join("M31/final")).unwrap();
    save(
        night,
        "M31",
        Path::new("/somewhere/M31_integration.xisf"),
        &measured(),
    )
    .unwrap();
    // Only the file's name is kept, never the folder it came from.
    let text = fs::read_to_string(night.join("M31").join(FILE_NAME)).unwrap();
    assert!(
        text.contains("M31_integration.xisf") && !text.contains("somewhere"),
        "{text}"
    );
    assert!(FILE_NAME.starts_with('.'));
    let record = load(night, "M31").unwrap();
    assert_eq!(record.master, "M31_integration.xisf");
    assert_eq!(record.quality, measured());
    // The images of the target find it, those of another target do not.
    let near = |relative: &str| record_near(&night.join(relative), night);
    assert!(near("M31/final/M31.tif").is_some());
    assert!(near("M31/stacked/M31_raw.tif").is_some());
    assert!(near("NGC 7000/final/NGC.tif").is_none());
    assert!(record_near(Path::new("/elsewhere/M31/final/x.tif"), night).is_none());
    forget(night, "M31").unwrap();
    forget(night, "M31").unwrap(); // already gone: nothing to say
    assert!(load(night, "M31").is_none());
}

#[cfg(unix)]
#[test]
fn the_script_is_asked_for_the_master_and_its_answer_read() {
    let dir = TempDir::new("master-quality-run");
    let record = dir.path().join("arguments.txt");
    let python = fake_tool(
        dir.path(),
        "python3",
        &format!("echo \"$@\" > '{}'\necho '{MEASURED}'", record.display()),
    );
    let answer = measure(
        &python,
        &dir.path().join("star_quality.py"),
        &dir.path().join("master.xisf"),
        None,
    )
    .unwrap();
    assert_eq!(answer, Answer::Measured(measured()));
    let given = fs::read_to_string(record).unwrap();
    assert!(
        given.contains("star_quality.py --measure ") && given.contains("master.xisf"),
        "{given}"
    );

    let silent = fake_tool(
        dir.path(),
        "python-silent",
        "echo 'Traceback: boom' >&2\nexit 1",
    );
    let failure = measure(
        &silent,
        Path::new("star_quality.py"),
        Path::new("m.fit"),
        None,
    )
    .unwrap_err();
    assert_eq!(failure, Failure::NoAnswer("Traceback: boom".into()));
    let missing = measure(
        &dir.path().join("no-such-python"),
        Path::new("star_quality.py"),
        Path::new("m.fit"),
        None,
    )
    .unwrap_err();
    assert!(matches!(missing, Failure::NotRun(_)));
}

fn stub_script(dir: &Path) -> PathBuf {
    let script = dir.join("star_quality.py");
    fs::write(
        &script,
        "# the stub python answers whatever the file says\n",
    )
    .unwrap();
    script
}

#[cfg(unix)]
#[test]
fn a_master_is_measured_kept_and_given_back_as_the_quality_of_its_target() {
    let dir = TempDir::new("master-quality-target");
    let night = dir.path().join("night");
    fs::create_dir_all(night.join("M31")).unwrap();
    let master = dir.path().join("M31_integration.xisf");
    fs::write(&master, "x").unwrap();
    let script = stub_script(dir.path());
    let python = fake_tool(dir.path(), "python3", &format!("echo '{MEASURED}'"));
    let run = |entitled| {
        measure_for_target(&night, "M31", &master, entitled, &python, &script, None).unwrap()
    };

    // Without the right nothing is measured and nothing is kept.
    assert_eq!(
        run(false),
        Outcome::Refused {
            code: "reserved".into()
        }
    );
    assert!(load(&night, "M31").is_none());

    let Outcome::Measured {
        master: name,
        quality,
    } = run(true)
    else {
        panic!("not measured")
    };
    assert_eq!(name, "M31_integration.xisf");
    assert_eq!((quality.fwhm_pixels, quality.star_count), (3.8, 212));
    assert_eq!(quality.fwhm_arcsec, None); // the frames say nothing of their optics here
    assert_eq!(
        current(&night, "M31"),
        Some(Outcome::Measured {
            master: name,
            quality
        })
    );
    assert_eq!(current(&night, "NGC 7000"), None);
    assert_eq!(current(&night, "../M31"), None);
}

#[cfg(unix)]
#[test]
fn what_cannot_be_measured_is_said_in_a_word_and_changes_nothing() {
    let dir = TempDir::new("master-quality-refused");
    let night = dir.path().join("night");
    fs::create_dir_all(night.join("M31")).unwrap();
    let master = dir.path().join("master.xisf");
    fs::write(&master, "x").unwrap();
    let script = stub_script(dir.path());
    let good = fake_tool(dir.path(), "python3", &format!("echo '{MEASURED}'"));
    measure_for_target(&night, "M31", &master, true, &good, &script, None).unwrap();
    let kept = load(&night, "M31").unwrap();

    let stretched = fake_tool(
        dir.path(),
        "python-stretched",
        r#"echo '{"status": "refused", "code": "stretched", "message": "m"}'"#,
    );
    let word = |python: &Path, target: &str, file: &Path| match measure_for_target(
        &night, target, file, true, python, &script, None,
    )
    .unwrap()
    {
        Outcome::Refused { code } => code,
        other => panic!("{other:?}"),
    };
    assert_eq!(word(&stretched, "M31", &master), "stretched");
    assert_eq!(word(&good, "NGC 7000", &master), "night_missing");
    assert_eq!(word(&good, "../x", &master), "night_missing");
    assert_eq!(
        word(&good, "M31", &dir.path().join("gone.fit")),
        "unreadable"
    );
    assert_eq!(
        word(&dir.path().join("no-python"), "M31", &master),
        "python_missing"
    );
    // A refusal never replaces what was measured before.
    assert_eq!(load(&night, "M31").unwrap(), kept);
}

/// G5: the command line's `measure-master <file>` measures without a night,
/// with the window's script and answers, and keeps nothing.
#[cfg(unix)]
#[test]
fn a_master_measured_alone_gives_the_window_s_outcome_and_keeps_nothing() {
    let dir = TempDir::new("master-quality-alone");
    let master = dir.path().join("M31_integration.fit");
    fs::write(&master, "x").unwrap();
    let script = stub_script(dir.path());
    let good = fake_tool(dir.path(), "python3", &format!("echo '{MEASURED}'"));
    let Outcome::Measured {
        master: name,
        quality,
    } = measure_alone(&master, &good, &script, None).unwrap()
    else {
        panic!("not measured")
    };
    assert_eq!(name, "M31_integration.fit");
    assert_eq!((quality.fwhm_pixels, quality.star_count), (3.8, 212));
    assert_eq!(quality.fwhm_arcsec, None, "no night, no pixel scale");
    let stretched = fake_tool(
        dir.path(),
        "python-stretched",
        r#"echo '{"status": "refused", "code": "stretched", "message": "m"}'"#,
    );
    assert_eq!(
        measure_alone(&master, &stretched, &script, None).unwrap(),
        Outcome::Refused {
            code: "stretched".into()
        }
    );
    assert_eq!(
        measure_alone(&dir.path().join("gone.fit"), &good, &script, None).unwrap(),
        Outcome::Refused {
            code: "unreadable".into()
        }
    );
    assert_eq!(
        measure_alone(&master, &dir.path().join("no-python"), &script, None).unwrap(),
        Outcome::Refused {
            code: "python_missing".into()
        }
    );
    // The master, the script and the two stubs: nothing else was written.
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 4);
}

use super::*;
use crate::external_process::fake_tool;
use crate::test_support::TempDir;
use std::collections::BTreeMap;

/// The scripts beside the app, three pinned packages.
fn resources(dir: &Path) -> StackingResources {
    let stacking = dir.join("stacking");
    fs::create_dir_all(&stacking).unwrap();
    fs::write(stacking.join("astro_sync_stack.py"), b"").unwrap();
    fs::write(stacking.join("check_environment.py"), b"").unwrap();
    fs::write(
        stacking.join("requirements.txt"),
        "# pinned\nnumpy==2.5.1 \\\n    --hash=sha256:aa\nastropy==8.0.1 \\\n    --hash=sha256:bb\ntifffile==2026.7.31 \\\n    --hash=sha256:cc\n",
    )
    .unwrap();
    StackingResources::find(&stacking).unwrap()
}

const CHECK_OK: &str = r#"{"python": {"version": "3.13.7", "ok": true, "supported": ">=3.12,<3.14"},
  "packages": [{"name": "numpy", "pinned": "2.5.1", "installed": "2.5.1", "ok": true},
               {"name": "astropy", "pinned": "8.0.1", "installed": "8.0.1", "ok": true}],
  "siril": {"path_found": false, "version": null}, "ok": false}"#;

/// A Python standing for the real one: `--version`, `-m venv` (copies
/// itself into the environment), `-m pip install` (fails while `fail-pip`
/// exists), the check (answers `check.json`). Every call is logged.
fn fake_python(dir: &Path, bin: &Path, name: &str, version: &str) -> PathBuf {
    let log = dir.join("calls.log");
    let fail = dir.join("fail-pip");
    let answer = dir.join("check.json");
    fake_tool(
        bin,
        name,
        &format!(
            r#"echo "$@" >> '{log}'
case "$1" in
  --version) echo "Python {version}" ;;
  -m)
    if [ "$2" = venv ]; then mkdir -p "$3/bin" && cp "$0" "$3/bin/python3"; exit 0; fi
    if [ -f '{fail}' ]; then echo 'Collecting numpy==2.5.1'; echo 'ERROR: connection reset' >&2; exit 1; fi
    echo 'Collecting numpy==2.5.1'; echo 'Requirement already satisfied: astropy==8.0.1'; echo 'Collecting tifffile==2026.7.31'; echo 'Successfully installed numpy tifffile'
    ;;
  *) cat '{answer}' ;;
esac"#,
            log = log.display(),
            fail = fail.display(),
            answer = answer.display(),
        ),
    )
}

fn context(dir: &Path, bin: &Path) -> DetectContext {
    DetectContext {
        data_dir: dir.join("data"),
        reference_stars: dir.join("data/reference-stars.bin"),
        search_dirs: vec![bin.to_path_buf()],
        known_paths: BTreeMap::new(),
        probe_timeout: Duration::from_secs(10),
        stacking: None,
    }
}

fn calls(dir: &Path, needle: &str) -> usize {
    fs::read_to_string(dir.join("calls.log"))
        .unwrap_or_default()
        .lines()
        .filter(|line| line.contains(needle))
        .count()
}

#[test]
fn the_range_is_the_registrys_and_the_newest_python_in_it_is_chosen() {
    let range = python_range();
    assert_eq!(
        range,
        PythonRange {
            min: (3, 12),
            below: (3, 14)
        }
    );
    assert!(range.contains("3.12.13") && range.contains("3.13.14"));
    assert!(!range.contains("3.11.16") && !range.contains("3.14.7") && !range.contains("3.9.6"));
    assert_eq!(
        range.first_and_last(),
        ("3.12".to_string(), "3.13".to_string())
    );
    assert_eq!(parse_range("nonsense"), None);

    let dir = TempDir::new("python-find");
    let bin = dir.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    fake_python(dir.path(), &bin, "python3", "3.9.6");
    fake_python(dir.path(), &bin, "python3.12", "3.12.13");
    fake_python(dir.path(), &bin, "python3.13", "3.13.7");
    let found = find_python(&context(dir.path(), &bin), range).unwrap();
    assert_eq!(found.version, "3.13.7");
    assert_eq!(found.path, bin.join("python3.13"));
}

#[test]
fn without_a_python_in_the_range_the_environment_is_missing_and_python_org_is_said() {
    let dir = TempDir::new("python-absent");
    let bin = dir.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    fake_python(dir.path(), &bin, "python3", "3.9.6");
    let context = context(dir.path(), &bin);
    let resources = resources(dir.path());
    let status = environment_status(&context.data_dir, Some(&resources));
    assert_eq!(status.state, ToolState::Missing);
    assert!(!status.interrupted);
    let failure = install_environment(
        &context,
        Some(&resources),
        EnvTimeouts::default(),
        &CancelFlag::new(),
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(
        failure,
        EnvFailure::NoPython {
            first: "3.12".to_string(),
            last: "3.13".to_string()
        }
    );
    assert!(!env_dir(&context.data_dir).exists(), "nothing was created");
    // Without the scripts beside the app, nothing can be installed.
    assert_eq!(
        install_environment(
            &context,
            None,
            EnvTimeouts::default(),
            &CancelFlag::new(),
            &mut |_| {}
        )
        .unwrap_err(),
        EnvFailure::NoResources
    );
}

#[test]
fn the_environment_is_created_installed_checked_then_present() {
    let dir = TempDir::new("python-install");
    let bin = dir.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    fake_python(dir.path(), &bin, "python3.13", "3.13.7");
    fs::write(dir.path().join("check.json"), CHECK_OK).unwrap();
    let context = context(dir.path(), &bin);
    let resources = resources(dir.path());
    assert_eq!(resources.package_count(), 3);

    let mut seen = Vec::new();
    let check = install_environment(
        &context,
        Some(&resources),
        EnvTimeouts::default(),
        &CancelFlag::new(),
        &mut |p| seen.push(p),
    )
    .unwrap();
    assert!(check.ok);
    assert_eq!(check.python_version, "3.13.7");
    let steps: Vec<EnvStep> = seen.iter().map(|p| p.step).collect();
    assert_eq!(steps.first(), Some(&EnvStep::Python));
    assert!(steps.contains(&EnvStep::Environment));
    assert_eq!(steps.last(), Some(&EnvStep::Check));
    let packages: Vec<u32> = seen
        .iter()
        .filter(|p| p.step == EnvStep::Packages)
        .map(|p| p.packages_done)
        .collect();
    assert_eq!(packages, vec![0, 1, 2, 3]);
    assert!(seen.iter().all(|p| p.packages_total == 3 && p.total == 4));

    // pip pinned and checked, binaries only; every script run without
    // bytecode nor the session's variables.
    let log = fs::read_to_string(dir.path().join("calls.log")).unwrap();
    assert!(log.contains("-m venv"), "{log}");
    assert!(
        log.contains("-m pip install --require-hashes --only-binary=:all:"),
        "{log}"
    );
    assert!(log.contains("-B -E -s"), "{log}");

    let status = environment_status(&context.data_dir, Some(&resources));
    assert_eq!(status.state, ToolState::Present);
    assert_eq!(status.python_version.as_deref(), Some("3.13.7"));
    assert_eq!(status.path, Some(env_dir(&context.data_dir)));

    // Another Astro Sync with other pins: outdated, until installed again.
    fs::write(
        resources.requirements(),
        "numpy==2.6.0 \\\n --hash=sha256:dd\n",
    )
    .unwrap();
    let status = environment_status(&context.data_dir, Some(&resources));
    assert_eq!(status.state, ToolState::Outdated);
}

#[test]
fn an_environment_made_with_a_python_out_of_the_range_is_outdated_and_made_again() {
    let dir = TempDir::new("python-out-of-range");
    let bin = dir.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    fake_python(dir.path(), &bin, "python3.14", "3.14.7");
    fake_python(dir.path(), &bin, "python3.13", "3.13.7");
    fs::write(dir.path().join("check.json"), CHECK_OK).unwrap();
    let context = context(dir.path(), &bin);
    let resources = resources(dir.path());
    // The environment an earlier Astro Sync made with 3.14, checked and
    // marked with the same pins.
    let env = env_dir(&context.data_dir);
    fs::create_dir_all(env_python(&context.data_dir).parent().unwrap()).unwrap();
    fs::copy(bin.join("python3.14"), env_python(&context.data_dir)).unwrap();
    fs::write(
        env.join("pyvenv.cfg"),
        "home = /usr/bin\nversion = 3.14.7\n",
    )
    .unwrap();
    fs::write(env.join("left-by-3.14"), b"").unwrap();
    fs::write(
        env.join(MARKER),
        serde_json::json!({
            "format": 1,
            "pythonVersion": "3.14.7",
            "requirementsSha256": resources.requirements_sha256().unwrap()
        })
        .to_string(),
    )
    .unwrap();
    let status = environment_status(&context.data_dir, Some(&resources));
    assert_eq!(status.state, ToolState::Outdated);

    let check = install_environment(
        &context,
        Some(&resources),
        EnvTimeouts::default(),
        &CancelFlag::new(),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(check.python_version, "3.13.7");
    assert!(!env.join("left-by-3.14").exists(), "made again, not reused");
    let log = fs::read_to_string(dir.path().join("calls.log")).unwrap();
    assert!(log.contains("-m venv"), "{log}");
    let status = environment_status(&context.data_dir, Some(&resources));
    assert_eq!(status.state, ToolState::Present);
    assert_eq!(status.python_version.as_deref(), Some("3.13.7"));
}

#[test]
fn a_failed_installation_is_said_kept_and_resumed() {
    let dir = TempDir::new("python-resume");
    let bin = dir.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    fake_python(dir.path(), &bin, "python3.13", "3.13.7");
    fs::write(dir.path().join("check.json"), CHECK_OK).unwrap();
    fs::write(dir.path().join("fail-pip"), b"").unwrap();
    let context = context(dir.path(), &bin);
    let resources = resources(dir.path());
    let install = || {
        install_environment(
            &context,
            Some(&resources),
            EnvTimeouts::default(),
            &CancelFlag::new(),
            &mut |_| {},
        )
    };
    match install().unwrap_err() {
        EnvFailure::Packages { detail } => assert!(detail.contains("connection reset"), "{detail}"),
        other => panic!("{other:?}"),
    }
    let status = environment_status(&context.data_dir, Some(&resources));
    assert_eq!(status.state, ToolState::Missing);
    assert!(status.interrupted, "« Resume the installation »");

    // The connection is back: the environment is kept, pip resumes.
    fs::remove_file(dir.path().join("fail-pip")).unwrap();
    assert!(install().unwrap().ok);
    assert_eq!(calls(dir.path(), "-m venv"), 1, "the environment is kept");
    assert_eq!(calls(dir.path(), "-m pip install"), 2);
    assert_eq!(
        environment_status(&context.data_dir, Some(&resources)).state,
        ToolState::Present
    );

    // A stop asked before pip: said, nothing marked.
    let cancel = CancelFlag::new();
    cancel.cancel();
    assert_eq!(
        install_environment(
            &context,
            Some(&resources),
            EnvTimeouts::default(),
            &cancel,
            &mut |_| {}
        )
        .unwrap_err(),
        EnvFailure::Cancelled
    );
    assert!(environment_status(&context.data_dir, Some(&resources)).interrupted);
}

#[test]
fn a_check_that_finds_a_problem_leaves_no_mark() {
    let dir = TempDir::new("python-check");
    let bin = dir.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    fake_python(dir.path(), &bin, "python3.13", "3.13.7");
    fs::write(
        dir.path().join("check.json"),
        CHECK_OK.replacen(
            r#""installed": "8.0.1", "ok": true"#,
            r#""installed": "7.0.0", "ok": false"#,
            1,
        ),
    )
    .unwrap();
    let context = context(dir.path(), &bin);
    let resources = resources(dir.path());
    match install_environment(
        &context,
        Some(&resources),
        EnvTimeouts::default(),
        &CancelFlag::new(),
        &mut |_| {},
    )
    .unwrap_err()
    {
        EnvFailure::Check { problems, .. } => assert_eq!(problems, vec!["astropy"]),
        other => panic!("{other:?}"),
    }
    assert!(!env_dir(&context.data_dir).join(MARKER).exists());
    assert_eq!(
        read_check("not json"),
        None,
        "an unreadable answer is no check"
    );
}

#[test]
fn a_script_is_told_which_version_of_astro_sync_runs_it() {
    // The scripts stamp their TIFF files with it.
    let command = script_command(
        Path::new("/env/bin/python3"),
        Path::new("/res/plan_stack.py"),
    );
    let version = command
        .get_envs()
        .find(|(name, _)| *name == SOFTWARE_VERSION_VARIABLE)
        .and_then(|(_, value)| value)
        .map(|value| value.to_string_lossy().into_owned());
    assert_eq!(version.as_deref(), Some(crate::version()));
}

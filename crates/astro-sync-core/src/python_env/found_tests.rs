//! An environment already in Astro Sync's data -- the one 0.1.0 left there
//! before an update -- is found and said as it is (its path, its Python, why
//! installing redoes something), never « missing »: the smoke of 1.0.1 on
//! Windows offered to install 101.8 MB over the `stacking-env` 0.1.0 made.
//! Each environment is simulated: the interpreter where this system's layout
//! puts it (`Scripts\python.exe` on Windows, `bin/python3` elsewhere), its
//! `pyvenv.cfg`, with or without the mark.

use super::*;
use crate::test_support::TempDir;

/// The scripts beside the app, one pinned package.
fn resources(dir: &Path) -> StackingResources {
    let stacking = dir.join("stacking");
    fs::create_dir_all(&stacking).unwrap();
    fs::write(stacking.join("astro_sync_stack.py"), b"").unwrap();
    fs::write(stacking.join("check_environment.py"), b"").unwrap();
    fs::write(
        stacking.join("requirements.txt"),
        "numpy==2.5.1 \\\n    --hash=sha256:aa\n",
    )
    .unwrap();
    StackingResources::find(&stacking).unwrap()
}

/// An environment made with Python `version`, as `python -m venv` leaves it.
fn simulated_env(data: &Path, version: &str) {
    let python = env_python(data);
    fs::create_dir_all(python.parent().unwrap()).unwrap();
    fs::write(&python, b"").unwrap();
    fs::write(
        env_dir(data).join("pyvenv.cfg"),
        format!("home = /usr/bin\ninclude-system-site-packages = false\nversion = {version}\n"),
    )
    .unwrap();
}

fn mark(data: &Path, python_version: &str, requirements_sha256: &str) {
    fs::write(
        env_dir(data).join(MARKER),
        serde_json::json!({
            "format": 1,
            "pythonVersion": python_version,
            "requirementsSha256": requirements_sha256,
        })
        .to_string(),
    )
    .unwrap();
}

#[test]
fn an_environment_an_earlier_astro_sync_made_is_found_with_its_path_and_python() {
    let dir = TempDir::new("python-found-unchecked");
    let data = dir.path().join("data");
    let resources = resources(dir.path());
    simulated_env(&data, "3.13.1");

    let status = environment_status(&data, Some(&resources));
    assert_eq!(
        status.state,
        ToolState::Outdated,
        "found, never « missing »"
    );
    assert_eq!(status.redo, Some(EnvRedo::Unchecked));
    assert_eq!(status.path.as_deref(), Some(env_dir(&data).as_path()));
    assert_eq!(status.python_version.as_deref(), Some("3.13.1"));
    assert!(!status.interrupted);
}

#[test]
fn an_environment_of_other_pins_or_another_python_says_what_will_be_redone() {
    let dir = TempDir::new("python-found-outdated");
    let data = dir.path().join("data");
    let resources = resources(dir.path());
    simulated_env(&data, "3.13.1");

    // Marked by another version of Astro Sync: its packages are updated.
    mark(&data, "3.13.1", "another-sha");
    let status = environment_status(&data, Some(&resources));
    assert_eq!(status.state, ToolState::Outdated);
    assert_eq!(status.redo, Some(EnvRedo::OtherPins));
    assert_eq!(status.python_version.as_deref(), Some("3.13.1"));

    // This Astro Sync's pins: present.
    mark(&data, "3.13.1", &resources.requirements_sha256().unwrap());
    let status = environment_status(&data, Some(&resources));
    assert_eq!(status.state, ToolState::Present);
    assert_eq!(status.redo, None);

    // A Python out of the range (0.1.0 took 3.14): made again.
    mark(&data, "3.14.0", &resources.requirements_sha256().unwrap());
    assert_eq!(
        environment_status(&data, Some(&resources)).redo,
        Some(EnvRedo::OtherPython)
    );
    fs::remove_file(env_dir(&data).join(MARKER)).unwrap();
    simulated_env(&data, "3.14.0");
    let status = environment_status(&data, Some(&resources));
    assert_eq!(status.state, ToolState::Outdated);
    assert_eq!(status.redo, Some(EnvRedo::OtherPython));
    assert_eq!(status.python_version.as_deref(), Some("3.14.0"));
}

#[test]
fn an_installation_under_way_or_no_interpreter_is_still_missing() {
    let dir = TempDir::new("python-found-installing");
    let data = dir.path().join("data");
    let resources = resources(dir.path());

    // Nothing at all.
    let status = environment_status(&data, Some(&resources));
    assert_eq!(status.state, ToolState::Missing);
    assert!(!status.interrupted);

    // An installation that stopped after the environment was made.
    simulated_env(&data, "3.13.1");
    fs::write(env_dir(&data).join(INSTALLING), b"").unwrap();
    let status = environment_status(&data, Some(&resources));
    assert_eq!(status.state, ToolState::Missing);
    assert!(status.interrupted, "« Resume the installation »");
    assert_eq!(status.redo, None);

    // A folder without its interpreter.
    fs::remove_file(env_python(&data)).unwrap();
    fs::remove_file(env_dir(&data).join(INSTALLING)).unwrap();
    let status = environment_status(&data, Some(&resources));
    assert_eq!(status.state, ToolState::Missing);
    assert!(status.interrupted);
}

#[test]
fn the_interpreter_is_where_this_system_puts_it() {
    let data = Path::new("data");
    let python = env_python(data);
    if cfg!(windows) {
        assert!(python.ends_with(Path::new("stacking-env").join("Scripts").join("python.exe")));
    } else {
        assert!(python.ends_with(Path::new("stacking-env").join("bin").join("python3")));
    }
}

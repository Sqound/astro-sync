//! `astro-sync --version`: the version, then the commit and the day stamped
//! at build time -- `astro-sync 1.0.0 (6aa28f767, 2026-10-13)` -- the same
//! `build_info` the window shows in its menu.

use std::process::Command;

#[test]
fn the_version_flag_names_the_version_the_commit_and_the_day() {
    let output = Command::new(env!("CARGO_BIN_EXE_astro-sync"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    let line = String::from_utf8(output.stdout).unwrap();
    let line = line.trim();
    let rest = line
        .strip_prefix(&format!("astro-sync {} (", env!("CARGO_PKG_VERSION")))
        .unwrap_or_else(|| panic!("unexpected version line: {line}"))
        .strip_suffix(')')
        .unwrap_or_else(|| panic!("unexpected version line: {line}"));
    let (sha, day) = rest.split_once(", ").expect("a commit and a day");
    assert!(
        sha == "unknown" || (sha.len() == 9 && sha.chars().all(|c| c.is_ascii_hexdigit())),
        "{sha}"
    );
    assert_eq!(day.len(), 10, "{day}");
    assert!(day.starts_with("20"), "{day}");
}

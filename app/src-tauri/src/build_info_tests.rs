//! `app_build_info`, driven through the IPC on the mock runtime like the
//! page does: the version comes from the configuration of the application,
//! the commit and the date from the stamp of the build.

use std::time::Duration;

use crate::wake_tests::{fast, harness, serialized};

#[test]
fn the_page_receives_the_version_the_commit_and_the_date_of_the_build() {
    let _serial = serialized();
    let harness = harness(fast());
    let info = harness
        .invoke("app_build_info", serde_json::json!({}))
        .answer(Duration::from_secs(5))
        .unwrap();
    assert!(
        info["version"].as_str().is_some_and(|v| !v.is_empty()),
        "{info}"
    );
    let sha = info["gitSha"].as_str().unwrap();
    assert!(
        sha == "unknown" || (sha.len() == 9 && sha.chars().all(|c| c.is_ascii_hexdigit())),
        "{sha}"
    );
    let date = info["buildDate"].as_str().unwrap();
    assert_eq!(date.len(), 20, "{date}");
    assert!(date.ends_with('Z'), "{date}");
}

#[test]
fn a_build_outside_a_repository_reads_unknown() {
    let info = astro_sync_core::build_info::from_stamp("0.1.0", "", "");
    assert_eq!(info.git_sha, "unknown");
    assert_eq!(info.build_date, "");
}

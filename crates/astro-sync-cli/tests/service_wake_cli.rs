//! What a person at a terminal sees when the service is asleep, with
//! the real `astro-sync` binary and its real, default waiting policy
//! (nothing shortened): the service takes a few seconds to answer the first
//! request, the command says the service is starting -- after 4 seconds of
//! silence, not before -- says when it answered, and standard output stays a
//! clean machine-readable result.

#[path = "../../astro-sync-core/tests/support/fake_service.rs"]
mod fake_service;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fake_service::{FakeService, UploadModel, Wake, error_reply};

fn unique_temp_dir(label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "astro-sync-cli-wake-{label}-{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_real_binary_says_the_service_is_starting_and_keeps_stdout_clean() {
    let model = Arc::new(UploadModel::default());
    let app_model = Arc::clone(&model);
    // Longer than the 4 s after which silence is announced, shorter than
    // any limit: the binary runs with its production policy.
    let service = FakeService::start(Wake::Held(Duration::from_secs(5)), move |request| {
        app_model
            .reply(request, 100)
            .unwrap_or_else(|| error_reply(404, "not_found", "no such route"))
    });

    let dir = unique_temp_dir("real-binary");
    let folder = dir.join("night");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("frame.tif"), vec![5_u8; 250]).unwrap();
    let token = dir.join("token.txt");
    std::fs::write(&token, "fake-token").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_astro-sync"))
        .arg("upload")
        .arg("--queue")
        .arg(dir.join("queue.sqlite"))
        .arg("--portal")
        .arg(service.base_url())
        .arg("--token")
        .arg(&token)
        .arg("--folder")
        .arg(&folder)
        .arg("--json")
        .output()
        .expect("run the astro-sync binary");
    let _ = std::fs::remove_dir_all(&dir);

    assert!(output.status.success(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(
        lines,
        [
            "The service is starting. It may take a minute.",
            "The service answered. Sending resumes."
        ],
        "stderr was: {stderr}"
    );
    let results: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("standard output stays one JSON document, whatever is said on standard error");
    assert_eq!(results[0]["ok"], true, "{results}");
    assert_eq!(model.sessions_created(), 1);
    assert_eq!(model.captures_created(), 1);
}

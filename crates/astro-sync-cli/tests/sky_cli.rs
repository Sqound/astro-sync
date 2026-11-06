//! `astro-sync sky`, the real binary: a night of frames that were never
//! solved (ASIAIR sub-frames, typically) gives no value and says why --
//! "unsolved frame" -- and nothing is sent to the service: no value is ever
//! guessed.

#[path = "../../astro-sync-core/tests/support/fake_service.rs"]
mod fake_service;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use fake_service::{FakeService, Reply, Wake};

fn unique_temp_dir(label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "astro-sync-cli-sky-{label}-{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A light frame's header, without a WCS: what an ASIAIR writes.
fn unsolved_frame(folder: &Path, name: &str, date_obs: &str) {
    let mut out = Vec::new();
    for (keyword, value) in [
        ("SIMPLE", "T".to_string()),
        ("BITPIX", "16".to_string()),
        ("NAXIS", "0".to_string()),
        ("IMAGETYP", "'Light'".to_string()),
        ("EXPTIME", "120.0".to_string()),
        ("DATE-OBS", format!("'{date_obs}'")),
        ("SITELAT", "45.0".to_string()),
        ("SITELONG", "-30.0".to_string()),
    ] {
        let mut card = format!("{keyword:<8}= {value}").into_bytes();
        card.resize(80, b' ');
        out.extend_from_slice(&card);
    }
    let mut end = b"END".to_vec();
    end.resize(80, b' ');
    out.extend_from_slice(&end);
    out.resize(2880, b' ');
    std::fs::write(folder.join(name), out).unwrap();
}

fn night_folder(dir: &Path) -> PathBuf {
    let folder = dir.join("night");
    std::fs::create_dir_all(&folder).unwrap();
    for (index, minute) in [0, 10, 20].iter().enumerate() {
        unsolved_frame(
            &folder,
            &format!("Light_{index:04}.fit"),
            &format!("2026-10-11T05:{minute:02}:00"),
        );
    }
    folder
}

#[test]
fn an_unsolved_night_gives_no_value_and_says_unsolved_frame() {
    let dir = unique_temp_dir("unsolved");
    let folder = night_folder(&dir);

    let json = Command::new(env!("CARGO_BIN_EXE_astro-sync"))
        .args(["sky", "--folder"])
        .arg(&folder)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        json.status.success(),
        "{}",
        String::from_utf8_lossy(&json.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(value["measure"]["nights"], serde_json::json!([]));
    assert_eq!(value["uploaded"], serde_json::Value::Null);
    let poses = value["measure"]["poses"].as_array().unwrap();
    assert_eq!(poses.len(), 3);
    assert!(poses.iter().all(|pose| pose["reason"] == "no_wcs"));

    let text = Command::new(env!("CARGO_BIN_EXE_astro-sync"))
        .args(["sky", "--folder"])
        .arg(&folder)
        .output()
        .unwrap();
    let printed = String::from_utf8_lossy(&text.stdout);
    assert!(
        printed.contains("No value: nothing is guessed."),
        "{printed}"
    );
    assert!(
        printed.contains("3 frame(s) without a value: unsolved frame"),
        "{printed}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn without_a_value_nothing_is_sent() {
    let dir = unique_temp_dir("nothing-sent");
    let folder = night_folder(&dir);
    let token = dir.join("token");
    std::fs::write(&token, "device-token\n").unwrap();
    let service = FakeService::start(Wake::Instant, |_| Reply::Json(200, "{}".to_string()));

    let output = Command::new(env!("CARGO_BIN_EXE_astro-sync"))
        .args(["sky", "--folder"])
        .arg(&folder)
        .args(["--portal", &service.base_url(), "--token"])
        .arg(&token)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("no night value to send"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(service.requests().is_empty(), "no request at all");
    let _ = std::fs::remove_dir_all(&dir);
}

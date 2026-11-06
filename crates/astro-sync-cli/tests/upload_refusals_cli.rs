//! `astro-sync upload`, the real binary, when the service refuses a file: what
//! a person at a terminal reads, in French, and what a script reads (`--json`:
//! a stable `code` and the sentence), with the queue left as it should be.
//!
//! A refusal for good sets the file aside and the command goes on; a full
//! storage stops it with every file kept; nothing is ever asked twice.

#[path = "../../astro-sync-core/tests/support/fake_service.rs"]
mod fake_service;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use fake_service::{FakeService, Reply, UploadModel, Wake, error_reply};

fn unique_temp_dir(label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "astro-sync-cli-refusals-{label}-{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// One folder with `files` (250 bytes each: three chunks of the model's
/// 100), a token file and a queue path, all under `dir`.
struct Night {
    queue: PathBuf,
    token: PathBuf,
    folder: PathBuf,
}

fn night(dir: &Path, files: &[&str]) -> Night {
    let folder = dir.join("night");
    std::fs::create_dir_all(&folder).unwrap();
    for (index, name) in files.iter().enumerate() {
        std::fs::write(folder.join(name), vec![index as u8 + 3; 250]).unwrap();
    }
    let token = dir.join("token.txt");
    std::fs::write(&token, "fake-token").unwrap();
    Night {
        queue: dir.join("queue.sqlite"),
        token,
        folder,
    }
}

fn upload(night: &Night, portal: &str, json: bool) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_astro-sync"));
    command
        .arg("upload")
        .arg("--queue")
        .arg(&night.queue)
        .arg("--portal")
        .arg(portal)
        .arg("--token")
        .arg(&night.token)
        .arg("--folder")
        .arg(&night.folder);
    if json {
        command.arg("--json");
    }
    command.output().expect("run the astro-sync binary")
}

fn queue_status(night: &Night) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_astro-sync"))
        .args(["queue", "status", "--json", "--queue"])
        .arg(&night.queue)
        .output()
        .unwrap();
    serde_json::from_slice(&output.stdout).unwrap()
}

/// The service refuses by the name the person gave the file; everything else
/// goes through the model of the resumable protocol.
fn service_refusing(
    model: &Arc<UploadModel>,
    refuse: impl Fn(&str) -> Option<Reply> + Send + Sync + 'static,
) -> FakeService {
    let model = Arc::clone(model);
    FakeService::start(Wake::Instant, move |request| {
        if request.method == "POST" && request.path == "/api/v1/uploads" {
            let name = request.json()["originalName"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            if let Some(reply) = refuse(&name) {
                return reply;
            }
        }
        model
            .reply(request, 100)
            .unwrap_or_else(|| error_reply(404, "not_found", "no such route"))
    })
}

fn creations(service: &FakeService) -> usize {
    service.app_hits("POST", "/api/v1/uploads") - service.app_hits("POST", "/api/v1/uploads/")
}

#[test]
fn a_document_refused_as_a_capture_is_said_in_english_and_the_next_file_goes_on() {
    let dir = unique_temp_dir("svg");
    let model = Arc::new(UploadModel::default());
    let service = service_refusing(&model, |name| {
        (name == "drawing.png").then(|| {
            error_reply(
                415,
                "capture_content_refused",
                "The bytes are an SVG document.",
            )
        })
    });
    let night = night(&dir, &["drawing.png", "frame.tif"]);
    let drawing = night.folder.join("drawing.png");
    let before = std::fs::read(&drawing).unwrap();

    let output = upload(&night, &service.base_url(), false);

    assert!(output.status.success(), "{output:?}");
    let stdout = text(&output.stdout);
    assert!(
        stdout.contains(
            "was refused and will not be sent again. This file is an SVG or HTML document"
        ),
        "{stdout}"
    );
    assert!(
        stdout.contains("export it as JPEG, PNG or TIFF"),
        "{stdout}"
    );
    assert!(
        stdout.contains("The file on your computer was not touched"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("capture_content_refused") && !stdout.contains("SVG document."),
        "never the raw code nor the service's English: {stdout}"
    );
    // The other file went through, once each.
    assert!(stdout.contains("sent (capture"), "{stdout}");
    assert_eq!(model.captures_created(), 1);
    assert_eq!(creations(&service), 2);
    assert_eq!(
        std::fs::read(&drawing).unwrap(),
        before,
        "the file is untouched"
    );

    let entries = queue_status(&night);
    let refused = entries
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["state"] == "refused")
        .expect("one entry is refused");
    assert_eq!(refused["lastErrorCode"], "capture_content_refused");

    // Started again: nothing to send, nothing asked.
    let again = upload(&night, &service.base_url(), false);
    assert!(
        text(&again.stdout).contains("Nothing left to upload"),
        "{again:?}"
    );
    assert_eq!(
        creations(&service),
        2,
        "the refused file is never asked again"
    );

    // The table says it in plain words too.
    let table = Command::new(env!("CARGO_BIN_EXE_astro-sync"))
        .args(["queue", "status", "--queue"])
        .arg(&night.queue)
        .output()
        .unwrap();
    let table = text(&table.stdout);
    assert!(table.contains("Refused"), "{table}");
    assert!(
        table.contains("This file is an SVG or HTML document"),
        "{table}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_script_reads_a_stable_code_and_a_sentence_and_standard_output_stays_json() {
    let dir = unique_temp_dir("json");
    let model = Arc::new(UploadModel::default());
    let service = service_refusing(&model, |name| {
        (name == "huge.tif").then(|| error_reply(413, "file_too_large", "Too large."))
    });
    let night = night(&dir, &["huge.tif"]);

    let output = upload(&night, &service.base_url(), true);

    assert!(output.status.success(), "{output:?}");
    let results: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("standard output is one JSON document");
    assert_eq!(results[0]["ok"], false);
    assert_eq!(results[0]["code"], "file_too_large");
    let sentence = results[0]["error"].as_str().unwrap();
    assert!(sentence.contains("too large"), "{sentence}");
    assert!(!sentence.contains("Too large"), "{sentence}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_full_storage_stops_the_command_with_every_file_kept_and_says_what_to_do() {
    let dir = unique_temp_dir("quota");
    let model = Arc::new(UploadModel::default());
    let service = service_refusing(&model, |_| {
        Some(Reply::Json(
            409,
            r#"{"error":{"code":"storage_quota_reached","message":"The space's storage is full."},"storage":{"usedBytes":10,"quotaBytes":10,"maxFileBytes":104857600}}"#
                .to_string(),
        ))
    });
    let night = night(&dir, &["one.tif", "two.tif"]);

    let output = upload(&night, &service.base_url(), false);

    assert!(output.status.success(), "{output:?}");
    let stdout = text(&output.stdout);
    assert!(
        stdout.contains(
            "Sending has stopped. Hosted storage is not available or is full for your space."
        ),
        "{stdout}"
    );
    assert!(
        stdout.contains("Start sending again as soon as your space has room again"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Your files stay in the queue: nothing is lost"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Run the command again once the problem is solved"),
        "{stdout}"
    );
    assert!(!stdout.contains("storage_quota_reached"), "{stdout}");
    assert_eq!(creations(&service), 1, "the other file is not even tried");

    let entries = queue_status(&night);
    let states: Vec<&str> = entries
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["state"].as_str().unwrap())
        .collect();
    assert_eq!(
        states.iter().filter(|state| **state == "held").count(),
        1,
        "{states:?}"
    );
    assert_eq!(
        states.iter().filter(|state| **state == "queued").count(),
        1,
        "{states:?}"
    );
    assert!(
        !states.contains(&"failed") && !states.contains(&"refused"),
        "{states:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A refusal nobody knows yet is said in French, never with its raw code or
/// the service's English, and the file is not sent again.
#[test]
fn a_refusal_nobody_knows_yet_is_said_plainly_and_not_asked_again() {
    let dir = unique_temp_dir("unknown");
    let model = Arc::new(UploadModel::default());
    let service = service_refusing(&model, |_| {
        Some(error_reply(
            422,
            "a_code_from_a_newer_service",
            "A sentence in the service's own English.",
        ))
    });
    let night = night(&dir, &["frame.tif"]);

    let output = upload(&night, &service.base_url(), false);

    let stdout = text(&output.stdout);
    assert!(
        stdout.contains("The service refused this file and will not accept it as it is."),
        "{stdout}"
    );
    assert!(
        !stdout.contains("a_code_from_a_newer_service") && !stdout.contains("English"),
        "{stdout}"
    );
    assert_eq!(creations(&service), 1);
    let _ = upload(&night, &service.base_url(), false);
    assert_eq!(creations(&service), 1, "not asked again");
    let _ = std::fs::remove_dir_all(&dir);
}

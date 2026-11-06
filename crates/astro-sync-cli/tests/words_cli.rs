//! The real binary: what the command line says when the service refuses,
//! cannot be reached or is asleep, and what it sends after a stacking. The sentences are the application's ten
//! dictionaries (`app/src/i18n`), read here from the files: a code, the name of
//! a Rust variant or the service's raw English is never what a person reads.

#[path = "../../astro-sync-core/tests/support/fake_service.rs"]
mod fake_service;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fake_service::{FakeService, Reply, UploadModel, Wake, error_reply};

fn temp(label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "astro-sync-cli-words-{label}-{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn s(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_astro-sync"))
        // The computer's own language never decides a test.
        .env_remove("ASTRO_SYNC_LANG")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .args(args)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn dictionary(locale: &str) -> serde_json::Value {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../app/src/i18n/{locale}.json"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../astro-sync-core/tests/fixtures/online")
            .join(name),
    )
    .unwrap()
}

/// A FITS header of 2880 bytes, no pixel.
fn fits(cards: &[(&str, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (keyword, value) in cards {
        let mut card = format!("{keyword:<8}= {value}").into_bytes();
        card.resize(80, b' ');
        out.extend_from_slice(&card);
    }
    let mut end = b"END".to_vec();
    end.resize(80, b' ');
    out.extend_from_slice(&end);
    out.resize(out.len().div_ceil(2880) * 2880, b' ');
    out
}

const NIGHT_ID: &str = "7f3a2c91-0000-4000-8000-00000000000a";

/// A night folder of M31 made by `night-folder`, with three lights and a
/// final PNG.
fn night(root: &Path) -> PathBuf {
    let output = run(&[
        "night-folder",
        "--root",
        &s(root),
        "--night-id",
        NIGHT_ID,
        "--service-url",
        "https://service.example",
        "--evening-date",
        "2026-10-07",
        "--label",
        "Col des Étoiles",
        "--target",
        "m31=M31",
        "--json",
    ]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    let made: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let folder = PathBuf::from(made["folder"].as_str().unwrap());
    for minute in 1..=3 {
        std::fs::write(
            folder.join(format!("M31/lights/Light_{minute}.fits")),
            fits(&[
                ("IMAGETYP", "'LIGHT'"),
                ("OBJECT", "'M31'"),
                ("FILTER", "'L'"),
                ("EXPTIME", "120"),
                ("GAIN", "100"),
                ("DATE-OBS", &format!("'2026-10-07T22:0{minute}:00'")),
            ]),
        )
        .unwrap();
    }
    folder
}

fn png(width: u32, height: u32, filler: usize) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
    bytes.resize(bytes.len() + filler, 7);
    bytes
}

/// A closed port: connecting to it is refused at once.
fn closed_portal() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

fn assert_no_raw_english(shown: &str) {
    for raw in [
        "tcp connect",
        "error sending request",
        "Connection refused",
        "network error talking",
        "refused by the service",
        "http://127.0.0.1",
        "ReplacedByStacking",
        "OneRenderOnly",
        "NotInSubscription",
        "file_too_large",
    ] {
        assert!(
            !shown.contains(raw),
            "« {raw} » shown to the person: {shown}"
        );
    }
}

/// The service cannot be reached -- `send-night`, `check` and
/// `detect-sessions` say it in the person's language, with the sentence of the
/// dictionary, and neither a URL nor the HTTP library's English.
#[test]
fn a_service_that_cannot_be_reached_is_said_in_the_language_of_the_person() {
    let parent = temp("unreachable");
    let root = parent.join("Astro Sync");
    std::fs::create_dir_all(&root).unwrap();
    let folder = night(&root);
    let token = parent.join("token");
    std::fs::write(&token, "device-token").unwrap();
    let portal = closed_portal();
    for locale in ["fr", "ja", "en"] {
        let expected = dictionary(locale)["errors.network_error"]
            .as_str()
            .unwrap()
            .to_string();
        let send = run(&[
            "send-night",
            "--portal",
            &portal,
            "--token",
            &s(&token),
            "--folder",
            &s(&folder),
            "--target",
            "m31=M31",
            "--session-id",
            NIGHT_ID,
            "--queue",
            &s(&parent.join("queue.sqlite")),
            "--data",
            &s(&parent.join("data")),
            "--lang",
            locale,
        ]);
        assert!(!send.status.success());
        let shown = text(&send.stderr);
        assert!(shown.contains(&expected), "{locale}: {shown}");
        assert_no_raw_english(&shown);
        let detect = run(&[
            "detect-sessions",
            "--fits",
            &s(&folder),
            "--portal",
            &portal,
            "--token",
            &s(&token),
            "--data",
            &s(&parent.join("data")),
            "--output",
            &s(&parent.join("proposal.json")),
            "--lang",
            locale,
        ]);
        let shown = text(&detect.stderr);
        assert!(shown.contains(&expected), "{locale}: {shown}");
        assert_no_raw_english(&shown);
    }
    // The language of the computer decides when none is asked.
    let by_env = Command::new(env!("CARGO_BIN_EXE_astro-sync"))
        .env("ASTRO_SYNC_LANG", "ja")
        .args(["check", "--portal", &portal, "--token", &s(&token)])
        .args(["--folder", &s(&folder), "--target", "m31=M31"])
        .args(["--session-id", NIGHT_ID])
        .output()
        .unwrap();
    assert!(
        text(&by_env.stderr).contains(dictionary("ja")["errors.network_error"].as_str().unwrap()),
        "{}",
        text(&by_env.stderr)
    );
}

/// The service keeps no analysis report: the refusal is the
/// sentence of the dictionary, never "refused by the service (…): …".
#[test]
fn the_refusal_of_the_report_is_the_sentence_of_the_dictionary() {
    let parent = temp("logs-refused");
    let folder = parent.join("night");
    std::fs::create_dir_all(folder.join("Sh2-190/lights")).unwrap();
    std::fs::write(
        folder.join("Sh2-190/lights/Light_1.fits"),
        fits(&[
            ("IMAGETYP", "'LIGHT'"),
            ("OBJECT", "'Sh2-190'"),
            ("EXPTIME", "90"),
            ("GAIN", "100"),
            ("DATE-OBS", "'2026-08-09T21:00:00'"),
        ]),
    )
    .unwrap();
    let service = FakeService::start(Wake::Instant, move |request| {
        if request.method == "POST" && request.path.ends_with("/analyses") {
            return error_reply(
                403,
                "analysis_not_entitled",
                "This account cannot upload local-analysis reports.",
            );
        }
        error_reply(404, "not_found", "no")
    });
    let token = parent.join("token");
    std::fs::write(&token, "device-token").unwrap();
    for locale in ["fr", "de"] {
        let output = run(&[
            "send-logs",
            "--folder",
            &s(&folder),
            "--portal",
            &service.base_url(),
            "--token",
            &s(&token),
            "--session-id",
            NIGHT_ID,
            "--lang",
            locale,
        ]);
        assert!(!output.status.success());
        let shown = text(&output.stderr);
        let expected = dictionary(locale)["errors.analysis_not_entitled"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(shown.contains(&expected), "{locale}: {shown}");
        assert!(!shown.contains("This account cannot upload"), "{shown}");
        assert_no_raw_english(&shown);
    }
}

/// The stacking is local and never gated: a space without
/// `analysis.progression` (the free account) is not refused it; what stops it
/// here is the tools, said in the sentence of the window -- not "ToolMissing".
#[test]
fn a_space_without_the_progression_is_not_refused_the_stacking() {
    let parent = temp("stack-refused");
    let root = parent.join("Astro Sync");
    std::fs::create_dir_all(&root).unwrap();
    let folder = night(&root);
    let storage = fixture("space_storage.json").replace(
        "\"analysis.progression\": true",
        "\"analysis.progression\": false",
    );
    let service = FakeService::start(Wake::Instant, move |request| {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/api/v1/space/storage") => Reply::Json(200, storage.clone()),
            _ => error_reply(404, "not_found", "no"),
        }
    });
    let token = parent.join("token");
    std::fs::write(&token, "device-token").unwrap();
    for locale in ["fr", "ja"] {
        let output = run(&[
            "stack",
            "--night",
            &s(&folder),
            "--target-folder",
            "M31",
            "--target-id",
            "m31",
            "--evening-date",
            "2026-10-07",
            "--data",
            &s(&parent.join("data")),
            "--portal",
            &service.base_url(),
            "--token",
            &s(&token),
            "--lang",
            locale,
        ]);
        assert!(!output.status.success());
        let shown = text(&output.stderr);
        let refused = dictionary(locale)["errors.stacking.not_in_subscription"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(!shown.contains(&refused), "{locale}: {shown}");
        let expected = dictionary(locale)["errors.stacking.tool_missing"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(shown.contains(&expected), "{locale}: {shown}");
        assert_no_raw_english(&shown);
    }
}

/// A dry run asks the question the real stacking asks first: a space without
/// `analysis.progression` is never warned of its offer (the stacking is never
/// gated), only of what the real run would stop at, here the tools, and the
/// dry run itself still succeeds (nothing is run).
#[test]
fn a_dry_run_of_a_space_without_the_progression_names_the_tools_not_the_offer() {
    let parent = temp("stack-dry-refused");
    let root = parent.join("Astro Sync");
    std::fs::create_dir_all(&root).unwrap();
    let folder = night(&root);
    let storage = fixture("space_storage.json").replace(
        "\"analysis.progression\": true",
        "\"analysis.progression\": false",
    );
    let service = FakeService::start(Wake::Instant, move |request| {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/api/v1/space/storage") => Reply::Json(200, storage.clone()),
            _ => error_reply(404, "not_found", "no"),
        }
    });
    let token = parent.join("token");
    std::fs::write(&token, "device-token").unwrap();
    let args = |locale: &str, json: bool| -> Vec<String> {
        let mut args: Vec<String> = [
            "stack",
            "--night",
            &s(&folder),
            "--target-folder",
            "M31",
            "--target-id",
            "m31",
            "--evening-date",
            "2026-10-07",
            "--data",
            &s(&parent.join("data")),
            "--portal",
            &service.base_url(),
            "--token",
            &s(&token),
            "--dry-run",
            "--lang",
            locale,
        ]
        .iter()
        .map(ToString::to_string)
        .collect();
        if json {
            args.push("--json".to_string());
        }
        args
    };
    for locale in ["fr", "en"] {
        let expected = dictionary(locale)["errors.stacking.tool_missing"]
            .as_str()
            .unwrap()
            .to_string();
        let output = run(&args(locale, true)
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>());
        assert!(output.status.success(), "{}", text(&output.stderr));
        let plan: serde_json::Value = serde_json::from_str(&text(&output.stdout)).unwrap();
        assert_eq!(plan["refusal"]["code"], "tool_missing");
        assert_eq!(plan["refusal"]["message"], expected.as_str());

        let output = run(&args(locale, false)
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>());
        assert!(output.status.success(), "{}", text(&output.stderr));
        let shown = text(&output.stdout);
        assert!(shown.contains("would be refused"), "{shown}");
        assert!(shown.contains(&expected), "{shown}");
        assert!(shown.contains("Dry run: nothing was run."), "{shown}");
    }
}

/// Without an account (no `--portal`, no cache) the stacking is not gated
/// either: the dry run names only what the real run would stop at, the tools.
#[test]
fn a_dry_run_without_an_account_is_not_refused_for_the_offer() {
    let parent = temp("stack-dry-unknown");
    let root = parent.join("Astro Sync");
    std::fs::create_dir_all(&root).unwrap();
    let folder = night(&root);
    let output = run(&[
        "stack",
        "--night",
        &s(&folder),
        "--target-folder",
        "M31",
        "--target-id",
        "m31",
        "--evening-date",
        "2026-10-07",
        "--data",
        &s(&parent.join("data")),
        "--dry-run",
        "--json",
    ]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    let plan: serde_json::Value = serde_json::from_str(&text(&output.stdout)).unwrap();
    assert_eq!(plan["refusal"]["code"], "tool_missing");
}

/// The stacking script starts in its own work folder, so every path
/// it is given is absolute whatever the way the person wrote them.
#[test]
fn a_relative_night_folder_gives_the_script_absolute_paths() {
    let parent = temp("stack-relative");
    let root = parent.join("Astro Sync");
    std::fs::create_dir_all(&root).unwrap();
    let folder = night(&root);
    // `--night` written relative to the current folder of the command.
    let relative = folder.strip_prefix(&parent).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_astro-sync"))
        .current_dir(&parent)
        .args(["stack", "--night"])
        .arg(relative)
        .args(["--target-folder", "M31", "--target-id", "m31"])
        .args(["--evening-date", "2026-10-07", "--data", "data"])
        .args(["--dry-run", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", text(&output.stderr));
    let planned: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let command = planned["jobs"][0]["command"].as_array().unwrap();
    let paths: Vec<&str> = command
        .iter()
        .filter_map(|word| word.as_str())
        .filter(|word| !word.starts_with('-'))
        .filter(|word| word.contains("lights") || word.contains("stacked") || word.contains("work"))
        .collect();
    assert!(!paths.is_empty(), "{command:?}");
    for path in paths {
        assert!(
            Path::new(path).is_absolute(),
            "a relative path reaches Siril's own folder: {path}"
        );
    }
}

/// The command line takes the image of the device for the family it
/// covers, as the window does by default, and stacks the others.
#[test]
fn the_stacking_takes_the_image_of_the_device_for_the_family_it_covers() {
    let parent = temp("stack-device");
    let folder = parent.join("night");
    let seestar = |exposure: &str, second: u32| {
        fits(&[
            ("IMAGETYP", "'LIGHT'"),
            ("OBJECT", "'M45'"),
            ("FILTER", "'IRCUT'"),
            ("EXPTIME", exposure),
            ("GAIN", "200"),
            ("DATE-OBS", &format!("'2026-10-10T21:00:{second:02}'")),
        ])
    };
    std::fs::create_dir_all(folder.join("M45/lights")).unwrap();
    std::fs::create_dir_all(folder.join("M45/device")).unwrap();
    for second in 0..3 {
        std::fs::write(
            folder.join(format!("M45/lights/L30_{second}.fits")),
            seestar("30", second),
        )
        .unwrap();
    }
    for second in 10..12 {
        std::fs::write(
            folder.join(format!("M45/lights/L10_{second}.fits")),
            seestar("10", second),
        )
        .unwrap();
    }
    std::fs::write(
        folder.join("M45/device/Stacked_3_M45_30.0s_IRCUT.fit"),
        fits(&[
            ("SIMPLE", "T"),
            ("BITPIX", "16"),
            ("NAXIS", "3"),
            ("NAXIS1", "2160"),
            ("NAXIS2", "3840"),
            ("NAXIS3", "3"),
            ("BZERO", "32768"),
            ("CREATOR", "'ZWO Seestar S50 Pro'"),
            ("STACKCNT", "3"),
            ("EXPTIME", "30"),
            ("TOTALEXP", "90"),
            ("FILTER", "'IRCUT'"),
            ("GAIN", "200"),
            ("OBJECT", "'M45'"),
        ]),
    )
    .unwrap();
    let plan = |extra: &[&str]| {
        let output = run(&[
            "stack",
            "--night",
            &s(&folder),
            "--target-folder",
            "M45",
            "--target-id",
            "openngc-mel022",
            "--evening-date",
            "2026-10-10",
            "--data",
            &s(&parent.join("data")),
            "--dry-run",
            "--json",
        ]
        .into_iter()
        .chain(extra.iter().map(|word| &**word))
        .collect::<Vec<&str>>());
        assert!(output.status.success(), "{}", text(&output.stderr));
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    let by_default = plan(&[]);
    let jobs = by_default["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 2, "{by_default}");
    let from_device: Vec<&serde_json::Value> = jobs
        .iter()
        .filter(|planned| !planned["job"]["deviceMaster"].is_null())
        .collect();
    assert_eq!(
        from_device.len(),
        1,
        "the covered family takes the device image"
    );
    assert!(
        from_device[0]["job"]["deviceMaster"]
            .as_str()
            .unwrap()
            .ends_with("Stacked_3_M45_30.0s_IRCUT.fit")
    );
    assert_eq!(by_default["setAside"].as_array().unwrap().len(), 0);
    // `--restack` stacks the poses of every family instead.
    let restacked = plan(&["--restack"]);
    assert!(
        restacked["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|planned| planned["job"]["deviceMaster"].is_null()),
        "{restacked}"
    );
}

/// A stacked image heavier than the space accepts does not leave the
/// night with nothing to send: the person's final image goes, and the
/// summary says in words (with the numbers) why the stacked file stays.
#[test]
fn a_stacked_image_over_the_ceiling_leaves_the_final_image_to_go_and_says_why() {
    let parent = temp("check-ceiling");
    let root = parent.join("Astro Sync");
    std::fs::create_dir_all(&root).unwrap();
    let folder = night(&root);
    std::fs::write(folder.join("M31/final/M31_final.png"), png(300, 200, 1000)).unwrap();
    std::fs::create_dir_all(folder.join("M31/stacked")).unwrap();
    // 30 MB: over the 25 MB a file may weigh in this space.
    std::fs::write(
        folder.join("M31/stacked/M31_20261007_raw.tif"),
        vec![9_u8; 30_000_000],
    )
    .unwrap();
    let storage = fixture("space_storage.json")
        .replace("\"maxFileBytes\": 1073741824", "\"maxFileBytes\": 25000000");
    let online = fixture("night_bare.json");
    let service = FakeService::start(Wake::Instant, move |request| {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/api/v1/space/storage") => Reply::Json(200, storage.clone()),
            ("GET", path) if path.ends_with("/online") => Reply::Json(200, online.clone()),
            ("POST", "/api/v1/uploads/check") => Reply::Json(
                200,
                r#"{"result":"accepted","adds":"capture","storage":{"usedBytes":1,"quotaBytes":100000,"maxFileBytes":25000000,"captureCount":2,"captureCountLimit":30,"variantLimit":12}}"#.to_string(),
            ),
            _ => error_reply(404, "not_found", "no"),
        }
    });
    let token = parent.join("token");
    std::fs::write(&token, "device-token").unwrap();
    let args = |lang: &'static str, json: bool| {
        let mut args = vec![
            "check".to_string(),
            "--portal".into(),
            service.base_url(),
            "--token".into(),
            s(&token),
            "--folder".into(),
            s(&folder),
            "--target".into(),
            "m31=M31".into(),
            "--session-id".into(),
            NIGHT_ID.into(),
            "--lang".into(),
            lang.into(),
        ];
        if json {
            args.push("--json".into());
        }
        args
    };
    let as_words = |args: Vec<String>| {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        run(&refs)
    };
    let json = as_words(args("fr", true));
    assert!(json.status.success(), "{}", text(&json.stderr));
    let check: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(check["quota"]["allowed"], true, "{check}");
    let plans = check["plans"].as_array().unwrap();
    let decision_of = |name: &str| {
        plans
            .iter()
            .find(|plan| plan["image"]["path"].as_str().unwrap().ends_with(name))
            .unwrap_or_else(|| panic!("no plan for {name}: {check}"))
    };
    assert_eq!(decision_of("M31_final.png")["decision"], "new_capture");
    assert_eq!(decision_of("M31_20261007_raw.tif")["decision"], "not_sent");
    assert_eq!(
        decision_of("M31_20261007_raw.tif")["reason"],
        "too_large_for_space"
    );
    for locale in ["fr", "ja"] {
        let words = as_words(args(locale, false));
        let shown = text(&words.stdout);
        assert_no_raw_english(&shown);
        assert!(
            !shown.contains("TooLargeForSpace"),
            "the name of a variant is shown: {shown}"
        );
        let expected = dictionary(locale)["wizard.summary.setAsideTooLarge"]
            .as_str()
            .unwrap()
            .replace("{file}", "M31_20261007_raw.tif")
            .replace("{size}", if locale == "fr" { "30 Mo" } else { "30 MB" })
            .replace("{max}", if locale == "fr" { "25 Mo" } else { "25 MB" });
        assert!(shown.contains(&expected), "{locale}: {shown}");
    }
}

/// A limit that stops the sending names the file
/// and the numbers in the window's sentence, not the code `file_too_large`.
#[test]
fn a_limit_that_stops_the_sending_is_said_with_its_numbers() {
    let parent = temp("check-limit");
    let root = parent.join("Astro Sync");
    std::fs::create_dir_all(&root).unwrap();
    let folder = night(&root);
    std::fs::write(folder.join("M31/final/M31.png"), png(300, 200, 30_000_000)).unwrap();
    let storage = fixture("space_storage.json")
        .replace("\"maxFileBytes\": 1073741824", "\"maxFileBytes\": 25000000");
    let online = fixture("night_bare.json");
    let service = FakeService::start(Wake::Instant, move |request| {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/api/v1/space/storage") => Reply::Json(200, storage.clone()),
            ("GET", path) if path.ends_with("/online") => Reply::Json(200, online.clone()),
            ("POST", "/api/v1/uploads/check") => Reply::Json(
                200,
                r#"{"result":"accepted","adds":"capture","storage":{"usedBytes":1,"quotaBytes":100000,"maxFileBytes":25000000,"captureCount":2,"captureCountLimit":30,"variantLimit":12}}"#.to_string(),
            ),
            _ => error_reply(404, "not_found", "no"),
        }
    });
    let token = parent.join("token");
    std::fs::write(&token, "device-token").unwrap();
    let output = run(&[
        "check",
        "--portal",
        &service.base_url(),
        "--token",
        &s(&token),
        "--folder",
        &s(&folder),
        "--target",
        "m31=M31",
        "--session-id",
        NIGHT_ID,
    ]);
    let shown = text(&output.stdout);
    assert_no_raw_english(&shown);
    let expected = dictionary("en")["wizard.summary.refused.fileTooLarge"]
        .as_str()
        .unwrap()
        .replace("{file}", "M31.png")
        .replace("{size}", "30 MB")
        .replace("{max}", "25 MB");
    assert!(shown.contains(&expected), "{shown}");
    assert!(
        shown.contains(
            dictionary("en")["wizard.summary.impossible"]
                .as_str()
                .unwrap()
        ),
        "{shown}"
    );
}

/// Nothing leaves, and the summary says why and what to do instead
/// of "nothing exceeds" on zero file.
#[test]
fn a_night_with_nothing_to_send_says_why() {
    let parent = temp("check-nothing");
    let root = parent.join("Astro Sync");
    std::fs::create_dir_all(&root).unwrap();
    // Three lights, no final image, no stacked image.
    let folder = night(&root);
    let storage = fixture("space_storage.json");
    let online = fixture("night_bare.json");
    let service = FakeService::start(Wake::Instant, move |request| {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/api/v1/space/storage") => Reply::Json(200, storage.clone()),
            ("GET", path) if path.ends_with("/online") => Reply::Json(200, online.clone()),
            _ => error_reply(404, "not_found", "no"),
        }
    });
    let token = parent.join("token");
    std::fs::write(&token, "device-token").unwrap();
    for locale in ["fr", "en"] {
        let output = run(&[
            "check",
            "--portal",
            &service.base_url(),
            "--token",
            &s(&token),
            "--folder",
            &s(&folder),
            "--target",
            "m31=M31",
            "--session-id",
            NIGHT_ID,
            "--lang",
            locale,
        ]);
        let shown = text(&output.stdout);
        let words = dictionary(locale);
        let expected = words["wizard.summary.nothingToSend"].as_str().unwrap();
        assert!(shown.contains(expected), "{locale}: {shown}");
        assert!(
            !shown
                .split_whitespace()
                .any(|word| word.len() > 6 && word.chars().all(|c| c.is_ascii_digit())),
            "a raw number of bytes: {shown}"
        );
    }
}

/// A service that is asleep is announced on the standard error by
/// the commands of today's journey too, after 4 s of silence, then said to have
/// answered; the standard output stays the result.
#[test]
fn a_sleeping_service_is_announced_by_check_and_detect_sessions() {
    let parent = temp("check-wake");
    let root = parent.join("Astro Sync");
    std::fs::create_dir_all(&root).unwrap();
    let folder = night(&root);
    let storage = fixture("space_storage.json");
    let online = fixture("night_bare.json");
    let service = FakeService::start(Wake::Held(Duration::from_secs(5)), move |request| {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/api/v1/space/storage") => Reply::Json(200, storage.clone()),
            ("GET", path) if path.ends_with("/online") => Reply::Json(200, online.clone()),
            _ => error_reply(404, "not_found", "no"),
        }
    });
    let token = parent.join("token");
    std::fs::write(&token, "device-token").unwrap();
    let output = run(&[
        "check",
        "--portal",
        &service.base_url(),
        "--token",
        &s(&token),
        "--folder",
        &s(&folder),
        "--target",
        "m31=M31",
        "--session-id",
        NIGHT_ID,
        "--json",
    ]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    let lines: Vec<String> = text(&output.stderr).lines().map(str::to_string).collect();
    assert_eq!(
        lines,
        [
            "The service is starting. It may take a minute.",
            "The service answered. Sending resumes."
        ],
        "{lines:?}"
    );
    serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .expect("standard output stays one JSON document");
}

/// The sentence after an image replaced another does not say it was
/// the phone's photo (the replaced image may be Astro Sync's own).
#[test]
fn a_replaced_image_is_said_without_inventing_the_phone() {
    let parent = temp("replaced");
    let folder = parent.join("fold");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("m45.jpg"), vec![5_u8; 250]).unwrap();
    let model = Arc::new(UploadModel::default());
    let service = FakeService::start(Wake::Instant, move |request| {
        let reply = model
            .reply(request, 100)
            .unwrap_or_else(|| error_reply(404, "not_found", "no such route"));
        match reply {
            Reply::Json(status, body) if request.path.ends_with("/finalize") => {
                let mut value: serde_json::Value = serde_json::from_str(&body).unwrap();
                value["replaced"] = serde_json::json!(true);
                Reply::Json(status, value.to_string())
            }
            other => other,
        }
    });
    let token = parent.join("token");
    std::fs::write(&token, "device-token").unwrap();
    let output = run(&[
        "upload",
        "--queue",
        &s(&parent.join("queue.sqlite")),
        "--portal",
        &service.base_url(),
        "--token",
        &s(&token),
        "--folder",
        &s(&folder),
        "--lang",
        "fr",
    ]);
    let shown = text(&output.stdout);
    assert!(output.status.success(), "{shown}{}", text(&output.stderr));
    let words = dictionary("fr");
    let sent = words["queue.completedNotice"].as_str().unwrap().to_string();
    let after_file = sent.split("{file}").last().unwrap();
    let marker = after_file.split("{captureId}").next().unwrap();
    assert!(shown.contains(marker), "{shown}");
    let expected = words["queue.replacedPhoto"].as_str().unwrap();
    assert!(shown.contains(expected), "{shown}");
    assert!(!shown.contains("téléphone"), "{shown}");
}

/// `upload`, `queue status`, `scan` and `tools` say what they print in the
/// language asked for -- `--lang` or `ASTRO_SYNC_LANG` -- with the sentences of
/// the dictionaries: the failure, the stop, the states, the columns, the names
/// of the tools and their badges. Never the French names beside the English
/// states of a tool, never "absente" in an English line.
#[test]
fn upload_queue_scan_and_tools_speak_the_language_that_was_asked_for() {
    let parent = temp("languages");
    let files = parent.join("files");
    std::fs::create_dir_all(&files).unwrap();
    std::fs::write(files.join("a.png"), png(300, 200, 200)).unwrap();
    let token = parent.join("token");
    std::fs::write(&token, "device-token").unwrap();
    let portal = closed_portal();
    let root = parent.join("Astro Sync");
    std::fs::create_dir_all(&root).unwrap();
    let night = night(&root);

    for locale in [
        "en", "fr", "ja", "de", "es", "it", "pt-BR", "ru", "zh-Hans", "zh-Hant",
    ] {
        let words = dictionary(locale);
        let said = |key: &str| words[key].as_str().unwrap().to_string();
        let queue = parent.join(format!("queue-{locale}.sqlite"));

        // `--lang` on the command.
        let upload = run(&[
            "upload",
            "--queue",
            &s(&queue),
            "--portal",
            &portal,
            "--token",
            &s(&token),
            "--folder",
            &s(&files),
            "--lang",
            locale,
        ]);
        let shown = format!("{}{}", text(&upload.stdout), text(&upload.stderr));
        assert!(
            shown.contains(&said("errors.network_error")),
            "{locale}: {shown}"
        );
        assert!(
            shown.contains(&said("queue.stoppedNoProgress")),
            "{locale}: {shown}"
        );
        if locale != "en" {
            for english in ["Could not reach", "Stopped:", "Queue updated", "Sending "] {
                assert!(!shown.contains(english), "{locale}: {shown}");
            }
        }

        // `queue status` takes `--lang` as well.
        let status = run(&["queue", "status", "--queue", &s(&queue), "--lang", locale]);
        assert!(
            status.status.success(),
            "{locale}: {}",
            text(&status.stderr)
        );
        let table = text(&status.stdout);
        assert!(table.contains(&said("cli.queue.file")), "{locale}: {table}");
        assert!(
            table.contains(&said("cli.queue.state.queued")),
            "{locale}: {table}"
        );

        // `ASTRO_SYNC_LANG` where there is no `--lang`.
        let by_env = Command::new(env!("CARGO_BIN_EXE_astro-sync"))
            .env("ASTRO_SYNC_LANG", locale)
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .args(["queue", "status", "--queue", &s(&queue)])
            .output()
            .unwrap();
        assert_eq!(text(&by_env.stdout), table, "{locale}");

        // The walk of a folder, then the rules of a night folder.
        let scan = run(&["scan", "--folder", &s(&files), "--lang", locale]);
        let walked = text(&scan.stdout);
        assert!(
            walked.contains(&said("cli.scan.path")),
            "{locale}: {walked}"
        );
        assert!(
            walked.contains(&said("cli.class.image")),
            "{locale}: {walked}"
        );
        let detect = run(&[
            "scan",
            "--detect",
            "--folder",
            &s(&night),
            "--target",
            "m31=M31",
            "--lang",
            locale,
        ]);
        let found = text(&detect.stdout);
        let line = words["cli.scan.targetLineMissing"]
            .as_str()
            .unwrap()
            .replace("{target}", "M31");
        let head = line.split("{frames}").next().unwrap();
        assert!(found.contains(head), "{locale}: {found}");
        assert!(!found.contains("absente"), "{locale}: {found}");
        assert!(
            found.contains(&said("wizard.files.finalMissing")),
            "{locale}: {found}"
        );

        // The tools: one language for the names and the states.
        let data = parent.join("data");
        std::fs::create_dir_all(&data).unwrap();
        let tools = run(&["tools", "--data", &s(&data), "--lang", locale]);
        assert!(tools.status.success(), "{locale}: {}", text(&tools.stderr));
        let list = text(&tools.stdout);
        let missing = said("deps.badge.missing");
        let not_installed = said("deps.badge.notInstalled");
        assert!(
            list.contains(&missing) || list.contains(&not_installed),
            "{locale}: {list}"
        );
        if locale != "fr" {
            assert!(!list.contains("Environnement Python"), "{locale}: {list}");
        }
        if locale != "en" {
            for english in ["Missing", "Present"] {
                assert!(
                    !list.split_whitespace().any(|word| word == english),
                    "{locale}: {list}"
                );
            }
        }
    }
    let list = text(&run(&["tools", "--data", &s(&parent.join("data")), "--lang", "fr"]).stdout);
    assert!(list.contains("Environnement Python"), "{list}");
}

/// A server that keeps no quota (a private server) has no space left to count:
/// `check` says "no limit", not a figure of millions of gigabytes. And a target
/// the space does not know is refused with a sentence that names the command
/// that lists the identifiers, not with its bare code.
#[test]
fn a_server_without_a_quota_says_no_limit_and_an_unknown_target_names_the_command() {
    let parent = temp("check-unlimited");
    let root = parent.join("Astro Sync");
    std::fs::create_dir_all(&root).unwrap();
    let folder = night(&root);
    std::fs::write(folder.join("M31/final/M31.png"), png(300, 200, 2_000)).unwrap();
    let unlimited = "9007199254740991";
    let storage = fixture("space_storage.json")
        .replace("268435456000", unlimited)
        .replace("268378456000", unlimited);
    let online = fixture("night_bare.json");
    let token = parent.join("token");
    std::fs::write(&token, "device-token").unwrap();
    let serve = |check: String| {
        let (storage, online) = (storage.clone(), online.clone());
        FakeService::start(Wake::Instant, move |request| {
            match (request.method.as_str(), request.path.as_str()) {
                ("GET", "/api/v1/space/storage") => Reply::Json(200, storage.clone()),
                ("GET", path) if path.ends_with("/online") => Reply::Json(200, online.clone()),
                ("POST", "/api/v1/uploads/check") => Reply::Json(200, check.clone()),
                _ => error_reply(404, "not_found", "no"),
            }
        })
    };
    let check_with = |service: &FakeService, locale: &str| {
        let output = run(&[
            "check",
            "--portal",
            &service.base_url(),
            "--token",
            &s(&token),
            "--folder",
            &s(&folder),
            "--target",
            "m31=M31",
            "--session-id",
            NIGHT_ID,
            "--lang",
            locale,
        ]);
        text(&output.stdout)
    };
    let storage_json = format!(
        r#""storage":{{"usedBytes":1,"quotaBytes":{unlimited},"maxFileBytes":1073741824,"captureCount":2,"captureCountLimit":{unlimited},"variantLimit":12}}"#
    );

    let accepting = serve(format!(
        r#"{{"result":"accepted","adds":"capture",{storage_json}}}"#
    ));
    for locale in ["en", "fr", "ja"] {
        let shown = check_with(&accepting, locale);
        let line = dictionary(locale)["wizard.summary.check.placeUnlimited"]
            .as_str()
            .unwrap()
            .split("{toSend}")
            .last()
            .unwrap()
            .to_string();
        assert!(shown.contains(&line), "{locale}: {shown}");
        assert!(!shown.contains("9007199"), "{locale}: {shown}");
    }

    let refusing = serve(format!(
        r#"{{"result":"refused","code":"target_not_found","message":"No such target.",{storage_json}}}"#
    ));
    for locale in ["en", "fr", "ja"] {
        let shown = check_with(&refusing, locale);
        let words = dictionary(locale);
        let expected = words["cli.targetNotFound"].as_str().unwrap();
        assert!(shown.contains(expected), "{locale}: {shown}");
        assert!(shown.contains("astro-sync targets"), "{locale}: {shown}");
        assert!(!shown.contains("target_not_found"), "{locale}: {shown}");
    }
}

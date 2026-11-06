//! The "Targets" step on the command line, the real binary against
//! a fake service: the search sends the normalised text, an addition keeps
//! the night's targets, a capture's header gives its target (the reference
//! `golden/fits_target.json`), a new target is checked before anything
//! leaves. Invented catalogue rows and headers only.

#[path = "../../astro-sync-core/tests/support/fake_service.rs"]
mod fake_service;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use fake_service::{FakeService, Reply, Wake};

fn temp(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "astro-sync-cli-targets-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(args: &[&str]) -> (bool, serde_json::Value, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_astro-sync"))
        .args(args)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let json = serde_json::from_str(&stdout).unwrap_or(serde_json::Value::Null);
    (
        output.status.success(),
        json,
        format!("{stdout}{}", String::from_utf8_lossy(&output.stderr)),
    )
}

/// A synthetic FITS primary header, no pixel.
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

const INDEX: &str = r#"{"v":1,"revision":"rev-1","total":2,"offset":0,"limit":8,"sort":"planner",
"types":[["Nébuleuse en émission","Nébuleuse en émission"]],"constellations":[["Cyg","Cygne"]],
"fields":["id","designation","name","aliases","ra","dec","sizeArcmin","magnitude","type","constellation","sizeLabel","photoUrl"],
"rows":[["t-ngc7000","NGC 7000","Nébuleuse de l'Amérique du Nord",[],20.9784,44.33,[120,100],4.0,0,0,"120 × 100′","/img/ngc7000.jpg"],
["t-ic5070","IC 5070","Nébuleuse du Pélican",[],20.8508,44.35,[60,50],8.0,0,0,"60 × 50′",null]]}"#;

struct Service {
    service: FakeService,
    night: Arc<Mutex<Vec<String>>>,
}

fn service() -> Service {
    let night = Arc::new(Mutex::new(vec!["t-ic1396".to_string()]));
    let held = Arc::clone(&night);
    let service =
        FakeService::start(Wake::Instant, move |request| {
            let path = request.path.split('?').next().unwrap_or("");
            match (request.method.as_str(), path) {
            ("GET", "/api/v1/target-index") => Reply::Json(200, INDEX.to_string()),
            ("GET", "/api/v1/sessions/night-7") => Reply::Json(
                200,
                serde_json::json!({ "night": { "targetIds": { "value": *held.lock().unwrap() } } })
                    .to_string(),
            ),
            ("PATCH", "/api/v1/sessions/night-7/night") => {
                let ids: Vec<String> =
                    serde_json::from_value(request.json()["targetIds"].clone()).unwrap();
                held.lock().unwrap().clone_from(&ids);
                Reply::Json(
                    200,
                    serde_json::json!({ "night": { "targetIds": { "value": ids } } }).to_string(),
                )
            }
            ("POST", "/api/v1/targets/personal") => Reply::Json(
                201,
                serde_json::json!({ "target": { "id": "p-1", "name": request.json()["name"] } })
                    .to_string(),
            ),
            _ => Reply::Json(404, r#"{"error":{"code":"not_found","message":"no"}}"#.to_string()),
        }
        });
    Service { service, night }
}

fn token(dir: &Path) -> String {
    let path = dir.join("token");
    std::fs::write(&path, "device-token\n").unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn targets_search_sends_the_normalised_text_and_prints_the_core_s_value() {
    let fake = service();
    let dir = temp("search");
    let base = fake.service.base_url();
    let token = token(&dir);
    let (ok, json, out) = run(&[
        "targets", "--portal", &base, "--token", &token, "--search", "ngc7000", "--json",
    ]);
    assert!(ok, "{out}");
    assert_eq!(json["query"], "NGC 7000");
    assert_eq!(json["total"], 2);
    assert_eq!(json["targets"][0]["designation"], "NGC 7000");
    assert_eq!(json["targets"][0]["typeLabel"], "Nébuleuse en émission");
    let asked = &fake.service.requests()[0].path;
    assert!(
        asked.contains("q=NGC+7000") || asked.contains("q=NGC%207000"),
        "{asked}"
    );
    assert!(asked.contains("sort=planner"), "{asked}");
    let (ok, _, out) = run(&[
        "targets", "--portal", &base, "--token", &token, "--search", "ngc",
    ]);
    assert!(ok, "{out}");
    assert!(out.contains("2 result(s) of 2"), "{out}");
}

#[test]
fn night_targets_add_keeps_the_others_and_its_dry_run_sends_nothing() {
    let fake = service();
    let dir = temp("night");
    let base = fake.service.base_url();
    let token = token(&dir);
    let args = |extra: &[&'static str]| {
        let mut all = vec![
            "night-targets",
            "--portal",
            &base,
            "--token",
            &token,
            "--session-id",
            "night-7",
            "--add",
            "t-ngc7000",
            "--json",
        ];
        all.extend_from_slice(extra);
        run(&all.iter().map(|s| &**s).collect::<Vec<_>>())
    };
    let (ok, json, out) = args(&["--dry-run"]);
    assert!(ok, "{out}");
    assert_eq!(json, serde_json::json!(["t-ic1396", "t-ngc7000"]));
    assert_eq!(fake.service.app_hits("PATCH", "/api/v1/sessions"), 0);
    let (ok, json, out) = args(&[]);
    assert!(ok, "{out}");
    assert_eq!(json, serde_json::json!(["t-ic1396", "t-ngc7000"]));
    assert_eq!(*fake.night.lock().unwrap(), ["t-ic1396", "t-ngc7000"]);
}

#[test]
fn fits_target_reads_the_header_and_matches_the_reference() {
    let fake = service();
    let dir = temp("fits");
    let base = fake.service.base_url();
    let token = token(&dir);
    let file = dir.join("Lights_0007.fit");
    std::fs::write(
        &file,
        fits(&[
            ("IMAGETYP", "'LIGHT'"),
            ("OBJCTRA", "'20 56 00'"),
            ("OBJCTDEC", "'+44 20 00'"),
            ("XPIXSZ", "3.76"),
            ("FOCALLEN", "400."),
            ("NAXIS1", "3880"),
            ("NAXIS2", "2954"),
        ]),
    )
    .unwrap();
    let data = dir.join("data");
    let (ok, json, out) = run(&[
        "fits-target",
        "--file",
        &file.to_string_lossy(),
        "--portal",
        &base,
        "--token",
        &token,
        "--data",
        &data.to_string_lossy(),
        "--json",
    ]);
    assert!(ok, "{out}");
    let reference: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/fits_target.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(json, reference);
    assert!(data.join("catalogue-index.json").exists());
    // Nothing of the file left: only the catalogue was asked.
    assert!(
        fake.service
            .requests()
            .iter()
            .all(|r| r.method == "GET" && r.path.starts_with("/api/v1/target-index"))
    );

    // Without the service: the header alone.
    let (ok, json, out) = run(&["fits-target", "--file", &file.to_string_lossy(), "--json"]);
    assert!(ok, "{out}");
    assert_eq!(json["positionSource"], "objctra");
    assert_eq!(json["object"], serde_json::Value::Null);
}

#[test]
fn personal_target_is_checked_before_anything_leaves_then_created_and_added() {
    let fake = service();
    let dir = temp("personal");
    let base = fake.service.base_url();
    let token = token(&dir);
    let (ok, _, out) = run(&[
        "personal-target",
        "--portal",
        &base,
        "--token",
        &token,
        "--name",
        "Comète C/2026 T1",
        "--label",
        "C/2026 T1",
        "--dry-run",
    ]);
    assert!(!ok);
    assert!(out.contains("seven"), "{out}");
    let (ok, json, out) = run(&[
        "personal-target",
        "--portal",
        &base,
        "--token",
        &token,
        "--name",
        "Comète C/2026 T1",
        "--label",
        "Grand champ",
        "--session-id",
        "night-7",
        "--json",
    ]);
    assert!(ok, "{out}");
    assert_eq!(json["target"]["id"], "p-1");
    assert_eq!(
        json["nightTargetIds"],
        serde_json::json!(["t-ic1396", "p-1"])
    );
    assert_eq!(fake.service.app_hits("POST", "/api/v1/targets/personal"), 1);
}

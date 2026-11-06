//! The multi-night plans on the command line: `astro-sync integrations` and
//! `astro-sync plan-image`, the real binary against a fake service. Invented
//! nights only.

#[path = "../../astro-sync-core/tests/support/fake_service.rs"]
mod fake_service;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use fake_service::{FakeService, Reply, Wake};

fn temp(label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "astro-sync-cli-multi-night-{label}-{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
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

fn s(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// A FITS header of 2880 bytes and a little data.
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
    out.resize(out.len().div_ceil(2880) * 2880 + 2880, b' ');
    out
}

/// A night folder of the working folder with IC 1805's master kept.
fn night(root: &Path, name: &str, night_id: &str, family: &str, filter: &str) -> PathBuf {
    let night = root.join(name);
    let stacked = night.join("IC 1805/stacked");
    std::fs::create_dir_all(&stacked).unwrap();
    std::fs::write(
        night.join(".astro-sync-night.json"),
        format!(
            r#"{{"format":1,"nightId":"{night_id}","serviceUrl":"http://x","createdAt":"2026-08-10T10:00:00Z"}}"#
        ),
    )
    .unwrap();
    std::fs::write(
        night.join("IC 1805/.astro-sync-target.json"),
        r#"{"format":1,"targetId":"ic-1805"}"#,
    )
    .unwrap();
    std::fs::write(
        stacked.join(format!("{family}_integration.fit")),
        fits(&[
            ("NAXIS", "3"),
            ("ASFAMILY", &format!("'{family}'")),
            ("FILTER", &format!("'{filter}'")),
            ("EXPTIME", "120.0"),
            ("GAIN", "100"),
            ("XBINNING", "1"),
            ("STACKCNT", "120"),
        ]),
    )
    .unwrap();
    night
}

const PROJECT: &str = r#"{"id":"p-heart","revision":1,"status":"active","target":{"targetId":"ic-1805","name":"IC 1805"},"integrations":[{"id":"i-9","projectId":"p-heart","nightId":"n-09","targetId":"ic-1805","filter":"L-eXtreme","exposureSeconds":180,"gain":100,"binning":1,"temperatureC":-10,"frameCount":98,"totalSeconds":17640,"eveningDate":"2026-08-09","sensor":null,"computerLabel":"Portable","installationId":null,"createdAt":"2026-08-10T10:00:00Z","updatedAt":"2026-08-10T10:00:00Z"}],"planCapture":null}"#;

#[test]
fn the_integrations_of_a_night_are_lines_and_a_private_server_receives_them() {
    let storage = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../astro-sync-core/tests/fixtures/online/space_storage.json"),
    )
    .unwrap();
    let hosted_storage = storage.replace(
        r#""analysis.progression": true"#,
        r#""analysis.progression": false"#,
    );
    assert_ne!(hosted_storage, storage, "the fixture names the capability");
    // A private server: it says so itself — it pairs —,
    // and never restricts the multi-night plans.
    let service = FakeService::start(Wake::Instant, move |request| {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/api/v1/service") => Reply::Json(
                200,
                r#"{"service":"astro-portal","apiVersion":1,"version":"1.0.0","signIn":{"method":"pairing"}}"#
                    .to_string(),
            ),
            ("GET", "/api/v1/projects/p-heart") => Reply::Json(200, PROJECT.to_string()),
            ("GET", "/api/v1/space/storage") => Reply::Json(200, storage.clone()),
            ("PUT", "/api/v1/projects/p-heart/integrations") => Reply::Json(
                200,
                r#"{"projectId":"p-heart","integrations":[{"id":"i-12","projectId":"p-heart","nightId":"n-12","targetId":"ic-1805","filter":"UV/IR-cut","exposureSeconds":120,"gain":100,"binning":1,"temperatureC":-10,"frameCount":120,"totalSeconds":14400,"eveningDate":"2026-08-12","sensor":null,"computerLabel":null,"installationId":null,"createdAt":"2026-08-13T10:00:00Z","updatedAt":"2026-08-13T10:00:00Z"}]}"#
                    .to_string(),
            ),
            _ => Reply::Json(
                404,
                r#"{"error":{"code":"not_found","message":"no"}}"#.to_string(),
            ),
        }
    });
    let parent = temp("integrations");
    let root = parent.join("Astro Sync");
    let twelfth = night(
        &root,
        "2026-08-12 · Cœur · n-12",
        "n-12",
        "UV_IR-cut_120s_g100",
        "UV/IR-cut",
    );
    let token = parent.join("token");
    std::fs::write(&token, "device-token").unwrap();
    let portal = service.base_url();

    let (ok, dry, all) = run(&[
        "integrations",
        "--night",
        &s(&twelfth),
        "--project",
        "p-heart",
        "--computer-label",
        "iMac du bureau",
        "--portal",
        &portal,
        "--token",
        &s(&token),
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{all}");
    let line = &dry["lines"][0];
    assert_eq!(line["nightId"], "n-12");
    assert_eq!(line["eveningDate"], "2026-08-12");
    assert_eq!(line["filter"], "UV/IR-cut");
    assert_eq!(line["frameCount"], 120);
    assert_eq!(line["computerLabel"], "iMac du bureau");
    assert!(dry["sent"].is_null());
    assert_eq!(service.app_hits("PUT", "/api/v1/projects"), 0);

    // A private server receives them, whatever its capabilities say.
    let (ok, sent, all) = run(&[
        "integrations",
        "--night",
        &s(&twelfth),
        "--project",
        "p-heart",
        "--portal",
        &portal,
        "--token",
        &s(&token),
        "--json",
    ]);
    assert!(ok, "{all}");
    assert_eq!(sent["sent"]["result"], "sent", "{all}");
    assert_eq!(service.app_hits("PUT", "/api/v1/projects"), 1);

    // A service that sells them (it creates a space for any address),
    // without "Advanced Sync+": closed, nothing asked, said in
    // French.
    let hosted = FakeService::start(Wake::Instant, move |request| {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/api/v1/service") => Reply::Json(
                200,
                r#"{"service":"astro-portal","apiVersion":1,"version":"1.0.0","signIn":{"method":"email-code","spaceCreation":"open"}}"#
                    .to_string(),
            ),
            ("GET", "/api/v1/projects/p-heart") => Reply::Json(200, PROJECT.to_string()),
            ("GET", "/api/v1/space/storage") => Reply::Json(200, hosted_storage.clone()),
            _ => Reply::Json(
                404,
                r#"{"error":{"code":"not_found","message":"no"}}"#.to_string(),
            ),
        }
    });
    let hosted_portal = hosted.base_url();
    let (ok, sent, all) = run(&[
        "integrations",
        "--night",
        &s(&twelfth),
        "--project",
        "p-heart",
        "--portal",
        &hosted_portal,
        "--token",
        &s(&token),
        "--json",
    ]);
    assert!(ok, "{all}");
    assert_eq!(sent["sent"]["result"], "closed");
    assert_eq!(sent["sent"]["closed"], "not_in_subscription");
    assert_eq!(hosted.app_hits("PUT", "/api/v1/projects"), 0);
    let (ok, _, words) = run(&[
        "integrations",
        "--night",
        &s(&twelfth),
        "--project",
        "p-heart",
        "--portal",
        &hosted_portal,
        "--token",
        &s(&token),
    ]);
    assert!(ok, "{words}");
    assert!(
        words.contains("multi-night plan closed (outside the subscription"),
        "{words}"
    );
    assert!(!words.contains("NotInSubscription"), "{words}");

    // The image of the plan: the 9th is on the laptop, only this night is
    // here; the plan's folder is not made in a dry run.
    let data = parent.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let (ok, image, all) = run(&[
        "plan-image",
        "--project",
        "p-heart",
        "--root",
        &s(&root),
        "--night",
        &s(&twelfth),
        "--data",
        &s(&data),
        "--portal",
        &portal,
        "--token",
        &s(&token),
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{all}");
    let places: Vec<&str> = image["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["place"].as_str().unwrap())
        .collect();
    assert_eq!(places, vec!["otherComputer", "thisNight"]);
    assert_eq!(image["plans"][0]["kind"], "oneNight");
    assert!(!root.join("IC 1805 · p-hear").exists());

    // A second night here: a job, aligned on the longest.
    night(
        &root,
        "2026-08-10 · Cœur · n-10",
        "n-10",
        "RGB_120s_g100",
        "RGB",
    );
    let project_with_tenth = PROJECT.replace(
        r#""integrations":["#,
        r#""integrations":[{"id":"i-10","projectId":"p-heart","nightId":"n-10","targetId":"ic-1805","filter":"RGB","exposureSeconds":120,"gain":100,"binning":1,"temperatureC":null,"frameCount":120,"totalSeconds":14400,"eveningDate":"2026-08-10","sensor":null,"computerLabel":null,"installationId":null,"createdAt":"2026-08-10T10:00:00Z","updatedAt":"2026-08-10T10:00:00Z"},"#,
    );
    let storage_second = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../astro-sync-core/tests/fixtures/online/space_storage.json"),
    )
    .unwrap();
    let second = FakeService::start(Wake::Instant, move |request| {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/api/v1/service") => Reply::Json(
                200,
                r#"{"service":"astro-portal","apiVersion":1,"version":"1.0.0","signIn":{"method":"pairing"}}"#
                    .to_string(),
            ),
            ("GET", "/api/v1/projects/p-heart") => Reply::Json(200, project_with_tenth.clone()),
            ("GET", "/api/v1/space/storage") => Reply::Json(200, storage_second.clone()),
            _ => Reply::Json(
                404,
                r#"{"error":{"code":"not_found","message":"no"}}"#.to_string(),
            ),
        }
    });
    let (ok, image, all) = run(&[
        "plan-image",
        "--project",
        "p-heart",
        "--root",
        &s(&root),
        "--night",
        &s(&twelfth),
        "--data",
        &s(&data),
        "--portal",
        &second.base_url(),
        "--token",
        &s(&token),
        "--no-render",
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{all}");
    assert_eq!(image["plans"][0]["kind"], "job");
    let masters = image["plans"][0]["masters"].as_array().unwrap();
    assert_eq!(masters.len(), 2);
    assert_eq!(image["plans"][0]["pipeline"]["pipelineId"], "linear");
    // The window's default, `presentation`, is the command's too;
    // `--contrast none` writes nothing more than the image.
    assert_eq!(image["plans"][0]["contrast"], "presentation");

    // The progression of the plan: its image after each night, the poses so
    // far; a dry run makes none (no JPEG yet) and queues nothing.
    let (ok, progression, all) = run(&[
        "plan-image",
        "--project",
        "p-heart",
        "--root",
        &s(&root),
        "--night",
        &s(&twelfth),
        "--data",
        &s(&data),
        "--portal",
        &second.base_url(),
        "--token",
        &s(&token),
        "--progression",
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{all}");
    let steps: Vec<(u64, u64, &str)> = progression["progression"]
        .as_array()
        .unwrap()
        .iter()
        .map(|step| {
            (
                step["nights"].as_u64().unwrap(),
                step["frames"].as_u64().unwrap(),
                step["eveningDate"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(steps.len(), 2, "one milestone per night: {all}");
    assert_eq!((steps[0].0, steps[1].0), (1, 2));
    assert!(
        steps[0].1 < steps[1].1,
        "the poses grow from night to night"
    );
    assert!(
        steps[0].2 < steps[1].2,
        "the nights come in the order of their evenings"
    );
    assert_eq!(progression["progressionEnqueued"], false);
    let (ok, without, all) = run(&[
        "plan-image",
        "--project",
        "p-heart",
        "--root",
        &s(&root),
        "--night",
        &s(&twelfth),
        "--data",
        &s(&data),
        "--portal",
        &second.base_url(),
        "--token",
        &s(&token),
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{all}");
    assert_eq!(without["progression"].as_array().unwrap().len(), 0);
    // The image of the plan takes the pipelines of its target, one job
    // each: the target's defaults (no dual-band integration here: "Natural
    // render" alone), or the ones named, "No processing" after them.
    let (ok, by_default, all) = run(&[
        "plan-image",
        "--project",
        "p-heart",
        "--root",
        &s(&root),
        "--night",
        &s(&twelfth),
        "--data",
        &s(&data),
        "--portal",
        &second.base_url(),
        "--token",
        &s(&token),
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{all}");
    let ids = |answer: &serde_json::Value| -> Vec<String> {
        answer["plans"]
            .as_array()
            .unwrap()
            .iter()
            .map(|plan| plan["pipeline"]["pipelineId"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(ids(&by_default), vec!["raw"]);
    let (ok, several, all) = run(&[
        "plan-image",
        "--project",
        "p-heart",
        "--root",
        &s(&root),
        "--night",
        &s(&twelfth),
        "--data",
        &s(&data),
        "--portal",
        &second.base_url(),
        "--token",
        &s(&token),
        "--pipeline",
        "hoo",
        "--pipeline",
        "raw",
        "--no-render",
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{all}");
    assert_eq!(ids(&several), vec!["hoo", "raw", "linear"]);
    let (ok, targeted, all) = run(&[
        "plan-image",
        "--project",
        "p-heart",
        "--root",
        &s(&root),
        "--night",
        &s(&twelfth),
        "--data",
        &s(&data),
        "--portal",
        &second.base_url(),
        "--token",
        &s(&token),
        "--pipeline",
        "raw",
        "--target-pipeline",
        "ic-1805=polished",
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{all}");
    assert_eq!(ids(&targeted), vec!["polished"]);
    // --contrast is a word of the job `plan_stack.py` reads.
    let (ok, curved, all) = run(&[
        "plan-image",
        "--project",
        "p-heart",
        "--root",
        &s(&root),
        "--night",
        &s(&twelfth),
        "--data",
        &s(&data),
        "--portal",
        &second.base_url(),
        "--token",
        &s(&token),
        "--contrast",
        "none",
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{all}");
    assert!(
        curved["plans"][0].get("contrast").is_none(),
        "none writes nothing more"
    );
    assert!(
        image["plans"][0]["work"]
            .as_str()
            .unwrap()
            .ends_with(".astro-sync-work/plan-image")
    );

    // C5: every path of the job is absolute, whatever the way `--data` was
    // given (here, relative to the folder the command starts in).
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_astro-sync"))
        .current_dir(&parent)
        .args([
            "plan-image",
            "--project",
            "p-heart",
            "--root",
            &s(&root),
            "--night",
            &s(&twelfth),
            "--data",
            "data",
            "--portal",
            &second.base_url(),
            "--token",
            &s(&token),
            "--no-render",
            "--dry-run",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let relative: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    for dir in relative["plans"][0]["pipelinesDirs"].as_array().unwrap() {
        let dir = Path::new(dir.as_str().unwrap());
        assert!(dir.is_absolute(), "{dir:?}");
        assert!(dir.ends_with("data/pipelines"), "{dir:?}");
    }

    // On a private server the image of the plan is never refused
    // for a right — here it stops only because the stacking is not
    // installed.
    let (ok, _, all) = run(&[
        "plan-image",
        "--project",
        "p-heart",
        "--root",
        &s(&root),
        "--night",
        &s(&twelfth),
        "--data",
        &s(&data),
        "--portal",
        &second.base_url(),
        "--token",
        &s(&token),
        "--no-render",
    ]);
    assert!(!all.contains("private server"), "{all}");
    assert!(!all.contains("outside the subscription"), "{all}");
    assert!(!all.contains("space rights"), "{all}");
    let _ = ok;

    // C1: on a service that sells them, without the right, it is refused in
    // a sentence that says why, never with the name of a variant of the code.
    let two_nights = PROJECT.replace(
        r#""integrations":["#,
        r#""integrations":[{"id":"i-10","projectId":"p-heart","nightId":"n-10","targetId":"ic-1805","filter":"RGB","exposureSeconds":120,"gain":100,"binning":1,"temperatureC":null,"frameCount":120,"totalSeconds":14400,"eveningDate":"2026-08-10","sensor":null,"computerLabel":null,"installationId":null,"createdAt":"2026-08-10T10:00:00Z","updatedAt":"2026-08-10T10:00:00Z"},"#,
    );
    let without_right = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../astro-sync-core/tests/fixtures/online/space_storage.json"),
    )
    .unwrap()
    .replace(
        r#""analysis.progression": true"#,
        r#""analysis.progression": false"#,
    );
    let selling = FakeService::start(Wake::Instant, move |request| {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/api/v1/service") => Reply::Json(
                200,
                r#"{"service":"astro-portal","apiVersion":1,"version":"1.0.0","signIn":{"method":"email-code","spaceCreation":"open"}}"#
                    .to_string(),
            ),
            ("GET", "/api/v1/projects/p-heart") => Reply::Json(200, two_nights.clone()),
            ("GET", "/api/v1/space/storage") => Reply::Json(200, without_right.clone()),
            _ => Reply::Json(
                404,
                r#"{"error":{"code":"not_found","message":"no"}}"#.to_string(),
            ),
        }
    });
    let (ok, _, all) = run(&[
        "plan-image",
        "--project",
        "p-heart",
        "--root",
        &s(&root),
        "--night",
        &s(&twelfth),
        "--data",
        &s(&data),
        "--portal",
        &selling.base_url(),
        "--token",
        &s(&token),
        "--no-render",
    ]);
    assert!(!ok, "{all}");
    assert!(all.contains("outside the subscription"), "{all}");
    assert!(all.contains("plan image impossible"), "{all}");
    assert!(!all.contains("NotInSubscription"), "{all}");
}

const NIGHTS: &str = r#"{"projectId":"p-heart","nights":[],"candidates":[{"sessionId":"n-12","planId":null,"planName":null,"eveningDate":"2026-08-12","sessionStatus":"finished","link":null,"integrationCount":0,"integrationSeconds":0,"projectId":null}]}"#;

#[test]
fn a_night_is_proposed_searched_and_attached_from_the_command_line() {
    let storage = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../astro-sync-core/tests/fixtures/online/space_storage.json"),
    )
    .unwrap();
    let service = FakeService::start(Wake::Instant, move |request| {
        match (
            request.method.as_str(),
            request.path.split('?').next().unwrap_or_default(),
        ) {
            ("GET", "/api/v1/projects") => {
                Reply::Json(200, format!(r#"{{"projects":[{PROJECT}]}}"#))
            }
            ("GET", "/api/v1/projects/p-heart/nights") => Reply::Json(200, NIGHTS.to_string()),
            ("GET", "/api/v1/space/storage") => Reply::Json(200, storage.clone()),
            _ => Reply::Json(
                404,
                r#"{"error":{"code":"not_found","message":"no"}}"#.to_string(),
            ),
        }
    });
    let parent = temp("attach");
    let root = parent.join("Astro Sync");
    let twelfth = night(
        &root,
        "2026-08-12 · Cœur · n-12",
        "n-12",
        "UV_IR-cut_120s_g100",
        "UV/IR-cut",
    );
    let token = parent.join("token");
    std::fs::write(&token, "device-token").unwrap();
    let portal = service.base_url();
    let service_args = [
        "--portal",
        portal.as_str(),
        "--token",
        token.to_str().unwrap(),
    ];

    // The plans proposed for the night: that of its target.
    let mut args = vec![
        "plan-attach",
        "--night",
        twelfth.to_str().unwrap(),
        "--json",
    ];
    args.extend(service_args);
    let (ok, listed, all) = run(&args);
    assert!(ok, "{all}");
    assert_eq!(listed["mode"], "list");
    assert_eq!(listed["nightId"], "n-12");
    assert_eq!(listed["choices"]["plans"][0]["projectId"], "p-heart");
    assert_eq!(listed["choices"]["plans"][0]["state"], "available");

    // The search, by the name of the target, with no night at all.
    let mut args = vec!["plan-attach", "--search", "ic 18", "--json"];
    args.extend(service_args);
    let (ok, found, all) = run(&args);
    assert!(ok, "{all}");
    assert_eq!(found["mode"], "search");
    assert_eq!(found["found"][0]["projectId"], "p-heart");

    // A service that never said whether it sells the multi-night plans
    // (no `GET /v1/service` here): the right is unknown, attaching,
    // detaching and creating ask nothing.
    for extra in [
        vec!["--project", "p-heart"],
        vec!["--project", "p-heart", "--detach"],
        vec!["--create", "--target-name", "IC 1805"],
    ] {
        let mut args = vec![
            "plan-attach",
            "--night",
            twelfth.to_str().unwrap(),
            "--json",
        ];
        args.extend(extra.clone());
        args.extend(service_args);
        let (ok, closed, all) = run(&args);
        assert!(ok, "{all}");
        let outcome = if extra[0] == "--create" {
            &closed["created"]
        } else {
            &closed["change"]
        };
        assert_eq!(outcome["result"], "closed", "{extra:?}");
        assert_eq!(outcome["closed"], "unknown");
    }
    assert_eq!(service.app_hits("POST", "/api/v1/projects"), 0);
    assert_eq!(service.app_hits("DELETE", "/api/v1/projects"), 0);

    // A dry run says what would be asked and asks nothing that writes.
    let mut args = vec![
        "plan-attach",
        "--night",
        twelfth.to_str().unwrap(),
        "--project",
        "p-heart",
        "--dry-run",
        "--json",
    ];
    args.extend(service_args);
    let (ok, dry, all) = run(&args);
    assert!(ok, "{all}");
    assert_eq!(
        (dry["mode"].as_str(), dry["dryRun"].as_bool()),
        (Some("attach"), Some(true))
    );
    assert!(dry["change"].is_null());
}

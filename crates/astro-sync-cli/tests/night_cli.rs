//! The commands of the rewritten journey, the real binary: each
//! one's `--dry-run` (or its read-only form) writes and sends nothing; the
//! working folder, the night folder and the prompt; the "Summary" against
//! a fake service. Invented nights only.

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
        "astro-sync-cli-night-{label}-{}-{nanos}-{}",
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

fn files_under(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            if entry.path().is_dir() {
                stack.push(entry.path());
            } else {
                files.push(entry.path());
            }
        }
    }
    files.sort();
    files
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

/// The working folder and a night folder made by the commands.
fn night(root: &Path) -> PathBuf {
    let (ok, made, all) = run(&[
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
        "--unticked",
        "ic1396=IC 1396",
        "--json",
    ]);
    assert!(ok, "{all}");
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
    std::fs::write(folder.join("M31/final/M31.png"), png(300, 200)).unwrap();
    folder
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
    bytes
}

#[test]
fn the_working_folder_and_the_night_folder_are_written_only_without_dry_run() {
    let parent = temp("folders");
    let root = parent.join("Astro Sync");
    let (ok, answer, all) = run(&[
        "working-folder",
        "--root",
        &s(&root),
        "--prepare",
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{all}");
    assert_eq!(answer["exists"], false);
    assert!(!root.exists(), "--dry-run writes nothing");
    let data = parent.join("data");
    let (ok, answer, all) = run(&[
        "working-folder",
        "--root",
        &s(&root),
        "--data",
        &s(&data),
        "--prepare",
        "--json",
    ]);
    assert!(ok, "{all}");
    assert_eq!(answer["assistantFile"]["outcome"], "created");
    assert!(root.join("ASSISTANT-IA.md").is_file());
    assert!(data.join("local-settings.json").is_file());

    let before = files_under(&root);
    let (ok, planned, all) = run(&[
        "night-folder",
        "--root",
        &s(&root),
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
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{all}");
    assert_eq!(planned["applied"], false);
    assert_eq!(files_under(&root), before, "--dry-run writes nothing");

    let folder = night(&root);
    assert!(folder.ends_with("2026-10-07 · Col des Étoiles · 7f3a2c"));
    assert!(
        !folder.join("IC 1396").exists(),
        "an unticked target has no folder"
    );
    let (ok, listed, all) = run(&["nights", "--root", &s(&root), "--json"]);
    assert!(ok, "{all}");
    assert_eq!(listed[0]["nightId"], NIGHT_ID);
}

/// `night-folder` is run again at each opening of the step -- the second call
/// creates nothing and leaves a dropped file alone -- and two folders carrying
/// the night are followed by `--follow`, never picked.
#[test]
fn night_folder_run_again_creates_nothing_and_a_copy_is_followed_on_request() {
    let parent = temp("again");
    let root = parent.join("Astro Sync");
    std::fs::create_dir_all(&root).unwrap();
    let folder = night(&root);
    let dropped = folder.join("M31/lights/Light_9.fits");
    std::fs::write(&dropped, b"kept").unwrap();
    let before = files_under(&root);

    let args = |extra: &[&str]| {
        let mut all = vec![
            "night-folder",
            "--root",
            &s(&root),
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
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
        all.extend(extra.iter().map(|item| (*item).to_string()));
        all
    };
    let run_args = |all: Vec<String>| {
        let borrowed: Vec<&str> = all.iter().map(String::as_str).collect();
        run(&borrowed)
    };
    let (ok, again, all) = run_args(args(&[]));
    assert!(ok, "{all}");
    assert_eq!(again["created"], false);
    assert_eq!(again["createdPaths"], serde_json::json!([]));
    assert_eq!(
        files_under(&root),
        before,
        "nothing was made, nothing moved"
    );
    assert_eq!(std::fs::read(&dropped).unwrap(), b"kept");

    // A copy of the night's identity elsewhere: two folders carry the night.
    let copy = root.join("copie");
    std::fs::create_dir(&copy).unwrap();
    for entry in std::fs::read_dir(&folder).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            std::fs::copy(entry.path(), copy.join(entry.file_name())).unwrap();
        }
    }
    let (ok, _, all) = run_args(args(&[]));
    assert!(!ok, "two copies, nothing chosen: refused\n{all}");
    assert!(all.contains("two folders carry the night"), "{all}");
    let (ok, followed, all) = run_args(args(&["--follow", &s(&copy)]));
    assert!(ok, "{all}");
    assert_eq!(followed["folder"], s(&copy));
    assert!(copy.join("M31/lights").is_dir());
}

#[test]
fn the_read_only_commands_and_the_dry_runs_of_the_treatments() {
    let parent = temp("treatments");
    let root = parent.join("Astro Sync");
    std::fs::create_dir_all(&root).unwrap();
    let folder = night(&root);
    let data = parent.join("data");
    let before = files_under(&folder);

    let (ok, detection, all) = run(&[
        "scan",
        "--folder",
        &s(&folder),
        "--detect",
        "--target",
        "m31=M31",
        "--evening-date",
        "2026-10-07",
        "--json",
    ]);
    assert!(ok, "{all}");
    assert_eq!(detection["targets"][0]["lightFrames"], 3);
    assert!(
        detection["findings"]
            .to_string()
            .contains("detection.flatsMissing")
    );

    let (ok, families, all) = run(&[
        "progression-images",
        "--night",
        &s(&folder),
        "--night-id",
        NIGHT_ID,
        "--target-folder",
        "M31",
        "--data",
        &s(&data),
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{all}");
    assert_eq!(
        families["families"][0]["framePaths"]
            .as_array()
            .unwrap()
            .len(),
        3
    );

    let (ok, plan, all) = run(&[
        "stack",
        "--night",
        &s(&folder),
        "--target-folder",
        "M31",
        "--target-id",
        "m31",
        "--evening-date",
        "2026-10-07",
        "--pipeline",
        "raw",
        "--pipeline",
        "mono-ha",
        "--trusted",
        &format!("mono-ha={}", "ab".repeat(32)),
        "--data",
        &s(&data),
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{all}");
    let command = plan["jobs"][0]["command"].to_string();
    assert!(
        command.contains("\"--pipeline\",\"raw\",\"--pipeline\",\"mono-ha\""),
        "{command}"
    );
    assert!(command.contains(&format!("mono-ha={}", "ab".repeat(32))));
    assert!(
        plan["jobs"][0]["job"]["outputDir"]
            .as_str()
            .unwrap()
            .ends_with("M31/stacked")
    );
    assert_eq!(plan["jobs"].as_array().unwrap().len(), 1, "one family");
    assert_eq!(
        plan["neededBytes"].as_u64(),
        Some(3 * 210 * 1000 * 1000),
        "about 210 MB per light frame"
    );
    // G10: `--leave-out <file>` leaves the poses named out of the stacking,
    // says how many, says the names that are no pose of the night, and moves
    // or deletes nothing.
    let list = parent.join("nuages.txt");
    std::fs::write(
        &list,
        "# clouds\nLight_2.fits\n\nM31/lights/Light_9.fits\nLight_99.fits\n",
    )
    .unwrap();
    let lights_before = files_under(&folder.join("M31/lights"));
    let (ok, left_out, all) = run(&[
        "stack",
        "--night",
        &s(&folder),
        "--target-folder",
        "M31",
        "--target-id",
        "m31",
        "--evening-date",
        "2026-10-07",
        "--leave-out",
        &s(&list),
        "--data",
        &s(&data),
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{all}");
    let frames = left_out["jobs"][0]["job"]["frames"].as_array().unwrap();
    assert_eq!(frames.len(), 2, "one of three poses left out: {frames:?}");
    assert!(
        frames
            .iter()
            .all(|f| !f.as_str().unwrap().ends_with("Light_2.fits"))
    );
    assert!(
        all.contains(
            "1 frame left out according to nuages.txt (they stay in lights/, nothing is deleted)"
        ),
        "{all}"
    );
    assert!(
        all.contains(
            "2 names in nuages.txt are not frames of this night: M31/lights/Light_9.fits, Light_99.fits"
        ),
        "{all}"
    );
    assert_eq!(files_under(&folder.join("M31/lights")), lights_before);
    // The room is measured on the night's disk (where the scratch is), and
    // the plan names that folder, as the refusal for lack of room does.
    assert_eq!(plan["folder"].as_str(), Some(s(&folder).as_str()));
    assert_eq!(
        plan["availableBytes"].is_null(),
        astro_sync_core::stacking::free_bytes(&folder).is_none()
    );
    // --contrast goes to the script and to the job, and a word that is not
    // one of the three is refused. Without it, `presentation`, the
    // window's default (one default everywhere); `--contrast none` writes
    // only the image without a curve.
    assert!(
        command.contains("\"--contrast\",\"presentation\""),
        "{command}"
    );
    assert_eq!(plan["jobs"][0]["job"]["contrast"], "presentation");
    let (ok, plain, all) = run(&[
        "stack",
        "--night",
        &s(&folder),
        "--target-folder",
        "M31",
        "--target-id",
        "m31",
        "--evening-date",
        "2026-10-07",
        "--contrast",
        "none",
        "--data",
        &s(&data),
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{all}");
    let command_none = plain["jobs"][0]["command"].to_string();
    assert!(!command_none.contains("--contrast"), "{command_none}");
    assert!(plain["jobs"][0]["job"].get("contrast").is_none());
    let (ok, curved, all) = run(&[
        "stack",
        "--night",
        &s(&folder),
        "--target-folder",
        "M31",
        "--target-id",
        "m31",
        "--evening-date",
        "2026-10-07",
        "--contrast",
        "strong",
        "--data",
        &s(&data),
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{all}");
    let command = curved["jobs"][0]["command"].to_string();
    assert!(command.contains("\"--contrast\",\"strong\""), "{command}");
    assert_eq!(curved["jobs"][0]["job"]["contrast"], "strong");
    let (ok, _, all) = run(&[
        "stack",
        "--night",
        &s(&folder),
        "--target-folder",
        "M31",
        "--target-id",
        "m31",
        "--evening-date",
        "2026-10-07",
        "--contrast",
        "extreme",
        "--data",
        &s(&data),
        "--dry-run",
        "--json",
    ]);
    assert!(!ok, "{all}");
    assert!(
        all.contains("presentation"),
        "the refusal names the choices: {all}"
    );
    // --finish gives one pipeline a finish of its own, like the « Finition » of
    // its card; --frame-size a notch of the frame. Both reach the script.
    let (ok, finished, all) = run(&[
        "stack",
        "--night",
        &s(&folder),
        "--target-folder",
        "M31",
        "--target-id",
        "m31",
        "--evening-date",
        "2026-10-07",
        "--pipeline",
        "raw",
        "--pipeline",
        "hoo",
        "--finish",
        "hoo=strong,20,50",
        "--frame-size",
        "1",
        "--data",
        &s(&data),
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{all}");
    let command = finished["jobs"][0]["command"].to_string();
    assert!(
        command.contains("\"--finish\",\"hoo=strong,20,50\""),
        "{command}"
    );
    assert!(!command.contains("--finish\",\"raw"), "{command}");
    assert!(command.contains("\"--frame-size\",\"1\""), "{command}");
    assert_eq!(finished["jobs"][0]["job"]["frame"], 1);
    for wrong in [
        ["--finish", "hoo=strong,20"],
        ["--frame-size", "5"],
        ["--frame-size", "half"],
    ] {
        let (ok, _, all) = run(&[
            "stack",
            "--night",
            &s(&folder),
            "--target-folder",
            "M31",
            "--target-id",
            "m31",
            "--evening-date",
            "2026-10-07",
            wrong[0],
            wrong[1],
            "--data",
            &s(&data),
            "--dry-run",
            "--json",
        ]);
        assert!(!ok, "{wrong:?} is refused: {all}");
    }
    // R348b: --polished-engine reaches the script and the job; the default says nothing,
    // and a word that is not one of the two engines is refused.
    assert!(!command.contains("--polished-engine"), "{command}");
    let (ok, siril, all) = run(&[
        "stack",
        "--night",
        &s(&folder),
        "--target-folder",
        "M31",
        "--target-id",
        "m31",
        "--evening-date",
        "2026-10-07",
        "--polished-engine",
        "siril",
        "--data",
        &s(&data),
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{all}");
    let command = siril["jobs"][0]["command"].to_string();
    assert!(
        command.contains("\"--polished-engine\",\"siril\""),
        "{command}"
    );
    assert_eq!(siril["jobs"][0]["job"]["polishedEngine"], "siril");
    let (ok, _, all) = run(&[
        "stack",
        "--night",
        &s(&folder),
        "--target-folder",
        "M31",
        "--target-id",
        "m31",
        "--evening-date",
        "2026-10-07",
        "--polished-engine",
        "magic",
        "--data",
        &s(&data),
        "--dry-run",
        "--json",
    ]);
    assert!(!ok, "{all}");
    assert!(
        all.contains("graxpert-ai"),
        "the refusal names the engines: {all}"
    );
    // "No processing" (--no-render): no render, the linear stack only.
    let (ok, linear, all) = run(&[
        "stack",
        "--night",
        &s(&folder),
        "--target-folder",
        "M31",
        "--target-id",
        "m31",
        "--evening-date",
        "2026-10-07",
        "--no-render",
        "--data",
        &s(&data),
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{all}");
    let command = linear["jobs"][0]["command"].to_string();
    assert!(command.contains("\"--linear-only\""), "{command}");
    assert!(
        !command.contains("--pipeline") && !command.contains("--starnet"),
        "{command}"
    );
    assert_eq!(
        linear["jobs"][0]["job"]["pipelines"][0]["pipelineId"].as_str(),
        Some("linear")
    );
    // The pipelines are chosen target by target. "No processing"
    // goes beside the renders (written after them, `--linear`), and the
    // `--target-pipeline` of the target stacked replaces `--pipeline`; those of
    // the other targets are ignored.
    let (ok, both, all) = run(&[
        "stack",
        "--night",
        &s(&folder),
        "--target-folder",
        "M31",
        "--target-id",
        "m31",
        "--evening-date",
        "2026-10-07",
        "--no-render",
        "--pipeline",
        "raw",
        "--target-pipeline",
        "NGC 7000=hoo",
        "--target-pipeline",
        "m31=polished,dualband-correction",
        "--data",
        &s(&data),
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{all}");
    let command = both["jobs"][0]["command"].to_string();
    assert!(
        command.contains(
            "\"--pipeline\",\"polished\",\"--pipeline\",\"dualband-correction\",\"--linear\""
        ),
        "{command}"
    );
    assert!(
        !command.contains("\"raw\"") && !command.contains("\"hoo\""),
        "{command}"
    );
    let ids: Vec<&str> = both["jobs"][0]["job"]["pipelines"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["pipelineId"].as_str())
        .collect();
    assert_eq!(ids, ["polished", "dualband-correction", "linear"]);
    let (ok, _, all) = run(&[
        "stack",
        "--night",
        &s(&folder),
        "--target-folder",
        "M31",
        "--target-id",
        "m31",
        "--evening-date",
        "2026-10-07",
        "--target-pipeline",
        "M31",
        "--data",
        &s(&data),
        "--dry-run",
        "--json",
    ]);
    assert!(!ok && all.contains("TARGET=ID,ID"), "{all}");

    // Not a dry run: without the service nor the capabilities in Astro
    // Sync's cache, the stacking is not gated (no account is needed); it
    // stops here at the tools, and the words say why.
    let (ok, _, all) = run(&[
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
        &s(&data),
        "--json",
    ]);
    assert!(!ok && !all.contains("capabilities unknown"), "{all}");
    assert!(all.contains("Settings › Dependencies"), "{all}");
    assert!(!folder.join("M31/stacked").exists(), "nothing written");

    let (ok, rows, all) = run(&["tools", "--data", &s(&data), "--json"]);
    assert!(ok, "{all}");
    assert!(rows.to_string().contains("\"stacking-pipeline\""));
    // GraXpert has its row, optional, with its download for this system.
    let graxpert = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "graxpert")
        .expect("a GraXpert row");
    assert_eq!(graxpert["required"], false);
    assert!(
        graxpert["licence"]
            .as_str()
            .unwrap()
            .contains("CC-BY-NC-SA-4.0")
    );
    let (ok, to_fetch, all) = run(&[
        "tools",
        "--data",
        &s(&data),
        "--install",
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{all}");
    // Only ASTAP and its star database carry a download; which of them is
    // missing depends on this computer.
    for id in to_fetch.as_array().expect("a list of ids") {
        assert!(
            matches!(id.as_str(), Some("astap" | "astap-database")),
            "{to_fetch}"
        );
    }

    let (ok, list, all) = run(&["list-pipelines", "--lang", "ja", "--json"]);
    assert!(ok, "{all}");
    assert_eq!(list["pipelines"][0]["id"], "raw");
    // Four pipelines, each with its steps, as written and in the
    // language asked ("Details" of the window).
    let ids: Vec<&str> = list["pipelines"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["id"].as_str())
        .collect();
    assert_eq!(ids, ["raw", "dualband-correction", "hoo", "polished"]);
    let polished = &list["pipelines"][3];
    assert_eq!(polished["displayName"], "丁寧な仕上がり");
    assert_eq!(polished["details"][0]["title"]["en"], "Sky background");
    assert_eq!(polished["displayDetails"][0]["title"], "背景補正");
    assert_eq!(
        polished["details"].as_array().map(Vec::len),
        polished["displayDetails"].as_array().map(Vec::len)
    );
    assert_eq!(list["pipelines"][2]["filter"], "dual-band");
    // Recommended or discouraged by band of target, never refused,
    // and the download size of each tool for this system.
    assert_eq!(
        list["pipelines"][2]["recommendedFor"],
        serde_json::json!(["dualband"])
    );
    assert_eq!(
        list["pipelines"][2]["discouragedFor"],
        serde_json::json!(["broadband"])
    );
    assert_eq!(
        list["pipelines"][0]["recommendedFor"],
        serde_json::json!(["broadband"])
    );
    assert_eq!(
        list["pipelines"][0]["discouragedFor"],
        serde_json::json!([])
    );
    if astro_sync_core::tools::this_system().is_some() {
        for dependency in list["pipelines"][3]["dependencies"].as_array().unwrap() {
            assert!(
                dependency["bytes"]
                    .as_u64()
                    .is_some_and(|bytes| bytes > 1_000_000),
                "{dependency}"
            );
        }
    }
    // The folder of the person's own definitions is said (`--data` given).
    assert!(list.get("userFolder").is_none(), "none without --data");
    let (ok, with_data, all) = run(&["list-pipelines", "--data", &s(&data), "--json"]);
    assert!(ok, "{all}");
    assert!(
        with_data["userFolder"]
            .as_str()
            .is_some_and(|folder| folder.ends_with("pipelines")),
        "{with_data}"
    );

    std::fs::create_dir_all(folder.join("logs")).unwrap();
    let (ok, report, all) = run(&["send-logs", "--folder", &s(&folder), "--dry-run", "--json"]);
    assert!(ok, "{all}");
    assert_eq!(report["report"]["reportVersion"], 3);

    // A header written for the PNG: solved without any solver, sent nowhere.
    let image = folder.join("M31/final/M31.png");
    let wcs: String = [
        ("NAXIS1", "300"),
        ("NAXIS2", "200"),
        ("CTYPE1", "'RA---TAN'"),
        ("CTYPE2", "'DEC--TAN'"),
        ("CRVAL1", "10.68"),
        ("CRVAL2", "41.27"),
        ("CRPIX1", "150.5"),
        ("CRPIX2", "100.5"),
        ("CD1_1", "-0.0005"),
        ("CD1_2", "0"),
        ("CD2_1", "0"),
        ("CD2_2", "0.0005"),
    ]
    .iter()
    .map(|(k, v)| format!("{k:<8}= {v}\n"))
    .collect();
    std::fs::write(parent.join("M31.wcs"), &wcs).unwrap();
    let outside = parent.join("M31.png");
    std::fs::copy(&image, &outside).unwrap();
    let (ok, solution, all) = run(&[
        "solve",
        "--image",
        &s(&outside),
        "--data",
        &s(&data),
        "--capture-id",
        "c-1",
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{all}");
    assert_eq!(solution["source"], "header");
    assert!((solution["pixelScaleArcsec"].as_f64().unwrap() - 1.8).abs() < 1e-6);

    let (ok, prompt, all) = run(&[
        "assistant-prompt",
        "--root",
        &s(&root),
        "--night-folder",
        "2026-10-07 · Col des Étoiles · 7f3a2c",
        "--target",
        "M31",
        "--lang",
        "ja",
        "--json",
    ]);
    assert!(ok, "{all}");
    assert!(
        prompt["prompt"]
            .as_str()
            .unwrap()
            .starts_with("日本語で話してください。\n")
    );

    // The night's date, the common name, and the file it points to.
    let (ok, prompt, all) = run(&[
        "assistant-prompt",
        "--root",
        &s(&root),
        "--night-folder",
        "2026-10-07 · Col des Étoiles · 7f3a2c",
        "--target",
        "M31",
        "--name",
        "M31=Galaxie d’Andromède",
        "--json",
    ]);
    assert!(ok, "{all}");
    let text = prompt["prompt"].as_str().unwrap();
    assert!(
        text.contains("from the evening of 2026-10-07 to the morning of 2026-10-08"),
        "{text}"
    );
    assert!(text.contains("M31 (Galaxie d’Andromède)"), "{text}");
    let (ok, file, all) = run(&["assistant-prompt", "--file", "--json"]);
    assert!(ok, "{all}");
    assert_eq!(file["file"], "ASSISTANT-IA.md");
    assert_eq!(
        file["text"].as_str().unwrap(),
        astro_sync_core::assistant_prompt::ASSISTANT_FILE_TEXT
    );
    let (ok, _, all) = run(&["assistant-prompt", "--night-folder", "x", "--json"]);
    assert!(!ok, "--root is required without --file: {all}");

    assert_eq!(
        files_under(&folder),
        before,
        "nothing was written in the night's folder"
    );
}

#[test]
fn the_summary_and_the_dry_run_of_a_sending_ask_the_service_and_send_nothing() {
    let fixture = |name: &str| {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../astro-sync-core/tests/fixtures/online")
                .join(name),
        )
        .unwrap()
    };
    let online = fixture("night_bare.json");
    let storage = fixture("space_storage.json");
    let service = FakeService::start(Wake::Instant, move |request| {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/api/v1/space/storage") => Reply::Json(200, storage.clone()),
            ("GET", path) if path.ends_with("/online") => Reply::Json(200, online.clone()),
            ("POST", "/api/v1/uploads/check") => Reply::Json(
                200,
                r#"{"result":"accepted","adds":"capture","storage":{"usedBytes":1,"quotaBytes":100000,"maxFileBytes":50000,"captureCount":2,"captureCountLimit":30,"variantLimit":12}}"#.to_string(),
            ),
            _ => Reply::Json(404, r#"{"error":{"code":"not_found","message":"no"}}"#.to_string()),
        }
    });
    let parent = temp("summary");
    let root = parent.join("Astro Sync");
    std::fs::create_dir_all(&root).unwrap();
    let folder = night(&root);
    let token = parent.join("token");
    std::fs::write(&token, "device-token").unwrap();
    let portal = service.base_url();

    let (ok, check, all) = run(&[
        "check",
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
        "--json",
    ]);
    assert!(ok, "{all}");
    assert_eq!(check["quota"]["allowed"], true);
    assert_eq!(check["plans"][0]["decision"], "new_capture");
    // The proposals are in the summary, read from the headers of the three
    // lights; nothing of them is a file, a position or a date.
    let proposed = &check["plans"][0]["metadata"];
    assert_eq!(proposed["acquisition"][0]["filter"], "L");
    assert_eq!(proposed["acquisition"][0]["frameCount"], 3);
    assert_eq!(proposed["acquisition"][0]["exposureSeconds"], 120.0);
    assert_eq!(proposed["acquisition"][0]["gain"], 100);
    assert!(
        proposed.get("processing").is_none(),
        "a final image: no processing"
    );

    let queue = parent.join("queue.sqlite");
    let (ok, sending, all) = run(&[
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
        &s(&queue),
        "--data",
        &s(&parent.join("data")),
        "--logs",
        "--solve",
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{all}");
    assert_eq!(sending["dryRun"], true);
    assert_eq!(sending["enqueued"], 0);
    assert_eq!(
        sending["precheck"]["plans"][0]["metadata"]["acquisition"][0]["frameCount"], 3,
        "--dry-run --json shows what would be proposed"
    );
    assert_eq!(service.app_hits("POST", "/api/v1/uploads/check"), 2);
    assert_eq!(
        service.app_hits("POST", "/api/v1/uploads/"),
        2,
        "only the checks, no upload"
    );
    assert_eq!(
        service.app_hits("POST", "/api/v1/sessions"),
        0,
        "no analysis sent"
    );
    assert!(!queue.exists(), "nothing enqueued");
}

#[test]
fn the_milestones_of_a_progression_the_folder_holds_are_part_of_the_sending() {
    let fixture = |name: &str| {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../astro-sync-core/tests/fixtures/online")
                .join(name),
        )
        .unwrap()
    };
    let online = fixture("night_bare.json");
    let storage = fixture("space_storage.json");
    let service = FakeService::start(Wake::Instant, move |request| {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/api/v1/space/storage") => Reply::Json(200, storage.clone()),
            ("GET", path) if path.ends_with("/online") => Reply::Json(200, online.clone()),
            ("POST", "/api/v1/uploads/check") => Reply::Json(
                200,
                r#"{"result":"accepted","adds":"capture","storage":{"usedBytes":1,"quotaBytes":100000,"maxFileBytes":50000,"captureCount":2,"captureCountLimit":30,"variantLimit":12}}"#.to_string(),
            ),
            _ => Reply::Json(404, r#"{"error":{"code":"not_found","message":"no"}}"#.to_string()),
        }
    });
    let parent = temp("progression-sent");
    let root = parent.join("Astro Sync");
    std::fs::create_dir_all(&root).unwrap();
    let folder = night(&root);
    // A progression made earlier: three milestones and the record of their poses.
    let progression = folder.join("M31/progression");
    std::fs::create_dir_all(&progression).unwrap();
    for (index, frames) in [(1, 1), (2, 2), (3, 3)] {
        std::fs::write(
            progression.join(format!("milestone_{index:02}.jpg")),
            format!("milestone {index}"),
        )
        .unwrap();
        let _ = frames;
    }
    std::fs::write(
        progression.join("progression.json"),
        r#"{"format":1,"images":[
            {"file":"milestone_01.jpg","frames":1},
            {"file":"milestone_02.jpg","frames":2},
            {"file":"milestone_03.jpg","frames":3}]}"#,
    )
    .unwrap();
    let token = parent.join("token");
    std::fs::write(&token, "device-token").unwrap();
    let (ok, check, all) = run(&[
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
    assert!(ok, "{all}");
    let plans = check["plans"].as_array().unwrap();
    let milestones: Vec<(&str, u64)> = plans
        .iter()
        .filter(|plan| plan["image"]["kind"] == "progression")
        .map(|plan| {
            (
                plan["decision"]
                    .as_str()
                    .unwrap_or_else(|| plan["decision"]["decision"].as_str().unwrap_or("?")),
                plan["image"]["frameCount"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(milestones.len(), 3, "{all}");
    assert_eq!(
        milestones.iter().map(|m| m.1).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    // They belong to the capture the final image creates in the same sending.
    assert!(
        milestones.iter().all(|m| m.0.contains("progression")),
        "{milestones:?}"
    );
}

#[test]
fn the_summary_proposes_a_renders_title_in_the_language_asked() {
    let fixture = |name: &str| {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../astro-sync-core/tests/fixtures/online")
                .join(name),
        )
        .unwrap()
    };
    let online = fixture("night_bare.json");
    let storage = fixture("space_storage.json");
    let service = FakeService::start(Wake::Instant, move |request| {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/api/v1/space/storage") => Reply::Json(200, storage.clone()),
            ("GET", path) if path.ends_with("/online") => Reply::Json(200, online.clone()),
            ("POST", "/api/v1/uploads/check") => Reply::Json(
                200,
                r#"{"result":"accepted","adds":"capture","storage":{"usedBytes":1,"quotaBytes":100000,"maxFileBytes":50000,"captureCount":2,"captureCountLimit":30,"variantLimit":12}}"#.to_string(),
            ),
            _ => Reply::Json(404, r#"{"error":{"code":"not_found","message":"no"}}"#.to_string()),
        }
    });
    let parent = temp("render-texts");
    let root = parent.join("Astro Sync");
    std::fs::create_dir_all(&root).unwrap();
    let folder = night(&root);
    std::fs::create_dir_all(folder.join("M31/stacked")).unwrap();
    std::fs::write(folder.join("M31/stacked/M31_2026-10-07_raw.tif"), b"tiff").unwrap();
    std::fs::write(
        folder.join("M31/stacked/M31_2026-10-07_stacking.json"),
        r#"{"format":1,"frames":{"lights":3,"darks":0},"stackedFrames":3,"tools":{"siril":"siril 1.4.4"},"outputs":[{"pipeline":"raw","file":"M31_2026-10-07_raw.tif"}]}"#,
    )
    .unwrap();
    let token = parent.join("token");
    std::fs::write(&token, "device-token").unwrap();
    let portal = service.base_url();

    for (lang, title) in [
        ("en", "Natural render"),
        ("fr", "Rendu naturel"),
        ("ja", "ナチュラルな仕上がり"),
    ] {
        let (ok, check, all) = run(&[
            "check",
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
            "--lang",
            lang,
            "--json",
        ]);
        assert!(ok, "{all}");
        let plan = check["plans"]
            .as_array()
            .unwrap()
            .iter()
            .find(|plan| plan["image"]["kind"] == "stacked")
            .expect("the stacked render is planned");
        // The ten languages whatever the window speaks: the service shows
        // each reader its own.
        assert_eq!(
            plan["metadata"]["variantGeneratedTitle"][lang], title,
            "{lang}"
        );
        assert_eq!(
            plan["metadata"]["variantGeneratedTitle"]["en"], "Natural render",
            "{lang}"
        );
        assert!(plan["metadata"].get("variantTitle").is_none(), "{lang}");
        assert_eq!(
            plan["metadata"]["processing"]["software"],
            format!("Astro Sync {} · Siril 1.4.4", astro_sync_core::version())
        );
    }
}

/// G1: the logs of a night are on the clock of the computer that ran them
/// (UTC+2) and the frames say UTC. `send-logs` and `send-night --logs` find
/// the offset by themselves (the whole number of half hours that puts the most
/// frames in front of their guiding), say it in `--json` (`logsUtcOffset`),
/// send the report read with it, and keep `--phd2-utc-offset` to force one.
#[test]
fn the_logs_clock_is_found_from_the_frames_and_said_by_send_logs_and_send_night() {
    let parent = temp("logs-clock");
    let folder = parent.join("night");
    std::fs::create_dir_all(folder.join("logs")).unwrap();
    std::fs::create_dir_all(folder.join("Sh2-190/lights")).unwrap();
    let clock = |seconds: u32, hours_ahead: u32| {
        let total = (21 + hours_ahead) * 3600 + seconds;
        format!(
            "{:02}:{:02}:{:02}",
            total / 3600,
            total % 3600 / 60,
            total % 60
        )
    };
    let mut phd2 = format!(
        "PHD2 version 2.6.13dev4, Log version 2.5. Log enabled at 2026-08-09 {}\nGuiding Begins at 2026-08-09 {}\nPixel scale = 1.85 arc-sec/px, Binning = 1, Focal length = 400 mm\nFrame,Time,mount,dx,dy,RARawDistance,DECRawDistance,RAGuideDistance,DECGuideDistance,RADuration,RADirection,DECDuration,DECDirection,XStep,YStep,StarMass,SNR,ErrorCode\n",
        clock(0, 2),
        clock(0, 2)
    );
    for frame in 1..=300_u32 {
        phd2.push_str(&format!(
            "{frame},{:.3},Mount,0.010,-0.020,0.100,-0.100,0.000,0.000,0,,0,,0,0,31000,13.0,0\n",
            f64::from(frame) * 2.0
        ));
    }
    phd2.push_str(&format!("Guiding Ends at 2026-08-09 {}\n", clock(601, 2)));
    std::fs::write(folder.join("logs/PHD2_GuideLog_2026-08-09.txt"), phd2).unwrap();
    for index in 0..6_u32 {
        std::fs::write(
            folder.join(format!("Sh2-190/lights/Light_{index:04}.fits")),
            fits(&[
                ("IMAGETYP", "'LIGHT'"),
                ("OBJECT", "'Sh2-190'"),
                ("EXPTIME", "90"),
                ("GAIN", "100"),
                (
                    "DATE-OBS",
                    &format!("'2026-08-09T{}'", clock(index * 100, 0)),
                ),
            ]),
        )
        .unwrap();
    }

    // The report alone.
    let (ok, found, all) = run(&["send-logs", "--folder", &s(&folder), "--dry-run", "--json"]);
    assert!(ok, "{all}");
    let said = &found["logsUtcOffset"];
    assert_eq!(said["detected"], true, "{found}");
    assert_eq!(said["phd2"], "+02:00");
    assert_eq!(said["frames"], 6);
    assert_eq!(said["phd2MatchedFrames"], 6, "every frame has its guiding");
    let with_rms = found["report"]["quality"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| !row["rmsTotalArcsec"].is_null())
        .count();
    assert_eq!(with_rms, 6);
    let (ok, _, all) = run(&["send-logs", "--folder", &s(&folder), "--dry-run"]);
    assert!(ok, "{all}");
    assert!(
        all.contains("PHD2 +02:00 (6 frame(s) of 6)") && all.contains("found from the frames"),
        "{all}"
    );

    // Forced: the person's offset is used and said as given.
    let (ok, forced, all) = run(&[
        "send-logs",
        "--folder",
        &s(&folder),
        "--phd2-utc-offset",
        "+00:00",
        "--dry-run",
        "--json",
    ]);
    assert!(ok, "{all}");
    assert_eq!(forced["logsUtcOffset"]["detected"], false);
    assert_eq!(forced["logsUtcOffset"]["phd2"], "+00:00");
    assert_eq!(forced["logsUtcOffset"]["phd2MatchedFrames"], 0);

    // The sending: `send-night --logs` sends the report read at +02:00.
    let service = FakeService::start(Wake::Instant, move |request| {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/api/v1/space/storage") => Reply::Json(
                200,
                std::fs::read_to_string(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../astro-sync-core/tests/fixtures/online/space_storage.json"),
                )
                .unwrap(),
            ),
            ("GET", path) if path.ends_with("/online") => Reply::Json(
                200,
                std::fs::read_to_string(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../astro-sync-core/tests/fixtures/online/night_bare.json"),
                )
                .unwrap(),
            ),
            ("POST", "/api/v1/uploads/check") => Reply::Json(
                200,
                r#"{"result":"accepted","adds":"capture","storage":{"usedBytes":1,"quotaBytes":100000,"maxFileBytes":50000,"captureCount":0,"captureCountLimit":30,"variantLimit":12}}"#.to_string(),
            ),
            ("POST", path) if path.ends_with("/analyses") => Reply::Json(
                201,
                r#"{"analysis":{"id":"6c0f3a8e-1b2d-4e5f-8a9b-0c1d2e3f4a5b","sessionId":"7f3a2c91-0000-4000-8000-00000000000a","reportVersion":2,"createdAt":"2026-10-08T09:12:00.000Z","originDeviceId":null,"deviceName":"Mac","report":{}},"replaced":false,"steps":{"proposed":1,"keptEdited":0}}"#.to_string(),
            ),
            _ => Reply::Json(404, r#"{"error":{"code":"not_found","message":"no"}}"#.to_string()),
        }
    });
    let token = parent.join("token");
    std::fs::write(&token, "device-token").unwrap();
    let (ok, sent, all) = run(&[
        "send-night",
        "--portal",
        &service.base_url(),
        "--token",
        &s(&token),
        "--folder",
        &s(&folder),
        "--target",
        "sh2-190=Sh2-190",
        "--session-id",
        NIGHT_ID,
        "--queue",
        &s(&parent.join("queue.sqlite")),
        "--data",
        &s(&parent.join("data")),
        "--logs",
        "--json",
    ]);
    assert!(ok, "{all}");
    assert_eq!(
        sent["analysis"]["logsUtcOffset"]["detected"], true,
        "{sent}"
    );
    assert_eq!(sent["analysis"]["logsUtcOffset"]["phd2"], "+02:00");
    assert_eq!(sent["analysis"]["logsUtcOffset"]["phd2MatchedFrames"], 6);
    let posted = service
        .requests()
        .into_iter()
        .find(|request| request.method == "POST" && request.path.ends_with("/analyses"))
        .expect("the report was sent");
    let body: serde_json::Value = serde_json::from_slice(&posted.body).unwrap();
    let rows = body["quality"].as_array().expect("quality rows");
    assert_eq!(rows.len(), 6);
    assert!(
        rows.iter().all(|row| !row["rmsTotalArcsec"].is_null()),
        "the report sent holds the guiding of every frame: {body}"
    );
}

/// G5: `measure-master <file>` is the window's "Measure a master…" (same
/// script `star_quality.py --measure`, same `Outcome` as JSON); a refusal is
/// printed in the window's words and ends with an error. A stand-in
/// interpreter answers what the script does.
#[cfg(unix)]
#[test]
fn measure_master_runs_the_script_of_the_window_and_prints_its_outcome() {
    use std::os::unix::fs::PermissionsExt;
    let parent = temp("measure-master");
    let data = parent.join("data");
    let stacking = parent.join("stacking");
    std::fs::create_dir_all(data.join("stacking-env/bin")).unwrap();
    std::fs::create_dir_all(&stacking).unwrap();
    std::fs::write(stacking.join("star_quality.py"), "# stand-in\n").unwrap();
    let python = data.join("stacking-env/bin/python3");
    let interpreter = |answer: &str| {
        std::fs::write(&python, format!("#!/bin/sh\necho '{answer}'\n")).unwrap();
        std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    let master = parent.join("IC_1805_integration.fit");
    std::fs::write(&master, "x").unwrap();
    let args = |json: bool| {
        let mut args = vec![
            "measure-master".to_string(),
            s(&master),
            "--data".into(),
            s(&data),
            "--stacking".into(),
            s(&stacking),
        ];
        if json {
            args.push("--json".into());
        }
        args
    };
    let run_with = |json: bool| {
        let args = args(json);
        run(&args.iter().map(String::as_str).collect::<Vec<_>>())
    };

    interpreter(
        r#"{"status": "measured", "quality": {"fwhmPixels": 2.402, "eccentricity": 0.556, "starCount": 36, "measuredOn": "linear", "measuredAt": "2026-10-14T09:00:00Z"}}"#,
    );
    let (ok, outcome, all) = run_with(true);
    assert!(ok, "{all}");
    assert_eq!(outcome["result"], "measured");
    assert_eq!(outcome["master"], "IC_1805_integration.fit");
    assert_eq!(
        outcome["quality"]["fwhmPixels"], 2.4,
        "as the window sends it"
    );
    assert_eq!(outcome["quality"]["starCount"], 36);
    assert_eq!(outcome["quality"]["measuredOn"], "linear");
    let (ok, _, text) = run_with(false);
    assert!(
        ok && text.contains("Master measured") && text.contains("2.4 px"),
        "{text}"
    );

    interpreter(
        r#"{"status": "refused", "code": "stretched", "message": "stretched image: the median sits at 3.0% of its range"}"#,
    );
    let (ok, outcome, all) = run_with(true);
    assert!(!ok, "a refused master ends with an error: {all}");
    assert_eq!(outcome["result"], "refused");
    assert_eq!(outcome["code"], "stretched");
    let (_, _, text) = run_with(false);
    assert!(text.contains("Stretched image"), "{text}");

    // Nothing was kept anywhere: no night given.
    assert!(
        files_under(&parent)
            .iter()
            .all(|file| !file.to_string_lossy().contains("master-quality"))
    );
}

//! Golden tests ("CLI
//! parity"): for a deterministic fixture folder, the real `astro-sync`
//! binary's `--json` output for `scan`, `dry-run`, `package` (its produced
//! `manifest.json`), and `queue status` must be identical, byte for byte
//! after canonical JSON serialization
//! (`astro_sync_core::canonical_json`), to the structures
//! `astro-sync-core`'s own functions produce directly. That is what "the
//! CLI is a thin call into the core" means in practice: nothing here
//! reshapes what the core already produced. `report` joins the same
//! doctrine: the file it writes must be exactly
//! `analysis::report::generate_report`'s `SessionReport`, canonically
//! rendered.
//!
//! Two things vary from run to run and are neutralized before any
//! comparison:
//!
//! - **Timestamps** (`generatedAt`/`addedAt` in a package manifest,
//!   `createdAt`/`updatedAt` in a queue row): every CLI subprocess that can
//!   write one is given `ASTRO_SYNC_DETERMINISTIC_TIME` set to
//!   [`FIXED_TIME`] (the env var is `astro_sync_core::DETERMINISTIC_TIME_ENV_VAR`;
//!   see `README.md`, "CLI / GUI parity"), which
//!   `now_iso8601_utc` then returns verbatim instead of reading the wall
//!   clock. The "structure produced directly by the core" side of each
//!   comparison either never reads the clock at all (`scan`, `dry-run`,
//!   reading an already-written queue row back) or is built with that same
//!   fixed value (`package::build_manifest`). Setting the variable only on
//!   a child process's own environment (`Command::env`) never touches this
//!   test binary's own process-wide environment, so nothing here needs
//!   `unsafe` and tests stay safe to run in parallel.
//! - **The fixture folder's own absolute path** (embedded in
//!   `ScanReport.root`, `DryRunReport.source_root`, and each queue row's
//!   `source_path`): every JSON string below has it replaced with the
//!   literal placeholder `<FIXTURE_ROOT>` before being compared or written,
//!   so a committed golden file stays valid regardless of where the
//!   fixture happened to land on a given machine or run. This substitution
//!   is this test harness's own fixture-path scrubbing, unrelated to
//!   `canonical_json` itself (a real, general-purpose serialization concern
//!   the Tauri GUI shares).
//!
//! Golden files live under `tests/golden/*.json` next to this file and
//! hold the exact canonical, path-scrubbed string each test below asserts
//! against. After a deliberate, reviewed change to a report's shape,
//! regenerate them with:
//!
//! ```sh
//! UPDATE_GOLDEN=1 cargo test -p astro-sync-cli --test golden
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use astro_sync_core::analysis;
use astro_sync_core::analysis::report::render_report_json;
use astro_sync_core::package::{ProducerInfo, build_manifest};
use astro_sync_core::upload_queue::UploadQueue;
use astro_sync_core::uploader::UploadEntryResult;
use astro_sync_core::{DETERMINISTIC_TIME_ENV_VAR, canonical_json, dry_run, scanner};

/// The fixed instant every subprocess in this file uses instead of the wall
/// clock, so every `generatedAt`/`addedAt`/`createdAt`/`updatedAt` this file
/// sees is exactly this string, on every run, on every machine.
const FIXED_TIME: &str = "2026-09-20T21:04:00Z";

/// Stands in for the fixture folder's own absolute path in every golden
/// file -- see the module doc comment.
const FIXTURE_ROOT_PLACEHOLDER: &str = "<FIXTURE_ROOT>";

/// A unique, self-cleaning temporary directory. `astro-sync-core`'s own
/// `test_support::TempDir` is `#[cfg(test)]`-only and not visible from this
/// separate integration-test binary, so this is the same minimal pattern
/// duplicated locally (see `README.md`, "Dev-dependency
/// deviation: no `tempfile`").
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "astro-sync-cli-golden-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Writes the same fixed-content files under `dir` on every run: candidate
/// assets of different classes (FITS frames, a metrics CSV, a final image)
/// and some nested under a subdirectory, so every report below has more than
/// one entry and exercises directory-structure preservation. The package
/// keeps the results only (the final image): the FITS and the CSV are in the
/// scan and not in the package.
fn write_fixture(dir: &Path) {
    fs::write(dir.join("frame-0001.fits"), vec![0xAB_u8; 32]).unwrap();
    fs::write(dir.join("quality.csv"), b"fwhm,rms\n1.2,0.4\n").unwrap();
    fs::create_dir_all(dir.join("2026-09-20")).unwrap();
    fs::write(
        dir.join("2026-09-20").join("frame-0002.fits"),
        vec![0xCD_u8; 16],
    )
    .unwrap();
    fs::create_dir_all(dir.join("cible 1").join("final")).unwrap();
    fs::write(
        dir.join("cible 1").join("final").join("image.jpg"),
        vec![0xEF_u8; 24],
    )
    .unwrap();
}

/// Replaces every occurrence of `root`'s own path string with
/// [`FIXTURE_ROOT_PLACEHOLDER`].
fn scrub_fixture_root(json: &str, root: &Path) -> String {
    json.replace(&root.display().to_string(), FIXTURE_ROOT_PLACEHOLDER)
}

/// A fixed-content FITS header, block-padded to a multiple of 2880 bytes --
/// the same mechanical builder `astro-sync-core`'s own
/// `analysis::report::tests`/`tests/analysis_report_fixtures.rs` each carry
/// their own private copy of (`#[cfg(test)]`-only, not visible from this
/// separate crate).
fn build_fits_header_bytes(cards: &[(&str, &str)]) -> Vec<u8> {
    const CARD_LEN: usize = 80;
    const BLOCK_LEN: usize = 2880;
    let mut out = Vec::new();
    for (keyword, value) in cards {
        let card = format!("{keyword:<8}= {value}");
        let mut bytes = card.into_bytes();
        bytes.resize(CARD_LEN, b' ');
        out.extend_from_slice(&bytes);
    }
    let mut end_card = b"END".to_vec();
    end_card.resize(CARD_LEN, b' ');
    out.extend_from_slice(&end_card);
    let remainder = out.len() % BLOCK_LEN;
    if remainder != 0 {
        out.resize(out.len() + (BLOCK_LEN - remainder), b' ');
    }
    out
}

/// A minimal PHD2 guide log + ASIAIR autorun log + one FITS frame -- enough
/// for `analysis::analyze_folder` to produce a non-empty
/// `quality`/`guiding`/`timeline`/`acquisitionFamilies` report. Every
/// timestamp here is a fixed literal (never the wall clock), so the
/// resulting report's *content* (everything but `generatedAtUtc`, which
/// `report_writes_the_core_generated_report_verbatim` neutralizes
/// separately) is itself deterministic across machines and runs.
fn write_report_fixture(dir: &Path) {
    fs::write(
        dir.join("PHD2_GuideLog_2026-09-20.txt"),
        "Guiding Begins at 2026-09-20 21:00:00\n\
         Pixel scale = 2.00 arc-sec/px\n\
         Frame,Time,mount,dx,dy,RARawDistance,DECRawDistance,RAGuideDistance,DECGuideDistance,RADuration,RADirection,DECDuration,DECDirection,XStep,YStep,StarMass,SNR,ErrorCode\n\
         1,1.000,Mount,0,0,0.100,0.100,0,0,0,,0,,0,0,30000,12,0\n\
         2,3.000,Mount,0,0,0.200,0.100,0,0,0,,0,,0,0,30000,12,0\n\
         Guiding Ends at 2026-09-20 21:04:00\n",
    )
    .unwrap();
    fs::write(
        dir.join("Autorun_Log_2026-09-20.txt"),
        "2026/09/20 21:01:00 [Guide] Dither\n\
         2026/09/20 21:01:10 [Guide] Dither Settle\n",
    )
    .unwrap();
    fs::write(
        dir.join("frame-0001.fits"),
        build_fits_header_bytes(&[
            ("EXPTIME", "300.0"),
            ("FILTER", "'Ha      '"),
            ("GAIN", "100"),
            ("CCD-TEMP", "-10.0"),
            ("DATE-OBS", "'2026-09-20T21:00:00.000'"),
            ("XBINNING", "1"),
            ("YBINNING", "1"),
        ]),
    )
    .unwrap();
}

/// Runs the built `astro-sync` binary (located via Cargo's own
/// `CARGO_BIN_EXE_<name>`, no extra process-spawning dependency needed),
/// asserts it exited successfully, and returns its stdout.
fn run_cli(configure: impl FnOnce(&mut Command)) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_astro-sync"));
    configure(&mut command);
    let output = command.output().expect("the astro-sync binary runs");
    assert!(
        output.status.success(),
        "astro-sync exited with {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout is valid UTF-8")
}

/// Parses `json` into a `serde_json::Value` and re-serializes it
/// canonically, so a comparison never depends on the CLI's own
/// pretty-printing or field declaration order -- only on the values
/// themselves.
fn canonicalize_json_str(json: &str) -> String {
    let value: serde_json::Value =
        serde_json::from_str(json).unwrap_or_else(|error| panic!("invalid JSON ({error}): {json}"));
    canonical_json(&value)
}

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.json"))
}

/// Compares `actual` (already canonical and, where relevant, scrubbed of
/// the fixture root) against the committed golden file
/// `tests/golden/<name>.json`, or (re)writes it there when `UPDATE_GOLDEN=1`
/// is set.
fn assert_matches_golden(name: &str, actual: &str) {
    let path = golden_path(name);
    if std::env::var("UPDATE_GOLDEN").as_deref() == Ok("1") {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "could not read golden file {} ({error}); run `UPDATE_GOLDEN=1 cargo test -p astro-sync-cli --test golden` to create it"
        , path.display())
    });
    assert_eq!(
        actual, expected,
        "CLI --json output for {name:?} no longer matches tests/golden/{name}.json -- \
         if this is a deliberate, reviewed change to the report's shape, regenerate it with \
         `UPDATE_GOLDEN=1 cargo test -p astro-sync-cli --test golden`"
    );
}

/// Binds a loopback port, then immediately releases it: connecting to it
/// afterwards fails fast and deterministically (`ECONNREFUSED`), with no
/// real network involved. The same trick `commands.rs`'s own
/// `upload_stops_instead_of_looping_when_the_portal_is_unreachable` test
/// uses.
fn unused_loopback_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

#[test]
fn scan_json_is_the_core_scan_report_verbatim() {
    let fixture = TempDir::new("scan");
    write_fixture(fixture.path());

    let core_report = scanner::scan_folder(fixture.path()).expect("scan the fixture");
    let core_canonical = scrub_fixture_root(&canonical_json(&core_report), fixture.path());

    let stdout = run_cli(|command| {
        command
            .arg("scan")
            .arg("--folder")
            .arg(fixture.path())
            .arg("--json");
    });
    let cli_canonical = canonicalize_json_str(&scrub_fixture_root(&stdout, fixture.path()));

    assert_eq!(
        cli_canonical, core_canonical,
        "`scan --json` must print exactly astro_sync_core::scanner::scan_folder's ScanReport, \
         with no reshaping in the CLI"
    );
    assert_matches_golden("scan", &core_canonical);
}

#[test]
fn dry_run_json_is_the_core_dry_run_report_verbatim() {
    let fixture = TempDir::new("dry-run");
    write_fixture(fixture.path());

    let scan = scanner::scan_folder(fixture.path()).expect("scan the fixture");
    let core_report = dry_run::build_package_report(&scan);
    let core_canonical = scrub_fixture_root(&canonical_json(&core_report), fixture.path());

    let stdout = run_cli(|command| {
        command
            .arg("dry-run")
            .arg("--folder")
            .arg(fixture.path())
            .arg("--json");
    });
    let cli_canonical = canonicalize_json_str(&scrub_fixture_root(&stdout, fixture.path()));

    assert_eq!(
        cli_canonical, core_canonical,
        "`dry-run --json` must print exactly astro_sync_core::dry_run::build_package_report's \
         DryRunReport, with no reshaping in the CLI"
    );
    assert_matches_golden("dry_run", &core_canonical);
}

/// The `package` subcommand's `--json` output is `PackageOutcome`, which
/// (correctly) carries absolute paths derived from the caller's own
/// `--output` argument -- there is no fixture-independent "golden" form of
/// that struct to commit. What *is* fixture-independent, and is the
/// package's actual portable artifact (the thing a real importer reads,
/// per `contracts/schemas/session-package/`), is
/// `manifest.json` -- so that is what this test golden-compares, while
/// still checking `PackageOutcome` structurally (it round-trips through the
/// exact core type, with the counts a direct call to the core would
/// produce).
#[test]
fn package_writes_the_core_built_manifest_verbatim() {
    let fixture = TempDir::new("package-source");
    write_fixture(fixture.path());
    let output_parent = TempDir::new("package-output");

    let scan = scanner::scan_folder(fixture.path()).expect("scan the fixture");
    let report = dry_run::build_package_report(&scan);

    let stdout = run_cli(|command| {
        command
            .env(DETERMINISTIC_TIME_ENV_VAR, FIXED_TIME)
            .arg("package")
            .arg("--folder")
            .arg(fixture.path())
            .arg("--output")
            .arg(output_parent.path())
            .arg("--json");
    });

    let outcome: astro_sync_core::package::PackageOutcome =
        serde_json::from_str(&stdout).expect("`package --json` deserializes as PackageOutcome");
    assert_eq!(
        outcome.output_dir,
        output_parent.path().join(scanner::PACKAGE_OUTPUT_DIR_NAME)
    );
    assert_eq!(outcome.asset_count, report.entries.len());
    assert_eq!(outcome.total_bytes, report.total_bytes);

    let cli_manifest_json =
        fs::read_to_string(&outcome.manifest_path).expect("manifest.json was written");
    let cli_canonical = canonicalize_json_str(&cli_manifest_json);

    // The CLI's own defaults (`producer-name` defaults to "astro-sync-cli",
    // `producer-version` defaults to `astro_sync_core::version()`, and no
    // `--session-*` flag was passed): mirrored here so the manifest this
    // test builds directly is the exact one the CLI invocation above should
    // have produced.
    let producer = Some(ProducerInfo {
        name: "astro-sync-cli".to_string(),
        version: Some(astro_sync_core::version().to_string()),
    });
    let core_manifest = build_manifest(&report, FIXED_TIME.to_string(), None, producer);
    let core_canonical = canonical_json(&core_manifest);

    assert_eq!(
        cli_canonical, core_canonical,
        "the manifest.json `package` writes must be exactly \
         astro_sync_core::package::build_manifest's Manifest, with no reshaping in the CLI"
    );
    assert_matches_golden("package_manifest", &core_canonical);
}

#[test]
fn queue_status_json_is_the_core_queue_entries_verbatim() {
    let fixture = TempDir::new("queue-source");
    write_fixture(fixture.path());
    // Only results enter the queue -- the fixture's frames and its CSV stay
    // here; a final image goes.
    fs::create_dir_all(fixture.path().join("M31").join("final")).unwrap();
    fs::write(
        fixture.path().join("M31").join("final").join("m31.tif"),
        vec![0xEF_u8; 24],
    )
    .unwrap();
    let queue_dir = TempDir::new("queue-db");
    let queue_path = queue_dir.path().join("queue.sqlite");
    let token_path = queue_dir.path().join("token.txt");
    fs::write(&token_path, "fake-token").unwrap();

    // An immediately-refused connection: `upload` enqueues the fixture's
    // files (deterministic `createdAt`/`updatedAt`, via the env var) and
    // then fails fast on the network, leaving every row `queued` -- exactly
    // the regression scenario `commands.rs`'s own
    // `upload_stops_instead_of_looping_when_the_portal_is_unreachable` test
    // covers, reused here only as a way to populate a queue database
    // deterministically.
    let port = unused_loopback_port();
    let upload_stdout = run_cli(|command| {
        command
            .env(DETERMINISTIC_TIME_ENV_VAR, FIXED_TIME)
            .arg("upload")
            .arg("--queue")
            .arg(&queue_path)
            .arg("--portal")
            .arg(format!("http://127.0.0.1:{port}"))
            .arg("--token")
            .arg(&token_path)
            .arg("--folder")
            .arg(fixture.path())
            .arg("--json");
    });
    let upload_results: Vec<UploadEntryResult> = serde_json::from_str(&upload_stdout)
        .expect("`upload --json` deserializes as a list of UploadEntryResult");
    // The portal is unreachable, a transient (`UploadError::Transport`)
    // failure: the attempted entry's state does not change, so `upload`'s
    // own no-spin guard (`commands.rs`, "Arrêt : aucune progression...")
    // stops after this single attempt -- the one result stays `queued` in
    // the database, which is what this test actually needs.
    assert_eq!(
        upload_results.len(),
        1,
        "upload() stops after one non-progressing attempt instead of spinning"
    );
    assert!(
        !upload_results[0].ok,
        "the portal is unreachable: the attempted entry must have failed to send"
    );

    let stdout = run_cli(|command| {
        command
            .arg("queue")
            .arg("status")
            .arg("--queue")
            .arg(&queue_path)
            .arg("--json");
    });
    let cli_canonical = canonicalize_json_str(&scrub_fixture_root(&stdout, fixture.path()));

    // The core-produced reference reads the exact same database file back
    // -- it never calls `now_iso8601_utc` itself, so nothing here needs the
    // deterministic-time override; the rows it reads were already written
    // with it by the `upload` subprocess above.
    let core_entries = UploadQueue::open(&queue_path)
        .expect("reopen the queue database `upload` just populated")
        .list()
        .expect("list every queue row");
    let core_canonical = scrub_fixture_root(&canonical_json(&core_entries), fixture.path());

    assert_eq!(
        cli_canonical, core_canonical,
        "`queue status --json` must print exactly astro_sync_core::upload_queue::UploadQueue::list's \
         Vec<QueueEntry>, with no reshaping in the CLI"
    );
    assert_matches_golden("queue_status", &core_canonical);
}

/// `report --folder … --out report.json`: the file it writes must be
/// exactly `analysis::report::generate_report`'s own `SessionReport`,
/// canonically rendered -- the same shape/bytes the GUI's Analysis screen
/// would upload via `POST /v1/sessions/{id}/analyses`. `generatedAtUtc` is
/// the one field that legitimately differs between two independent calls a
/// few milliseconds apart (it is not read from any fixture, only from the
/// clock): the CLI subprocess gets `ASTRO_SYNC_DETERMINISTIC_TIME` via
/// `Command::env` (the child process's own environment only -- see the
/// module doc comment, "nothing here needs `unsafe`"), and the core
/// reference overrides the same field directly (`SessionReport`'s fields
/// are all `pub`) rather than touching this test binary's own process
/// environment to get a matching value.
#[test]
fn report_writes_the_core_generated_report_verbatim() {
    let fixture = TempDir::new("report-source");
    write_report_fixture(fixture.path());
    let out_dir = TempDir::new("report-output");
    let out_path = out_dir.path().join("report.json");

    // The command reads the clocks of the logs from the frames and says them
    // (`logClock`): the reference does the same.
    let (outcome, clock) = analysis::log_offset::analyze_folder_detecting(
        fixture.path(),
        &analysis::log_offset::ForcedLogOffsets::default(),
        None,
    )
    .expect("analyze the fixture folder");
    let mut core_report = analysis::report::report_from_outcome(&outcome);
    core_report.log_clock = clock.as_ref().map(analysis::report::LogClock::from);
    core_report.generated_at_utc = FIXED_TIME.to_string();
    let core_canonical = render_report_json(&core_report);

    run_cli(|command| {
        command
            .env(DETERMINISTIC_TIME_ENV_VAR, FIXED_TIME)
            .arg("report")
            .arg("--folder")
            .arg(fixture.path())
            .arg("--out")
            .arg(&out_path);
    });

    let written = fs::read_to_string(&out_path).expect("report.json was written");
    let cli_canonical = canonicalize_json_str(&written);

    assert_eq!(
        cli_canonical, core_canonical,
        "`report --out report.json` must write exactly \
         astro_sync_core::analysis::report::generate_report's SessionReport, canonically \
         rendered, with no reshaping in the CLI"
    );
    assert_matches_golden("report", &core_canonical);
}

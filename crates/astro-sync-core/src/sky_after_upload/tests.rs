use super::*;
use crate::analysis::wcs::tan_cards;
use crate::raster::samples;
use crate::sky_solution::{
    MissingCapture, SkySolutionPut, SolverChoice, Solvers, WrittenSkySolution,
};
use crate::test_support::TempDir;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::Duration;

const NOW: &str = "2026-10-23T17:10:00Z";
const SHA: &str = "ab";

/// Header text, one card per line: a TAN solution centred on the image.
fn solved_header(width: u32, height: u32) -> String {
    let crpix = (f64::from(width) / 2.0 + 0.5, f64::from(height) / 2.0 + 0.5);
    let mut cards = vec![
        ("NAXIS1", width.to_string()),
        ("NAXIS2", height.to_string()),
        ("SWCREATE", "'N.I.N.A. 3.1'".to_string()),
    ];
    cards.extend(tan_cards(38.0, 61.4, crpix, 2.0, 0.0));
    cards
        .iter()
        .map(|(key, value)| format!("{key:<8}= {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The service: the captures still without a solution, what was put.
struct Service {
    missing: Vec<serde_json::Value>,
    /// The listing fails (the network is down).
    down: bool,
    put: RefCell<Vec<(String, SkySolutionPut)>>,
    listed: RefCell<u32>,
}

impl Service {
    fn with_missing(capture_id: &str, sha: &str) -> Self {
        Self {
            missing: vec![serde_json::json!({
                "id": capture_id, "originalName": "IC_1805_plan_raw.tif", "sha256": sha,
                "byteSize": 100, "mainImageId": "img-1", "mainImageSource": "astro-sync",
                "targets": [{"id": "t-ic1805", "designation": "IC 1805",
                             "raHours": 2.53, "decDegrees": 61.4}]
            })],
            down: false,
            put: RefCell::new(Vec::new()),
            listed: RefCell::new(0),
        }
    }
}

impl SkySolutionTransport for Service {
    fn put_sky_solution(
        &self,
        _base_url: &str,
        _token: &str,
        capture_id: &str,
        body: &SkySolutionPut,
    ) -> Result<WrittenSkySolution, GuiBridgeError> {
        self.put
            .borrow_mut()
            .push((capture_id.to_string(), body.clone()));
        Ok(WrittenSkySolution {
            sky_solution: serde_json::to_value(&body.solution).unwrap(),
            created: true,
            changed: true,
        })
    }

    fn missing_page(
        &self,
        _base_url: &str,
        _token: &str,
        _since: Option<&str>,
    ) -> Result<serde_json::Value, GuiBridgeError> {
        *self.listed.borrow_mut() += 1;
        if self.down {
            return Err(GuiBridgeError::Transport("no network".into()));
        }
        Ok(serde_json::json!({"captures": self.missing, "hasMore": false, "nextSince": null}))
    }
}

impl OriginalTransport for Service {
    fn original(
        &self,
        _base_url: &str,
        _token: &str,
        _capture_id: &str,
        _image_id: &str,
        _from: u64,
    ) -> Result<(u16, Vec<u8>), GuiBridgeError> {
        panic!("the image just sent is here: its original is never fetched");
    }
}

struct Fixture {
    dir: TempDir,
    originals: PathBuf,
    solve: PathBuf,
    queue: UploadQueue,
    image: UploadedImage,
}

/// A plan image on the disk (a PNG, no header) already in the queue.
fn fixture(kind: ResultKind) -> Fixture {
    let dir = TempDir::new("sky-after-upload");
    let path = dir.path().join("IC_1805_plan_raw.png");
    std::fs::write(&path, samples::png(2000, 1500)).unwrap();
    let queue = UploadQueue::open(&dir.path().join("queue.sqlite")).unwrap();
    let sha = SHA.repeat(32);
    Fixture {
        image: UploadedImage {
            sha256: sha,
            capture_id: "cap-plan".to_string(),
            path,
            kind,
        },
        queue,
        originals: dir.path().join("originals"),
        solve: dir.path().join("solve"),
        dir,
    }
}

fn setup<'a>(solvers: &'a Solvers, fx: &'a Fixture) -> SolveSetup<'a> {
    SolveSetup {
        solvers,
        choice: SolverChoice::Astap,
        originals_dir: &fx.originals,
        work_root: &fx.solve,
        timeout: Duration::from_secs(20),
        cancel: None,
        now: NOW,
    }
}

fn run(service: &Service, fx: &Fixture, solvers: &Solvers) -> (SkyAnnotation, Vec<SkyState>) {
    let mut seen = Vec::new();
    let last = annotate_uploaded(
        service,
        "http://service.invalid",
        "token",
        &fx.queue,
        &fx.image,
        &setup(solvers, fx),
        &mut |annotation| seen.push(annotation.state),
    );
    (last, seen)
}

#[cfg(unix)]
fn fake_astap(dir: &Path, solves: bool) -> Solvers {
    use crate::external_process::fake_tool;
    use crate::sky_solution::SolverTool;
    let cards = dir.join("answer.wcs");
    std::fs::write(&cards, solved_header(2000, 1500)).unwrap();
    let copy = if solves {
        format!("cp '{}' \"$out.wcs\"\n", cards.display())
    } else {
        String::new()
    };
    let body = format!(
        "out=''\nwhile [ $# -gt 0 ]; do case \"$1\" in -o) out=\"$2\"; shift;; esac; shift; done\n{copy}exit {}",
        if solves { 0 } else { 1 }
    );
    Solvers {
        astap: Some(SolverTool {
            path: fake_tool(dir, "astap", &body),
            version: Some("2026.10".to_string()),
            database: None,
        }),
        astrometry_net: None,
    }
}

#[test]
fn a_progression_image_has_no_annotations_of_its_own() {
    assert!(!applies_to(ResultKind::Progression));
    for kind in [ResultKind::Final, ResultKind::Stacked, ResultKind::Plan] {
        assert!(applies_to(kind));
    }
}

#[test]
fn without_a_solver_the_row_says_so_and_nothing_is_sent() {
    let fx = fixture(ResultKind::Plan);
    let service = Service::with_missing("cap-plan", &fx.image.sha256);
    let (last, seen) = run(&service, &fx, &Solvers::default());
    assert_eq!(seen, vec![SkyState::Running, SkyState::Failed]);
    assert_eq!(last.state, SkyState::Failed);
    assert_eq!(last.failure, Some(SolveFailure::SolverMissing));
    assert!(last.retryable());
    assert!(service.put.borrow().is_empty());
    // Kept beside the queue row: the page reads it after a restart.
    let kept = fx.queue.sky_of(&fx.image.sha256).unwrap().unwrap();
    assert_eq!(kept.failure, Some(SolveFailure::SolverMissing));
    assert_eq!(kept.state, SkyState::Failed);
}

#[cfg(unix)]
#[test]
fn a_solver_that_finds_nothing_leaves_a_failure_with_its_reason() {
    let fx = fixture(ResultKind::Plan);
    let service = Service::with_missing("cap-plan", &fx.image.sha256);
    let solvers = fake_astap(fx.dir.path(), false);
    let (last, _) = run(&service, &fx, &solvers);
    assert_eq!(last.state, SkyState::Failed);
    assert!(last.failure.is_some(), "{last:?}");
    assert!(last.message.as_deref().is_some_and(|m| !m.is_empty()));
    assert!(service.put.borrow().is_empty());
    assert_eq!(
        fx.queue.sky_of(&fx.image.sha256).unwrap().unwrap().failure,
        last.failure
    );
}

#[cfg(unix)]
#[test]
fn the_image_just_sent_is_solved_from_the_file_here_and_its_solution_is_sent() {
    let fx = fixture(ResultKind::Plan);
    let service = Service::with_missing("cap-plan", &fx.image.sha256);
    let solvers = fake_astap(fx.dir.path(), true);
    let (last, seen) = run(&service, &fx, &solvers);
    assert_eq!(seen, vec![SkyState::Running, SkyState::Sent]);
    assert_eq!(last.state, SkyState::Sent);
    assert_eq!(last.source, Some(SolutionSource::Astap));
    assert!(last.elapsed_ms.is_some());
    assert_eq!(last.failure, None);
    let put = service.put.borrow();
    assert_eq!(put.len(), 1);
    assert_eq!(put[0].0, "cap-plan");
    assert_eq!(put[0].1.sha256, fx.image.sha256);
    // The record outlives the run, with its source.
    let kept = fx.queue.sky_of(&fx.image.sha256).unwrap().unwrap();
    assert_eq!(kept.state, SkyState::Sent);
    assert_eq!(kept.source, Some(SolutionSource::Astap));
    assert_eq!(kept.elapsed_ms, last.elapsed_ms);
    // A try again solves nothing more: the service has it.
    let after = Service {
        missing: Vec::new(),
        ..Service::with_missing("cap-plan", &fx.image.sha256)
    };
    let (again, _) = run(&after, &fx, &solvers);
    assert_eq!(again.state, SkyState::Already);
}

#[test]
fn a_header_the_image_carries_is_sent_without_any_solver() {
    let fx = fixture(ResultKind::Stacked);
    std::fs::write(
        &fx.image.path,
        samples::tiff(2000, 1500, Some(&solved_header(2000, 1500))),
    )
    .unwrap();
    let service = Service::with_missing("cap-plan", &fx.image.sha256);
    let (last, _) = run(&service, &fx, &Solvers::default());
    assert_eq!(last.state, SkyState::Sent);
    assert_eq!(last.source, Some(SolutionSource::Header));
    assert_eq!(service.put.borrow().len(), 1);
}

#[test]
fn an_image_the_night_wizard_already_solved_is_not_solved_twice() {
    let fx = fixture(ResultKind::Final);
    // The service lists another capture only: this one has its solution.
    let service = Service::with_missing("another-capture", &"cd".repeat(32));
    let (last, seen) = run(&service, &fx, &Solvers::default());
    assert_eq!(seen, vec![SkyState::Running, SkyState::Already]);
    assert_eq!(last.state, SkyState::Already);
    assert!(!last.retryable());
    assert!(service.put.borrow().is_empty());
    assert_eq!(*service.listed.borrow(), 1);
}

#[test]
fn a_service_that_cannot_be_reached_is_a_failure_to_try_again() {
    let fx = fixture(ResultKind::Plan);
    let mut service = Service::with_missing("cap-plan", &fx.image.sha256);
    service.down = true;
    let (last, _) = run(&service, &fx, &Solvers::default());
    assert_eq!(last.state, SkyState::Failed);
    assert_eq!(last.error_code.as_deref(), Some("network_error"));
    assert_eq!(last.failure, None);
    assert!(last.retryable());
}

#[test]
fn a_resolution_an_application_that_stopped_left_running_is_ended() {
    let fx = fixture(ResultKind::Plan);
    fx.queue
        .set_sky(&SkyAnnotation::new(
            &fx.image.sha256,
            "cap-plan",
            SkyState::Running,
            NOW,
        ))
        .unwrap();
    fx.queue
        .set_sky(&SkyAnnotation::new(
            &"cd".repeat(32),
            "cap-final",
            SkyState::Sent,
            NOW,
        ))
        .unwrap();
    assert_eq!(fx.queue.end_interrupted_sky().unwrap(), 1);
    let ended = fx.queue.sky_of(&fx.image.sha256).unwrap().unwrap();
    assert_eq!(ended.state, SkyState::Failed);
    assert_eq!(ended.error_code.as_deref(), Some(INTERRUPTED));
    assert_eq!(
        fx.queue.sky_of(&"cd".repeat(32)).unwrap().unwrap().state,
        SkyState::Sent
    );
}

#[test]
fn the_annotations_go_over_the_wire_in_camel_case() {
    let annotation = SkyAnnotation {
        failure: Some(SolveFailure::Timeout { seconds: 180 }),
        source: Some(SolutionSource::AstrometryNet),
        elapsed_ms: Some(4000),
        ..SkyAnnotation::new("ab", "cap", SkyState::Failed, NOW)
    };
    let json = serde_json::to_value(&annotation).unwrap();
    assert_eq!(json["state"], "failed");
    assert_eq!(json["captureId"], "cap");
    assert_eq!(json["failure"]["code"], "timeout");
    assert_eq!(json["failure"]["seconds"], 180);
    assert_eq!(json["source"], "astrometry-net");
    assert_eq!(json["elapsedMs"], 4000);
    assert!(json.get("errorCode").is_none());
    let _: MissingCapture = serde_json::from_value(serde_json::json!({
        "id": "c", "originalName": "x.tif", "sha256": "ab", "byteSize": 1
    }))
    .unwrap();
}

/// A plan image as Astro Sync writes it (a Deflate TIFF) is given to ASTAP as
/// an uncompressed FITS with the target's position; a field that matches
/// nothing says so in its own words (not "Field outside the installed
/// indexes"), and "Try again" runs the same path again, which sends the
/// solution once ASTAP finds it.
#[cfg(unix)]
#[test]
fn a_deflate_plan_image_is_solved_from_a_fits_and_a_retry_runs_it_again() {
    use crate::external_process::fake_tool;
    use crate::sky_solution::SolverTool;
    let fx = fixture(ResultKind::Plan);
    let (width, height) = (64_usize, 40_usize);
    let pixels: Vec<u16> = (0..width * height * 3)
        .map(|i| 3000 + ((i * 53) % 300) as u16)
        .collect();
    std::fs::write(
        &fx.image.path,
        crate::sky_solution::input::tests::tiff16(width, height, 3, &pixels, 32946, 2, 14),
    )
    .unwrap();
    let dir = fx.dir.path();
    let answer = dir.join("answer.wcs");
    std::fs::write(&answer, solved_header(64, 40)).unwrap();
    let body = format!(
        "d='{dir}'\necho \"$@\" >> \"$d/calls.txt\"\nout=''; f=''\nwhile [ $# -gt 0 ]; do case \"$1\" in -f) f=\"$2\"; shift;; -o) out=\"$2\"; shift;; esac; shift; done\nhead -c 30 \"$f\" > \"$d/head.txt\"\nif [ -f \"$d/solves\" ]; then echo PLTSOLVD=T > \"$out.ini\"; cp '{answer}' \"$out.wcs\"; exit 0; fi\necho PLTSOLVD=F > \"$out.ini\"\nexit 1",
        dir = dir.display(),
        answer = answer.display(),
    );
    let solvers = Solvers {
        astap: Some(SolverTool {
            path: fake_tool(dir, "astap", &body),
            version: Some("2026.09.01".to_string()),
            database: None,
        }),
        astrometry_net: None,
    };
    let service = Service::with_missing("cap-plan", &fx.image.sha256);
    let (last, _) = run(&service, &fx, &solvers);
    assert_eq!(last.state, SkyState::Failed);
    assert_eq!(last.failure, Some(SolveFailure::NotSolved));
    let message = last.message.clone().unwrap();
    assert!(!message.contains("index"), "{message}");
    assert!(last.retryable());
    let calls = std::fs::read_to_string(dir.join("calls.txt")).unwrap();
    assert_eq!(calls.lines().count(), 2, "{calls}");
    for call in calls.lines() {
        assert!(call.contains("input-"), "{call}");
        assert!(call.contains(".fit "), "{call}");
        // The target of the capture: 2.53 h, +61.4°.
        assert!(call.contains("-ra 2.530000 -spd 151.400000"), "{call}");
    }
    assert!(
        std::fs::read_to_string(dir.join("head.txt"))
            .unwrap()
            .starts_with("SIMPLE  =")
    );
    // "Try again": the same function, the same path; ASTAP solves now.
    std::fs::write(dir.join("solves"), "").unwrap();
    let (again, seen) = run(&service, &fx, &solvers);
    assert_eq!(seen, vec![SkyState::Running, SkyState::Sent]);
    assert_eq!(again.source, Some(SolutionSource::Astap));
    let put = service.put.borrow();
    assert_eq!(put.len(), 1);
    assert_eq!(
        (put[0].1.solution.width, put[0].1.solution.height),
        (64, 40)
    );
}

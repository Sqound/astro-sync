use super::*;
use crate::multi_night::project_of;
use crate::plan_image::{Band, Place, PlanDisplay, PlanField, PlanImager, PlanSource, band_of};
use crate::stacking::{Contrast, StackerOutput};
use crate::test_support::TempDir;
use std::cell::RefCell;

fn line(night: &str, evening: &str, filter: &str, frames: u64, seconds: f64) -> PlanLine {
    PlanLine {
        key: format!("{night}|{filter}"),
        night_id: night.to_string(),
        evening_date: evening.to_string(),
        filter: Some(filter.to_string()),
        band: band_of(Some(filter)),
        frame_count: frames,
        exposure_seconds: Some(seconds / frames as f64),
        total_seconds: seconds,
        place: Place::ThisComputer,
        computer_label: None,
        master: Some(PathBuf::from(format!("{night}.fit"))),
        bytes: 1,
        source: PlanSource::Master,
        device: None,
    }
}

fn project() -> NightProject {
    project_of(
        &serde_json::json!({
            "id": "project-1",
            "target": { "targetId": "sharpless-103", "name": "Sh2-103" },
            "integrations": []
        }),
        "sharpless-103",
    )
    .unwrap()
}

/// The three nights of the Veil: a dual-band one, a colour one, a dual-band one.
fn veil() -> Vec<PlanLine> {
    vec![
        line("n-12", "2026-08-12", "Dualband", 100, 18_000.0),
        line("n-07", "2026-08-07", "Dualband", 120, 21_600.0),
        line("n-10", "2026-08-10", "RGB", 89, 16_020.0),
    ]
}

fn keys(lines: &[PlanLine]) -> Vec<String> {
    lines.iter().map(|l| l.key.clone()).collect()
}

#[test]
fn the_milestones_are_the_plan_after_each_night_in_the_order_of_the_evenings() {
    let dir = TempDir::new("plan-progression-jobs");
    let lines = veil();
    let jobs = plan_progression_jobs(
        &project(),
        &lines,
        &keys(&lines),
        &dir.path().join("Veil · p"),
        &PipelineChoice::new("raw"),
        &PlanMaking::default(),
    );
    assert_eq!(jobs.len(), 3);
    let steps: Vec<_> = jobs
        .iter()
        .map(|j| (j.step.evening_date.as_str(), j.step.nights, j.step.frames))
        .collect();
    assert_eq!(
        steps,
        vec![
            ("2026-08-07", 1, 120),
            ("2026-08-10", 2, 209),
            ("2026-08-12", 3, 309)
        ]
    );
    // Each milestone combines the nights so far and nothing after.
    let nights: Vec<Vec<String>> = jobs.iter().map(|j| j.job.night_ids()).collect();
    assert_eq!(nights[0], vec!["n-07"]);
    assert_eq!(nights[1], vec!["n-07", "n-10"]);
    assert_eq!(nights[2], vec!["n-07", "n-10", "n-12"]);
    // One pipeline for all, a JPEG each, the scratch away from stacked/.
    assert!(
        jobs.iter()
            .all(|j| j.job.jpeg && j.job.pipeline.pipeline_id == "raw")
    );
    assert!(jobs.iter().all(|j| !j.job.out.ends_with("stacked")));
    assert_eq!(
        jobs[1].job.band_of_masters(),
        vec![Band::Dualband, Band::Broadband]
    );
}

#[test]
fn a_milestone_leaves_out_what_the_person_left_out() {
    let dir = TempDir::new("plan-progression-left-out");
    let lines = veil();
    let chosen: Vec<String> = keys(&lines)
        .into_iter()
        .filter(|k| k != "n-10|RGB")
        .collect();
    let jobs = plan_progression_jobs(
        &project(),
        &lines,
        &chosen,
        &dir.path().join("Veil · p"),
        &PipelineChoice::new("raw"),
        &PlanMaking::default(),
    );
    assert_eq!(
        jobs.iter().map(|j| j.step.frames).collect::<Vec<_>>(),
        vec![120, 220]
    );
}

#[test]
fn a_single_night_has_no_progression_of_a_plan() {
    let dir = TempDir::new("plan-progression-one");
    let lines = vec![line("n-07", "2026-08-07", "Dualband", 120, 21_600.0)];
    assert!(
        plan_progression_jobs(
            &project(),
            &lines,
            &keys(&lines),
            dir.path(),
            &PipelineChoice::new("raw"),
            &PlanMaking::default()
        )
        .is_empty()
    );
}

/// Writes what `plan_stack.py` writes for a job that asks a JPEG.
struct FakeImager {
    made: RefCell<Vec<Vec<String>>>,
    fail_at: Option<usize>,
    /// The jobs as they were given, in the order they were made.
    seen: RefCell<Vec<PlanImageJob>>,
    /// Whether the record carries a display (the script of a render that has none does not).
    records_display: bool,
}

/// The display a job's image is shown with: the one given, else its own, which depends on its nights
/// (more nights, less noise, a black point nearer the sky).
fn display_shown(job: &PlanImageJob) -> serde_json::Value {
    job.display.map_or_else(
        || {
            let nights = job.night_ids().len() as f64;
            serde_json::json!({"black": [0.03 / nights, 0.031 / nights, 0.032 / nights], "scale": 0.9 - 0.1 * nights, "imposed": false})
        },
        |d| serde_json::json!({"black": d.black, "scale": d.scale, "imposed": true}),
    )
}

impl PlanImager for FakeImager {
    fn make(
        &self,
        job: &PlanImageJob,
        _: &CancelFlag,
        _: &mut dyn FnMut(&StackEvent),
    ) -> Result<Vec<StackerOutput>, StackFailure> {
        self.made.borrow_mut().push(job.night_ids());
        self.seen.borrow_mut().push(job.clone());
        if self.fail_at == Some(self.made.borrow().len()) {
            return Err(StackFailure::Failed {
                code: "combination_failed".into(),
                message: "no".into(),
                logs: None,
            });
        }
        fs::create_dir_all(&job.work).unwrap();
        let mut written = Vec::new();
        for ext in ["tif", "png", "jpg"] {
            let file = format!("{}_{}.{ext}", job.name, job.pipeline.pipeline_id);
            fs::write(
                job.out.join(&file),
                format!("{ext} of {} nights", job.night_ids().len()),
            )
            .unwrap();
            written.push(StackerOutput {
                pipeline: job.pipeline.pipeline_id.clone(),
                file,
                contrast: Contrast::None,
                retouch: crate::stacking::Retouch::default(),
            });
        }
        // The rectangle the script cut and the master whose frame it is on: the one given, else its own.
        let crop = job.crop.map_or_else(
            || serde_json::json!({"applied": true, "reason": "commonArea", "top": 444, "left": 419, "height": 2095,
                                  "width": 2137, "frameHeight": 2977, "frameWidth": 2995}),
            |c| serde_json::to_value(c).unwrap(),
        );
        let master = job.frame_master.clone().map_or_else(
            || serde_json::json!({"path": job.masters[job.reference].path, "nightId": job.masters[job.reference].night_id}),
            |m| serde_json::to_value(m).unwrap(),
        );
        fs::write(
            job.out.join(crate::plan_image::record_file_name(&job.name)),
            if self.records_display {
                serde_json::json!({"crop": crop, "frameMaster": master, "display": display_shown(job)})
            } else {
                serde_json::json!({"crop": crop, "frameMaster": master})
            }
            .to_string(),
        )
        .unwrap();
        Ok(written)
    }
}

const OPEN: PlanImageGate = PlanImageGate {
    allowed: true,
    tool_present: true,
    image_quality: false,
};

#[test]
fn the_milestones_are_written_in_the_progression_folder_and_the_scratch_goes() {
    let dir = TempDir::new("plan-progression-run");
    let plan_dir = dir.path().join("Veil · p");
    let lines = veil();
    let jobs = plan_progression_jobs(
        &project(),
        &lines,
        &keys(&lines),
        &plan_dir,
        &PipelineChoice::new("raw"),
        &PlanMaking::default(),
    );
    // A milestone of an earlier run is replaced; a file of the person stays.
    let folder = plan_dir.join("progression");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("Sh2-103_plan_night-01_050-frames.jpg"), "old").unwrap();
    fs::write(folder.join("mine.jpg"), "mine").unwrap();

    let imager = FakeImager {
        made: RefCell::default(),
        fail_at: None,
        seen: RefCell::default(),
        records_display: true,
    };
    let run = run_plan_progression(
        &imager,
        &jobs,
        &plan_dir,
        OPEN,
        &CancelFlag::new(),
        &mut |_| {},
    );
    assert!(run.failure.is_none(), "{:?}", run.failure);
    assert_eq!(imager.made.borrow().len(), 3);
    let names: Vec<String> = run
        .steps
        .iter()
        .map(|s| s.jpeg.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        vec![
            "Sh2-103_plan_night-01_120-frames.jpg",
            "Sh2-103_plan_night-02_209-frames.jpg",
            "Sh2-103_plan_night-03_309-frames.jpg"
        ]
    );
    assert!(!folder.join("Sh2-103_plan_night-01_050-frames.jpg").exists());
    assert_eq!(fs::read_to_string(folder.join("mine.jpg")).unwrap(), "mine");
    assert_eq!(
        fs::read_to_string(&run.steps[2].jpeg).unwrap(),
        "jpg of 3 nights"
    );
    assert!(run.steps.iter().all(|s| s.size > 0 && s.sha256.len() == 64));
    // Nothing is left of the work: the plan's folder holds the milestones only.
    assert!(!plan_dir.join(".astro-sync-work").exists());
    assert!(!plan_dir.join("stacked").exists());
}

#[test]
fn a_milestone_that_fails_stops_the_next_and_keeps_the_ones_made() {
    let dir = TempDir::new("plan-progression-stop");
    let plan_dir = dir.path().join("Veil · p");
    let lines = veil();
    let jobs = plan_progression_jobs(
        &project(),
        &lines,
        &keys(&lines),
        &plan_dir,
        &PipelineChoice::new("raw"),
        &PlanMaking::default(),
    );
    let imager = FakeImager {
        made: RefCell::default(),
        fail_at: Some(2),
        seen: RefCell::default(),
        records_display: true,
    };
    let run = run_plan_progression(
        &imager,
        &jobs,
        &plan_dir,
        OPEN,
        &CancelFlag::new(),
        &mut |_| {},
    );
    assert_eq!(run.steps.len(), 1);
    assert!(run.failure.is_some());
    assert!(run.steps[0].jpeg.is_file());
    assert_eq!(imager.made.borrow().len(), 2, "the third is not tried");
}

#[test]
fn the_last_milestone_is_made_first_and_gives_its_rectangle_and_frame_to_the_others() {
    let dir = TempDir::new("plan-progression-crop");
    let plan_dir = dir.path().join("Veil · p");
    let lines = veil();
    let jobs = plan_progression_jobs(
        &project(),
        &lines,
        &keys(&lines),
        &plan_dir,
        &PipelineChoice::new("raw"),
        &PlanMaking::default(),
    );
    let imager = FakeImager {
        made: RefCell::default(),
        fail_at: None,
        seen: RefCell::default(),
        records_display: true,
    };
    let run = run_plan_progression(
        &imager,
        &jobs,
        &plan_dir,
        OPEN,
        &CancelFlag::new(),
        &mut |_| {},
    );
    assert!(run.failure.is_none(), "{:?}", run.failure);
    let seen = imager.seen.borrow();
    // Every night first (the image of the plan), then the milestones in their order.
    let nights: Vec<usize> = seen.iter().map(|j| j.night_ids().len()).collect();
    assert_eq!(nights, vec![3, 1, 2]);
    assert!(seen[0].crop.is_none() && seen[0].frame_master.is_none());
    let rectangle = PlanCrop {
        top: 444,
        left: 419,
        height: 2095,
        width: 2137,
        frame_height: 2977,
        frame_width: 2995,
    };
    for milestone in &seen[1..] {
        assert_eq!(milestone.crop, Some(rectangle));
        let master = milestone.frame_master.as_ref().unwrap();
        // The frame of the image of the plan: its longest dual-band night's.
        assert_eq!(master.path, PathBuf::from("n-07.fit"));
        assert_eq!(master.night_id.as_deref(), Some("n-07"));
        assert_eq!(milestone.plan_field, PlanField::Clean);
    }
    // The steps come back in the order of the nights.
    let made: Vec<usize> = run.steps.iter().map(|s| s.nights).collect();
    assert_eq!(made, vec![1, 2, 3]);

    // « Zone commune à toutes les nuits »: every milestone is cut the same way.
    let making = PlanMaking {
        plan_field: PlanField::Common,
        ..PlanMaking::default()
    };
    let jobs = plan_progression_jobs(
        &project(),
        &lines,
        &keys(&lines),
        &plan_dir,
        &PipelineChoice::new("raw"),
        &making,
    );
    assert!(jobs.iter().all(|j| j.job.plan_field == PlanField::Common));
}

#[test]
fn the_milestones_leave_as_the_history_of_the_capture_of_the_plan() {
    let dir = TempDir::new("plan-progression-results");
    let plan_dir = dir.path().join("Veil · p");
    let lines = veil();
    let jobs = plan_progression_jobs(
        &project(),
        &lines,
        &keys(&lines),
        &plan_dir,
        &PipelineChoice::new("raw"),
        &PlanMaking::default(),
    );
    let imager = FakeImager {
        made: RefCell::default(),
        fail_at: None,
        seen: RefCell::default(),
        records_display: true,
    };
    let run = run_plan_progression(
        &imager,
        &jobs,
        &plan_dir,
        OPEN,
        &CancelFlag::new(),
        &mut |_| {},
    );
    let results = plan_progression_results(&run.steps, "sharpless-103", "capture-9");
    assert_eq!(results.len(), 3);
    for (result, frames) in results.iter().zip([120, 209, 309]) {
        assert_eq!(result.kind, ResultKind::Progression);
        assert_eq!(result.target.variant_of.as_deref(), Some("capture-9"));
        assert_eq!(result.target.frame_count, Some(frames));
        assert_eq!(result.target.session_id, None);
        assert!(result.target.plan.is_none());
    }
    // The queue takes them as progression images, by the folder they are in.
    let queue = crate::upload_queue::UploadQueue::open(&dir.path().join("queue.sqlite")).unwrap();
    queue.enqueue_results(&results).unwrap();
    let rows = queue.list().unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|r| r.kind == ResultKind::Progression));
    assert!(
        crate::upload_queue::result_kind_of(&results[0].path)
            .is_ok_and(|kind| kind == ResultKind::Progression)
    );
}

#[test]
fn the_last_milestone_gives_its_display_to_the_others() {
    let dir = TempDir::new("plan-progression-display");
    let plan_dir = dir.path().join("Veil · p");
    let lines = veil();
    let jobs = plan_progression_jobs(
        &project(),
        &lines,
        &keys(&lines),
        &plan_dir,
        &PipelineChoice::new("raw"),
        &PlanMaking::default(),
    );
    assert!(jobs.iter().all(|j| j.job.display.is_none()));
    let imager = FakeImager {
        made: RefCell::default(),
        fail_at: None,
        seen: RefCell::default(),
        records_display: true,
    };
    let run = run_plan_progression(
        &imager,
        &jobs,
        &plan_dir,
        OPEN,
        &CancelFlag::new(),
        &mut |_| {},
    );
    assert!(run.failure.is_none(), "{:?}", run.failure);
    let seen = imager.seen.borrow();
    // The image of the plan measures its own; the others are shown with it.
    assert_eq!(seen[0].night_ids().len(), 3);
    assert!(seen[0].display.is_none());
    let plan_display = PlanDisplay {
        black: [0.01, 0.031 / 3.0, 0.032 / 3.0],
        scale: 0.9 - 0.3,
        sky: None,
        final_with_stars: None,
    };
    for milestone in &seen[1..] {
        let given = milestone
            .display
            .expect("the display of the image of the plan");
        assert!((given.scale - plan_display.scale).abs() < 1e-12);
        assert!((given.black[0] - plan_display.black[0]).abs() < 1e-12);
        assert!((given.black[2] - plan_display.black[2]).abs() < 1e-12);
    }
    // The job file says it as the script reads it.
    let json = serde_json::to_value(&seen[1]).unwrap();
    assert!((json["display"]["scale"].as_f64().unwrap() - plan_display.scale).abs() < 1e-12);
    assert_eq!(json["display"]["black"].as_array().unwrap().len(), 3);
    assert!(
        serde_json::to_value(&seen[0])
            .unwrap()
            .get("display")
            .is_none()
    );
}

#[test]
fn without_a_recorded_display_the_milestones_keep_their_own_and_the_record_says_so() {
    let dir = TempDir::new("plan-progression-no-display");
    let plan_dir = dir.path().join("Veil · p");
    let lines = veil();
    let jobs = plan_progression_jobs(
        &project(),
        &lines,
        &keys(&lines),
        &plan_dir,
        &PipelineChoice::new("raw"),
        &PlanMaking::default(),
    );
    let imager = FakeImager {
        made: RefCell::default(),
        fail_at: None,
        seen: RefCell::default(),
        records_display: false,
    };
    let run = run_plan_progression(
        &imager,
        &jobs,
        &plan_dir,
        OPEN,
        &CancelFlag::new(),
        &mut |_| {},
    );
    assert!(run.failure.is_none(), "{:?}", run.failure);
    let seen = imager.seen.borrow();
    assert_eq!(seen.len(), 3);
    assert!(seen.iter().all(|j| j.display.is_none()));
    // The rectangle and the frame are still shared: only the display is missing.
    assert!(
        seen[1..]
            .iter()
            .all(|j| j.crop.is_some() && j.frame_master.is_some())
    );
    // What a milestone made without it records (the script's `imposed`): its own measure.
    assert_eq!(display_shown(&seen[1])["imposed"], false);
}

//! The progression of a multi-night plan: the image of the plan after each
//! night, so the person sees the target grow from one night to the next
//! (the history of the capture, never a capture of its own).
//!
//! The nights of the plan are taken in the order of their evenings. The first
//! milestone is the first night alone; the next one combines the first two
//! nights, and so on up to the image of the plan itself. Each milestone is the
//! image [`crate::plan_image`] makes of those nights -- the same alignment,
//! level match and render as the plan's image, with **one** pipeline (the main
//! image's), so the milestones read as one sequence -- written as a JPEG in
//! `<plan>/progression/` (the folder name makes it a
//! [`ResultKind::Progression`]) with the poses combined so far. The
//! milestones of an earlier run are replaced.
//!
//! **One frame, one rectangle**: the image of the plan is cut to the area
//! every night covers, on the frame of its master (`plan_crop.py`). The last
//! milestone (every night: the image of the plan itself) is made first; its
//! record gives the frame master and the rectangle, and every other milestone
//! is made with them ([`PlanImageJob::crop`], [`PlanImageJob::frame_master`]):
//! registered on that master when its own nights give it another frame, then
//! cut the same way, so that the milestones line up with one another and with
//! the image of the plan.
//!
//! **One level**: the display of an image is two measures, the black point of
//! each channel and the scale of the stretch, and a stack of one night is
//! noisier than a stack of three: shown with its own measures its sky would
//! look brighter. The record of the image of the plan gives them too
//! ([`recorded_display`]) and every other milestone is shown with them
//! ([`PlanImageJob::display`]); when the image of the plan could not record
//! them the milestones keep their own measures, and their records say so
//! (`imposed: false`).
//!
//! What leaves ([`plan_progression_results`]): each JPEG as a progression of
//! the capture of the plan (an existing capture, or the one another image of
//! the same sending creates), with its frames: the service shows them under
//! the capture, never among its variants.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::local_work::CancelFlag;
use crate::multi_night::NightProject;
use crate::plan_image::{
    PlanCrop, PlanDisplay, PlanFrameMaster, PlanImageGate, PlanImageJob, PlanImagePlan, PlanLine,
    PlanMaking, plan_image_job_of, recorded_crop, recorded_display, run_plan_image,
};
use crate::stacking::{PipelineChoice, StackEvent, StackFailure};
use crate::upload_queue::{EntryTarget, ResultFile, ResultKind};
use crate::working_folder::{PROGRESSION_DIR, WORK_DIR};

/// The scratch of the milestones: `<plan>/.astro-sync-work/plan-progression/`.
pub const PLAN_PROGRESSION_SCRATCH_DIR: &str = "plan-progression";

/// One milestone of the progression of a plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanProgressionStep {
    /// The night this milestone ends with.
    pub night_id: String,
    pub evening_date: String,
    /// The nights combined so far (this one included).
    pub nights: usize,
    /// The poses combined so far.
    pub frames: u64,
    /// The integration time combined so far, seconds.
    pub seconds: f64,
    /// `<plan>/progression/<name>_night-NN_NNN-frames.jpg`; empty until made.
    pub jpeg: PathBuf,
    pub sha256: String,
    pub size: u64,
}

/// A milestone to make: what it will say, and the image that makes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanProgressionJob {
    pub step: PlanProgressionStep,
    pub job: Box<PlanImageJob>,
}

/// The milestones of the plan made from the lines `chosen`, with `pipeline`
/// for every one: one per night, the last being the image of the plan itself.
/// Fewer than two nights: none (a single night has the progression of its own
/// poses).
#[must_use]
pub fn plan_progression_jobs(
    project: &NightProject,
    lines: &[PlanLine],
    chosen: &[String],
    plan_dir: &Path,
    pipeline: &PipelineChoice,
    making: &PlanMaking,
) -> Vec<PlanProgressionJob> {
    let picked: Vec<&PlanLine> = lines
        .iter()
        .filter(|l| chosen.contains(&l.key) && l.combinable())
        .collect();
    let mut nights: Vec<(&str, &str)> = picked
        .iter()
        .map(|l| (l.evening_date.as_str(), l.night_id.as_str()))
        .collect();
    nights.sort_unstable();
    nights.dedup();
    if nights.len() < 2 {
        return Vec::new();
    }
    let scratch = plan_dir.join(WORK_DIR).join(PLAN_PROGRESSION_SCRATCH_DIR);
    let mut jobs = Vec::new();
    for (index, (evening, night_id)) in nights.iter().enumerate() {
        let so_far = &nights[..=index];
        let of_these: Vec<&&PlanLine> = picked
            .iter()
            .filter(|l| so_far.iter().any(|(_, id)| *id == l.night_id.as_str()))
            .collect();
        let keys: Vec<String> = of_these.iter().map(|l| l.key.clone()).collect();
        let PlanImagePlan::Job(mut job) = plan_image_job_of(
            project,
            lines,
            &keys,
            plan_dir,
            Some(pipeline.clone()),
            making,
            true,
        ) else {
            continue;
        };
        job.out = scratch.join(format!("step-{:02}", index + 1));
        job.jpeg = true;
        jobs.push(PlanProgressionJob {
            step: PlanProgressionStep {
                night_id: (*night_id).to_string(),
                evening_date: (*evening).to_string(),
                nights: index + 1,
                frames: of_these.iter().map(|l| l.frame_count).sum(),
                seconds: of_these.iter().map(|l| l.total_seconds).sum(),
                jpeg: PathBuf::new(),
                sha256: String::new(),
                size: 0,
            },
            job,
        });
    }
    jobs
}

/// The name of a milestone's JPEG: the plan's stem, the night's rank and the
/// poses combined (`IC_1805_plan_night-02_187-frames.jpg`).
fn milestone_name(stem: &str, step: &PlanProgressionStep) -> String {
    format!(
        "{stem}_night-{:02}_{:03}-frames.jpg",
        step.nights, step.frames
    )
}

/// A JPEG of an earlier run of this progression (Astro Sync's own file).
fn is_milestone(name: &str, stem: &str) -> bool {
    name.strip_prefix(stem)
        .and_then(|rest| rest.strip_prefix("_night-"))
        .is_some_and(|rest| rest.ends_with("-frames.jpg"))
}

/// What [`run_plan_progression`] made: every milestone made, and why the next
/// one was not.
#[derive(Debug)]
pub struct PlanProgressionRun {
    pub steps: Vec<PlanProgressionStep>,
    pub failure: Option<StackFailure>,
}

/// Makes the milestones, each into `<plan>/progression/`: the last one (every
/// night) first, whose frame master, rectangle and display the others take (see the
/// module documentation), then the others in their order; stops at the first
/// that fails (the ones made stay made). The steps come back in the order of
/// the nights. The scratch is gone when this returns.
pub fn run_plan_progression(
    imager: &dyn crate::plan_image::PlanImager,
    jobs: &[PlanProgressionJob],
    plan_dir: &Path,
    gate: PlanImageGate,
    cancel: &CancelFlag,
    on_event: &mut dyn FnMut(&StackEvent),
) -> PlanProgressionRun {
    let mut run = PlanProgressionRun {
        steps: Vec::new(),
        failure: None,
    };
    let Some(first) = jobs.first() else {
        return run;
    };
    let failed = |code: &str, message: String| StackFailure::Failed {
        code: code.to_string(),
        message,
        logs: None,
    };
    let folder = plan_dir.join(PROGRESSION_DIR);
    let stem = first.job.name.clone();
    if let Err(error) = fs::create_dir_all(&folder) {
        run.failure = Some(failed("io_error", error.to_string()));
        return run;
    }
    // The milestones of an earlier run go: this run replaces them.
    if let Ok(entries) = fs::read_dir(&folder) {
        for entry in entries.filter_map(Result::ok) {
            if is_milestone(&entry.file_name().to_string_lossy(), &stem) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    // The last milestone first: the rectangle and the frame of the image of the plan.
    let mut order: Vec<PlanProgressionJob> = Vec::with_capacity(jobs.len());
    order.extend(jobs.last().cloned());
    order.extend(jobs[..jobs.len() - 1].iter().cloned());
    let mut framing: Option<(PlanCrop, PlanFrameMaster)> = None;
    let mut display: Option<PlanDisplay> = None;
    for (rank, mut planned) in order.into_iter().enumerate() {
        if rank > 0
            && let Some((crop, master)) = &framing
        {
            planned.job.crop = Some(*crop);
            planned.job.frame_master = Some(master.clone());
        }
        if rank > 0 {
            planned.job.display = display;
        }
        let outcome = run_plan_image(imager, &planned.job, gate, cancel, on_event);
        if rank == 0 {
            framing = recorded_crop(&planned.job);
            display = recorded_display(&planned.job);
        }
        let made = outcome.and_then(|outcome| {
            let jpeg = outcome
                .other_files
                .iter()
                .find(|file| {
                    file.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| e.eq_ignore_ascii_case("jpg"))
                })
                .cloned()
                .ok_or_else(|| failed("no_output", "the milestone was not written".into()))?;
            let target = folder.join(milestone_name(&stem, &planned.step));
            fs::copy(&jpeg, &target).map_err(|e| failed("io_error", e.to_string()))?;
            let (sha256, size) = crate::stacking::sha256_and_size(&target)
                .map_err(|e| failed("io_error", e.to_string()))?;
            Ok(PlanProgressionStep {
                jpeg: target,
                sha256,
                size,
                ..planned.step.clone()
            })
        });
        let _ = fs::remove_dir_all(&planned.job.out);
        match made {
            Ok(step) => run.steps.push(step),
            Err(failure) => {
                run.failure = Some(failure);
                break;
            }
        }
    }
    run.steps.sort_by_key(|step| step.nights);
    let scratch = plan_dir.join(WORK_DIR);
    let _ = fs::remove_dir(scratch.join(PLAN_PROGRESSION_SCRATCH_DIR));
    let _ = fs::remove_dir(scratch);
    run
}

/// The milestones as results to enqueue: each a progression of the capture
/// `variant_of` (the capture's identifier, or `sha256:<digest>` of the image
/// of the same sending that creates it), with the poses it combines.
#[must_use]
pub fn plan_progression_results(
    steps: &[PlanProgressionStep],
    target_id: &str,
    variant_of: &str,
) -> Vec<ResultFile> {
    steps
        .iter()
        .map(|step| ResultFile {
            path: step.jpeg.clone(),
            sha256: step.sha256.clone(),
            size: step.size,
            kind: ResultKind::Progression,
            target: EntryTarget {
                session_id: None,
                target_ids: vec![target_id.to_string()],
                variant_of: Some(variant_of.to_string()),
                source: Some("stacking".to_string()),
                metadata: None,
                frame_count: Some(step.frames),
                plan: None,
            },
        })
        .collect()
}

#[cfg(test)]
mod tests;

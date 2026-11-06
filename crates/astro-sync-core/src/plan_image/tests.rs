use super::*;
use crate::multi_night::project_of;
use crate::multi_night::tests::{heart, write_device_image, write_master};
use crate::test_support::TempDir;
use crate::upload_queue::UploadQueue;
use crate::uploader::{UploadTarget, build_metadata};
use crate::working_folder::{NIGHT_IDENTITY_FILE, TARGET_IDENTITY_FILE};
use std::cell::Cell;

/// A night folder of the working folder, with its identity and the plan's
/// target (its own identity).
fn night_folder(root: &Path, name: &str, night_id: &str) -> PathBuf {
    let night = root.join(name);
    let target = night.join("IC 1805");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        night.join(NIGHT_IDENTITY_FILE),
        format!(
            r#"{{"format":1,"nightId":"{night_id}","serviceUrl":"https://s","createdAt":"2026-08-09T10:00:00Z"}}"#
        ),
    )
    .unwrap();
    fs::write(
        target.join(TARGET_IDENTITY_FILE),
        r#"{"format":1,"targetId":"ic-1805"}"#,
    )
    .unwrap();
    night
}

/// The Heart on this computer: the 9th August (dual-band) and the 12th (the
/// night being sent, UV/IR-cut); the 10th (RGB) was sent from the laptop.
fn heart_here(root: &Path) -> (NightProject, CurrentNight) {
    let mut project = heart();
    project["integrations"][2]["computerLabel"] = "Portable".into();
    let project = project_of(&project, "ic-1805").unwrap();
    let ninth = night_folder(root, "2026-08-09 · Cœur · n-09", "n-09");
    write_master(
        &ninth.join("IC 1805/stacked"),
        "L-eXtreme_180s_g100",
        Some("L-eXtreme"),
        180.0,
        98,
    );
    let twelfth = night_folder(root, "2026-08-12 · Cœur UV-IR · n-12", "n-12");
    write_master(
        &twelfth.join("IC 1805/stacked"),
        "UV-IR-cut_120s_g100",
        Some("UV/IR-cut"),
        120.0,
        120,
    );
    // A night of another plan: never read.
    night_folder(root, "2026-08-07 · M31 · n-07", "n-07");
    (
        project,
        CurrentNight {
            night_id: "n-12".into(),
            evening_date: "2026-08-12".into(),
            folder: twelfth,
        },
    )
}

#[test]
fn a_dual_band_filter_is_told_by_its_name() {
    for name in [
        "L-eXtreme",
        "L-Ultimate",
        "ALP-T",
        "NBZ",
        "Dual band",
        "Bibande",
        "Ha/OIII",
    ] {
        assert_eq!(band_of(Some(name)), Band::Dualband, "{name}");
    }
    for name in ["UV/IR-cut", "RGB", "L-Pro", "Lum", "Ha"] {
        assert_eq!(band_of(Some(name)), Band::Broadband, "{name}");
    }
    assert_eq!(band_of(None), Band::Broadband);
}

#[test]
fn the_plan_lists_each_integration_here_or_on_another_computer() {
    let dir = TempDir::new("plan-image-lines");
    let (project, current) = heart_here(dir.path());
    let lines = plan_lines(dir.path(), &project, Some(&current));
    let summary: Vec<(&str, Option<&str>, Place, bool)> = lines
        .iter()
        .map(|l| {
            (
                l.evening_date.as_str(),
                l.filter.as_deref(),
                l.place,
                l.master.is_some(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            ("2026-08-09", Some("L-eXtreme"), Place::ThisComputer, true),
            ("2026-08-10", Some("RGB"), Place::OtherComputer, false),
            ("2026-08-12", Some("UV/IR-cut"), Place::ThisNight, true),
        ]
    );
    assert_eq!(lines[0].band, Band::Dualband);
    assert_eq!(lines[1].computer_label.as_deref(), Some("Portable"));
    assert_eq!(lines[0].key, "n-09|L-eXtreme_180s_g100");
    assert!(lines[2].bytes > 0);
    // The night being sent before its lines reached the service is listed.
    let mut fresh = project.clone();
    fresh.integrations.retain(|i| i.night_id != "n-12");
    let lines = plan_lines(dir.path(), &fresh, Some(&current));
    assert!(
        lines
            .iter()
            .any(|l| l.night_id == "n-12" && l.place == Place::ThisNight)
    );
}

#[test]
fn one_night_combined_is_the_image_of_that_night() {
    let dir = TempDir::new("plan-image-one-night");
    let (project, current) = heart_here(dir.path());
    let lines = plan_lines(dir.path(), &project, Some(&current));
    let plan_dir = dir.path().join("plan");
    // The 10th is on the laptop: ticking it combines nothing.
    let chosen = vec![lines[0].key.clone(), lines[1].key.clone()];
    assert_eq!(
        plan_image_job(&project, &lines, &chosen, &plan_dir, None),
        PlanImagePlan::OneNight {
            night_id: "n-09".into()
        }
    );
    assert_eq!(
        plan_image_job(&project, &lines, &[], &plan_dir, None),
        PlanImagePlan::Nothing
    );
}

#[test]
fn the_job_aligns_on_the_longest_night_and_names_its_files() {
    let dir = TempDir::new("plan-image-job");
    let (project, current) = heart_here(dir.path());
    let lines = plan_lines(dir.path(), &project, Some(&current));
    let plan_dir = plan_folder(dir.path(), &project).unwrap();
    assert_eq!(
        plan_dir.file_name().unwrap().to_string_lossy(),
        "IC 1805 · p-hear"
    );
    // Found again by its identity, whatever its name.
    let renamed = dir.path().join("Le Cœur, mon plan");
    fs::rename(&plan_dir, &renamed).unwrap();
    assert_eq!(plan_folder(dir.path(), &project).unwrap(), renamed);
    let chosen: Vec<String> = lines.iter().map(|l| l.key.clone()).collect();
    let PlanImagePlan::Job(job) = plan_image_job(
        &project,
        &lines,
        &chosen,
        &renamed,
        Some(PipelineChoice::new("dualband-correction")),
    ) else {
        panic!("a job")
    };
    assert_eq!(job.masters.len(), 2);
    // 4 h 54 of dual-band against 4 h 00 of UV/IR-cut: the plan mixes the two, so the
    // reference is the longest dual-band night, the 9th (its frame is the image's).
    assert_eq!(job.masters[job.reference].night_id, "n-09");
    assert_eq!(job.night_ids(), vec!["n-09", "n-12"]);
    assert_eq!(job.name, "IC_1805_plan");
    assert_eq!(job.out, renamed.join("stacked"));
    assert_eq!(job.work, renamed.join(".astro-sync-work/plan-image"));
    assert_eq!(
        job.job_file(),
        renamed.join(".astro-sync-work/plan-image.job.json")
    );
    let json = serde_json::to_value(&job).unwrap();
    assert_eq!(json["masters"][0]["band"], "dualband");
    assert_eq!(json["pipeline"]["pipelineId"], "dualband-correction");
}

/// A line of the plan that is only what the reference reads: its filter, its band, its time.
fn line_for(night: &str, evening: &str, filter: Option<&str>, seconds: f64) -> PlanLine {
    PlanLine {
        key: format!("{night}|{}", filter.unwrap_or("nofilter")),
        night_id: night.to_string(),
        evening_date: evening.to_string(),
        filter: filter.map(str::to_string),
        band: band_of(filter),
        frame_count: 10,
        exposure_seconds: Some(seconds / 10.0),
        total_seconds: seconds,
        place: Place::ThisComputer,
        computer_label: None,
        master: Some(PathBuf::from(format!("{night}.fit"))),
        bytes: 1,
        source: PlanSource::Master,
        device: None,
    }
}

fn night_of_reference(lines: &[PlanLine]) -> &str {
    let picked: Vec<&PlanLine> = lines.iter().collect();
    &lines[reference_of(&picked).unwrap()].night_id
}

#[test]
fn the_reference_is_the_longest_dual_band_night_when_the_plan_mixes_both() {
    // A broadband night is the longest of all: the dual-band one still gives the frame.
    let mixed = [
        line_for("n-1", "2026-10-01", Some("L-eXtreme"), 25_200.0),
        line_for("n-2", "2026-10-02", Some("UV/IR-cut"), 10_800.0),
        line_for("n-3", "2026-10-03", None, 34_400.0),
        line_for("n-4", "2026-10-04", Some("NBZ"), 28_800.0),
    ];
    assert_eq!(night_of_reference(&mixed), "n-4");
    // The order of the lines changes nothing.
    let mut reversed = mixed.clone();
    reversed.reverse();
    assert_eq!(night_of_reference(&reversed), "n-4");
}

#[test]
fn without_a_mix_the_reference_is_still_the_longest_night() {
    let broadband = [
        line_for("n-1", "2026-10-01", Some("UV/IR-cut"), 7_200.0),
        line_for("n-2", "2026-10-02", None, 9_000.0),
    ];
    assert_eq!(night_of_reference(&broadband), "n-2");
    let dual_band = [
        line_for("n-1", "2026-10-01", Some("L-eXtreme"), 9_000.0),
        line_for("n-2", "2026-10-02", Some("ALP-T"), 7_200.0),
    ];
    assert_eq!(night_of_reference(&dual_band), "n-1");
    // A tie goes to the earliest evening, mixed or not.
    let tie = [
        line_for("n-2", "2026-10-02", None, 3_600.0),
        line_for("n-1", "2026-10-01", None, 3_600.0),
    ];
    assert_eq!(night_of_reference(&tie), "n-1");
    let picked: Vec<&PlanLine> = Vec::new();
    assert_eq!(reference_of(&picked), None);
}

/// The fake maker of the tests: writes the render the job asks for.
struct FakeImager {
    calls: Cell<u32>,
}

impl PlanImager for FakeImager {
    fn make(
        &self,
        job: &PlanImageJob,
        _: &CancelFlag,
        on_event: &mut dyn FnMut(&StackEvent),
    ) -> Result<Vec<StackerOutput>, StackFailure> {
        self.calls.set(self.calls.get() + 1);
        fs::create_dir_all(&job.work).unwrap();
        fs::write(job.work.join("siril.log"), b"log").unwrap();
        for (index, step) in ["alignment", "combination", "render"].iter().enumerate() {
            on_event(&StackEvent::Step {
                step: (*step).to_string(),
                index: index as u32 + 1,
                total: 3,
            });
        }
        let mut written = Vec::new();
        // The record of the combination, with the quality `plan_stack.py`
        // read on the combined linear master.
        fs::write(
            job.out.join(record_file_name(&job.name)),
            serde_json::json!({
                "format": 1,
                "tools": {"siril": "siril 1.4.4"},
                "combination": {
                    "method": "weighted_mean",
                    "masters": [{"weight": 0.5517}, {"weight": 0.4483}],
                    "levelMatch": [{
                        "nightId": "n-09", "to": "n-12", "method": "aperture",
                        "scales": [1.01234, 0.99801, 1.0], "stars": 41, "apertureRadius": 11.504
                    }],
                    "noData": 0.0083
                },
                "quality": {
                    "fwhmPixels": 4.2, "eccentricity": 0.27, "starCount": 188,
                    "measuredOn": "linear", "measuredAt": "2026-08-12T23:10:00Z"
                }
            })
            .to_string(),
        )
        .unwrap();
        for ext in ["tif", "png"] {
            let file = format!("{}_{}.{ext}", job.name, job.pipeline.pipeline_id);
            fs::write(job.out.join(&file), ext).unwrap();
            written.push(StackerOutput {
                pipeline: job.pipeline.pipeline_id.clone(),
                file,
                contrast: crate::stacking::Contrast::None,
                retouch: Default::default(),
            });
        }
        if !job.contrast.is_none() || !job.retouch.is_none() {
            // The same image with the curve and the finish, written after the plain one.
            let file = format!(
                "{}_{}{}{}.tif",
                job.name,
                job.pipeline.pipeline_id,
                job.contrast.suffix(),
                job.retouch.suffix()
            );
            fs::write(job.out.join(&file), "curved").unwrap();
            written.push(StackerOutput {
                pipeline: job.pipeline.pipeline_id.clone(),
                file,
                contrast: job.contrast,
                retouch: job.retouch,
            });
        }
        Ok(written)
    }
}

/// The colour intensity and the softened stars of a plan's image are two more
/// words of the job file, written only when chosen, flattened beside the contrast.
#[test]
fn the_finish_of_a_plan_image_is_a_word_of_the_job_file_only_when_chosen() {
    let dir = TempDir::new("plan-image-finish");
    let (project, current) = heart_here(dir.path());
    let lines = plan_lines(dir.path(), &project, Some(&current));
    let plan_dir = plan_folder(dir.path(), &project).unwrap();
    let chosen: Vec<String> = lines.iter().map(|l| l.key.clone()).collect();
    let PlanImagePlan::Job(mut job) = plan_image_job(
        &project,
        &lines,
        &chosen,
        &plan_dir,
        Some(PipelineChoice::new("raw")),
    ) else {
        panic!("a job")
    };
    assert!(job.retouch.is_none());
    let written = serde_json::to_string(&job).unwrap();
    assert!(
        !written.contains("\"colour\"") && !written.contains("\"softStars\""),
        "the defaults write nothing in the job file: {written}"
    );
    job.retouch = crate::stacking::Retouch {
        colour: crate::stacking::Colour::Strong,
        soft_stars: crate::stacking::SoftStars::Light,
    };
    let written = serde_json::to_string(&job).unwrap();
    assert!(written.contains("\"colour\":40"), "{written}");
    assert!(written.contains("\"softStars\":50"), "{written}");
    assert_eq!(
        serde_json::from_str::<PlanImageJob>(&written).unwrap(),
        *job
    );
}

/// With a contrast the plan's image stays the plain one (the capture of
/// the plan), the image with the curve is kept beside it in `stacked/`.
#[test]
fn the_image_of_a_plan_with_a_contrast_stays_the_plain_one_and_keeps_the_curve_beside_it() {
    let dir = TempDir::new("plan-image-contrast");
    let (project, current) = heart_here(dir.path());
    let lines = plan_lines(dir.path(), &project, Some(&current));
    let plan_dir = plan_folder(dir.path(), &project).unwrap();
    let chosen: Vec<String> = lines.iter().map(|l| l.key.clone()).collect();
    let PlanImagePlan::Job(mut job) = plan_image_job(
        &project,
        &lines,
        &chosen,
        &plan_dir,
        Some(PipelineChoice::new("raw")),
    ) else {
        panic!("a job")
    };
    assert_eq!(job.contrast, crate::stacking::Contrast::None);
    assert!(
        !serde_json::to_string(&job)
            .unwrap()
            .contains("\"contrast\""),
        "the default writes nothing in the job file"
    );
    job.contrast = crate::stacking::Contrast::Strong;
    assert!(
        serde_json::to_string(&job)
            .unwrap()
            .contains("\"contrast\":\"strong\"")
    );
    let open = PlanImageGate {
        allowed: true,
        tool_present: true,
        image_quality: false,
    };
    let imager = FakeImager {
        calls: Cell::new(0),
    };
    let outcome = run_plan_image(&imager, &job, open, &CancelFlag::new(), &mut |_| {}).unwrap();
    assert_eq!(
        outcome.render.path,
        plan_dir.join("stacked/IC_1805_plan_raw.tif")
    );
    assert_eq!(outcome.render.contrast, crate::stacking::Contrast::None);
    assert_eq!(outcome.plan_render.as_deref(), Some("natural"));
    // The image with the curve is not set aside: it is an image of the same capture.
    assert_eq!(outcome.finishes.len(), 1);
    assert_eq!(
        outcome.finishes[0].render.path,
        plan_dir.join("stacked/IC_1805_plan_raw_strong.tif")
    );
    assert_eq!(
        outcome.finishes[0].render.contrast,
        crate::stacking::Contrast::Strong
    );
    assert!(
        !outcome
            .other_files
            .contains(&plan_dir.join("stacked/IC_1805_plan_raw_strong.tif"))
    );
    assert_eq!(plan_results(&outcome).len(), 2, "the capture and its curve");
}

/// The finishes (contrast, colours, softened stars) of a plan's image leave
/// with it: one capture, the render without a finish as its main image and
/// every finish an image of that capture, named after its treatment and
/// carrying its own steps -- never the finish on the main image's steps.
#[test]
fn the_finishes_of_a_plan_image_leave_as_images_of_the_one_capture_named_after_their_treatment() {
    let dir = TempDir::new("plan-image-finishes");
    let (project, current) = heart_here(dir.path());
    let lines = plan_lines(dir.path(), &project, Some(&current));
    let plan_dir = plan_folder(dir.path(), &project).unwrap();
    let chosen: Vec<String> = lines.iter().map(|l| l.key.clone()).collect();
    let PlanImagePlan::Job(mut job) = plan_image_job(
        &project,
        &lines,
        &chosen,
        &plan_dir,
        Some(PipelineChoice::new("raw")),
    ) else {
        panic!("a job")
    };
    job.contrast = crate::stacking::Contrast::Strong;
    job.retouch = crate::stacking::Retouch {
        colour: crate::stacking::Colour::Strong,
        soft_stars: crate::stacking::SoftStars::Strong,
    };
    job.render_texts = (
        Some(GeneratedText::from([
            ("en".to_string(), "Natural render".to_string()),
            ("fr".to_string(), "Rendu naturel".to_string()),
        ])),
        None,
    );
    let open = PlanImageGate {
        allowed: true,
        tool_present: true,
        image_quality: false,
    };
    let imager = FakeImager {
        calls: Cell::new(0),
    };
    let outcome = run_plan_image(&imager, &job, open, &CancelFlag::new(), &mut |_| {}).unwrap();
    let results = plan_results(&outcome);
    assert_eq!(results.len(), 2);
    let (capture, finish) = (&results[0], &results[1]);
    assert_eq!(capture.target.variant_of, None);
    assert!(capture.path.ends_with("IC_1805_plan_raw.tif"));
    assert!(
        finish
            .path
            .ends_with("IC_1805_plan_raw_strong_colour-40_stars-85.tif")
    );
    assert_eq!(
        finish.target.variant_of,
        Some(format!("sha256:{}", capture.sha256))
    );
    // The main image says no finish, in the capture's steps as in its own.
    let steps = capture
        .target
        .metadata
        .as_ref()
        .and_then(|m| m.processing.as_ref())
        .and_then(|p| p.steps.clone())
        .unwrap();
    assert!(
        !steps.contains("contrast") && !steps.contains("colour"),
        "{steps}"
    );
    // The finish names its treatment in each language and keeps its own steps.
    let proposals = finish.target.metadata.as_ref().unwrap();
    assert_eq!(
        proposals.processing, None,
        "the capture's steps are the main image's"
    );
    let finish_steps = proposals
        .variant_processing
        .as_ref()
        .and_then(|p| p.steps.clone())
        .unwrap();
    assert!(
        finish_steps.contains("contrast strong · colour 40 · soft stars 85"),
        "{finish_steps}"
    );
    let titles = proposals.variant_generated_title.as_ref().unwrap();
    assert_eq!(
        titles["en"],
        "Natural render · Contrast 3.6 · Colours +40% · Softened stars 85%"
    );
    assert!(titles["fr"].starts_with("Rendu naturel · Contraste 3,6 · Couleurs"));
    assert_eq!(
        capture
            .target
            .metadata
            .as_ref()
            .and_then(|m| m.variant_generated_title.as_ref())
            .map(|t| t["en"].as_str()),
        Some("Natural render"),
        "the render without a finish keeps the render's own title"
    );
}

/// The plan's capture is already online: nothing creates another one, every
/// image -- the render and its finishes -- is added to it.
#[test]
fn the_images_of_a_plan_whose_capture_is_online_join_that_capture() {
    let mut raw = outcome_of("raw", false);
    raw.finishes.push(PlanFinish {
        render: StackedRender {
            contrast: crate::stacking::Contrast::Strong,
            path: PathBuf::from("/x/stacked/Veil_plan_raw_strong.tif"),
            sha256: "f".repeat(64),
            ..raw.render.clone()
        },
        processing: None,
    });
    let outcomes = [raw, outcome_of("hoo", false)];
    let results = plan_results_of(&outcomes, None, Some("capture-7"));
    assert_eq!(results.len(), 3);
    assert!(
        results
            .iter()
            .all(|r| r.target.variant_of.as_deref() == Some("capture-7")),
        "{:?}",
        results
            .iter()
            .map(|r| r.target.variant_of.clone())
            .collect::<Vec<_>>()
    );
    // The main render goes first, its finishes behind it.
    assert!(results[0].path.ends_with("Veil_plan_raw.tif"));
    assert!(results[1].path.ends_with("Veil_plan_raw_strong.tif"));
    assert!(results[2].path.ends_with("Veil_plan_hoo.tif"));
}

/// One run, one number: every pipeline of a run shares the name, and a run of
/// the same nights takes its name again instead of numbering new files.
#[test]
fn one_run_of_a_plan_has_one_number_and_a_second_run_of_the_same_nights_rewrites() {
    let dir = TempDir::new("plan-image-numbers");
    let (project, current) = heart_here(dir.path());
    let lines = plan_lines(dir.path(), &project, Some(&current));
    let plan_dir = plan_folder(dir.path(), &project).unwrap();
    let chosen: Vec<String> = lines.iter().map(|l| l.key.clone()).collect();
    let pipelines = [PipelineChoice::new("raw"), PipelineChoice::new("hoo")];
    let names = |plans: &[PlanImagePlan]| -> Vec<String> {
        plans
            .iter()
            .map(|plan| match plan {
                PlanImagePlan::Job(job) => job.name.clone(),
                other => panic!("a job, not {other:?}"),
            })
            .collect()
    };
    // A file of an earlier run of other nights holds the plain name and a
    // number only some pipelines have: the run takes the next number free for all.
    let out = plan_dir.join("stacked");
    fs::create_dir_all(&out).unwrap();
    fs::write(out.join("IC_1805_plan_raw.tif"), "older").unwrap();
    fs::write(out.join("IC_1805_plan-2_hoo.tif"), "older").unwrap();
    let first = plan_image_jobs(&project, &lines, &chosen, &plan_dir, &pipelines);
    assert_eq!(names(&first), ["IC_1805_plan-3", "IC_1805_plan-3"]);
    // The run is made: its record says the nights it combined.
    let PlanImagePlan::Job(job) = &first[0] else {
        panic!("a job")
    };
    fs::write(out.join(format!("{}_raw.tif", job.name)), "made").unwrap();
    fs::write(
        out.join(record_file_name(&job.name)),
        serde_json::json!({
            "masters": job.masters.iter().map(|m| serde_json::json!({
                "nightId": m.night_id, "filter": m.filter, "frameCount": m.frame_count
            })).collect::<Vec<_>>()
        })
        .to_string(),
    )
    .unwrap();
    // The same nights again: the same number, the files are rewritten.
    let again = plan_image_jobs(&project, &lines, &chosen, &plan_dir, &pipelines);
    assert_eq!(names(&again), ["IC_1805_plan-3", "IC_1805_plan-3"]);
}

#[test]
fn the_image_of_the_plan_is_made_here_and_leaves_as_a_capture_of_the_plan() {
    let dir = TempDir::new("plan-image-run");
    let (project, current) = heart_here(dir.path());
    let lines = plan_lines(dir.path(), &project, Some(&current));
    let plan_dir = plan_folder(dir.path(), &project).unwrap();
    let chosen: Vec<String> = lines.iter().map(|l| l.key.clone()).collect();
    let PlanImagePlan::Job(job) = plan_image_job(
        &project,
        &lines,
        &chosen,
        &plan_dir,
        Some(PipelineChoice::new("dualband-correction")),
    ) else {
        panic!("a job")
    };
    let imager = FakeImager {
        calls: Cell::new(0),
    };

    // Without the right: nothing runs.
    let closed = PlanImageGate {
        allowed: false,
        tool_present: true,
        image_quality: true,
    };
    assert_eq!(
        run_plan_image(&imager, &job, closed, &CancelFlag::new(), &mut |_| {}),
        Err(StackFailure::NotInSubscription)
    );
    assert_eq!(imager.calls.get(), 0);

    let open = PlanImageGate {
        allowed: true,
        tool_present: true,
        image_quality: true,
    };
    // The reference master says its optics: a 3.76 µm pixel behind 250 mm.
    fs::write(
        &job.masters[job.reference].path,
        crate::test_support::fits_bytes(
            &[("SIMPLE", "T"), ("XPIXSZ", "3.76"), ("FOCALLEN", "250")],
            32,
        ),
    )
    .unwrap();
    let mut steps = Vec::new();
    let outcome = run_plan_image(&imager, &job, open, &CancelFlag::new(), &mut |e| {
        if let StackEvent::Step { step, .. } = e {
            steps.push(step.clone());
        }
    })
    .unwrap();
    assert_eq!(steps, vec!["alignment", "combination", "render"]);
    assert_eq!(
        outcome.render.path,
        plan_dir.join("stacked/IC_1805_plan_dualband-correction.tif")
    );
    assert_eq!(outcome.other_files.len(), 1);
    assert_eq!(outcome.combined_night_ids, vec!["n-09", "n-12"]);
    assert_eq!(outcome.plan_render.as_deref(), Some("dualband"));
    let totals: Vec<(Option<&str>, u64)> = outcome
        .totals_by_filter
        .iter()
        .map(|t| (t.filter.as_deref(), t.total_seconds as u64))
        .collect();
    assert_eq!(
        totals,
        vec![(Some("L-eXtreme"), 17_640), (Some("UV/IR-cut"), 14_400)]
    );
    // The scratch and the job file are gone, the hidden folder with them.
    assert!(!plan_dir.join(WORK_DIR).exists());

    // It leaves as the capture of the plan, never of a night.
    let queue = UploadQueue::open(&dir.path().join("queue.sqlite")).unwrap();
    queue.enqueue_results(&plan_results(&outcome)).unwrap();
    let entry = queue.list().unwrap().remove(0);
    assert_eq!(entry.kind, ResultKind::Plan);
    let metadata = build_metadata(
        &entry,
        &UploadTarget {
            session_id: Some("n-12".into()),
            plan_id: Some("plan-12".into()),
            target_ids: Vec::new(),
        },
    );
    let json = serde_json::to_value(&metadata).unwrap();
    assert_eq!(json["role"], "plan");
    assert_eq!(json["projectId"], "p-heart");
    assert_eq!(
        json["combinedNightIds"],
        serde_json::json!(["n-09", "n-12"])
    );
    assert_eq!(json["planRender"], "dualband");
    // The quality of the combined image leaves with it, as the image's own.
    let quality = outcome.image_quality.as_ref().unwrap();
    assert_eq!((quality.fwhm_pixels, quality.star_count), (4.2, 188));
    assert_eq!(quality.eccentricity, Some(0.27));
    assert!((quality.fwhm_arcsec.unwrap() - 4.2 * 3.1022).abs() < 0.01);
    assert_eq!(json["imageQuality"]["measuredOn"], "linear");
    assert_eq!(json["imageQuality"]["starCount"], 188);
    assert_eq!(json["totalsByFilter"][0]["totalSeconds"], 17_640.0);
    assert_eq!(json["targetIds"], serde_json::json!(["ic-1805"]));
    for absent in ["sessionId", "planId", "captureId"] {
        assert!(json.get(absent).is_none(), "{absent}");
    }
    // A new image of the plan never replaces the last one's file.
    let PlanImagePlan::Job(again) = plan_image_job(
        &project,
        &lines,
        &chosen,
        &plan_dir,
        Some(PipelineChoice::new("dualband-correction")),
    ) else {
        panic!("a job")
    };
    assert_eq!(again.name, "IC_1805_plan-2");
}

#[test]
fn the_image_of_a_plan_has_no_quality_without_analysis_logs_or_without_a_measure() {
    let dir = TempDir::new("plan-image-no-quality");
    let (project, current) = heart_here(dir.path());
    let lines = plan_lines(dir.path(), &project, Some(&current));
    let plan_dir = plan_folder(dir.path(), &project).unwrap();
    let chosen: Vec<String> = lines.iter().map(|l| l.key.clone()).collect();
    let PlanImagePlan::Job(job) = plan_image_job(
        &project,
        &lines,
        &chosen,
        &plan_dir,
        Some(PipelineChoice::new("raw")),
    ) else {
        panic!("a job")
    };
    let imager = FakeImager {
        calls: Cell::new(0),
    };
    let gate = |image_quality| PlanImageGate {
        allowed: true,
        tool_present: true,
        image_quality,
    };
    // Without the right: the image is made and leaves, nothing is built.
    let without =
        run_plan_image(&imager, &job, gate(false), &CancelFlag::new(), &mut |_| {}).unwrap();
    assert_eq!(without.image_quality, None);
    // ... except what made it: the image always says so.
    let metadata = plan_results(&without)[0].target.metadata.clone().unwrap();
    assert_eq!(metadata.image_quality, None);
    assert!(metadata.processing.is_some());
    // With the right, but fewer than twenty stars (`quality: null`).
    let record = job.out.join(record_file_name(&job.name));
    let nothing = || {
        fs::write(&record, r#"{"format":1,"quality":null}"#).unwrap();
        plan_image_quality(&job)
    };
    assert_eq!(nothing(), None);
    // A pixel scale unknown: the figure in pixels only.
    fs::write(
        &record,
        r#"{"quality":{"fwhmPixels":4.2,"measuredOn":"linear","starCount":40}}"#,
    )
    .unwrap();
    let pixels = plan_image_quality(&job).unwrap();
    assert_eq!(
        (pixels.fwhm_pixels, pixels.fwhm_arcsec, pixels.eccentricity),
        (4.2, None, None)
    );
    let sent = plan_results(&PlanImageOutcome {
        image_quality: Some(pixels),
        ..without
    });
    let metadata = sent[0].target.metadata.as_ref().unwrap();
    // The quality is the image's part; what made it, the capture's.
    assert!(metadata.has_image_part() && metadata.has_capture_part());
}

#[test]
fn a_declared_line_without_frames_is_read_and_is_not_an_integration_to_combine() {
    // The Journal declares a line for a night of the plan whose
    // poses are not known: `frameCount` 0, `exposureSeconds` 0, the time
    // declared.
    let dir = TempDir::new("plan-image-declared");
    let (_, current) = heart_here(dir.path());
    let mut heart = heart();
    heart["integrations"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "id": "i-n-14", "projectId": "p-heart", "nightId": "n-14", "targetId": "ic-1805",
            "filter": null, "exposureSeconds": 0, "frameCount": 0, "totalSeconds": 7200,
            "eveningDate": "2026-08-14", "source": "declared", "sessionId": "n-14"
        }));
    // A line of an older service may even leave the zeros out.
    heart["integrations"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "id": "i-n-15", "projectId": "p-heart", "nightId": "n-15", "targetId": "ic-1805",
            "totalSeconds": 3600, "eveningDate": "2026-08-15", "source": "declared"
        }));
    let project = project_of(&heart, "ic-1805").unwrap();
    // Both are read (the plan's total counts their declared time)...
    assert_eq!(project.integrations.len(), 5);
    assert_eq!(project.night_count, 5);
    let declared = project
        .integrations
        .iter()
        .find(|i| i.night_id == "n-14")
        .unwrap();
    assert!(declared.is_declared_without_frames());
    assert_eq!((declared.frame_count, declared.seconds()), (0, 7200.0));
    // ...but the lines to combine are the ones a computer holds.
    let lines = plan_lines(dir.path(), &project, Some(&current));
    assert_eq!(lines.len(), 3);
    assert!(
        lines
            .iter()
            .all(|l| l.night_id != "n-14" && l.night_id != "n-15")
    );
    // A measured line of zero frames is not a declaration: it stays listed.
    let mut measured = project.clone();
    measured.integrations[0].source = None;
    assert!(!measured.integrations[0].is_declared_without_frames());
}

/// C2: the record of a plan's combination is `<name>_plan.json`, never
/// `…_plan_plan.json` (the name of a plan's image already ends in `_plan`),
/// the rule of `plan_stack.py`.
#[test]
fn the_record_of_a_plan_is_never_named_plan_twice() {
    assert_eq!(record_file_name("IC_1805_plan"), "IC_1805_plan.json");
    assert_eq!(record_file_name("IC_1805"), "IC_1805_plan.json");
    assert!(!record_file_name("IC_1805_plan").contains("plan_plan"));
}

/// R348b: the service's word for each shipped render, "polished render" and
/// « Palette HOO » included; a pipeline the person added says nothing.
#[test]
fn the_service_has_a_word_for_each_shipped_render() {
    for (pipeline, word) in [
        ("raw", "natural"),
        ("dualband-correction", "dualband"),
        ("polished", "polished"),
        ("hoo", "hoo"),
        (LINEAR_PIPELINE, "none"),
    ] {
        assert_eq!(
            plan_render_of(pipeline).as_deref(),
            Some(word),
            "{pipeline}"
        );
    }
    assert_eq!(plan_render_of("mono-ha"), None);
    assert_eq!(plan_render_of("my-own-pipeline"), None);
}

/// The image of a plan takes the pipelines chosen for its target: one
/// image per pipeline, each a capture of the plan, the target's defaults when
/// none was chosen.
#[test]
fn the_image_of_a_plan_takes_the_pipelines_chosen_for_its_target() {
    let dir = TempDir::new("plan-image-pipelines");
    let (project, current) = heart_here(dir.path());
    let lines = plan_lines(dir.path(), &project, Some(&current));
    let plan_dir = plan_folder(dir.path(), &project).unwrap();
    let chosen: Vec<String> = lines.iter().map(|l| l.key.clone()).collect();
    let pipeline_ids = |plans: &[PlanImagePlan]| -> Vec<String> {
        plans
            .iter()
            .map(|plan| match plan {
                PlanImagePlan::Job(job) => job.pipeline.pipeline_id.clone(),
                other => panic!("a job, not {other:?}"),
            })
            .collect()
    };

    // None chosen: the defaults of the target; a dual-band integration is
    // combined, so "Dual-band correction" comes with "Natural render".
    let by_default = plan_image_jobs(&project, &lines, &chosen, &plan_dir, &[]);
    assert_eq!(
        pipeline_ids(&by_default),
        vec!["raw", "dualband-correction"]
    );
    // Without the dual-band integration (the 12th alone with another night
    // would be one night: tick only the RGB-free lines of the fixture).
    let broadband: Vec<PlanLine> = lines
        .iter()
        .cloned()
        .map(|mut line| {
            line.band = Band::Broadband;
            line
        })
        .collect();
    assert_eq!(
        default_plan_pipelines(&broadband, &chosen)
            .iter()
            .map(|p| p.pipeline_id.as_str())
            .collect::<Vec<_>>(),
        vec!["raw"]
    );

    // The pipelines chosen, each once, "No processing" among them.
    let asked = [
        PipelineChoice::new("hoo"),
        PipelineChoice::new("linear"),
        PipelineChoice::new("hoo"),
    ];
    let jobs = plan_image_jobs(&project, &lines, &chosen, &plan_dir, &asked);
    assert_eq!(pipeline_ids(&jobs), vec!["hoo", "linear"]);

    // Nothing to combine stays one answer, whatever the pipelines.
    assert_eq!(
        plan_image_jobs(&project, &lines, &[], &plan_dir, &asked),
        vec![PlanImagePlan::Nothing]
    );

    // Each job makes its image, in turn; each leaves as a capture of the plan.
    let imager = FakeImager {
        calls: Cell::new(0),
    };
    let open = PlanImageGate {
        allowed: true,
        tool_present: true,
        image_quality: false,
    };
    let planned: Vec<PlanImageJob> = jobs
        .into_iter()
        .map(|plan| match plan {
            PlanImagePlan::Job(job) => *job,
            other => panic!("a job, not {other:?}"),
        })
        .collect();
    let refs: Vec<&PlanImageJob> = planned.iter().collect();
    let run = run_plan_images(&imager, &refs, open, &CancelFlag::new(), &mut |_| {});
    assert!(run.failure.is_none(), "{:?}", run.failure);
    assert_eq!(imager.calls.get(), 2);
    let renders: Vec<Option<&str>> = run
        .outcomes
        .iter()
        .map(|outcome| outcome.plan_render.as_deref())
        .collect();
    assert_eq!(renders, vec![Some("hoo"), Some("none")]);
    let results = plan_results_of(&run.outcomes, None, None);
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|r| r.kind == ResultKind::Plan));
    assert_ne!(results[0].path, results[1].path);
    // One capture of the plan: the second render is an image of it.
    assert_eq!(results[0].target.variant_of, None);
    assert!(results[0].target.plan.is_some());
    assert_eq!(
        results[1].target.variant_of,
        Some(format!("sha256:{}", results[0].sha256))
    );

    // A tool missing stops the series where it stands: nothing made, said.
    let closed = PlanImageGate {
        allowed: true,
        tool_present: false,
        image_quality: false,
    };
    let stopped = run_plan_images(&imager, &refs, closed, &CancelFlag::new(), &mut |_| {});
    assert!(stopped.outcomes.is_empty());
    assert_eq!(stopped.failure, Some(StackFailure::ToolMissing));
}

/* --------------- The image of a device, and the adjustable frame --------------- */

/// The Heart on this computer where the 9th August has only the image its device stacked, and the 12th (the
/// night being sent) has a master kept and the device's image too.
fn heart_with_devices(root: &Path) -> (NightProject, CurrentNight) {
    let (project, current) = heart_here(root);
    let ninth = root.join("2026-08-09 · Cœur · n-09");
    // The 9th was stacked by the device alone: the master of the family goes, the device image stays.
    fs::remove_dir_all(ninth.join("IC 1805/stacked")).unwrap();
    write_device_image(&ninth.join("IC 1805/device"), 98, "L-eXtreme", 180.0);
    write_device_image(
        &current.folder.join("IC 1805/device"),
        120,
        "UV/IR-cut",
        120.0,
    );
    (project, current)
}

#[test]
fn a_night_with_only_the_image_of_its_device_is_a_line_of_the_plan() {
    let dir = TempDir::new("plan-image-device-line");
    let (project, current) = heart_with_devices(dir.path());
    let lines = plan_lines(dir.path(), &project, Some(&current));
    let ninth = &lines[0];
    assert_eq!(ninth.night_id, "n-09");
    assert_eq!(ninth.master, None);
    assert!(ninth.combinable());
    assert_eq!(ninth.source, PlanSource::Device);
    let device = ninth.device.as_ref().unwrap();
    assert_eq!(device.device, "Seestar S50 Pro");
    assert_eq!(device.frame_count, 98);
    assert_eq!(device.total_seconds, 17_640.0);
    assert!(
        device
            .path
            .ends_with("IC 1805/device/Stacked_98_IC 1805_180.0s_L-eXtreme_20260812-061931.fit")
    );
    // Its line says what the header says, not the service's: the key names the device's family.
    assert!(ninth.key.starts_with("n-09|"), "{}", ninth.key);
    // The 10th is on another computer: nothing to combine, whatever the 9th has.
    assert!(!lines[1].combinable());
    assert_eq!(lines[1].device, None);
}

#[test]
fn a_night_with_a_master_and_an_image_of_its_device_takes_the_master_unless_asked() {
    let dir = TempDir::new("plan-image-both");
    let (project, current) = heart_with_devices(dir.path());
    let lines = plan_lines(dir.path(), &project, Some(&current));
    let twelfth = &lines[2];
    assert_eq!(twelfth.night_id, "n-12");
    assert!(twelfth.master.is_some() && twelfth.device.is_some());
    // The poses are here: the master ("Stack again") is the default.
    assert_eq!(twelfth.source, PlanSource::Master);
    assert_eq!(twelfth.source_for(None), PlanSource::Master);
    assert_eq!(
        twelfth.source_for(Some(PlanSource::Device)),
        PlanSource::Device
    );
    // A source that is not here is not taken: the default stands.
    assert_eq!(
        lines[0].source_for(Some(PlanSource::Master)),
        PlanSource::Device
    );
    let plan_dir = plan_folder(dir.path(), &project).unwrap();
    let chosen: Vec<String> = lines.iter().map(|l| l.key.clone()).collect();
    let PlanImagePlan::Job(job) = plan_image_job(&project, &lines, &chosen, &plan_dir, None) else {
        panic!("a job")
    };
    let sources: Vec<(&str, PlanSource)> = job
        .masters
        .iter()
        .map(|m| (m.night_id.as_str(), m.source))
        .collect();
    assert_eq!(
        sources,
        vec![("n-09", PlanSource::Device), ("n-12", PlanSource::Master)]
    );
    // The device image's own totals are the night's (its header), its file the job's path.
    assert_eq!(job.masters[0].frame_count, 98);
    assert!(
        job.masters[0]
            .path
            .to_string_lossy()
            .contains("/device/Stacked_98")
    );
    assert!(
        job.masters[1]
            .path
            .to_string_lossy()
            .ends_with("_integration.fit")
    );
    // A master is the default and is not written: the job of a plan of masters alone is what it was.
    let json = serde_json::to_value(&job).unwrap();
    assert_eq!(json["masters"][0]["source"], "device");
    assert!(json["masters"][1].get("source").is_none());
    assert!(json.get("frame").is_none());
    // Asked the device's image for the 12th too: its totals (120 poses of 120 s) and its file.
    let mut making = PlanMaking::default();
    making
        .sources
        .insert(lines[2].key.clone(), PlanSource::Device);
    let PlanImagePlan::Job(by_device) =
        plan_image_job_with(&project, &lines, &chosen, &plan_dir, None, &making)
    else {
        panic!("a job")
    };
    assert_eq!(by_device.masters[1].source, PlanSource::Device);
    assert_eq!(by_device.masters[1].frame_count, 120);
    assert!(
        by_device.masters[1]
            .path
            .to_string_lossy()
            .contains("/device/")
    );
}

#[test]
fn the_reference_follows_the_totals_of_the_source_taken() {
    let dir = TempDir::new("plan-image-device-reference");
    let (project, current) = heart_with_devices(dir.path());
    let lines = plan_lines(dir.path(), &project, Some(&current));
    let plan_dir = plan_folder(dir.path(), &project).unwrap();
    let chosen: Vec<String> = lines.iter().map(|l| l.key.clone()).collect();
    // Mixed bands (L-eXtreme + UV/IR-cut): the reference is the dual-band night whatever the source.
    let PlanImagePlan::Job(job) = plan_image_job(&project, &lines, &chosen, &plan_dir, None) else {
        panic!("a job")
    };
    assert_eq!(job.masters[job.reference].night_id, "n-09");
}

#[test]
fn a_device_image_that_a_master_covers_is_not_a_line_of_its_own_nor_sent_twice() {
    let dir = TempDir::new("plan-image-device-covered");
    let (project, current) = heart_with_devices(dir.path());
    let lines = plan_lines(dir.path(), &project, Some(&current));
    // One line for the 12th, not two.
    assert_eq!(lines.iter().filter(|l| l.night_id == "n-12").count(), 1);
    // The integration lines of the night the service gets: the master's, never the device image of the same family.
    let masters = crate::multi_night::night_sources(
        &crate::multi_night::target_masters(&current.folder, "ic-1805"),
        &crate::multi_night::target_device_images(&current.folder, "ic-1805"),
    );
    assert_eq!(masters.len(), 1);
    assert!(
        masters[0]
            .path
            .to_string_lossy()
            .ends_with("_integration.fit")
    );
    // The 9th has no master: the integration line is read from the header of the device's image.
    let ninth = dir.path().join("2026-08-09 · Cœur · n-09");
    let sources = crate::multi_night::night_sources(
        &crate::multi_night::target_masters(&ninth, "ic-1805"),
        &crate::multi_night::target_device_images(&ninth, "ic-1805"),
    );
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].frame_count, 98);
    assert_eq!(sources[0].filter.as_deref(), Some("L-eXtreme"));
    assert_eq!(sources[0].exposure_seconds, Some(180.0));
}

#[test]
fn two_images_of_a_family_leave_the_bigger_and_a_stretched_name_is_not_a_device_image() {
    let dir = TempDir::new("plan-image-device-split");
    let target = dir.path().join("IC 1805");
    write_device_image(&target.join("device"), 28, "IRCUT", 10.0);
    write_device_image(&target.join("device"), 8, "IRCUT", 10.0);
    write_device_image(&target.join("device"), 430, "IRCUT", 30.0);
    fs::write(target.join("device/Stacked_note.fit"), b"not a fits file").unwrap();
    fs::write(target.join("device/Stacked_28_IC 1805.jpg"), b"jpeg").unwrap();
    let found = crate::multi_night::night_device_images(dir.path(), "IC 1805");
    let counts: Vec<(String, u64)> = found
        .iter()
        .map(|d| (d.family.clone(), d.frame_count))
        .collect();
    // Never combined: the biggest of each family (the Seestar cuts its 10 s stack in two: 28 + 8).
    assert_eq!(
        counts,
        vec![
            ("IRCUT_10s_g100".to_string(), 28),
            ("IRCUT_30s_g100".to_string(), 430)
        ]
    );
}

/// A record of an image of the plan, as `plan_stack.py` writes it (the parts the screen reads).
fn write_record(stacked: &Path, name: &str, masters: &[(&str, Option<&str>, &str)], degraded: f64) {
    fs::create_dir_all(stacked).unwrap();
    let notch = |top: u32,
                 left: u32,
                 height: u32,
                 width: u32,
                 share: f64,
                 bad: f64,
                 low: u64,
                 kind: &str| {
        serde_json::json!({
            "top": top, "left": left, "height": height, "width": width,
            "shareOfFrame": share, "degradedShare": bad, "emptyShare": 0.0,
            "lowestSeconds": low, "lowestDualBandSeconds": 0, "t": 0.0,
            "recommended": kind == "mark", "kind": kind, "default": kind == "whole",
        })
    };
    let record = serde_json::json!({
        "format": 1,
        "masters": masters.iter().map(|(night, filter, source)| serde_json::json!({
            "nightId": night, "filter": filter, "source": source, "band": "broadband",
        })).collect::<Vec<_>>(),
        "coverage": {
            "noDualBandShare": 0.381, "degradedShare": degraded, "hasDualBand": true,
            "fullSeconds": 12_900, "fullDualBandSeconds": 6_450, "recommendedDegradedShare": 0.05,
            "fullPolygon": [[0, 1231], [2160, 1100], [2160, 2700], [0, 2609]],
            "fullCoverage": { "top": 0, "left": 0, "height": 1378, "width": 2152, "shareOfFrame": 0.36,
                              "frameHeight": 3840, "frameWidth": 2160 },
            "zones": [],
            "framings": [
                notch(1231, 4, 1378, 2152, 0.36, 0.0, 12_900, "cropped"),
                notch(923, 3, 1994, 2154, 0.52, 0.05, 6_450, "mark"),
                notch(0, 0, 3840, 2160, 1.0, 0.381, 6_450, "whole"),
            ],
        },
        "preview": format!("{name}.preview.png"),
        "frame": null,
        // An image of the default setting (« Bords nets »), with nothing to cut.
        "crop": { "mode": "clean", "applied": false, "reason": "alreadyWhole" },
    });
    fs::write(stacked.join(format!("{name}.json")), record.to_string()).unwrap();
    fs::write(
        stacked.join(format!("{name}.preview.png")),
        [0x89, b'P', b'N', b'G', 1],
    )
    .unwrap();
}

#[test]
fn the_frames_on_offer_are_read_from_the_last_image_made_with_the_same_integrations() {
    let dir = TempDir::new("plan-image-framing");
    let (project, current) = heart_here(dir.path());
    let lines = plan_lines(dir.path(), &project, Some(&current));
    let plan_dir = plan_folder(dir.path(), &project).unwrap();
    let stacked = plan_dir.join("stacked");
    let chosen: Vec<String> = lines.iter().map(|l| l.key.clone()).collect();
    // Before the first image: nothing (the first image is always whole).
    assert_eq!(
        plan_framing(&plan_dir, &lines, &chosen, &PlanMaking::default()),
        None
    );
    let both = [
        ("n-09", Some("L-eXtreme"), "master"),
        ("n-12", Some("UV/IR-cut"), "master"),
    ];
    write_record(&stacked, "IC_1805_plan", &both, 0.381);
    // The 10th is on the laptop and cannot be ticked into the set: ticking it changes nothing.
    let chosen: Vec<String> = lines
        .iter()
        .filter(|l| l.combinable())
        .map(|l| l.key.clone())
        .collect();
    let framing = plan_framing(&plan_dir, &lines, &chosen, &PlanMaking::default()).unwrap();
    // The notches are those of an image cut the same way: another setting reads none of them.
    let common = PlanMaking {
        plan_field: PlanField::Common,
        ..PlanMaking::default()
    };
    assert_eq!(plan_framing(&plan_dir, &lines, &chosen, &common), None);
    assert_eq!(framing.record, "IC_1805_plan.json");
    assert!(framing.offered);
    assert_eq!(framing.notches.len(), 3);
    assert_eq!(framing.notches[0].kind, "cropped");
    assert_eq!(
        framing.notches[1].frame(),
        PlanFrame {
            top: 923,
            left: 3,
            height: 1994,
            width: 2154
        }
    );
    assert!(framing.notches[2].default);
    assert_eq!((framing.frame_height, framing.frame_width), (3840, 2160));
    assert_eq!(framing.full_polygon.len(), 4);
    assert!((framing.no_dual_band_share - 0.381).abs() < 1e-9);
    assert_eq!(framing.full_dual_band_seconds, 6_450);
    // The picture goes to the window as a data URL.
    assert!(
        framing
            .preview
            .as_deref()
            .is_some_and(|p| p.starts_with("data:image/png;base64,iVBORw"))
    );
    // Another set of integrations (the 12th unticked): the record is not for it.
    let only_ninth = vec![chosen[0].clone()];
    assert_eq!(
        plan_framing(&plan_dir, &lines, &only_ninth, &PlanMaking::default()),
        None
    );
}

#[test]
fn the_source_taken_is_part_of_the_set_a_record_is_for() {
    let dir = TempDir::new("plan-image-framing-source");
    let (project, current) = heart_with_devices(dir.path());
    let lines = plan_lines(dir.path(), &project, Some(&current));
    let plan_dir = plan_folder(dir.path(), &project).unwrap();
    let stacked = plan_dir.join("stacked");
    let chosen: Vec<String> = lines
        .iter()
        .filter(|l| l.combinable())
        .map(|l| l.key.clone())
        .collect();
    // The 9th by its device image (the only one it has), the 12th by its master.
    write_record(
        &stacked,
        "IC_1805_plan",
        &[
            ("n-09", Some("L-eXtreme"), "device"),
            ("n-12", Some("UV/IR-cut"), "master"),
        ],
        0.381,
    );
    assert!(plan_framing(&plan_dir, &lines, &chosen, &PlanMaking::default()).is_some());
    // The device image of the 12th taken instead of its master: another set of integrations, no record yet.
    let mut making = PlanMaking::default();
    making
        .sources
        .insert(lines[2].key.clone(), PlanSource::Device);
    assert_eq!(plan_framing(&plan_dir, &lines, &chosen, &making), None);
}

#[test]
fn the_newest_record_of_the_set_is_the_one_read_and_under_one_percent_nothing_is_offered() {
    let dir = TempDir::new("plan-image-framing-newest");
    let (project, current) = heart_here(dir.path());
    let lines = plan_lines(dir.path(), &project, Some(&current));
    let plan_dir = plan_folder(dir.path(), &project).unwrap();
    let stacked = plan_dir.join("stacked");
    let both = [
        ("n-09", Some("L-eXtreme"), "master"),
        ("n-12", Some("UV/IR-cut"), "master"),
    ];
    write_record(&stacked, "IC_1805_plan", &both, 0.381);
    std::thread::sleep(std::time::Duration::from_millis(30));
    // The next image (a name taken: « -2 ») had 0.5 % of its pixels below the full exposure.
    write_record(&stacked, "IC_1805_plan-2_plan", &both, 0.005);
    let chosen: Vec<String> = lines
        .iter()
        .filter(|l| l.combinable())
        .map(|l| l.key.clone())
        .collect();
    let framing = plan_framing(&plan_dir, &lines, &chosen, &PlanMaking::default()).unwrap();
    assert_eq!(framing.record, "IC_1805_plan-2_plan.json");
    assert!(
        !framing.offered,
        "under {FRAMING_OFFERED_FROM} of the image"
    );
}

#[test]
fn the_image_that_leaves_is_the_cut_one_and_the_whole_one_stays_beside_it() {
    struct Framing;
    impl PlanImager for Framing {
        fn make(
            &self,
            job: &PlanImageJob,
            _: &CancelFlag,
            _: &mut dyn FnMut(&StackEvent),
        ) -> Result<Vec<StackerOutput>, StackFailure> {
            fs::create_dir_all(&job.work).unwrap();
            let mut written = Vec::new();
            for file in [
                format!("{}_raw.tif", job.name),
                format!("{}_raw_framed.tif", job.name),
            ] {
                fs::write(job.out.join(&file), file.as_bytes()).unwrap();
                written.push(StackerOutput {
                    pipeline: "raw".into(),
                    file,
                    contrast: crate::stacking::Contrast::None,
                    retouch: Default::default(),
                });
            }
            Ok(written)
        }
    }
    let dir = TempDir::new("plan-image-framed-render");
    let (project, current) = heart_here(dir.path());
    let lines = plan_lines(dir.path(), &project, Some(&current));
    let plan_dir = plan_folder(dir.path(), &project).unwrap();
    let chosen: Vec<String> = lines
        .iter()
        .filter(|l| l.combinable())
        .map(|l| l.key.clone())
        .collect();
    let frame = PlanFrame {
        top: 10,
        left: 4,
        height: 300,
        width: 200,
    };
    let making = PlanMaking {
        frame: Some(frame),
        ..PlanMaking::default()
    };
    let PlanImagePlan::Job(job) = plan_image_job_with(
        &project,
        &lines,
        &chosen,
        &plan_dir,
        Some(PipelineChoice::new("raw")),
        &making,
    ) else {
        panic!("a job")
    };
    // The job of the same image without a frame, made before the first leaves its files (a name taken is renamed).
    let PlanImagePlan::Job(plain) = plan_image_job(
        &project,
        &lines,
        &chosen,
        &plan_dir,
        Some(PipelineChoice::new("raw")),
    ) else {
        panic!("a job")
    };
    assert_eq!(job.frame, Some(frame));
    let json = serde_json::to_value(&job).unwrap();
    assert_eq!(
        json["frame"],
        serde_json::json!({ "top": 10, "left": 4, "height": 300, "width": 200 })
    );
    let gate = PlanImageGate {
        allowed: true,
        tool_present: true,
        image_quality: false,
    };
    let outcome =
        run_plan_image(&Framing, &job, gate, &CancelFlag::default(), &mut |_| {}).unwrap();
    assert!(outcome.render.path.ends_with("IC_1805_plan_raw_framed.tif"));
    assert_eq!(outcome.other_files.len(), 1);
    assert!(outcome.other_files[0].ends_with("IC_1805_plan_raw.tif"));
    // Without a frame in the job the first image leaves, as before.
    let outcome =
        run_plan_image(&Framing, &plain, gate, &CancelFlag::default(), &mut |_| {}).unwrap();
    assert!(outcome.render.path.ends_with("IC_1805_plan_raw.tif"));
}

#[test]
fn the_base64_of_the_preview_is_the_standard_one() {
    for (bytes, text) in [
        (&b""[..], ""),
        (b"f", "Zg=="),
        (b"fo", "Zm8="),
        (b"foo", "Zm9v"),
        (b"foob", "Zm9vYg=="),
        (b"foobar", "Zm9vYmFy"),
    ] {
        assert_eq!(base64(bytes), text);
    }
}

#[test]
fn a_plan_combined_on_its_dual_band_master_says_it_was_aligned_on_the_stars() {
    let dir = TempDir::new("plan-image-dualband-steps");
    let (project, current) = heart_here(dir.path());
    let lines = plan_lines(dir.path(), &project, Some(&current));
    let plan_dir = plan_folder(dir.path(), &project).unwrap();
    let chosen: Vec<String> = lines.iter().map(|l| l.key.clone()).collect();
    let PlanImagePlan::Job(job) = plan_image_job(
        &project,
        &lines,
        &chosen,
        &plan_dir,
        Some(PipelineChoice::new("raw")),
    ) else {
        panic!("a job")
    };
    fs::create_dir_all(&job.out).unwrap();
    // The record `plan_stack.py` writes for a plan combined on its dual-band
    // master (`dualband_reference.py`): no Siril alignment, the scales of
    // the 99th percentiles, the broadband master at 0.18.
    fs::write(
        job.out.join(record_file_name(&job.name)),
        serde_json::json!({
            "format": 1,
            "tools": {"siril": "siril 1.4.4"},
            "combination": {
                "method": "dualband_reference",
                "masters": [{"weight": 1.0}, {"weight": 0.18}],
                "fusion": {"method": "dualband_reference", "weight": 0.18},
                "levelMatch": [{
                    "nightId": "n-12", "to": "n-09", "method": "percentile99",
                    "scales": [0.24329, 0.28934, 0.29103], "stars": 60
                }],
                "noData": 0.0196
            }
        })
        .to_string(),
    )
    .unwrap();
    let steps = plan_processing(&job, &outcome_of("raw", false).render)
        .steps
        .unwrap();
    assert!(
        steps.starts_with(
            "pipeline natural (raw) · alignment stars (astroalign) · level match percentile99 scales 0.243/0.289/0.291 · dualband_reference"
        ),
        "{steps}"
    );
    assert!(steps.contains("weights 1/0.18"), "{steps}");
}

#[test]
fn the_crop_of_the_frame_is_a_step_of_the_image_and_its_kind_a_word_of_the_job() {
    let dir = TempDir::new("plan-image-crop");
    let (project, current) = heart_here(dir.path());
    let lines = plan_lines(dir.path(), &project, Some(&current));
    let plan_dir = plan_folder(dir.path(), &project).unwrap();
    let chosen: Vec<String> = lines.iter().map(|l| l.key.clone()).collect();
    let PlanImagePlan::Job(job) = plan_image_job(
        &project,
        &lines,
        &chosen,
        &plan_dir,
        Some(PipelineChoice::new("raw")),
    ) else {
        panic!("a job")
    };
    // By default the edges without data are cut (nothing written: the
    // script's default); a plan's image has no rectangle or frame master given.
    assert_eq!(job.plan_field, PlanField::Clean);
    let json = serde_json::to_value(&job).unwrap();
    for key in ["planField", "crop", "frameMaster"] {
        assert!(json.get(key).is_none(), "{key}");
    }
    let making = PlanMaking {
        plan_field: PlanField::Common,
        ..PlanMaking::default()
    };
    let PlanImagePlan::Job(common) = plan_image_job_with(
        &project,
        &lines,
        &chosen,
        &plan_dir,
        Some(PipelineChoice::new("raw")),
        &making,
    ) else {
        panic!("a job")
    };
    assert_eq!(
        serde_json::to_value(&*common).unwrap()["planField"],
        "common"
    );
    // A milestone's rectangle and frame master, as the script reads them.
    let mut milestone = job.clone();
    milestone.crop = Some(PlanCrop {
        top: 444,
        left: 419,
        height: 2095,
        width: 2137,
        frame_height: 2977,
        frame_width: 2995,
    });
    milestone.frame_master = Some(PlanFrameMaster {
        path: PathBuf::from("/n/Dual-band_180s_g100_integration.fit"),
        source: PlanSource::Master,
        night_id: Some("n-07".into()),
    });
    let json = serde_json::to_value(&milestone).unwrap();
    assert_eq!(
        json["crop"],
        serde_json::json!({"top": 444, "left": 419, "height": 2095, "width": 2137, "frameHeight": 2977, "frameWidth": 2995})
    );
    assert_eq!(
        json["frameMaster"],
        serde_json::json!({"path": "/n/Dual-band_180s_g100_integration.fit", "nightId": "n-07"})
    );
    // The record of the image of the Veil (three nights, the third turned by 39°): the rectangle kept, the edges lost.
    fs::create_dir_all(&job.out).unwrap();
    let crop = serde_json::json!({
        "mode": "common", "applied": true, "reason": "commonArea", "top": 444, "left": 419, "height": 2095, "width": 2137,
        "frameHeight": 2977, "frameWidth": 2995,
        "lost": {"top": 444, "left": 419, "bottom": 438, "right": 439}, "keptShare": 0.5021
    });
    fs::write(
        job.out.join(record_file_name(&job.name)),
        serde_json::json!({
            "format": 1,
            "combination": {"method": "dualband_reference", "noData": 0.0},
            "crop": crop,
            "frameMaster": {"path": "/n/Dual-band_180s_g100_integration.fit", "source": "master", "nightId": "n-07"}
        })
        .to_string(),
    )
    .unwrap();
    let steps = plan_processing(&job, &outcome_of("raw", false).render)
        .steps
        .unwrap();
    assert!(
        steps.contains(
            "crop to common area 2137x2095 px of 2995x2977, kept 50.2 % (edges lost top/left/bottom/right 444/419/438/439 px)"
        ),
        "{steps}"
    );
    assert!(!steps.contains("no data"), "{steps}");
    let (rectangle, master) = recorded_crop(&job).unwrap();
    assert_eq!(rectangle, milestone.crop.unwrap());
    assert_eq!(master, milestone.frame_master.unwrap());
    // Nothing cut: no step, and the whole frame is what a milestone is given.
    fs::write(
        job.out.join(record_file_name(&job.name)),
        serde_json::json!({
            "crop": {"mode": "whole", "applied": false, "reason": "wholeField", "top": 0, "left": 0, "height": 2977,
                     "width": 2995, "frameHeight": 2977, "frameWidth": 2995},
            "frameMaster": {"path": "/n/a.fit"}
        })
        .to_string(),
    )
    .unwrap();
    let steps = plan_processing(&job, &outcome_of("raw", false).render)
        .steps
        .unwrap();
    assert!(!steps.contains("crop"), "{steps}");
    assert_eq!(recorded_crop(&job).unwrap().0.height, 2977);
    // « Bords nets »: the edges without data only (the Veil: 97 % of the field).
    fs::write(
        job.out.join(record_file_name(&job.name)),
        serde_json::json!({
            "crop": {"mode": "clean", "applied": true, "reason": "noEmptyPixel", "top": 22, "left": 20,
                     "height": 2939, "width": 2945, "frameHeight": 2977, "frameWidth": 2995,
                     "lost": {"top": 22, "left": 20, "bottom": 16, "right": 30}, "keptShare": 0.9708},
            "frameMaster": {"path": "/n/a.fit"}
        })
        .to_string(),
    )
    .unwrap();
    let steps = plan_processing(&job, &outcome_of("raw", false).render)
        .steps
        .unwrap();
    assert!(
        steps.contains("crop to edges with data 2945x2939 px of 2995x2977, kept 97.1 % (edges lost top/left/bottom/right 22/20/16/30 px)"),
        "{steps}"
    );
}

#[test]
fn the_image_of_a_plan_says_what_made_it_and_never_the_tiff_library() {
    let dir = TempDir::new("plan-image-processing");
    let (project, current) = heart_here(dir.path());
    let lines = plan_lines(dir.path(), &project, Some(&current));
    let plan_dir = plan_folder(dir.path(), &project).unwrap();
    let chosen: Vec<String> = lines.iter().map(|l| l.key.clone()).collect();
    let PlanImagePlan::Job(mut job) = plan_image_job(
        &project,
        &lines,
        &chosen,
        &plan_dir,
        Some(PipelineChoice::new("raw")),
    ) else {
        panic!("a job")
    };
    job.siril = Some(PathBuf::from("/tools/siril"));
    let gate = PlanImageGate {
        allowed: true,
        tool_present: true,
        image_quality: false,
    };
    let imager = FakeImager {
        calls: Cell::new(0),
    };
    let outcome = run_plan_image(&imager, &job, gate, &CancelFlag::new(), &mut |_| {}).unwrap();
    let processing = outcome.processing.clone().unwrap();
    assert_eq!(
        processing.software,
        Some(format!("Astro Sync {} · Siril 1.4.4", crate::version()))
    );
    let steps = processing.steps.unwrap();
    // The pipeline first, then the steps in the order they ran, each with
    // its parameters as the record of the combination gives them.
    assert!(
        steps.starts_with(
            "pipeline natural (raw) · alignment Siril 2-pass affine · level match aperture r=11.5 px scales 1.012/0.998/1 · weighted_mean 2 nights weights 0.552/0.448 · no data 0.8 %"
        ),
        "{steps}"
    );
    assert!(steps.chars().count() <= crate::metadata::MAX_STEPS);

    // It leaves with the image, as the capture's processing.
    let queue = UploadQueue::open(&dir.path().join("queue.sqlite")).unwrap();
    queue.enqueue_results(&plan_results(&outcome)).unwrap();
    let entry = queue.list().unwrap().remove(0);
    let json = serde_json::to_value(build_metadata(&entry, &UploadTarget::default())).unwrap();
    assert_eq!(
        json["processing"]["software"],
        format!("Astro Sync {} · Siril 1.4.4", crate::version())
    );
    assert_eq!(json["metadataSource"], "astro-sync");

    // A pipeline of the person's own is named by its own name.
    job.pipeline.pipeline_id = "my-render".to_string();
    job.pipeline_label = Some("My render".to_string());
    let custom = run_plan_image(&imager, &job, gate, &CancelFlag::new(), &mut |_| {}).unwrap();
    assert!(
        custom
            .processing
            .unwrap()
            .steps
            .unwrap()
            .starts_with("pipeline My render (my-render) · ")
    );

    // A row queued without any processing (an earlier build) is given the
    // software of Astro Sync, never nothing.
    let mut old = entry.clone();
    old.target.metadata = None;
    let json = serde_json::to_value(build_metadata(&old, &UploadTarget::default())).unwrap();
    assert_eq!(
        json["processing"]["software"],
        format!("Astro Sync {}", crate::version())
    );
    assert!(
        json["processing"]["steps"]
            .as_str()
            .unwrap()
            .contains("2 nights")
    );
    assert_eq!(json["metadataSource"], "astro-sync");
}

/// The image of a multi-night plan the space does not hold yet, made from
/// the import's own nights (their provisional folders), then its folder
/// renamed once the plan is created.
#[test]
fn the_image_of_a_plan_to_create_is_made_from_the_import_s_nights() {
    use crate::session_detection::provisional::{Settled, settle_nights, settle_plans};
    use crate::working_folder::{PendingNight, read_night_identity, write_night_identity};

    let dir = TempDir::new("plan-image-local");
    let root = dir.path();
    let ninth_id = "local-aaaaaaaaaaaa";
    let twelfth_id = "local-bbbbbbbbbbbb";
    let ninth = night_folder(root, "2026-08-09 · Cœur · local-aaaaaa", ninth_id);
    write_master(
        &ninth.join("IC 1805/stacked"),
        "L-eXtreme_180s_g100",
        Some("L-eXtreme"),
        180.0,
        98,
    );
    let twelfth = night_folder(root, "2026-08-12 · Cœur · local-bbbbbb", twelfth_id);
    write_master(
        &twelfth.join("IC 1805/stacked"),
        "UV-IR-cut_120s_g100",
        Some("UV/IR-cut"),
        120.0,
        120,
    );
    // A night of the space this plan does not name: never read.
    night_folder(root, "2026-08-07 · M31 · n-07", "n-07");
    let project = NightProject {
        project_id: "local-pppppppppppp".into(),
        target_id: "ic-1805".into(),
        name: "IC 1805".into(),
        common_name: None,
        integrations: Vec::new(),
        total_seconds: 0.0,
        night_count: 2,
        plan_capture: None,
    };
    let nights = vec![ninth_id.to_string(), twelfth_id.to_string()];
    let lines = local_plan_lines(root, &project, &nights);
    let summary: Vec<(&str, &str, Place, bool)> = lines
        .iter()
        .map(|l| {
            (
                l.night_id.as_str(),
                l.evening_date.as_str(),
                l.place,
                l.master.is_some(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            (ninth_id, "2026-08-09", Place::ThisComputer, true),
            (twelfth_id, "2026-08-12", Place::ThisComputer, true),
        ]
    );
    let plan_dir = plan_folder(root, &project).unwrap();
    assert_eq!(
        plan_dir.file_name().unwrap().to_string_lossy(),
        "IC 1805 · local-pppppp"
    );
    let chosen: Vec<String> = lines.iter().map(|l| l.key.clone()).collect();
    let PlanImagePlan::Job(job) = plan_image_job(
        &project,
        &lines,
        &chosen,
        &plan_dir,
        Some(PipelineChoice::new("dualband-correction")),
    ) else {
        panic!("a job")
    };
    let imager = FakeImager {
        calls: Cell::new(0),
    };
    let open = PlanImageGate {
        allowed: true,
        tool_present: true,
        image_quality: false,
    };
    let outcome = run_plan_image(&imager, &job, open, &CancelFlag::new(), &mut |_| {}).unwrap();
    assert_eq!(outcome.project_id, "local-pppppppppppp");
    assert_eq!(outcome.combined_night_ids, nights);
    // What Astro Sync keeps of the plan names the provisional nights.
    fs::write(
        plan_dir.join("stacked/sources.json"),
        serde_json::json!({
            "projectId": project.project_id,
            "masters": [job.masters[0].path, job.masters[1].path],
            "nights": nights,
        })
        .to_string(),
    )
    .unwrap();

    // The space created the sessions and the plan: the nights, then the plan,
    // take their names.
    for (folder, created) in [(&ninth, "sess-0909"), (&twelfth, "sess-1212")] {
        let mut identity = read_night_identity(folder).unwrap();
        identity.pending = Some(PendingNight {
            pending_session_id: Some(created.into()),
            folder: Some(folder.clone()),
            detected_id: "d".into(),
            evening_date: folder.file_name().unwrap().to_string_lossy()[..10].into(),
            place: "Cœur".into(),
            targets: Vec::new(),
            equipment_ids: Vec::new(),
            calibration: Vec::new(),
            logs: Vec::new(),
        });
        write_night_identity(folder, &identity).unwrap();
    }
    let mut identity = read_plan_identity(&plan_dir).unwrap();
    identity.pending = Some(PendingPlan {
        pending_project_id: Some("prj-777".into()),
        folder: Some(plan_dir.clone()),
        group_id: "g-1".into(),
    });
    write_plan_identity(&plan_dir, &identity).unwrap();
    let settled: Vec<Settled> = settle_nights(root, None).unwrap();
    assert_eq!(settled.len(), 2);
    let plans = settle_plans(root, &settled, None).unwrap();
    assert_eq!(plans.len(), 1);
    let renamed = root.join("IC 1805 · prj-77");
    assert_eq!(plans[0].to, renamed);
    assert!(!plan_dir.exists());
    let identity = read_plan_identity(&renamed).unwrap();
    assert_eq!(identity.project_id, "prj-777");
    assert!(identity.pending.is_none());
    assert!(find_plan_folder(root, "prj-777").is_some());
    let kept: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(renamed.join("stacked/sources.json")).unwrap())
            .unwrap();
    assert_eq!(kept["projectId"], "prj-777");
    assert_eq!(
        kept["nights"],
        serde_json::json!(["sess-0909", "sess-1212"])
    );
    let first = PathBuf::from(kept["masters"][0].as_str().unwrap());
    assert!(first.starts_with(root.join("2026-08-09 · Cœur · sess-0")));
    assert!(first.is_file(), "{}", first.display());
    // The plan's image, made under the provisional plan, is still there.
    assert!(
        renamed
            .join("stacked/IC_1805_plan_dualband-correction.tif")
            .is_file()
    );
}

/* ---------------- One capture per plan, the other renders are its variants ---------------- */

fn outcome_of(pipeline: &str, quality: bool) -> PlanImageOutcome {
    PlanImageOutcome {
        project_id: "project-1".into(),
        target_id: "sharpless-103".into(),
        render: StackedRender {
            pipeline_id: pipeline.into(),
            contrast: crate::stacking::Contrast::None,
            retouch: crate::stacking::Retouch::default(),
            path: PathBuf::from(format!("/x/stacked/Veil_plan_{pipeline}.tif")),
            sha256: format!("{pipeline:0<64}"),
            size: 10,
            family: None,
        },
        finishes: Vec::new(),
        other_files: Vec::new(),
        combined_night_ids: vec!["n1".into(), "n2".into()],
        totals_by_filter: Vec::new(),
        plan_render: plan_render_of(pipeline),
        image_quality: quality.then(|| ImageQuality {
            fwhm_pixels: 3.0,
            fwhm_arcsec: None,
            eccentricity: None,
            star_count: 100,
            measured_on: "linear".into(),
            measured_at: "2026-08-12T00:00:00Z".into(),
            tool: crate::metadata::QualityTool {
                name: "Astro Sync".into(),
                version: "1".into(),
            },
        }),
        processing: Some(crate::metadata::processing_proposal(
            "Astro Sync",
            &[format!("pipeline {pipeline}")],
        )),
        generated_title: Some(GeneratedText::from([(
            "en".to_string(),
            format!("Title {pipeline}"),
        )])),
        generated_description: Some(GeneratedText::from([(
            "en".to_string(),
            format!("About {pipeline}"),
        )])),
    }
}

#[test]
fn the_renders_of_a_plan_are_one_capture_and_its_variants_named_after_their_treatment() {
    let outcomes = [
        outcome_of("raw", true),
        outcome_of("dualband-correction", true),
        outcome_of("hoo", false),
    ];
    // The pipeline asked as the main one becomes the capture, whatever its place.
    let results = plan_results_of(&outcomes, Some("dualband-correction"), None);
    assert_eq!(results.len(), 3);
    let main = &results[0];
    assert_eq!(main.target.variant_of, None);
    assert!(main.target.plan.is_some(), "the capture is the plan's");
    assert_eq!(main.path, outcomes[1].render.path);
    let metadata = main.target.metadata.as_ref().unwrap();
    assert!(metadata.processing.is_some());
    assert_eq!(
        metadata
            .variant_generated_title
            .as_ref()
            .map(|t| t["en"].as_str()),
        Some("Title dualband-correction")
    );
    for (variant, source) in results[1..].iter().zip([&outcomes[0], &outcomes[2]]) {
        assert_eq!(variant.path, source.render.path);
        assert_eq!(
            variant.target.variant_of,
            Some(format!("sha256:{}", main.sha256))
        );
        let metadata = variant.target.metadata.as_ref().unwrap();
        // How the capture was made is the main image's to say.
        assert_eq!(metadata.processing, None);
        assert_eq!(metadata.variant_generated_title, source.generated_title);
        assert_eq!(
            metadata.variant_generated_description,
            source.generated_description
        );
        assert!(!metadata.has_capture_part());
    }
    // Without a main named: the first render made.
    let by_default = plan_results_of(&outcomes, None, None);
    assert_eq!(by_default[0].path, outcomes[0].render.path);
    // A main that was not made: the first one.
    let unknown = plan_results_of(&outcomes, Some("polished"), None);
    assert_eq!(unknown[0].path, outcomes[0].render.path);
    assert!(plan_results_of(&[], None, None).is_empty());
}

#[test]
fn the_variants_of_a_plan_leave_through_the_queue_as_images_of_one_capture() {
    let dir = TempDir::new("plan-image-variants-queue");
    let queue = UploadQueue::open(&dir.path().join("queue.sqlite")).unwrap();
    let outcomes = [outcome_of("raw", false), outcome_of("hoo", false)];
    queue
        .enqueue_results(&plan_results_of(&outcomes, None, None))
        .unwrap();
    let rows = queue.list().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter()
            .filter(|r| r.target.variant_of.is_none())
            .count(),
        1
    );
}

/// The contrast chosen in the wizard travels with the pipeline: the plan's
/// image gets its finish, not the default of the settings.
#[test]
fn the_contrast_chosen_for_a_pipeline_reaches_the_finish_of_the_plan() {
    use crate::stacking::{Contrast, PipelineFinish};
    let dir = TempDir::new("plan-image-own-finish");
    let (project, current) = heart_here(dir.path());
    let lines = plan_lines(dir.path(), &project, Some(&current));
    let plan_dir = plan_folder(dir.path(), &project).unwrap();
    let chosen: Vec<String> = lines.iter().map(|l| l.key.clone()).collect();
    let default = PipelineFinish {
        contrast: Contrast::Presentation,
        ..PipelineFinish::default()
    };
    for (own, expected) in [
        (Contrast::Strong, "strong"),
        (Contrast::from_gain(3.35).unwrap(), "contrast-3.35"),
    ] {
        let choice = PipelineChoice::new("raw").with_finish(PipelineFinish {
            contrast: own,
            ..PipelineFinish::default()
        });
        let PlanImagePlan::Job(mut job) =
            plan_image_job(&project, &lines, &chosen, &plan_dir, Some(choice))
        else {
            panic!("a job")
        };
        job.apply_finish(default);
        assert_eq!(job.contrast, own);
        let open = PlanImageGate {
            allowed: true,
            tool_present: true,
            image_quality: false,
        };
        let imager = FakeImager {
            calls: Cell::new(0),
        };
        let outcome = run_plan_image(&imager, &job, open, &CancelFlag::new(), &mut |_| {}).unwrap();
        assert_eq!(outcome.finishes.len(), 1);
        let file = outcome.finishes[0].render.path.file_name().unwrap();
        assert!(file.to_string_lossy().contains(expected), "{file:?}");
        let steps = format!("{:?}", outcome.finishes[0].processing);
        assert!(
            steps.contains(&format!("contrast {}", own.token())),
            "{steps}"
        );
    }
    // Without a finish of its own, the default of the settings.
    let PlanImagePlan::Job(mut job) = plan_image_job(
        &project,
        &lines,
        &chosen,
        &plan_dir,
        Some(PipelineChoice::new("raw")),
    ) else {
        panic!("a job")
    };
    job.apply_finish(default);
    assert_eq!(job.contrast, Contrast::Presentation);
}

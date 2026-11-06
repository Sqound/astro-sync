use super::*;
use crate::test_support::{TempDir, fits_bytes};
use std::cell::Cell;

/// The fake stacker of the tests.
enum Behaviour {
    Writes,
    WaitsForTheStop,
    Fails,
    WritesIntoFinal,
}

struct FakeStacker {
    behaviour: Behaviour,
    calls: Cell<u32>,
}

impl FakeStacker {
    fn new(behaviour: Behaviour) -> Self {
        Self {
            behaviour,
            calls: Cell::new(0),
        }
    }
}

impl Stacker for FakeStacker {
    fn stack(
        &self,
        job: &StackJob,
        cancel: &CancelFlag,
        on_event: &mut dyn FnMut(&StackEvent),
    ) -> Result<Vec<StackerOutput>, StackFailure> {
        self.calls.set(self.calls.get() + 1);
        // Like the script: a scratch tree in the job's work folder.
        fs::create_dir_all(job.work_dir.join("process")).unwrap();
        fs::write(job.work_dir.join("siril.log"), b"log").unwrap();
        on_event(&StackEvent::Step {
            step: "calibrate".to_string(),
            index: 1,
            total: 5,
        });
        // Like the script: the master of the family kept.
        if matches!(self.behaviour, Behaviour::Writes)
            && let Some(family) = &job.integration
        {
            let frames = job.light_count().to_string();
            let cards = [
                ("NAXIS", "3"),
                ("ASFAMILY", &*format!("'{family}'")),
                ("FILTER", "'Ha'"),
                ("EXPTIME", "300.0"),
                ("GAIN", "100"),
                ("XBINNING", "1"),
                ("CCD-TEMP", "-10.0"),
                ("INSTRUME", "'ZWO ASI2600MC Pro'"),
                ("STACKCNT", &*frames),
            ];
            fs::write(integration_path(job, family), fits_bytes(&cards, 16)).unwrap();
        }
        match self.behaviour {
            Behaviour::Writes => Ok(job
                .pipelines
                .iter()
                .flat_map(|p| {
                    // Like the script: "No processing" writes the TIFF only.
                    let formats: &[&str] = if p.is_linear() {
                        &["tif"]
                    } else {
                        &["tif", "png"]
                    };
                    formats
                        .iter()
                        .map(|ext| {
                            let file = format!("{}_{}.{ext}", job.name, p.pipeline_id);
                            fs::write(
                                job.output_dir.join(&file),
                                format!("{} {ext}", p.pipeline_id),
                            )
                            .unwrap();
                            StackerOutput {
                                pipeline: p.pipeline_id.clone(),
                                file,
                                contrast: Contrast::None,
                                retouch: Default::default(),
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .collect()),
            Behaviour::WaitsForTheStop => {
                cancel.cancel();
                Err(StackFailure::Cancelled)
            }
            Behaviour::Fails => Err(StackFailure::Failed {
                code: "siril_failed".to_string(),
                message: "Siril failed (1)".to_string(),
                logs: None,
            }),
            Behaviour::WritesIntoFinal => {
                let file = job.output_dir.parent().unwrap().join("final/M31.tif");
                fs::write(&file, b"x").unwrap();
                Ok(vec![StackerOutput {
                    pipeline: "raw".to_string(),
                    file: "../final/M31.tif".to_string(),
                    contrast: Contrast::None,
                    retouch: Default::default(),
                }])
            }
        }
    }
}

/// A night with M31's lights and darks.
fn night(dir: &Path) -> PathBuf {
    let night = dir.join("2026-10-07 · Col des Étoiles · 7f3a2c");
    for folder in [
        "M31/lights",
        "M31/final",
        "calibration/darks",
        "calibration/flats",
    ] {
        fs::create_dir_all(night.join(folder)).unwrap();
    }
    let light = fits_bytes(&[("IMAGETYP", "'LIGHT'"), ("OBJECT", "'M31'")], 0);
    fs::write(night.join("M31/lights/Light_0001.fits"), &light).unwrap();
    fs::write(night.join("calibration/darks/Dark_0001.fits"), b"dark").unwrap();
    night
}

const OPEN: StackGate = StackGate {
    tool_present: true,
    variant_limit: 12,
};

#[test]
fn the_stacked_images_go_to_stacked_and_leave_as_stacked_results() {
    let dir = TempDir::new("stacking-success");
    let night = night(dir.path());
    let choices = [
        PipelineChoice::new("raw"),
        PipelineChoice::new("dualband-correction"),
    ];
    let job = plan_stack(&night, "M31", "2026-10-07", &choices);
    assert_eq!(job.darks, Some(night.join("calibration/darks")));
    assert_eq!(job.flats, None, "an empty calibration folder is not passed");
    assert_eq!(job.output_dir, night.join("M31/stacked"));
    assert_eq!(job.name, "M31_2026-10-07");

    let stacker = FakeStacker::new(Behaviour::Writes);
    let mut events = Vec::new();
    let outcome = run_stacking(&stacker, &job, OPEN, &CancelFlag::new(), None, &mut |e| {
        events.push(e.clone())
    })
    .unwrap();
    assert_eq!(events.len(), 1);
    let ids: Vec<&str> = outcome
        .renders
        .iter()
        .map(|r| r.pipeline_id.as_str())
        .collect();
    assert_eq!(ids, vec!["raw", "dualband-correction"]);
    assert_eq!(outcome.other_files.len(), 2);
    assert!(
        outcome
            .renders
            .iter()
            .all(|r| r.path.starts_with(night.join("M31/stacked")))
    );
    assert!(
        fs::read_dir(night.join("M31/final"))
            .unwrap()
            .next()
            .is_none()
    );

    let candidates = stacked_candidates(&outcome, "t-m31");
    assert!(candidates.iter().all(|c| c.kind == ResultKind::Stacked));
    assert_eq!(candidates[1].render.as_deref(), Some("dualband-correction"));
    assert_eq!(candidates[0].size, 7);

    // A second stacking never replaces the first one's files.
    let again = plan_stack(&night, "M31", "2026-10-07", &choices);
    assert_eq!(again.name, "M31_2026-10-07-2");
}

#[test]
fn without_processing_alone_leaves_the_linear_tiff_only() {
    let dir = TempDir::new("stacking-linear");
    let night = night(dir.path());
    // Beside renders, "No processing" is written too,
    // after them (`--linear`), so the first render stays the capture's image;
    // each pipeline is kept once.
    let both = plan_stack(
        &night,
        "M31",
        "2026-10-07",
        &[
            PipelineChoice::linear(),
            PipelineChoice::new("raw"),
            PipelineChoice::new("hoo"),
            PipelineChoice::new("raw"),
        ],
    );
    let ids: Vec<&str> = both
        .pipelines
        .iter()
        .map(|p| p.pipeline_id.as_str())
        .collect();
    assert_eq!(ids, ["raw", "hoo", "linear"]);
    assert!(!both.is_linear_only() && both.adds_linear());
    let joined = stack_args(&both, &[], None, None, None).join(" ");
    assert!(
        joined.contains("--pipeline raw --pipeline hoo --linear")
            && !joined.contains("--linear-only"),
        "{joined}"
    );
    // Alone, it runs no render.
    let job = plan_stack(&night, "M31", "2026-10-07", &[PipelineChoice::linear()]);
    assert_eq!(job.pipelines, vec![PipelineChoice::linear()]);
    assert!(job.is_linear_only() && !job.adds_linear());
    assert!(!plan_stack(&night, "M31", "2026-10-07", &[]).is_linear_only());

    let stacker = FakeStacker::new(Behaviour::Writes);
    // One choice, so a space with no variant accepts it.
    let gate = StackGate {
        variant_limit: 0,
        ..OPEN
    };
    let outcome =
        run_stacking(&stacker, &job, gate, &CancelFlag::new(), None, &mut |_| {}).unwrap();
    assert_eq!(outcome.renders.len(), 1);
    assert_eq!(outcome.renders[0].pipeline_id, "linear");
    assert_eq!(
        outcome.renders[0].path,
        night.join("M31/stacked/M31_2026-10-07_linear.tif")
    );
    assert!(outcome.other_files.is_empty(), "no PNG");
    // It leaves like a render: a stacked result whose `render` is `linear`.
    let candidates = stacked_candidates(&outcome, "t-m31");
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].kind, ResultKind::Stacked);
    assert_eq!(candidates[0].render.as_deref(), Some("linear"));

    // A second stacking never replaces the first one's linear file.
    let again = plan_stack(&night, "M31", "2026-10-07", &[PipelineChoice::linear()]);
    assert_eq!(again.name, "M31_2026-10-07-2");
}

#[test]
fn without_processing_runs_the_script_with_linear_only_and_nothing_of_the_renders() {
    let dir = TempDir::new("stacking-linear-args");
    let night = night(dir.path());
    let mut linear = plan_stack(&night, "M31", "2026-10-07", &[PipelineChoice::linear()]);
    linear.contrast = Contrast::Strong;
    let args = stack_args(
        &linear,
        &[dir.path().join("data/pipelines")],
        Some(Path::new("/opt/siril-cli")),
        Some(Path::new("/opt/starnet2")),
        Some(Path::new("/opt/graxpert")),
    );
    let joined = args.join(" ");
    assert!(joined.contains("--linear-only"), "{joined}");
    // No render: no StarNet2, no GraXpert, and no contrast twin.
    for refused in [
        "--pipeline",
        "--trusted",
        "--pipelines-dir",
        "--starnet",
        "--graxpert",
        "--contrast",
    ] {
        assert!(!joined.contains(refused), "{refused} in {joined}");
    }
    assert!(joined.contains("--siril /opt/siril-cli") && joined.contains("--darks"));
    // A render keeps its arguments, and never gets the option.
    let raw = plan_stack(&night, "M31", "2026-10-07", &[]);
    let joined = stack_args(&raw, &[], None, Some(Path::new("/opt/starnet2")), None).join(" ");
    assert!(!joined.contains("--linear-only"));
    assert!(joined.contains("--pipeline raw") && joined.contains("--starnet"));
}

#[cfg(unix)]
#[test]
fn without_processing_stacks_each_family_and_the_launcher_passes_the_option() {
    use crate::external_process::fake_tool;
    let dir = TempDir::new("stacking-linear-families");
    let night = night(dir.path());
    light(&night, "Light_Ha_1.fits", "Ha", "300", "-10");
    light(&night, "Light_Ha_2.fits", "Ha", "300", "-10");
    light(&night, "Light_OIII_1.fits", "OIII", "300", "-10");
    let plan = plan_target(&night, "M31", "2026-10-07", &[PipelineChoice::linear()], 12);
    assert!(plan.jobs.len() >= 2, "{plan:?}");
    assert!(plan.jobs.iter().all(StackJob::is_linear_only));

    // The real launcher: the fake interpreter records what it was given.
    let job = &plan.jobs[0];
    fs::create_dir_all(&job.output_dir).unwrap();
    let record = dir.path().join("arguments.txt");
    let out = job.output_dir.join(format!("{}_linear.tif", job.name));
    let python = fake_tool(
        dir.path(),
        "python3",
        &format!(
            "echo \"$@\" > '{}'\nprintf 'tif' > '{}'\necho '{{\"event\": \"output\", \"pipeline\": \"linear\", \"file\": \"{}_linear.tif\"}}'",
            record.display(),
            out.display(),
            job.name
        ),
    );
    let stacker = PythonStacker {
        python,
        script: dir.path().join("astro_sync_stack.py"),
        pipelines_dirs: vec![dir.path().join("pipelines")],
        siril: None,
        starnet: Some(PathBuf::from("/opt/starnet2")),
        graxpert: None,
        timeout: Duration::from_secs(30),
    };
    let outcome = run_stacking(&stacker, job, OPEN, &CancelFlag::new(), None, &mut |_| {}).unwrap();
    assert_eq!(outcome.renders[0].pipeline_id, "linear");
    let given = fs::read_to_string(record).unwrap();
    assert!(given.contains("--linear-only"), "{given}");
    assert!(
        !given.contains("--pipeline") && !given.contains("--starnet"),
        "{given}"
    );
}

/// The colour intensity and the softened stars are words of the job, given to
/// the script only when chosen, flattened beside the contrast in the job file;
/// the script's `output` events say which image carries them, and they are what
/// makes an image a variant (never the main one).
#[cfg(unix)]
#[test]
fn the_finish_goes_to_the_script_and_comes_back_on_the_images() {
    use crate::external_process::fake_tool;
    let dir = TempDir::new("stacking-finish");
    let night = night(dir.path());
    // Numbers, and the words the first version used.
    assert_eq!("40".parse::<Colour>(), Ok(Colour::Strong));
    assert_eq!("enhanced".parse::<Colour>(), Ok(Colour::Enhanced));
    assert_eq!("35".parse::<Colour>().map(Colour::percent), Ok(35));
    assert_eq!("85".parse::<SoftStars>(), Ok(SoftStars::Strong));
    assert_eq!("light".parse::<SoftStars>(), Ok(SoftStars::Light));
    assert_eq!("none".parse::<SoftStars>(), Ok(SoftStars::None));
    for bad in ["vivid", "101", "-1", "nan"] {
        assert!(bad.parse::<Colour>().is_err(), "{bad}");
        assert!(bad.parse::<SoftStars>().is_err(), "{bad}");
    }
    assert_eq!(
        [Colour::None, Colour::Enhanced, Colour::Strong].map(|c| (c.token(), c.gain())),
        [
            ("none".to_string(), None),
            ("20".to_string(), Some(1.2)),
            ("40".to_string(), Some(1.4))
        ]
    );
    assert_eq!(
        [SoftStars::None, SoftStars::Light, SoftStars::Strong].map(|s| (s.token(), s.percent())),
        [
            ("none".to_string(), None),
            ("50".to_string(), Some(50)),
            ("85".to_string(), Some(85))
        ]
    );
    let plain = plan_stack(&night, "M31", "2026-10-07", &[]);
    assert!(plain.retouch.is_none());
    let joined = stack_args(&plain, &[], None, None, None).join(" ");
    assert!(
        !joined.contains("--colour") && !joined.contains("--soft-stars"),
        "{joined}"
    );
    let written = serde_json::to_string(&plain).unwrap();
    assert!(
        !written.contains("\"colour\"") && !written.contains("\"softStars\""),
        "the defaults write nothing: {written}"
    );

    let retouch = Retouch {
        colour: Colour::Enhanced,
        soft_stars: SoftStars::Light,
    };
    let plan = plan_target(&night, "M31", "2026-10-07", &[], 12).with_retouch(retouch);
    let job = &plan.jobs[0];
    assert_eq!(job.contrast, Contrast::None);
    let joined = stack_args(job, &[], None, None, None).join(" ");
    assert!(joined.contains("--colour 20 --soft-stars 50"), "{joined}");
    assert!(!joined.contains("--contrast"), "{joined}");
    let written = serde_json::to_string(job).unwrap();
    assert!(
        written.contains("\"colour\":20") && written.contains("\"softStars\":50"),
        "{written}"
    );
    assert_eq!(&serde_json::from_str::<StackJob>(&written).unwrap(), job);
    // "No processing" takes no finish.
    let mut linear = job.clone();
    linear.pipelines = vec![PipelineChoice::new(LINEAR_PIPELINE)];
    let joined = stack_args(&linear, &[], None, None, None).join(" ");
    assert!(
        !joined.contains("--colour") && !joined.contains("--soft-stars"),
        "{joined}"
    );

    fs::create_dir_all(&job.output_dir).unwrap();
    let (base, twin) = (
        format!("{}_raw.tif", job.name),
        format!("{}_raw_colour-20_stars-50.tif", job.name),
    );
    let python = fake_tool(
        dir.path(),
        "python3",
        &format!(
            "printf 'a' > '{base_path}'\nprintf 'b' > '{twin_path}'\n\
             echo '{{\"event\": \"output\", \"pipeline\": \"raw\", \"file\": \"{base}\"}}'\n\
             echo '{{\"event\": \"output\", \"pipeline\": \"raw\", \"file\": \"{twin}\", \"colour\": \"20\", \"softStars\": \"50\"}}'",
            base_path = job.output_dir.join(&base).display(),
            twin_path = job.output_dir.join(&twin).display(),
        ),
    );
    let stacker = PythonStacker {
        python,
        script: dir.path().join("astro_sync_stack.py"),
        pipelines_dirs: Vec::new(),
        siril: None,
        starnet: None,
        graxpert: None,
        timeout: Duration::from_secs(30),
    };
    let mut events = Vec::new();
    let outcome = run_stacking(&stacker, job, OPEN, &CancelFlag::new(), None, &mut |e| {
        events.push(e.clone());
    })
    .unwrap();
    assert_eq!(
        outcome
            .renders
            .iter()
            .map(|r| r.retouch)
            .collect::<Vec<_>>(),
        [Retouch::default(), retouch]
    );
    assert!(events.iter().any(|e| matches!(
        e,
        StackEvent::Output { colour: Colour::Enhanced, soft_stars: SoftStars::Light, file, .. } if *file == twin
    )));
    let candidates = stacked_candidates(&outcome, "t-m31");
    assert_eq!(
        candidates.iter().map(|c| c.retouch).collect::<Vec<_>>(),
        [Retouch::default(), retouch]
    );
    assert_eq!(Retouch::default().suffix(), "");
    assert_eq!(retouch.suffix(), "_colour-20_stars-50");
}

/// The contrast of presentation is a word of the job, given to the
/// script only when chosen; the script's `output` events say which image
/// carries the curve, and the outcome keeps the render without it first.
#[cfg(unix)]
#[test]
fn the_contrast_of_presentation_goes_to_the_script_and_comes_back_on_the_images() {
    use crate::external_process::fake_tool;
    let dir = TempDir::new("stacking-contrast");
    let night = night(dir.path());
    assert_eq!("strong".parse::<Contrast>(), Ok(Contrast::Strong));
    assert!("extreme".parse::<Contrast>().is_err());
    assert_eq!(
        [Contrast::None, Contrast::Presentation, Contrast::Strong].map(|c| (
            c.token(),
            c.gain(),
            c.suffix()
        )),
        [
            ("none".to_string(), None, String::new()),
            (
                "presentation".to_string(),
                Some(2.0),
                "_presentation".to_string()
            ),
            ("strong".to_string(), Some(3.6), "_strong".to_string())
        ]
    );
    // Any gain from 0 to 6, as a number (`2.5`) or in the job file; the two
    // values the window always offered keep their words.
    assert_eq!("2.5".parse::<Contrast>().map(Contrast::gain), Ok(Some(2.5)));
    assert_eq!("2.0".parse::<Contrast>(), Ok(Contrast::Presentation));
    assert_eq!("3.6".parse::<Contrast>(), Ok(Contrast::Strong));
    assert_eq!("0".parse::<Contrast>(), Ok(Contrast::None));
    for bad in ["6.1", "-1", "nan", "loud"] {
        assert!(bad.parse::<Contrast>().is_err(), "{bad}");
    }
    let custom = Contrast::from_gain(2.5).unwrap();
    assert_eq!(
        (custom.token(), custom.suffix()),
        ("2.5".to_string(), "_contrast-2.5".to_string())
    );
    assert_eq!(serde_json::to_string(&custom).unwrap(), "2.5");
    assert_eq!(
        serde_json::to_string(&Contrast::Strong).unwrap(),
        "\"strong\""
    );
    for json in ["2.5", "\"2.5\"", "\"2.50\""] {
        assert_eq!(
            serde_json::from_str::<Contrast>(json).unwrap(),
            custom,
            "{json}"
        );
    }
    assert_eq!(
        serde_json::from_str::<Contrast>("3.6").unwrap(),
        Contrast::Strong
    );
    assert_eq!(
        serde_json::from_str::<Contrast>("0").unwrap(),
        Contrast::None
    );
    assert!(serde_json::from_str::<Contrast>("7").is_err());
    assert_eq!(serde_json::from_str::<Colour>("35").unwrap().percent(), 35);
    assert_eq!(
        serde_json::from_str::<SoftStars>("\"light\"").unwrap(),
        SoftStars::Light
    );
    // The job the script reads carries the number.
    let plan = plan_target(&night, "M31", "2026-10-07", &[], 12).with_contrast(custom);
    let joined = stack_args(&plan.jobs[0], &[], None, None, None).join(" ");
    assert!(joined.contains("--contrast 2.5"), "{joined}");
    let plain = plan_stack(&night, "M31", "2026-10-07", &[]);
    assert_eq!(plain.contrast, Contrast::None);
    let joined = stack_args(&plain, &[], None, None, None).join(" ");
    assert!(!joined.contains("--contrast"), "{joined}");
    assert!(
        !serde_json::to_string(&plain)
            .unwrap()
            .contains("\"contrast\""),
        "the default writes nothing"
    );

    let plan =
        plan_target(&night, "M31", "2026-10-07", &[], 12).with_contrast(Contrast::Presentation);
    let job = &plan.jobs[0];
    assert_eq!(job.contrast, Contrast::Presentation);
    let joined = stack_args(job, &[], None, None, None).join(" ");
    assert!(joined.contains("--contrast presentation"), "{joined}");

    // The real launcher: the fake interpreter says the two images, the curve's last.
    fs::create_dir_all(&job.output_dir).unwrap();
    let record = dir.path().join("arguments.txt");
    let (base, curved) = (
        format!("{}_raw.tif", job.name),
        format!("{}_raw_presentation.tif", job.name),
    );
    let python = fake_tool(
        dir.path(),
        "python3",
        &format!(
            "echo \"$@\" > '{record}'\nprintf 'a' > '{base_path}'\nprintf 'b' > '{curved_path}'\n\
             echo '{{\"event\": \"output\", \"pipeline\": \"raw\", \"file\": \"{base}\"}}'\n\
             echo '{{\"event\": \"output\", \"pipeline\": \"raw\", \"file\": \"{curved}\", \"contrast\": \"presentation\"}}'",
            record = record.display(),
            base_path = job.output_dir.join(&base).display(),
            curved_path = job.output_dir.join(&curved).display(),
        ),
    );
    let stacker = PythonStacker {
        python,
        script: dir.path().join("astro_sync_stack.py"),
        pipelines_dirs: Vec::new(),
        siril: None,
        starnet: None,
        graxpert: None,
        timeout: Duration::from_secs(30),
    };
    let mut events = Vec::new();
    let outcome = run_stacking(&stacker, job, OPEN, &CancelFlag::new(), None, &mut |e| {
        events.push(e.clone());
    })
    .unwrap();
    assert!(
        fs::read_to_string(record)
            .unwrap()
            .contains("--contrast presentation")
    );
    assert_eq!(
        outcome
            .renders
            .iter()
            .map(|r| (r.pipeline_id.as_str(), r.contrast))
            .collect::<Vec<_>>(),
        [("raw", Contrast::None), ("raw", Contrast::Presentation)]
    );
    assert!(events.iter().any(|e| matches!(
        e,
        StackEvent::Output { contrast: Contrast::Presentation, file, .. } if *file == curved
    )));
    let candidates = stacked_candidates(&outcome, "t-m31");
    assert_eq!(
        candidates.iter().map(|c| c.contrast).collect::<Vec<_>>(),
        [Contrast::None, Contrast::Presentation]
    );
}

#[test]
fn the_gate_and_the_renders_follow_the_capabilities() {
    let dir = TempDir::new("stacking-gate");
    let night = night(dir.path());
    let two = [
        PipelineChoice::new("raw"),
        PipelineChoice::new("dualband-correction"),
    ];
    let job = plan_stack(&night, "M31", "2026-10-07", &two);
    let stacker = FakeStacker::new(Behaviour::Writes);
    let cancel = CancelFlag::new();
    let refused = |gate: StackGate| {
        run_stacking(&stacker, &job, gate, &cancel, None, &mut |_| {}).unwrap_err()
    };
    assert_eq!(
        refused(StackGate {
            tool_present: false,
            ..OPEN
        }),
        StackFailure::ToolMissing
    );
    // No variant kept: one render only, chosen before; nothing runs.
    assert_eq!(
        refused(StackGate {
            variant_limit: 0,
            ..OPEN
        }),
        StackFailure::OneRenderOnly { asked: 2 }
    );
    assert_eq!(stacker.calls.get(), 0);
    let one = plan_stack(&night, "M31", "2026-10-07", &[]);
    assert_eq!(one.pipelines, vec![PipelineChoice::new("raw")]);
    run_stacking(
        &stacker,
        &one,
        StackGate {
            variant_limit: 0,
            ..OPEN
        },
        &cancel,
        None,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(stacker.calls.get(), 1);

    // The gate read from the capabilities.
    let mut capabilities: Capabilities = serde_json::from_value(serde_json::json!({
        "planner.multiple": true, "planner.mosaic": true, "pointing.solve": true,
        "sync.plan_limit": 0, "gallery.private": true, "portal.invitations": true,
        "capture.variant_limit": 12, "capture.count_limit": 0, "capture.public": true,
        "analysis.logs": true, "analysis.progression": true,
        "storage.max_file_bytes": 1, "storage.quota_bytes": 1, "history.retention_days": 0
    }))
    .unwrap();
    assert_eq!(StackGate::new(&capabilities, true), OPEN);
    // The stacking is local and never gated: a space without
    // `analysis.progression` (the free account) stacks, and so does no space
    // at all, with no limit on the renders.
    capabilities.analysis_progression = false;
    assert_eq!(StackGate::new(&capabilities, true), OPEN);
    assert_eq!(StackGate::of(Some(&capabilities), true), OPEN);
    assert_eq!(
        StackGate::of(None, true),
        StackGate {
            tool_present: true,
            variant_limit: crate::stacking::UNLIMITED,
        }
    );
    assert_eq!(StackGate::without_space(true).refusal(&job), None);
    let free = run_stacking(
        &stacker,
        &job,
        StackGate::new(&capabilities, true),
        &cancel,
        None,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(free.renders.len(), 2);
    let alone = run_stacking(
        &stacker,
        &job,
        StackGate::without_space(true),
        &cancel,
        None,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(alone.renders.len(), 2);
}

#[test]
fn a_stop_a_failure_and_a_file_outside_stacked() {
    let dir = TempDir::new("stacking-failures");
    let night = night(dir.path());
    let job = plan_stack(&night, "M31", "2026-10-07", &[]);
    let cancel = CancelFlag::new();
    assert_eq!(
        run_stacking(
            &FakeStacker::new(Behaviour::WaitsForTheStop),
            &job,
            OPEN,
            &cancel,
            None,
            &mut |_| {}
        )
        .unwrap_err(),
        StackFailure::Cancelled
    );
    match run_stacking(
        &FakeStacker::new(Behaviour::Fails),
        &job,
        OPEN,
        &CancelFlag::new(),
        None,
        &mut |_| {},
    )
    .unwrap_err()
    {
        StackFailure::Failed { code, message, .. } => {
            assert_eq!(code, "siril_failed");
            assert!(message.contains("Siril"));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        run_stacking(
            &FakeStacker::new(Behaviour::WritesIntoFinal),
            &job,
            OPEN,
            &CancelFlag::new(),
            None,
            &mut |_| {}
        )
        .unwrap_err(),
        StackFailure::OutsideStacked {
            file: "../final/M31.tif".to_string()
        }
    );
    // No lights: nothing runs.
    let empty = plan_stack(&night, "IC 1396", "2026-10-07", &[]);
    assert_eq!(
        run_stacking(
            &FakeStacker::new(Behaviour::Writes),
            &empty,
            OPEN,
            &CancelFlag::new(),
            None,
            &mut |_| {}
        )
        .unwrap_err(),
        StackFailure::NoLights
    );
}

#[test]
fn the_launcher_gets_the_pipelines_and_the_trusted_digest() {
    let dir = TempDir::new("stacking-args");
    let night = night(dir.path());
    let choices = [
        PipelineChoice::new("raw"),
        PipelineChoice {
            pipeline_id: "mono-ha".to_string(),
            trusted_sha256: Some("AB".repeat(32)),
            finish: None,
        },
    ];
    let job = plan_stack(&night, "M31", "2026-10-07", &choices);
    let args = stack_args(
        &job,
        &[dir.path().join("data/pipelines")],
        Some(Path::new("/opt/siril-cli")),
        Some(Path::new("/opt/starnet2")),
        Some(Path::new("/opt/graxpert")),
    );
    let joined = args.join(" ");
    assert!(
        joined.contains("--pipeline raw --pipeline mono-ha"),
        "{joined}"
    );
    assert!(joined.contains(&format!("--trusted mono-ha={}", "ab".repeat(32))));
    assert!(!joined.contains("--trusted raw"));
    assert!(joined.contains("--darks") && !joined.contains("--flats"));
    assert!(joined.contains("--siril /opt/siril-cli"));
    assert!(joined.contains("--starnet /opt/starnet2"));
    // GraXpert where "Dependencies" found it, for the "polished render".
    assert!(joined.contains("--graxpert /opt/graxpert"), "{joined}");
    assert!(
        !joined.contains("--frames-from"),
        "one family: the folder as it is"
    );
    assert_eq!(args.last().map(String::as_str), Some("--json"));
}

/// A pipeline with a finish of its own gets `--finish <id>=…`; one that
/// keeps the night's default, or names the default itself, gets nothing, and
/// "No processing" never does.
#[test]
fn a_pipeline_with_its_own_finish_is_given_it_and_the_others_keep_the_default() {
    let dir = TempDir::new("stacking-finish-per-pipeline");
    let night = night(dir.path());
    let own = PipelineFinish {
        contrast: Contrast::Strong,
        retouch: Retouch {
            colour: Colour::Enhanced,
            soft_stars: SoftStars::Light,
        },
    };
    let night_default = PipelineFinish {
        contrast: Contrast::Presentation,
        retouch: Retouch::default(),
    };
    let choices = [
        PipelineChoice::new("raw"),
        PipelineChoice::new("hoo").with_finish(own),
        PipelineChoice::new("dualband-correction").with_finish(night_default),
        PipelineChoice::new("polished").with_finish(PipelineFinish::default()),
        PipelineChoice::linear(),
    ];
    let mut job = plan_stack(&night, "M31", "2026-10-07", &choices);
    job.contrast = Contrast::Presentation;
    let args = stack_args(&job, &[], None, None, None).join(" ");
    assert!(args.contains("--contrast presentation"), "{args}");
    assert!(args.contains("--finish hoo=strong,20,50"), "{args}");
    assert!(args.contains("--finish polished=none,none,none"), "{args}");
    assert!(!args.contains("--finish raw"), "{args}");
    assert!(!args.contains("--finish dualband-correction"), "{args}");
    assert!(!args.contains("--finish linear"), "{args}");
    // The finish travels with the job file, and a choice without one reads as before.
    let written = serde_json::to_string(&job).unwrap();
    let read: StackJob = serde_json::from_str(&written).unwrap();
    assert_eq!(read.pipelines[1].finish, Some(own));
    assert_eq!(read.pipelines[0].finish, None);
    let old: PipelineChoice = serde_json::from_str(r#"{"pipelineId":"raw"}"#).unwrap();
    assert_eq!(old, PipelineChoice::new("raw"));
    let given: PipelineChoice = serde_json::from_str(
        r#"{"pipelineId":"hoo","finish":{"contrast":"strong","colour":20,"softStars":50}}"#,
    )
    .unwrap();
    assert_eq!(given.finish, Some(own));
    // "No processing" alone: no render, no finish.
    let linear = plan_stack(
        &night,
        "M31",
        "2026-10-07",
        &[PipelineChoice::linear().with_finish(own)],
    );
    assert!(
        !stack_args(&linear, &[], None, None, None)
            .join(" ")
            .contains("--finish")
    );
}

#[cfg(unix)]
#[test]
fn the_stacking_script_is_run_and_its_events_read() {
    use crate::external_process::fake_tool;
    let dir = TempDir::new("stacking-script");
    let night = night(dir.path());
    let job = plan_stack(&night, "M31", "2026-10-07", &[]);
    fs::create_dir_all(&job.output_dir).unwrap();
    let out = job.output_dir.join("M31_2026-10-07_raw.tif");
    let python = fake_tool(
        dir.path(),
        "python3",
        &format!(
            "echo '{{\"event\": \"step\", \"step\": \"register\", \"index\": 2, \"total\": 5}}'\necho 'not json'\nprintf 'tif' > '{}'\necho '{{\"event\": \"output\", \"pipeline\": \"raw\", \"file\": \"M31_2026-10-07_raw.tif\", \"bytes\": 3, \"sha256\": \"x\"}}'\necho '{{\"event\": \"done\"}}'",
            out.display()
        ),
    );
    let stacker = PythonStacker {
        python,
        script: dir.path().join("astro_sync_stack.py"),
        pipelines_dirs: Vec::new(),
        siril: None,
        starnet: None,
        graxpert: None,
        timeout: Duration::from_secs(30),
    };
    let mut events = Vec::new();
    let outcome = run_stacking(&stacker, &job, OPEN, &CancelFlag::new(), None, &mut |e| {
        events.push(e.clone())
    })
    .unwrap();
    assert_eq!(outcome.renders.len(), 1);
    assert_eq!(
        events[0],
        StackEvent::Step {
            step: "register".to_string(),
            index: 2,
            total: 5
        }
    );

    let refusing = fake_tool(
        dir.path(),
        "python-refusing",
        "echo '{\"event\": \"error\", \"code\": \"script_not_trusted\", \"message\": \"it changed since\"}'\nexit 2",
    );
    let stacker = PythonStacker {
        python: refusing,
        ..stacker
    };
    assert_eq!(
        run_stacking(&stacker, &job, OPEN, &CancelFlag::new(), None, &mut |_| {}).unwrap_err(),
        StackFailure::Failed {
            code: "script_not_trusted".to_string(),
            message: "it changed since".to_string(),
            logs: None,
        }
    );
}

#[cfg(unix)]
#[test]
fn siril_journal_is_read_while_the_script_runs_and_news_goes_out_as_progress() {
    use crate::external_process::fake_tool;
    let dir = TempDir::new("stacking-progress");
    let night = night(dir.path());
    let job = plan_stack(&night, "M31", "2026-10-07", &[]);
    fs::create_dir_all(&job.output_dir).unwrap();
    let out = job.output_dir.join("M31_2026-10-07_raw.tif");
    let log = job.work_dir.join("siril.log");
    // Siril's journal as it is on disk: terminal sequences and a stray
    // character before the carriage return on the progress lines.
    let python = fake_tool(
        dir.path(),
        "python3",
        &format!(
            "mkdir -p '{work}'\necho '{{\"event\": \"step\", \"step\": \"integration\", \"index\": 6, \"total\": 7}}'\nsleep 0.3\nprintf 'log: Integrating 90 images...\\n\\033[A\\033[2KT\\rprogress: 28.18%%\\n' >> '{log}'\nsleep 1.4\nprintf 'log: Rejection stacking complete.\\n' >> '{log}'\nsleep 1.1\nprintf 'tif' > '{out}'\necho '{{\"event\": \"output\", \"pipeline\": \"raw\", \"file\": \"M31_2026-10-07_raw.tif\", \"bytes\": 3, \"sha256\": \"x\"}}'",
            work = job.work_dir.display(),
            log = log.display(),
            out = out.display()
        ),
    );
    let stacker = PythonStacker {
        python,
        script: dir.path().join("astro_sync_stack.py"),
        pipelines_dirs: Vec::new(),
        siril: None,
        starnet: None,
        graxpert: None,
        timeout: Duration::from_secs(30),
    };
    let mut events = Vec::new();
    run_stacking(&stacker, &job, OPEN, &CancelFlag::new(), None, &mut |e| {
        events.push(e.clone());
    })
    .unwrap();
    let progress: Vec<(Option<f64>, Option<String>)> = events
        .iter()
        .filter_map(|event| match event {
            StackEvent::Progress { percent, line, .. } => Some((*percent, line.clone())),
            _ => None,
        })
        .collect();
    assert!(
        progress.contains(&(Some(28.18), Some("Integrating 90 images...".to_string()))),
        "{progress:?}"
    );
    assert_eq!(
        progress.last().unwrap().1.as_deref(),
        Some("Rejection stacking complete."),
        "the last line is the last one written: {progress:?}"
    );
    // The step comes before the news that belong to it.
    let step = events
        .iter()
        .position(|e| matches!(e, StackEvent::Step { .. }))
        .unwrap();
    let first = events
        .iter()
        .position(|e| matches!(e, StackEvent::Progress { .. }))
        .unwrap();
    assert!(step < first, "{events:?}");
}

#[test]
fn the_progress_event_reads_as_one_line_of_text_for_the_command_line() {
    let event = StackEvent::Progress {
        percent: Some(28.18),
        line: Some("Integrating 90 images…".to_string()),
        at: 1,
    };
    assert_eq!(
        event.progress_text().as_deref(),
        Some("28\u{a0}% · Integrating 90 images…")
    );
    let only_percent = StackEvent::Progress {
        percent: Some(3.0),
        line: None,
        at: 1,
    };
    assert_eq!(only_percent.progress_text().as_deref(), Some("3\u{a0}%"));
    assert!(
        StackEvent::Output {
            pipeline: "raw".to_string(),
            file: "x".to_string(),
            contrast: Contrast::None,
            colour: Colour::None,
            soft_stars: SoftStars::None,
        }
        .progress_text()
        .is_none()
    );
}

/// A light frame of `filter`, `exposure` s, gain 100, at `temperature` °C.
fn light(night: &Path, name: &str, filter: &str, exposure: &str, temperature: &str) {
    let bytes = fits_bytes(
        &[
            ("IMAGETYP", "'LIGHT'"),
            ("OBJECT", "'M31'"),
            ("FILTER", &format!("'{filter}'")),
            ("EXPTIME", exposure),
            ("GAIN", "100"),
            ("XBINNING", "1"),
            ("YBINNING", "1"),
            ("CCD-TEMP", temperature),
        ],
        0,
    );
    fs::write(night.join("M31/lights").join(name), bytes).unwrap();
}

#[test]
fn one_stacking_per_family_and_one_family_only_without_variants() {
    let dir = TempDir::new("stacking-families");
    let night = night(dir.path());
    fs::remove_file(night.join("M31/lights/Light_0001.fits")).unwrap();
    // Ha at -10 °C and at -9 °C: one family (a degree splits no stacking).
    light(&night, "Ha_001.fits", "Ha", "300.0", "-10.0");
    light(&night, "Ha_002.fits", "Ha", "300.0", "-10.0");
    light(&night, "Ha_003.fits", "Ha", "300.0", "-9.0");
    light(&night, "OIII_001.fits", "OIII", "300.0", "-10.0");
    light(&night, "OIII_002.fits", "OIII", "300.0", "-10.0");
    let choices = [PipelineChoice::new("raw")];

    let families = stack_families(&night, "M31");
    let labels: Vec<&str> = families.iter().map(|f| f.label.as_str()).collect();
    assert_eq!(labels, vec!["Ha_300s_g100", "OIII_300s_g100"]);
    assert_eq!(families[0].frames.len(), 3);

    let plan = plan_target(&night, "M31", "2026-10-07", &choices, 12);
    assert!(plan.set_aside.is_empty());
    let names: Vec<&str> = plan.jobs.iter().map(|j| j.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "M31_2026-10-07_Ha_300s_g100",
            "M31_2026-10-07_OIII_300s_g100"
        ]
    );
    assert_eq!(plan.jobs[0].frames.len(), 3);
    assert_eq!(plan.jobs[0].light_count(), 3);
    assert_eq!(plan.jobs[1].family.as_deref(), Some("OIII_300s_g100"));
    let args = stack_args(&plan.jobs[0], &[], None, None, None).join(" ");
    assert!(
        args.contains(&format!(
            "--frames-from {}",
            stacking_scratch_root(&night)
                .join("M31_2026-10-07_Ha_300s_g100.lights.txt")
                .display()
        )),
        "{args}"
    );

    // Each family stacked, its renders in stacked/ only, the largest first.
    let stacker = FakeStacker::new(Behaviour::Writes);
    let mut events = Vec::new();
    let outcome = run_target_stacking(
        &stacker,
        &plan,
        &night,
        OPEN,
        &|_| None,
        &CancelFlag::new(),
        None,
        &mut |e| events.push(e.clone()),
    )
    .unwrap();
    assert_eq!(stacker.calls.get(), 2);
    let files: Vec<String> = outcome
        .renders
        .iter()
        .map(|r| r.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        files,
        vec![
            "M31_2026-10-07_Ha_300s_g100_raw.tif",
            "M31_2026-10-07_OIII_300s_g100_raw.tif"
        ]
    );
    assert_eq!(outcome.renders[1].family.as_deref(), Some("OIII_300s_g100"));
    assert!(events.contains(&StackEvent::Family {
        family: Some("Ha_300s_g100".to_string()),
        index: 1,
        total: 2
    }));
    assert!(
        fs::read_dir(night.join("M31/final"))
            .unwrap()
            .next()
            .is_none()
    );

    // A space without variants: the largest family only, the other said.
    let plan = plan_target(&night, "M31", "2026-10-07", &choices, 0);
    assert_eq!(plan.jobs.len(), 1);
    assert_eq!(plan.jobs[0].family.as_deref(), Some("Ha_300s_g100"));
    assert_eq!(
        plan.set_aside,
        vec![SetAsideFamily {
            label: "OIII_300s_g100".to_string(),
            frames: 2
        }]
    );

    // One family: the whole lights/ folder, named as before.
    for name in ["OIII_001.fits", "OIII_002.fits"] {
        fs::remove_file(night.join("M31/lights").join(name)).unwrap();
    }
    let plan = plan_target(&night, "M31", "2026-10-08", &choices, 12);
    assert_eq!(plan.jobs.len(), 1);
    assert!(plan.jobs[0].frames.is_empty() && plan.jobs[0].family.is_none());
    assert_eq!(plan.jobs[0].name, "M31_2026-10-08");
}

#[test]
fn the_stacked_image_of_a_target_online_is_a_variant_never_the_main_image() {
    use crate::night_online::NightOnline;
    use crate::precheck::{Decision, plan_images};
    let dir = TempDir::new("stacking-variant");
    let night = night(dir.path());
    let job = plan_stack(
        &night,
        "M31",
        "2026-10-07",
        &[
            PipelineChoice::new("raw"),
            PipelineChoice::new("dualband-correction"),
        ],
    );
    let outcome = run_stacking(
        &FakeStacker::new(Behaviour::Writes),
        &job,
        OPEN,
        &CancelFlag::new(),
        None,
        &mut |_| {},
    )
    .unwrap();
    let online: NightOnline = serde_json::from_str(include_str!(
        "../../tests/fixtures/online/night_in_progress.json"
    ))
    .unwrap();
    let plans = plan_images(&online, &stacked_candidates(&outcome, "m31"), 12);
    assert_eq!(plans.len(), 2);
    for plan in &plans {
        assert!(
            matches!(plan.decision, Decision::Variant { .. }),
            "{:?}",
            plan.decision
        );
    }
}

#[test]
fn the_room_is_checked_before_anything_runs() {
    let dir = TempDir::new("stacking-room");
    let night = night(dir.path());
    let plan = plan_target(&night, "M31", "2026-10-07", &[], 12);
    let stacker = FakeStacker::new(Behaviour::Writes);
    let refused = run_target_stacking(
        &stacker,
        &plan,
        &night,
        OPEN,
        &|_| Some(100 * 1000 * 1000),
        &CancelFlag::new(),
        None,
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(
        refused,
        StackFailure::NotEnoughSpace {
            needed: BYTES_PER_LIGHT,
            available: 100 * 1000 * 1000,
            folder: night.display().to_string(),
        }
    );
    assert_eq!(stacker.calls.get(), 0);
    // The room is measured on the disk of the night, not in Astro Sync's
    // data; nothing is created by a refusal.
    let measured = std::cell::RefCell::new(Vec::new());
    let _ = run_target_stacking(
        &stacker,
        &plan,
        &night,
        OPEN,
        &|path| {
            measured.borrow_mut().push(path.to_path_buf());
            Some(0)
        },
        &CancelFlag::new(),
        None,
        &mut |_| {},
    );
    assert_eq!(*measured.borrow(), vec![night.clone()]);
    assert!(!night.join(WORK_DIR).exists());
    // The tool is read first, then the room.
    assert_eq!(
        run_target_stacking(
            &stacker,
            &plan,
            &night,
            StackGate {
                tool_present: false,
                ..OPEN
            },
            &|_| Some(0),
            &CancelFlag::new(),
            None,
            &mut |_| {},
        )
        .unwrap_err(),
        StackFailure::ToolMissing
    );
    run_target_stacking(
        &stacker,
        &plan,
        &night,
        OPEN,
        &|_| Some(10 * BYTES_PER_LIGHT),
        &CancelFlag::new(),
        None,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(stacker.calls.get(), 1);
    if cfg!(unix) {
        assert!(free_bytes(dir.path()).is_some_and(|b| b > 0));
    }
}

#[cfg(unix)]
#[test]
fn the_script_is_stopped_gently_and_its_failures_keep_their_kind() {
    use crate::external_process::fake_tool;
    let dir = TempDir::new("stacking-stop");
    let night = night(dir.path());
    light(&night, "Ha_002.fits", "Ha", "300.0", "-10.0");
    light(&night, "OIII_001.fits", "OIII", "300.0", "-10.0");
    let plan = plan_target(&night, "M31", "2026-10-07", &[], 12);
    let job = &plan.jobs[0];
    assert!(!job.frames.is_empty());
    let seen = dir.path().join("seen.txt");
    let stopping = fake_tool(
        dir.path(),
        "python-stopping",
        &format!(
            "echo \"$@\" > '{seen}'\ncat \"$(echo \"$@\" | sed 's/.*--frames-from \\(.*\\.lights\\.txt\\).*/\\1/')\" >> '{seen}'\ntrap 'echo \"{{\\\"event\\\": \\\"error\\\", \\\"code\\\": \\\"cancelled\\\", \\\"message\\\": \\\"stopped\\\"}}\"; exit 130' TERM\necho '{{\"event\": \"step\", \"step\": \"calibration\", \"index\": 1, \"total\": 5}}'\nwhile true; do sleep 0.05; done",
            seen = seen.display()
        ),
    );
    let stacker = PythonStacker {
        python: stopping,
        script: dir.path().join("astro_sync_stack.py"),
        pipelines_dirs: Vec::new(),
        siril: None,
        starnet: None,
        graxpert: None,
        timeout: Duration::from_secs(60),
    };
    // The person stops it once its first step is shown.
    let cancel = CancelFlag::new();
    let stopper = cancel.clone();
    let mut events = Vec::new();
    assert_eq!(
        run_stacking(&stacker, job, OPEN, &cancel, None, &mut |e| {
            events.push(e.clone());
            stopper.cancel();
        })
        .unwrap_err(),
        StackFailure::Cancelled
    );
    assert_eq!(events.len(), 1, "the step read before the stop");
    let seen = fs::read_to_string(&seen).unwrap();
    assert!(seen.starts_with("-B -E -s "), "{seen}");
    assert!(
        seen.contains("Ha_002.fits") && !seen.contains("OIII_001.fits"),
        "{seen}"
    );
    assert!(
        !job.frames_list().exists(),
        "the list is removed after the run"
    );

    // A failure without its own code: the kind read from the exit code.
    for (exit, code) in [(2, "bad_input"), (3, "tool_missing"), (4, "tool_failed")] {
        let failing = fake_tool(
            dir.path(),
            &format!("python-exit-{exit}"),
            &format!("echo 'Siril went away' >&2\nexit {exit}"),
        );
        let stacker = PythonStacker {
            python: failing,
            ..stacker.clone()
        };
        assert_eq!(
            run_stacking(&stacker, job, OPEN, &CancelFlag::new(), None, &mut |_| {}).unwrap_err(),
            StackFailure::Failed {
                code: code.to_string(),
                message: "Siril went away".to_string(),
                logs: None,
            }
        );
    }
}

#[test]
fn the_scratch_lives_in_the_night_and_is_removed_after_a_success_a_failure_and_a_stop() {
    let dir = TempDir::new("stacking-scratch");
    let night = night(dir.path());
    let plan = plan_target(&night, "M31", "2026-10-07", &[], 12);
    assert_eq!(
        plan.jobs[0].work_dir,
        night.join(".astro-sync-work/stacking/M31_2026-10-07")
    );
    assert_eq!(
        stacking_scratch_root(&night),
        night.join(".astro-sync-work/stacking")
    );

    for behaviour in [
        Behaviour::Writes,
        Behaviour::Fails,
        Behaviour::WaitsForTheStop,
    ] {
        let ok = matches!(behaviour, Behaviour::Writes);
        let stacker = FakeStacker::new(behaviour);
        let result = run_target_stacking(
            &stacker,
            &plan,
            &night,
            OPEN,
            &|_| None,
            &CancelFlag::new(),
            None,
            &mut |_| {},
        );
        assert_eq!(result.is_ok(), ok);
        assert_eq!(stacker.calls.get(), 1, "the stacker did run");
        assert!(
            !night.join(WORK_DIR).exists(),
            "the scratch and its folder are gone ({result:?})"
        );
    }
    // The renders stay where they belong.
    assert!(night.join("M31/stacked/M31_2026-10-07_raw.tif").exists());

    // Only the job's own folder goes: anything else in .astro-sync-work/
    // stays, and so does the folder around it.
    let other = night.join(".astro-sync-work/other");
    fs::create_dir_all(&other).unwrap();
    let stacker = FakeStacker::new(Behaviour::Fails);
    let _ = run_target_stacking(
        &stacker,
        &plan,
        &night,
        OPEN,
        &|_| None,
        &CancelFlag::new(),
        None,
        &mut |_| {},
    );
    assert!(other.exists());
    assert!(!night.join(".astro-sync-work/stacking").exists());
}

/// The journals folders of `logs_dir`, sorted.
fn log_folders(logs_dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(logs_dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn a_failure_or_a_stop_keeps_the_journals_and_a_success_keeps_nothing() {
    let dir = TempDir::new("stacking-logs");
    let night = night(dir.path());
    let logs = dir.path().join("data/stacking-logs");
    let plan = plan_target(&night, "M31", "2026-10-07", &[], 12);
    let job = &plan.jobs[0];

    // A success: nothing is copied (the folder is not even made).
    let stacker = FakeStacker::new(Behaviour::Writes);
    run_target_stacking(
        &stacker,
        &plan,
        &night,
        OPEN,
        &|_| None,
        &CancelFlag::new(),
        Some(&logs),
        &mut |_| {},
    )
    .unwrap();
    assert!(!logs.exists(), "a success keeps no journal");
    assert!(!night.join(WORK_DIR).exists());

    // A failure: the journals (and no image) are copied, the failure names
    // the folder, the scratch is gone.
    let scratch = job.work_dir.clone();
    let failing = FakeStacker::new(Behaviour::Fails);
    // The fake writes siril.log; this one adds what the script adds, and
    // two images.
    struct Adds<'a>(&'a FakeStacker);
    impl Stacker for Adds<'_> {
        fn stack(
            &self,
            job: &StackJob,
            cancel: &CancelFlag,
            on_event: &mut dyn FnMut(&StackEvent),
        ) -> Result<Vec<StackerOutput>, StackFailure> {
            let result = self.0.stack(job, cancel, on_event);
            fs::write(job.work_dir.join("render.log"), b"render").unwrap();
            fs::write(job.work_dir.join("process/notes.txt"), b"notes").unwrap();
            fs::write(job.work_dir.join("process/pp_light.fit"), b"image").unwrap();
            fs::write(job.work_dir.join("result.tif"), b"image").unwrap();
            result
        }
    }
    let failure = run_target_stacking(
        &Adds(&failing),
        &plan,
        &night,
        OPEN,
        &|_| None,
        &CancelFlag::new(),
        Some(&logs),
        &mut |_| {},
    )
    .unwrap_err();
    let names = log_folders(&logs);
    assert_eq!(names.len(), 1, "{names:?}");
    assert!(names[0].starts_with("M31_2026-10-07-"), "{names:?}");
    let kept = logs.join(&names[0]);
    assert_eq!(fs::read(kept.join("siril.log")).unwrap(), b"log");
    assert_eq!(fs::read(kept.join("render.log")).unwrap(), b"render");
    assert_eq!(fs::read(kept.join("process/notes.txt")).unwrap(), b"notes");
    assert!(!kept.join("process/pp_light.fit").exists());
    assert!(!kept.join("result.tif").exists());
    match failure {
        StackFailure::Failed {
            code,
            logs: Some(folder),
            ..
        } => {
            assert_eq!(code, "siril_failed");
            assert_eq!(PathBuf::from(folder), kept);
        }
        other => panic!("{other:?}"),
    }
    assert!(!scratch.exists(), "the scratch is removed after the copy");
    assert!(!night.join(WORK_DIR).exists());

    // A stop copies them too (the stop itself has no folder to name).
    let logs_stop = dir.path().join("data/stacking-logs-stop");
    let stopped = run_target_stacking(
        &FakeStacker::new(Behaviour::WaitsForTheStop),
        &plan,
        &night,
        OPEN,
        &|_| None,
        &CancelFlag::new(),
        Some(&logs_stop),
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(stopped, StackFailure::Cancelled);
    assert_eq!(log_folders(&logs_stop).len(), 1);
    assert!(!night.join(WORK_DIR).exists());

    // No folder of journals given: nothing is kept, the failure names none.
    let none = run_target_stacking(
        &FakeStacker::new(Behaviour::Fails),
        &plan,
        &night,
        OPEN,
        &|_| None,
        &CancelFlag::new(),
        None,
        &mut |_| {},
    )
    .unwrap_err();
    assert!(matches!(none, StackFailure::Failed { logs: None, .. }));
}

#[test]
fn only_the_ten_most_recent_journal_folders_are_kept() {
    let dir = TempDir::new("stacking-logs-rotation");
    let night = night(dir.path());
    let logs = dir.path().join("data/stacking-logs");
    // Twelve old folders (three of them with a `-2`, a name that was taken),
    // and one that is not ours.
    for day in 1..=12 {
        let name = if day % 4 == 0 {
            format!("M31_2026-09-{day:02}-2026{:02}{day:02}-210000-2", 9)
        } else {
            format!("M31_2026-09-{day:02}-2026{:02}{day:02}-210000", 9)
        };
        fs::create_dir_all(logs.join(name)).unwrap();
    }
    fs::create_dir_all(logs.join("my-own-notes")).unwrap();
    let plan = plan_target(&night, "M31", "2026-10-07", &[], 12);
    let _ = run_target_stacking(
        &FakeStacker::new(Behaviour::Fails),
        &plan,
        &night,
        OPEN,
        &|_| None,
        &CancelFlag::new(),
        Some(&logs),
        &mut |_| {},
    )
    .unwrap_err();
    let names = log_folders(&logs);
    // 10 journal folders + the one that is not ours.
    assert_eq!(names.len(), 11, "{names:?}");
    assert!(names.contains(&"my-own-notes".to_string()));
    // The new one is there; the three oldest (days 1 to 3) are gone, the fourth (a `-2`) stays.
    assert!(names.iter().any(|n| n.starts_with("M31_2026-10-07-")));
    for day in 1..=3 {
        assert!(
            !names
                .iter()
                .any(|n| n.starts_with(&format!("M31_2026-09-{day:02}-"))),
            "{names:?}"
        );
    }
    assert!(names.iter().any(|n| n.starts_with("M31_2026-09-04-")));
}

#[test]
fn a_scratch_left_by_a_hard_stop_is_cleared_before_the_next_stacking() {
    let dir = TempDir::new("stacking-orphan");
    let night = night(dir.path());
    let plan = plan_target(&night, "M31", "2026-10-07", &[], 12);
    // What a killed app leaves: a scratch tree and a list of frames, in
    // `stacking/`; and something else in `.astro-sync-work/`.
    let orphan = night.join(".astro-sync-work/stacking/M31_2026-10-06");
    fs::create_dir_all(orphan.join("process")).unwrap();
    fs::write(orphan.join("process/big.fit"), b"x").unwrap();
    fs::write(
        night.join(".astro-sync-work/stacking/M31_2026-10-06.lights.txt"),
        b"x",
    )
    .unwrap();
    let other = night.join(".astro-sync-work/other");
    fs::create_dir_all(&other).unwrap();

    // The room is measured after the clearing: it is cleared even when the
    // stacking is then refused for lack of room.
    let refused = run_target_stacking(
        &FakeStacker::new(Behaviour::Writes),
        &plan,
        &night,
        OPEN,
        &|_| Some(0),
        &CancelFlag::new(),
        None,
        &mut |_| {},
    )
    .unwrap_err();
    assert!(matches!(refused, StackFailure::NotEnoughSpace { .. }));
    assert!(!orphan.exists());
    assert!(!night.join(".astro-sync-work/stacking").exists());
    assert!(
        other.exists(),
        "nothing else of the hidden folder is touched"
    );

    // And before a stacking that runs: the job's own scratch is fresh.
    let stale = night.join(".astro-sync-work/stacking/M31_2026-10-07");
    fs::create_dir_all(&stale).unwrap();
    fs::write(stale.join("old.log"), b"old").unwrap();
    let stacker = FakeStacker::new(Behaviour::Writes);
    run_target_stacking(
        &stacker,
        &plan,
        &night,
        OPEN,
        &|_| None,
        &CancelFlag::new(),
        None,
        &mut |_| {},
    )
    .unwrap();
    assert!(!stale.exists());
    assert!(other.exists());
}

/// A light frame of `filter` taken at `date_obs` (UTC).
fn dated_light(night: &Path, name: &str, filter: &str, date_obs: &str) {
    let bytes = fits_bytes(
        &[
            ("IMAGETYP", "'LIGHT'"),
            ("OBJECT", "'M31'"),
            ("FILTER", &format!("'{filter}'")),
            ("EXPTIME", "300.0"),
            ("GAIN", "100"),
            ("XBINNING", "1"),
            ("CCD-TEMP", "-10.0"),
            ("DATE-OBS", &format!("'{date_obs}'")),
        ],
        0,
    );
    fs::write(night.join("M31/lights").join(name), bytes).unwrap();
}

#[test]
fn the_master_of_each_family_is_kept_and_described() {
    let dir = TempDir::new("stacking-masters");
    let night = night(dir.path());
    fs::remove_file(night.join("M31/lights/Light_0001.fits")).unwrap();
    light(&night, "Ha_001.fits", "Ha", "300.0", "-10.0");
    light(&night, "Ha_002.fits", "Ha", "300.0", "-10.0");
    light(&night, "Ha_003.fits", "Ha", "300.0", "-9.0");
    light(&night, "OIII_001.fits", "OIII", "300.0", "-10.0");
    light(&night, "OIII_002.fits", "OIII", "300.0", "-10.0");
    let plan = plan_target(&night, "M31", "2026-10-07", &[], 12);
    assert_eq!(
        plan.jobs
            .iter()
            .map(|j| j.integration.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("Ha_300s_g100"), Some("OIII_300s_g100")]
    );
    let args = stack_args(&plan.jobs[0], &[], None, None, None).join(" ");
    assert!(args.contains("--integration Ha_300s_g100"), "{args}");
    let outcome = run_target_stacking(
        &FakeStacker::new(Behaviour::Writes),
        &plan,
        &night,
        OPEN,
        &|_| None,
        &CancelFlag::new(),
        None,
        &mut |_| {},
    )
    .unwrap();
    let kept: Vec<String> = outcome
        .integrations
        .iter()
        .map(|m| m.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        kept,
        vec![
            "Ha_300s_g100_integration.fit",
            "OIII_300s_g100_integration.fit"
        ]
    );
    let ha = &outcome.integrations[0];
    assert_eq!(ha.family, "Ha_300s_g100");
    assert_eq!(ha.frame_count, 3);
    assert_eq!(ha.exposure_seconds, Some(300.0));
    assert!((ha.total_seconds - 900.0).abs() < 1e-9);
    assert_eq!(ha.sensor.as_deref(), Some("ZWO ASI2600MC Pro"));
    assert_eq!(ha.bytes, fs::metadata(&ha.path).unwrap().len());
    assert!(ha.colour);
    // The master is no image to send: never a candidate.
    let candidates = stacked_candidates(&outcome, "t-m31");
    assert!(
        candidates
            .iter()
            .all(|c| !c.path.to_string_lossy().ends_with(INTEGRATION_SUFFIX))
    );
    // The scratch is gone, the master stays.
    assert!(!night.join(WORK_DIR).exists());
    assert!(ha.path.exists());
}

#[test]
fn the_frames_outside_the_night_are_left_out_of_the_integration() {
    let dir = TempDir::new("stacking-window");
    let night = night(dir.path());
    fs::remove_file(night.join("M31/lights/Light_0001.fits")).unwrap();
    // The night of the 10th August at 10° E (local noon 11:20 UTC): three
    // frames after midnight UTC, one of the evening before.
    dated_light(&night, "a.fits", "Ha", "2026-08-11T00:00:00");
    dated_light(&night, "b.fits", "Ha", "2026-08-11T01:30:00");
    dated_light(&night, "c.fits", "Ha", "2026-08-11T03:01:00");
    dated_light(&night, "old.fits", "Ha", "2026-08-09T22:00:00");
    let window = NightWindow::of_evening("2026-08-10", Some(10.0)).unwrap();
    let plan = plan_target_within(&night, "M31", "2026-08-10", &[], 12, Some(&window));
    assert_eq!(plan.jobs.len(), 1);
    assert_eq!(plan.outside, vec![night.join("M31/lights/old.fits")]);
    let job = &plan.jobs[0];
    assert_eq!(job.integration.as_deref(), Some("Ha_300s_g100"));
    assert_eq!(job.light_count(), 3);
    assert!(!job.frames.iter().any(|f| f.ends_with("old.fits")));
    // The job lists its frames: the script never reads the whole folder.
    let args = stack_args(job, &[], None, None, None).join(" ");
    assert!(args.contains("--frames-from"), "{args}");
    let outcome = run_target_stacking(
        &FakeStacker::new(Behaviour::Writes),
        &plan,
        &night,
        OPEN,
        &|_| None,
        &CancelFlag::new(),
        None,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(outcome.outside_frames, 1);
    assert_eq!(outcome.integrations[0].frame_count, 3);
    // Without the window: every frame, the whole folder as before.
    let all = plan_target(&night, "M31", "2026-08-10", &[], 12);
    assert!(all.outside.is_empty() && all.jobs[0].frames.is_empty());
    assert_eq!(all.jobs[0].light_count(), 4);
}

/// R348b: the engine of the "polished render" is the task's to choose; the
/// default (GraXpert's AI models) passes nothing, Siril's own is passed to
/// the script, and "No processing" never gets it.
#[test]
fn the_polished_engine_is_passed_to_the_script_only_when_it_is_not_the_default() {
    let dir = TempDir::new("stacking-polished-engine");
    let night = night(dir.path());
    let mut job = plan_stack(
        &night,
        "M31",
        "2026-10-07",
        &[PipelineChoice::new("polished")],
    );
    assert_eq!(job.polished_engine, PolishedEngine::GraxpertAi);
    assert!(
        !stack_args(&job, &[], None, None, None)
            .join(" ")
            .contains("--polished-engine")
    );
    job.polished_engine = PolishedEngine::Siril;
    let joined = stack_args(&job, &[], None, None, None).join(" ");
    assert!(joined.contains("--polished-engine siril"), "{joined}");
    // The job file keeps the default out and reads it back.
    let text = serde_json::to_string(&job).unwrap();
    assert!(text.contains(r#""polishedEngine":"siril""#), "{text}");
    let back: StackJob = serde_json::from_str(&text).unwrap();
    assert_eq!(back.polished_engine, PolishedEngine::Siril);
    job.polished_engine = PolishedEngine::default();
    assert!(
        !serde_json::to_string(&job)
            .unwrap()
            .contains("polishedEngine")
    );
    // "No processing": no render, no option.
    let mut linear = plan_stack(&night, "M31", "2026-10-07", &[PipelineChoice::linear()]);
    linear.polished_engine = PolishedEngine::Siril;
    assert!(
        !stack_args(&linear, &[], None, None, None)
            .join(" ")
            .contains("--polished-engine")
    );
    assert_eq!(PolishedEngine::Siril.as_str(), "siril");
    assert_eq!(PolishedEngine::GraxpertAi.as_str(), "graxpert-ai");
}

#[cfg(unix)]
#[test]
fn the_registration_the_script_reports_comes_out_as_an_event() {
    // Finding TD1: the script measures the field rotation of the night and says the
    // framing it chose and the lights it left out; the window words them.
    use crate::external_process::fake_tool;
    let dir = TempDir::new("stacking-registration");
    let night = night(dir.path());
    let job = plan_stack(&night, "M31", "2026-10-07", &[]);
    fs::create_dir_all(&job.output_dir).unwrap();
    let out = job.output_dir.join("M31_2026-10-07_raw.tif");
    let python = fake_tool(
        dir.path(),
        "python3",
        &format!(
            "echo '{{\"event\": \"step\", \"step\": \"registration\", \"index\": 3, \"total\": 5}}'\necho '{{\"event\": \"registration\", \"unit\": \"lights\", \"frames\": 430, \"registered\": 429, \"unregistered\": [\"Light_M 45_30.0s_IRCUT_20261011-045331.fit\"], \"fieldRotationDegrees\": 125.29, \"fieldRotation\": \"progressive\", \"framing\": \"full\", \"frame\": {{\"frameHeight\": 3840, \"frameWidth\": 2160, \"notches\": [{{\"rank\": 0, \"top\": 1200, \"left\": 400, \"height\": 1536, \"width\": 1520, \"shareOfFrame\": 0.28, \"fullShare\": 1.0, \"lowestFrames\": 429}}, {{\"rank\": 4, \"top\": 0, \"left\": 0, \"height\": 3840, \"width\": 2160, \"shareOfFrame\": 1.0, \"fullShare\": 0.705, \"lowestFrames\": 4}}], \"automatic\": 4, \"requested\": 0, \"rank\": 0}}}}'\necho '{{\"event\": \"registration\", \"frames\": 90, \"registered\": 90, \"unregistered\": [], \"fieldRotationDegrees\": null, \"fieldRotation\": null, \"framing\": \"common\"}}'\necho '{{\"event\": \"registration\", \"unit\": \"masters\", \"frames\": 3, \"registered\": 2, \"unregistered\": [\"2026-10-12 · n-2\"], \"fieldRotationDegrees\": 130.0, \"fieldRotation\": \"progressive\", \"framing\": \"full\"}}'\necho '{{\"event\": \"coverage\", \"noDualBandShare\": 0.1875}}'\necho '{{\"event\": \"crop\", \"mode\": \"common\", \"applied\": true, \"reason\": \"commonArea\", \"top\": 444, \"left\": 419, \"height\": 2095, \"width\": 2137, \"frameHeight\": 2977, \"frameWidth\": 2995, \"lost\": {{\"top\": 444, \"left\": 419, \"bottom\": 438, \"right\": 439}}, \"keptShare\": 0.5021}}'\nprintf 'tif' > '{}'\necho '{{\"event\": \"output\", \"pipeline\": \"raw\", \"file\": \"M31_2026-10-07_raw.tif\", \"bytes\": 3, \"sha256\": \"x\"}}'",
            out.display()
        ),
    );
    let stacker = PythonStacker {
        python,
        script: dir.path().join("astro_sync_stack.py"),
        pipelines_dirs: Vec::new(),
        siril: None,
        starnet: None,
        graxpert: None,
        timeout: Duration::from_secs(30),
    };
    let mut events = Vec::new();
    run_stacking(&stacker, &job, OPEN, &CancelFlag::new(), None, &mut |e| {
        events.push(e.clone());
    })
    .unwrap();
    let registrations: Vec<&StackEvent> = events
        .iter()
        .filter(|e| matches!(e, StackEvent::Registration { .. }))
        .collect();
    assert_eq!(
        registrations,
        [
            &StackEvent::Registration {
                unit: RegisteredFrames::Lights,
                frames: 430,
                registered: 429,
                unregistered: vec!["Light_M 45_30.0s_IRCUT_20261011-045331.fit".to_string()],
                field_rotation_degrees: Some(125.29),
                framing: Framing::Full,
                frame: Some(FrameMeasure {
                    frame_height: Some(3840),
                    frame_width: Some(2160),
                    notches: vec![
                        FrameNotch {
                            rank: 0,
                            top: 1200,
                            left: 400,
                            height: 1536,
                            width: 1520,
                            share_of_frame: 0.28,
                            full_share: 1.0,
                            lowest_frames: 429,
                        },
                        FrameNotch {
                            rank: 4,
                            top: 0,
                            left: 0,
                            height: 3840,
                            width: 2160,
                            share_of_frame: 1.0,
                            full_share: 0.705,
                            lowest_frames: 4,
                        },
                    ],
                    automatic: Some(4),
                    rank: Some(0),
                }),
            },
            &StackEvent::Registration {
                unit: RegisteredFrames::Lights,
                frames: 90,
                registered: 90,
                unregistered: Vec::new(),
                field_rotation_degrees: None,
                framing: Framing::Common,
                frame: None,
            },
            &StackEvent::Registration {
                unit: RegisteredFrames::Masters,
                frames: 3,
                registered: 2,
                unregistered: vec!["2026-10-12 · n-2".to_string()],
                field_rotation_degrees: Some(130.0),
                framing: Framing::Full,
                frame: None,
            },
        ]
    );
    // What the combination said of the frame: the corners the dual-band nights miss.
    let coverage: Vec<&StackEvent> = events
        .iter()
        .filter(|e| matches!(e, StackEvent::Coverage { .. }))
        .collect();
    assert_eq!(
        coverage,
        [&StackEvent::Coverage {
            no_dual_band_share: 0.1875
        }]
    );
    assert_eq!(
        serde_json::to_value(coverage[0]).unwrap(),
        serde_json::json!({"event": "coverage", "noDualBandShare": 0.1875})
    );
    // What the image of a plan was cut to: the area every night covers.
    let crop = events
        .iter()
        .find(|e| matches!(e, StackEvent::Crop { .. }))
        .unwrap();
    assert_eq!(
        crop,
        &StackEvent::Crop {
            mode: "common".into(),
            applied: true,
            reason: "commonArea".into(),
            top: 444,
            left: 419,
            height: 2095,
            width: 2137,
            frame_height: 2977,
            frame_width: 2995,
            kept_share: 0.5021,
        }
    );
    assert_eq!(
        serde_json::to_value(crop).unwrap()["keptShare"],
        serde_json::json!(0.5021)
    );
}

#[test]
fn the_registration_event_goes_to_the_window_in_camel_case() {
    let event = StackEvent::Registration {
        unit: RegisteredFrames::Lights,
        frames: 430,
        registered: 429,
        unregistered: vec!["a.fit".to_string()],
        field_rotation_degrees: Some(125.29),
        framing: Framing::Full,
        frame: None,
    };
    assert_eq!(
        serde_json::to_value(&event).unwrap(),
        serde_json::json!({
            "event": "registration",
            "unit": "lights",
            "frames": 430,
            "registered": 429,
            "unregistered": ["a.fit"],
            "fieldRotationDegrees": 125.29,
            "framing": "full"
        })
    );
    assert!(event.progress_text().is_none());
    // The other events keep their words.
    assert_eq!(
        serde_json::to_value(&StackEvent::Step {
            step: "registration".to_string(),
            index: 3,
            total: 5
        })
        .unwrap(),
        serde_json::json!({"event": "step", "step": "registration", "index": 3, "total": 5})
    );
}

/* ----------------------- The image of a device ----------------------- */

/// A Seestar-like stacked image of `filter` at `exposure` s, `poses` poses, in
/// `M31/device/`.
fn device_image(night: &Path, name: &str, filter: &str, exposure: &str, poses: u32) {
    let bytes = fits_bytes(
        &[
            ("SIMPLE", "T"),
            ("BITPIX", "16"),
            ("NAXIS", "3"),
            ("NAXIS1", "2160"),
            ("NAXIS2", "3840"),
            ("NAXIS3", "3"),
            ("BZERO", "32768"),
            ("CREATOR", "'ZWO Seestar S50 Pro'"),
            ("STACKCNT", &poses.to_string()),
            ("FILTER", &format!("'{filter}'")),
            ("EXPTIME", exposure),
            ("GAIN", "100"),
            ("XBINNING", "1"),
            ("YBINNING", "1"),
        ],
        0,
    );
    let folder = night.join("M31/device");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join(name), bytes).unwrap();
}

fn uses(files: &[&str]) -> Vec<DeviceUse> {
    files
        .iter()
        .map(|file| DeviceUse {
            file: (*file).to_string(),
        })
        .collect()
}

#[test]
fn without_a_device_choice_the_plan_is_what_it_always_was() {
    let dir = TempDir::new("stacking-device-none");
    let night = night(dir.path());
    device_image(&night, "Stacked_3.fit", "Ha", "300.0", 3);
    let before = plan_target_within(&night, "M31", "2026-10-07", &[], 12, None);
    let after = plan_target_with_device(&night, "M31", "2026-10-07", &[], 12, None, &[]);
    assert_eq!(before, after);
    assert!(before.jobs.iter().all(|job| job.device_master.is_none()));
}

#[test]
fn a_family_that_takes_the_device_image_renders_it_and_stacks_nothing() {
    let dir = TempDir::new("stacking-device-one");
    let night = night(dir.path());
    fs::remove_file(night.join("M31/lights/Light_0001.fits")).unwrap();
    for name in ["Ha_001.fits", "Ha_002.fits", "Ha_003.fits"] {
        light(&night, name, "Ha", "300.0", "-10.0");
    }
    // The family's label is `Ha_300s_g100`: the image says the same.
    device_image(&night, "Stacked_3.fit", "Ha", "300.0", 3);
    let plan = plan_target_with_device(
        &night,
        "M31",
        "2026-10-07",
        &[PipelineChoice::new("raw")],
        12,
        None,
        &uses(&["Stacked_3.fit"]),
    );
    assert_eq!(plan.jobs.len(), 1);
    let job = &plan.jobs[0];
    assert_eq!(
        job.device_master,
        Some(night.join("M31/device/Stacked_3.fit"))
    );
    assert_eq!(job.family.as_deref(), Some("Ha_300s_g100"));
    // The poses are not stacked again, and no master is kept for a plan.
    assert!(job.frames.is_empty());
    assert_eq!(job.integration, None);
    assert_eq!(job.light_count(), 0);
    assert_eq!(job.name, "M31_2026-10-07");
    assert!(plan.set_aside.is_empty());

    // The script is given the image and none of what a stacking of poses has.
    let args = stack_args(job, &[], None, None, None);
    let at = args.iter().position(|a| a == "--from-master").unwrap();
    assert_eq!(
        args[at + 1],
        night.join("M31/device/Stacked_3.fit").display().to_string()
    );
    for flag in [
        "--lights",
        "--frames-from",
        "--darks",
        "--flats",
        "--integration",
    ] {
        assert!(!args.iter().any(|a| a == flag), "{flag} in {args:?}");
    }
    assert!(args.windows(2).any(|w| w == ["--pipeline", "raw"]));

    // It runs although `lights/` holds nothing the script reads: the stacker
    // is given the job, the renders come back as stacked results.
    let stacker = FakeStacker::new(Behaviour::Writes);
    let outcome = run_target_stacking(
        &stacker,
        &plan,
        &night,
        OPEN,
        &|_| Some(0),
        &CancelFlag::new(),
        None,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(stacker.calls.get(), 1);
    assert_eq!(outcome.renders.len(), 1);
    assert!(outcome.integrations.is_empty());
}

#[test]
fn the_room_a_device_image_needs_is_none() {
    let dir = TempDir::new("stacking-device-room");
    let night = night(dir.path());
    fs::remove_file(night.join("M31/lights/Light_0001.fits")).unwrap();
    for name in ["Ha_001.fits", "Ha_002.fits"] {
        light(&night, name, "Ha", "300.0", "-10.0");
    }
    device_image(&night, "Stacked_2.fit", "Ha", "300.0", 2);
    let plan = plan_target_with_device(
        &night,
        "M31",
        "2026-10-07",
        &[],
        12,
        None,
        &uses(&["Stacked_2.fit"]),
    );
    // No room at all: a stacking of the poses would be refused, the image is not.
    let outcome = run_target_stacking(
        &FakeStacker::new(Behaviour::Writes),
        &plan,
        &night,
        OPEN,
        &|_| Some(1),
        &CancelFlag::new(),
        None,
        &mut |_| {},
    );
    assert!(outcome.is_ok(), "{outcome:?}");
}

#[test]
fn a_target_with_only_the_device_image_is_rendered_from_it() {
    let dir = TempDir::new("stacking-device-only");
    let night = night(dir.path());
    fs::remove_file(night.join("M31/lights/Light_0001.fits")).unwrap();
    device_image(&night, "Stacked_28.fit", "IRCUT", "10.0", 28);
    let plan = plan_target_with_device(
        &night,
        "M31",
        "2026-10-07",
        &[],
        12,
        None,
        &uses(&["Stacked_28.fit"]),
    );
    assert_eq!(plan.jobs.len(), 1);
    assert_eq!(plan.jobs[0].family.as_deref(), Some("IRCUT_10s_g100"));
    let outcome = run_target_stacking(
        &FakeStacker::new(Behaviour::Writes),
        &plan,
        &night,
        OPEN,
        &|_| None,
        &CancelFlag::new(),
        None,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(outcome.renders.len(), 1);
}

#[test]
fn each_family_takes_its_own_source_and_a_space_without_variants_keeps_the_largest() {
    let dir = TempDir::new("stacking-device-families");
    let night = night(dir.path());
    fs::remove_file(night.join("M31/lights/Light_0001.fits")).unwrap();
    for name in ["Ha_001.fits", "Ha_002.fits", "Ha_003.fits"] {
        light(&night, name, "Ha", "300.0", "-10.0");
    }
    for name in ["OIII_001.fits", "OIII_002.fits"] {
        light(&night, name, "OIII", "300.0", "-10.0");
    }
    // The device stacked the OIII poses (and counts 2); Ha has no image.
    device_image(&night, "Stacked_2.fit", "OIII", "300.0", 2);
    let plan = plan_target_with_device(
        &night,
        "M31",
        "2026-10-07",
        &[],
        12,
        None,
        &uses(&["Stacked_2.fit"]),
    );
    let by_family = |label: &str| {
        plan.jobs
            .iter()
            .find(|job| job.family.as_deref() == Some(label))
            .unwrap()
    };
    // Ha is stacked from its poses (with its master kept), OIII from the image.
    assert_eq!(by_family("Ha_300s_g100").device_master, None);
    assert_eq!(
        by_family("Ha_300s_g100").integration.as_deref(),
        Some("Ha_300s_g100")
    );
    assert_eq!(by_family("Ha_300s_g100").frames.len(), 3);
    assert!(by_family("OIII_300s_g100").device_master.is_some());
    assert_eq!(by_family("OIII_300s_g100").integration, None);
    let names: Vec<&str> = plan.jobs.iter().map(|j| j.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "M31_2026-10-07_Ha_300s_g100",
            "M31_2026-10-07_OIII_300s_g100"
        ]
    );

    // A space without variants: the largest family only (an image counts the
    // poses it stacked), the other said.
    let one = plan_target_with_device(
        &night,
        "M31",
        "2026-10-07",
        &[],
        0,
        None,
        &uses(&["Stacked_2.fit"]),
    );
    assert_eq!(one.jobs.len(), 1);
    assert_eq!(one.jobs[0].family.as_deref(), Some("Ha_300s_g100"));
    assert_eq!(
        one.set_aside,
        vec![SetAsideFamily {
            label: "OIII_300s_g100".to_string(),
            frames: 2
        }]
    );
}

#[test]
fn a_device_image_the_poses_do_not_have_is_a_family_of_its_own() {
    let dir = TempDir::new("stacking-device-extra");
    let night = night(dir.path());
    fs::remove_file(night.join("M31/lights/Light_0001.fits")).unwrap();
    for name in ["Ha_001.fits", "Ha_002.fits"] {
        light(&night, name, "Ha", "300.0", "-10.0");
    }
    // 30 poses stacked in another exposure, whose poses are not here.
    device_image(&night, "Stacked_30.fit", "Ha", "60.0", 30);
    let plan = plan_target_with_device(
        &night,
        "M31",
        "2026-10-07",
        &[],
        12,
        None,
        &uses(&["Stacked_30.fit"]),
    );
    let labels: Vec<Option<&str>> = plan.jobs.iter().map(|j| j.family.as_deref()).collect();
    // The biggest first: the image counts 30 poses against 2.
    assert_eq!(labels, vec![Some("Ha_60s_g100"), Some("Ha_300s_g100")]);
    assert!(plan.jobs[0].device_master.is_some());
    assert!(plan.jobs[1].device_master.is_none());
}

#[test]
fn what_is_not_a_device_image_is_left_out_of_the_choice() {
    let dir = TempDir::new("stacking-device-unknown");
    let night = night(dir.path());
    device_image(&night, "Stacked_3.fit", "Ha", "300.0", 3);
    // A path that climbs out of `device/`, a file that is not there, a file
    // that is no device image: none is taken, the plan is the usual one.
    fs::write(
        night.join("M31/device/Stacked_asiair.fit"),
        fits_bytes(&[("SIMPLE", "T"), ("STACKCNT", "40")], 0),
    )
    .unwrap();
    let plan = plan_target_with_device(
        &night,
        "M31",
        "2026-10-07",
        &[],
        12,
        None,
        &uses(&[
            "../lights/Light_0001.fits",
            "nothing.fit",
            "Stacked_asiair.fit",
        ]),
    );
    assert_eq!(
        plan,
        plan_target_within(&night, "M31", "2026-10-07", &[], 12, None)
    );
    assert!(plan.jobs.iter().all(|job| job.device_master.is_none()));
}

#[test]
fn a_device_image_that_vanished_stops_the_job_in_words() {
    let dir = TempDir::new("stacking-device-gone");
    let night = night(dir.path());
    device_image(&night, "Stacked_3.fit", "Ha", "300.0", 3);
    let plan = plan_target_with_device(
        &night,
        "M31",
        "2026-10-07",
        &[],
        12,
        None,
        &uses(&["Stacked_3.fit"]),
    );
    fs::remove_file(night.join("M31/device/Stacked_3.fit")).unwrap();
    let stacker = FakeStacker::new(Behaviour::Writes);
    let failure = run_target_stacking(
        &stacker,
        &plan,
        &night,
        OPEN,
        &|_| None,
        &CancelFlag::new(),
        None,
        &mut |_| {},
    )
    .unwrap_err();
    assert!(
        matches!(&failure, StackFailure::Failed { code, .. } if code == "folder_missing"),
        "{failure:?}"
    );
    assert_eq!(stacker.calls.get(), 0);
}

/* ------------------------ Flats by filter ------------------------ */

/// `count` flats in `calibration/flats/<sub>` (loose when `sub` is empty).
fn flats(night: &Path, sub: &str, count: usize) {
    let folder = night.join("calibration/flats").join(sub);
    fs::create_dir_all(&folder).unwrap();
    for i in 0..count {
        fs::write(folder.join(format!("Flat_{i:03}.fits")), b"flat").unwrap();
    }
}

fn two_filter_night(dir: &Path) -> PathBuf {
    let night = night(dir);
    fs::remove_file(night.join("M31/lights/Light_0001.fits")).unwrap();
    for (name, filter) in [
        ("Ha_001.fits", "Ha"),
        ("Ha_002.fits", "Ha"),
        ("Ha_003.fits", "Ha"),
        ("OIII_001.fits", "OIII"),
        ("OIII_002.fits", "OIII"),
    ] {
        light(&night, name, filter, "300.0", "-10.0");
    }
    night
}

#[test]
fn each_family_stacks_with_the_flats_of_its_own_filter() {
    let dir = TempDir::new("stacking-flats-by-filter");
    let night = two_filter_night(dir.path());
    flats(&night, "Ha", 11);
    flats(&night, "OIII", 10);
    let plan = plan_target(&night, "M31", "2026-10-07", &[], 12);
    assert_eq!(plan.jobs.len(), 2);
    assert_eq!(plan.jobs[0].family.as_deref(), Some("Ha_300s_g100"));
    assert_eq!(plan.jobs[0].flats, Some(night.join("calibration/flats/Ha")));
    assert_eq!(
        plan.jobs[1].flats,
        Some(night.join("calibration/flats/OIII"))
    );
    assert!(plan.jobs.iter().all(|j| j.flats_missing_for.is_none()));
    let args = stack_args(&plan.jobs[1], &[], None, None, None).join(" ");
    assert!(
        args.contains(&format!(
            "--flats {}",
            night.join("calibration/flats/OIII").display()
        )),
        "{args}"
    );
}

#[test]
fn a_family_without_flats_of_its_filter_takes_none_and_says_which_filter() {
    let dir = TempDir::new("stacking-flats-missing");
    let night = two_filter_night(dir.path());
    flats(&night, "Ha", 5);
    let plan = plan_target(&night, "M31", "2026-10-07", &[], 12);
    assert_eq!(plan.jobs[0].flats, Some(night.join("calibration/flats/Ha")));
    assert_eq!(plan.jobs[0].flats_missing_for, None);
    assert_eq!(plan.jobs[1].flats, None, "never another filter's flats");
    assert_eq!(plan.jobs[1].flats_missing_for.as_deref(), Some("OIII"));
    let args = stack_args(&plan.jobs[1], &[], None, None, None).join(" ");
    assert!(!args.contains("--flats"), "{args}");
}

#[test]
fn a_single_family_takes_its_filter_folder_and_a_loose_layout_is_read_as_before() {
    let dir = TempDir::new("stacking-flats-layouts");
    let night = two_filter_night(dir.path());
    for name in ["OIII_001.fits", "OIII_002.fits"] {
        fs::remove_file(night.join("M31/lights").join(name)).unwrap();
    }
    // Flats of two filters: the single family (Ha) takes only its own.
    flats(&night, "Ha", 4);
    flats(&night, "OIII", 4);
    let plan = plan_target(&night, "M31", "2026-10-07", &[], 12);
    assert_eq!(plan.jobs.len(), 1);
    assert_eq!(plan.jobs[0].flats, Some(night.join("calibration/flats/Ha")));
    // The name of a folder is matched without its case.
    fs::rename(
        night.join("calibration/flats/Ha"),
        night.join("calibration/flats/ha-tmp"),
    )
    .unwrap();
    fs::rename(
        night.join("calibration/flats/ha-tmp"),
        night.join("calibration/flats/HA"),
    )
    .unwrap();
    let plan = plan_target(&night, "M31", "2026-10-07", &[], 12);
    assert_eq!(plan.jobs[0].flats, Some(night.join("calibration/flats/HA")));
    // Flats kept loose (no filter folder): the folder itself, as before.
    let loose = TempDir::new("stacking-flats-loose");
    let night = two_filter_night(loose.path());
    flats(&night, "", 6);
    let plan = plan_target(&night, "M31", "2026-10-07", &[], 12);
    assert_eq!(plan.jobs.len(), 2);
    for job in &plan.jobs {
        assert_eq!(job.flats, Some(night.join("calibration/flats")));
        assert_eq!(job.flats_missing_for, None);
    }
}

#[test]
fn a_family_without_filter_takes_the_only_filter_folder_there_is() {
    let dir = TempDir::new("stacking-flats-no-filter");
    let night = night(dir.path());
    // The fixture's single light has no FILTER.
    flats(&night, "L", 3);
    let plan = plan_target(&night, "M31", "2026-10-07", &[], 12);
    assert_eq!(plan.jobs[0].flats, Some(night.join("calibration/flats/L")));
    flats(&night, "R", 3);
    let plan = plan_target(&night, "M31", "2026-10-07", &[], 12);
    assert_eq!(plan.jobs[0].flats, None, "two filters, none to choose");
}

/* ------------------------- Camera RAW files ------------------- */

use crate::raw_header::fixture::{Shot, write_raw};

fn raw_night(dir: &Path) -> PathBuf {
    let night = night(dir);
    fs::remove_file(night.join("M31/lights/Light_0001.fits")).unwrap();
    night
}

fn raw_light(
    night: &Path,
    name: &str,
    taken: &'static str,
    camera: Option<(&'static str, &'static str)>,
) {
    let mut shot = Shot::new(taken, 30, 800);
    if let Some((make, model)) = camera {
        shot.make = make;
        shot.model = model;
    }
    write_raw(&night.join("M31/lights").join(name), &shot);
}

#[test]
fn camera_raw_lights_are_one_family_per_camera_with_the_iso_as_the_gain() {
    let dir = TempDir::new("stacking-raw-families");
    let night = raw_night(dir.path());
    raw_light(&night, "a.NEF", "2026:10:07 23:00:00", None);
    raw_light(&night, "b.NEF", "2026:10:07 23:01:00", None);
    raw_light(&night, "c.NEF", "2026:10:07 23:02:00", None);
    raw_light(
        &night,
        "d.CR2",
        "2026:10:07 23:00:30",
        Some(("Canon", "Canon EOS 6D")),
    );
    raw_light(
        &night,
        "e.CR2",
        "2026:10:07 23:01:30",
        Some(("Canon", "Canon EOS 6D")),
    );
    let families = stack_families(&night, "M31");
    assert_eq!(
        families
            .iter()
            .map(|f| (f.label.as_str(), f.frames.len()))
            .collect::<Vec<_>>(),
        vec![
            ("nofilter_30s_g800_NIKON_D3300", 3),
            ("nofilter_30s_g800_Canon_EOS_6D", 2)
        ]
    );
    let plan = plan_target(&night, "M31", "2026-10-07", &[], 12);
    assert_eq!(plan.jobs.len(), 2);
    assert_eq!(
        plan.jobs[0].integration.as_deref(),
        Some("nofilter_30s_g800_NIKON_D3300")
    );
    // The job lists its frames: the script reads those and no other.
    assert_eq!(plan.jobs[0].light_count(), 3);
    // A space that keeps no variant gets the largest only, the other said.
    let single = plan_target(&night, "M31", "2026-10-07", &[], 0);
    assert_eq!(single.jobs.len(), 1);
    assert_eq!(single.set_aside[0].label, "nofilter_30s_g800_Canon_EOS_6D");
}

#[test]
fn raw_files_and_fits_frames_are_never_one_family() {
    let dir = TempDir::new("stacking-raw-and-fits");
    let night = raw_night(dir.path());
    for index in 0..2 {
        raw_light(
            &night,
            &format!("r{index}.NEF"),
            "2026:10:07 23:00:00",
            None,
        );
    }
    // FITS frames of the same exposure and the same gain.
    for index in 0..3 {
        let bytes = fits_bytes(
            &[
                ("IMAGETYP", "'LIGHT'"),
                ("EXPTIME", "30.0"),
                ("GAIN", "800"),
                ("XBINNING", "1"),
            ],
            0,
        );
        fs::write(night.join(format!("M31/lights/f{index}.fits")), bytes).unwrap();
    }
    let families = stack_families(&night, "M31");
    assert_eq!(
        families
            .iter()
            .map(|f| (f.label.as_str(), f.frames.len()))
            .collect::<Vec<_>>(),
        vec![
            ("nofilter_30s_g800", 3),
            ("nofilter_30s_g800_NIKON_D3300", 2)
        ]
    );
    for family in &families {
        let raw = family
            .frames
            .iter()
            .filter(|p| p.extension().is_some_and(|e| e == "NEF"))
            .count();
        assert!(raw == 0 || raw == family.frames.len(), "{family:?}");
    }
}

#[test]
fn a_camera_clock_without_a_zone_keeps_the_poses_of_the_night_in_the_integration() {
    let dir = TempDir::new("stacking-raw-window");
    let night = raw_night(dir.path());
    // 139.7° E, the night of the 7th: wall clock 23:30, 03:00 (the 8th) and
    // 21:00 (the 8th, the next evening).
    raw_light(&night, "a.NEF", "2026:10:07 23:30:00", None);
    raw_light(&night, "b.NEF", "2026:10:08 03:00:00", None);
    raw_light(&night, "c.NEF", "2026:10:08 21:00:00", None);
    let window = NightWindow::of_evening("2026-10-07", Some(139.7)).unwrap();
    let plan = plan_target_within(&night, "M31", "2026-10-07", &[], 12, Some(&window));
    assert_eq!(plan.outside, vec![night.join("M31/lights/c.NEF")]);
    assert_eq!(plan.jobs[0].light_count(), 2);
    // With its offset the clock is UTC and says the same.
    for (name, taken) in [
        ("a", "2026:10:07 23:30:00"),
        ("b", "2026:10:08 03:00:00"),
        ("c", "2026:10:08 21:00:00"),
    ] {
        let mut shot = Shot::new(taken, 30, 800);
        shot.offset = Some("+09:00");
        write_raw(&night.join(format!("M31/lights/{name}.NEF")), &shot);
    }
    let plan = plan_target_within(&night, "M31", "2026-10-07", &[], 12, Some(&window));
    assert_eq!(plan.outside, vec![night.join("M31/lights/c.NEF")]);
}

#[test]
fn a_folder_of_raw_files_has_lights() {
    let dir = TempDir::new("stacking-raw-has-lights");
    let night = raw_night(dir.path());
    assert!(!has_lights(&night.join("M31/lights")));
    raw_light(&night, "a.NEF", "2026:10:07 23:30:00", None);
    assert!(has_lights(&night.join("M31/lights")));
}

#[test]
fn the_gate_names_the_rule_that_refuses_a_job_in_the_order_the_run_applies_them() {
    let job = plan_stack(
        Path::new("/n"),
        "M31",
        "2026-10-07",
        &[PipelineChoice::new("raw"), PipelineChoice::new("hoo")],
    );
    let gate = |tool_present, variant_limit| StackGate {
        tool_present,
        variant_limit,
    };
    assert_eq!(
        gate(false, 0).refusal(&job),
        Some(StackFailure::ToolMissing)
    );
    assert_eq!(
        gate(true, 0).refusal(&job),
        Some(StackFailure::OneRenderOnly { asked: 2 })
    );
    assert_eq!(gate(true, 1).refusal(&job), None);
}

/* ------------- The filter the folder records, for lights that name none ------------- */

fn record_filter_of_m31(night: &Path, filter: Option<&str>) {
    let identity = crate::working_folder::TargetIdentity {
        format: 1,
        target_id: "m31".into(),
        filter: filter.map(str::to_string),
    };
    fs::write(
        night
            .join("M31")
            .join(crate::working_folder::TARGET_IDENTITY_FILE),
        serde_json::to_string(&identity).unwrap(),
    )
    .unwrap();
}

#[test]
fn lights_that_name_no_filter_take_the_one_the_target_folder_records() {
    let dir = TempDir::new("stacking-assumed-filter");
    let night = night(dir.path());
    // As an ASIAIR writes them: no FILTER card, and the stacking says so.
    let before = stack_families(&night, "M31");
    assert_eq!(before[0].filter, None);
    assert!(
        before[0].label.starts_with("nofilter"),
        "{}",
        before[0].label
    );

    record_filter_of_m31(&night, Some("Dualband"));
    let families = stack_families(&night, "M31");
    assert_eq!(families[0].filter.as_deref(), Some("Dualband"));
    assert!(
        families[0].label.starts_with("Dualband"),
        "{}",
        families[0].label
    );

    // The master is named after it and the script is told the filter, so the
    // master it keeps is stamped with it.
    let plan = plan_target(&night, "M31", "2026-10-07", &[], 12);
    assert_eq!(
        plan.jobs[0].integration.as_deref(),
        Some(families[0].label.as_str())
    );
    assert_eq!(plan.jobs[0].capture_filter.as_deref(), Some("Dualband"));
    let args = stack_args(&plan.jobs[0], &[], None, None, None);
    let at = args.iter().position(|a| a == "--capture-filter").unwrap();
    assert_eq!(args[at + 1], "Dualband");

    // Without a record nothing is passed.
    record_filter_of_m31(&night, None);
    let plan = plan_target(&night, "M31", "2026-10-07", &[], 12);
    assert_eq!(plan.jobs[0].capture_filter, None);
    assert!(
        !stack_args(&plan.jobs[0], &[], None, None, None).contains(&"--capture-filter".to_string())
    );
}

#[test]
fn a_filter_the_headers_name_wins_over_the_recorded_one() {
    let dir = TempDir::new("stacking-header-filter-wins");
    let night = night(dir.path());
    fs::remove_file(night.join("M31/lights/Light_0001.fits")).unwrap();
    light(&night, "Ha_001.fits", "Ha", "300.0", "-10.0");
    record_filter_of_m31(&night, Some("Dualband"));
    let families = stack_families(&night, "M31");
    assert_eq!(families.len(), 1);
    assert_eq!(families[0].filter.as_deref(), Some("Ha"));
}

#[test]
fn the_frame_set_by_hand_goes_to_the_script_and_automatic_says_nothing() {
    let dir = TempDir::new("stacking-frame");
    let night = night(dir.path());
    let job = plan_stack(&night, "M31", "2026-10-07", &[]);
    assert_eq!(job.frame, None);
    assert!(!stack_args(&job, &[], None, None, None).contains(&"--frame-size".to_string()));
    let plan = TargetPlan {
        jobs: vec![job.clone(), job.clone()],
        set_aside: Vec::new(),
        outside: Vec::new(),
    };
    // One frame for the target: every family takes the same rank.
    let framed = plan.clone().with_frame(Some(2));
    assert!(framed.jobs.iter().all(|job| job.frame == Some(2)));
    let args = stack_args(&framed.jobs[0], &[], None, None, None).join(" ");
    assert!(args.contains("--frame-size 2"), "{args}");
    // A rank beyond the notches is Automatic.
    assert!(
        plan.clone()
            .with_frame(Some(FRAME_NOTCHES))
            .jobs
            .iter()
            .all(|job| job.frame.is_none())
    );
    // The image of a device was registered by nobody: no frame.
    let mut device = framed.jobs[0].clone();
    device.device_master = Some(night.join("M31/device/Stacked_1.fit"));
    assert!(!stack_args(&device, &[], None, None, None).contains(&"--frame-size".to_string()));
    // The frame travels with the job file; a job without one reads as Automatic.
    let read: StackJob =
        serde_json::from_str(&serde_json::to_string(&framed.jobs[0]).unwrap()).unwrap();
    assert_eq!(read.frame, Some(2));
    let written = serde_json::to_value(&job).unwrap();
    assert!(written.get("frame").is_none());
}

#[test]
fn the_frame_of_a_target_is_read_from_its_last_stackings() {
    let dir = TempDir::new("stacking-target-frame");
    let night = night(dir.path());
    assert_eq!(target_frame(&night, "M31"), None);
    let stacked = night.join("M31/stacked");
    fs::create_dir_all(&stacked).unwrap();
    let record = |registered: u32, degrees: f64, notches: usize| {
        serde_json::json!({
            "registration": {
                "frames": registered, "registered": registered, "unregistered": [],
                "fieldRotationDegrees": degrees, "framing": "full",
                "frame": {
                    "frameHeight": 3840, "frameWidth": 2160, "automatic": 4, "requested": "auto", "rank": null,
                    "notches": (0..notches).map(|rank| serde_json::json!({
                        "rank": rank, "top": 0, "left": 0, "height": 3840, "width": 2160,
                        "shareOfFrame": 1.0, "fullShare": 0.7, "lowestFrames": 1
                    })).collect::<Vec<_>>()
                }
            }
        })
        .to_string()
    };
    // A record of before L2 (no frame) and the image of a device say nothing.
    fs::write(
        stacked.join("M31_old_stacking.json"),
        r#"{"registration": {"frames": 400, "registered": 400, "framing": "full"}}"#,
    )
    .unwrap();
    fs::write(
        stacked.join("M31_device_stacking.json"),
        r#"{"registration": null}"#,
    )
    .unwrap();
    // Two families: the one with the most lights registered gives the measure.
    fs::write(stacked.join("M31_Ha_stacking.json"), record(120, 125.3, 5)).unwrap();
    fs::write(stacked.join("M31_OIII_stacking.json"), record(60, 110.0, 5)).unwrap();
    let found = target_frame(&night, "M31").unwrap();
    assert_eq!(found.registered, 120);
    assert_eq!(found.field_rotation_degrees, Some(125.3));
    assert_eq!(found.measure.notches.len(), 5);
    assert_eq!(
        (found.measure.automatic, found.measure.rank),
        (Some(4), None)
    );
    assert_eq!(
        serde_json::to_value(&found).unwrap()["measure"]["notches"][0]["shareOfFrame"],
        serde_json::json!(1.0)
    );
}

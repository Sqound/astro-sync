use super::*;
use crate::test_support::{TempDir, fits_bytes};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;

/// A night with `M45/lights/`: `count10` frames of 10 s then `count30` of 30 s,
/// named by their order; the exposure is in the header.
fn night(dir: &Path, count10: usize, count30: usize) -> PathBuf {
    let night = dir.join("2026-10-10 · Maison · abc123");
    fs::create_dir_all(night.join("M45/lights")).unwrap();
    let write = |name: String, exposure: &str| {
        let bytes = fits_bytes(
            &[
                ("IMAGETYP", "'LIGHT'"),
                ("OBJECT", "'M45'"),
                ("FILTER", "'IRCUT'"),
                ("EXPTIME", exposure),
                ("GAIN", "200"),
                ("XBINNING", "1"),
                ("YBINNING", "1"),
                ("DATE-OBS", "'2026-10-10T14:00:00'"),
            ],
            0,
        );
        fs::write(night.join("M45/lights").join(name), bytes).unwrap();
    };
    for i in 0..count10 {
        write(format!("a_{i:04}.fit"), "10.0");
    }
    for i in 0..count30 {
        write(format!("b_{i:04}.fit"), "30.0");
    }
    night
}

fn raw(
    path: &Path,
    unix_offset: f64,
    fwhm: Option<f64>,
    ecc: Option<f64>,
    stars: u32,
    sky: f64,
) -> RawPose {
    // 2026-10-10T12:00:00Z + offset seconds.
    let base = parse_iso8601_utc_to_unix_seconds("2026-10-10T12:00:00Z").unwrap();
    let seconds = base + unix_offset;
    let stamp = crate::time_util::system_time_to_iso8601_utc(
        std::time::UNIX_EPOCH + Duration::from_secs_f64(seconds),
    );
    RawPose {
        path: path.to_path_buf(),
        status: "measured".to_string(),
        date_obs: Some(stamp.trim_end_matches('Z').to_string()),
        fwhm_pixels: fwhm,
        eccentricity: ecc,
        star_count: Some(stars),
        sky_background: Some(sky),
        star_widths: Vec::new(),
        star_eccentricities: Vec::new(),
    }
}

/// A stand-in: answers from a table (path -> pose), counts what it was asked,
/// and can stop the measure after some answers.
struct Fake {
    answers: HashMap<PathBuf, RawPose>,
    asked: RefCell<Vec<Vec<PathBuf>>>,
    stop_after: Option<(usize, CancelFlag)>,
    answered: RefCell<usize>,
}

impl Fake {
    fn new(answers: Vec<RawPose>) -> Self {
        Self {
            answers: answers.into_iter().map(|a| (a.path.clone(), a)).collect(),
            asked: RefCell::new(Vec::new()),
            stop_after: None,
            answered: RefCell::new(0),
        }
    }
}

impl Measurer for Fake {
    fn measure(
        &self,
        frames: &[PathBuf],
        cancel: Option<&CancelFlag>,
        on_pose: &mut dyn FnMut(RawPose),
    ) -> Result<(), String> {
        self.asked.borrow_mut().push(frames.to_vec());
        for frame in frames {
            if cancel.is_some_and(CancelFlag::is_cancelled) {
                return Ok(());
            }
            if let Some(answer) = self.answers.get(frame) {
                on_pose(answer.clone());
            }
            *self.answered.borrow_mut() += 1;
            if let Some((count, flag)) = &self.stop_after
                && *self.answered.borrow() >= *count
            {
                flag.cancel();
            }
        }
        Ok(())
    }
}

fn request(night: &Path) -> MeasureRequest {
    MeasureRequest {
        night_folder: night.to_path_buf(),
        target_folder: "M45".to_string(),
        evening_date: "2026-10-10".to_string(),
        // Japan: the night's window is local noon to local noon.
        longitude: Some(139.7),
        latitude: Some(36.7),
        resume: Vec::new(),
    }
}

/// Answers for every frame of the 30 s family: a steady night, with a cloud
/// (few stars) at `cloud`, trailed stars at `trailed`, and a rising sky from
/// `dawn_from`.
fn steady_night(
    frames: &[PathBuf],
    start_seconds: f64,
    cloud: &[usize],
    trailed: &[usize],
    dawn_from: usize,
) -> Vec<RawPose> {
    frames
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let at = start_seconds + i as f64 * 30.0;
            let jitter = (i % 5) as f64 * 0.02;
            let mut sky = 400.0 + (i % 3) as f64;
            if i >= dawn_from {
                sky *= 4.0 + (i - dawn_from) as f64;
            }
            let (mut fwhm, mut ecc, mut stars) = (Some(3.2 + jitter), Some(0.31 + jitter), 240);
            if cloud.contains(&i) {
                stars = 40;
                fwhm = Some(3.5);
                ecc = Some(0.33);
            }
            if trailed.contains(&i) {
                ecc = Some(0.6);
            }
            raw(path, at, fwhm, ecc, stars, sky)
        })
        .collect()
}

#[test]
fn a_family_up_to_two_hundred_poses_is_measured_whole_and_above_one_in_four() {
    assert_eq!(sample_indices(36), (0..36).collect::<Vec<_>>());
    assert_eq!(sample_indices(200).len(), 200);
    let sampled = sample_indices(430);
    assert_eq!(sampled.len(), 109, "108 in four plus the last");
    assert_eq!(sampled[1], 4);
    assert_eq!(*sampled.last().unwrap(), 429);
    assert_eq!(
        sample_indices(201).len(),
        51,
        "0, 4, … 200: the last is a multiple of four"
    );
    assert!(sample_indices(0).is_empty());
}

#[test]
fn the_families_are_the_stackings_and_never_mix() {
    let dir = TempDir::new("pose-families");
    let night = night(dir.path(), 4, 6);
    let families = pose_families(&night, "M45", None);
    assert_eq!(families.len(), 2);
    assert_eq!(families[0].frames.len(), 6, "the largest first");
    assert_eq!(families[0].exposure_seconds, Some(30.0));
    assert_eq!(families[1].label, "IRCUT_10s_g200");
    assert_eq!(families[1].exposure_seconds, Some(10.0));
    // Sorted by path.
    assert!(families[0].frames.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn a_pose_is_flagged_with_its_first_reason_and_a_steady_night_flags_nothing() {
    let frames: Vec<PathBuf> = (0..40)
        .map(|i| PathBuf::from(format!("f{i:03}.fit")))
        .collect();
    let steady: Vec<PoseMeasure> = steady_night(&frames, 0.0, &[], &[], 1000)
        .into_iter()
        .enumerate()
        .map(|(i, r)| r.into_measure(i))
        .collect();
    let stats = judge("IRCUT_30s_g200", Some(30.0), 40, &steady, None);
    assert!(stats.judged);
    assert!(stats.flagged.is_empty());
    assert!((stats.medians.fwhm_pixels.unwrap() - 3.24).abs() < 0.05);

    let departing: Vec<PoseMeasure> = steady_night(&frames, 0.0, &[10, 11], &[20], 38)
        .into_iter()
        .enumerate()
        .map(|(i, r)| r.into_measure(i))
        .collect();
    let stats = judge("IRCUT_30s_g200", Some(30.0), 40, &departing, None);
    let reasons: Vec<(usize, Reason)> = stats.flagged.iter().map(|f| (f.index, f.reason)).collect();
    assert_eq!(reasons[0], (10, Reason::Clouds));
    assert_eq!(reasons[1], (11, Reason::Clouds));
    assert_eq!(reasons[2], (20, Reason::Trailed));
    // No place: a high sky is a high sky, not dawn.
    assert!(matches!(reasons[3], (38, Reason::Sky { .. })));
    assert!(matches!(reasons[4], (39, Reason::Sky { .. })));
}

#[test]
fn the_dawn_needs_the_sun_just_under_the_horizon() {
    let frames: Vec<PathBuf> = (0..40)
        .map(|i| PathBuf::from(format!("f{i:03}.fit")))
        .collect();
    // Japan, 10 October: sunrise is about 05:30 (UTC+9) = 20:30 UTC the day
    // before, so a pose at 19:55 UTC is in the twilight; the night frames
    // start at 14:00 UTC (23:00 local) and are far under the horizon.
    let base = parse_iso8601_utc_to_unix_seconds("2026-10-10T12:00:00Z").unwrap();
    let place = Some(Place {
        latitude: 36.7,
        longitude: 139.99,
    });
    let night_start = 2.0 * 3600.0; // 14:00 UTC
    let mut poses: Vec<PoseMeasure> = steady_night(&frames, night_start, &[], &[], 1000)
        .into_iter()
        .enumerate()
        .map(|(i, r)| r.into_measure(i))
        .collect();
    // Two late poses at a sky ten times higher: one in the twilight (19:55
    // UTC), one at 14:30 UTC (deep night: a lamp, not dawn).
    let late = |at_utc_hours: f64| base + (at_utc_hours - 12.0) * 3600.0;
    poses[38].unix = Some(late(19.0 + 55.0 / 60.0));
    poses[38].sky_background = Some(4000.0);
    poses[5].unix = Some(late(14.5));
    poses[5].sky_background = Some(4100.0);
    let stats = judge("x", None, 40, &poses, place);
    let by_index: HashMap<usize, Reason> =
        stats.flagged.iter().map(|f| (f.index, f.reason)).collect();
    assert!(matches!(by_index[&38], Reason::Dawn { factor } if (factor - 10.0).abs() < 0.5));
    assert!(matches!(by_index[&5], Reason::Sky { .. }));
}

#[test]
fn under_ten_poses_there_is_no_median_so_nothing_is_flagged() {
    let frames: Vec<PathBuf> = (0..9).map(|i| PathBuf::from(format!("f{i}.fit"))).collect();
    let poses: Vec<PoseMeasure> = steady_night(&frames, 0.0, &[3], &[], 100)
        .into_iter()
        .enumerate()
        .map(|(i, r)| r.into_measure(i))
        .collect();
    let stats = judge("x", None, 9, &poses, None);
    assert!(!stats.judged);
    assert!(stats.flagged.is_empty());
    assert_eq!(stats.medians, Medians::default());
}

#[test]
fn under_twenty_stars_no_shape_but_the_count_and_the_sky_are_judged() {
    // A narrow band: 31 of 40 poses have no width; the cloud is still told.
    let frames: Vec<PathBuf> = (0..40)
        .map(|i| PathBuf::from(format!("n{i:02}.fit")))
        .collect();
    let poses: Vec<PoseMeasure> = frames
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let stars = if i == 7 { 3 } else { 15 + (i % 4) as u32 };
            let shape = (i < 9).then_some(3.0);
            raw(
                path,
                i as f64 * 180.0,
                shape,
                shape.map(|_| 0.3),
                stars,
                520.0,
            )
            .into_measure(i)
        })
        .collect();
    let stats = judge("Ha_180s_g100", Some(180.0), 40, &poses, None);
    assert!(stats.judged);
    assert_eq!(
        stats.medians.fwhm_pixels, None,
        "nine poses: no median of a width"
    );
    assert_eq!(stats.shape_missing, Some(31));
    assert!(stats.medians.star_count.is_some());
    // Pose 7 has 3 stars (< half of 16): a cloud even though it has no shape.
    assert_eq!(
        stats
            .flagged
            .iter()
            .map(|f| (f.index, f.reason))
            .collect::<Vec<_>>(),
        vec![(7, Reason::Clouds)]
    );
}

/// Sixteen poses of a sparse field, none with a width of its own, the stars
/// they did read handed over (`per_pose[i]` stars of pose `i`, widths cycling
/// through `widths`).
fn sparse_poses(per_pose: &[usize], widths: &[f64]) -> Vec<PoseMeasure> {
    let mut next = 0;
    per_pose
        .iter()
        .enumerate()
        .map(|(i, count)| {
            let mut pose = raw(
                &PathBuf::from(format!("s{i:02}.fit")),
                i as f64 * 32.0,
                None,
                None,
                30,
                1000.0,
            )
            .into_measure(i);
            for _ in 0..*count {
                pose.star_widths.push(widths[next % widths.len()]);
                pose.star_eccentricities.push(0.4);
                next += 1;
            }
            pose
        })
        .collect()
}

#[test]
fn stars_too_few_in_each_pose_give_a_width_of_the_family_and_none_of_a_pose() {
    // 25 stars over 16 poses (one or two each): no pose has a width, the
    // family has the median of the pooled stars.
    let per_pose = [2, 1, 2, 1, 2, 1, 2, 1, 2, 1, 2, 1, 2, 1, 2, 2];
    assert_eq!(per_pose.iter().sum::<usize>(), 25);
    let poses = sparse_poses(&per_pose, &[2.6, 2.8, 3.0, 3.2, 3.4]);
    let stats = judge("R_32s_g100", Some(32.0), 16, &poses, None);
    assert!(stats.judged);
    assert!(poses.iter().all(|p| p.fwhm_pixels.is_none()));
    assert_eq!(stats.medians.fwhm_pixels, None, "no width for a pose");
    assert_eq!(stats.shape_missing, Some(16));
    let family = stats.family_shape.expect("a width for the family");
    assert_eq!(family.stars, 25);
    assert_eq!(family.poses, 16);
    assert!((family.fwhm_pixels - 3.0).abs() < 1e-9, "{family:?}");
    assert_eq!(family.eccentricity, Some(0.4));
    // The family's width never flags a pose as wide or trailed.
    assert!(
        stats
            .flagged
            .iter()
            .all(|f| !matches!(f.reason, Reason::Wide | Reason::Trailed))
    );
}

#[test]
fn under_eight_stars_in_all_nothing_is_given_for_the_family() {
    let poses = sparse_poses(&[1, 0, 2, 0, 1, 0, 1, 0, 1, 0, 0, 0, 0, 1, 0, 0], &[3.0]);
    let stars: usize = poses.iter().map(|p| p.star_widths.len()).sum();
    assert_eq!(stars, 7);
    let stats = judge("R_32s_g100", Some(32.0), 16, &poses, None);
    assert_eq!(stats.family_shape, None);
    assert_eq!(stats.shape_missing, Some(16));
    // Eight is enough.
    let eight = sparse_poses(&[1, 0, 2, 0, 1, 0, 1, 0, 1, 0, 0, 0, 0, 1, 0, 1], &[3.0]);
    let stats = judge("R_32s_g100", Some(32.0), 16, &eight, None);
    assert_eq!(stats.family_shape.map(|s| s.stars), Some(8));
}

#[test]
fn a_family_whose_poses_have_a_width_is_not_given_a_pooled_one() {
    let frames: Vec<PathBuf> = (0..14)
        .map(|i| PathBuf::from(format!("w{i:02}.fit")))
        .collect();
    let mut poses: Vec<PoseMeasure> = steady_night(&frames, 0.0, &[], &[], 100)
        .into_iter()
        .enumerate()
        .map(|(i, r)| r.into_measure(i))
        .collect();
    poses[2].star_widths = vec![3.0; 12];
    let stats = judge("x", None, 14, &poses, None);
    assert!(stats.medians.fwhm_pixels.is_some());
    assert_eq!(stats.family_shape, None);
}

#[test]
fn the_stars_handed_over_by_the_script_are_read_from_its_line() {
    let line = r#"{"path":"a.fit","status":"measured","dateObs":"2026-10-10T14:00:00","fwhmPixels":null,"eccentricity":null,"starCount":31,"readableStars":2,"skyBackground":1204.0,"starWidths":[3.1,2.9],"starEccentricities":[0.4,0.5]}"#;
    let raw: RawPose = serde_json::from_str(line).unwrap();
    let pose = raw.into_measure(0);
    assert_eq!(pose.star_widths, vec![3.1, 2.9]);
    assert_eq!(pose.star_eccentricities, vec![0.4, 0.5]);
    // A pose with a width of its own carries no list, and sends none.
    let own: RawPose = serde_json::from_str(
        r#"{"path":"b.fit","status":"measured","fwhmPixels":3.2,"eccentricity":0.3,"starCount":400}"#,
    )
    .unwrap();
    let json = serde_json::to_string(&own.into_measure(1)).unwrap();
    assert!(!json.contains("starWidths"), "{json}");
}

#[test]
fn two_families_are_measured_apart_and_the_sample_adds_the_neighbours_of_a_flagged_pose() {
    let dir = TempDir::new("pose-measure");
    let night = night(dir.path(), 12, 300);
    let families = pose_families(&night, "M45", None);
    let thirty = &families[0];
    assert_eq!(thirty.frames.len(), 300);
    // A cloud at pose 101 (a sampled pose is 100 and 104: 101 is a neighbour,
    // not in the sample) and at 104 (sampled): the 104 is seen, its
    // neighbours are then measured and 101 turns out to be a cloud too.
    let mut answers = steady_night(&thirty.frames, 0.0, &[101, 104], &[], 1000);
    answers.extend(families[1].frames.iter().enumerate().map(|(i, p)| {
        raw(
            p,
            -9000.0 + i as f64 * 10.0,
            Some(3.3),
            Some(0.34),
            119,
            139.0,
        )
    }));
    let fake = Fake::new(answers);
    let mut events = Vec::new();
    let stats = measure_target(&request(&night), &fake, None, &mut |e| {
        events.push(e.clone())
    })
    .unwrap();
    let asked = fake.asked.borrow();
    assert_eq!(asked.len(), 2, "the sample, then the neighbours");
    // 12 of 10 s whole + one in four of 300 (75) and the last.
    assert_eq!(asked[0].len(), 12 + 76);
    assert!(asked[1].iter().all(|p| thirty.frames.contains(p)));
    assert!(asked[1].contains(&thirty.frames[101]));
    assert!(!asked[1].contains(&thirty.frames[104]), "already measured");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, PoseEvent::Neighbours { count, .. } if *count == asked[1].len()))
    );
    assert!(!stats.stopped);
    assert_eq!(stats.total, 312);
    assert_eq!(stats.planned, 88 + asked[1].len());
    let thirty_stats = &stats.families[0];
    assert_eq!(thirty_stats.label, thirty.label);
    let flagged: Vec<usize> = thirty_stats.flagged.iter().map(|f| f.index).collect();
    assert_eq!(flagged, vec![101, 104]);
    // The 10 s family has its own medians and flags nothing.
    assert!(stats.families[1].flagged.is_empty());
    assert_eq!(stats.families[1].medians.sky_background, Some(139.0));
    assert_eq!(stats.flagged_count(), 2);
    // By time.
    assert!(
        thirty_stats
            .measured
            .windows(2)
            .all(|w| w[0].unix.unwrap() <= w[1].unix.unwrap())
    );
    // The last event says everything measured and planned.
    let Some(PoseEvent::Pose { done, planned, .. }) = events
        .iter()
        .rfind(|e| matches!(e, PoseEvent::Pose { .. }))
        .cloned()
    else {
        panic!("a pose event");
    };
    assert_eq!(done, stats.measured_count());
    assert_eq!(planned, stats.planned);
}

#[test]
fn a_stop_keeps_what_is_measured_and_a_resume_measures_only_the_rest() {
    let dir = TempDir::new("pose-stop");
    let night = night(dir.path(), 0, 30);
    let families = pose_families(&night, "M45", None);
    let answers = steady_night(&families[0].frames, 0.0, &[], &[], 1000);
    let mut fake = Fake::new(answers.clone());
    let flag = CancelFlag::new();
    fake.stop_after = Some((12, flag.clone()));
    let stats = measure_target(&request(&night), &fake, Some(&flag), &mut |_| {}).unwrap();
    assert!(stats.stopped);
    assert_eq!(stats.measured_count(), 12);
    assert_eq!(stats.planned, 30);
    assert!(stats.families[0].judged, "twelve poses make a median");

    // Resumed: only the 18 others are asked.
    let kept: Vec<PoseMeasure> = stats.families[0].measured.clone();
    let again = Fake::new(answers);
    let mut next = request(&night);
    next.resume = kept;
    let stats = measure_target(&next, &again, None, &mut |_| {}).unwrap();
    assert_eq!(again.asked.borrow()[0].len(), 18);
    assert!(!stats.stopped);
    assert_eq!(stats.measured_count(), 30);
}

#[test]
fn a_measure_that_cannot_run_says_why_unless_some_poses_were_measured() {
    struct Broken;
    impl Measurer for Broken {
        fn measure(
            &self,
            _: &[PathBuf],
            _: Option<&CancelFlag>,
            _: &mut dyn FnMut(RawPose),
        ) -> Result<(), String> {
            Err("python missing".to_string())
        }
    }
    let dir = TempDir::new("pose-broken");
    let night = night(dir.path(), 0, 12);
    let error = measure_target(&request(&night), &Broken, None, &mut |_| {}).unwrap_err();
    assert_eq!(error, "python missing");
}

#[test]
fn an_unreadable_pose_is_neither_judged_nor_counted_in_the_medians() {
    let frames: Vec<PathBuf> = (0..14)
        .map(|i| PathBuf::from(format!("f{i:02}.fit")))
        .collect();
    let mut poses: Vec<PoseMeasure> = steady_night(&frames, 0.0, &[], &[], 100)
        .into_iter()
        .enumerate()
        .map(|(i, r)| r.into_measure(i))
        .collect();
    poses[3] = RawPose {
        path: frames[3].clone(),
        status: "unreadable".to_string(),
        date_obs: None,
        fwhm_pixels: None,
        eccentricity: None,
        star_count: None,
        sky_background: None,
        star_widths: Vec::new(),
        star_eccentricities: Vec::new(),
    }
    .into_measure(3);
    let stats = judge("x", None, 14, &poses, None);
    assert_eq!(stats.measured.len(), 13);
    assert!(stats.flagged.is_empty());
}

#[test]
fn discarding_leaves_the_poses_out_of_the_jobs_and_deletes_nothing() {
    let dir = TempDir::new("pose-discard");
    let night = night(dir.path(), 0, 10);
    let mut plan = crate::stacking::plan_target_within(&night, "M45", "2026-10-10", &[], 3, None);
    assert!(plan.jobs[0].frames.is_empty(), "every frame of lights/");
    let families = pose_families(&night, "M45", None);
    let discarded: HashSet<PathBuf> = families[0].frames[7..].iter().cloned().collect();
    let left_out = apply_discard(&mut plan, &discarded, &night, "M45", None);
    assert_eq!(left_out, 3);
    assert_eq!(plan.jobs[0].frames.len(), 7);
    assert!(plan.jobs[0].frames.iter().all(|f| !discarded.contains(f)));
    assert_eq!(plan.jobs[0].light_count(), 7);
    for frame in &discarded {
        assert!(frame.exists(), "nothing is deleted");
    }
    // Nothing to discard: the plan is untouched.
    let mut other = crate::stacking::plan_target_within(&night, "M45", "2026-10-10", &[], 3, None);
    assert_eq!(
        apply_discard(&mut other, &HashSet::new(), &night, "M45", None),
        0
    );
    assert!(other.jobs[0].frames.is_empty());
    // A job that renders the image a device stacked reads no pose: nothing to leave out.
    let mut device = crate::stacking::plan_target_within(&night, "M45", "2026-10-10", &[], 3, None);
    device.jobs[0].device_master = Some(night.join("M45/device/Stacked_10.fit"));
    assert_eq!(
        apply_discard(&mut device, &discarded, &night, "M45", None),
        0
    );
    assert!(device.jobs[0].frames.is_empty());
    // Every pose discarded: the job is left as it is, a stacking needs a pose.
    let everything: HashSet<PathBuf> = families[0].frames.iter().cloned().collect();
    assert_eq!(
        apply_discard(&mut other, &everything, &night, "M45", None),
        0
    );
    assert!(other.jobs[0].frames.is_empty());
}

#[test]
fn the_answer_of_the_script_is_read_line_by_line() {
    let line = r#"{"path": "/n/M45/lights/a.fit", "status": "measured", "dateObs": "2026-10-10T14:31:15.649137", "exposure": 30.0, "skyBackground": 3186.0, "starCount": 224, "fwhmPixels": 4.317, "eccentricity": 0.538}"#;
    let raw: RawPose = serde_json::from_str(line).unwrap();
    let pose = raw.into_measure(5);
    assert_eq!(pose.index, 5);
    assert_eq!(pose.star_count, Some(224));
    assert!(!pose.unreadable);
    let expected = parse_iso8601_utc_to_unix_seconds("2026-10-10T14:31:15Z").unwrap() + 0.649137;
    assert!((pose.unix.unwrap() - expected).abs() < 1e-6);
    let unreadable: RawPose =
        serde_json::from_str(r#"{"path": "/n/b.fit", "status": "unreadable", "message": "x"}"#)
            .unwrap();
    assert!(unreadable.into_measure(0).unreadable);
}

#[test]
fn the_python_measurer_runs_a_script_standing_for_the_measure() {
    // A shell script stands for `python pose_stats.py --frames-from LIST`: it
    // answers one line per frame of the list.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("pose-script");
        let python = dir.path().join("fake-python");
        fs::write(
            &python,
            "#!/bin/sh\nfor last; do :; done\nwhile IFS= read -r f; do printf '{\"path\": \"%s\", \"status\": \"measured\", \"skyBackground\": 100.0, \"starCount\": 50}\\n' \"$f\"; done < \"$last\"\n",
        )
        .unwrap();
        fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).unwrap();
        let night = dir.path().join("night");
        fs::create_dir_all(&night).unwrap();
        let measurer = PythonMeasurer::for_night(&night, python, PathBuf::from("pose_stats.py"));
        let frames = vec![PathBuf::from("/n/a.fit"), PathBuf::from("/n/b.fit")];
        let mut got = Vec::new();
        measurer
            .measure(&frames, None, &mut |r| got.push(r))
            .unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[1].path, PathBuf::from("/n/b.fit"));
        assert_eq!(got[0].star_count, Some(50));
        assert!(!measurer.scratch.exists(), "the scratch is removed");
    }
}

// ---------------------------------------------------------------------------
// The real nights: poses cut from the measures of the Seestar night of M 45
// and of three Askar 50P nights, tuned there. The fixture holds the figures
// of each pose (width, eccentricity, stars, sky), never a pixel.
// ---------------------------------------------------------------------------

/// One family of the fixture, as measured poses, and the place of the night.
fn real_family(key: &str) -> (Vec<PoseMeasure>, Place) {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/real_nights.json")).unwrap();
    let night = &fixture[key];
    let first = parse_iso8601_utc_to_unix_seconds(night["firstPoseUtc"].as_str().unwrap()).unwrap();
    let poses = night["poses"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let offset = row[0].as_f64().unwrap();
            PoseMeasure {
                path: PathBuf::from(format!("{key}/{index:03}.fit")),
                index,
                date_obs: None,
                unix: Some(first + offset),
                fwhm_pixels: row[1].as_f64(),
                eccentricity: row[2].as_f64(),
                star_count: row[3].as_u64().map(|n| n as u32),
                sky_background: row[4].as_f64(),
                star_widths: Vec::new(),
                star_eccentricities: Vec::new(),
                unreadable: false,
            }
        })
        .collect();
    let place = Place {
        latitude: night["latitude"].as_f64().unwrap(),
        longitude: night["longitude"].as_f64().unwrap(),
    };
    (poses, place)
}

fn reasons(stats: &FamilyStats) -> Vec<(usize, Reason)> {
    stats.flagged.iter().map(|f| (f.index, f.reason)).collect()
}

fn count_of(stats: &FamilyStats, wanted: fn(&Reason) -> bool) -> usize {
    stats.flagged.iter().filter(|f| wanted(&f.reason)).count()
}

#[test]
fn the_start_of_the_dawn_of_the_seestar_night_is_dawn_and_not_a_probable_cloud() {
    let (poses, place) = real_family("seestar_m45_30s");
    assert_eq!(poses.len(), 430);
    let stats = judge("IRCUT_30s_g200", Some(30.0), 430, &poses, Some(place));
    // The two poses of 04:49 and 04:52 (sky 1.9 and 2.5 times the median,
    // fewer stars): the first of the dawn, no cloud.
    assert_eq!(count_of(&stats, |r| matches!(r, Reason::Clouds)), 0);
    for index in [418, 419] {
        let found = stats.flagged.iter().find(|f| f.index == index).unwrap();
        assert!(matches!(found.reason, Reason::Dawn { .. }), "{found:?}");
    }
    // The rise is one block, from the first pose at 1.5 times the median
    // (04:46) to the last pose of the night.
    let dawn: Vec<usize> = stats
        .flagged
        .iter()
        .filter(|f| matches!(f.reason, Reason::Dawn { .. }))
        .map(|f| f.index)
        .collect();
    assert_eq!(dawn, (413..430).collect::<Vec<_>>());
    assert_eq!(stats.flagged.len(), 17, "nothing else is flagged");
}

#[test]
fn the_scatter_of_a_small_sensor_and_the_field_rotation_are_not_trailed_stars() {
    // The 10 s poses (eccentricity 0.37 to 0.69 around 0.45): four were told
    // « trailed » against the median plus 0.15.
    let (ten, place) = real_family("seestar_m45_10s");
    let stats = judge("IRCUT_10s_g200", Some(10.0), 36, &ten, Some(place));
    assert!(stats.flagged.is_empty(), "{:?}", reasons(&stats));
    // The 30 s poses: the field rotation takes the eccentricity of a Seestar
    // night from 0.5 up to 0.9 and back; against the median of the whole
    // night 67 poses were « trailed ».
    let (thirty, place) = real_family("seestar_m45_30s");
    let stats = judge("IRCUT_30s_g200", Some(30.0), 430, &thirty, Some(place));
    assert_eq!(count_of(&stats, |r| matches!(r, Reason::Trailed)), 0);
}

#[test]
fn trailed_stars_are_still_found_against_the_local_median_and_the_noise_of_the_night() {
    // A cooled Askar night (scatter 0.03): a pose 0.2 above its neighbours
    // is trailed (the margin is 0.15).
    let (mut poses, place) = real_family("sadr_2026_08_14");
    let local = poses[50].eccentricity.unwrap();
    poses[50].eccentricity = Some(local + 0.3);
    let stats = judge("x", Some(180.0), 114, &poses, Some(place));
    assert!(
        reasons(&stats).contains(&(50, Reason::Trailed)),
        "{:?}",
        reasons(&stats)
    );
    // On the Seestar night (scatter 0.07: the limit is four times that) a
    // pose 0.45 above the eccentricity of its neighbours is, one 0.2 above
    // is the scatter.
    let (mut poses, place) = real_family("seestar_m45_30s");
    let near = poses[200].eccentricity.unwrap();
    poses[200].eccentricity = Some(near + 0.45);
    poses[100].eccentricity = Some(poses[100].eccentricity.unwrap() + 0.2);
    let stats = judge("x", Some(30.0), 430, &poses, Some(place));
    let trailed: Vec<usize> = stats
        .flagged
        .iter()
        .filter(|f| matches!(f.reason, Reason::Trailed))
        .map(|f| f.index)
        .collect();
    assert_eq!(trailed, vec![200]);
}

#[test]
fn the_askar_dawns_lose_their_probable_cloud_and_a_real_cloud_stays() {
    // Sadr 14/15 August: the pose at 03:34 UTC (sky 2.75 times) was a
    // « cloud », the real cloud of the night is the pose 63.
    let (poses, place) = real_family("sadr_2026_08_14");
    let stats = judge("x", Some(180.0), 114, &poses, Some(place));
    let flagged = reasons(&stats);
    assert_eq!(flagged[0], (63, Reason::Clouds));
    assert!(
        flagged[1..]
            .iter()
            .all(|(_, r)| matches!(r, Reason::Dawn { .. }))
    );
    assert_eq!(
        flagged[1..].iter().map(|(i, _)| *i).collect::<Vec<_>>(),
        (107..114).collect::<Vec<_>>()
    );
    // SH2-190 9/10 August: the pose 96 (sky 2.98 times, below the factor of
    // three) was a cloud; the whole rise is the dawn.
    let (poses, place) = real_family("sh2_190_2026_08_09");
    let stats = judge("x", Some(180.0), 100, &poses, Some(place));
    assert_eq!(
        reasons(&stats).iter().map(|(i, _)| *i).collect::<Vec<_>>(),
        (94..100).collect::<Vec<_>>()
    );
    assert_eq!(count_of(&stats, |r| matches!(r, Reason::Clouds)), 0);
    // A night with clouds (Mu Cephei 19/20 August: a cloudy second half):
    // the clouds are told as before.
    let (poses, place) = real_family("mu_cephei_2026_08_19");
    let stats = judge("x", Some(180.0), 120, &poses, Some(place));
    assert_eq!(count_of(&stats, |r| matches!(r, Reason::Clouds)), 22);
}

#[test]
fn the_dawn_is_found_on_the_sample_too_not_only_on_the_whole_night() {
    // What the window measures first: one pose in four (and the last); the
    // neighbours of the flagged poses come after. Neither pass calls the start
    // of the dawn a cloud.
    let (poses, place) = real_family("seestar_m45_30s");
    let sampled: Vec<PoseMeasure> = sample_indices(430)
        .into_iter()
        .map(|i| poses[i].clone())
        .collect();
    let stats = judge("x", Some(30.0), 430, &sampled, Some(place));
    assert_eq!(count_of(&stats, |r| matches!(r, Reason::Clouds)), 0);
    assert_eq!(count_of(&stats, |r| matches!(r, Reason::Trailed)), 0);
    assert!(
        stats.flagged.iter().any(|f| f.index == 420),
        "{:?}",
        reasons(&stats)
    );
}

#[test]
fn a_rise_of_the_sky_that_stops_or_falls_before_the_end_of_the_night_is_no_dawn() {
    let (mut poses, place) = real_family("sadr_2026_08_14");
    let base = poses[0].sky_background.unwrap();
    // A bright patch (a passing car light) in the middle of the night, rising
    // to ten times the sky then falling back: a high sky, no dawn.
    for (offset, factor) in [(40, 1.8), (41, 4.0), (42, 10.0), (43, 3.0)] {
        poses[offset].sky_background = Some(base * factor);
    }
    // And the end of the night: the sky is back to normal, nothing rises.
    for pose in &mut poses[107..] {
        pose.sky_background = Some(base);
    }
    let stats = judge("x", Some(180.0), 114, &poses, Some(place));
    let flagged = reasons(&stats);
    assert!(
        flagged
            .iter()
            .all(|(_, r)| !matches!(r, Reason::Dawn { .. })),
        "{flagged:?}"
    );
    for index in [41, 42, 43] {
        assert!(
            flagged
                .iter()
                .any(|(i, r)| *i == index && matches!(r, Reason::Sky { .. })),
            "{index}: {flagged:?}"
        );
    }
}

#[test]
fn a_dawn_that_never_reaches_three_times_the_sky_is_not_told() {
    // The night ends while the sky has only gone up to twice the median: the
    // rise is real but not a pose to discard.
    let (mut poses, place) = real_family("sadr_2026_08_14");
    let base = poses[0].sky_background.unwrap();
    for (i, pose) in poses[107..].iter_mut().enumerate() {
        pose.sky_background = Some(base * (1.6 + i as f64 * 0.05));
    }
    let stats = judge("x", Some(180.0), 114, &poses, Some(place));
    assert!(
        stats
            .flagged
            .iter()
            .all(|f| !matches!(f.reason, Reason::Dawn { .. })),
        "{:?}",
        reasons(&stats)
    );
}

#[test]
fn a_list_names_poses_by_file_name_or_path_and_says_the_unknown_ones() {
    let poses: Vec<PathBuf> = [
        "/night/M31/lights/Light_1.fits",
        "/night/M31/lights/Light_2.fits",
        "/night/M31/lights/Light_3.fits",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();
    let list = "# clouds\nLight_2.fits\n\n  \"M31/lights/Light_3.fits\"  \nLight_2.fits\nlight_1.fits\nLight_99.fits\n";
    let (named, unknown) = poses_named(list, &poses);
    assert_eq!(named.len(), 2);
    assert!(named.contains(&poses[1]) && named.contains(&poses[2]));
    assert_eq!(
        unknown,
        vec!["light_1.fits", "Light_99.fits"],
        "case counts"
    );
    assert_eq!(poses_named("", &poses), (HashSet::new(), Vec::new()));
}

/* ------------------------- Camera RAW files ------------------- */

use crate::raw_header::fixture::{Shot, write_raw};

fn pose_of(path: &Path) -> PoseMeasure {
    // What the script answers for a RAW pose: no date (see `raw_clock`).
    RawPose {
        path: path.to_path_buf(),
        status: "measured".to_string(),
        date_obs: None,
        fwhm_pixels: Some(3.0),
        eccentricity: Some(0.2),
        star_count: Some(80),
        sky_background: Some(500.0),
        star_widths: Vec::new(),
        star_eccentricities: Vec::new(),
    }
    .into_measure(0)
}

#[test]
fn the_poses_of_a_raw_night_are_families_like_the_others() {
    let dir = TempDir::new("pose-stats-raw-families");
    let night = night(dir.path(), 0, 0);
    for i in 0..4 {
        write_raw(
            &night.join(format!("M45/lights/DSC_{i:04}.NEF")),
            &Shot::new("2026:10:10 22:00:00", 30, 800),
        );
    }
    let families = pose_families(&night, "M45", None);
    assert_eq!(families.len(), 1);
    assert_eq!(families[0].label, "nofilter_30s_g800_NIKON_D3300");
    assert_eq!(families[0].frames.len(), 4);
    assert_eq!(families[0].exposure_seconds, Some(30.0));
}

#[test]
fn a_raw_pose_takes_its_date_from_the_file_not_from_the_converted_copy() {
    let dir = TempDir::new("pose-stats-raw-date");
    let path = dir.path().join("DSC_0001.NEF");
    let mut shot = Shot::new("2026:10:10 23:30:00", 30, 800);
    shot.offset = Some("+09:00");
    write_raw(&path, &shot);
    let mut pose = pose_of(&path);
    assert_eq!(pose.date_obs, None);
    raw_clock(&mut pose, None);
    // 23:30 at UTC+9 is 14:30 UTC.
    assert_eq!(pose.date_obs.as_deref(), Some("2026-10-10T14:30:00Z"));
    assert_eq!(
        pose.unix,
        parse_iso8601_utc_to_unix_seconds("2026-10-10T14:30:00Z")
    );
}

#[test]
fn a_clock_without_a_zone_is_brought_to_utc_by_the_place_of_the_night() {
    let dir = TempDir::new("pose-stats-raw-wall");
    let path = dir.path().join("DSC_0001.NEF");
    write_raw(&path, &Shot::new("2026:10:10 23:30:00", 30, 800));
    let place = Some(Place {
        latitude: 35.7,
        longitude: 135.0,
    });
    let mut pose = pose_of(&path);
    raw_clock(&mut pose, place);
    // Mean solar time at 135° E is UTC + 9 h.
    assert_eq!(
        pose.unix,
        parse_iso8601_utc_to_unix_seconds("2026-10-10T14:30:00Z")
    );
    // Without a place: as the camera wrote it.
    let mut pose = pose_of(&path);
    raw_clock(&mut pose, None);
    assert_eq!(
        pose.unix,
        parse_iso8601_utc_to_unix_seconds("2026-10-10T23:30:00Z")
    );
}

#[test]
fn a_fits_pose_keeps_the_date_the_script_gave() {
    let dir = TempDir::new("pose-stats-fits-date");
    let path = dir.path().join("a.fit");
    fs::write(&path, b"x").unwrap();
    let mut pose = raw(&path, 0.0, Some(3.0), Some(0.2), 80, 500.0).into_measure(0);
    let before = pose.clone();
    raw_clock(
        &mut pose,
        Some(Place {
            latitude: 0.0,
            longitude: 135.0,
        }),
    );
    assert_eq!(pose, before);
}

#[test]
fn the_measurer_passes_the_siril_it_was_given() {
    let measurer =
        PythonMeasurer::for_night(Path::new("/n"), "python".into(), "pose_stats.py".into());
    assert_eq!(measurer.siril, None);
    let measurer = measurer.with_siril(Some(PathBuf::from("/opt/siril/bin/siril-cli")));
    assert_eq!(
        measurer.siril.as_deref(),
        Some(Path::new("/opt/siril/bin/siril-cli"))
    );
}

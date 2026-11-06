//! A temporary working folder with the person's files (some of them links),
//! the files the pipelines produced, and what "Clean" may and may not do.

use super::*;
use crate::test_support::TempDir;
use crate::upload_queue::{EntryTarget, QueueState, ResultFile, ResultKind, UploadQueue};
use std::os::unix::fs::symlink;

fn write(path: &Path, bytes: usize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![b'x'; bytes]).unwrap();
}

struct World {
    _temp: TempDir,
    root: PathBuf,
    data: PathBuf,
    /// A raw frame outside the working folder that a link points to.
    outside_frame: PathBuf,
    night_one: PathBuf,
    night_two: PathBuf,
    plan: PathBuf,
}

/// Two nights (M31 with masters, results, a scratch holding a link to a
/// frame; M42 with results only), a plan with renders, and the person's own
/// files everywhere: copied lights and logs, a final image, a linked frame, a
/// linked folder, an unknown file in `stacked/`.
fn world() -> World {
    let temp = TempDir::new("processing-files");
    let root = temp.path().join("Astro Sync");
    let data = temp.path().join("data");
    fs::create_dir_all(&data).unwrap();
    let outside = temp.path().join("elsewhere");
    let outside_frame = outside.join("Light_9999.fits");
    write(&outside_frame, 5_000);
    write(&outside.join("folder/inside.fits"), 7_000);

    let night_one = root.join("2026-10-07 · Col · aaaaaa");
    let night_two = root.join("2026-10-08 · Col · bbbbbb");
    for (night, target) in [(&night_one, "M31"), (&night_two, "M42")] {
        write(&night.join(NIGHT_IDENTITY_FILE), 10);
        write(&night.join(target).join(TARGET_IDENTITY_FILE), 10);
        // The person's.
        write(&night.join(target).join("lights/Light_0001.fits"), 1_000);
        write(&night.join(target).join("final/final.tif"), 900);
        write(&night.join("calibration/darks/master_dark.fit"), 800);
        write(&night.join("logs/PHD2_log.txt"), 100);
    }
    // Links the person made: a frame and a whole folder.
    symlink(&outside_frame, night_one.join("M31/lights/Light_LINK.fits")).unwrap();
    symlink(outside.join("folder"), night_one.join("M31/lights/linked")).unwrap();
    // The person's file in `stacked/` the stacking did not write.
    write(&night_one.join("M31/stacked/my notes.txt"), 33);
    // Produced, night one: a master, a render with its picture and record.
    write(
        &night_one.join("M31/stacked/IRCUT_30s_integration.fit"),
        4_000,
    );
    write(&night_one.join("M31/stacked/M31_natural.tif"), 3_000);
    write(&night_one.join("M31/stacked/M31_natural.png"), 500);
    write(&night_one.join("M31/stacked/M31_stacking.json"), 20);
    // A link in `stacked/` to the person's frame: never listed, never deleted.
    symlink(
        &outside_frame,
        night_one.join("M31/stacked/linked_render.tif"),
    )
    .unwrap();
    // The progression images: produced, not cleanable.
    write(&night_one.join("M31/progression/milestone_01.jpg"), 60);
    // The scratch of a stacking left by a hard stop, holding a link to a light.
    write(
        &night_one.join(".astro-sync-work/stacking/job/frame_00001.fits"),
        70,
    );
    symlink(
        night_one.join("M31/lights/Light_0001.fits"),
        night_one.join(".astro-sync-work/stacking/job/link_00002.fits"),
    )
    .unwrap();
    // Night two: results only, no master.
    write(&night_two.join("M42/stacked/M42_natural.tif"), 2_000);
    write(&night_two.join("M42/stacked/M42_natural.png"), 100);
    // A plan.
    let plan = root.join("IC 1805 · cccccc");
    write(&plan.join(PLAN_IDENTITY_FILE), 15);
    write(&plan.join("stacked/IC1805_plan.tif"), 6_000);
    write(&plan.join("stacked/IC1805_plan.json"), 40);
    // A folder of the person's own in the root (no identity): never read.
    write(&root.join("Mes images/stacked/theirs.tif"), 12_345);
    write(&root.join("ASSISTANT-IA.md"), 8);
    // The legacy scratch and the environment, in Astro Sync's data.
    write(&data.join("stacking-work/old/part"), 111);
    write(&data.join("stacking-env/lib/pkg.py"), 222);
    World {
        _temp: temp,
        root,
        data,
        outside_frame,
        night_one,
        night_two,
        plan,
    }
}

fn queue_with_sent(world: &World, files: &[&Path]) -> UploadQueue {
    let queue = UploadQueue::open(&world.data.join("queue.sqlite")).unwrap();
    for (index, file) in files.iter().enumerate() {
        let sha = format!("{index:064x}");
        queue
            .enqueue_results(&[ResultFile {
                path: file.to_path_buf(),
                sha256: sha.clone(),
                size: 1,
                kind: ResultKind::Stacked,
                target: EntryTarget::default(),
            }])
            .unwrap();
        queue.mark_completed(&sha, "capture").unwrap();
    }
    queue
}

fn category(scan: &ProcessingScan, kind: CategoryKind) -> &CategoryScan {
    scan.categories.iter().find(|c| c.kind == kind).unwrap()
}

#[test]
fn the_scan_measures_the_produced_files_apart_from_the_persons_own() {
    let world = world();
    let sent = sent_files(&queue_with_sent(
        &world,
        &[&world.night_one.join("M31/stacked/M31_natural.tif")],
    ));
    let scan = scan(&world.root, &world.data, &sent);
    assert!(scan.reachable);
    assert_eq!(scan.python_env_bytes, 222);

    let masters = category(&scan, CategoryKind::Masters);
    assert_eq!((masters.bytes, masters.count), (4_000, 1));

    // Night one is sent (its only TIFF is a completed row), night two is not.
    let nights = category(&scan, CategoryKind::NightResults);
    assert_eq!(nights.bytes, 3_000 + 500 + 20 + 2_000 + 100);
    assert_eq!(nights.sent_bytes, 3_000 + 500 + 20);
    assert_eq!(nights.unsent_bytes, 2_100);
    assert_eq!(
        (nights.count, nights.sent_count, nights.unsent_count),
        (2, 1, 1)
    );

    // The plan has no row: not sent.
    let plans = category(&scan, CategoryKind::PlanRenders);
    assert_eq!(
        (plans.bytes, plans.sent_bytes, plans.unsent_bytes),
        (6_040, 0, 6_040)
    );
    assert_eq!(
        (plans.count, plans.sent_count, plans.unsent_count),
        (1, 0, 1)
    );

    // The scratch of night one (a link weighs nothing) and the legacy one.
    let temp = category(&scan, CategoryKind::Temp);
    assert_eq!((temp.bytes, temp.count), (70 + 111, 2));

    // The person's (ASSISTANT-IA.md is Astro Sync's): lights (the links weigh nothing), finals, a calibration
    // master of their own, logs, `my notes.txt`, the folder without identity,
    // twice for the two nights.
    let theirs = 2 * (1_000 + 900 + 800 + 100) + 33 + 12_345;
    // ... and the link to a TIFF in `stacked/`, which is a link: nothing.
    assert_eq!(scan.user_bytes, theirs);
    assert_eq!(
        scan.folder_bytes,
        scan.user_bytes
            + 4_000
            + 5_620
            + 6_040
            + 70
            + 60 // the progression images
            + 10 * 4 // the four identity files
            + 15 // the plan's identity
            + 8 // ASSISTANT-IA.md
    );
}

#[test]
fn a_folder_that_is_not_there_is_not_reachable_and_says_zero_everywhere() {
    let world = world();
    let scan = scan(&world.root.join("absent"), &world.data, &HashSet::new());
    assert!(!scan.reachable);
    assert_eq!((scan.folder_bytes, scan.user_bytes), (0, 0));
    assert_eq!(scan.python_env_bytes, 222);
    assert_eq!(category(&scan, CategoryKind::NightResults).bytes, 0);
    assert!(matches!(
        clean(
            &world.root.join("absent"),
            &world.data,
            &HashSet::new(),
            &[CleanKind::Masters]
        ),
        Err(CleanError::Unreachable(_))
    ));
}

#[test]
fn cleaning_deletes_only_what_was_asked_and_never_the_persons_files_or_a_links_target() {
    let world = world();
    let tif = world.night_one.join("M31/stacked/M31_natural.tif");
    let sent = sent_files(&queue_with_sent(&world, &[&tif]));
    let before = scan(&world.root, &world.data, &sent);

    let outcome = clean(
        &world.root,
        &world.data,
        &sent,
        &[CleanKind::Masters, CleanKind::SentResults, CleanKind::Temp],
    )
    .unwrap();
    assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
    assert_eq!(outcome.freed_bytes, 4_000 + 3_520 + 70 + 111);

    // Gone: the master, the sent render, its picture and record, the scratches.
    assert!(
        !world
            .night_one
            .join("M31/stacked/IRCUT_30s_integration.fit")
            .exists()
    );
    assert!(!tif.exists());
    assert!(!world.night_one.join("M31/stacked/M31_natural.png").exists());
    assert!(!world.night_one.join(".astro-sync-work").exists());
    assert!(!world.data.join("stacking-work").exists());
    assert!(outcome.removed.contains(&tif));

    // Kept: what was not asked (not sent: night two and the plan)...
    assert!(
        world
            .night_two
            .join("M42/stacked/M42_natural.tif")
            .is_file()
    );
    assert!(world.plan.join("stacked/IC1805_plan.tif").is_file());
    // ... the environment, the progression, the identities...
    assert!(world.data.join("stacking-env/lib/pkg.py").is_file());
    assert!(
        world
            .night_one
            .join("M31/progression/milestone_01.jpg")
            .is_file()
    );
    assert!(world.night_one.join(NIGHT_IDENTITY_FILE).is_file());
    // ... and everything the person made theirs.
    for kept in [
        "M31/lights/Light_0001.fits",
        "M31/final/final.tif",
        "calibration/darks/master_dark.fit",
        "logs/PHD2_log.txt",
        "M31/stacked/my notes.txt",
    ] {
        assert!(world.night_one.join(kept).is_file(), "{kept}");
    }
    assert!(world.root.join("Mes images/stacked/theirs.tif").is_file());
    // The links are still links, and their targets are untouched.
    for link in [
        "M31/lights/Light_LINK.fits",
        "M31/lights/linked",
        "M31/stacked/linked_render.tif",
    ] {
        let meta = fs::symlink_metadata(world.night_one.join(link)).unwrap();
        assert!(meta.file_type().is_symlink(), "{link}");
    }
    assert_eq!(fs::metadata(&world.outside_frame).unwrap().len(), 5_000);
    assert_eq!(
        fs::metadata(
            world
                .outside_frame
                .parent()
                .unwrap()
                .join("folder/inside.fits")
        )
        .unwrap()
        .len(),
        7_000
    );

    // The scan after says what is left: the person's own files are the same.
    let after = scan(&world.root, &world.data, &sent);
    assert_eq!(after.user_bytes, before.user_bytes);
    assert_eq!(category(&after, CategoryKind::Masters).bytes, 0);
    assert_eq!(category(&after, CategoryKind::Temp).bytes, 0);
    assert_eq!(category(&after, CategoryKind::NightResults).sent_bytes, 0);
    assert_eq!(
        category(&after, CategoryKind::NightResults).unsent_bytes,
        2_100
    );
}

#[test]
fn the_results_not_sent_go_only_when_they_are_asked_for() {
    let world = world();
    let sent = HashSet::new();
    let outcome = clean(&world.root, &world.data, &sent, &[CleanKind::SentResults]).unwrap();
    assert_eq!(outcome.freed_bytes, 0, "nothing is sent, so nothing goes");
    assert!(
        world
            .night_two
            .join("M42/stacked/M42_natural.tif")
            .is_file()
    );

    let outcome = clean(&world.root, &world.data, &sent, &[CleanKind::UnsentResults]).unwrap();
    assert_eq!(outcome.freed_bytes, 3_520 + 2_100 + 6_040);
    assert!(!world.night_two.join("M42/stacked/M42_natural.tif").exists());
    assert!(!world.plan.join("stacked/IC1805_plan.tif").exists());
    // The master is not a result: it stays when only results are asked.
    assert!(
        world
            .night_one
            .join("M31/stacked/IRCUT_30s_integration.fit")
            .is_file()
    );
    assert!(world.night_one.join("M31/stacked/my notes.txt").is_file());
}

#[test]
fn a_night_folder_that_is_a_link_is_neither_read_nor_cleaned() {
    let world = world();
    let elsewhere = world.root.parent().unwrap().join("moved-night");
    fs::rename(&world.night_two, &elsewhere).unwrap();
    symlink(&elsewhere, &world.night_two).unwrap();
    let scan = scan(&world.root, &world.data, &HashSet::new());
    assert_eq!(category(&scan, CategoryKind::NightResults).count, 1);
    clean(
        &world.root,
        &world.data,
        &HashSet::new(),
        &[
            CleanKind::UnsentResults,
            CleanKind::Masters,
            CleanKind::Temp,
        ],
    )
    .unwrap();
    assert!(elsewhere.join("M42/stacked/M42_natural.tif").is_file());
}

#[test]
fn a_stacked_folder_that_is_a_link_leaves_what_it_points_to_alone() {
    let world = world();
    let outside = world.root.parent().unwrap().join("their-renders");
    write(&outside.join("render.tif"), 4_444);
    write(&outside.join("x_integration.fit"), 5_555);
    fs::remove_dir_all(world.night_two.join("M42/stacked")).unwrap();
    symlink(&outside, world.night_two.join("M42/stacked")).unwrap();
    let all = [
        CleanKind::Masters,
        CleanKind::SentResults,
        CleanKind::UnsentResults,
        CleanKind::Temp,
    ];
    clean(&world.root, &world.data, &HashSet::new(), &all).unwrap();
    assert_eq!(
        fs::metadata(outside.join("render.tif")).unwrap().len(),
        4_444
    );
    assert_eq!(
        fs::metadata(outside.join("x_integration.fit"))
            .unwrap()
            .len(),
        5_555
    );
}

#[test]
fn a_row_not_finished_of_a_deleted_file_is_forgotten_a_finished_one_stays() {
    let world = world();
    let sent_tif = world.night_one.join("M31/stacked/M31_natural.tif");
    let queue = queue_with_sent(&world, &[&sent_tif]);
    let waiting = world.night_two.join("M42/stacked/M42_natural.tif");
    queue
        .enqueue_results(&[ResultFile {
            path: waiting.clone(),
            sha256: "f".repeat(64),
            size: 1,
            kind: ResultKind::Stacked,
            target: EntryTarget::default(),
        }])
        .unwrap();
    let dropped = queue
        .forget_unfinished_files(&[sent_tif.clone(), waiting.clone()])
        .unwrap();
    assert_eq!(dropped, 1);
    let rows = queue.list().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].source_path, sent_tif);
    assert_eq!(rows[0].state, QueueState::Completed);
}

#[test]
fn the_python_environment_is_removed_apart_links_unlinked_never_followed() {
    let world = world();
    let interpreter = world.outside_frame.clone();
    symlink(&interpreter, world.data.join("stacking-env/python3")).unwrap();
    assert_eq!(remove_python_environment(&world.data).unwrap(), 222);
    assert!(!world.data.join("stacking-env").exists());
    assert_eq!(fs::metadata(&interpreter).unwrap().len(), 5_000);
    assert_eq!(remove_python_environment(&world.data).unwrap(), 0);
}

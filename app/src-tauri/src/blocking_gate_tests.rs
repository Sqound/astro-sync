//! The window may ask the same heavy thing hundreds of times before the
//! first answer (an effect of the page that ran at every change of its
//! state did): the threads of the process stay bounded, every asking is
//! answered, and work that panics ends as an error of its command.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::commands::{run_blocking, run_gated, BLOCKING_AT_ONCE};
use crate::wake_tests::{fast, harness, serialized, unique_dir};

/// The threads of this process, as the system counts them.
fn threads_now() -> usize {
    let out = std::process::Command::new("ps")
        .args(["-M", "-p", &std::process::id().to_string()])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .count()
        .saturating_sub(1)
}

/// `count` askings of work that lasts `each`, through `ask`; the most that
/// ran at once and the most threads the process had meanwhile.
fn flood<F>(count: usize, each: Duration, ask: F) -> (usize, usize)
where
    F: Fn(
        Box<dyn FnOnce() -> Result<(), crate::error::CommandError> + Send>,
    ) -> tauri::async_runtime::JoinHandle<bool>,
{
    let running = Arc::new(AtomicUsize::new(0));
    let most = Arc::new(AtomicUsize::new(0));
    let handles: Vec<_> = (0..count)
        .map(|_| {
            let running = Arc::clone(&running);
            let most = Arc::clone(&most);
            ask(Box::new(move || {
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                most.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(each);
                running.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            }))
        })
        .collect();
    let mut threads = 0;
    while handles.iter().any(|handle| !handle.inner().is_finished()) {
        threads = threads.max(threads_now());
        std::thread::sleep(Duration::from_millis(20));
    }
    for handle in handles {
        assert!(tauri::async_runtime::block_on(handle).unwrap());
    }
    (most.load(Ordering::SeqCst), threads)
}

#[cfg(unix)]
#[test]
fn five_hundred_askings_never_become_five_hundred_threads() {
    let _serial = serialized();
    let before = threads_now();
    let (at_once, threads) = flood(500, Duration::from_millis(40), |work| {
        tauri::async_runtime::spawn(async move { run_blocking(work).await.is_ok() })
    });
    assert!(at_once <= BLOCKING_AT_ONCE, "{at_once} ran at once");
    assert!(
        threads <= before + BLOCKING_AT_ONCE + 24,
        "{threads} threads during the flood, {before} before"
    );

    // A command with a gate of its own (the summary of « Ranger »): two.
    static NARROW: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
    let (at_once, _) = flood(60, Duration::from_millis(10), |work| {
        tauri::async_runtime::spawn(async move { run_gated(&NARROW, work).await.is_ok() })
    });
    assert!(at_once <= 2, "{at_once} ran at once");
}

#[test]
fn work_that_panics_ends_its_command_with_an_error_and_frees_its_place() {
    let _serial = serialized();
    static ONE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
    let stopped = tauri::async_runtime::block_on(run_gated::<()>(&ONE, || {
        panic!("index out of bounds in /Us\u{65}rs/someone/Astro/night one/l.fit")
    }))
    .unwrap_err();
    assert_eq!(stopped.code, "work_stopped");
    assert!(!stopped.message.contains("someone"), "{}", stopped.message);
    // The place is free again: the next asking runs.
    let next = tauri::async_runtime::block_on(run_gated(&ONE, || Ok(7)));
    assert_eq!(next.unwrap(), 7);
}

/// The window asking the summary of « Ranger les fichiers » two hundred
/// times at once, as the page did at every change of its state: every asking
/// is answered with the same plan, and the process keeps its threads.
#[cfg(unix)]
#[test]
fn the_summary_of_tidying_asked_two_hundred_times_at_once_answers_each_one() {
    let _serial = serialized();
    let harness = harness(fast());
    let _ = std::fs::remove_file(harness.config_dir.join("settings.json"));
    let root = unique_dir("tidy-flood");
    let lights = root.join("fits").join("lights");
    std::fs::create_dir_all(&lights).unwrap();
    let frames = 300;
    for i in 0..frames {
        let seconds = i * 20;
        let date = format!(
            "'2026-08-17T{:02}:{:02}:{:02}'",
            20 + seconds / 3600,
            (seconds / 60) % 60,
            seconds % 60
        );
        let mut header = Vec::new();
        for (keyword, value) in [
            ("IMAGETYP", "'Light'"),
            ("OBJECT", "'M 45'"),
            ("DATE-OBS", date.as_str()),
            ("EXPTIME", "10."),
            ("SITELAT", "43.88"),
            ("SITELONG", "7.43"),
        ] {
            let mut card = format!("{keyword:<8}= {value}").into_bytes();
            card.resize(80, b' ');
            header.extend_from_slice(&card);
        }
        let mut end = b"END".to_vec();
        end.resize(80, b' ');
        header.extend_from_slice(&end);
        header.resize(header.len().div_ceil(2880) * 2880, b' ');
        std::fs::write(lights.join(format!("l{i:03}.fit")), header).unwrap();
    }
    let proposal = harness
        .invoke(
            "sessions_detect",
            serde_json::json!({
                "request": { "fits": [root.join("fits")] },
                "user": { "account": { "advancedPlanning": true } },
                "catalogue": []
            }),
        )
        .answer(Duration::from_secs(60))
        .expect("the detection answers");
    let args = serde_json::json!({
        "proposal": proposal,
        "destination": root.join("organised"),
        "mode": "symlink",
    });
    let before = threads_now();
    let pending: Vec<_> = (0..200)
        .map(|_| harness.invoke("sessions_organise_plan", args.clone()))
        .collect();
    let during = threads_now();
    for asked in pending {
        let plan = asked
            .answer(Duration::from_secs(120))
            .expect("each asking is answered");
        assert_eq!(plan["files"], frames);
    }
    assert!(
        during <= before + 24,
        "{during} threads while 200 summaries were asked, {before} before"
    );
}

#[test]
fn a_stop_is_written_to_the_journal_without_a_path_and_the_journal_stays_short() {
    let dir = unique_dir("stops-journal");
    std::fs::create_dir_all(&dir).unwrap();
    let journal = dir.join("stops.log");
    crate::error_reports::note_stop(
        &journal,
        "1.1.4",
        "failed to spawn thread while reading /Us\u{65}rs/someone/Astro/M45/l1.fit",
        Some("at mod.rs:12:5 (tokio)"),
    );
    let text = std::fs::read_to_string(&journal).unwrap();
    assert!(
        text.contains("astro-sync 1.1.4 panic: failed to spawn thread"),
        "{text}"
    );
    assert!(
        text.trim_end().ends_with("at mod.rs:12:5 (tokio)"),
        "{text}"
    );
    assert!(!text.contains("someone") && !text.contains("M45"), "{text}");
    for n in 0..80 {
        crate::error_reports::note_stop(&journal, "1.1.4", &format!("stop {n}"), None);
    }
    let text = std::fs::read_to_string(&journal).unwrap();
    assert_eq!(text.lines().count(), 50);
    assert!(
        text.trim_end().ends_with("panic: stop 79 at unknown"),
        "{text}"
    );
}

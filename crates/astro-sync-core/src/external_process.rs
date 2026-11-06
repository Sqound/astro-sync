//! Running an external tool: ASTAP, astrometry.net's
//! `solve-field`, Siril, the stacking script. They are installed by the
//! person (never bundled) and may run for minutes, so
//! every run here
//!
//! - reads the tool's standard output and error on threads of their own (a
//!   tool that writes a lot never blocks on a full pipe);
//! - stops it at a time limit (`timed_out`) or when the person asks
//!   ([`CancelFlag`], `cancelled`), killing the process;
//! - can hand each line of standard output to the caller as it comes
//!   ([`run_streaming`]: the stacking script's JSON events).
//!
//! Tests drive it with small shell scripts standing for the tools.

use std::io::{BufRead, BufReader, Read};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::local_work::CancelFlag;

/// How a run ended.
#[derive(Debug)]
pub struct ProcessEnd {
    /// The exit status, `None` when the process was killed.
    pub status: Option<ExitStatus>,
    pub timed_out: bool,
    pub cancelled: bool,
    pub stdout: String,
    pub stderr: String,
}

impl ProcessEnd {
    /// The exit code of a process that ended by itself.
    #[must_use]
    pub fn code(&self) -> Option<i32> {
        self.status.and_then(|status| status.code())
    }

    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.status.is_some_and(|status| status.success())
    }
}

/// The name of the configuration file Astro Sync gives Siril in a work folder.
pub const SIRIL_CONFIG_NAME: &str = "siril-config.ini";

/// A configuration of its own for the runs of Siril working in `work_dir`:
/// `siril-cli` rewrites the person's `~/.config/siril/config.<version>.ini`
/// at every exit unless it is given `-i <file>`, and Astro Sync writes
/// nothing of the person's. Siril refuses a file that does not exist and
/// fills an empty one with its defaults, so the file is created empty when
/// missing.
pub fn siril_config_in(work_dir: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    std::fs::create_dir_all(work_dir)?;
    let path = work_dir.join(SIRIL_CONFIG_NAME);
    if !path.exists() {
        std::fs::write(&path, "")?;
    }
    Ok(path)
}

/// How often the run looks at its clock and its stop switch.
const POLL: Duration = Duration::from_millis(25);

/// How long the output of an ended process is still awaited.
const GRACE: Duration = Duration::from_secs(2);

/// Runs `command` to its end, at most `timeout`, stopped by `cancel`.
/// An error only when the process cannot be started at all.
pub fn run(
    command: Command,
    timeout: Duration,
    cancel: Option<&CancelFlag>,
) -> std::io::Result<ProcessEnd> {
    run_streaming(command, timeout, cancel, &mut |_| {})
}

/// [`run`], handing each line of standard output to `on_line` as it comes
/// (the line is also kept in [`ProcessEnd::stdout`]).
pub fn run_streaming(
    command: Command,
    timeout: Duration,
    cancel: Option<&CancelFlag>,
    on_line: &mut dyn FnMut(&str),
) -> std::io::Result<ProcessEnd> {
    run_inner(command, timeout, cancel, None, on_line, None)
}

/// [`run_streaming`] for a tool that cleans up after itself when asked to
/// stop (the stacking script stops Siril and says `cancelled`, code 130):
/// on a stop, it is first sent `SIGTERM` and given `grace` to end by itself
/// -- its last lines are still read -- and killed only after. Elsewhere than
/// on Unix it is killed at once.
pub fn run_streaming_gently(
    command: Command,
    timeout: Duration,
    cancel: Option<&CancelFlag>,
    grace: Duration,
    on_line: &mut dyn FnMut(&str),
) -> std::io::Result<ProcessEnd> {
    run_inner(command, timeout, cancel, Some(grace), on_line, None)
}

/// [`run_streaming`] (and [`run_streaming_gently`] when `grace` is given)
/// that also calls `on_poll` every `every` while the process runs, in the
/// same thread as `on_line` -- so a caller can read a journal the tool
/// writes, or count the files it has made, and report as it goes without a
/// thread of its own.
pub fn run_polled(
    command: Command,
    timeout: Duration,
    cancel: Option<&CancelFlag>,
    grace: Option<Duration>,
    every: Duration,
    on_line: &mut dyn FnMut(&str),
    on_poll: &mut dyn FnMut(),
) -> std::io::Result<ProcessEnd> {
    run_inner(
        command,
        timeout,
        cancel,
        grace,
        on_line,
        Some((every, on_poll)),
    )
}

/// Windows: a program started by an application with no console of its own
/// (the release build of the app is a window-only program) would open a
/// console window for every run: one flashes for each version query, each
/// script, each Siril. `CREATE_NO_WINDOW` keeps it hidden. Nothing on the
/// other systems.
pub fn hide_console(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW.
        command.creation_flags(0x0800_0000);
    }
    command
}

/// Asks the process `pid` to stop (`SIGTERM`), through the system's own
/// `kill` (no library for one signal). `false` when it could not be sent.
fn terminate(pid: u32) -> bool {
    #[cfg(unix)]
    {
        Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

fn run_inner(
    mut command: Command,
    timeout: Duration,
    cancel: Option<&CancelFlag>,
    stop_grace: Option<Duration>,
    on_line: &mut dyn FnMut(&str),
    mut poll: Option<(Duration, &mut dyn FnMut())>,
) -> std::io::Result<ProcessEnd> {
    hide_console(&mut command);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().expect("stdout is piped");
    let mut stderr = child.stderr.take().expect("stderr is piped");

    let (lines_tx, lines_rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if lines_tx.send(line).is_err() {
                break;
            }
        }
    });
    let (err_tx, err_rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        let mut text = Vec::new();
        let _ = stderr.read_to_end(&mut text);
        let _ = err_tx.send(String::from_utf8_lossy(&text).into_owned());
    });

    let started = Instant::now();
    let mut polled_at = started;
    let mut collected = String::new();
    let mut keep = |line: String, collected: &mut String| {
        on_line(&line);
        collected.push_str(&line);
        collected.push('\n');
    };
    let (status, timed_out, cancelled) = loop {
        while let Ok(line) = lines_rx.try_recv() {
            keep(line, &mut collected);
        }
        if let Some((every, on_poll)) = poll.as_mut()
            && polled_at.elapsed() >= *every
        {
            polled_at = Instant::now();
            on_poll();
        }
        if let Some(status) = child.try_wait()? {
            break (Some(status), false, false);
        }
        let cancelled = cancel.is_some_and(CancelFlag::is_cancelled);
        let timed_out = started.elapsed() >= timeout;
        if cancelled || timed_out {
            if let Some(grace) = stop_grace.filter(|_| cancelled)
                && terminate(child.id())
            {
                let until = Instant::now() + grace;
                let mut ended = None;
                while Instant::now() < until {
                    while let Ok(line) = lines_rx.try_recv() {
                        keep(line, &mut collected);
                    }
                    if let Some(status) = child.try_wait()? {
                        ended = Some(status);
                        break;
                    }
                    thread::sleep(POLL);
                }
                if ended.is_some() {
                    break (ended, false, true);
                }
            }
            let _ = child.kill();
            let _ = child.wait();
            break (None, timed_out && !cancelled, cancelled);
        }
        thread::sleep(POLL);
    };
    // The rest of the output. A process the tool started itself may keep the
    // pipes open after it ended: the readers are then left behind after a
    // short grace instead of holding the caller.
    let grace = Instant::now() + GRACE;
    loop {
        let left = grace.saturating_duration_since(Instant::now());
        match lines_rx.recv_timeout(left) {
            Ok(line) => keep(line, &mut collected),
            Err(_) => break,
        }
    }
    let stderr = err_rx
        .recv_timeout(grace.saturating_duration_since(Instant::now()))
        .unwrap_or_default();
    Ok(ProcessEnd {
        status,
        timed_out,
        cancelled,
        stdout: collected,
        stderr,
    })
}

/// Test helper: an executable shell script standing for a tool.
#[cfg(all(test, unix))]
pub(crate) fn fake_tool(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn siril_gets_a_configuration_of_its_own_in_the_work_folder() {
        let dir = TempDir::new("siril-ini");
        let work = dir.path().join("work");
        let ini = siril_config_in(&work).unwrap();
        assert_eq!(ini, work.join(SIRIL_CONFIG_NAME));
        assert_eq!(std::fs::read_to_string(&ini).unwrap(), "");
        // What Siril filled in is kept by the next call.
        std::fs::write(&ini, "[core]\nwd=/x\n").unwrap();
        assert_eq!(siril_config_in(&work).unwrap(), ini);
        assert_eq!(std::fs::read_to_string(&ini).unwrap(), "[core]\nwd=/x\n");
    }

    #[test]
    fn a_polled_run_calls_its_poll_while_the_tool_works() {
        let dir = TempDir::new("process-polled");
        let tool = fake_tool(dir.path(), "tool", "echo one\nsleep 0.5\necho two");
        let mut lines = Vec::new();
        let mut polls = 0;
        let end = run_polled(
            Command::new(tool),
            Duration::from_secs(10),
            None,
            None,
            Duration::from_millis(100),
            &mut |line| lines.push(line.to_string()),
            &mut || polls += 1,
        )
        .unwrap();
        assert!(end.succeeded());
        assert_eq!(lines, ["one", "two"]);
        assert!(polls >= 2, "polled {polls} times in half a second");
    }

    #[test]
    fn a_tool_that_answers_gives_its_output_and_code() {
        let dir = TempDir::new("process-answers");
        let tool = fake_tool(
            dir.path(),
            "tool",
            "echo one\necho two\necho oops >&2\nexit 3",
        );
        let mut lines = Vec::new();
        let end = run_streaming(
            Command::new(&tool),
            Duration::from_secs(10),
            None,
            &mut |line| lines.push(line.to_string()),
        )
        .unwrap();
        assert_eq!(end.code(), Some(3));
        assert_eq!(lines, vec!["one", "two"]);
        assert_eq!(end.stdout, "one\ntwo\n");
        assert_eq!(end.stderr.trim(), "oops");
        assert!(!end.timed_out && !end.cancelled);
    }

    #[test]
    fn a_tool_past_its_time_is_stopped() {
        let dir = TempDir::new("process-timeout");
        let tool = fake_tool(dir.path(), "tool", "sleep 30");
        let started = Instant::now();
        let end = run(Command::new(&tool), Duration::from_millis(200), None).unwrap();
        assert!(end.timed_out);
        assert!(end.status.is_none());
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_tool_is_stopped_on_request() {
        let dir = TempDir::new("process-cancel");
        let tool = fake_tool(dir.path(), "tool", "sleep 30");
        let cancel = CancelFlag::new();
        cancel.cancel();
        let end = run(Command::new(&tool), Duration::from_secs(30), Some(&cancel)).unwrap();
        assert!(end.cancelled && !end.timed_out);
    }

    #[test]
    fn a_gentle_stop_lets_the_tool_say_it_stopped() {
        let dir = TempDir::new("process-gentle");
        let tool = fake_tool(
            dir.path(),
            "tool",
            "trap 'echo stopped; exit 130' TERM\necho started\nwhile true; do sleep 0.05; done",
        );
        // Stopped once it said it started (its trap is set by then).
        let cancel = CancelFlag::new();
        let stopper = cancel.clone();
        let mut lines = Vec::new();
        let end = run_streaming_gently(
            Command::new(&tool),
            Duration::from_secs(30),
            Some(&cancel),
            Duration::from_secs(10),
            &mut |line| {
                lines.push(line.to_string());
                stopper.cancel();
            },
        )
        .unwrap();
        assert!(end.cancelled && !end.timed_out);
        assert_eq!(end.code(), Some(130));
        assert_eq!(lines, vec!["started", "stopped"]);
    }

    #[test]
    fn a_missing_tool_is_an_error() {
        assert!(
            run(
                Command::new("/nonexistent/astro-sync-tool"),
                Duration::from_secs(1),
                None
            )
            .is_err()
        );
    }
}

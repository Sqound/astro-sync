//! What a running Siril tells while it works: the last percentage and the
//! last useful line of its journal, read live.
//!
//! Siril prints its journal as plain lines (`log: Integration of 12
//! images…`) and its progress as lines that start with `progress:` (`progress:
//! 28.18%`, or `progress: Registration, 62.50%`), most of them preceded by
//! terminal sequences (`ESC [ A ESC [ 2 K`) and a stray carriage return that
//! only make sense on a terminal. [`SirilFollower`] reads such lines one by
//! one and keeps the latest percentage and the latest line worth showing;
//! [`LogTail`] feeds it from a journal file that a tool writes (the stacking's
//! `siril.log`), and the progression feeds it from Siril's standard output.
//! Pure functions and plain structs: the callers poll them from the loop that
//! already waits for the process, so no thread of their own is needed.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// The longest line kept, in characters (an ellipsis closes a longer one).
pub const LINE_LIMIT: usize = 120;

/// How often a running tool is looked at for news.
pub const POLL_EVERY: std::time::Duration = std::time::Duration::from_millis(500);

/// The clock the reports are stamped with: milliseconds since the Unix
/// epoch (the window reads the same clock, on the same computer).
#[must_use]
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// What one line of Siril's output is.
#[derive(Debug, Clone, PartialEq)]
pub enum SirilLine {
    /// Nothing to show: blank, or terminal noise.
    Skip,
    /// A progress line, with its percentage when it carries one.
    Progress(Option<f64>),
    /// A line of the journal worth showing, cleaned and cut.
    Text(String),
}

/// Removes the terminal sequences (`ESC [ … letter`), then keeps what follows
/// the last carriage return (Siril redraws its progress line over itself).
#[must_use]
pub fn clean_line(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&next) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out.rsplit('\r')
        .find(|segment| !segment.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn percent_of(progress: &str) -> Option<f64> {
    let value = progress.trim_end().strip_suffix('%')?;
    let token = value.rsplit([',', ' ', ':']).next()?;
    let percent: f64 = token.trim().parse().ok()?;
    percent.is_finite().then_some(percent.clamp(0.0, 100.0))
}

/// What Siril 1.4 says of the Python environment it prepares in the
/// background at every start, for the scripts of its own editor. A stacking
/// runs commands only and never uses it; where Siril cannot find the module
/// it would install (a Homebrew build: « Failed to install Python module:
/// Failed to open file "pyproject.toml" »), it complains at every start, in
/// the middle of the alignment, about something that changes nothing.
const PYTHON_ENVIRONMENT_NOISE: [&str; 8] = [
    "Preparing python virtual environment",
    "Checking the python module is up-to-date",
    "Python module is up-to-date",
    "Installing / updating python module",
    "Warning: unable to install or update the Siril python module",
    "Failed to prepare virtual environment",
    "Failed to initialize Python virtual environment",
    "Failed to install Python module",
];

/// The lines of Siril that say nothing a person would read.
fn is_noise(line: &str) -> bool {
    let line_without_log = line.strip_prefix("log:").map_or(line, str::trim);
    PYTHON_ENVIRONMENT_NOISE
        .iter()
        .any(|noise| line_without_log.contains(noise))
        || line.starts_with("HDU ")
        || line.starts_with("Block ")
        || line.starts_with("WARNING: version mismatch")
        || line == "closing pipes"
        || line.starts_with("findstar_args")
        || line
            .split_once(": running command")
            .is_some_and(|(stamp, _)| stamp.bytes().all(|b| b.is_ascii_digit()))
}

fn truncated(line: &str) -> String {
    if line.chars().count() <= LINE_LIMIT {
        return line.to_string();
    }
    let mut cut: String = line.chars().take(LINE_LIMIT - 1).collect();
    cut.push('…');
    cut
}

/// Reads one raw line of Siril's output.
#[must_use]
pub fn parse_line(raw: &str) -> SirilLine {
    let line = clean_line(raw);
    if line.is_empty() {
        return SirilLine::Skip;
    }
    if let Some(rest) = line.strip_prefix("progress:") {
        return SirilLine::Progress(percent_of(rest));
    }
    if is_noise(&line) {
        return SirilLine::Skip;
    }
    let text = line.strip_prefix("log:").map_or(line.as_str(), str::trim);
    SirilLine::Text(truncated(text))
}

/// What a tool reports to the window while it works.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SirilReport {
    /// The last percentage Siril gave for what it is doing now.
    pub percent: Option<f64>,
    /// The last useful line of its journal.
    pub line: Option<String>,
    /// When that line was read (milliseconds since the Unix epoch); the
    /// time of the report itself while no line has come yet.
    pub at: u64,
}

/// The latest percentage and the latest useful line of a Siril that runs.
#[derive(Debug, Clone, Default)]
pub struct SirilFollower {
    percent: Option<f64>,
    line: Option<String>,
    line_at: u64,
    dirty: bool,
}

impl SirilFollower {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads one raw line, read at `now` (milliseconds).
    pub fn feed(&mut self, raw: &str, now: u64) {
        match parse_line(raw) {
            SirilLine::Skip => {}
            SirilLine::Progress(percent) => {
                if percent.is_some() && percent != self.percent {
                    self.percent = percent;
                    self.dirty = true;
                }
            }
            SirilLine::Text(text) => {
                self.line = Some(text);
                self.line_at = now;
                // The same text twice is still a line that came: the clock
                // of the last line moves.
                self.dirty = true;
            }
        }
    }

    /// A new command of Siril starts: its percentage starts again.
    pub fn restart_percent(&mut self) {
        if self.percent.take().is_some() {
            self.dirty = true;
        }
    }

    /// What changed since the last call, if anything.
    pub fn take(&mut self, now: u64) -> Option<SirilReport> {
        if !std::mem::take(&mut self.dirty) {
            return None;
        }
        Some(SirilReport {
            percent: self.percent,
            line: self.line.clone(),
            at: if self.line.is_some() {
                self.line_at
            } else {
                now
            },
        })
    }
}

/// A journal file read as it grows: each call to [`LogTail::poll`] hands the
/// lines completed since the last one to a [`SirilFollower`]. The file may
/// not exist yet (the tool has not written its first line), and one that
/// shrinks is read again from its start.
#[derive(Debug)]
pub struct LogTail {
    path: PathBuf,
    offset: u64,
    partial: Vec<u8>,
}

impl LogTail {
    #[must_use]
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            offset: 0,
            partial: Vec::new(),
        }
    }

    pub fn poll(&mut self, follower: &mut SirilFollower, now: u64) {
        let Ok(mut file) = File::open(&self.path) else {
            return;
        };
        let Ok(length) = file.metadata().map(|m| m.len()) else {
            return;
        };
        if length < self.offset {
            self.offset = 0;
            self.partial.clear();
        }
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return;
        }
        let mut fresh = Vec::new();
        if file
            .take(length - self.offset)
            .read_to_end(&mut fresh)
            .is_err()
        {
            return;
        }
        self.offset += fresh.len() as u64;
        self.partial.extend_from_slice(&fresh);
        let Some(end) = self.partial.iter().rposition(|b| *b == b'\n') else {
            return;
        };
        let complete: Vec<u8> = self.partial.drain(..=end).collect();
        for line in String::from_utf8_lossy(&complete).lines() {
            follower.feed(line, now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    use std::io::Write;

    #[test]
    fn terminal_sequences_and_the_stray_character_before_a_carriage_return_go() {
        assert_eq!(
            clean_line("\u{1b}[A\u{1b}[2KT\rprogress: 28.18%"),
            "progress: 28.18%"
        );
        assert_eq!(
            clean_line("log: Integrating 90 images…\r"),
            "log: Integrating 90 images…"
        );
        assert_eq!(clean_line("   "), "");
    }

    #[test]
    fn a_progress_line_gives_its_percentage_with_or_without_a_label() {
        assert_eq!(
            parse_line("\u{1b}[A\u{1b}[2KT\rprogress: 28.18%"),
            SirilLine::Progress(Some(28.18))
        );
        assert_eq!(
            parse_line("progress: Rejection stacking in progress..., 0.00%"),
            SirilLine::Progress(Some(0.0))
        );
        assert_eq!(
            parse_line("progress: FindStar. Processing image 4 (light_00004.fit), 8.13%"),
            SirilLine::Progress(Some(8.13))
        );
        assert_eq!(
            parse_line("progress: , 100.00%"),
            SirilLine::Progress(Some(100.0))
        );
        assert_eq!(parse_line("progress: no number"), SirilLine::Progress(None));
    }

    #[test]
    fn a_journal_line_loses_its_prefix_and_the_noise_is_skipped() {
        assert_eq!(
            parse_line("log: Integration of 12 images on 12 of the sequence:"),
            SirilLine::Text("Integration of 12 images on 12 of the sequence:".to_string())
        );
        assert_eq!(
            parse_line("Reading sequence file `r_light_.seq'."),
            SirilLine::Text("Reading sequence file `r_light_.seq'.".to_string())
        );
        for noise in [
            "",
            "HDU 1: type=0, EXTNAME=",
            "Block 3: channel 0, from 63 to 83 (h = 21)",
            "1791799606: running command stack",
            "closing pipes",
            "WARNING: version mismatch between CFITSIO header (v40603) and linked library (v40700).",
            // Siril's background Python environment, which a stacking never uses.
            "log: Preparing python virtual environment: ~/.local/share/siril/venv.",
            "log: Checking the python module is up-to-date...",
            "log: Warning: unable to install or update the Siril python module.",
            "** (siril-cli:9661): WARNING **: 22:17:23.631: Failed to install Python module: Failed to open file “pyproject.toml”: No such file or directory",
            "log: Failed to prepare virtual environment: Failed to install Python module: Failed to open file “pyproject.toml”: No such file or directory",
        ] {
            assert_eq!(parse_line(noise), SirilLine::Skip, "{noise}");
        }
    }

    #[test]
    fn a_long_line_is_cut_at_the_limit_with_an_ellipsis() {
        let SirilLine::Text(text) = parse_line(&format!("log: {}", "é".repeat(300))) else {
            panic!("a text line");
        };
        assert_eq!(text.chars().count(), LINE_LIMIT);
        assert!(text.ends_with('…'));
    }

    #[test]
    fn the_follower_keeps_the_last_percentage_and_the_last_line_with_its_time() {
        let mut follower = SirilFollower::new();
        assert!(follower.take(1).is_none());
        follower.feed("progress: 10.00%", 100);
        // Percentage only: the report is stamped with the clock of the call.
        let first = follower.take(150).unwrap();
        assert_eq!(
            (first.percent, first.line.as_deref(), first.at),
            (Some(10.0), None, 150)
        );
        follower.feed("log: Starting stacking...", 200);
        follower.feed("progress: 20.00%", 300);
        let second = follower.take(900).unwrap();
        assert_eq!(second.percent, Some(20.0));
        assert_eq!(second.line.as_deref(), Some("Starting stacking..."));
        assert_eq!(second.at, 200, "the time of the line, not of the report");
        assert!(follower.take(901).is_none(), "nothing changed");
        // The same text again is a line that came.
        follower.feed("log: Starting stacking...", 1000);
        assert_eq!(follower.take(1100).unwrap().at, 1000);
        follower.restart_percent();
        assert_eq!(follower.take(1200).unwrap().percent, None);
    }

    #[test]
    fn a_tail_reads_only_complete_lines_and_what_was_added() {
        let dir = TempDir::new("siril-tail");
        let path = dir.path().join("siril.log");
        let mut tail = LogTail::new(&path);
        let mut follower = SirilFollower::new();
        tail.poll(&mut follower, 1);
        assert!(follower.take(1).is_none(), "no file yet");
        let mut file = File::create(&path).unwrap();
        write!(
            file,
            "log: First\n\u{1b}[A\u{1b}[2KT\rprogress: 5.00%\nlog: Half"
        )
        .unwrap();
        tail.poll(&mut follower, 10);
        let report = follower.take(10).unwrap();
        assert_eq!(
            (report.percent, report.line.as_deref()),
            (Some(5.0), Some("First"))
        );
        writeln!(file, " a line").unwrap();
        tail.poll(&mut follower, 20);
        let report = follower.take(20).unwrap();
        assert_eq!(report.line.as_deref(), Some("Half a line"));
        assert_eq!(report.at, 20);
        tail.poll(&mut follower, 30);
        assert!(
            follower.take(30).is_none(),
            "nothing new, nothing read twice"
        );
    }
}

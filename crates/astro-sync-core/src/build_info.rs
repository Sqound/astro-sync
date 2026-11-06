//! What identifies the build of Astro Sync. Every review `.dmg` carries the
//! same version (`1.0.0`); what tells two of them apart is the commit of
//! `main` that was built, which the file name already carries. The window
//! (menu and Settings) and `astro-sync --version` show the same thing, so a
//! version is never mistaken for another.
//!
//! The commit and the date are stamped by `build.rs` at compile time and
//! never looked up at run time (no `git`, no clock). Outside a repository the
//! commit reads `unknown`.

use serde::{Deserialize, Serialize};

/// The commit shown when the build was made outside a repository (or the
/// stamp is not a hexadecimal commit).
pub const UNKNOWN_SHA: &str = "unknown";

/// Length of the short commit.
pub const SHA_LENGTH: usize = 9;

/// The version, commit and date of the build, as the window receives them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildInfo {
    /// The application version (`tauri.conf.json` for the window, the crate
    /// version for the command line).
    pub version: String,
    /// The built commit, 9 hexadecimal characters, or [`UNKNOWN_SHA`].
    pub git_sha: String,
    /// The build instant, ISO 8601 in UTC (`2026-10-13T08:12:00Z`); empty when
    /// unknown.
    pub build_date: String,
}

/// A commit as it is shown: its first [`SHA_LENGTH`] characters in lower
/// case when it is hexadecimal, [`UNKNOWN_SHA`] otherwise (empty, `unknown`,
/// anything else).
pub fn normalize_sha(raw: &str) -> String {
    let raw = raw.trim();
    if raw.len() >= 7 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
        raw.chars()
            .take(SHA_LENGTH)
            .collect::<String>()
            .to_ascii_lowercase()
    } else {
        UNKNOWN_SHA.to_string()
    }
}

/// The build date as stamped, or empty when it does not look like
/// `YYYY-MM-DDTHH:MM:SSZ`.
pub fn normalize_date(raw: &str) -> String {
    let raw = raw.trim();
    let bytes = raw.as_bytes();
    let shaped = bytes.len() == 20
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            10 => *byte == b'T',
            13 | 16 => *byte == b':',
            19 => *byte == b'Z',
            _ => byte.is_ascii_digit(),
        });
    if shaped {
        raw.to_string()
    } else {
        String::new()
    }
}

/// The build information for an application `version`.
pub fn build_info(version: &str) -> BuildInfo {
    from_stamp(
        version,
        option_env!("ASTRO_SYNC_GIT_SHA").unwrap_or(""),
        option_env!("ASTRO_SYNC_BUILD_DATE").unwrap_or(""),
    )
}

/// The same from explicit stamps (what `build.rs` produced), normalised.
pub fn from_stamp(version: &str, sha: &str, date: &str) -> BuildInfo {
    BuildInfo {
        version: version.to_string(),
        git_sha: normalize_sha(sha),
        build_date: normalize_date(date),
    }
}

impl BuildInfo {
    /// The version as the command line writes it after its name:
    /// `1.0.0 (6aa28f767, 2026-10-13)` (the day only, in UTC).
    pub fn cli_version(&self) -> String {
        let day = self.build_date.get(..10).unwrap_or("");
        if day.is_empty() {
            format!("{} ({})", self.version, self.git_sha)
        } else {
            format!("{} ({}, {day})", self.version, self.git_sha)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_commit_is_nine_lowercase_hex_characters() {
        assert_eq!(normalize_sha("6aa28f767"), "6aa28f767");
        assert_eq!(normalize_sha("6AA28F7679DEADBEEF\n"), "6aa28f767");
        assert_eq!(normalize_sha("6aa28f7"), "6aa28f7");
    }

    #[test]
    fn anything_else_falls_back_to_unknown() {
        for raw in ["", "  ", "unknown", "not-a-sha", "6aa2", "6aa28f76g"] {
            assert_eq!(normalize_sha(raw), UNKNOWN_SHA, "{raw:?}");
        }
    }

    #[test]
    fn the_date_is_kept_only_when_it_is_iso_utc() {
        assert_eq!(
            normalize_date("2026-10-13T08:12:00Z"),
            "2026-10-13T08:12:00Z"
        );
        assert_eq!(normalize_date(""), "");
        assert_eq!(normalize_date("13/10/2026"), "");
        assert_eq!(normalize_date("2026-10-13T08:12:00+09:00"), "");
    }

    #[test]
    fn the_stamp_of_this_build_is_well_formed() {
        let info = build_info("1.0.0");
        assert_eq!(info.version, "1.0.0");
        assert!(
            info.git_sha == UNKNOWN_SHA
                || (info.git_sha.len() == SHA_LENGTH
                    && info.git_sha.chars().all(|c| c.is_ascii_hexdigit())),
            "{}",
            info.git_sha
        );
        assert!(!info.build_date.is_empty(), "build.rs stamps a date");
    }

    #[test]
    fn the_command_line_reads_version_commit_and_day() {
        let info = from_stamp("1.0.0", "6aa28f767", "2026-10-13T08:12:00Z");
        assert_eq!(info.cli_version(), "1.0.0 (6aa28f767, 2026-10-13)");
        let undated = from_stamp("1.0.0", "", "");
        assert_eq!(undated.cli_version(), "1.0.0 (unknown)");
    }

    #[test]
    fn the_window_receives_camel_case() {
        let info = from_stamp("1.0.0", "6aa28f767", "2026-10-13T08:12:00Z");
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["gitSha"], "6aa28f767");
        assert_eq!(json["buildDate"], "2026-10-13T08:12:00Z");
    }
}

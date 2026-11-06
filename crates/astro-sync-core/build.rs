//! Stamps the build into the crate: `ASTRO_SYNC_GIT_SHA` (the short commit,
//! 9 characters, `unknown` outside a repository) and `ASTRO_SYNC_BUILD_DATE`
//! (ISO 8601, UTC). `build_info` reads them with `option_env!`, so nothing
//! is ever looked up at run time. Both the command line and the window call
//! `astro_sync_core::build_info`, which is why the stamp lives in this crate
//! rather than in each binary.
//!
//! An already set `ASTRO_SYNC_GIT_SHA` / `ASTRO_SYNC_BUILD_DATE` wins (a
//! build from a source archive, where there is no repository), and
//! `SOURCE_DATE_EPOCH` makes the date reproducible.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    println!("cargo:rerun-if-env-changed=ASTRO_SYNC_GIT_SHA");
    println!("cargo:rerun-if-env-changed=ASTRO_SYNC_BUILD_DATE");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    watch_git_head();

    let sha = std::env::var("ASTRO_SYNC_GIT_SHA")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(git_short_sha)
        .unwrap_or_default();
    let date = std::env::var("ASTRO_SYNC_BUILD_DATE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| iso_utc(build_seconds()));
    println!("cargo:rustc-env=ASTRO_SYNC_GIT_SHA={}", sha.trim());
    println!("cargo:rustc-env=ASTRO_SYNC_BUILD_DATE={}", date.trim());
}

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn git_short_sha() -> Option<String> {
    git(&["rev-parse", "--short=9", "HEAD"])
}

/// A new commit (or a checkout) re-stamps the build: watch `HEAD` and the
/// branch file it points to. Works in a linked worktree too (`--git-path`).
fn watch_git_head() {
    if let Some(head) = git(&["rev-parse", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={head}");
    }
    if let Some(reference) = git(&["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = git(&["rev-parse", "--git-path", &reference])
    {
        println!("cargo:rerun-if-changed={path}");
    }
}

fn build_seconds() -> i64 {
    if let Some(epoch) = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|value| value.trim().parse::<i64>().ok())
    {
        return epoch;
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// `2026-10-13T08:12:00Z` (Hinnant's civil-from-days, no calendar crate).
fn iso_utc(unix_seconds: i64) -> String {
    let days = unix_seconds.div_euclid(86_400);
    let rest = unix_seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

//! Minimal ISO 8601 UTC timestamp formatting and parsing.
//!
//! The open session-package manifest
//! (`contracts/schemas/session-package/manifest.schema.json`)
//! needs a handful of `generatedAt`/`addedAt` timestamps shaped like
//! `"2026-09-20T21:04:00Z"`. That was the only date/time behavior this crate
//! needed at first, so it was implemented here directly instead of pulling in a
//! date/time crate (`chrono`/`time`).
//!
//! The analysis (`crate::analysis`) adds one more, narrow need: guiding-log
//! timestamps (PHD2, ASIAIR) are written in the *local* wall-clock time of
//! whoever configured the capture software, next to a fixed UTC offset the
//! caller supplies (there is no timezone database involved, no daylight saving
//! rules -- just "this log's clock reads UTC+9"). The functions below extend
//! the same zero-dependency approach: [`days_from_civil`] is the exact inverse
//! of [`civil_from_days`] (same Hinnant algorithm, run backwards),
//! [`local_datetime_to_unix_seconds`] combines it with a fixed offset to get a
//! fractional Unix timestamp (fractional because a guide frame's elapsed-time
//! column has sub-second resolution), and [`format_iso8601_utc_millis`] renders
//! that back out for `metrics/*.csv` and `events/session.jsonl`. Still no
//! calendar library, still no timezone database.

use std::time::{SystemTime, UNIX_EPOCH};

/// Test/reproducibility escape hatch: when this environment variable is set
/// to a non-empty value, [`now_iso8601_utc`] returns it verbatim instead of
/// reading the wall clock. Every other timestamp in this crate (queue
/// `createdAt`/`updatedAt`, a package's `generatedAt`/`addedAt`) is derived
/// from it, so a single value neutralizes every volatile field a golden
/// test would otherwise see. This is what lets `astro-sync-cli`'s golden
/// tests (`crates/astro-sync-cli/tests/golden.rs`) run the real CLI binary
/// as a child process and still get byte-identical output across runs; see
/// `README.md`, "CLI / GUI parity". Never read for any other
/// purpose, and never something an end user has a reason to set.
pub const DETERMINISTIC_TIME_ENV_VAR: &str = "ASTRO_SYNC_DETERMINISTIC_TIME";

/// Guards every unit test in this crate (`time_util.rs`'s own tests and
/// `analysis::report::tests`, both compiled into the same `--lib` test
/// binary) that sets, clears, or could concurrently read
/// [`DETERMINISTIC_TIME_ENV_VAR`] via [`now_iso8601_utc`].
/// `std::env::set_var`/`remove_var` mutate process-wide state and are
/// documented as unsound to race against any other thread's concurrent
/// read or write of the environment; Rust's default test harness runs
/// `#[test]` functions on multiple threads within one binary, so a test
/// that overrides this variable is not made safe merely because no *other*
/// test in the same binary happens to assert on the value it would
/// observe if it raced -- the race itself, not just its visible effect, is
/// what must be avoided (this is exactly why `set_var`/`remove_var` are
/// `unsafe fn`). Every test that sets/removes the variable, or that calls
/// a function that reads it (`now_iso8601_utc`, and transitively
/// `analysis::report::generate_report`/`report_from_outcome`), takes
/// [`lock_deterministic_time_env_for_test`] for its entire critical
/// section. A plain `Mutex<()>`: only mutual exclusion matters here, there
/// is no data to protect. `tests/analysis_report_fixtures.rs` (a separate
/// integration-test binary/process) cannot reach this private-module item
/// at all -- it is not racing this binary's tests regardless -- so it
/// rolls its own local equivalent instead. `#[cfg(test)]`: this exists for
/// tests only, and would otherwise be reported as dead code in a normal
/// build (`time_util` is a private module, so even a `pub` item here is
/// not part of the crate's public API).
#[cfg(test)]
pub(crate) static DETERMINISTIC_TIME_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Acquires [`DETERMINISTIC_TIME_ENV_LOCK`], recovering from a poisoned
/// lock the same way every caller does: a panic under the lock in one test
/// must not cascade into spurious failures for every other test that only
/// reads the (real, non-overridden) wall clock.
#[cfg(test)]
pub(crate) fn lock_deterministic_time_env_for_test() -> std::sync::MutexGuard<'static, ()> {
    DETERMINISTIC_TIME_ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Returns the current UTC time formatted as `"YYYY-MM-DDTHH:MM:SSZ"`
/// (seconds resolution, matching the manifest schema's examples) -- or
/// [`DETERMINISTIC_TIME_ENV_VAR`]'s value, verbatim, when it is set.
#[must_use]
pub fn now_iso8601_utc() -> String {
    if let Ok(fixed) = std::env::var(DETERMINISTIC_TIME_ENV_VAR)
        && !fixed.is_empty()
    {
        return fixed;
    }
    system_time_to_iso8601_utc(SystemTime::now())
}

/// Formats an arbitrary [`SystemTime`] the same way; exposed so tests can
/// check known instants without depending on the wall clock.
#[must_use]
pub fn system_time_to_iso8601_utc(time: SystemTime) -> String {
    let seconds_since_epoch = time
        .duration_since(UNIX_EPOCH)
        .expect("system time before 1970-01-01 is not supported")
        .as_secs() as i64;
    let days = seconds_since_epoch.div_euclid(86_400);
    let seconds_of_day = seconds_since_epoch.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3600;
    let minute = (seconds_of_day % 3600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Converts a day count relative to the Unix epoch (1970-01-01) into a
/// proleptic-Gregorian `(year, month, day)`, using the constant-time
/// civil-calendar algorithm popularized by Howard Hinnant's `date` library.
/// Correct for the full range of dates this software will ever encounter.
pub(crate) fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097); // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365; // [0, 399]
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100); // [0, 365]
    let month_position = (5 * day_of_year + 2) / 153; // [0, 11]
    let day = (day_of_year - (153 * month_position + 2) / 5 + 1) as u32; // [1, 31]
    let month = if month_position < 10 {
        month_position + 3
    } else {
        month_position - 9
    } as u32; // [1, 12]
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

/// Exact inverse of [`civil_from_days`] (Howard Hinnant's algorithm run
/// backwards): the day count relative to the Unix epoch (1970-01-01) for a
/// proleptic-Gregorian `(year, month, day)`. Used by
/// [`local_datetime_to_unix_seconds`] to turn a guiding log's local
/// calendar date into a Unix timestamp.
pub(crate) fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = (if year >= 0 { year } else { year - 399 }).div_euclid(400);
    let year_of_era = year - era * 400; // [0, 399]
    let month_position = i64::from(if month > 2 { month - 3 } else { month + 9 }); // [0, 11]
    let day_of_year = (153 * month_position + 2).div_euclid(5) + i64::from(day) - 1; // [0, 365]
    let day_of_era =
        year_of_era * 365 + year_of_era.div_euclid(4) - year_of_era.div_euclid(100) + day_of_year; // [0, 146096]
    era * 146_097 + day_of_era - 719_468
}

/// Parses a fixed `"+HH:MM"`/`"-HH:MM"` UTC offset (the shape every source
/// this crate's log parsers accept uses -- a guiding application does not
/// carry a timezone database, only "this clock reads UTC+9") into signed
/// seconds east of UTC. Returns `None` for anything else, including the
/// `Z`/`+00:00` forms `parse_fixed_offset_seconds` itself already covers
/// via a plain `"+00:00"`.
#[must_use]
pub(crate) fn parse_fixed_offset_seconds(offset: &str) -> Option<i64> {
    let bytes = offset.as_bytes();
    if bytes.len() != 6 || bytes[3] != b':' {
        return None;
    }
    let sign = match bytes[0] {
        b'+' => 1_i64,
        b'-' => -1_i64,
        _ => return None,
    };
    let hours: i64 = offset.get(1..3)?.parse().ok()?;
    let minutes: i64 = offset.get(4..6)?.parse().ok()?;
    if hours > 23 || minutes > 59 {
        return None;
    }
    Some(sign * (hours * 3600 + minutes * 60))
}

/// Converts a local wall-clock date/time (as printed in a guiding log) plus
/// a fixed UTC offset (seconds east of UTC, from
/// [`parse_fixed_offset_seconds`]) into seconds since the Unix epoch.
/// Fractional seconds are accepted (and returned) because a PHD2 guide
/// frame's elapsed-time column has sub-second resolution.
#[must_use]
pub(crate) fn local_datetime_to_unix_seconds(
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: f64,
    offset_seconds: i64,
) -> f64 {
    let days = days_from_civil(year, month, day);
    let local_seconds =
        (days * 86_400) as f64 + f64::from(hour) * 3600.0 + f64::from(minute) * 60.0 + second;
    local_seconds - offset_seconds as f64
}

/// Formats a (possibly fractional) Unix timestamp as ISO 8601 UTC with
/// millisecond precision: `"YYYY-MM-DDTHH:MM:SS.sssZ"`. Used by
/// `crate::analysis`'s writers, which need sub-second resolution to keep
/// guide frames a fraction of a second apart distinguishable -- unlike
/// [`now_iso8601_utc`]'s whole-second manifest timestamps.
#[must_use]
pub(crate) fn format_iso8601_utc_millis(unix_seconds: f64) -> String {
    let total_millis = (unix_seconds * 1000.0).round() as i64;
    let whole_seconds = total_millis.div_euclid(1000);
    let millis = total_millis.rem_euclid(1000);
    let days = whole_seconds.div_euclid(86_400);
    let seconds_of_day = whole_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3600;
    let minute = (seconds_of_day % 3600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// Parses a `"YYYY-MM-DDTHH:MM:SS[.sss]Z"` timestamp -- the exact shape
/// [`format_iso8601_utc_millis`] produces, and the shape FITS `DATE-OBS`
/// values are normalized to (`crate::analysis::fits::extract_acquisition_record`)
/// -- back into seconds since the Unix epoch. Used only to correlate a
/// FITS exposure's time window against guiding samples
/// (`crate::analysis::analyze_folder`); a timestamp carrying an explicit
/// non-`Z` offset (rare in practice -- see that function's module doc
/// comment) is not handled and returns `None`, since this crate has no
/// general-purpose offset parser beyond the fixed `"+HH:MM"` log-header
/// form [`parse_fixed_offset_seconds`] already covers.
#[must_use]
pub(crate) fn parse_iso8601_utc_to_unix_seconds(text: &str) -> Option<f64> {
    let text = text.strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    let mut time_parts = time.split(':');
    let hour: u32 = time_parts.next()?.parse().ok()?;
    let minute: u32 = time_parts.next()?.parse().ok()?;
    let second: f64 = time_parts.next()?.parse().ok()?;
    Some(local_datetime_to_unix_seconds(
        year, month, day, hour, minute, second, 0,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn epoch_formats_as_expected() {
        assert_eq!(
            system_time_to_iso8601_utc(UNIX_EPOCH),
            "1970-01-01T00:00:00Z"
        );
    }

    #[test]
    fn known_instant_matches_the_manifest_examples_timestamp() {
        // 2026-09-20T21:04:00Z is generatedAt in
        // contracts/schemas/session-package/examples/manifest.minimal.json.
        let time = UNIX_EPOCH + Duration::from_secs(1_789_938_240);
        assert_eq!(system_time_to_iso8601_utc(time), "2026-09-20T21:04:00Z");
    }

    #[test]
    fn leap_day_round_trips() {
        // 2000-02-29T12:00:00Z: 11016 days after the epoch, at noon.
        let time = UNIX_EPOCH + Duration::from_secs(11_016 * 86_400 + 43_200);
        assert_eq!(system_time_to_iso8601_utc(time), "2000-02-29T12:00:00Z");
    }

    #[test]
    fn now_is_well_formed() {
        // Reads the (real, non-overridden) wall clock, but still takes the
        // shared lock: without it, this call could race with another
        // thread's `set_var`/`remove_var` of `DETERMINISTIC_TIME_ENV_VAR`
        // below (see `DETERMINISTIC_TIME_ENV_LOCK`'s own doc comment).
        let _guard = lock_deterministic_time_env_for_test();
        let formatted = now_iso8601_utc();
        assert_eq!(formatted.len(), 20);
        assert!(formatted.starts_with("20"));
        assert!(formatted.ends_with('Z'));
    }

    /// The fixed value used here ("2026-09-20T21:04:00Z", the same known
    /// instant as `known_instant_matches_the_manifest_examples_timestamp`
    /// above) is exactly the shape every other test in this crate that
    /// happens to call `now_iso8601_utc()` already tolerates (20
    /// characters, starts with `"20"`, ends with `'Z'` -- see
    /// `now_is_well_formed`, and the `generatedAt`/`addedAt` checks in
    /// `package.rs`'s tests). That tolerance is a belt-and-braces fallback,
    /// not the actual safety mechanism: `lock_deterministic_time_env_for_test`
    /// (held for this test's entire critical section) is what actually
    /// prevents any other thread from observing -- or racing the mutation
    /// of -- this environment variable while it is set.
    #[test]
    fn deterministic_override_returns_the_env_var_verbatim_when_set() {
        let _guard = lock_deterministic_time_env_for_test();
        // Safety: mutating the process environment races with any other
        // thread reading or writing it at the same instant; the lock above
        // is what rules that out (see `DETERMINISTIC_TIME_ENV_LOCK`'s own
        // doc comment) -- not merely that no other test's assertion would
        // notice.
        unsafe {
            std::env::set_var(DETERMINISTIC_TIME_ENV_VAR, "2026-09-20T21:04:00Z");
        }
        let observed = now_iso8601_utc();
        unsafe {
            std::env::remove_var(DETERMINISTIC_TIME_ENV_VAR);
        }
        assert_eq!(observed, "2026-09-20T21:04:00Z");
    }

    #[test]
    fn an_empty_override_falls_back_to_the_wall_clock() {
        let _guard = lock_deterministic_time_env_for_test();
        // Safety: see `deterministic_override_returns_the_env_var_verbatim_when_set`.
        unsafe {
            std::env::set_var(DETERMINISTIC_TIME_ENV_VAR, "");
        }
        let observed = now_iso8601_utc();
        unsafe {
            std::env::remove_var(DETERMINISTIC_TIME_ENV_VAR);
        }
        assert_eq!(observed.len(), 20);
        assert!(observed.starts_with("20"));
        assert!(observed.ends_with('Z'));
    }

    #[test]
    fn days_from_civil_is_the_exact_inverse_of_civil_from_days() {
        // Sweep a range wide enough to cross several leap years and every
        // month boundary, in both directions, and check the round trip.
        for days in -20_000_i64..20_000 {
            let (year, month, day) = civil_from_days(days);
            assert_eq!(days_from_civil(year, month, day), days, "{days}");
        }
    }

    #[test]
    fn days_from_civil_matches_the_epoch_and_a_known_leap_day() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 2, 29), 11_016);
    }

    #[test]
    fn parse_fixed_offset_seconds_covers_zero_positive_and_negative() {
        assert_eq!(parse_fixed_offset_seconds("+00:00"), Some(0));
        assert_eq!(parse_fixed_offset_seconds("+09:00"), Some(9 * 3600));
        assert_eq!(
            parse_fixed_offset_seconds("-05:30"),
            Some(-(5 * 3600 + 30 * 60))
        );
    }

    #[test]
    fn parse_fixed_offset_seconds_rejects_malformed_input() {
        for bad in ["Z", "+9:00", "09:00", "+24:00", "+00:60", "", "+00-00"] {
            assert_eq!(parse_fixed_offset_seconds(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn local_datetime_to_unix_seconds_matches_the_manifest_examples_instant() {
        // Same known instant as `known_instant_matches_the_manifest_examples_timestamp`,
        // read as a UTC (zero-offset) local wall clock.
        let seconds = local_datetime_to_unix_seconds(2026, 9, 20, 21, 4, 0.0, 0);
        assert_eq!(seconds, 1_789_938_240.0);
    }

    #[test]
    fn local_datetime_to_unix_seconds_applies_a_positive_offset_correctly() {
        // 2026-08-19 22:00:00 at UTC+9 is 2026-08-19T13:00:00Z.
        let seconds = local_datetime_to_unix_seconds(2026, 8, 19, 22, 0, 0.0, 9 * 3600);
        let expected = local_datetime_to_unix_seconds(2026, 8, 19, 13, 0, 0.0, 0);
        assert_eq!(seconds, expected);
    }

    #[test]
    fn local_datetime_to_unix_seconds_keeps_fractional_seconds() {
        // The two instants are ~1.8e9 seconds from the epoch, so their f64
        // difference carries a little floating-point noise in the last
        // couple of decimal digits; `format_iso8601_utc_millis` rounds to
        // milliseconds precisely to absorb that, which is what this test
        // checks for instead of an exact subtraction.
        let base = local_datetime_to_unix_seconds(2026, 8, 19, 22, 0, 0.0, 0);
        let plus_a_fraction = local_datetime_to_unix_seconds(2026, 8, 19, 22, 0, 1.201, 0);
        assert!((plus_a_fraction - base - 1.201).abs() < 1e-6);
    }

    #[test]
    fn format_iso8601_utc_millis_matches_the_manifest_examples_instant() {
        assert_eq!(
            format_iso8601_utc_millis(1_789_938_240.0),
            "2026-09-20T21:04:00.000Z"
        );
    }

    #[test]
    fn format_iso8601_utc_millis_keeps_fractional_seconds() {
        assert_eq!(
            format_iso8601_utc_millis(1_789_938_240.201),
            "2026-09-20T21:04:00.201Z"
        );
    }

    #[test]
    fn format_iso8601_utc_millis_carries_a_rounded_millisecond_into_the_next_second() {
        // 999.9997 ms rounds to 1000 ms, which must carry into the next
        // second rather than rendering an invalid ".1000" or ":60".
        assert_eq!(
            format_iso8601_utc_millis(1_789_938_240.999_7),
            "2026-09-20T21:04:01.000Z"
        );
    }

    #[test]
    fn parse_iso8601_utc_to_unix_seconds_round_trips_with_the_formatter() {
        let formatted = format_iso8601_utc_millis(1_789_938_240.201);
        assert_eq!(
            parse_iso8601_utc_to_unix_seconds(&formatted),
            Some(1_789_938_240.201)
        );
    }

    #[test]
    fn parse_iso8601_utc_to_unix_seconds_rejects_a_non_z_timestamp() {
        assert_eq!(
            parse_iso8601_utc_to_unix_seconds("2026-09-20T21:04:00+09:00"),
            None
        );
    }

    #[test]
    fn local_datetime_and_format_round_trip_through_a_positive_offset() {
        let seconds = local_datetime_to_unix_seconds(2026, 8, 7, 22, 38, 3.5, 9 * 3600);
        assert_eq!(
            format_iso8601_utc_millis(seconds),
            "2026-08-07T13:38:03.500Z"
        );
    }
}

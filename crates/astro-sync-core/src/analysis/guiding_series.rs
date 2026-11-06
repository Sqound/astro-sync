//! The night's guiding by slices of two minutes (`guidingSeries[]`
//! of the report v2): the
//! root-mean-square of the recorded offsets in right ascension, in
//! declination and in total over each slice, with the number of samples it
//! holds. Slices start at the night's first sample, every two minutes (the
//! old portal's length); a slice without a sample is not written. The raw
//! samples stay in the report's `guiding[]`, as before.

use serde::Serialize;

use super::GuidingRow;
use crate::time_util::{format_iso8601_utc_millis, parse_iso8601_utc_to_unix_seconds};

/// The slice length, in seconds.
pub const SLICE_SECONDS: f64 = 120.0;

/// One slice of `guidingSeries[]` (`session-analysis.schema.json`,
/// `guidingSlice`), in arcseconds.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuidingSlice {
    pub start: String,
    pub end: String,
    pub rms_ra: Option<f64>,
    pub rms_dec: Option<f64>,
    pub rms_total: Option<f64>,
    pub samples: u64,
}

fn rms(values: &[f64]) -> Option<f64> {
    (!values.is_empty())
        .then(|| (values.iter().map(|v| v * v).sum::<f64>() / values.len() as f64).sqrt())
}

/// Cuts `rows` (any order) into slices of [`SLICE_SECONDS`].
#[must_use]
pub fn guiding_series(rows: &[GuidingRow]) -> Vec<GuidingSlice> {
    let mut samples: Vec<(f64, Option<f64>, Option<f64>)> = rows
        .iter()
        .filter_map(|row| {
            let at = parse_iso8601_utc_to_unix_seconds(&row.timestamp_utc)?;
            at.is_finite()
                .then_some((at, row.ra_offset_arcsec, row.dec_offset_arcsec))
        })
        .collect();
    samples.sort_by(|a, b| a.0.total_cmp(&b.0));
    let Some(first) = samples.first().map(|s| s.0) else {
        return Vec::new();
    };

    let mut slices = Vec::new();
    let mut index = 0;
    while index < samples.len() {
        let slot = ((samples[index].0 - first) / SLICE_SECONDS).floor();
        let start = first + slot * SLICE_SECONDS;
        let end = start + SLICE_SECONDS;
        let mut ra = Vec::new();
        let mut dec = Vec::new();
        let mut count = 0_u64;
        while index < samples.len() && samples[index].0 < end {
            let (_, r, d) = samples[index];
            if let Some(r) = r {
                ra.push(r);
            }
            if let Some(d) = d {
                dec.push(d);
            }
            count += 1;
            index += 1;
        }
        let rms_ra = rms(&ra);
        let rms_dec = rms(&dec);
        slices.push(GuidingSlice {
            start: format_iso8601_utc_millis(start),
            end: format_iso8601_utc_millis(end),
            rms_ra,
            rms_dec,
            rms_total: match (rms_ra, rms_dec) {
                (Some(r), Some(d)) => Some(r.hypot(d)),
                _ => None,
            },
            samples: count,
        });
    }
    slices
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(at: &str, ra: f64, dec: f64) -> GuidingRow {
        GuidingRow {
            timestamp_utc: at.to_string(),
            source: "phd2".to_string(),
            ra_offset_arcsec: Some(ra),
            dec_offset_arcsec: Some(dec),
            ..GuidingRow::default()
        }
    }

    #[test]
    fn cuts_the_samples_in_two_minute_slices_and_skips_an_empty_one() {
        let rows = vec![
            row("2026-10-07T21:10:00.000Z", 0.3, 0.4),
            row("2026-10-07T21:11:00.000Z", 0.3, 0.4),
            row("2026-10-07T21:12:30.000Z", 0.6, 0.8),
            // 21:14–21:16 holds nothing; 21:16:10 opens the fourth slot.
            row("2026-10-07T21:16:10.000Z", 1.0, 0.0),
        ];
        let slices = guiding_series(&rows);
        assert_eq!(slices.len(), 3);
        assert_eq!(slices[0].start, "2026-10-07T21:10:00.000Z");
        assert_eq!(slices[0].end, "2026-10-07T21:12:00.000Z");
        assert_eq!(slices[0].samples, 2);
        assert!((slices[0].rms_ra.unwrap() - 0.3).abs() < 1e-9);
        assert!((slices[0].rms_total.unwrap() - 0.5).abs() < 1e-9);
        assert_eq!(slices[1].start, "2026-10-07T21:12:00.000Z");
        assert!((slices[1].rms_total.unwrap() - 1.0).abs() < 1e-9);
        assert_eq!(slices[2].start, "2026-10-07T21:16:00.000Z");
        assert_eq!(slices[2].samples, 1);
    }

    #[test]
    fn no_sample_no_slice() {
        assert!(guiding_series(&[]).is_empty());
    }
}

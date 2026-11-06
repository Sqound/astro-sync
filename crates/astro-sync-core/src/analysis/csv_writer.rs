//! Writers for `metrics/quality.csv` and `metrics/guiding.csv`, exactly
//! matching the column specification in
//! `contracts/schemas/session-package/metrics.md`. Both header
//! rows are `pub const`s so a test can assert the exact string.
//!
//! Every numeric column is rendered at the fixed precision `metrics.md`
//! documents (never trimmed, so two files are always byte-comparable), and
//! a missing value is always an empty field, never the text `"null"` or a
//! placeholder number -- CSV's own way of saying "not recorded", matching
//! the evidence rule (a value that was not parsed is never invented).
//!
//! `QualityRow`/`GuidingRow` also derive `Serialize` (`analysis::report`):
//! `#[serde(rename_all = "camelCase")]` renders the exact same field names
//! as the CSV header constants below, so the report generator's JSON and
//! these CSV files describe the same rows under the same names. Purely
//! additive -- nothing here reads or writes JSON itself.

use serde::Serialize;

pub const QUALITY_CSV_HEADER: &str = "timestampUtc,acquisitionFamily,filter,exposureSeconds,gain,iso,binning,sensorTempC,rmsTotalArcsec,rmsRaArcsec,rmsDecArcsec,guidingSampleCount,skyBackgroundAdu,fwhmPixels,eccentricity,sourcePath";

pub const GUIDING_CSV_HEADER: &str = "timestampUtc,source,frame,elapsedSeconds,raOffsetArcsec,decOffsetArcsec,raDurationMs,decDurationMs,starMass,snr,errorCode";

/// One `metrics/quality.csv` row: one science sub-exposure ("pose"),
/// mostly sourced from its FITS header, with the guiding RMS aggregated
/// over that exposure's time window from `metrics/guiding.csv`'s raw
/// samples (a root-mean-square of a directly recorded quantity, in one
/// physical unit, arcseconds -- not a composite score; see
/// `metrics.md`, "Why this is not a score").
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityRow {
    pub timestamp_utc: String,
    pub acquisition_family: String,
    pub filter: Option<String>,
    pub exposure_seconds: Option<f64>,
    pub gain: Option<f64>,
    pub iso: Option<f64>,
    pub binning: Option<String>,
    pub sensor_temp_c: Option<f64>,
    pub rms_total_arcsec: Option<f64>,
    pub rms_ra_arcsec: Option<f64>,
    pub rms_dec_arcsec: Option<f64>,
    pub guiding_sample_count: Option<u64>,
    pub sky_background_adu: Option<f64>,
    pub fwhm_pixels: Option<f64>,
    pub eccentricity: Option<f64>,
    pub source_path: String,
}

/// One `metrics/guiding.csv` row: one raw guide sample, from either
/// source.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuidingRow {
    pub timestamp_utc: String,
    pub source: String,
    pub frame: Option<u64>,
    pub elapsed_seconds: Option<f64>,
    pub ra_offset_arcsec: Option<f64>,
    pub dec_offset_arcsec: Option<f64>,
    pub ra_duration_ms: Option<f64>,
    pub dec_duration_ms: Option<f64>,
    pub star_mass: Option<f64>,
    pub snr: Option<f64>,
    pub error_code: Option<i64>,
    /// The way the pulses went (`+1` east / north, `-1` west / south), for
    /// the reduced trace of the report v3 (`guiding_trace`); not a column of
    /// `metrics/guiding.csv`.
    #[serde(skip)]
    pub ra_direction: Option<i8>,
    #[serde(skip)]
    pub dec_direction: Option<i8>,
}

/// Quotes a CSV field per RFC 4180 only when needed (contains a comma, a
/// double quote, or a newline): doubling embedded quotes and wrapping the
/// whole field in `"`.
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn fmt_f64(value: Option<f64>, decimals: usize) -> String {
    value.map_or_else(String::new, |v| format!("{v:.decimals$}"))
}

fn fmt_int<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(String::new, |v| v.to_string())
}

fn fmt_opt_str(value: &Option<String>) -> String {
    value.as_deref().map_or_else(String::new, csv_field)
}

/// Renders every row into `metrics/quality.csv`'s exact shape (including
/// the asserted header line and a trailing newline); an empty `rows` slice
/// still produces a header-only file, since the contract's minimal package
/// never requires this file to carry data, only a valid shape when present.
#[must_use]
pub fn write_quality_csv(rows: &[QualityRow]) -> String {
    let mut out = String::from(QUALITY_CSV_HEADER);
    out.push('\n');
    for row in rows {
        out.push_str(&csv_field(&row.timestamp_utc));
        out.push(',');
        out.push_str(&csv_field(&row.acquisition_family));
        out.push(',');
        out.push_str(&fmt_opt_str(&row.filter));
        out.push(',');
        out.push_str(&fmt_f64(row.exposure_seconds, 3));
        out.push(',');
        out.push_str(&fmt_f64(row.gain, 2));
        out.push(',');
        out.push_str(&fmt_f64(row.iso, 0));
        out.push(',');
        out.push_str(&fmt_opt_str(&row.binning));
        out.push(',');
        out.push_str(&fmt_f64(row.sensor_temp_c, 1));
        out.push(',');
        out.push_str(&fmt_f64(row.rms_total_arcsec, 3));
        out.push(',');
        out.push_str(&fmt_f64(row.rms_ra_arcsec, 3));
        out.push(',');
        out.push_str(&fmt_f64(row.rms_dec_arcsec, 3));
        out.push(',');
        out.push_str(&fmt_int(row.guiding_sample_count));
        out.push(',');
        out.push_str(&fmt_f64(row.sky_background_adu, 2));
        out.push(',');
        out.push_str(&fmt_f64(row.fwhm_pixels, 2));
        out.push(',');
        out.push_str(&fmt_f64(row.eccentricity, 3));
        out.push(',');
        out.push_str(&csv_field(&row.source_path));
        out.push('\n');
    }
    out
}

/// Renders every row into `metrics/guiding.csv`'s exact shape.
#[must_use]
pub fn write_guiding_csv(rows: &[GuidingRow]) -> String {
    let mut out = String::from(GUIDING_CSV_HEADER);
    out.push('\n');
    for row in rows {
        out.push_str(&csv_field(&row.timestamp_utc));
        out.push(',');
        out.push_str(&csv_field(&row.source));
        out.push(',');
        out.push_str(&fmt_int(row.frame));
        out.push(',');
        out.push_str(&fmt_f64(row.elapsed_seconds, 3));
        out.push(',');
        out.push_str(&fmt_f64(row.ra_offset_arcsec, 3));
        out.push(',');
        out.push_str(&fmt_f64(row.dec_offset_arcsec, 3));
        out.push(',');
        out.push_str(&fmt_f64(row.ra_duration_ms, 0));
        out.push(',');
        out.push_str(&fmt_f64(row.dec_duration_ms, 0));
        out.push(',');
        out.push_str(&fmt_f64(row.star_mass, 0));
        out.push(',');
        out.push_str(&fmt_f64(row.snr, 2));
        out.push(',');
        out.push_str(&fmt_int(row.error_code));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_csv_header_is_exactly_the_approved_spec() {
        assert_eq!(
            QUALITY_CSV_HEADER,
            "timestampUtc,acquisitionFamily,filter,exposureSeconds,gain,iso,binning,sensorTempC,rmsTotalArcsec,rmsRaArcsec,rmsDecArcsec,guidingSampleCount,skyBackgroundAdu,fwhmPixels,eccentricity,sourcePath"
        );
    }

    #[test]
    fn guiding_csv_header_is_exactly_the_approved_spec() {
        assert_eq!(
            GUIDING_CSV_HEADER,
            "timestampUtc,source,frame,elapsedSeconds,raOffsetArcsec,decOffsetArcsec,raDurationMs,decDurationMs,starMass,snr,errorCode"
        );
    }

    #[test]
    fn an_empty_quality_csv_is_header_only() {
        assert_eq!(write_quality_csv(&[]), format!("{QUALITY_CSV_HEADER}\n"));
    }

    #[test]
    fn an_empty_guiding_csv_is_header_only() {
        assert_eq!(write_guiding_csv(&[]), format!("{GUIDING_CSV_HEADER}\n"));
    }

    #[test]
    fn missing_values_render_as_empty_fields_not_placeholders() {
        let row = QualityRow {
            timestamp_utc: "2026-08-07T13:40:03.000Z".to_string(),
            acquisition_family: "Ha_300s_g100_1x1_t-10C".to_string(),
            source_path: "images/frame-0001.fits".to_string(),
            ..Default::default()
        };
        let csv = write_quality_csv(&[row]);
        let data_line = csv.lines().nth(1).unwrap();
        assert_eq!(
            data_line,
            "2026-08-07T13:40:03.000Z,Ha_300s_g100_1x1_t-10C,,,,,,,,,,,,,,images/frame-0001.fits"
        );
    }

    #[test]
    fn a_full_quality_row_formats_every_column_at_its_fixed_precision() {
        let row = QualityRow {
            timestamp_utc: "2026-08-07T13:40:03.000Z".to_string(),
            acquisition_family: "Ha_300s_g100_1x1_t-10C".to_string(),
            filter: Some("Ha".to_string()),
            exposure_seconds: Some(300.0),
            gain: Some(100.0),
            iso: None,
            binning: Some("1x1".to_string()),
            sensor_temp_c: Some(-10.2),
            rms_total_arcsec: Some(0.812_345),
            rms_ra_arcsec: Some(0.5),
            rms_dec_arcsec: Some(0.6),
            guiding_sample_count: Some(120),
            sky_background_adu: Some(512.4),
            fwhm_pixels: Some(2.4),
            eccentricity: Some(0.12),
            source_path: "images/frame-0001.fits".to_string(),
        };
        let csv = write_quality_csv(&[row]);
        let data_line = csv.lines().nth(1).unwrap();
        assert_eq!(
            data_line,
            "2026-08-07T13:40:03.000Z,Ha_300s_g100_1x1_t-10C,Ha,300.000,100.00,,1x1,-10.2,0.812,0.500,0.600,120,512.40,2.40,0.120,images/frame-0001.fits"
        );
    }

    #[test]
    fn a_filter_name_containing_a_comma_is_quoted() {
        let row = QualityRow {
            filter: Some("Ha, narrowband".to_string()),
            ..Default::default()
        };
        let csv = write_quality_csv(&[row]);
        assert!(csv.contains("\"Ha, narrowband\""));
    }

    #[test]
    fn a_full_guiding_row_formats_every_column() {
        let row = GuidingRow {
            timestamp_utc: "2026-08-07T13:38:04.201Z".to_string(),
            source: "phd2".to_string(),
            frame: Some(1),
            elapsed_seconds: Some(1.201),
            ra_offset_arcsec: Some(-0.04),
            dec_offset_arcsec: Some(0.602),
            ra_duration_ms: Some(0.0),
            dec_duration_ms: Some(0.0),
            star_mass: Some(32059.0),
            snr: Some(12.7),
            error_code: Some(0),
            ..GuidingRow::default()
        };
        let csv = write_guiding_csv(&[row]);
        let data_line = csv.lines().nth(1).unwrap();
        assert_eq!(
            data_line,
            "2026-08-07T13:38:04.201Z,phd2,1,1.201,-0.040,0.602,0,0,32059,12.70,0"
        );
    }

    #[test]
    fn csv_lines_are_lf_terminated_only() {
        let csv = write_guiding_csv(&[GuidingRow::default()]);
        assert!(!csv.contains('\r'));
        assert!(csv.ends_with('\n'));
    }
}

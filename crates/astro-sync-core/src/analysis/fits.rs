//! FITS header extraction: a minimal, dependency-free reader for the fixed
//! 80-character "card" / 2880-byte "block" structure every FITS file
//! begins with, plus the small set of standard and de-facto-standard
//! keywords capture software (N.I.N.A., ASIAIR, SharpCap, PixInsight...)
//! writes into a light frame's primary header.
//!
//! A home-grown reader was chosen over the pure-Rust and cfitsio-binding
//! crate candidates: the
//! FITS header format has been fixed since 1981 (NOST 100-2.0), reading it
//! needs no image-data decoding, no WCS, no compression support, and no
//! write path, so the handful of rules below (80 bytes per card, cards
//! packed into 2880-byte blocks, an `END` card closes the header, blank
//! padding after it) covers everything v1 needs with zero dependencies and
//! zero licence questions. **v1 reads only the values FITS-writing software
//! already recorded; it never computes anything from pixel data.**

use std::fmt;

/// One 80-character card.
pub(crate) const CARD_LEN: usize = 80;
/// Cards are packed into 2880-byte blocks (36 cards each); the header ends
/// with an `END` card, and the rest of its last block is blank padding.
pub(crate) const BLOCK_LEN: usize = 2880;
/// Safety cap on how many blocks this reader will scan looking for `END`
/// before giving up -- generous for any real light-frame primary header
/// (a handful of blocks at most) while refusing to walk arbitrarily far
/// into a large FITS file's pixel data if a header is malformed and never
/// closes.
const MAX_HEADER_BLOCKS: usize = 64;

#[derive(Debug, PartialEq, Eq)]
pub enum FitsError {
    /// Fewer than 80 bytes: not even one card fits.
    TooShortForACard,
    /// No `END` card found within [`MAX_HEADER_BLOCKS`] blocks.
    MissingEndCard,
    /// A card was not valid ASCII (FITS headers are pure ASCII by
    /// definition); carries the byte offset of the offending card.
    NonAsciiCard { offset: usize },
}

impl fmt::Display for FitsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FitsError::TooShortForACard => {
                write!(f, "fewer than {CARD_LEN} bytes: not a FITS header")
            }
            FitsError::MissingEndCard => write!(
                f,
                "no END card found within the first {MAX_HEADER_BLOCKS} blocks"
            ),
            FitsError::NonAsciiCard { offset } => {
                write!(f, "non-ASCII bytes in the card at offset {offset}")
            }
        }
    }
}

impl std::error::Error for FitsError {}

/// One parsed header card. `value` is `None` for a blank, `COMMENT`, or
/// `HISTORY` card (anything without a `= ` value indicator in columns 9-10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FitsCard {
    pub keyword: String,
    pub value: Option<String>,
    pub comment: Option<String>,
}

/// Every card from a FITS primary header, in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FitsHeader {
    pub cards: Vec<FitsCard>,
}

impl FitsHeader {
    /// The raw (already-unquoted-if-a-string) value text for the first card
    /// matching `keyword` (case-insensitive, as FITS keywords always are).
    #[must_use]
    pub fn get(&self, keyword: &str) -> Option<&str> {
        self.cards
            .iter()
            .find(|card| card.keyword.eq_ignore_ascii_case(keyword))
            .and_then(|card| card.value.as_deref())
    }

    /// The comment of the first card matching `keyword` (what follows the
    /// `/` after the value), which is where some writers state a unit
    /// (`EXPOSURE = 10000 / [ms] Total Exposure Time`).
    #[must_use]
    pub fn comment(&self, keyword: &str) -> Option<&str> {
        self.cards
            .iter()
            .find(|card| card.keyword.eq_ignore_ascii_case(keyword))
            .and_then(|card| card.comment.as_deref())
    }

    /// Parses the value as a FITS floating-point literal: `D`/`d` exponent
    /// markers (Fortran double-precision notation, still emitted by some
    /// capture/reduction software) are normalized to `E` before parsing.
    #[must_use]
    pub fn get_f64(&self, keyword: &str) -> Option<f64> {
        self.get(keyword)?
            .replace(['D', 'd'], "E")
            .trim()
            .parse()
            .ok()
    }

    /// Parses the value as an integer, tolerating a value written with a
    /// trailing `.0` (some software writes `GAIN` or binning factors as a
    /// FITS float literal even though the value is conceptually an
    /// integer).
    #[must_use]
    pub fn get_i64(&self, keyword: &str) -> Option<i64> {
        let text = self.get(keyword)?.trim();
        text.parse()
            .ok()
            .or_else(|| self.get_f64(keyword).map(|value| value.round() as i64))
            .filter(|_| !text.is_empty())
    }
}

/// Reads the primary header of a FITS byte stream: every card up to and
/// excluding `END`. Never reads past the header into the data unit.
pub fn parse_primary_header(bytes: &[u8]) -> Result<FitsHeader, FitsError> {
    parse_primary_header_with_data_offset(bytes).map(|(header, _)| header)
}

/// [`parse_primary_header`], plus the byte offset where the primary data
/// unit starts: the end of the block that holds the `END` card (the header
/// is padded to a whole number of 2880-byte blocks). The pixel reader of
/// `analysis::sky` starts there.
pub fn parse_primary_header_with_data_offset(
    bytes: &[u8],
) -> Result<(FitsHeader, usize), FitsError> {
    if bytes.len() < CARD_LEN {
        return Err(FitsError::TooShortForACard);
    }
    let scan_limit = bytes.len().min(BLOCK_LEN * MAX_HEADER_BLOCKS);
    let mut cards = Vec::new();
    let mut offset = 0;
    loop {
        if offset + CARD_LEN > scan_limit {
            return Err(FitsError::MissingEndCard);
        }
        let raw = &bytes[offset..offset + CARD_LEN];
        let text = std::str::from_utf8(raw).map_err(|_| FitsError::NonAsciiCard { offset })?;
        offset += CARD_LEN;

        let keyword = text[0..8].trim_end();
        if keyword == "END" {
            let data_offset = offset.div_ceil(BLOCK_LEN) * BLOCK_LEN;
            return Ok((FitsHeader { cards }, data_offset));
        }
        if keyword.is_empty() {
            continue; // blank card: padding, allowed anywhere in the header
        }
        let rest = &text[8..];
        if let Some(value_part) = rest.strip_prefix("= ") {
            let (value, comment) = split_value_and_comment(value_part);
            cards.push(FitsCard {
                keyword: keyword.to_string(),
                value: Some(value),
                comment,
            });
        } else {
            let comment = rest.trim();
            cards.push(FitsCard {
                keyword: keyword.to_string(),
                value: None,
                comment: (!comment.is_empty()).then(|| comment.to_string()),
            });
        }
    }
}

/// Reads the primary header of the FITS file at `path`, block by block,
/// stopping at the block that holds `END` (the detection reads the first
/// header of every frame of a night, and a frame weighs tens of megabytes
/// -- never read its pixels for that).
pub fn read_primary_header_file(path: &std::path::Path) -> std::io::Result<FitsHeader> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut bytes = Vec::with_capacity(BLOCK_LEN);
    let mut block = vec![0_u8; BLOCK_LEN];
    for _ in 0..MAX_HEADER_BLOCKS {
        let mut filled = 0;
        while filled < BLOCK_LEN {
            let read = file.read(&mut block[filled..])?;
            if read == 0 {
                break;
            }
            filled += read;
        }
        bytes.extend_from_slice(&block[..filled]);
        match parse_primary_header(&bytes) {
            Ok(header) => return Ok(header),
            Err(FitsError::MissingEndCard) if filled == BLOCK_LEN => {}
            Err(error) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    error.to_string(),
                ));
            }
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        FitsError::MissingEndCard.to_string(),
    ))
}

/// Splits a card's value field (everything after the `"= "` indicator)
/// into the value proper and an optional trailing `/ comment`, honoring
/// FITS's quoted-string rule: a value starting with `'` runs to the next
/// *unescaped* `'` (two consecutive quotes inside the string are one
/// literal quote), and only text after that closing quote can start a
/// comment.
fn split_value_and_comment(value_part: &str) -> (String, Option<String>) {
    let trimmed_start = value_part.trim_start();
    if let Some(after_quote) = trimmed_start.strip_prefix('\'') {
        let mut result = String::new();
        let mut chars = after_quote.char_indices().peekable();
        let mut end_byte = after_quote.len();
        while let Some((index, ch)) = chars.next() {
            if ch == '\'' {
                if after_quote[index + 1..].starts_with('\'') {
                    result.push('\'');
                    chars.next(); // consume the escaped quote's second char
                    continue;
                }
                end_byte = index + 1;
                break;
            }
            result.push(ch);
        }
        let remainder = after_quote.get(end_byte..).unwrap_or("").trim_start();
        let comment = remainder
            .strip_prefix('/')
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(str::to_string);
        (result.trim_end().to_string(), comment)
    } else {
        let (value, comment) = match trimmed_start.split_once('/') {
            Some((value, comment)) => (value, Some(comment)),
            None => (trimmed_start, None),
        };
        (
            value.trim().to_string(),
            comment
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .map(str::to_string),
        )
    }
}

/// The unit a card's comment states between brackets or parentheses
/// (`[ms] Total Exposure Time`), as a factor to seconds: `Some(0.001)` for
/// milliseconds, `Some(1.0)` for seconds, `None` when the comment says no
/// time unit.
fn time_unit_of_comment(comment: &str) -> Option<f64> {
    let comment = comment.trim_start();
    let inner = comment
        .strip_prefix('[')
        .and_then(|rest| rest.split_once(']'))
        .or_else(|| {
            comment
                .strip_prefix('(')
                .and_then(|rest| rest.split_once(')'))
        })
        .map(|(unit, _)| unit.trim().to_ascii_lowercase())?;
    match inner.as_str() {
        "ms" | "msec" | "millisecond" | "milliseconds" => Some(0.001),
        "s" | "sec" | "second" | "seconds" => Some(1.0),
        _ => None,
    }
}

/// Instruments known to write `EXPOSURE` in milliseconds, whatever the
/// comment says (Vaonis Stellina writes `[ms]`; the table keeps the rule
/// when a firmware drops the comment).
fn instrument_exposure_in_milliseconds(header: &FitsHeader) -> bool {
    header
        .get("INSTRUME")
        .map(|name| name.trim().to_ascii_lowercase())
        .is_some_and(|name| name.starts_with("stellina"))
}

/// The exposure of one frame in seconds. `EXPTIME` is always seconds. Else
/// `EXPOSURE`, whose unit is not fixed: the unit its comment states
/// (`[ms]`, `[s]`), else the unit of a known instrument (Stellina:
/// milliseconds), else seconds, as before.
#[must_use]
pub fn exposure_seconds(header: &FitsHeader) -> Option<f64> {
    if let Some(seconds) = header.get_f64("EXPTIME") {
        return Some(seconds);
    }
    let raw = header.get_f64("EXPOSURE")?;
    let factor = header
        .comment("EXPOSURE")
        .and_then(time_unit_of_comment)
        .unwrap_or(if instrument_exposure_in_milliseconds(header) {
            0.001
        } else {
            1.0
        });
    Some(raw * factor)
}

fn positive(value: Option<f64>) -> Option<f64> {
    value.filter(|v| v.is_finite() && *v > 0.0)
}

/// The size of one pixel in micrometres: `XPIXSZ` (N.I.N.A., MaxIm, with
/// the binning included), else `PIXSZ` (Stellina and others).
#[must_use]
pub fn pixel_size_microns(header: &FitsHeader) -> Option<f64> {
    positive(header.get_f64("XPIXSZ")).or_else(|| positive(header.get_f64("PIXSZ")))
}

/// The focal length in millimetres: `FOCALLEN`, else `FOCAL`.
#[must_use]
pub fn focal_length_mm(header: &FitsHeader) -> Option<f64> {
    positive(header.get_f64("FOCALLEN")).or_else(|| positive(header.get_f64("FOCAL")))
}

/// The recorded (never computed) acquisition metadata this crate extracts
/// from one FITS primary header. Every field is a direct pass-through of a
/// header keyword -- see `contracts/schemas/session-package/metrics.md`
/// for the exact keyword each one comes from and its unit.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FitsAcquisitionRecord {
    pub date_obs_utc: Option<String>,
    pub exposure_seconds: Option<f64>,
    pub filter: Option<String>,
    pub gain: Option<f64>,
    pub iso: Option<f64>,
    pub binning: Option<String>,
    pub sensor_temp_c: Option<f64>,
    /// `SET-TEMP`: the set point of the cooler (only a cooled camera writes
    /// one).
    pub set_temp_c: Option<f64>,
    pub fwhm_pixels: Option<f64>,
    pub eccentricity: Option<f64>,
    pub sky_background_adu: Option<f64>,
    /// The camera of a RAW file: a RAW pose is stacked only with the poses
    /// of the same camera, so it takes part in the family. `None` for a FITS
    /// or XISF frame.
    pub camera: Option<String>,
    /// The `DATE-OBS` is the camera clock as written, its time zone unknown
    /// (a RAW file without `OffsetTimeOriginal` nor GPS time): the night
    /// windows compare it with the wall clock of the night.
    pub date_zone_unknown: bool,
}

/// Normalizes a `DATE-OBS` value to end in `Z`: FITS's `DATE-OBS` is
/// conventionally UTC unless a `TIMESYS` keyword says otherwise (out of
/// scope for v1 -- no capture tool in this project's fleet writes one), so
/// a value with no explicit zone is *assumed* UTC and gets `Z` appended
/// verbatim; a value that already carries an offset or `Z` is returned
/// unchanged. No calendar validation beyond a plausible length: this is a
/// pass-through of a recorded value, not a parse-and-reformat.
fn normalize_date_obs(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.len() < 10 {
        return None; // shorter than "YYYY-MM-DD": not usable as a timestamp
    }
    if trimmed.ends_with('Z') || trimmed.contains('+') || trimmed.matches('-').count() > 2 {
        return Some(trimmed.to_string());
    }
    Some(format!("{trimmed}Z"))
}

/// Extracts every recorded field this crate knows how to read. Missing
/// keywords simply leave the corresponding field `None` -- there is no
/// required keyword, matching "v1 reads only recorded values" (no field is
/// ever synthesized when its header keyword is absent).
#[must_use]
pub fn extract_acquisition_record(header: &FitsHeader) -> FitsAcquisitionRecord {
    let binning = match (header.get_i64("XBINNING"), header.get_i64("YBINNING")) {
        (Some(x), Some(y)) => Some(format!("{x}x{y}")),
        (Some(x), None) => Some(format!("{x}x{x}")),
        _ => None,
    };
    FitsAcquisitionRecord {
        date_obs_utc: header.get("DATE-OBS").and_then(normalize_date_obs),
        exposure_seconds: exposure_seconds(header),
        filter: header.get("FILTER").map(str::to_string),
        gain: header.get_f64("GAIN"),
        iso: header.get_f64("ISO"),
        binning,
        sensor_temp_c: header.get_f64("CCD-TEMP"),
        set_temp_c: header.get_f64("SET-TEMP"),
        fwhm_pixels: header.get_f64("FWHM"),
        eccentricity: header
            .get_f64("ECCENTRICITY")
            .or_else(|| header.get_f64("ECC")),
        sky_background_adu: header
            .get_f64("SKYBG")
            .or_else(|| header.get_f64("BACKGROUND")),
        camera: header
            .get(crate::raw_header::RAW_FILE_CARD)
            .and_then(|_| header.get("INSTRUME"))
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty()),
        date_zone_unknown: header.get(crate::raw_header::TIME_ZONE_CARD)
            == Some(crate::raw_header::TIME_ZONE_UNKNOWN),
    }
}

/// One group of light frames sharing the identical filter, exposure time,
/// gain, binning, and sensor-temperature class -- what is called an
/// "acquisition family" and what `crate::analysis::siril` reuses to
/// pick a homogeneous set of frames for a stacking milestone. `Deserialize`
/// (in addition to `Serialize`) is needed so a whole family can round-trip
/// through `analysis::siril::ProgressionRecord`, which embeds one verbatim.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcquisitionFamily {
    /// Stable, human-readable identifier, e.g. `"Ha_300s_g100_1x1_t-10C"`.
    /// Built from the fields below; never itself a computed "quality"
    /// value, only a label.
    pub key: String,
    pub filter: Option<String>,
    pub exposure_seconds: Option<f64>,
    pub gain: Option<f64>,
    pub binning: Option<String>,
    /// The sensor temperature, in whole degrees Celsius, that every member
    /// of this family shares -- `sensor_temp_c` rounded to the nearest
    /// degree. Rounding (not a named bucket like "cold"/"warm") is the
    /// only transformation applied to the recorded value, precisely so the
    /// grouping stays a transparent equivalence class rather than a
    /// judgment call.
    pub temperature_class_c: Option<i64>,
    /// The camera of a family of RAW poses; `None` for FITS and XISF
    /// frames, whose families never carried one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<String>,
    /// Source paths of every frame folded into this family, in the order
    /// they were supplied -- the traceability the evidence rule requires:
    /// which exact files this family's row(s) in `metrics/quality.csv`
    /// summarize.
    pub frame_paths: Vec<String>,
}

/// Equality/ordering key for grouping: floats are compared via a
/// millidegree/milliarcsecond-scaled integer so two records that agree to
/// three decimal places land in the same family regardless of tiny
/// floating-point noise from unit conversions upstream.
type FamilyKey = (
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<String>,
    Option<i64>,
    Option<String>,
);

fn scale_milli(value: Option<f64>) -> Option<i64> {
    value.map(|v| (v * 1000.0).round() as i64)
}

fn family_key(record: &FitsAcquisitionRecord) -> FamilyKey {
    (
        record.filter.clone(),
        scale_milli(record.exposure_seconds),
        scale_milli(record.gain),
        record.binning.clone(),
        record.sensor_temp_c.map(f64::round).map(|v| v as i64),
        record.camera.clone(),
    )
}

fn format_family_key(record: &FitsAcquisitionRecord, temperature_class_c: Option<i64>) -> String {
    let filter = record.filter.as_deref().unwrap_or("none");
    let exposure = record
        .exposure_seconds
        .map_or_else(|| "unknown".to_string(), |v| format!("{v}s"));
    let gain = record
        .gain
        .map_or_else(|| "unknown".to_string(), |v| format!("g{v}"));
    let binning = record.binning.as_deref().unwrap_or("unknown");
    let temperature =
        temperature_class_c.map_or_else(|| "unknown".to_string(), |v| format!("{v}C"));
    match record.camera.as_deref() {
        Some(camera) => format!("{filter}_{exposure}_{gain}_{binning}_t{temperature}_{camera}"),
        None => format!("{filter}_{exposure}_{gain}_{binning}_t{temperature}"),
    }
}

/// Groups already-extracted FITS acquisition records into acquisition
/// families: identical filter, exposure, gain, binning, and (rounded)
/// sensor temperature. `records` pairs each record with the source path it
/// was extracted from, purely for the family's own traceability list.
/// Families are returned sorted by key, for deterministic output.
#[must_use]
pub fn group_into_families(records: &[(String, FitsAcquisitionRecord)]) -> Vec<AcquisitionFamily> {
    let mut families: Vec<(FamilyKey, AcquisitionFamily)> = Vec::new();
    for (path, record) in records {
        let key = family_key(record);
        if let Some((_, family)) = families.iter_mut().find(|(k, _)| *k == key) {
            family.frame_paths.push(path.clone());
        } else {
            let temperature_class_c = key.4;
            families.push((
                key,
                AcquisitionFamily {
                    key: format_family_key(record, temperature_class_c),
                    filter: record.filter.clone(),
                    exposure_seconds: record.exposure_seconds,
                    gain: record.gain,
                    binning: record.binning.clone(),
                    temperature_class_c,
                    camera: record.camera.clone(),
                    frame_paths: vec![path.clone()],
                },
            ));
        }
    }
    families.sort_by(|a, b| a.1.key.cmp(&b.1.key));
    families.into_iter().map(|(_, family)| family).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal, valid, synthetic FITS primary header from a list
    /// of `(keyword, value)` string cards -- padded to 80 bytes each,
    /// closed with `END`, and padded to a whole 2880-byte block, exactly
    /// as the real format requires. This is the "construct a header inside
    /// the test rather than committing a binary fixture" approach: the
    /// format is entirely mechanical, so there's nothing a committed
    /// binary fixture would show that this builder doesn't already make
    /// explicit.
    fn build_header_bytes(cards: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (keyword, value) in cards {
            let card = format!("{keyword:<8}= {value}");
            let mut bytes = card.into_bytes();
            bytes.resize(CARD_LEN, b' ');
            assert_eq!(bytes.len(), CARD_LEN);
            out.extend_from_slice(&bytes);
        }
        let mut end_card = b"END".to_vec();
        end_card.resize(CARD_LEN, b' ');
        out.extend_from_slice(&end_card);
        let remainder = out.len() % BLOCK_LEN;
        if remainder != 0 {
            out.resize(out.len() + (BLOCK_LEN - remainder), b' ');
        }
        out
    }

    #[test]
    fn parses_a_minimal_synthetic_header() {
        let bytes = build_header_bytes(&[
            ("SIMPLE", "T"),
            ("EXPTIME", "300.0"),
            ("FILTER", "'Ha      '"),
            ("GAIN", "100"),
            ("CCD-TEMP", "-10.2"),
            ("DATE-OBS", "'2026-08-07T22:40:03.500'"),
            ("XBINNING", "1"),
            ("YBINNING", "1"),
        ]);
        assert_eq!(bytes.len() % BLOCK_LEN, 0);

        let header = parse_primary_header(&bytes).unwrap();
        assert_eq!(header.get("SIMPLE"), Some("T"));
        assert_eq!(header.get_f64("EXPTIME"), Some(300.0));
        assert_eq!(header.get("FILTER"), Some("Ha"));
        assert_eq!(header.get_i64("GAIN"), Some(100));
        assert_eq!(header.get_f64("CCD-TEMP"), Some(-10.2));
        assert_eq!(header.get("DATE-OBS"), Some("2026-08-07T22:40:03.500"));

        let record = extract_acquisition_record(&header);
        assert_eq!(record.exposure_seconds, Some(300.0));
        assert_eq!(record.filter.as_deref(), Some("Ha"));
        assert_eq!(record.gain, Some(100.0));
        assert_eq!(record.sensor_temp_c, Some(-10.2));
        assert_eq!(record.binning.as_deref(), Some("1x1"));
        assert_eq!(
            record.date_obs_utc.as_deref(),
            Some("2026-08-07T22:40:03.500Z")
        );
    }

    #[test]
    fn a_date_obs_that_already_carries_a_zone_is_left_untouched() {
        let bytes = build_header_bytes(&[("DATE-OBS", "'2026-08-07T22:40:03Z'")]);
        let header = parse_primary_header(&bytes).unwrap();
        let record = extract_acquisition_record(&header);
        assert_eq!(record.date_obs_utc.as_deref(), Some("2026-08-07T22:40:03Z"));
    }

    #[test]
    fn handles_a_doubled_single_quote_inside_a_string_value() {
        let bytes = build_header_bytes(&[("OBJECT", "'Mira''s target'")]);
        let header = parse_primary_header(&bytes).unwrap();
        assert_eq!(header.get("OBJECT"), Some("Mira's target"));
    }

    #[test]
    fn a_trailing_comment_after_the_value_is_kept_separately() {
        let mut out = Vec::new();
        let card = format!("{:<8}= {:<20}/ a comment", "GAIN", "100");
        let mut bytes = card.into_bytes();
        bytes.resize(CARD_LEN, b' ');
        out.extend_from_slice(&bytes);
        let mut end_card = b"END".to_vec();
        end_card.resize(CARD_LEN, b' ');
        out.extend_from_slice(&end_card);
        out.resize(BLOCK_LEN, b' ');

        let header = parse_primary_header(&out).unwrap();
        let card = header.cards.iter().find(|c| c.keyword == "GAIN").unwrap();
        assert_eq!(card.value.as_deref(), Some("100"));
        assert_eq!(card.comment.as_deref(), Some("a comment"));
    }

    #[test]
    fn a_blank_or_comment_card_has_no_value() {
        let bytes = build_header_bytes(&[("SIMPLE", "T")]);
        // Replace the header's END block padding with a COMMENT card to
        // exercise the no-value branch, then re-close with END.
        let mut out = build_header_bytes(&[("SIMPLE", "T")]);
        out.truncate(CARD_LEN); // keep only the SIMPLE card
        let mut comment_card = b"COMMENT this has no value indicator".to_vec();
        comment_card.resize(CARD_LEN, b' ');
        out.extend_from_slice(&comment_card);
        let mut end_card = b"END".to_vec();
        end_card.resize(CARD_LEN, b' ');
        out.extend_from_slice(&end_card);
        out.resize(BLOCK_LEN, b' ');

        let header = parse_primary_header(&out).unwrap();
        let comment_card = header
            .cards
            .iter()
            .find(|c| c.keyword == "COMMENT")
            .unwrap();
        assert_eq!(comment_card.value, None);
        assert_eq!(
            comment_card.comment.as_deref(),
            Some("this has no value indicator")
        );
        let _ = bytes; // silence unused warning from the first builder call
    }

    #[test]
    fn refuses_a_header_shorter_than_one_card() {
        assert_eq!(
            parse_primary_header(b"short"),
            Err(FitsError::TooShortForACard)
        );
    }

    #[test]
    fn refuses_a_header_with_no_end_card() {
        let mut bytes = vec![b' '; BLOCK_LEN];
        bytes[0..8].copy_from_slice(b"SIMPLE  ");
        assert_eq!(parse_primary_header(&bytes), Err(FitsError::MissingEndCard));
    }

    #[test]
    fn groups_identical_families_and_keeps_distinct_ones_apart() {
        let a = FitsAcquisitionRecord {
            filter: Some("Ha".to_string()),
            exposure_seconds: Some(300.0),
            gain: Some(100.0),
            binning: Some("1x1".to_string()),
            sensor_temp_c: Some(-10.4),
            ..Default::default()
        };
        let b = FitsAcquisitionRecord {
            sensor_temp_c: Some(-9.6), // rounds to the same -10C class as `a`
            ..a.clone()
        };
        let c = FitsAcquisitionRecord {
            filter: Some("OIII".to_string()),
            ..a.clone()
        };
        let records = vec![
            ("frame-0001.fits".to_string(), a),
            ("frame-0002.fits".to_string(), b),
            ("frame-0003.fits".to_string(), c),
        ];

        let families = group_into_families(&records);
        assert_eq!(families.len(), 2);
        let ha_family = families
            .iter()
            .find(|f| f.filter.as_deref() == Some("Ha"))
            .unwrap();
        assert_eq!(
            ha_family.frame_paths,
            vec!["frame-0001.fits", "frame-0002.fits"]
        );
        assert_eq!(ha_family.temperature_class_c, Some(-10));
        let oiii_family = families
            .iter()
            .find(|f| f.filter.as_deref() == Some("OIII"))
            .unwrap();
        assert_eq!(oiii_family.frame_paths, vec!["frame-0003.fits"]);
    }

    #[test]
    fn family_key_is_a_readable_label() {
        let record = FitsAcquisitionRecord {
            filter: Some("Ha".to_string()),
            exposure_seconds: Some(300.0),
            gain: Some(100.0),
            binning: Some("1x1".to_string()),
            sensor_temp_c: Some(-10.4),
            ..Default::default()
        };
        let families = group_into_families(&[("f.fits".to_string(), record)]);
        assert_eq!(families[0].key, "Ha_300s_g100_1x1_t-10C");
    }

    fn header_with_comments(cards: &[(&str, &str, &str)]) -> FitsHeader {
        let mut out = Vec::new();
        for (keyword, value, comment) in cards {
            let card = format!("{keyword:<8}= {value:>20} / {comment}");
            let mut bytes = card.into_bytes();
            bytes.resize(CARD_LEN, b' ');
            out.extend_from_slice(&bytes);
        }
        let mut end_card = b"END".to_vec();
        end_card.resize(CARD_LEN, b' ');
        out.extend_from_slice(&end_card);
        out.resize(BLOCK_LEN, b' ');
        parse_primary_header(&out).unwrap()
    }

    #[test]
    fn an_exposure_in_milliseconds_is_read_in_seconds() {
        // Stellina (Vaonis), as written.
        let header = header_with_comments(&[
            ("INSTRUME", "'stellina-7ba7ce'", "STELLINA by Vaonis"),
            ("EXPOSURE", "10000", "[ms] Total Exposure Time"),
        ]);
        let record = extract_acquisition_record(&header);
        assert_eq!(record.exposure_seconds, Some(10.0));
        let families = group_into_families(&[("a.fits".to_string(), record)]);
        assert!(
            families[0].key.starts_with("none_10s_"),
            "{}",
            families[0].key
        );
    }

    #[test]
    fn an_exposure_without_a_unit_is_in_seconds() {
        let header = header_with_comments(&[("EXPOSURE", "10", "Exposure time")]);
        assert_eq!(exposure_seconds(&header), Some(10.0));
        let bare = header_with_comments(&[("EXPOSURE", "10", "")]);
        assert_eq!(exposure_seconds(&bare), Some(10.0));
        let stated = header_with_comments(&[("EXPOSURE", "10", "[s] Exposure time")]);
        assert_eq!(exposure_seconds(&stated), Some(10.0));
    }

    #[test]
    fn a_known_instrument_gives_the_unit_when_the_comment_does_not() {
        let header = header_with_comments(&[
            ("INSTRUME", "'stellina-7ba7ce'", ""),
            ("EXPOSURE", "10000", ""),
        ]);
        assert_eq!(exposure_seconds(&header), Some(10.0));
        // A unit the comment states wins over the instrument's habit.
        let stated = header_with_comments(&[
            ("INSTRUME", "'stellina-7ba7ce'", ""),
            ("EXPOSURE", "10", "[s] Exposure"),
        ]);
        assert_eq!(exposure_seconds(&stated), Some(10.0));
    }

    #[test]
    fn exptime_is_always_seconds() {
        let header = header_with_comments(&[
            ("EXPTIME", "300.0", ""),
            ("EXPOSURE", "10000", "[ms] Total Exposure Time"),
        ]);
        assert_eq!(exposure_seconds(&header), Some(300.0));
    }

    #[test]
    fn the_pixel_size_and_the_focal_length_have_their_other_names() {
        let stellina = header_with_comments(&[
            ("PIXSZ", "2.4", "[um] Pixel size"),
            ("FOCAL", "400", "[mm] Focal length"),
        ]);
        assert_eq!(pixel_size_microns(&stellina), Some(2.4));
        assert_eq!(focal_length_mm(&stellina), Some(400.0));
        let nina = header_with_comments(&[
            ("XPIXSZ", "3.76", ""),
            ("PIXSZ", "2.4", ""),
            ("FOCALLEN", "800", ""),
            ("FOCAL", "400", ""),
        ]);
        assert_eq!(pixel_size_microns(&nina), Some(3.76));
        assert_eq!(focal_length_mm(&nina), Some(800.0));
        assert_eq!(
            pixel_size_microns(&header_with_comments(&[("GAIN", "1", "")])),
            None
        );
    }
}

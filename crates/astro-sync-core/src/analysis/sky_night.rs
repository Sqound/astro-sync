//! The night's measured sky background, from a folder of light
//! frames.
//!
//! Every light frame of the folder is looked at; the ones whose sky compares
//! with the dark zenith the model of the planner estimates are measured
//! (`analysis::photometry`), and the night's value is their median. What
//! leaves the computer is that one number per night, with the number of
//! poses it is the median of and the method version
//! ([`upload_body`]); the frames never do.
//!
//! ## Which poses count
//!
//! In this order, each refusal named by a stable code ([`PoseVerdict::reason`]):
//!
//! 1. a light frame (`IMAGETYP` absent or naming a light; darks and biases
//!    are read as pedestals, flats ignored) whose pixels can be read;
//! 2. a broadband filter, or none: no `FILTER`, a luminance or clear
//!    filter, a UV/IR cut, or a green / V filter. A narrowband,
//!    light-pollution or unknown filter does not see the sky a sky meter
//!    sees (`filter_not_broadband`);
//! 3. an instant (`DATE-AVG`, else `DATE-OBS` plus half the exposure) and a
//!    place (`SITELAT`/`SITELONG`, `LAT-OBS`/`LONG-OBS`, `OBSGEO-B`/`OBSGEO-L`,
//!    else the place the person gives) -- `no_time`, `no_site`;
//! 4. a solved frame: a `TAN` WCS in the header (`no_wcs`, "unsolved frame": Astro Sync never solves a frame itself);
//! 5. a dark sky: the Sun under [`SUN_MAX_ALTITUDE_DEG`] (`twilight`), the
//!    Moon under [`MOON_MAX_ALTITUDE_DEG`] whatever its phase (`moon_up`),
//!    the frame's centre at least [`MIN_ALTITUDE_DEG`] high (`low_altitude`);
//! 6. a known pedestal (`unknown_pedestal`): the value the person gives,
//!    else the frame's own `PEDESTAL` (a calibrated frame), else 0 for a
//!    frame calibrated with a dark or a bias (`CALSTAT`), else the level of
//!    a dark of the same exposure, gain, binning and temperature (± 3 °C)
//!    found in the folder, else the level of a bias of the same gain and
//!    binning. A pedestal is never guessed: one half the sky too low reads
//!    the sky 0.44 mag too bright;
//! 7. a reference list (`no_reference_stars`) and enough measured stars on
//!    the frame (`too_few_stars`, and the other codes of
//!    `photometry::PoseRefusal`).
//!
//! At most [`MAX_POSES_PER_NIGHT`] of the poses that pass 1 to 6 are
//! measured, evenly spread over the night (`sampled_out` for the others):
//! the median of thirty is as good as the median of three hundred, and
//! reading every pixel of a long night would take minutes.
//!
//! ## The value
//!
//! Each measured pose's sky gets back the extinction its zero point took
//! out of the stars' light, [`EXTINCTION_MAG_PER_AIRMASS`] per airmass,
//! so that it compares with a sky meter's reading. The poses are grouped by
//! night -- the date the evening starts on, from the place's longitude
//! alone (UTC + round(longitude / 15) hours, minus 12 hours: the rule the
//! service applies to a place without a time zone) -- and a night with at
//! least [`MIN_POSES_PER_NIGHT`] measured poses gets the median, to two
//! decimals. A header's own `SKYBG` stays what it is in
//! `metrics/quality.csv`: ADU, never converted.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::AnalysisError;
use super::ephemeris::{airmass, altitude_deg, julian_date, moon_altitude_deg, sun_altitude_deg};
use super::fits::{FitsHeader, extract_acquisition_record, parse_primary_header};
use super::photometry::{ReferenceStarList, measure_pose};
use super::sky::{FitsImage, estimate_background, exposure_seconds, read_fits_image};
use super::wcs::TanWcs;
use crate::scanner::{self, FileClass};
use crate::time_util::{format_iso8601_utc_millis, parse_iso8601_utc_to_unix_seconds};

/// The method version that travels with every value.
pub const SKY_METHOD_VERSION: &str = "astro-sync-sky/1";
/// The Sun must be under this altitude: astronomical night.
pub const SUN_MAX_ALTITUDE_DEG: f64 = -18.0;
/// The Moon must be under this altitude, whatever its phase.
pub const MOON_MAX_ALTITUDE_DEG: f64 = -2.0;
/// The frame's centre must be at least this high (airmass 1.56).
pub const MIN_ALTITUDE_DEG: f64 = 40.0;
/// The extinction added back to each pose, V band, mag per airmass.
pub const EXTINCTION_MAG_PER_AIRMASS: f64 = 0.20;
/// A night gets a value from this many measured poses.
pub const MIN_POSES_PER_NIGHT: usize = 3;
/// At most this many poses are measured per night.
pub const MAX_POSES_PER_NIGHT: usize = 30;

/// A place on Earth, degrees, longitude positive east.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Site {
    pub latitude: f64,
    pub longitude: f64,
}

/// What the person gives the measure.
#[derive(Debug, Clone, Default)]
pub struct SkyOptions {
    /// The reference stars; `None`: no list on this computer.
    pub stars: Option<ReferenceStarList>,
    /// The place, for frames whose header names none.
    pub site: Option<Site>,
    /// The sensor's pedestal per pixel, ADU, when the person knows it.
    pub pedestal_adu: Option<f64>,
}

/// What became of one light frame.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PoseVerdict {
    pub source_path: String,
    /// The middle of the exposure.
    pub observed_at: Option<String>,
    /// Whether its sky counts in its night's median.
    pub kept: bool,
    /// Why not, as a stable code (see the module comment).
    pub reason: Option<String>,
    /// mag/arcsec², extinction added back.
    pub sky_mag_arcsec2: Option<f64>,
    pub stars_measured: Option<usize>,
    pub sun_altitude_deg: Option<f64>,
    pub moon_altitude_deg: Option<f64>,
    pub altitude_deg: Option<f64>,
}

/// One night's value: what goes up to the service.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NightSkyMeasure {
    /// The date the evening starts on, `YYYY-MM-DD`.
    pub night: String,
    /// mag/arcsec², two decimals.
    pub sqm: f64,
    pub pose_count: usize,
    pub method_version: String,
    /// The median instant of the poses, ISO 8601 UTC.
    pub observed_at: String,
    pub site: Site,
}

/// The measure of a folder.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderSkyOutcome {
    /// The nights that have a value, oldest first.
    pub nights: Vec<NightSkyMeasure>,
    /// Every light frame and what became of it, in folder order.
    pub poses: Vec<PoseVerdict>,
    /// Darks and biases read as pedestals.
    pub calibration_frames: usize,
    /// How many stars the reference list holds (0: no list).
    pub reference_stars: usize,
}

/// Parses a latitude or longitude written as a number or as sexagesimal
/// text (`'+45 30 00'`, `'45:30:00'`, `'-30d15m'`).
#[must_use]
pub fn parse_angle(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if let Ok(value) = trimmed.parse::<f64>() {
        return value.is_finite().then_some(value);
    }
    let negative = trimmed.starts_with('-');
    let parts: Vec<f64> = trimmed
        .trim_start_matches(['+', '-'])
        .split(|c: char| c.is_whitespace() || matches!(c, ':' | 'd' | 'm' | 's' | '°' | '\'' | '"'))
        .filter(|part| !part.is_empty())
        .map(str::parse::<f64>)
        .collect::<Result<_, _>>()
        .ok()?;
    let value = match parts.as_slice() {
        [d] => *d,
        [d, m] => d + m / 60.0,
        [d, m, s] => d + m / 60.0 + s / 3600.0,
        _ => return None,
    };
    Some(if negative { -value } else { value })
}

/// The place a frame's header names, if any.
#[must_use]
pub fn header_site(header: &FitsHeader) -> Option<Site> {
    for (lat_key, lon_key) in [
        ("SITELAT", "SITELONG"),
        ("LAT-OBS", "LONG-OBS"),
        ("OBSGEO-B", "OBSGEO-L"),
    ] {
        if let (Some(latitude), Some(longitude)) = (
            header.get(lat_key).and_then(parse_angle),
            header.get(lon_key).and_then(parse_angle),
        ) && (-90.0..=90.0).contains(&latitude)
            && (-180.0..=360.0).contains(&longitude)
        {
            let longitude = if longitude > 180.0 {
                longitude - 360.0
            } else {
                longitude
            };
            return Some(Site {
                latitude,
                longitude,
            });
        }
    }
    None
}

/// The middle of the exposure, Unix seconds.
#[must_use]
pub fn mid_exposure_unix(header: &FitsHeader) -> Option<f64> {
    if let Some(average) = header.get("DATE-AVG") {
        let mut probe = FitsHeader::default();
        probe.cards.push(super::fits::FitsCard {
            keyword: "DATE-OBS".to_string(),
            value: Some(average.to_string()),
            comment: None,
        });
        if let Some(unix) = extract_acquisition_record(&probe)
            .date_obs_utc
            .as_deref()
            .and_then(parse_iso8601_utc_to_unix_seconds)
        {
            return Some(unix);
        }
    }
    let start = extract_acquisition_record(header)
        .date_obs_utc
        .as_deref()
        .and_then(parse_iso8601_utc_to_unix_seconds)?;
    Some(start + exposure_seconds(header).unwrap_or(0.0) / 2.0)
}

/// The night an instant belongs to at a longitude: the date of
/// `UTC + round(longitude / 15) h - 12 h` (the service's rule for a place
/// without a time zone, `longitudeOffsetMinutes` in `astro-core`).
#[must_use]
pub fn night_of(unix_seconds: f64, longitude: f64) -> String {
    let offset_hours = (longitude / 15.0).round().clamp(-12.0, 12.0);
    format_iso8601_utc_millis(unix_seconds + (offset_hours - 12.0) * 3600.0)[..10].to_string()
}

/// What `IMAGETYP` says a frame is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameKind {
    Light,
    Dark,
    Bias,
    Other,
}

fn frame_kind(header: &FitsHeader) -> FrameKind {
    let Some(kind) = header.get("IMAGETYP").or_else(|| header.get("FRAME")) else {
        return FrameKind::Light;
    };
    let kind = kind.to_ascii_lowercase();
    if kind.contains("flat") {
        FrameKind::Other
    } else if kind.contains("dark") {
        FrameKind::Dark
    } else if kind.contains("bias") || kind.contains("offset") || kind.contains("zero") {
        FrameKind::Bias
    } else if kind.contains("light") || kind.contains("object") || kind.contains("science") {
        FrameKind::Light
    } else {
        FrameKind::Other
    }
}

/// Whether a filter name passes the light a sky meter sees (see the module
/// comment): none, luminance, clear, UV/IR cut, green or V.
#[must_use]
pub fn is_broadband_filter(filter: Option<&str>) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    let name: String = filter
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    matches!(
        name.as_str(),
        "" | "l"
            | "lum"
            | "luminance"
            | "clear"
            | "c"
            | "none"
            | "nofilter"
            | "open"
            | "empty"
            | "uvir"
            | "uvircut"
            | "ircut"
            | "uvirblock"
            | "luvir"
            | "g"
            | "green"
            | "v"
    )
}

/// A dark or a bias read as a pedestal.
#[derive(Debug, Clone)]
struct CalibrationFrame {
    kind: FrameKind,
    exposure: Option<f64>,
    gain: Option<f64>,
    binning: Option<String>,
    temperature: Option<f64>,
    /// ADU per sensor pixel.
    level: f64,
}

fn close(a: Option<f64>, b: Option<f64>, tolerance: f64) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => (a - b).abs() <= tolerance,
        _ => true,
    }
}

/// The pedestal of a light frame, per sensor pixel, or `None`.
fn pedestal_for(
    header: &FitsHeader,
    image: Option<&FitsImage>,
    calibration: &[CalibrationFrame],
    options: &SkyOptions,
) -> Option<f64> {
    if let Some(value) = options.pedestal_adu {
        return Some(value);
    }
    if let Some(pedestal) = header.get_f64("PEDESTAL") {
        // PixInsight writes it in 16-bit units even on a frame normalised
        // to [0, 1].
        let normalised = header.get_i64("BITPIX").is_some_and(|bitpix| bitpix < 0)
            && image.is_some_and(|image| {
                image
                    .pixels
                    .iter()
                    .copied()
                    .filter(|v| v.is_finite())
                    .fold(f32::MIN, f32::max)
                    <= 1.0
            });
        return Some(if normalised {
            pedestal / 65535.0
        } else {
            pedestal
        });
    }
    if header
        .get("CALSTAT")
        .is_some_and(|status| status.contains('D') || status.contains('B'))
    {
        return Some(0.0);
    }
    let record = extract_acquisition_record(header);
    let matching = |kind: FrameKind| -> Option<f64> {
        let mut levels: Vec<f64> = calibration
            .iter()
            .filter(|frame| frame.kind == kind)
            .filter(|frame| {
                (kind == FrameKind::Bias
                    || close(
                        frame.exposure,
                        record.exposure_seconds,
                        0.01 * record.exposure_seconds.unwrap_or(0.0).max(0.5),
                    ))
                    && close(frame.gain, record.gain, 0.5)
                    && (frame.binning.is_none()
                        || record.binning.is_none()
                        || frame.binning == record.binning)
                    && (kind == FrameKind::Bias
                        || close(frame.temperature, record.sensor_temp_c, 3.0))
            })
            .map(|frame| frame.level)
            .collect();
        super::sky::median_in_place(&mut levels)
    };
    matching(FrameKind::Dark).or_else(|| matching(FrameKind::Bias))
}

/// A light frame that passed the header checks, waiting for its pixels.
struct Candidate {
    verdict_index: usize,
    bytes_path: std::path::PathBuf,
    unix: f64,
    site: Site,
    altitude: f64,
}

fn refuse(verdict: &mut PoseVerdict, code: &str) {
    verdict.kept = false;
    verdict.reason = Some(code.to_string());
}

/// Measures the sky of every light frame under `root` (read-only) and gives
/// each night's value.
pub fn measure_folder_sky(
    root: &Path,
    options: &SkyOptions,
) -> Result<FolderSkyOutcome, AnalysisError> {
    let scan = scanner::walk_folder(root, None).map_err(AnalysisError::Scan)?;
    let mut lights: Vec<(String, std::path::PathBuf, FitsHeader)> = Vec::new();
    let mut calibration: Vec<CalibrationFrame> = Vec::new();
    for file in scan
        .files
        .iter()
        .filter(|file| file.class == FileClass::Fits)
    {
        let path = root.join(&file.relative_path);
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(header) = parse_primary_header(&bytes) else {
            lights.push((file.relative_path.clone(), path, FitsHeader::default()));
            continue;
        };
        match frame_kind(&header) {
            FrameKind::Light => lights.push((file.relative_path.clone(), path, header)),
            kind @ (FrameKind::Dark | FrameKind::Bias) => {
                if let Ok(image) = read_fits_image(&bytes)
                    && let Some(background) = estimate_background(&image)
                {
                    let record = extract_acquisition_record(&header);
                    calibration.push(CalibrationFrame {
                        kind,
                        exposure: record.exposure_seconds,
                        gain: record.gain,
                        binning: record.binning,
                        temperature: record.sensor_temp_c,
                        level: background.level / (image.superpixel * image.superpixel) as f64,
                    });
                }
            }
            FrameKind::Other => {}
        }
    }

    let mut verdicts: Vec<PoseVerdict> = Vec::new();
    let mut candidates: Vec<Candidate> = Vec::new();
    for (relative_path, path, header) in lights {
        let mut verdict = PoseVerdict {
            source_path: relative_path,
            observed_at: None,
            kept: false,
            reason: None,
            sky_mag_arcsec2: None,
            stars_measured: None,
            sun_altitude_deg: None,
            moon_altitude_deg: None,
            altitude_deg: None,
        };
        let index = verdicts.len();
        if header.cards.is_empty() {
            refuse(&mut verdict, "unreadable");
            verdicts.push(verdict);
            continue;
        }
        let record = extract_acquisition_record(&header);
        let unix = mid_exposure_unix(&header);
        verdict.observed_at = unix.map(format_iso8601_utc_millis);
        let site = header_site(&header).or(options.site);
        let wcs = TanWcs::from_header(&header);
        let reason = if !is_broadband_filter(record.filter.as_deref()) {
            Some("filter_not_broadband")
        } else if unix.is_none() {
            Some("no_time")
        } else if site.is_none() {
            Some("no_site")
        } else if wcs.is_none() {
            Some("no_wcs")
        } else {
            None
        };
        if let Some(code) = reason {
            refuse(&mut verdict, code);
            verdicts.push(verdict);
            continue;
        }
        let (unix, site, wcs) = (
            unix.expect("checked"),
            site.expect("checked"),
            wcs.expect("checked"),
        );
        let sun = sun_altitude_deg(unix, site.latitude, site.longitude);
        let moon = moon_altitude_deg(unix, site.latitude, site.longitude);
        let altitude = altitude_deg(
            wcs.reference_ra_deg,
            wcs.reference_dec_deg,
            julian_date(unix),
            site.latitude,
            site.longitude,
        );
        verdict.sun_altitude_deg = Some(round_to(sun, 2));
        verdict.moon_altitude_deg = Some(round_to(moon, 2));
        verdict.altitude_deg = Some(round_to(altitude, 2));
        let reason = if sun >= SUN_MAX_ALTITUDE_DEG {
            Some("twilight")
        } else if moon >= MOON_MAX_ALTITUDE_DEG {
            Some("moon_up")
        } else if altitude < MIN_ALTITUDE_DEG {
            Some("low_altitude")
        } else {
            None
        };
        if let Some(code) = reason {
            refuse(&mut verdict, code);
            verdicts.push(verdict);
            continue;
        }
        verdicts.push(verdict);
        candidates.push(Candidate {
            verdict_index: index,
            bytes_path: path,
            unix,
            site,
            altitude,
        });
    }

    // At most MAX_POSES_PER_NIGHT per night, evenly spread.
    let mut by_night: BTreeMap<String, Vec<Candidate>> = BTreeMap::new();
    for candidate in candidates {
        by_night
            .entry(night_of(candidate.unix, candidate.site.longitude))
            .or_default()
            .push(candidate);
    }
    let mut nights = Vec::new();
    for (night, mut group) in by_night {
        group.sort_by(|a, b| a.unix.total_cmp(&b.unix));
        let total = group.len();
        let chosen: Vec<bool> = (0..total)
            .map(|i| {
                total <= MAX_POSES_PER_NIGHT
                    || (0..MAX_POSES_PER_NIGHT)
                        .any(|k| k * (total - 1) / (MAX_POSES_PER_NIGHT - 1) == i)
            })
            .collect();
        let mut values: Vec<(f64, f64, Site)> = Vec::new();
        for (candidate, take) in group.iter().zip(chosen) {
            let verdict = &mut verdicts[candidate.verdict_index];
            if !take {
                refuse(verdict, "sampled_out");
                continue;
            }
            let Ok(bytes) = std::fs::read(&candidate.bytes_path) else {
                refuse(verdict, "unreadable");
                continue;
            };
            let image = match read_fits_image(&bytes) {
                Ok(image) => image,
                Err(_) => {
                    refuse(verdict, "unreadable");
                    continue;
                }
            };
            let Some(pedestal) = pedestal_for(&image.header, Some(&image), &calibration, options)
            else {
                refuse(verdict, "unknown_pedestal");
                continue;
            };
            let Some(stars) = options.stars.as_ref().filter(|stars| !stars.is_empty()) else {
                refuse(verdict, "no_reference_stars");
                continue;
            };
            match measure_pose(&image, stars, pedestal) {
                Ok(pose) => {
                    let sky = pose.sky_mag_arcsec2
                        + EXTINCTION_MAG_PER_AIRMASS * airmass(candidate.altitude);
                    verdict.kept = true;
                    verdict.reason = None;
                    verdict.sky_mag_arcsec2 = Some(round_to(sky, 3));
                    verdict.stars_measured = Some(pose.stars_measured);
                    values.push((sky, candidate.unix, candidate.site));
                }
                Err(refusal) => {
                    refuse(verdict, refusal.code());
                    verdict.stars_measured = match refusal {
                        super::photometry::PoseRefusal::TooFewStars { measured, .. } => {
                            Some(measured)
                        }
                        _ => None,
                    };
                }
            }
        }
        if values.len() < MIN_POSES_PER_NIGHT {
            continue;
        }
        let mut skies: Vec<f64> = values.iter().map(|(sky, _, _)| *sky).collect();
        let sqm = super::sky::median_in_place(&mut skies).expect("not empty");
        let middle = &values[(values.len() - 1) / 2]; // sorted by time
        nights.push(NightSkyMeasure {
            night,
            sqm: round_to(sqm, 2),
            pose_count: values.len(),
            method_version: SKY_METHOD_VERSION.to_string(),
            observed_at: format_iso8601_utc_millis(middle.1),
            site: Site {
                latitude: round_to(middle.2.latitude, 5),
                longitude: round_to(middle.2.longitude, 5),
            },
        });
    }
    Ok(FolderSkyOutcome {
        nights,
        poses: verdicts,
        calibration_frames: calibration.len(),
        reference_stars: options.stars.as_ref().map_or(0, ReferenceStarList::len),
    })
}

/// Every refusal code of [`PoseVerdict::reason`], with its English
/// sentence: what the command line prints (the window writes its own, in
/// the language of the person).
pub const POSE_REASONS: [(&str, &str); 15] = [
    ("unreadable", "unreadable file"),
    ("filter_not_broadband", "narrow-band or unrecognised filter"),
    ("no_time", "frame time missing from the header"),
    ("no_site", "site position missing from the header"),
    (
        "no_wcs",
        "unsolved frame (no astrometric solution in the header)",
    ),
    ("twilight", "Sun too high (twilight or day)"),
    ("moon_up", "Moon up"),
    ("low_altitude", "target below 40° altitude"),
    (
        "unknown_pedestal",
        "sensor offset unknown: add a dark or a bias to the folder",
    ),
    (
        "no_reference_stars",
        "reference star list missing from this computer",
    ),
    ("too_few_stars", "fewer than 3 measurable reference stars"),
    ("no_exposure", "exposure time missing from the header"),
    ("no_background", "sky background cannot be estimated"),
    (
        "sky_below_pedestal",
        "sky background below the sensor offset",
    ),
    (
        "sampled_out",
        "not measured: 30 frames are enough for a night",
    ),
];

/// The English sentence of a pose's refusal code.
#[must_use]
pub fn reason_sentence(code: &str) -> Option<&'static str> {
    POSE_REASONS
        .iter()
        .find(|(known, _)| *known == code)
        .map(|(_, sentence)| *sentence)
}

fn round_to(value: f64, decimals: i32) -> f64 {
    let factor = 10f64.powi(decimals);
    (value * factor).round() / factor
}

/// The body of `POST /v1/place-sky-measures`: the night's one number, the
/// number of poses, the method version, the median instant and the place of
/// the frames (the service resolves the observer's place and the night from
/// the session's plan when there is one, else from these). Nothing else.
#[must_use]
pub fn upload_body(measure: &NightSkyMeasure, session_id: Option<&str>) -> serde_json::Value {
    let mut body = json!({
        "observedAt": measure.observed_at,
        "sqm": measure.sqm,
        "poseCount": measure.pose_count,
        "methodVersion": measure.method_version,
        "site": { "latitude": measure.site.latitude, "longitude": measure.site.longitude },
    });
    if let Some(session_id) = session_id.map(str::trim).filter(|id| !id.is_empty()) {
        body["sessionId"] = json!(session_id);
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::photometry::tests::{CENTRE, Truth, solved_frame};
    use crate::analysis::sky_synthetic::{SyntheticFrame, quoted};
    use crate::test_support::TempDir;

    // The night of 2026-10-10 at 45° N, 30° W (the captures' place, in the
    // Atlantic): at 05:00 UTC the Sun is at -33°, the Moon at -42°, the
    // frame's centre (RA 83.82°, Dec +22.01°) 62° high; at 23:30 UTC it is
    // 8° high; at 19:30 UTC the Sun is at -2°; on the 15th at 21:00 UTC the
    // Moon is 4° up.
    const SITE: Site = Site {
        latitude: 45.0,
        longitude: -30.0,
    };

    fn site_cards(date_obs: &str) -> Vec<(&'static str, String)> {
        vec![
            ("DATE-OBS", quoted(date_obs)),
            ("SITELAT", "45.0".into()),
            ("SITELONG", "-30.0".into()),
            ("IMAGETYP", quoted("Light Frame")),
            ("GAIN", "100".into()),
            ("CCD-TEMP", "-10.0".into()),
        ]
    }

    fn reference() -> ReferenceStarList {
        solved_frame(Truth::default(), &[]).1
    }

    #[test]
    fn the_sun_the_moon_and_the_altitude_select_the_poses() {
        let dir = TempDir::new("sky-selection");
        // Dark and high: kept.
        for (name, date) in [
            ("dark-1.fits", "2026-10-11T05:00:00"),
            ("dark-2.fits", "2026-10-11T05:10:00"),
            ("dark-3.fits", "2026-10-11T05:20:00"),
            ("twilight.fits", "2026-10-10T19:30:00"),
            ("moon.fits", "2026-10-15T21:00:00"),
            ("low.fits", "2026-10-10T23:30:00"),
        ] {
            let (bytes, _) = solved_frame(Truth::default(), &site_cards(date));
            std::fs::write(dir.path().join(name), bytes).unwrap();
        }
        let options = SkyOptions {
            stars: Some(reference()),
            pedestal_adu: Some(500.0),
            ..SkyOptions::default()
        };
        let outcome = measure_folder_sky(dir.path(), &options).unwrap();
        let reason = |name: &str| {
            outcome
                .poses
                .iter()
                .find(|pose| pose.source_path == name)
                .unwrap()
                .reason
                .clone()
        };
        assert_eq!(reason("dark-1.fits"), None);
        assert_eq!(reason("twilight.fits").as_deref(), Some("twilight"));
        assert_eq!(reason("moon.fits").as_deref(), Some("moon_up"));
        assert_eq!(reason("low.fits").as_deref(), Some("low_altitude"));
        assert_eq!(outcome.nights.len(), 1);
        let night = &outcome.nights[0];
        assert_eq!(night.night, "2026-10-10");
        assert_eq!(night.pose_count, 3);
        assert_eq!(night.observed_at, "2026-10-11T05:11:00.000Z");
        assert_eq!(night.method_version, SKY_METHOD_VERSION);
        // The synthetic sky is 20.80 without atmosphere; the extinction
        // added back for an airmass of ~1.13 makes it 0.23 darker.
        let pose = outcome
            .poses
            .iter()
            .find(|p| p.source_path == "dark-1.fits")
            .unwrap();
        let x = airmass(pose.altitude_deg.unwrap());
        assert!(
            (night.sqm - (20.8 + EXTINCTION_MAG_PER_AIRMASS * x)).abs() <= 0.02,
            "{}",
            night.sqm
        );
    }

    #[test]
    fn unsolved_narrowband_or_placeless_poses_give_no_value_and_say_why() {
        let dir = TempDir::new("sky-refusals");
        let mut cards = site_cards("2026-10-11T05:00:00");
        cards.push(("FILTER", quoted("Ha 7nm")));
        let (bytes, _) = solved_frame(Truth::default(), &cards);
        std::fs::write(dir.path().join("ha.fits"), bytes).unwrap();
        // Never solved: no WCS at all.
        let mut frame = SyntheticFrame::new(64, 64, 300.0);
        frame.seed = 3;
        let unsolved = frame.to_fits(&[
            ("DATE-OBS", quoted("2026-10-11T05:00:00")),
            ("EXPTIME", "120.0".into()),
            ("SITELAT", "45.0".into()),
            ("SITELONG", "-30.0".into()),
        ]);
        std::fs::write(dir.path().join("asiair.fits"), unsolved).unwrap();
        let placeless = frame.to_fits(&[("DATE-OBS", quoted("2026-10-11T05:00:00"))]);
        std::fs::write(dir.path().join("nowhere.fits"), placeless).unwrap();
        let outcome = measure_folder_sky(
            dir.path(),
            &SkyOptions {
                stars: Some(reference()),
                pedestal_adu: Some(500.0),
                ..SkyOptions::default()
            },
        )
        .unwrap();
        let reasons: BTreeMap<_, _> = outcome
            .poses
            .iter()
            .map(|pose| (pose.source_path.as_str(), pose.reason.as_deref()))
            .collect();
        assert_eq!(reasons["ha.fits"], Some("filter_not_broadband"));
        assert_eq!(reasons["asiair.fits"], Some("no_wcs"));
        assert_eq!(reasons["nowhere.fits"], Some("no_site"));
        assert!(outcome.nights.is_empty(), "no value is guessed");
        assert!(is_broadband_filter(Some("Lum")) && is_broadband_filter(Some("UV/IR Cut")));
        assert!(!is_broadband_filter(Some("L-eXtreme")) && !is_broadband_filter(Some("OIII")));
    }

    #[test]
    fn the_pedestal_comes_from_a_dark_or_a_bias_and_is_never_guessed() {
        let dir = TempDir::new("sky-pedestal");
        for (index, minute) in [0, 10, 20].iter().enumerate() {
            let (bytes, _) = solved_frame(
                Truth {
                    seed: 20 + index as u64,
                    ..Truth::default()
                },
                &[
                    site_cards(&format!("2026-10-11T05:{minute:02}:00")),
                    vec![("EXPOSURE", "120.0".into())],
                ]
                .concat(),
            );
            std::fs::write(dir.path().join(format!("light-{index}.fits")), bytes).unwrap();
        }
        let stars = Some(reference());
        let without = measure_folder_sky(
            dir.path(),
            &SkyOptions {
                stars: stars.clone(),
                ..SkyOptions::default()
            },
        )
        .unwrap();
        assert!(without.nights.is_empty());
        assert!(
            without
                .poses
                .iter()
                .all(|pose| pose.reason.as_deref() == Some("unknown_pedestal"))
        );

        // A dark of another exposure does not count; a bias of the same gain does.
        let mut dark = SyntheticFrame::new(64, 64, 0.0);
        dark.pedestal_adu = 500.0;
        let other_dark = dark.to_fits(&[
            ("IMAGETYP", quoted("Dark Frame")),
            ("EXPTIME", "300.0".into()),
            ("GAIN", "100".into()),
        ]);
        std::fs::write(dir.path().join("dark-300.fits"), other_dark).unwrap();
        let still = measure_folder_sky(
            dir.path(),
            &SkyOptions {
                stars: stars.clone(),
                ..SkyOptions::default()
            },
        )
        .unwrap();
        assert_eq!(still.calibration_frames, 1);
        assert!(still.nights.is_empty());
        let bias = dark.to_fits(&[("IMAGETYP", quoted("Bias Frame")), ("GAIN", "100".into())]);
        std::fs::write(dir.path().join("bias.fits"), bias).unwrap();
        let with = measure_folder_sky(
            dir.path(),
            &SkyOptions {
                stars,
                ..SkyOptions::default()
            },
        )
        .unwrap();
        assert_eq!(with.nights.len(), 1);
        assert_eq!(with.nights[0].pose_count, 3);
        let x = airmass(with.poses[0].altitude_deg.unwrap());
        assert!((with.nights[0].sqm - (20.8 + EXTINCTION_MAG_PER_AIRMASS * x)).abs() <= 0.02);
    }

    #[test]
    fn without_a_reference_list_nothing_is_measured() {
        let dir = TempDir::new("sky-no-list");
        let (bytes, _) = solved_frame(Truth::default(), &site_cards("2026-10-11T05:00:00"));
        std::fs::write(dir.path().join("light.fits"), bytes).unwrap();
        let outcome = measure_folder_sky(
            dir.path(),
            &SkyOptions {
                pedestal_adu: Some(500.0),
                ..SkyOptions::default()
            },
        )
        .unwrap();
        assert_eq!(outcome.reference_stars, 0);
        assert_eq!(
            outcome.poses[0].reason.as_deref(),
            Some("no_reference_stars")
        );
    }

    #[test]
    fn the_night_is_read_from_the_longitude_and_the_angles_from_the_header() {
        // 02:00 UTC at 30° W (UTC-2) is midnight local: the night of the 10th.
        let unix = parse_iso8601_utc_to_unix_seconds("2026-10-11T02:00:00Z").unwrap();
        assert_eq!(night_of(unix, -30.0), "2026-10-10");
        // 14:00 UTC is 16:00 local at 30° E: already the evening of the 11th.
        let unix = parse_iso8601_utc_to_unix_seconds("2026-10-11T14:00:00Z").unwrap();
        assert_eq!(night_of(unix, 30.0), "2026-10-11");
        // 23:00 local at 139.7° E (UTC+9) on the 1st: night of the 1st.
        let unix = parse_iso8601_utc_to_unix_seconds("2026-01-01T14:00:00Z").unwrap();
        assert_eq!(night_of(unix, 139.7), "2026-01-01");
        assert_eq!(parse_angle("+45 30 00"), Some(45.5));
        assert_eq!(parse_angle("-30:15:00"), Some(-30.25));
        assert_eq!(parse_angle("12.5"), Some(12.5));
        assert_eq!(parse_angle("north"), None);
    }

    #[test]
    fn the_night_is_the_median_of_its_poses() {
        // Three poses whose skies differ: the median is the middle one, not
        // the mean.
        let dir = TempDir::new("sky-median");
        for (index, (sky, minute)) in [(20.6, 0), (20.9, 10), (21.6, 20)].iter().enumerate() {
            let (bytes, _) = solved_frame(
                Truth {
                    sky: *sky,
                    seed: 40 + index as u64,
                    ..Truth::default()
                },
                &site_cards(&format!("2026-10-11T05:{minute:02}:00")),
            );
            std::fs::write(dir.path().join(format!("p{index}.fits")), bytes).unwrap();
        }
        let outcome = measure_folder_sky(
            dir.path(),
            &SkyOptions {
                stars: Some(reference()),
                pedestal_adu: Some(500.0),
                ..SkyOptions::default()
            },
        )
        .unwrap();
        let night = &outcome.nights[0];
        let middle = outcome
            .poses
            .iter()
            .find(|p| p.source_path == "p1.fits")
            .unwrap();
        assert!((night.sqm - round_to(middle.sky_mag_arcsec2.unwrap(), 2)).abs() < 1e-9);
        assert!(
            night.sqm < 21.3,
            "a mean would be pulled up by the 21.6 pose"
        );
    }

    #[test]
    fn the_upload_body_carries_one_number_per_night_and_nothing_else() {
        let measure = NightSkyMeasure {
            night: "2026-10-10".into(),
            sqm: 21.04,
            pose_count: 14,
            method_version: SKY_METHOD_VERSION.into(),
            observed_at: "2026-10-11T02:11:00.000Z".into(),
            site: SITE,
        };
        let body = upload_body(&measure, Some(" 0b9f3c1e-6d0a-4f5e-9a3b-2c1d0e9f8a7b "));
        assert_eq!(
            body,
            json!({
                "sessionId": "0b9f3c1e-6d0a-4f5e-9a3b-2c1d0e9f8a7b",
                "observedAt": "2026-10-11T02:11:00.000Z",
                "sqm": 21.04,
                "poseCount": 14,
                "methodVersion": "astro-sync-sky/1",
                "site": { "latitude": 45.0, "longitude": -30.0 }
            })
        );
        assert!(upload_body(&measure, Some("  ")).get("sessionId").is_none());
        assert!(upload_body(&measure, None).get("sessionId").is_none());
    }

    /// The evidence rule of `analysis` (no `score`, `rating` or `grade`),
    /// checked on what this measure renders: the outcome and the upload body.
    #[test]
    fn the_sky_measure_never_produces_a_composite_score() {
        let dir = TempDir::new("sky-no-score");
        for minute in [0, 10, 20] {
            let (bytes, _) = solved_frame(
                Truth {
                    seed: 60 + minute,
                    ..Truth::default()
                },
                &site_cards(&format!("2026-10-11T05:{minute:02}:00")),
            );
            std::fs::write(dir.path().join(format!("p{minute}.fits")), bytes).unwrap();
        }
        let outcome = measure_folder_sky(
            dir.path(),
            &SkyOptions {
                stars: Some(reference()),
                pedestal_adu: Some(500.0),
                ..SkyOptions::default()
            },
        )
        .unwrap();
        let rendered = [
            serde_json::to_value(&outcome).unwrap(),
            upload_body(&outcome.nights[0], Some("session")),
        ];
        fn keys(value: &serde_json::Value, out: &mut Vec<String>) {
            match value {
                serde_json::Value::Object(map) => {
                    for (key, nested) in map {
                        out.push(key.to_ascii_lowercase());
                        keys(nested, out);
                    }
                }
                serde_json::Value::Array(items) => items.iter().for_each(|item| keys(item, out)),
                _ => {}
            }
        }
        let mut all = Vec::new();
        rendered.iter().for_each(|value| keys(value, &mut all));
        assert!(all.contains(&"sqm".to_string()));
        for key in all {
            for forbidden in ["score", "rating", "grade"] {
                assert!(!key.contains(forbidden), "{key}");
            }
        }
        let _ = CENTRE;
    }

    /// Every reason a pose is set aside has its sentence (the command line
    /// says it). The window no longer lists the poses one by one since the
    /// "Local analysis" screen left it: the night's value is computed with
    /// the analysis of the logs and sent with it.
    #[test]
    fn every_refusal_has_a_sentence() {
        let codes = [
            "unreadable",
            "filter_not_broadband",
            "no_time",
            "no_site",
            "no_wcs",
            "twilight",
            "moon_up",
            "low_altitude",
            "unknown_pedestal",
            "no_reference_stars",
            "sampled_out",
        ];
        for code in codes {
            assert!(reason_sentence(code).is_some(), "{code}");
        }
        for refusal in [
            crate::analysis::photometry::PoseRefusal::Unreadable(String::new()),
            crate::analysis::photometry::PoseRefusal::NoWcs,
            crate::analysis::photometry::PoseRefusal::NoExposure,
            crate::analysis::photometry::PoseRefusal::NoBackground,
            crate::analysis::photometry::PoseRefusal::TooFewStars {
                measured: 0,
                in_field: 0,
            },
            crate::analysis::photometry::PoseRefusal::SkyBelowPedestal,
        ] {
            let code = refusal.code();
            assert!(reason_sentence(code).is_some(), "{code}");
        }
    }
}

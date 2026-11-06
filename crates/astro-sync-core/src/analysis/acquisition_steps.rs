//! The acquisition steps of a night, from the headers of its light frames
//! (`acquisitionSteps[]` of the report v2).
//!
//! A step is the light frames of one target sharing a filter, an exposure
//! and a gain; binning and sensor temperature are kept (never shown). The
//! target is the header's `OBJECT`, or, for a frame that names none, the
//! target NINA was imaging at its `DATE-OBS` (its `target-started` window):
//! that is the `targetHint` the service resolves to a target of the space.
//! Each step has a key unique within the report, and every quality row says
//! its step by that key. Calibration frames make no step.
//!
//! A night with no light frame header but a NINA log gets its steps from the
//! frames NINA started (same grouping), so a night whose frames stayed on
//! another disk still has its steps.

use serde::Serialize;
use std::collections::BTreeMap;

use super::fits::{FitsAcquisitionRecord, FitsHeader};
use super::nina::NinaExposure;
use crate::detection::normalize_object;

/// One step of the report (`session-analysis.schema.json`,
/// `acquisitionStep`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcquisitionStep {
    pub key: String,
    pub target_hint: Option<String>,
    pub filter: Option<String>,
    pub frame_count: u64,
    pub exposure_seconds: f64,
    pub gain: Option<i64>,
    pub binning: Option<String>,
    pub sensor_temp_c: Option<f64>,
}

/// A light frame as the grouping reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct StepFrame {
    /// Relative to the night's folder; what a quality row points back to.
    pub path: String,
    pub target_hint: Option<String>,
    pub record: FitsAcquisitionRecord,
}

/// The pixel scale of a frame in arcseconds per pixel: `PIXSCALE` or
/// `SCALE` when the header states it, else `206.265 × XPIXSZ or PIXSZ (µm) /
/// FOCALLEN or FOCAL (mm)` (the pixel size as written, binning included).
#[must_use]
pub fn header_pixel_scale(header: &FitsHeader) -> Option<f64> {
    let positive = |value: Option<f64>| value.filter(|v| v.is_finite() && *v > 0.0);
    positive(header.get_f64("PIXSCALE"))
        .or_else(|| positive(header.get_f64("SCALE")))
        .or_else(|| {
            let pixel_microns = positive(super::fits::pixel_size_microns(header))?;
            let focal_mm = positive(super::fits::focal_length_mm(header))?;
            Some(206.265 * pixel_microns / focal_mm)
        })
}

/// The scale of the night: the median of its frames' scales, rounded to the
/// hundredth; `None` when no frame gives one.
#[must_use]
pub fn night_pixel_scale(scales: &[f64]) -> Option<f64> {
    median(scales).map(|scale| (scale * 100.0).round() / 100.0)
}

fn median(values: &[f64]) -> Option<f64> {
    let mut sorted: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    Some(if sorted.len().is_multiple_of(2) {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    } else {
        sorted[middle]
    })
}

/// The grouping key: target (compared without case or spaces), filter (as
/// written), exposure to the millisecond, gain.
type GroupKey = (String, Option<String>, i64, Option<i64>);

fn group_key(
    target: Option<&str>,
    filter: Option<&str>,
    exposure: f64,
    gain: Option<i64>,
) -> GroupKey {
    (
        target.map(normalize_object).unwrap_or_default(),
        filter.map(str::to_string),
        (exposure * 1000.0).round() as i64,
        gain,
    )
}

/// `ngc7000-ha-180s-g100`: readable, ASCII, at most 200 characters.
fn step_slug(
    target: Option<&str>,
    filter: Option<&str>,
    exposure: f64,
    gain: Option<i64>,
) -> String {
    let clean = |text: &str| -> String {
        let mut out = String::new();
        for c in text.chars() {
            if c.is_ascii_alphanumeric() {
                out.push(c.to_ascii_lowercase());
            } else if !out.ends_with('-') && !out.is_empty() {
                out.push('-');
            }
        }
        out.trim_end_matches('-').to_string()
    };
    let mut parts = Vec::new();
    if let Some(target) = target.map(clean).filter(|t| !t.is_empty()) {
        parts.push(target);
    }
    parts.push(
        filter
            .map(clean)
            .filter(|f| !f.is_empty())
            .unwrap_or_else(|| "sans-filtre".to_string()),
    );
    let seconds = if exposure.fract().abs() < 1e-9 {
        format!("{}s", exposure as i64)
    } else {
        format!(
            "{}s",
            format!("{exposure:.3}")
                .trim_end_matches('0')
                .replace('.', "p")
        )
    };
    parts.push(seconds);
    if let Some(gain) = gain {
        parts.push(format!("g{gain}"));
    }
    let mut slug = parts.join("-");
    slug.truncate(190);
    slug
}

struct Building {
    target_hint: Option<String>,
    filter: Option<String>,
    exposure: f64,
    gain: Option<i64>,
    frames: u64,
    binnings: BTreeMap<String, u64>,
    temperatures: Vec<f64>,
}

fn finish(groups: Vec<Building>) -> (Vec<AcquisitionStep>, Vec<String>) {
    let mut used: BTreeMap<String, u32> = BTreeMap::new();
    let mut keys = Vec::new();
    let mut steps = Vec::new();
    for group in groups {
        let base = step_slug(
            group.target_hint.as_deref(),
            group.filter.as_deref(),
            group.exposure,
            group.gain,
        );
        let count = used.entry(base.clone()).or_insert(0);
        *count += 1;
        let key = if *count == 1 {
            base
        } else {
            format!("{base}-{count}")
        };
        keys.push(key.clone());
        let binning = group
            .binnings
            .iter()
            .max_by_key(|(_, n)| **n)
            .map(|(b, _)| b.clone());
        steps.push(AcquisitionStep {
            key,
            target_hint: group.target_hint.map(|t| t.chars().take(200).collect()),
            filter: group.filter.map(|f| f.chars().take(40).collect()),
            frame_count: group.frames,
            exposure_seconds: group.exposure,
            gain: group.gain,
            binning: binning.map(|b| b.chars().take(20).collect()),
            sensor_temp_c: median(&group.temperatures).map(|t| (t * 10.0).round() / 10.0),
        });
    }
    (steps, keys)
}

/// Groups the light frames into steps. Returns the steps (ordered by their
/// first frame's date, then path) and, for each frame of `frames` in order,
/// the key of its step (`None` for a frame without an exposure time).
#[must_use]
pub fn steps_from_frames(frames: &[StepFrame]) -> (Vec<AcquisitionStep>, Vec<Option<String>>) {
    let mut order: Vec<usize> = (0..frames.len()).collect();
    order.sort_by(|a, b| {
        let fa = &frames[*a];
        let fb = &frames[*b];
        fa.record
            .date_obs_utc
            .cmp(&fb.record.date_obs_utc)
            .then(fa.path.cmp(&fb.path))
    });
    let mut index_of: BTreeMap<GroupKey, usize> = BTreeMap::new();
    let mut groups: Vec<Building> = Vec::new();
    let mut group_of_frame: Vec<Option<usize>> = vec![None; frames.len()];
    for i in order {
        let frame = &frames[i];
        let Some(exposure) = frame.record.exposure_seconds.filter(|e| *e >= 0.001) else {
            continue;
        };
        let gain = frame.record.gain.map(|g| g.round() as i64);
        let filter = frame
            .record
            .filter
            .as_deref()
            .map(str::trim)
            .filter(|f| !f.is_empty());
        let key = group_key(frame.target_hint.as_deref(), filter, exposure, gain);
        let group = *index_of.entry(key).or_insert_with(|| {
            groups.push(Building {
                target_hint: frame.target_hint.clone(),
                filter: filter.map(str::to_string),
                exposure,
                gain,
                frames: 0,
                binnings: BTreeMap::new(),
                temperatures: Vec::new(),
            });
            groups.len() - 1
        });
        let building = &mut groups[group];
        building.frames += 1;
        if let Some(binning) = &frame.record.binning {
            *building.binnings.entry(binning.clone()).or_insert(0) += 1;
        }
        if let Some(temperature) = frame.record.sensor_temp_c {
            building.temperatures.push(temperature);
        }
        group_of_frame[i] = Some(group);
    }
    let (steps, keys) = finish(groups);
    let frame_keys = group_of_frame
        .into_iter()
        .map(|group| group.map(|g| keys[g].clone()))
        .collect();
    (steps, frame_keys)
}

/// The steps of a night known only from NINA's log (no light frame header).
#[must_use]
pub fn steps_from_nina(exposures: &[NinaExposure]) -> Vec<AcquisitionStep> {
    let frames: Vec<StepFrame> = exposures
        .iter()
        .enumerate()
        .map(|(i, exposure)| StepFrame {
            path: format!("nina:{i:06}"),
            target_hint: exposure.target.clone(),
            record: FitsAcquisitionRecord {
                date_obs_utc: Some(exposure.timestamp_utc.clone()),
                exposure_seconds: Some(exposure.exposure_seconds),
                filter: exposure.filter.clone(),
                gain: exposure.gain.map(|g| g as f64),
                ..FitsAcquisitionRecord::default()
            },
        })
        .collect();
    steps_from_frames(&frames).0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::fits::parse_primary_header;

    fn frame(
        path: &str,
        target: Option<&str>,
        filter: Option<&str>,
        exposure: f64,
        at: &str,
    ) -> StepFrame {
        StepFrame {
            path: path.into(),
            target_hint: target.map(str::to_string),
            record: FitsAcquisitionRecord {
                date_obs_utc: Some(at.into()),
                exposure_seconds: Some(exposure),
                filter: filter.map(str::to_string),
                gain: Some(100.0),
                binning: Some("1x1".into()),
                sensor_temp_c: Some(-10.0),
                ..FitsAcquisitionRecord::default()
            },
        }
    }

    #[test]
    fn frames_group_by_target_filter_exposure_and_gain() {
        let frames = vec![
            frame(
                "NGC 7000/lights/a.fits",
                Some("NGC 7000"),
                Some("Ha"),
                180.0,
                "2026-10-07T21:10:00Z",
            ),
            frame(
                "NGC 7000/lights/b.fits",
                Some("NGC7000"),
                Some("Ha"),
                180.0,
                "2026-10-07T21:13:00Z",
            ),
            frame(
                "NGC 7000/lights/c.fits",
                Some("NGC 7000"),
                Some("OIII"),
                180.0,
                "2026-10-07T22:00:00Z",
            ),
            frame(
                "M31/lights/d.fits",
                Some("M 31"),
                None,
                120.0,
                "2026-10-07T23:00:00Z",
            ),
            frame(
                "M31/lights/e.fits",
                Some("M 31"),
                None,
                120.5,
                "2026-10-07T23:03:00Z",
            ),
        ];
        let (steps, keys) = steps_from_frames(&frames);
        let summary: Vec<(&str, u64)> = steps
            .iter()
            .map(|s| (s.key.as_str(), s.frame_count))
            .collect();
        assert_eq!(
            summary,
            [
                ("ngc-7000-ha-180s-g100", 2),
                ("ngc-7000-oiii-180s-g100", 1),
                ("m-31-sans-filtre-120s-g100", 1),
                ("m-31-sans-filtre-120p5s-g100", 1)
            ]
        );
        assert_eq!(keys[1].as_deref(), Some("ngc-7000-ha-180s-g100"));
        assert_eq!(steps[0].target_hint.as_deref(), Some("NGC 7000"));
        assert_eq!(steps[0].binning.as_deref(), Some("1x1"));
        assert_eq!(steps[0].sensor_temp_c, Some(-10.0));
        assert_eq!(steps[2].filter, None);
    }

    #[test]
    fn a_frame_without_exposure_has_no_step() {
        let mut without = frame("x.fits", None, Some("L"), 1.0, "2026-10-07T21:00:00Z");
        without.record.exposure_seconds = None;
        let (steps, keys) = steps_from_frames(&[without]);
        assert!(steps.is_empty());
        assert_eq!(keys, [None]);
    }

    #[test]
    fn the_pixel_scale_comes_from_the_header() {
        let build = |cards: &[(&str, &str)]| {
            let mut bytes = Vec::new();
            for (k, v) in cards {
                let mut card = format!("{k:<8}= {v}").into_bytes();
                card.resize(80, b' ');
                bytes.extend(card);
            }
            let mut end = b"END".to_vec();
            end.resize(80, b' ');
            bytes.extend(end);
            bytes.resize(2880, b' ');
            parse_primary_header(&bytes).unwrap()
        };
        assert_eq!(
            header_pixel_scale(&build(&[("PIXSCALE", "1.5")])),
            Some(1.5)
        );
        let derived =
            header_pixel_scale(&build(&[("XPIXSZ", "3.76"), ("FOCALLEN", "400")])).unwrap();
        assert!((derived - 1.9389).abs() < 1e-3);
        assert_eq!(header_pixel_scale(&build(&[("FOCALLEN", "400")])), None);
        assert_eq!(night_pixel_scale(&[1.94, 1.93, 3.1]), Some(1.94));
        assert_eq!(night_pixel_scale(&[]), None);
    }
}

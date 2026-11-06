//! The camera's clock, checked against the sky (« Wrong camera clock »): the
//! Sun above the horizon at `DATE-OBS` for lights, the target below it,
//! `OBJCTALT`/`OBJCTAZ` or `LST` that disagree with the ones computed from
//! `DATE-OBS`. When the clock is suspect, the shift of whole hours that makes
//! the night consistent is proposed (ticked), applied to the session only,
//! never to the files.

use serde::{Deserialize, Serialize};

use crate::analysis::ephemeris::{
    altitude_deg, greenwich_sidereal_deg, julian_date, sun_altitude_deg,
};

use super::group::iso;

/// The furthest shift tried, hours (the extremes of the world's clocks).
const MAX_SHIFT_HOURS: i32 = 14;
/// Header and computed altitudes further apart than this disagree.
const ALTITUDE_SLACK_DEG: f64 = 5.0;
/// Header and computed sidereal times further apart than this disagree.
const LST_SLACK_HOURS: f64 = 0.25;

/// One light, as the check needs it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockInput {
    /// The middle of the exposure, Unix seconds, as written.
    pub mid: f64,
    /// The header's coordinates, else the target's, degrees.
    pub position: Option<(f64, f64)>,
    pub altitude_deg: Option<f64>,
    pub azimuth_deg: Option<f64>,
    pub lst_hours: Option<f64>,
}

/// Where the check was made.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "from")]
pub enum PlaceUsed {
    /// `SITELAT`/`SITELONG`.
    #[default]
    Header,
    /// The frames name no place: the default place of "My places".
    #[serde(rename_all = "camelCase")]
    DefaultPlace { place_id: String },
    /// No place at all: nothing could be checked.
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClockReasonCode {
    SunUp,
    TargetBelowHorizon,
    AltitudeDisagrees,
    LstDisagrees,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClockReason {
    pub code: ClockReasonCode,
    pub frames: usize,
}

/// The sky at the first light, as written and shifted: the computation the
/// review screen (and the evidence) shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClockSample {
    pub shift_hours: i32,
    pub utc: String,
    pub sun_altitude_deg: f64,
    pub target_altitude_deg: Option<f64>,
}

/// What the check found for a session.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClockFinding {
    pub place: PlaceUsed,
    pub frames_checked: usize,
    /// The clock looks wrong.
    pub suspect: bool,
    pub reasons: Vec<ClockReason>,
    /// Consistent lights as written and after the proposed shift.
    pub consistent_as_written: usize,
    pub consistent_after_shift: usize,
    /// Whole hours to add to `DATE-OBS`; ticked by default when present.
    pub proposed_shift_hours: Option<i32>,
    pub samples: Vec<ClockSample>,
}

struct Verdict {
    consistent: usize,
    /// Consistent, and in full night: the Sun below −12°, the target above
    /// 10° (what a shift is chosen on: twilight is possible, not likely).
    dark: usize,
    sun_up: usize,
    below: usize,
    altitude: usize,
    lst: usize,
}

fn local_sidereal_hours(unix: f64, longitude: f64) -> f64 {
    ((greenwich_sidereal_deg(julian_date(unix)) + longitude) / 15.0).rem_euclid(24.0)
}

fn hours_apart(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(24.0);
    d.min(24.0 - d)
}

fn judge(inputs: &[ClockInput], site: (f64, f64), shift_hours: i32) -> Verdict {
    let shift = f64::from(shift_hours) * 3600.0;
    let mut verdict = Verdict {
        consistent: 0,
        dark: 0,
        sun_up: 0,
        below: 0,
        altitude: 0,
        lst: 0,
    };
    for input in inputs {
        let t = input.mid + shift;
        let sun = sun_altitude_deg(t, site.0, site.1);
        let mut good = sun < 0.0;
        let mut dark = sun < -12.0;
        if !good {
            verdict.sun_up += 1;
        }
        if let Some((ra, dec)) = input.position {
            let altitude = altitude_deg(ra, dec, julian_date(t), site.0, site.1);
            dark &= altitude > 10.0;
            if altitude < 0.0 {
                verdict.below += 1;
                good = false;
            }
            if let Some(written) = input.altitude_deg
                && (written - altitude).abs() > ALTITUDE_SLACK_DEG
            {
                verdict.altitude += 1;
                good = false;
            }
        }
        if let Some(lst) = input.lst_hours
            && hours_apart(lst, local_sidereal_hours(t, site.1)) > LST_SLACK_HOURS
        {
            verdict.lst += 1;
            good = false;
        }
        if good {
            verdict.consistent += 1;
            if dark {
                verdict.dark += 1;
            }
        }
    }
    verdict
}

fn sample(input: &ClockInput, site: (f64, f64), shift_hours: i32) -> ClockSample {
    let t = input.mid + f64::from(shift_hours) * 3600.0;
    let round = |v: f64| (v * 10.0).round() / 10.0;
    ClockSample {
        shift_hours,
        utc: iso(t),
        sun_altitude_deg: round(sun_altitude_deg(t, site.0, site.1)),
        target_altitude_deg: input
            .position
            .map(|(ra, dec)| round(altitude_deg(ra, dec, julian_date(t), site.0, site.1))),
    }
}

/// The finding of a session whose frames name no place and with no default
/// place: the check waits for the place.
#[must_use]
pub fn without_place() -> ClockFinding {
    ClockFinding {
        place: PlaceUsed::None,
        ..ClockFinding::default()
    }
}

/// Checks the lights of a session at a place (`(latitude, longitude)`).
/// `None` when there is no light to check.
#[must_use]
pub fn evaluate(inputs: &[ClockInput], site: (f64, f64), place: PlaceUsed) -> Option<ClockFinding> {
    let first = inputs.iter().min_by(|a, b| a.mid.total_cmp(&b.mid))?;
    let n = inputs.len();
    let written = judge(inputs, site, 0);
    let mut reasons = Vec::new();
    for (code, frames) in [
        (ClockReasonCode::SunUp, written.sun_up),
        (ClockReasonCode::TargetBelowHorizon, written.below),
        (ClockReasonCode::AltitudeDisagrees, written.altitude),
        (ClockReasonCode::LstDisagrees, written.lst),
    ] {
        if frames * 2 > n {
            reasons.push(ClockReason { code, frames });
        }
    }
    let suspect = !reasons.is_empty();
    let mut finding = ClockFinding {
        place,
        frames_checked: n,
        suspect,
        reasons,
        consistent_as_written: written.consistent,
        consistent_after_shift: written.consistent,
        proposed_shift_hours: None,
        samples: vec![sample(first, site, 0)],
    };
    if !suspect {
        return Some(finding);
    }
    // The shift that puts the most lights in full night, then the most
    // consistent ones, the nearest to zero on a tie; it must make most of
    // them consistent.
    let mut best: Option<(i32, (usize, usize))> = None;
    for step in 1..=MAX_SHIFT_HOURS {
        for shift in [step, -step] {
            let verdict = judge(inputs, site, shift);
            let score = (verdict.dark, verdict.consistent);
            if best.is_none_or(|(_, kept)| score > kept) {
                best = Some((shift, score));
            }
        }
    }
    if let Some((shift, (_, consistent))) = best
        && consistent > written.consistent
        && consistent * 2 > n
    {
        finding.proposed_shift_hours = Some(shift);
        finding.consistent_after_shift = consistent;
        finding.samples.push(sample(first, site, shift));
    }
    Some(finding)
}

//! A series of poses: the light frames of one night and one target taken with
//! the same filter, exposure, gain and binning, stacked together and never
//! with another exposure. The service keeps one capture per session, target
//! and series (`capture.schema.json` `series`, `uploads.md`); Astro Sync sends
//! the series with each image it stacked, so the images of one series go to
//! the same capture and a second series of the same night and target to a
//! capture of its own. The rules mirror the service's
//! (`packages/portal/src/captures/capture-series.js`).

use serde::{Deserialize, Serialize};

/// `uploadMetadata.series`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Series {
    /// The filter the frames name; `None`: no filter.
    #[serde(default)]
    pub filter: Option<String>,
    /// One frame's exposure, seconds.
    pub exposure_seconds: f64,
    #[serde(default)]
    pub gain: Option<i64>,
    /// 1 for 1×1.
    #[serde(default = "one")]
    pub binning: u32,
    /// The frames the series holds, when known.
    #[serde(default)]
    pub frames: Option<u64>,
    /// Its total exposure, seconds, when known.
    #[serde(default)]
    pub total_seconds: Option<f64>,
}

const fn one() -> u32 {
    1
}

// Every number is finite (read from a header or a record, filtered): the
// equality is total.
impl Eq for Series {}

fn plain(value: f64) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    if (rounded - rounded.trunc()).abs() < f64::EPSILON {
        format!("{}", rounded as i64)
    } else {
        format!("{rounded}")
    }
}

impl Series {
    /// The key of the series (filter without case, exposure, gain, binning),
    /// the service's `seriesKey`: `IRCUT|30|200|1`.
    #[must_use]
    pub fn key(&self) -> String {
        let filter = self
            .filter
            .as_deref()
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .map_or_else(|| "-".to_string(), str::to_uppercase);
        let gain = self.gain.map_or_else(|| "-".to_string(), |g| g.to_string());
        format!(
            "{filter}|{}|{gain}|{}",
            plain(self.exposure_seconds),
            self.binning.max(1)
        )
    }

    /// The total seconds: what it says, else frames × exposure, else 0.
    #[must_use]
    pub fn seconds(&self) -> f64 {
        match (self.total_seconds, self.frames) {
            (Some(total), _) if total.is_finite() && total >= 0.0 => total,
            (_, Some(frames)) => frames as f64 * self.exposure_seconds,
            _ => 0.0,
        }
    }

    /// The series a family label names (`IRCUT_30s_g200`,
    /// `Ha_300s_g100_bin2`, the stem of a render
    /// `IRCUT_30s_g200_raw_strong.png`), with the total when known; `None`
    /// when the label names no exposure and gain.
    #[must_use]
    pub fn from_label(label: &str, total_seconds: Option<f64>) -> Option<Self> {
        let base = label.rsplit(['/', '\\']).next().unwrap_or(label);
        // The extension of a file name goes; a decimal exposure keeps its dot.
        let stem = base
            .rsplit_once('.')
            .filter(|(_, ext)| !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphabetic()))
            .map_or(base, |(stem, _)| stem);
        let parts: Vec<&str> = stem.split('_').collect();
        // `<filter>_<exposure>s_g<gain>[_bin<n>]…`, the filter possibly holding
        // a dash or a dot-free name.
        let exposure_at = parts.iter().position(|p| {
            p.strip_suffix('s')
                .is_some_and(|n| !n.is_empty() && n.parse::<f64>().is_ok())
        })?;
        if exposure_at == 0 {
            return None;
        }
        let exposure: f64 = parts[exposure_at].strip_suffix('s')?.parse().ok()?;
        let after = exposure_at + 1;
        let gain: i64 = parts
            .get(after)?
            .strip_prefix('g')
            .and_then(|g| g.parse().ok())?;
        let binning = parts
            .get(after + 1)
            .and_then(|p| p.strip_prefix("bin"))
            .and_then(|b| b.split('x').next())
            .and_then(|b| b.parse().ok())
            .unwrap_or(1);
        let filter = parts[..exposure_at].join("_");
        if exposure.is_nan() || exposure <= 0.0 {
            return None;
        }
        let frames = total_seconds
            .filter(|t| t.is_finite() && *t > 0.0)
            .map(|t| (t / exposure).round() as u64)
            .filter(|f| *f > 0);
        Some(Self {
            filter: (filter != "nofilter").then_some(filter),
            exposure_seconds: exposure,
            gain: Some(gain),
            binning,
            frames,
            total_seconds: total_seconds.filter(|t| t.is_finite() && *t >= 0.0),
        })
    }

    /// The series an image's file name tells (`M45_2026-10-10_IRCUT_30s_g200_raw.png`:
    /// the word before the exposure is the filter, a date there says none),
    /// the service's `seriesFromImageName`. A name tells its filter only
    /// roughly: compare with [`Self::same_named`].
    #[must_use]
    pub fn from_file_name(name: &str) -> Option<Self> {
        let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
        let stem = base
            .rsplit_once('.')
            .filter(|(_, ext)| !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphabetic()))
            .map_or(base, |(stem, _)| stem);
        let tokens: Vec<&str> = stem.split('_').collect();
        let at = (0..tokens.len()).find(|&i| {
            tokens[i]
                .strip_suffix('s')
                .is_some_and(|n| !n.is_empty() && n.parse::<f64>().is_ok())
                && tokens
                    .get(i + 1)
                    .and_then(|g| g.strip_prefix('g'))
                    .is_some_and(|g| g.parse::<i64>().is_ok())
        })?;
        let exposure: f64 = tokens[at].strip_suffix('s')?.parse().ok()?;
        let gain: i64 = tokens[at + 1].strip_prefix('g')?.parse().ok()?;
        let binning = tokens
            .get(at + 2)
            .and_then(|p| p.strip_prefix("bin"))
            .and_then(|b| b.split('x').next())
            .and_then(|b| b.parse().ok())
            .unwrap_or(1);
        let before = at.checked_sub(1).map(|i| tokens[i]);
        let is_date = |t: &str| {
            t.len() == 10
                && t.chars().enumerate().all(|(i, c)| {
                    if i == 4 || i == 7 {
                        c == '-'
                    } else {
                        c.is_ascii_digit()
                    }
                })
        };
        let filter = before
            .filter(|t| !t.is_empty() && !is_date(t) && *t != "nofilter")
            .map(str::to_string);
        (exposure > 0.0).then_some(Self {
            filter,
            exposure_seconds: exposure,
            gain: Some(gain),
            binning,
            frames: None,
            total_seconds: None,
        })
    }

    /// Whether a series read from a file name is this one: exposure, gain and
    /// binning.
    #[must_use]
    pub fn same_named(&self, named: &Self) -> bool {
        (self.exposure_seconds - named.exposure_seconds).abs() < 1e-6
            && self.gain == named.gain
            && self.binning.max(1) == named.binning.max(1)
    }

    /// The series of a master the stacking integrated.
    #[must_use]
    pub fn of_master(master: &crate::multi_night::IntegrationMaster) -> Option<Self> {
        let exposure = master
            .exposure_seconds
            .filter(|e| e.is_finite() && *e > 0.0)?;
        Some(Self {
            filter: master.filter.clone(),
            exposure_seconds: exposure,
            gain: master
                .gain
                .filter(|g| g.is_finite() && (g - g.round()).abs() < 1e-6)
                .map(|g| g.round() as i64),
            binning: master.binning.unwrap_or(1).max(1),
            frames: (master.frame_count > 0).then_some(master.frame_count),
            total_seconds: master
                .total_seconds
                .is_finite()
                .then_some(master.total_seconds.max(0.0)),
        })
    }

    /// Whether an acquisition line is of this series (filter without case,
    /// exposure, gain when both know it).
    #[must_use]
    pub fn holds_line(&self, line: &crate::metadata::AcquisitionLine) -> bool {
        let same_filter = line.filter.as_deref().map(|f| f.trim().to_uppercase())
            == self.filter.as_deref().map(|f| f.trim().to_uppercase());
        same_filter
            && (line.exposure_seconds - self.exposure_seconds).abs() < 1e-6
            && match (line.gain, self.gain) {
                (Some(a), Some(b)) => a == b,
                _ => true,
            }
    }
}

/// Whether a series is minor beside another of the same target and filter:
/// less than a tenth of its total time (decision 4 of CAPTURES-SERIES-1).
/// Such a series is left unticked by default; the person may tick it.
pub const MINOR_SERIES_SHARE: f64 = 0.1;

/// Whether `series` is minor among `others` (the other series of the same
/// target): less than [`MINOR_SERIES_SHARE`] of the time of another series of
/// the same filter.
#[must_use]
pub fn is_minor(series: &Series, others: &[&Series]) -> bool {
    let mine = series.seconds();
    let filter = series.filter.as_deref().map(|f| f.trim().to_uppercase());
    others.iter().any(|other| {
        other.key() != series.key()
            && other.filter.as_deref().map(|f| f.trim().to_uppercase()) == filter
            && mine < MINOR_SERIES_SHARE * other.seconds()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn long() -> Series {
        Series {
            filter: Some("IRCUT".into()),
            exposure_seconds: 30.0,
            gain: Some(200),
            binning: 1,
            frames: Some(429),
            total_seconds: Some(12870.0),
        }
    }

    #[test]
    fn the_key_is_the_services() {
        assert_eq!(long().key(), "IRCUT|30|200|1");
        let lower = Series {
            filter: Some("ircut".into()),
            frames: Some(430),
            ..long()
        };
        assert_eq!(lower.key(), long().key());
        let none = Series {
            filter: None,
            gain: None,
            exposure_seconds: 0.5,
            ..long()
        };
        assert_eq!(none.key(), "-|0.5|-|1");
    }

    #[test]
    fn a_family_label_names_its_series() {
        let s = Series::from_label("IRCUT_30s_g200", Some(12870.0)).unwrap();
        assert_eq!(s.key(), "IRCUT|30|200|1");
        assert_eq!(s.frames, Some(429));
        let b = Series::from_label("Ha_300s_g100_bin2_polished_strong.png", None).unwrap();
        assert_eq!(b.key(), "HA|300|100|2");
        assert_eq!(b.frames, None);
        let n = Series::from_label("nofilter_10s_g120", None).unwrap();
        assert_eq!(n.filter, None);
        assert!(Series::from_label("M31_final", None).is_none());
        assert!(Series::from_label("30s_g200", None).is_none());
        let named = Series::from_file_name("M45_2026-10-10_IRCUT_30s_g200_raw_strong.png").unwrap();
        assert_eq!(named.key(), "IRCUT|30|200|1");
        assert!(long().same_named(&named));
        let dated = Series::from_file_name("M45_2026-10-10_30s_g200.png").unwrap();
        assert_eq!(dated.filter, None);
        let half = Series::from_label("L_0.5s_g90.png", None).unwrap();
        assert_eq!(half.key(), "L|0.5|90|1");
    }

    #[test]
    fn a_series_under_a_tenth_of_another_of_its_filter_is_minor() {
        let short = Series {
            exposure_seconds: 10.0,
            frames: Some(36),
            total_seconds: Some(360.0),
            ..long()
        };
        let l = long();
        assert!(is_minor(&short, &[&l]));
        assert!(!is_minor(&l, &[&short]));
        let other_filter = Series {
            filter: Some("Ha".into()),
            ..short.clone()
        };
        assert!(!is_minor(&other_filter, &[&l]));
    }
}

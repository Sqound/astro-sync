//! The zero point of a frame, measured on the frame itself, and the sky
//! background in mag/arcsec² it gives.
//!
//! A frame's background in ADU says nothing about the sky until the same
//! frame says how many ADU a star of known magnitude gives. Here, the stars
//! of a reference list ([`ReferenceStarList`]) that fall on a solved frame
//! (its `TAN` WCS, `analysis::wcs`) are found on the pixels, re-centred on
//! their own light, and measured through a circular aperture with the local
//! background of an annulus subtracted. Each star gives one zero point,
//! `m + 2.5 log10(flux / exposure)` -- the magnitude of a source that gives
//! 1 ADU per second --, and the frame's zero point is their median. The sky
//! background is then
//!
//! ```text
//! sky = ZP - 2.5 log10((background - pedestal) / (exposure * scale²))
//! ```
//!
//! in mag/arcsec², `scale` being the image pixel in arcseconds. The
//! extinction of the stars' light is in the zero point; `analysis::sky_night`
//! adds it back for the airmass of the frame, so that the value compares
//! with a sky meter's.
//!
//! What is never measured: a star touching a saturated pixel, a star too
//! faint for a clean aperture (signal-to-noise under 20), a star with
//! another reference star of comparable brightness in its annulus, a star
//! whose aperture leaves the frame. Fewer than [`MIN_REFERENCE_STARS`]
//! stars left, no value. No WCS in the header, no value: nothing is guessed.
//!
//! ## The reference list
//!
//! A compact binary file (`ASRSTAR1`, [`ReferenceStarList::to_bytes`]) made
//! from the Hipparcos main catalogue (ESA 1997, CDS I/239, `hip_main.dat`)
//! or the Tycho-2 catalogue (ESA 2000, CDS I/259, `tyc2.dat`) by
//! `astro-sync reference-stars`; neither is a share-alike licence, and both
//! are credited where the value is shown. The list is not in the
//! repository: a measure without one says so.

use std::fmt;
use std::path::Path;

use super::sky::{
    FitsImage, estimate_background, exposure_seconds, median_in_place, robust_sigma,
    sensor_pixel_scale, sigma_clipped, sky_rate,
};
use super::wcs::{TanWcs, angular_distance_deg};

/// A frame gives no value with fewer measured reference stars than this.
pub const MIN_REFERENCE_STARS: usize = 3;
/// A star is measured only above this signal-to-noise ratio (aperture flux
/// over the background noise of the aperture's area).
const MIN_STAR_SNR: f64 = 20.0;
/// A star must peak at least this many background sigmas above the sky to
/// be found at all.
const MIN_PEAK_SIGMAS: f64 = 10.0;
/// How far from its catalogue position a star is looked for, in
/// arcseconds (a solution's error and the stars' proper motions); never
/// fewer than 4 image pixels.
const SEARCH_RADIUS_ARCSEC: f64 = 8.0;

/// One star of the reference list: J2000 position and V magnitude.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReferenceStar {
    pub ra_deg: f64,
    pub dec_deg: f64,
    pub v_mag: f64,
}

/// Why a reference list could not be read.
#[derive(Debug, PartialEq, Eq)]
pub enum StarListError {
    NotAStarList,
    Truncated,
    Io(String),
}

impl fmt::Display for StarListError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StarListError::NotAStarList => write!(f, "not a reference star list (ASRSTAR1)"),
            StarListError::Truncated => write!(f, "truncated reference star list"),
            StarListError::Io(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for StarListError {}

/// The reference stars, sorted by declination for a quick look-up.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReferenceStarList {
    stars: Vec<ReferenceStar>,
}

const STAR_LIST_MAGIC: &[u8; 8] = b"ASRSTAR1";
const STAR_RECORD_LEN: usize = 10;

impl ReferenceStarList {
    #[must_use]
    pub fn new(mut stars: Vec<ReferenceStar>) -> Self {
        stars.retain(|star| {
            star.ra_deg.is_finite() && star.dec_deg.is_finite() && star.v_mag.is_finite()
        });
        stars.sort_by(|a, b| a.dec_deg.total_cmp(&b.dec_deg));
        Self { stars }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.stars.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stars.is_empty()
    }

    /// The file: `ASRSTAR1`, a little-endian `u32` count, then ten bytes per
    /// star -- right ascension and declination as little-endian `f32`
    /// degrees (0.1″ at worst), V magnitude as a little-endian `i16` in
    /// millimagnitudes.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(12 + self.stars.len() * STAR_RECORD_LEN);
        out.extend_from_slice(STAR_LIST_MAGIC);
        out.extend_from_slice(&(self.stars.len() as u32).to_le_bytes());
        for star in &self.stars {
            out.extend_from_slice(&(star.ra_deg as f32).to_le_bytes());
            out.extend_from_slice(&(star.dec_deg as f32).to_le_bytes());
            let millimag = (star.v_mag * 1000.0).round().clamp(-32768.0, 32767.0) as i16;
            out.extend_from_slice(&millimag.to_le_bytes());
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, StarListError> {
        if bytes.len() < 12 || &bytes[..8] != STAR_LIST_MAGIC {
            return Err(StarListError::NotAStarList);
        }
        let count = u32::from_le_bytes(bytes[8..12].try_into().expect("four bytes")) as usize;
        let body = &bytes[12..];
        if body.len() < count * STAR_RECORD_LEN {
            return Err(StarListError::Truncated);
        }
        let stars = body
            .chunks_exact(STAR_RECORD_LEN)
            .take(count)
            .map(|record| ReferenceStar {
                ra_deg: f64::from(f32::from_le_bytes(record[0..4].try_into().expect("4"))),
                dec_deg: f64::from(f32::from_le_bytes(record[4..8].try_into().expect("4"))),
                v_mag: f64::from(i16::from_le_bytes(record[8..10].try_into().expect("2"))) / 1000.0,
            })
            .collect();
        Ok(Self::new(stars))
    }

    pub fn read(path: &Path) -> Result<Self, StarListError> {
        let bytes = std::fs::read(path)
            .map_err(|error| StarListError::Io(format!("{}: {error}", path.display())))?;
        Self::from_bytes(&bytes)
    }

    /// The stars within `radius_deg` of `(ra, dec)`.
    #[must_use]
    pub fn within(&self, ra_deg: f64, dec_deg: f64, radius_deg: f64) -> Vec<ReferenceStar> {
        let low = dec_deg - radius_deg;
        let start = self.stars.partition_point(|star| star.dec_deg < low);
        self.stars[start..]
            .iter()
            .take_while(|star| star.dec_deg <= dec_deg + radius_deg)
            .filter(|star| {
                angular_distance_deg(ra_deg, dec_deg, star.ra_deg, star.dec_deg) <= radius_deg
            })
            .copied()
            .collect()
    }
}

/// Moves a position by its proper motion: `pm_ra` is μα·cos δ, both in
/// milliarcseconds per year, over `years`.
fn propagate(ra_deg: f64, dec_deg: f64, pm_ra: f64, pm_dec: f64, years: f64) -> (f64, f64) {
    let dec = dec_deg + pm_dec * years / 3.6e6;
    let cos_dec = dec_deg.to_radians().cos().max(1e-6);
    let ra = ra_deg + pm_ra * years / (3.6e6 * cos_dec);
    (ra.rem_euclid(360.0), dec)
}

fn field(fields: &[&str], index: usize) -> Option<f64> {
    fields.get(index)?.trim().parse::<f64>().ok()
}

/// Stars of the Hipparcos main catalogue (`hip_main.dat`, CDS I/239: fields
/// separated by `|`; H5 `Vmag`, H8 `RAdeg`, H9 `DEdeg` at epoch J1991.25,
/// H12 `pmRA`, H13 `pmDE` in mas/yr), moved to J2000 by their proper
/// motion. A line without a position or a magnitude is skipped.
#[must_use]
pub fn parse_hipparcos_main(text: &str) -> Vec<ReferenceStar> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('|').collect();
            let v_mag = field(&fields, 5)?;
            let ra = field(&fields, 8)?;
            let dec = field(&fields, 9)?;
            let pm_ra = field(&fields, 12).unwrap_or(0.0);
            let pm_dec = field(&fields, 13).unwrap_or(0.0);
            let (ra_deg, dec_deg) = propagate(ra, dec, pm_ra, pm_dec, 2000.0 - 1991.25);
            Some(ReferenceStar {
                ra_deg,
                dec_deg,
                v_mag,
            })
        })
        .collect()
}

/// The Johnson V magnitude of a Tycho-2 star: the catalogue's own
/// conversion, `VT - 0.090 (BT - VT)`, or `VT` alone when `BT` is missing.
#[must_use]
pub fn tycho2_v_mag(vt: f64, bt: Option<f64>) -> f64 {
    match bt {
        Some(bt) => vt - 0.090 * (bt - vt),
        None => vt,
    }
}

/// Stars of the Tycho-2 catalogue (`tyc2.dat`, CDS I/259, fields separated
/// by `|`: `mRAdeg`, `mDEdeg` -- the mean J2000 position -- at 2 and 3,
/// `BTmag` at 17, `VTmag` at 19, the observed `RAdeg`, `DEdeg` at 24 and 25
/// for a star without a mean position). V is the catalogue's own
/// conversion, `VT - 0.090 (BT - VT)`, or `VT` alone when `BT` is missing; a
/// star without `VT` is skipped.
#[must_use]
pub fn parse_tycho2(text: &str) -> Vec<ReferenceStar> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('|').collect();
            let vt = field(&fields, 19)?;
            let v_mag = tycho2_v_mag(vt, field(&fields, 17));
            let (ra_deg, dec_deg) = match (field(&fields, 2), field(&fields, 3)) {
                (Some(ra), Some(dec)) => (ra, dec),
                _ => (field(&fields, 24)?, field(&fields, 25)?),
            };
            Some(ReferenceStar {
                ra_deg,
                dec_deg,
                v_mag,
            })
        })
        .collect()
}

/// Why a frame gives no sky value. [`PoseRefusal::code`] is what the report
/// and the interface read.
#[derive(Debug, Clone, PartialEq)]
pub enum PoseRefusal {
    /// The pixels could not be read (a compressed file, a cube...).
    Unreadable(String),
    /// No `TAN` WCS in the header: "unsolved frame".
    NoWcs,
    /// No exposure time in the header.
    NoExposure,
    /// No background could be estimated.
    NoBackground,
    /// Fewer than [`MIN_REFERENCE_STARS`] reference stars could be measured.
    TooFewStars { measured: usize, in_field: usize },
    /// The background is not above the pedestal.
    SkyBelowPedestal,
}

impl PoseRefusal {
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            PoseRefusal::Unreadable(_) => "unreadable",
            PoseRefusal::NoWcs => "no_wcs",
            PoseRefusal::NoExposure => "no_exposure",
            PoseRefusal::NoBackground => "no_background",
            PoseRefusal::TooFewStars { .. } => "too_few_stars",
            PoseRefusal::SkyBelowPedestal => "sky_below_pedestal",
        }
    }
}

/// One frame's sky, before the extinction and the selection of
/// `analysis::sky_night`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PoseSky {
    /// mag/arcsec², the extinction of the zero point not yet added back.
    pub sky_mag_arcsec2: f64,
    /// The magnitude of a source giving 1 ADU per second.
    pub zero_point: f64,
    /// Robust standard deviation of the stars' own zero points.
    pub zero_point_spread: f64,
    pub stars_measured: usize,
    /// ADU per image pixel over the exposure, pedestal included.
    pub background_adu: f64,
    /// ADU the sensor adds to each of its pixels.
    pub pedestal_adu: f64,
    pub image_arcsec_per_pixel: f64,
    pub exposure_seconds: f64,
    /// The centre of the frame on the sky (J2000, degrees).
    pub centre_ra_deg: f64,
    pub centre_dec_deg: f64,
}

/// A star found on the frame near its catalogue position.
#[derive(Debug, Clone, Copy)]
struct FoundStar {
    x: f64,
    y: f64,
    sigma: f64,
    v_mag: f64,
}

/// Looks for the light of a star near `(x, y)` (0-based image coordinates)
/// and re-centres on it: the brightest unsaturated pixel of the search box,
/// then three rounds of a weighted centroid. Its Gaussian width comes from
/// the second moment.
fn find_star(
    image: &FitsImage,
    background: f64,
    noise: f64,
    x: f64,
    y: f64,
    search: f64,
) -> Option<(f64, f64, f64)> {
    let (w, h) = (image.width as i64, image.height as i64);
    let reach = search.ceil() as i64;
    let (cx, cy) = (x.round() as i64, y.round() as i64);
    let mut best: Option<(i64, i64, f64)> = None;
    for py in (cy - reach).max(0)..=(cy + reach).min(h - 1) {
        for px in (cx - reach).max(0)..=(cx + reach).min(w - 1) {
            if ((px as f64 - x).powi(2) + (py as f64 - y).powi(2)).sqrt() > search {
                continue;
            }
            let value = f64::from(image.at(px as usize, py as usize)) - background;
            if value.is_finite() && best.is_none_or(|(_, _, peak)| value > peak) {
                best = Some((px, py, value));
            }
        }
    }
    let (px, py, peak) = best?;
    if image.saturated[(py as usize) * image.width + px as usize] {
        return None; // the brightest pixel itself is clipped: never measured
    }
    if peak < MIN_PEAK_SIGMAS * noise.max(1e-12) {
        return None;
    }
    let (mut sx, mut sy) = (px as f64, py as f64);
    let mut sigma = 1.0;
    for _ in 0..3 {
        let window = 5.0_f64.max(3.0 * sigma);
        let (mut total, mut mx, mut my) = (0.0, 0.0, 0.0);
        let mut pixels = Vec::new();
        let reach = window.ceil() as i64;
        for qy in (sy.round() as i64 - reach).max(0)..=(sy.round() as i64 + reach).min(h - 1) {
            for qx in (sx.round() as i64 - reach).max(0)..=(sx.round() as i64 + reach).min(w - 1) {
                let d2 = (qx as f64 - sx).powi(2) + (qy as f64 - sy).powi(2);
                if d2 > window * window {
                    continue;
                }
                let value = f64::from(image.at(qx as usize, qy as usize)) - background;
                if value.is_finite() && value > 0.0 {
                    total += value;
                    mx += value * qx as f64;
                    my += value * qy as f64;
                    pixels.push((qx as f64, qy as f64, value));
                }
            }
        }
        if total <= 0.0 {
            return None;
        }
        sx = mx / total;
        sy = my / total;
        let second: f64 = pixels
            .iter()
            .map(|(qx, qy, value)| value * ((qx - sx).powi(2) + (qy - sy).powi(2)))
            .sum::<f64>()
            / total;
        sigma = (second / 2.0).sqrt().clamp(0.5, 20.0);
    }
    (((sx - x).powi(2) + (sy - y).powi(2)).sqrt() <= search + 1.0).then_some((sx, sy, sigma))
}

/// The share of the pixel centred at `(px, py)` inside the circle of
/// `radius` centred at `(x, y)`: 1 or 0 away from the edge, a 5 x 5
/// sub-sampling on it.
fn aperture_weight(px: f64, py: f64, x: f64, y: f64, radius: f64) -> f64 {
    let distance = ((px - x).powi(2) + (py - y).powi(2)).sqrt();
    if distance <= radius - 0.75 {
        return 1.0;
    }
    if distance >= radius + 0.75 {
        return 0.0;
    }
    let mut inside = 0;
    for i in 0..5 {
        for j in 0..5 {
            let sx = px - 0.4 + 0.2 * f64::from(i);
            let sy = py - 0.4 + 0.2 * f64::from(j);
            if (sx - x).powi(2) + (sy - y).powi(2) <= radius * radius {
                inside += 1;
            }
        }
    }
    f64::from(inside) / 25.0
}

/// Aperture photometry of one star: background-subtracted flux in ADU, or
/// `None` when the star cannot be measured cleanly.
fn aperture_flux(
    image: &FitsImage,
    star: &FoundStar,
    radius: f64,
    inner: f64,
    outer: f64,
    noise: f64,
) -> Option<f64> {
    let (w, h) = (image.width as f64, image.height as f64);
    if star.x - outer < 0.0
        || star.y - outer < 0.0
        || star.x + outer > w - 1.0
        || star.y + outer > h - 1.0
    {
        return None;
    }
    let reach = outer.ceil() as i64;
    let (cx, cy) = (star.x.round() as i64, star.y.round() as i64);
    let (mut sum, mut area) = (0.0, 0.0);
    let mut annulus = Vec::new();
    for py in cy - reach..=cy + reach {
        for px in cx - reach..=cx + reach {
            let (fx, fy) = (px as f64, py as f64);
            let index = py as usize * image.width + px as usize;
            let value = f64::from(image.pixels[index]);
            let distance = ((fx - star.x).powi(2) + (fy - star.y).powi(2)).sqrt();
            let weight = aperture_weight(fx, fy, star.x, star.y, radius);
            if weight > 0.0 {
                if image.saturated[index] || !value.is_finite() {
                    return None;
                }
                sum += weight * value;
                area += weight;
            } else if (inner..=outer).contains(&distance)
                && value.is_finite()
                && !image.saturated[index]
            {
                annulus.push(value);
            }
        }
    }
    let (local_background, _) = sigma_clipped(&annulus)?;
    let flux = sum - area * local_background;
    (flux > MIN_STAR_SNR * noise * area.sqrt()).then_some(flux)
}

/// Measures the sky of one solved frame against `stars`, the sensor adding
/// `pedestal_adu` to every one of its pixels.
pub fn measure_pose(
    image: &FitsImage,
    stars: &ReferenceStarList,
    pedestal_adu: f64,
) -> Result<PoseSky, PoseRefusal> {
    let wcs = TanWcs::from_header(&image.header).ok_or(PoseRefusal::NoWcs)?;
    let exposure = exposure_seconds(&image.header).ok_or(PoseRefusal::NoExposure)?;
    let sensor_scale = sensor_pixel_scale(&image.header)
        .ok_or(PoseRefusal::NoWcs)?
        .arcsec_per_pixel;
    let image_scale = sensor_scale * image.superpixel as f64;
    let background = estimate_background(image).ok_or(PoseRefusal::NoBackground)?;

    let sensor_width = (image.width * image.superpixel) as f64;
    let sensor_height = (image.height * image.superpixel) as f64;
    let (centre_ra, centre_dec) =
        wcs.pixel_to_world(0.5 + sensor_width / 2.0, 0.5 + sensor_height / 2.0);
    let radius_deg = 0.5 * sensor_width.hypot(sensor_height) * sensor_scale / 3600.0 + 0.01;
    let in_field = stars.within(centre_ra, centre_dec, radius_deg);

    let search = (SEARCH_RADIUS_ARCSEC / image_scale).max(4.0);
    let mut found: Vec<FoundStar> = Vec::new();
    let mut on_frame = 0;
    for star in &in_field {
        let Some((sx, sy)) = wcs.world_to_pixel(star.ra_deg, star.dec_deg) else {
            continue;
        };
        let (ix, iy) = image.sensor_to_image(sx, sy);
        let (x, y) = (ix - 1.0, iy - 1.0); // 0-based
        if x < 0.0 || y < 0.0 || x > image.width as f64 - 1.0 || y > image.height as f64 - 1.0 {
            continue;
        }
        on_frame += 1;
        if let Some((fx, fy, sigma)) =
            find_star(image, background.level, background.noise, x, y, search)
        {
            found.push(FoundStar {
                x: fx,
                y: fy,
                sigma,
                v_mag: star.v_mag,
            });
        }
    }
    let mut sigmas: Vec<f64> = found.iter().map(|star| star.sigma).collect();
    let psf_sigma = median_in_place(&mut sigmas).unwrap_or(1.5);
    let radius = (4.0 * psf_sigma).clamp(3.0, 25.0);
    let inner = radius + 2.0;
    let outer = inner + radius.max(6.0);

    let mut zero_points: Vec<f64> = Vec::new();
    for (index, star) in found.iter().enumerate() {
        // Another reference star of comparable brightness in the annulus
        // spoils both.
        let crowded = found.iter().enumerate().any(|(other_index, other)| {
            other_index != index
                && other.v_mag < star.v_mag + 3.0
                && (other.x - star.x).hypot(other.y - star.y) < outer + radius
        });
        if crowded {
            continue;
        }
        if let Some(flux) = aperture_flux(image, star, radius, inner, outer, background.noise) {
            zero_points.push(star.v_mag + 2.5 * (flux / exposure).log10());
        }
    }
    if zero_points.len() < MIN_REFERENCE_STARS {
        return Err(PoseRefusal::TooFewStars {
            measured: zero_points.len(),
            in_field: on_frame,
        });
    }
    // One discordant star (a variable, a blend the list does not know) is
    // set aside before the median.
    let mut sorted = zero_points.clone();
    let first = median_in_place(&mut sorted).expect("not empty");
    let spread = robust_sigma(&zero_points, first).max(0.02);
    let mut kept: Vec<f64> = zero_points
        .into_iter()
        .filter(|zp| (zp - first).abs() <= 3.0 * spread)
        .collect();
    if kept.len() < MIN_REFERENCE_STARS {
        return Err(PoseRefusal::TooFewStars {
            measured: kept.len(),
            in_field: on_frame,
        });
    }
    let zero_point = median_in_place(&mut kept).expect("not empty");
    let zero_point_spread = robust_sigma(&kept, zero_point);
    // The pedestal is per sensor pixel; a superpixel sums several.
    let above = background.level - pedestal_adu * (image.superpixel * image.superpixel) as f64;
    if above <= 0.0 {
        return Err(PoseRefusal::SkyBelowPedestal);
    }
    let rate = sky_rate(above, exposure, image_scale);
    Ok(PoseSky {
        sky_mag_arcsec2: zero_point - 2.5 * rate.log10(),
        zero_point,
        zero_point_spread,
        stars_measured: kept.len(),
        background_adu: background.level,
        pedestal_adu,
        image_arcsec_per_pixel: image_scale,
        exposure_seconds: exposure,
        centre_ra_deg: centre_ra,
        centre_dec_deg: centre_dec,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::analysis::sky::read_fits_image;
    use crate::analysis::sky_synthetic::{Rng, Storage, SyntheticFrame, SyntheticStar, quoted};
    use crate::analysis::wcs::tan_cards;

    /// The truth a synthetic solved frame is built from.
    #[derive(Debug, Clone, Copy)]
    pub(crate) struct Truth {
        /// mag/arcsec² of the sky.
        pub(crate) sky: f64,
        /// The magnitude of a source giving 1 ADU/s.
        pub(crate) zero_point: f64,
        pub(crate) exposure: f64,
        /// Sensor pixel, arcseconds.
        pub(crate) scale: f64,
        pub(crate) pedestal: f64,
        pub(crate) storage: Storage,
        pub(crate) bayer: bool,
        /// The noise.
        pub(crate) seed: u64,
        /// Where the stars are: frames of one layout share one reference list.
        pub(crate) layout: u64,
    }

    impl Default for Truth {
        fn default() -> Self {
            Self {
                sky: 20.8,
                zero_point: 20.0,
                exposure: 120.0,
                scale: 2.0,
                pedestal: 500.0,
                storage: Storage::Unsigned16,
                bayer: false,
                seed: 11,
                layout: 11,
            }
        }
    }

    pub(crate) const CENTRE: (f64, f64) = (83.82, 22.01);

    /// A solved frame and the reference list its stars come from: 24
    /// measurable reference stars (V 10.8 to 14.0, catalogue positions off
    /// by up to half a pixel), 3 bright ones that saturate, 120 faint stars
    /// the list does not know.
    pub(crate) fn solved_frame(
        truth: Truth,
        extra: &[(&str, String)],
    ) -> (Vec<u8>, ReferenceStarList) {
        let (width, height) = (600, 450);
        let sky_per_pixel = truth.exposure
            * truth.scale
            * truth.scale
            * 10f64.powf(-0.4 * (truth.sky - truth.zero_point));
        let mut frame = SyntheticFrame::new(width, height, sky_per_pixel);
        frame.pedestal_adu = truth.pedestal;
        frame.storage = truth.storage;
        frame.seed = truth.seed;
        frame.sigma_psf = 1.5;
        let mut cards = tan_cards(
            CENTRE.0,
            CENTRE.1,
            (width as f64 / 2.0 + 0.5, height as f64 / 2.0 + 0.5),
            truth.scale,
            17.0,
        );
        cards.push(("EXPTIME", format!("{:.1}", truth.exposure)));
        if truth.bayer {
            cards.push(("BAYERPAT", quoted("RGGB")));
        }
        cards.extend(extra.iter().cloned());
        let header = {
            let owned: Vec<(String, String)> = cards
                .iter()
                .map(|(k, v)| ((*k).to_string(), v.clone()))
                .collect();
            crate::analysis::fits::parse_primary_header(
                &crate::analysis::sky_synthetic::header_bytes(&owned),
            )
            .unwrap()
        };
        let wcs = TanWcs::from_header(&header).unwrap();
        let mut rng = Rng::new(truth.layout.wrapping_mul(31).wrapping_add(5));
        let flux = |mag: f64| truth.exposure * 10f64.powf(-0.4 * (mag - truth.zero_point));
        let mut list = Vec::new();
        let place = |rng: &mut Rng, margin: f64| {
            (
                margin + rng.uniform() * (width as f64 - 2.0 * margin),
                margin + rng.uniform() * (height as f64 - 2.0 * margin),
            )
        };
        for index in 0..27 {
            let (x, y) = place(&mut rng, 30.0);
            let mag = if index < 3 {
                7.0 + rng.uniform()
            } else {
                10.8 + rng.uniform() * 3.2
            };
            frame.stars.push(SyntheticStar::point(x, y, flux(mag)));
            // The catalogue position, up to half a pixel away.
            let (ra, dec) =
                wcs.pixel_to_world(x + (rng.uniform() - 0.5), y + (rng.uniform() - 0.5));
            list.push(ReferenceStar {
                ra_deg: ra,
                dec_deg: dec,
                v_mag: mag,
            });
        }
        for _ in 0..120 {
            let (x, y) = place(&mut rng, 0.0);
            frame
                .stars
                .push(SyntheticStar::point(x, y, flux(14.5 + rng.uniform() * 3.0)));
        }
        (frame.to_fits(&cards), ReferenceStarList::new(list))
    }

    /// The tolerance of the feasibility checkpoint: the chain gives back
    /// the sky of a synthetic frame within 0.05 mag/arcsec².
    pub(crate) const CHECKPOINT_TOLERANCE_MAG: f64 = 0.05;

    #[test]
    fn the_chain_gives_back_the_sky_of_a_synthetic_frame_within_the_tolerance() {
        println!(
            "| sky (truth) | storage | colour | seed | sky (measured) | error | zero point | stars |"
        );
        println!("| --- | --- | --- | --- | --- | --- | --- | --- |");
        let mut worst: f64 = 0.0;
        for sky in [18.0, 19.5, 20.8, 21.8] {
            for (storage, bayer) in [
                (Storage::Unsigned16, false),
                (Storage::Float32, false),
                (Storage::Unsigned16, true),
            ] {
                for seed in [11, 12] {
                    let truth = Truth {
                        sky,
                        storage,
                        bayer,
                        seed,
                        layout: seed,
                        ..Truth::default()
                    };
                    let (bytes, list) = solved_frame(truth, &[]);
                    let image = read_fits_image(&bytes).unwrap();
                    let pose = measure_pose(&image, &list, truth.pedestal).unwrap();
                    let error = pose.sky_mag_arcsec2 - sky;
                    worst = worst.max(error.abs());
                    println!(
                        "| {sky:.2} | {storage:?} | {bayer} | {seed} | {:.3} | {error:+.3} | {:.3} | {} |",
                        pose.sky_mag_arcsec2, pose.zero_point, pose.stars_measured
                    );
                    assert!(
                        error.abs() <= CHECKPOINT_TOLERANCE_MAG,
                        "sky {sky}, {storage:?}, bayer {bayer}, seed {seed}: measured {}",
                        pose.sky_mag_arcsec2
                    );
                    assert!((pose.zero_point - truth.zero_point).abs() <= 0.03);
                    assert!(pose.stars_measured >= 15, "{}", pose.stars_measured);
                }
            }
        }
        println!("worst error: {worst:.3} mag/arcsec²");
    }

    #[test]
    fn saturated_reference_stars_are_never_measured() {
        let (bytes, list) = solved_frame(Truth::default(), &[]);
        let image = read_fits_image(&bytes).unwrap();
        let pose = measure_pose(&image, &list, 500.0).unwrap();
        // Only the three bright ones are set aside: the result stays true.
        assert!(pose.stars_measured <= 24);
        // A list of the three saturated stars alone gives no value.
        let bright = ReferenceStarList::new(
            list.stars
                .iter()
                .copied()
                .filter(|star| star.v_mag < 8.5)
                .collect(),
        );
        assert_eq!(bright.len(), 3);
        assert_eq!(
            measure_pose(&image, &bright, 500.0),
            Err(PoseRefusal::TooFewStars {
                measured: 0,
                in_field: 3
            })
        );
    }

    #[test]
    fn a_frame_without_a_wcs_gives_no_value() {
        let mut frame = SyntheticFrame::new(200, 150, 300.0);
        frame
            .stars
            .push(SyntheticStar::point(100.0, 75.0, 50_000.0));
        let image = read_fits_image(&frame.to_fits(&[("EXPTIME", "120.0".into())])).unwrap();
        let (_, list) = solved_frame(Truth::default(), &[]);
        let refusal = measure_pose(&image, &list, 0.0).unwrap_err();
        assert_eq!(refusal, PoseRefusal::NoWcs);
        assert_eq!(refusal.code(), "no_wcs");
    }

    #[test]
    fn too_few_stars_or_a_sky_under_the_pedestal_give_no_value() {
        let (bytes, list) = solved_frame(Truth::default(), &[]);
        let image = read_fits_image(&bytes).unwrap();
        let two = ReferenceStarList::new(
            list.stars
                .iter()
                .copied()
                .filter(|star| star.v_mag > 9.0)
                .take(2)
                .collect(),
        );
        assert!(matches!(
            measure_pose(&image, &two, 500.0),
            Err(PoseRefusal::TooFewStars { measured: 2, .. })
        ));
        assert_eq!(
            measure_pose(&image, &list, 5_000.0),
            Err(PoseRefusal::SkyBelowPedestal)
        );
        assert!(measure_pose(&image, &ReferenceStarList::default(), 500.0).is_err());
    }

    #[test]
    fn a_wrong_pedestal_moves_the_value_the_way_it_should() {
        // A pedestal half the sky too low reads the sky brighter: the reason
        // the measure never guesses one.
        let truth = Truth::default();
        let (bytes, list) = solved_frame(truth, &[]);
        let image = read_fits_image(&bytes).unwrap();
        let right = measure_pose(&image, &list, truth.pedestal).unwrap();
        let sky_per_pixel = right.background_adu - truth.pedestal; // not a colour frame
        let wrong = measure_pose(&image, &list, truth.pedestal - sky_per_pixel / 2.0).unwrap();
        assert!(
            (right.sky_mag_arcsec2 - wrong.sky_mag_arcsec2 - 2.5 * 1.5f64.log10()).abs() < 1e-9
        );
    }

    #[test]
    fn the_reference_list_round_trips_and_reads_hipparcos_and_tycho_lines() {
        let list = ReferenceStarList::new(vec![
            ReferenceStar {
                ra_deg: 10.684_708,
                dec_deg: 41.268_75,
                v_mag: 9.123,
            },
            ReferenceStar {
                ra_deg: 359.999,
                dec_deg: -89.5,
                v_mag: -1.46,
            },
        ]);
        let back = ReferenceStarList::from_bytes(&list.to_bytes()).unwrap();
        assert_eq!(back.len(), 2);
        let near = back.within(10.7, 41.27, 0.1);
        assert_eq!(near.len(), 1);
        assert!((near[0].ra_deg - 10.684_708).abs() < 2e-5);
        assert!((near[0].v_mag - 9.123).abs() < 1e-9);
        assert_eq!(
            ReferenceStarList::from_bytes(b"NOTSTARS\0\0\0\0"),
            Err(StarListError::NotAStarList)
        );
        let mut cut = list.to_bytes();
        cut.truncate(cut.len() - 3);
        assert_eq!(
            ReferenceStarList::from_bytes(&cut),
            Err(StarListError::Truncated)
        );

        // Two lines shaped like hip_main.dat (fields H0..H13 shown; the
        // rest of a real line follows and is ignored), the second without
        // a magnitude.
        let hipparcos = "H|           1| |00 00 00.22|+01 05 20.4| 9.10| |H|000.00091185|+01.08901332| |   3.54|   -5.20|   -1.88|  1.32\n\
                         H|           2| |00 00 00.91|-19 29 55.8|     | |H|000.00379737|-19.49883745| |  21.90|  181.21|   -0.93|  1.28\n";
        let stars = parse_hipparcos_main(hipparcos);
        assert_eq!(stars.len(), 1);
        assert!((stars[0].v_mag - 9.10).abs() < 1e-9);
        // -5.20 mas/yr (μα·cos δ) in RA over 8.75 years: -45.5 mas.
        let cos_dec = 1.08901332f64.to_radians().cos();
        assert!((stars[0].ra_deg - (0.00091185 - 45.5 / (3.6e6 * cos_dec))).abs() < 1e-9);
        assert!((stars[0].dec_deg - (1.08901332 - 1.88 * 8.75 / 3.6e6)).abs() < 1e-9);

        // A line shaped like tyc2.dat.
        let tycho = "0001 00008 1| |  2.31750494|  2.23184345|  -16.3|   -9.0| 68| 73| 1.7| 1.8|1958.89|1951.94| 4|1.0|1.0|0.9|1.0|12.146|0.158|12.146|0.223|999| |         |  2.31754222|  2.23186444|1.67|1.54| 88.0|100.8| |-0.2\n\
                     0001 00013 1|X|            |            |       |       |   |   |    |    |       |       |  |   |   |   |   |10.488|0.038| 8.670|0.015|999| |         |  1.12558209|  2.26739400|1.81|1.52| 9.3|12.7| |-0.2\n";
        let stars = parse_tycho2(tycho);
        assert_eq!(stars.len(), 2);
        assert!((stars[0].v_mag - 12.146).abs() < 1e-9);
        assert!((stars[0].ra_deg - 2.31750494).abs() < 1e-9);
        assert!((stars[1].v_mag - (8.670 - 0.090 * (10.488 - 8.670))).abs() < 1e-9);
        assert!(
            (stars[1].ra_deg - 1.12558209).abs() < 1e-9,
            "observed position"
        );
    }
}

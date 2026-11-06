//! The world coordinate system of a solved frame: the gnomonic (`TAN`)
//! projection a plate solver writes into the header, read just far enough
//! to place a catalogue star on the frame.
//!
//! Only `RA---TAN` / `DEC--TAN` is read (with a `-SIP` suffix the
//! distortion polynomial is ignored: the photometry re-centres every star
//! on its own light, a few pixels away at most). A frame without these
//! keywords was never solved: [`TanWcs::from_header`] answers `None`, and
//! the measure says "unsolved frame" instead of guessing.

use super::fits::FitsHeader;
use super::sky::header_cd_matrix;

/// A `TAN` projection: reference point, reference pixel, linear transform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TanWcs {
    /// `CRVAL1`, `CRVAL2`: right ascension and declination of the
    /// reference pixel, degrees.
    pub reference_ra_deg: f64,
    pub reference_dec_deg: f64,
    /// `CRPIX1`, `CRPIX2`: the reference pixel (FITS coordinates).
    pub reference_x: f64,
    pub reference_y: f64,
    /// Degrees per pixel (`header_cd_matrix`).
    pub cd: [[f64; 2]; 2],
}

impl TanWcs {
    /// The projection of a solved frame, or `None` (not solved, another
    /// projection, or an unusable matrix).
    #[must_use]
    pub fn from_header(header: &FitsHeader) -> Option<Self> {
        let ctype1 = header.get("CTYPE1")?.trim().to_ascii_uppercase();
        let ctype2 = header.get("CTYPE2")?.trim().to_ascii_uppercase();
        if !ctype1.starts_with("RA---TAN") || !ctype2.starts_with("DEC--TAN") {
            return None;
        }
        let wcs = Self {
            reference_ra_deg: header.get_f64("CRVAL1")?,
            reference_dec_deg: header.get_f64("CRVAL2")?,
            reference_x: header.get_f64("CRPIX1")?,
            reference_y: header.get_f64("CRPIX2")?,
            cd: header_cd_matrix(header)?,
        };
        [
            wcs.reference_ra_deg,
            wcs.reference_dec_deg,
            wcs.reference_x,
            wcs.reference_y,
        ]
        .iter()
        .all(|value| value.is_finite())
        .then_some(wcs)
    }

    /// Sky to frame: the FITS pixel coordinates of `(ra, dec)` (degrees), or
    /// `None` for a point 90° or more from the reference (behind the plane).
    #[must_use]
    pub fn world_to_pixel(&self, ra_deg: f64, dec_deg: f64) -> Option<(f64, f64)> {
        let (ra, dec) = (ra_deg.to_radians(), dec_deg.to_radians());
        let (ra0, dec0) = (
            self.reference_ra_deg.to_radians(),
            self.reference_dec_deg.to_radians(),
        );
        let delta = ra - ra0;
        let cos_c = dec0.sin() * dec.sin() + dec0.cos() * dec.cos() * delta.cos();
        if cos_c <= 1e-9 {
            return None;
        }
        let xi = (dec.cos() * delta.sin() / cos_c).to_degrees();
        let eta =
            ((dec0.cos() * dec.sin() - dec0.sin() * dec.cos() * delta.cos()) / cos_c).to_degrees();
        let [[a, b], [c, d]] = self.cd;
        let determinant = a * d - b * c;
        let dx = (d * xi - b * eta) / determinant;
        let dy = (-c * xi + a * eta) / determinant;
        Some((self.reference_x + dx, self.reference_y + dy))
    }

    /// Frame to sky: `(ra, dec)` in degrees, right ascension in `[0, 360)`.
    #[must_use]
    pub fn pixel_to_world(&self, x: f64, y: f64) -> (f64, f64) {
        let (dx, dy) = (x - self.reference_x, y - self.reference_y);
        let [[a, b], [c, d]] = self.cd;
        let xi = (a * dx + b * dy).to_radians();
        let eta = (c * dx + d * dy).to_radians();
        let (ra0, dec0) = (
            self.reference_ra_deg.to_radians(),
            self.reference_dec_deg.to_radians(),
        );
        let denominator = dec0.cos() - eta * dec0.sin();
        let ra = ra0 + xi.atan2(denominator);
        let dec = (dec0.sin() + eta * dec0.cos()).atan2(xi.hypot(denominator));
        (ra.to_degrees().rem_euclid(360.0), dec.to_degrees())
    }
}

/// The angle between two points of the sky, degrees.
#[must_use]
pub fn angular_distance_deg(ra1: f64, dec1: f64, ra2: f64, dec2: f64) -> f64 {
    let (ra1, dec1, ra2, dec2) = (
        ra1.to_radians(),
        dec1.to_radians(),
        ra2.to_radians(),
        dec2.to_radians(),
    );
    let half_dra = (ra2 - ra1) / 2.0;
    let half_ddec = (dec2 - dec1) / 2.0;
    let h = half_ddec.sin().powi(2) + dec1.cos() * dec2.cos() * half_dra.sin().powi(2);
    (2.0 * h.sqrt().min(1.0).asin()).to_degrees()
}

/// Test helper: the header cards of a `TAN` solution centred on
/// `(ra, dec)` at `scale` arcseconds per pixel, rotated by `rotation`
/// degrees, with the reference pixel at `(crpix1, crpix2)`.
#[cfg(test)]
pub(crate) fn tan_cards(
    ra: f64,
    dec: f64,
    crpix: (f64, f64),
    scale_arcsec: f64,
    rotation_deg: f64,
) -> Vec<(&'static str, String)> {
    let s = scale_arcsec / 3600.0;
    let (sin, cos) = rotation_deg.to_radians().sin_cos();
    vec![
        ("CTYPE1", "'RA---TAN'".to_string()),
        ("CTYPE2", "'DEC--TAN'".to_string()),
        ("CRVAL1", format!("{ra:.10}")),
        ("CRVAL2", format!("{dec:.10}")),
        ("CRPIX1", format!("{:.4}", crpix.0)),
        ("CRPIX2", format!("{:.4}", crpix.1)),
        ("CD1_1", format!("{:.12E}", -s * cos)),
        ("CD1_2", format!("{:.12E}", s * sin)),
        ("CD2_1", format!("{:.12E}", s * sin)),
        ("CD2_2", format!("{:.12E}", s * cos)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::fits::parse_primary_header;
    use crate::analysis::sky_synthetic::header_bytes;

    fn header(cards: &[(&str, String)]) -> FitsHeader {
        let owned: Vec<(String, String)> = cards
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect();
        parse_primary_header(&header_bytes(&owned)).unwrap()
    }

    #[test]
    fn a_solved_header_round_trips_between_sky_and_frame() {
        for (ra, dec) in [(83.82, -5.39), (10.68, 41.27), (359.9, 88.5), (180.0, 0.0)] {
            let wcs = TanWcs::from_header(&header(&tan_cards(ra, dec, (512.5, 384.5), 1.8, 23.0)))
                .unwrap();
            let (x, y) = wcs.world_to_pixel(ra, dec).unwrap();
            assert!((x - 512.5).abs() < 1e-6 && (y - 384.5).abs() < 1e-6);
            for (px, py) in [(1.0, 1.0), (1024.0, 768.0), (300.3, 700.7)] {
                let (wra, wdec) = wcs.pixel_to_world(px, py);
                let (bx, by) = wcs.world_to_pixel(wra, wdec).unwrap();
                assert!(
                    (bx - px).abs() < 1e-6 && (by - py).abs() < 1e-6,
                    "{ra} {dec}"
                );
                // 1.8" pixels: the corner is about 0.4° from the centre.
                assert!(angular_distance_deg(ra, dec, wra, wdec) < 0.5);
            }
        }
        // North is up and east is left for a negative CD1_1 and no rotation.
        let wcs = TanWcs::from_header(&header(&tan_cards(150.0, 20.0, (100.0, 100.0), 3.6, 0.0)))
            .unwrap();
        let (x, y) = wcs.world_to_pixel(150.0, 20.01).unwrap();
        assert!((x - 100.0).abs() < 1e-6 && (y - 110.0).abs() < 1e-3);
        let (x, _) = wcs.world_to_pixel(150.02, 20.0).unwrap();
        assert!(x < 100.0);
    }

    #[test]
    fn a_frame_that_was_never_solved_has_no_wcs() {
        assert_eq!(
            TanWcs::from_header(&header(&[("EXPTIME", "60".into())])),
            None
        );
        let mut cards = tan_cards(10.0, 10.0, (1.0, 1.0), 1.0, 0.0);
        cards[0].1 = "'RA---SIN'".to_string();
        assert_eq!(
            TanWcs::from_header(&header(&cards)),
            None,
            "another projection"
        );
        let cards = tan_cards(10.0, 10.0, (1.0, 1.0), 1.0, 0.0);
        let without_crval: Vec<_> = cards.into_iter().filter(|(k, _)| *k != "CRVAL2").collect();
        assert_eq!(TanWcs::from_header(&header(&without_crval)), None);
    }
}

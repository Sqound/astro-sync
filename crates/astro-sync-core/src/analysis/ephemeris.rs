//! The Sun, the Moon and a frame's centre above a place, at an instant:
//! just enough astronomy to keep only the poses whose sky compares with a
//! dark zenith (`analysis::sky_night`).
//!
//! Low-precision formulas of the Astronomical Almanac: the Sun to about
//! 0.01°, the Moon to about 0.3° (its main periodic terms, then the
//! topocentric parallax), which is far finer than the thresholds they feed
//! (a Sun under -18°, a Moon under -2°). No refraction: the thresholds are
//! below the horizon, where it does not matter. Angles in degrees.

/// Julian date of a Unix time, in seconds.
#[must_use]
pub fn julian_date(unix_seconds: f64) -> f64 {
    unix_seconds / 86_400.0 + 2_440_587.5
}

/// Greenwich mean sidereal time, degrees in `[0, 360)`.
#[must_use]
pub fn greenwich_sidereal_deg(jd: f64) -> f64 {
    let d = jd - 2_451_545.0;
    let t = d / 36_525.0;
    (280.460_618_37 + 360.985_647_366_29 * d + 0.000_387_933 * t * t).rem_euclid(360.0)
}

fn obliquity_deg(jd: f64) -> f64 {
    23.439_291 - 0.013_004_2 * (jd - 2_451_545.0) / 36_525.0
}

fn ecliptic_to_equatorial(longitude: f64, latitude: f64, jd: f64) -> (f64, f64) {
    let (l, b, e) = (
        longitude.to_radians(),
        latitude.to_radians(),
        obliquity_deg(jd).to_radians(),
    );
    let ra = (l.sin() * e.cos() - b.tan() * e.sin()).atan2(l.cos());
    let dec = (b.sin() * e.cos() + b.cos() * e.sin() * l.sin()).asin();
    (ra.to_degrees().rem_euclid(360.0), dec.to_degrees())
}

/// The Sun's apparent right ascension and declination.
#[must_use]
pub fn sun_equatorial(jd: f64) -> (f64, f64) {
    let n = jd - 2_451_545.0;
    let mean_longitude = 280.460 + 0.985_647_4 * n;
    let anomaly = (357.528 + 0.985_600_3 * n).to_radians();
    let longitude = mean_longitude + 1.915 * anomaly.sin() + 0.020 * (2.0 * anomaly).sin();
    ecliptic_to_equatorial(longitude.rem_euclid(360.0), 0.0, jd)
}

/// The Moon's geocentric right ascension and declination, and its
/// horizontal parallax.
#[must_use]
pub fn moon_equatorial(jd: f64) -> (f64, f64, f64) {
    let t = (jd - 2_451_545.0) / 36_525.0;
    let s = |a: f64, b: f64| (a + b * t).to_radians().sin();
    let c = |a: f64, b: f64| (a + b * t).to_radians().cos();
    let longitude = 218.32 + 481_267.881 * t + 6.29 * s(135.0, 477_198.87)
        - 1.27 * s(259.3, -413_335.36)
        + 0.66 * s(235.7, 890_534.22)
        + 0.21 * s(269.9, 954_397.74)
        - 0.19 * s(357.5, 35_999.05)
        - 0.11 * s(186.5, 966_404.03);
    let latitude = 5.13 * s(93.3, 483_202.02) + 0.28 * s(228.2, 960_400.89)
        - 0.28 * s(318.3, 6_003.15)
        - 0.17 * s(217.6, -407_332.21);
    let parallax = 0.9508
        + 0.0518 * c(135.0, 477_198.87)
        + 0.0095 * c(259.3, -413_335.36)
        + 0.0078 * c(235.7, 890_534.22)
        + 0.0028 * c(269.9, 954_397.74);
    let (ra, dec) = ecliptic_to_equatorial(longitude.rem_euclid(360.0), latitude, jd);
    (ra, dec, parallax)
}

/// The altitude of `(ra, dec)` above a place, geometric (no refraction).
#[must_use]
pub fn altitude_deg(ra: f64, dec: f64, jd: f64, latitude: f64, longitude_east: f64) -> f64 {
    let hour_angle = (greenwich_sidereal_deg(jd) + longitude_east - ra).to_radians();
    let (phi, delta) = (latitude.to_radians(), dec.to_radians());
    (phi.sin() * delta.sin() + phi.cos() * delta.cos() * hour_angle.cos())
        .clamp(-1.0, 1.0)
        .asin()
        .to_degrees()
}

/// The Sun's altitude above a place at a Unix time.
#[must_use]
pub fn sun_altitude_deg(unix_seconds: f64, latitude: f64, longitude_east: f64) -> f64 {
    let jd = julian_date(unix_seconds);
    let (ra, dec) = sun_equatorial(jd);
    altitude_deg(ra, dec, jd, latitude, longitude_east)
}

/// The Moon's topocentric altitude above a place at a Unix time.
#[must_use]
pub fn moon_altitude_deg(unix_seconds: f64, latitude: f64, longitude_east: f64) -> f64 {
    let jd = julian_date(unix_seconds);
    let (ra, dec, parallax) = moon_equatorial(jd);
    let geocentric = altitude_deg(ra, dec, jd, latitude, longitude_east);
    geocentric - parallax * geocentric.to_radians().cos()
}

/// Airmass for a geometric altitude (Kasten and Young, 1989), capped at 40.
#[must_use]
pub fn airmass(altitude_deg: f64) -> f64 {
    if altitude_deg <= 0.0 {
        return 40.0;
    }
    let z = 90.0 - altitude_deg;
    (1.0 / (altitude_deg.to_radians().sin() + 0.505_72 * (96.079_95 - z).powf(-1.6364))).min(40.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Reference altitudes from the `astronomy-engine` library the portal
    // already uses for its night windows (topocentric, no refraction).
    // (unix seconds, latitude, longitude east, Sun altitude, Moon altitude)
    const REFERENCES: [(f64, f64, f64, f64, f64); 7] = [
        (1_791_669_600.0, 45.0, -30.0, -28.055, -31.523),
        (1_791_691_200.0, 45.0, -30.0, -42.293, -50.256),
        (1_792_098_000.0, 45.0, -30.0, -19.293, 3.994),
        (1_793_134_800.0, 45.0, -30.0, -22.578, 14.234),
        (1_767_225_600.0, 35.68, 139.69, 19.567, -28.710),
        (1_780_000_000.0, -33.9, 18.4, -57.453, 74.417),
        (1_800_000_000.0, 64.1, -21.9, -16.466, -14.297),
    ];

    #[test]
    fn the_sun_and_the_moon_are_where_the_reference_library_puts_them() {
        for (unix, latitude, longitude, sun, moon) in REFERENCES {
            let s = sun_altitude_deg(unix, latitude, longitude);
            let m = moon_altitude_deg(unix, latitude, longitude);
            assert!((s - sun).abs() < 0.05, "Sun {s} vs {sun} at {unix}");
            assert!((m - moon).abs() < 0.5, "Moon {m} vs {moon} at {unix}");
        }
    }

    #[test]
    fn the_airmass_is_one_at_the_zenith_and_grows_toward_the_horizon() {
        assert!((airmass(90.0) - 1.0).abs() < 1e-3);
        assert!((airmass(30.0) - 1.995).abs() < 0.01);
        assert!((airmass(40.0) - 1.554).abs() < 0.01);
        assert_eq!(airmass(-5.0), 40.0);
    }
}

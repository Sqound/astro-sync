//! The sky background read from the pixels of a light frame.
//!
//! The other values of `metrics/quality.csv` are header keywords, passed
//! through (`fits.rs`). The measured sky background is computed from the
//! pixels themselves, on the person's computer -- the frames never
//! leave it; only the night's one number does (`analysis::sky_night`).
//!
//! Three steps live here:
//!
//! 1. [`read_fits_image`] -- the primary data unit of an uncompressed FITS
//!    file, integer (`BITPIX` 8, 16, 32, 64) or float (`-32`, `-64`), scaled
//!    by `BZERO`/`BSCALE` into physical ADU. A colour sensor's raw frame
//!    (`BAYERPAT`) is read as 2 x 2 superpixels (the sum of the four
//!    photosites), so that the background and the stars are measured on the
//!    same summed light and the colour pattern never reads as noise. A
//!    tile-compressed file, a cube, or an empty primary unit is refused, said
//!    plainly: nothing is guessed.
//! 2. [`estimate_background`] -- the background level with the stars
//!    masked: the frame is cut into tiles, each tile gives an iteratively
//!    3-sigma-clipped median (the clipping removes the stars and their
//!    wings), and the frame's level is the median of the tile levels after
//!    the tiles far from the others (a galaxy, a bright nebula, a gradient's
//!    corner) are set aside. The clipped standard deviation of the tiles is
//!    the background noise the photometry uses.
//! 3. [`sensor_pixel_scale`] -- the pixel scale, from the header: the WCS
//!    linear transform first (`CD` matrix, or `CDELT` with `PC`/`CROTA2`),
//!    then a scale keyword (`PIXSCALE`, `SCALE`, `SECPIX`), then the optics
//!    (`XPIXSZ` in micrometres, already including the binning as N.I.N.A.
//!    and MaxIm write it, over `FOCALLEN` in millimetres). The measure itself
//!    needs a WCS (`analysis::photometry`), so it always uses the first.
//!
//! The level is in ADU per pixel over the exposure; [`sky_rate`] turns it
//! into ADU per second and per square arcsecond, the quantity the zero
//! point of `analysis::photometry` converts to mag/arcsec².

use std::fmt;

use super::fits::{FitsError, FitsHeader, parse_primary_header_with_data_offset};

/// Why a frame's pixels could not be read.
#[derive(Debug, PartialEq)]
pub enum PixelReadError {
    /// The header itself could not be read.
    Header(FitsError),
    /// The primary unit holds no two-dimensional image this reader takes:
    /// `NAXIS` other than 2 (or 3 with one plane), a tile-compressed file
    /// (its image lives in an extension), or a missing dimension.
    NotAnImage(&'static str),
    /// A `BITPIX` outside the six the FITS standard defines.
    UnsupportedBitpix(i64),
    /// The file ends before the last pixel.
    Truncated {
        expected_bytes: usize,
        found_bytes: usize,
    },
}

impl fmt::Display for PixelReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PixelReadError::Header(error) => write!(f, "{error}"),
            PixelReadError::NotAnImage(why) => write!(f, "not a readable image: {why}"),
            PixelReadError::UnsupportedBitpix(bitpix) => write!(f, "unsupported BITPIX {bitpix}"),
            PixelReadError::Truncated {
                expected_bytes,
                found_bytes,
            } => write!(
                f,
                "truncated data unit: {found_bytes} bytes of {expected_bytes}"
            ),
        }
    }
}

impl std::error::Error for PixelReadError {}

/// One frame's pixels, in physical ADU, row by row: index `(y - 1) * width
/// + (x - 1)` holds the pixel whose centre is at FITS coordinates `(x, y)`
/// (the first pixel's centre is `(1, 1)`).
#[derive(Debug, Clone)]
pub struct FitsImage {
    pub header: FitsHeader,
    pub width: usize,
    pub height: usize,
    /// How many sensor pixels each image pixel sums along each axis: 2 for a
    /// colour sensor's raw frame read as superpixels, 1 otherwise.
    pub superpixel: usize,
    pub pixels: Vec<f32>,
    /// Whether any sensor pixel summed into this one reached the saturation
    /// level ([`saturation_level`]): a star touching one is never measured.
    pub saturated: Vec<bool>,
}

impl FitsImage {
    /// The value at the 0-based column `x` and row `y`.
    #[must_use]
    pub fn at(&self, x: usize, y: usize) -> f32 {
        self.pixels[y * self.width + x]
    }

    /// Sensor FITS coordinates (what a WCS speaks) to this image's FITS
    /// coordinates: the identity, or the superpixel that holds the point.
    #[must_use]
    pub fn sensor_to_image(&self, x: f64, y: f64) -> (f64, f64) {
        let s = self.superpixel as f64;
        ((x - 0.5) / s + 0.5, (y - 0.5) / s + 0.5)
    }
}

/// The level at or above which a sensor pixel counts as saturated: the
/// header's own `SATURATE` or `DATAMAX` when the capture software wrote one,
/// else 98 % of the brightest pixel of the frame. The fallback is cautious on
/// purpose: on a frame with no saturated star it sets aside the brightest
/// star only, never a saturated one.
#[must_use]
pub fn saturation_level(header: &FitsHeader, values: &[f32]) -> f32 {
    if let Some(level) = header
        .get_f64("SATURATE")
        .or_else(|| header.get_f64("DATAMAX"))
        .filter(|level| level.is_finite() && *level > 0.0)
    {
        return level as f32;
    }
    let max = values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .fold(f32::MIN, f32::max);
    0.98 * max
}

/// Reads the primary image of an uncompressed FITS file into physical ADU.
pub fn read_fits_image(bytes: &[u8]) -> Result<FitsImage, PixelReadError> {
    let (header, data_offset) =
        parse_primary_header_with_data_offset(bytes).map_err(PixelReadError::Header)?;
    if header
        .get("ZIMAGE")
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("T"))
    {
        return Err(PixelReadError::NotAnImage("tile-compressed image"));
    }
    let naxis = header.get_i64("NAXIS").unwrap_or(0);
    let planes = header.get_i64("NAXIS3").unwrap_or(1);
    if naxis == 0 {
        return Err(PixelReadError::NotAnImage(
            "empty primary unit (a compressed or multi-extension file)",
        ));
    }
    if !(naxis == 2 || (naxis == 3 && planes == 1)) {
        return Err(PixelReadError::NotAnImage(
            "not a single two-dimensional plane",
        ));
    }
    let width = header
        .get_i64("NAXIS1")
        .filter(|value| *value > 0)
        .ok_or(PixelReadError::NotAnImage("no NAXIS1"))? as usize;
    let height = header
        .get_i64("NAXIS2")
        .filter(|value| *value > 0)
        .ok_or(PixelReadError::NotAnImage("no NAXIS2"))? as usize;
    let bitpix = header.get_i64("BITPIX").unwrap_or(0);
    let bytes_per_value = match bitpix {
        8 => 1,
        16 => 2,
        32 | -32 => 4,
        64 | -64 => 8,
        other => return Err(PixelReadError::UnsupportedBitpix(other)),
    };
    let count = width * height;
    let expected = data_offset + count * bytes_per_value;
    if bytes.len() < expected {
        return Err(PixelReadError::Truncated {
            expected_bytes: expected,
            found_bytes: bytes.len(),
        });
    }
    let bzero = header.get_f64("BZERO").unwrap_or(0.0);
    let bscale = header.get_f64("BSCALE").unwrap_or(1.0);
    let data = &bytes[data_offset..expected];
    let raw: Vec<f32> = data
        .chunks_exact(bytes_per_value)
        .map(|chunk| {
            let value = match bitpix {
                8 => f64::from(chunk[0]),
                16 => f64::from(i16::from_be_bytes([chunk[0], chunk[1]])),
                32 => f64::from(i32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])),
                64 => i64::from_be_bytes(chunk.try_into().expect("eight bytes")) as f64,
                -32 => f64::from(f32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])),
                _ => f64::from_be_bytes(chunk.try_into().expect("eight bytes")),
            };
            (bzero + bscale * value) as f32
        })
        .collect();
    let level = saturation_level(&header, &raw);
    let bayer = header
        .get("BAYERPAT")
        .is_some_and(|pattern| !pattern.trim().is_empty());
    if !bayer {
        let saturated = raw.iter().map(|value| *value >= level).collect();
        return Ok(FitsImage {
            header,
            width,
            height,
            superpixel: 1,
            pixels: raw,
            saturated,
        });
    }
    let (binned_width, binned_height) = (width / 2, height / 2);
    if binned_width == 0 || binned_height == 0 {
        return Err(PixelReadError::NotAnImage(
            "colour frame smaller than 2 x 2",
        ));
    }
    let mut pixels = Vec::with_capacity(binned_width * binned_height);
    let mut saturated = Vec::with_capacity(binned_width * binned_height);
    for y in 0..binned_height {
        for x in 0..binned_width {
            let corners = [
                raw[(2 * y) * width + 2 * x],
                raw[(2 * y) * width + 2 * x + 1],
                raw[(2 * y + 1) * width + 2 * x],
                raw[(2 * y + 1) * width + 2 * x + 1],
            ];
            pixels.push(corners.iter().sum());
            saturated.push(corners.iter().any(|value| *value >= level));
        }
    }
    Ok(FitsImage {
        header,
        width: binned_width,
        height: binned_height,
        superpixel: 2,
        pixels,
        saturated,
    })
}

/// The background of one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackgroundEstimate {
    /// ADU per image pixel over the whole exposure, pedestal included.
    pub level: f64,
    /// The clipped standard deviation of the background, ADU per pixel.
    pub noise: f64,
    /// How many tiles the level is the median of.
    pub tiles: usize,
}

/// The median of `values` (sorted in place). `None` when empty.
pub(crate) fn median_in_place(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let middle = values.len() / 2;
    Some(if values.len().is_multiple_of(2) {
        0.5 * (values[middle - 1] + values[middle])
    } else {
        values[middle]
    })
}

/// 1.4826 times the median absolute deviation from `centre`: the standard
/// deviation of a normal distribution, robust to outliers.
pub(crate) fn robust_sigma(values: &[f64], centre: f64) -> f64 {
    let mut deviations: Vec<f64> = values.iter().map(|value| (value - centre).abs()).collect();
    1.4826 * median_in_place(&mut deviations).unwrap_or(0.0)
}

/// Iterative 3-sigma clipping around the median, at most ten rounds: the
/// median and robust sigma of what remains. Stars sit far above the
/// background and are cut away on the first rounds; their faint wings on
/// the next ones.
pub(crate) fn sigma_clipped(values: &[f64]) -> Option<(f64, f64)> {
    let mut kept: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if kept.is_empty() {
        return None;
    }
    for _ in 0..10 {
        let centre = median_in_place(&mut kept)?;
        let sigma = robust_sigma(&kept, centre);
        if sigma <= 0.0 {
            return Some((centre, 0.0));
        }
        let before = kept.len();
        kept.retain(|value| (value - centre).abs() <= 3.0 * sigma);
        if kept.len() == before || kept.is_empty() {
            break;
        }
    }
    if kept.is_empty() {
        return None;
    }
    let median = median_in_place(&mut kept)?;
    let sigma = robust_sigma(&kept, median);
    // What the clipping leaves of the stars' faint wings skews the
    // distribution upwards: the mode (`2.5 median - 1.5 mean`, the estimator
    // SExtractor uses) is closer to the true background then; on a
    // strongly skewed tile (more than 0.3 sigma between mean and median)
    // the median is kept.
    let mean = kept.iter().sum::<f64>() / kept.len() as f64;
    let level = if sigma > 0.0 && (mean - median).abs() < 0.3 * sigma {
        2.5 * median - 1.5 * mean
    } else {
        median
    };
    Some((level, sigma))
}

/// At most this many tiles along each axis.
const MAX_TILES_PER_AXIS: usize = 16;
/// A tile is at least this many pixels on a side.
const MIN_TILE_SIDE: usize = 32;
/// At most this many pixels are read in each tile (an even sub-sample).
const MAX_SAMPLES_PER_TILE: usize = 4096;

/// The background level and noise of `image`, stars masked. `None` for a
/// frame with no finite pixel.
#[must_use]
pub fn estimate_background(image: &FitsImage) -> Option<BackgroundEstimate> {
    let tiles_x = (image.width / MIN_TILE_SIDE).clamp(1, MAX_TILES_PER_AXIS);
    let tiles_y = (image.height / MIN_TILE_SIDE).clamp(1, MAX_TILES_PER_AXIS);
    let mut levels = Vec::with_capacity(tiles_x * tiles_y);
    let mut sigmas = Vec::with_capacity(tiles_x * tiles_y);
    for ty in 0..tiles_y {
        let y0 = ty * image.height / tiles_y;
        let y1 = (ty + 1) * image.height / tiles_y;
        for tx in 0..tiles_x {
            let x0 = tx * image.width / tiles_x;
            let x1 = (tx + 1) * image.width / tiles_x;
            let area = (x1 - x0) * (y1 - y0);
            let stride =
                ((area as f64 / MAX_SAMPLES_PER_TILE as f64).sqrt().ceil() as usize).max(1);
            let mut samples = Vec::with_capacity(area / (stride * stride) + 1);
            for y in (y0..y1).step_by(stride) {
                for x in (x0..x1).step_by(stride) {
                    let value = f64::from(image.at(x, y));
                    if value.is_finite() {
                        samples.push(value);
                    }
                }
            }
            if let Some((level, sigma)) = sigma_clipped(&samples) {
                levels.push(level);
                sigmas.push(sigma);
            }
        }
    }
    if levels.is_empty() {
        return None;
    }
    // Tiles far from the others (an extended object, a corner of a
    // gradient) are set aside before the median.
    let mut sorted = levels.clone();
    let centre = median_in_place(&mut sorted)?;
    let spread = robust_sigma(&levels, centre);
    let mut kept: Vec<f64> = Vec::new();
    let mut kept_sigmas: Vec<f64> = Vec::new();
    for (level, sigma) in levels.iter().zip(&sigmas) {
        if spread <= 0.0 || (level - centre).abs() <= 3.0 * spread {
            kept.push(*level);
            kept_sigmas.push(*sigma);
        }
    }
    let tiles = kept.len();
    Some(BackgroundEstimate {
        level: median_in_place(&mut kept)?,
        noise: median_in_place(&mut kept_sigmas)?,
        tiles,
    })
}

/// Where a pixel scale was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScaleSource {
    /// The WCS linear transform (`CD`, or `CDELT` with `PC`/`CROTA2`).
    Wcs,
    /// A scale keyword: `PIXSCALE`, `SCALE` or `SECPIX`.
    Keyword,
    /// `XPIXSZ` (micrometres, binning included) over `FOCALLEN` (millimetres).
    Optics,
}

/// The scale of one sensor pixel, in arcseconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelScale {
    pub arcsec_per_pixel: f64,
    pub source: ScaleSource,
}

/// The WCS linear transform, in degrees per pixel: `[[CD1_1, CD1_2],
/// [CD2_1, CD2_2]]`, from the `CD` keywords, else from `CDELT1`/`CDELT2`
/// with a `PC` matrix or a `CROTA2` angle (`CROTA2` absent: no rotation).
#[must_use]
pub fn header_cd_matrix(header: &FitsHeader) -> Option<[[f64; 2]; 2]> {
    let cd = |key: &str| header.get_f64(key);
    if let (Some(a), Some(d)) = (cd("CD1_1"), cd("CD2_2")) {
        let matrix = [
            [a, cd("CD1_2").unwrap_or(0.0)],
            [cd("CD2_1").unwrap_or(0.0), d],
        ];
        return finite_and_invertible(matrix);
    }
    let (cdelt1, cdelt2) = (cd("CDELT1")?, cd("CDELT2")?);
    let pc = if let (Some(p11), Some(p22)) = (cd("PC1_1"), cd("PC2_2")) {
        [
            [p11, cd("PC1_2").unwrap_or(0.0)],
            [cd("PC2_1").unwrap_or(0.0), p22],
        ]
    } else {
        let rotation = cd("CROTA2").unwrap_or(0.0).to_radians();
        let (sin, cos) = rotation.sin_cos();
        // The classic AIPS convention: CROTA2 turns both axes together.
        [[cos, -sin * cdelt2 / cdelt1], [sin * cdelt1 / cdelt2, cos]]
    };
    finite_and_invertible([
        [cdelt1 * pc[0][0], cdelt1 * pc[0][1]],
        [cdelt2 * pc[1][0], cdelt2 * pc[1][1]],
    ])
}

fn finite_and_invertible(matrix: [[f64; 2]; 2]) -> Option<[[f64; 2]; 2]> {
    let determinant = matrix[0][0] * matrix[1][1] - matrix[0][1] * matrix[1][0];
    (matrix.iter().flatten().all(|value| value.is_finite()) && determinant != 0.0).then_some(matrix)
}

/// The scale of one sensor pixel, from the header (see the module comment
/// for the order). `None` when the header says nothing usable.
#[must_use]
pub fn sensor_pixel_scale(header: &FitsHeader) -> Option<PixelScale> {
    if let Some(matrix) = header_cd_matrix(header) {
        let determinant = matrix[0][0] * matrix[1][1] - matrix[0][1] * matrix[1][0];
        return Some(PixelScale {
            arcsec_per_pixel: determinant.abs().sqrt() * 3600.0,
            source: ScaleSource::Wcs,
        });
    }
    for keyword in ["PIXSCALE", "SCALE", "SECPIX"] {
        if let Some(scale) = header
            .get_f64(keyword)
            .filter(|s| s.is_finite() && *s > 0.0)
        {
            return Some(PixelScale {
                arcsec_per_pixel: scale,
                source: ScaleSource::Keyword,
            });
        }
    }
    let pixel_microns = super::fits::pixel_size_microns(header)?;
    let focal_mm = super::fits::focal_length_mm(header)?;
    Some(PixelScale {
        arcsec_per_pixel: 206.264_806 * pixel_microns / focal_mm,
        source: ScaleSource::Optics,
    })
}

/// The exposure, in seconds: `EXPTIME`, else `EXPOSURE` in its unit.
#[must_use]
pub fn exposure_seconds(header: &FitsHeader) -> Option<f64> {
    super::fits::exposure_seconds(header).filter(|seconds| seconds.is_finite() && *seconds > 0.0)
}

/// A background level above the pedestal, in ADU per image pixel over
/// `exposure_seconds`, as ADU per second and per square arcsecond, for
/// image pixels of `image_arcsec_per_pixel` on a side.
#[must_use]
pub fn sky_rate(
    level_above_pedestal: f64,
    exposure_seconds: f64,
    image_arcsec_per_pixel: f64,
) -> f64 {
    level_above_pedestal / (exposure_seconds * image_arcsec_per_pixel * image_arcsec_per_pixel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::fits::parse_primary_header;
    use crate::analysis::sky_synthetic::{
        Rng, Storage, SyntheticFrame, SyntheticStar, header_bytes, quoted,
    };

    fn header_of(cards: &[(&str, &str)]) -> FitsHeader {
        let owned: Vec<(String, String)> = cards
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        parse_primary_header(&header_bytes(&owned)).unwrap()
    }

    fn crowded_frame(storage: Storage) -> SyntheticFrame {
        let mut frame = SyntheticFrame::new(256, 192, 300.0);
        frame.pedestal_adu = 500.0;
        frame.storage = storage;
        let mut rng = Rng::new(7);
        for _ in 0..150 {
            frame.stars.push(SyntheticStar::point(
                1.0 + rng.uniform() * 255.0,
                1.0 + rng.uniform() * 191.0,
                2_000.0 + rng.uniform() * 60_000.0,
            ));
        }
        frame
    }

    #[test]
    fn reads_an_unsigned_16_bit_frame_in_physical_adu() {
        let mut frame = SyntheticFrame::new(40, 30, 1_000.0);
        frame.read_noise_adu = 0.0;
        frame.gain = 1e12; // no shot noise: every pixel is exactly the sky
        frame.pedestal_adu = 40_000.0; // above 32767: BZERO must be applied
        let image = read_fits_image(&frame.to_fits(&[])).unwrap();
        assert_eq!((image.width, image.height, image.superpixel), (40, 30, 1));
        assert!(image.pixels.iter().all(|value| *value == 41_000.0));
    }

    #[test]
    fn reads_a_float_frame_and_keeps_its_fractions() {
        let mut frame = SyntheticFrame::new(32, 32, 0.125);
        frame.storage = Storage::Float32;
        frame.read_noise_adu = 0.0;
        frame.gain = 1e12;
        let image = read_fits_image(&frame.to_fits(&[])).unwrap();
        assert!(
            image
                .pixels
                .iter()
                .all(|value| (*value - 0.125).abs() < 1e-5)
        );
    }

    #[test]
    fn refuses_a_compressed_an_empty_or_a_truncated_file() {
        let empty = header_bytes(&[
            ("SIMPLE".into(), "T".into()),
            ("BITPIX".into(), "8".into()),
            ("NAXIS".into(), "0".into()),
        ]);
        assert!(matches!(
            read_fits_image(&empty),
            Err(PixelReadError::NotAnImage(_))
        ));
        let cube = header_bytes(&[
            ("SIMPLE".into(), "T".into()),
            ("BITPIX".into(), "16".into()),
            ("NAXIS".into(), "3".into()),
            ("NAXIS1".into(), "4".into()),
            ("NAXIS2".into(), "4".into()),
            ("NAXIS3".into(), "3".into()),
        ]);
        assert!(matches!(
            read_fits_image(&cube),
            Err(PixelReadError::NotAnImage(_))
        ));
        let mut cut = SyntheticFrame::new(64, 64, 10.0).to_fits(&[]);
        cut.truncate(cut.len() - 4_000);
        assert!(matches!(
            read_fits_image(&cut),
            Err(PixelReadError::Truncated { .. })
        ));
    }

    #[test]
    fn a_colour_frame_is_read_as_summed_superpixels() {
        let mut frame = SyntheticFrame::new(64, 48, 100.0);
        frame.read_noise_adu = 0.0;
        frame.gain = 1e12;
        let image = read_fits_image(&frame.to_fits(&[("BAYERPAT", quoted("RGGB"))])).unwrap();
        assert_eq!((image.width, image.height, image.superpixel), (32, 24, 2));
        assert!(image.pixels.iter().all(|value| *value == 400.0));
        // A point of the sensor lands in the superpixel that holds it.
        assert_eq!(image.sensor_to_image(1.5, 1.5), (1.0, 1.0));
        assert_eq!(image.sensor_to_image(3.5, 2.5), (2.0, 1.5));
    }

    #[test]
    fn the_background_ignores_the_stars_and_an_extended_patch() {
        for storage in [Storage::Unsigned16, Storage::Float32] {
            let mut frame = crowded_frame(storage);
            // A bright, wide patch (a galaxy) covering a corner.
            frame.stars.push(SyntheticStar {
                x: 40.0,
                y: 40.0,
                flux_adu: 3.0e6,
                sigma: Some(15.0),
            });
            let image = read_fits_image(&frame.to_fits(&[])).unwrap();
            let background = estimate_background(&image).unwrap();
            let truth = 800.0;
            let mean =
                image.pixels.iter().map(|v| f64::from(*v)).sum::<f64>() / image.pixels.len() as f64;
            assert!(
                mean - truth > 20.0,
                "the stars lift the plain mean ({mean}): the test frame is crowded enough"
            );
            assert!(
                (background.level - truth).abs() < 2.0,
                "background {} for a truth of {truth}",
                background.level
            );
            // Shot noise of 300 ADU of sky with a read noise of 3: ~17.6,
            // a little more with what remains of the wings.
            assert!(
                (17.0..20.5).contains(&background.noise),
                "{}",
                background.noise
            );
        }
    }

    #[test]
    fn saturated_pixels_are_flagged_from_the_header_or_the_brightest_pixel() {
        let mut frame = SyntheticFrame::new(64, 64, 200.0);
        frame.stars.push(SyntheticStar::point(32.0, 32.0, 5.0e6));
        let image = read_fits_image(&frame.to_fits(&[])).unwrap();
        assert!(image.saturated[31 * 64 + 31], "the clipped core is flagged");
        assert!(
            !image.saturated[31 * 64 + 28],
            "a bright wing below the full well is not"
        );
        assert!(!image.saturated[5 * 64 + 5], "the sky is not");
        let declared = read_fits_image(&frame.to_fits(&[("SATURATE", "1000.0".into())])).unwrap();
        assert!(
            declared.saturated[31 * 64 + 28],
            "a lower declared level reaches the wings"
        );
    }

    #[test]
    fn the_scale_comes_from_the_wcs_then_a_keyword_then_the_optics() {
        let cd = header_of(&[
            ("CD1_1", "-5.0E-4"),
            ("CD1_2", "1.0E-5"),
            ("CD2_1", "1.0E-5"),
            ("CD2_2", "5.0E-4"),
            ("PIXSCALE", "9.9"),
        ]);
        let scale = sensor_pixel_scale(&cd).unwrap();
        assert_eq!(scale.source, ScaleSource::Wcs);
        assert!((scale.arcsec_per_pixel - 1.8).abs() < 1e-3);

        let cdelt = header_of(&[
            ("CDELT1", "-2.5E-4"),
            ("CDELT2", "2.5E-4"),
            ("CROTA2", "30.0"),
        ]);
        let scale = sensor_pixel_scale(&cdelt).unwrap();
        assert_eq!(scale.source, ScaleSource::Wcs);
        assert!((scale.arcsec_per_pixel - 0.9).abs() < 1e-9);

        let keyword = header_of(&[("PIXSCALE", "1.25")]);
        assert_eq!(
            sensor_pixel_scale(&keyword),
            Some(PixelScale {
                arcsec_per_pixel: 1.25,
                source: ScaleSource::Keyword
            })
        );

        let optics = header_of(&[("XPIXSZ", "3.76"), ("FOCALLEN", "400.0")]);
        let scale = sensor_pixel_scale(&optics).unwrap();
        assert_eq!(scale.source, ScaleSource::Optics);
        assert!((scale.arcsec_per_pixel - 1.9389).abs() < 1e-3);

        assert_eq!(sensor_pixel_scale(&header_of(&[("GAIN", "100")])), None);
    }

    #[test]
    fn the_sky_rate_is_per_second_and_per_square_arcsecond() {
        // 480 ADU over 120 s on 2" pixels: 1 ADU/s/arcsec².
        assert!((sky_rate(480.0, 120.0, 2.0) - 1.0).abs() < 1e-12);
        let header = header_of(&[("EXPOSURE", "60")]);
        assert_eq!(exposure_seconds(&header), Some(60.0));
        assert_eq!(exposure_seconds(&header_of(&[("EXPTIME", "0")])), None);
    }
}

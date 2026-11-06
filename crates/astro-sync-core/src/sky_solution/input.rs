//! The image ASTAP is given.
//!
//! The stacks of a night and the images of a multi-night plan never solved,
//! while any single pose of the same field solves in a tenth of a second.
//! Two causes, both on the side of the input:
//!
//! 1. **The file**: Astro Sync writes its TIFFs with Deflate compression,
//!    which ASTAP cannot read (« Unsupported compression »).
//! 2. **The depth**: ASTAP ranks the stars it detects by their
//!    signal-to-noise ratio measured against the background around each star,
//!    and keeps the 500 best to compare with as many stars of its database
//!    (ranked by magnitude). On one pose that background is uniform noise, so
//!    the ratio follows the brightness. On a deep stack the noise is so low
//!    that the background around a star is the nebula, faint neighbours and
//!    halos: the ratio follows the place, not the brightness, a third of the
//!    500 stars kept are not among the brightest, and the quads no longer
//!    match. The geometry is not at fault (the stack matches a pose to
//!    0.5 px, no shear), nor the registration (affine, similarity and shift
//!    give the same result).
//!
//! So every image is given to ASTAP as an **uncompressed 16-bit mono FITS**
//! written in the solve's work folder ([`write_input`]): the luminance (mean of
//! the colour planes), the same pixels and size, the rows in the FITS
//! convention (row 1 at the bottom: a raster image is turned the way ASTAP
//! itself turns a TIFF), with the position and the scale known as cards.
//! The first attempt is **levelled** ([`Preparation::Levelled`]): uniform
//! Gaussian noise four times the background's texture (the 90th percentile of
//! the clipped deviation of 32-pixel tiles) is added, deterministic, so that
//! the background around every star is that noise again and the ranking
//! follows the brightness, as on a pose. The second attempt, only when the
//! first finds no field, is the same image without noise
//! ([`Preparation::Plain`]). A format this module does not decode (JPEG, a
//! tiled TIFF...) is given to ASTAP as it is.

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::analysis::fits::{FitsHeader, parse_primary_header_with_data_offset};
use crate::analysis::sky::sensor_pixel_scale;
use crate::raster::{ImageSize, is_fits};

/// How the luminance is written for the solver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preparation {
    /// With uniform noise over the background's texture (the first attempt).
    Levelled,
    /// The luminance alone (the second attempt).
    Plain,
}

/// The luminance of an image, in the FITS row order: index `y * width + x`
/// is the pixel of FITS coordinates `(x + 1, y + 1)`; row 0 is the bottom
/// row of a raster image.
#[derive(Debug, Clone)]
pub struct Luminance {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<f32>,
}

/// Why an image is not decoded here: it then goes to the solver as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// A format or a variant this module does not read.
    Unsupported(&'static str),
    /// A file that ends early or contradicts itself.
    Corrupt(&'static str),
}

/// The image written for the solver.
#[derive(Debug, Clone)]
pub struct SolverInput {
    pub path: PathBuf,
    pub size: ImageSize,
    /// The deviation of the noise added, in the units of the file written
    /// (0 for [`Preparation::Plain`]).
    pub added_noise: f32,
}

/// What the solver is told beside the pixels, written as cards.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct InputCards {
    pub ra_deg: Option<f64>,
    pub dec_deg: Option<f64>,
    pub pixel_scale_arcsec: Option<f64>,
}

/* ------------------------------- Decoding -------------------------------- */

/// The luminance of the image at `path` (FITS, TIFF or PNG, by its first
/// bytes), and its FITS header when it is a FITS file.
pub fn read_luminance(path: &Path) -> Result<(Luminance, Option<FitsHeader>), DecodeError> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|_| DecodeError::Corrupt("unreadable file"))?;
    if is_fits(&bytes) {
        let (luminance, header) = fits_luminance(&bytes)?;
        return Ok((luminance, Some(header)));
    }
    if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        return tiff_luminance(&bytes).map(|l| (l, None));
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return png_luminance(&bytes).map(|l| (l, None));
    }
    Err(DecodeError::Unsupported("format"))
}

/// The mean of the first `colours` samples of each pixel, rows reversed (a
/// raster image's first row is its top).
fn raster_luminance(
    width: usize,
    height: usize,
    samples: usize,
    values: &[f32],
) -> Result<Luminance, DecodeError> {
    if values.len() < width * height * samples {
        return Err(DecodeError::Corrupt("too few pixels"));
    }
    let colours = if samples >= 3 { 3 } else { 1 };
    let mut pixels = vec![0_f32; width * height];
    for row in 0..height {
        let target = height - 1 - row;
        for x in 0..width {
            let at = (row * width + x) * samples;
            let sum: f32 = values[at..at + colours].iter().sum();
            pixels[target * width + x] = sum / colours as f32;
        }
    }
    Ok(Luminance {
        width,
        height,
        pixels,
    })
}

fn fits_luminance(bytes: &[u8]) -> Result<(Luminance, FitsHeader), DecodeError> {
    let (header, offset) = parse_primary_header_with_data_offset(bytes)
        .map_err(|_| DecodeError::Corrupt("FITS header"))?;
    if header
        .get("ZIMAGE")
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("T"))
    {
        return Err(DecodeError::Unsupported("tile-compressed FITS"));
    }
    let naxis = header.get_i64("NAXIS").unwrap_or(0);
    let planes = if naxis == 3 {
        header.get_i64("NAXIS3").unwrap_or(1)
    } else {
        1
    };
    if !(naxis == 2 || naxis == 3) || !(1..=4).contains(&planes) {
        return Err(DecodeError::Unsupported("FITS without a 2-D picture"));
    }
    let dimension = |key: &str| {
        header
            .get_i64(key)
            .and_then(|v| usize::try_from(v).ok())
            .filter(|v| *v > 0)
            .ok_or(DecodeError::Corrupt("FITS dimension"))
    };
    let (width, height) = (dimension("NAXIS1")?, dimension("NAXIS2")?);
    let planes = planes as usize;
    let bitpix = header.get_i64("BITPIX").unwrap_or(0);
    let size = match bitpix {
        8 => 1,
        16 => 2,
        32 | -32 => 4,
        64 | -64 => 8,
        _ => return Err(DecodeError::Unsupported("BITPIX")),
    };
    let plane = width * height;
    let end = offset + plane * planes * size;
    let data = bytes
        .get(offset..end)
        .ok_or(DecodeError::Corrupt("FITS data unit"))?;
    let bzero = header.get_f64("BZERO").unwrap_or(0.0);
    let bscale = header.get_f64("BSCALE").unwrap_or(1.0);
    let value = |chunk: &[u8]| -> f64 {
        let raw = match bitpix {
            8 => f64::from(chunk[0]),
            16 => f64::from(i16::from_be_bytes([chunk[0], chunk[1]])),
            32 => f64::from(i32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])),
            64 => i64::from_be_bytes(chunk.try_into().unwrap_or([0; 8])) as f64,
            -32 => f64::from(f32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])),
            _ => f64::from_be_bytes(chunk.try_into().unwrap_or([0; 8])),
        };
        bzero + bscale * raw
    };
    // A colour cube: the mean of its first three planes (a fourth is alpha).
    let colours = planes.min(3);
    let mut pixels = vec![0_f32; plane];
    for p in 0..colours {
        let start = p * plane * size;
        for (i, chunk) in data[start..start + plane * size]
            .chunks_exact(size)
            .enumerate()
        {
            pixels[i] += (value(chunk) / colours as f64) as f32;
        }
    }
    Ok((
        Luminance {
            width,
            height,
            pixels,
        },
        header,
    ))
}

/* --------------------------------- TIFF ---------------------------------- */

struct Reader<'a> {
    bytes: &'a [u8],
    little: bool,
}

impl Reader<'_> {
    fn u16(&self, at: usize) -> Option<u16> {
        let b = self.bytes.get(at..at + 2)?;
        Some(if self.little {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
    }

    fn u32(&self, at: usize) -> Option<u32> {
        let b = self.bytes.get(at..at + 4)?;
        let quad = [b[0], b[1], b[2], b[3]];
        Some(if self.little {
            u32::from_le_bytes(quad)
        } else {
            u32::from_be_bytes(quad)
        })
    }

    /// The values of the entry at `entry` (SHORT or LONG), wherever they lie.
    fn values(&self, entry: usize) -> Option<Vec<u32>> {
        let kind = self.u16(entry + 2)?;
        let count = self.u32(entry + 4)? as usize;
        let width = match kind {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 => 4,
            _ => return None,
        };
        let start = if count * width <= 4 {
            entry + 8
        } else {
            self.u32(entry + 8)? as usize
        };
        (0..count)
            .map(|i| match width {
                1 => self.bytes.get(start + i).map(|b| u32::from(*b)),
                2 => self.u16(start + 2 * i).map(u32::from),
                _ => self.u32(start + 4 * i),
            })
            .collect()
    }
}

fn tiff_luminance(bytes: &[u8]) -> Result<Luminance, DecodeError> {
    let reader = Reader {
        bytes,
        little: bytes.starts_with(b"II"),
    };
    let corrupt = DecodeError::Corrupt("TIFF directory");
    let ifd = reader.u32(4).ok_or(corrupt.clone())? as usize;
    let count = usize::from(reader.u16(ifd).ok_or(corrupt.clone())?);
    let mut tags = std::collections::BTreeMap::new();
    for i in 0..count {
        let entry = ifd + 2 + 12 * i;
        let tag = reader.u16(entry).ok_or(corrupt.clone())?;
        if let Some(values) = reader.values(entry) {
            tags.insert(tag, values);
        }
    }
    let one = |tag: u16, default: Option<u32>| -> Result<u32, DecodeError> {
        tags.get(&tag)
            .and_then(|v| v.first().copied())
            .or(default)
            .ok_or(DecodeError::Corrupt("TIFF tag missing"))
    };
    if tags.contains_key(&322) {
        return Err(DecodeError::Unsupported("tiled TIFF"));
    }
    let width = one(256, None)? as usize;
    let height = one(257, None)? as usize;
    let samples = one(277, Some(1))? as usize;
    let bits = one(258, Some(1))?;
    let compression = one(259, Some(1))?;
    let photometric = one(262, Some(1))?;
    let planar = one(284, Some(1))?;
    let predictor = one(317, Some(1))?;
    let format = one(339, Some(1))?;
    let rows_per_strip = (one(278, Some(height as u32))? as usize).clamp(1, height.max(1));
    if width == 0 || height == 0 || !(1..=4).contains(&samples) {
        return Err(DecodeError::Corrupt("TIFF size"));
    }
    if photometric == 3 || !matches!(bits, 8 | 16 | 32 | 64) || predictor == 3 {
        return Err(DecodeError::Unsupported("TIFF sample layout"));
    }
    if format == 3 && !matches!(bits, 32 | 64) {
        return Err(DecodeError::Unsupported("TIFF float width"));
    }
    let offsets = tags.get(&273).ok_or(corrupt.clone())?;
    let lengths = tags.get(&279).ok_or(corrupt)?;
    if offsets.len() != lengths.len() {
        return Err(DecodeError::Corrupt("TIFF strips"));
    }
    let bytes_per_sample = bits as usize / 8;
    // One plane of chunky pixels, or one plane per sample.
    let (strip_samples, plane_count) = if planar == 2 {
        (1, samples)
    } else {
        (samples, 1)
    };
    let row_bytes = width * strip_samples * bytes_per_sample;
    let strips_per_plane = height.div_ceil(rows_per_strip);
    if offsets.len() < strips_per_plane * plane_count {
        return Err(DecodeError::Corrupt("TIFF strip count"));
    }
    let mut values = vec![0_f32; width * height * samples];
    for plane in 0..plane_count {
        for strip in 0..strips_per_plane {
            let index = plane * strips_per_plane + strip;
            let start = offsets[index] as usize;
            let raw = bytes
                .get(start..start + lengths[index] as usize)
                .ok_or(DecodeError::Corrupt("TIFF strip"))?;
            let first_row = strip * rows_per_strip;
            let rows = rows_per_strip.min(height - first_row);
            let data = decompress(compression, raw, rows * row_bytes)?;
            if data.len() < rows * row_bytes {
                return Err(DecodeError::Corrupt("TIFF strip too short"));
            }
            for row in 0..rows {
                let line = &data[row * row_bytes..(row + 1) * row_bytes];
                let mut row_values = sample_values(line, bits, format, reader.little);
                if predictor == 2 {
                    undo_horizontal_difference(&mut row_values, strip_samples, bits, format);
                }
                let y = first_row + row;
                for x in 0..width {
                    for s in 0..strip_samples {
                        let sample = if planar == 2 { plane } else { s };
                        values[(y * width + x) * samples + sample] =
                            row_values[x * strip_samples + s] as f32;
                    }
                }
            }
        }
    }
    raster_luminance(width, height, samples, &values)
}

fn sample_values(line: &[u8], bits: u32, format: u32, little: bool) -> Vec<f64> {
    let size = bits as usize / 8;
    line.chunks_exact(size)
        .map(|c| {
            let mut b = [0_u8; 8];
            if little {
                b[..size].copy_from_slice(c);
            } else {
                for (i, byte) in c.iter().rev().enumerate() {
                    b[i] = *byte;
                }
            }
            match (bits, format) {
                (8, 2) => f64::from(b[0] as i8),
                (8, _) => f64::from(b[0]),
                (16, 2) => f64::from(i16::from_le_bytes([b[0], b[1]])),
                (16, _) => f64::from(u16::from_le_bytes([b[0], b[1]])),
                (32, 3) => f64::from(f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                (32, 2) => f64::from(i32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                (32, _) => f64::from(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                (64, 3) => f64::from_le_bytes(b),
                (64, 2) => i64::from_le_bytes(b) as f64,
                _ => u64::from_le_bytes(b) as f64,
            }
        })
        .collect()
}

/// TIFF predictor 2: each sample is the difference with the same sample of
/// the pixel before it, modulo the sample's width.
fn undo_horizontal_difference(values: &mut [f64], samples: usize, bits: u32, format: u32) {
    if format == 3 {
        return;
    }
    let modulus = 2_f64.powi(bits as i32);
    for i in samples..values.len() {
        let mut v = values[i] + values[i - samples];
        if format == 2 {
            let half = modulus / 2.0;
            v = (v + half).rem_euclid(modulus) - half;
        } else {
            v = v.rem_euclid(modulus);
        }
        values[i] = v;
    }
}

fn decompress(compression: u32, raw: &[u8], expected: usize) -> Result<Vec<u8>, DecodeError> {
    match compression {
        1 => Ok(raw.to_vec()),
        8 | 32946 => {
            let mut out = Vec::with_capacity(expected);
            flate2::read::ZlibDecoder::new(raw)
                .read_to_end(&mut out)
                .map_err(|_| DecodeError::Corrupt("Deflate stream"))?;
            Ok(out)
        }
        5 => lzw_decode(raw, expected),
        32773 => Ok(packbits_decode(raw, expected)),
        _ => Err(DecodeError::Unsupported("TIFF compression")),
    }
}

/// TIFF's LZW (MSB first, early change).
fn lzw_decode(raw: &[u8], expected: usize) -> Result<Vec<u8>, DecodeError> {
    const CLEAR: usize = 256;
    const END: usize = 257;
    let mut out = Vec::with_capacity(expected);
    let mut table: Vec<Vec<u8>> = Vec::with_capacity(4096);
    let reset = |table: &mut Vec<Vec<u8>>| {
        table.clear();
        table.extend((0..=255_u8).map(|b| vec![b]));
        table.push(Vec::new());
        table.push(Vec::new());
    };
    reset(&mut table);
    let (mut bit, mut width) = (0_usize, 9_usize);
    let mut previous: Option<usize> = None;
    let total_bits = raw.len() * 8;
    while bit + width <= total_bits {
        let mut code = 0_usize;
        for i in 0..width {
            let at = bit + i;
            code = (code << 1) | usize::from((raw[at / 8] >> (7 - at % 8)) & 1);
        }
        bit += width;
        if code == END {
            break;
        }
        if code == CLEAR {
            reset(&mut table);
            width = 9;
            previous = None;
            continue;
        }
        let entry = if code < table.len() {
            table[code].clone()
        } else if let Some(p) = previous {
            let mut e = table[p].clone();
            e.push(table[p][0]);
            e
        } else {
            return Err(DecodeError::Corrupt("LZW code"));
        };
        out.extend_from_slice(&entry);
        if let Some(p) = previous.filter(|_| table.len() < 4096) {
            let mut added = table[p].clone();
            added.push(entry[0]);
            table.push(added);
        }
        previous = Some(code);
        width = match table.len() + 1 {
            n if n >= 2048 => 12,
            n if n >= 1024 => 11,
            n if n >= 512 => 10,
            _ => 9,
        };
        if table.len() >= 4095 {
            width = 12;
        }
    }
    Ok(out)
}

fn packbits_decode(raw: &[u8], expected: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(expected);
    let mut i = 0;
    while i < raw.len() && out.len() < expected {
        let n = raw[i] as i8;
        i += 1;
        if n >= 0 {
            let count = n as usize + 1;
            out.extend_from_slice(&raw[i..(i + count).min(raw.len())]);
            i += count;
        } else if n != -128 {
            if let Some(byte) = raw.get(i) {
                out.extend(std::iter::repeat_n(*byte, (1 - i32::from(n)) as usize));
            }
            i += 1;
        }
    }
    out
}

/* ---------------------------------- PNG ---------------------------------- */

fn png_luminance(bytes: &[u8]) -> Result<Luminance, DecodeError> {
    let mut at = 8;
    let (mut width, mut height, mut depth, mut colour, mut interlace) = (0, 0, 0, 0, 0);
    let mut compressed = Vec::new();
    while at + 8 <= bytes.len() {
        let length = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap_or([0; 4])) as usize;
        let kind = &bytes[at + 4..at + 8];
        let data = bytes
            .get(at + 8..at + 8 + length)
            .ok_or(DecodeError::Corrupt("PNG chunk"))?;
        match kind {
            b"IHDR" if length >= 13 => {
                width = u32::from_be_bytes(data[0..4].try_into().unwrap_or([0; 4])) as usize;
                height = u32::from_be_bytes(data[4..8].try_into().unwrap_or([0; 4])) as usize;
                depth = data[8];
                colour = data[9];
                interlace = data[12];
            }
            b"IDAT" => compressed.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        at += 12 + length;
    }
    let samples = match colour {
        0 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => return Err(DecodeError::Unsupported("PNG palette")),
    };
    if !matches!(depth, 8 | 16) || interlace != 0 {
        return Err(DecodeError::Unsupported("PNG depth or interlace"));
    }
    if width == 0 || height == 0 || compressed.is_empty() {
        return Err(DecodeError::Corrupt("PNG image"));
    }
    let bytes_per_sample = usize::from(depth / 8);
    let pixel = samples * bytes_per_sample;
    let stride = width * pixel;
    let mut data = Vec::with_capacity((stride + 1) * height);
    flate2::read::ZlibDecoder::new(compressed.as_slice())
        .read_to_end(&mut data)
        .map_err(|_| DecodeError::Corrupt("PNG data"))?;
    if data.len() < (stride + 1) * height {
        return Err(DecodeError::Corrupt("PNG too short"));
    }
    let mut previous = vec![0_u8; stride];
    let mut values = Vec::with_capacity(width * height * samples);
    for row in 0..height {
        let filter = data[row * (stride + 1)];
        let mut line = data[row * (stride + 1) + 1..(row + 1) * (stride + 1)].to_vec();
        for i in 0..stride {
            let left = if i >= pixel { line[i - pixel] } else { 0 };
            let up = previous[i];
            let corner = if i >= pixel { previous[i - pixel] } else { 0 };
            let add = match filter {
                0 => 0,
                1 => left,
                2 => up,
                3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
                4 => paeth(left, up, corner),
                _ => return Err(DecodeError::Corrupt("PNG filter")),
            };
            line[i] = line[i].wrapping_add(add);
        }
        for sample in line.chunks_exact(bytes_per_sample) {
            values.push(if bytes_per_sample == 2 {
                f32::from(u16::from_be_bytes([sample[0], sample[1]]))
            } else {
                f32::from(sample[0])
            });
        }
        previous = line;
    }
    raster_luminance(width, height, samples, &values)
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let (pa, pb, pc) = (
        (p - i16::from(a)).abs(),
        (p - i16::from(b)).abs(),
        (p - i16::from(c)).abs(),
    );
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/* ------------------------------- Levelling ------------------------------- */

fn median(values: &mut [f32]) -> f32 {
    if values.is_empty() {
        return f32::NAN;
    }
    let middle = values.len() / 2;
    let (_, m, _) = values.select_nth_unstable_by(middle, f32::total_cmp);
    *m
}

/// The clipped deviation of one tile: three-sigma clipping around the median
/// (the stars and their wings out), twice.
fn tile_deviation(values: &mut Vec<f32>) -> Option<f32> {
    values.retain(|v| v.is_finite());
    if values.len() < 16 {
        return None;
    }
    let mut centre = median(values);
    let mut spread = {
        let mut deviations: Vec<f32> = values.iter().map(|v| (v - centre).abs()).collect();
        1.4826 * median(&mut deviations)
    };
    for _ in 0..2 {
        if spread <= 0.0 {
            break;
        }
        let kept: Vec<f32> = values
            .iter()
            .copied()
            .filter(|v| (v - centre).abs() < 3.0 * spread)
            .collect();
        if kept.len() < 8 {
            break;
        }
        let n = kept.len() as f64;
        let mean = kept.iter().map(|v| f64::from(*v)).sum::<f64>() / n;
        let variance = kept
            .iter()
            .map(|v| (f64::from(*v) - mean).powi(2))
            .sum::<f64>()
            / n;
        centre = mean as f32;
        spread = variance.sqrt() as f32;
    }
    Some(spread)
}

/// The texture of the background: the 90th percentile of the clipped
/// deviation of the 32-pixel tiles.
#[must_use]
pub fn background_texture(luminance: &Luminance) -> Option<f32> {
    const TILE: usize = 32;
    let (w, h) = (luminance.width, luminance.height);
    let mut deviations = Vec::new();
    let mut tile = Vec::with_capacity(TILE * TILE);
    for ty in 0..h / TILE {
        for tx in 0..w / TILE {
            tile.clear();
            for y in ty * TILE..(ty + 1) * TILE {
                tile.extend_from_slice(
                    &luminance.pixels[y * w + tx * TILE..y * w + (tx + 1) * TILE],
                );
            }
            if let Some(d) = tile_deviation(&mut tile) {
                deviations.push(d);
            }
        }
    }
    if deviations.is_empty() {
        return None;
    }
    let at = (deviations.len() * 9 / 10).min(deviations.len() - 1);
    let (_, p90, _) = deviations.select_nth_unstable_by(at, f32::total_cmp);
    Some(*p90).filter(|d| d.is_finite() && *d > 0.0)
}

/// The noise added to the background: four times its texture.
pub const LEVELLING_FACTOR: f32 = 4.0;

/// A small deterministic generator (SplitMix64): the same image is always
/// given the same noise, so a retry is the same attempt.
struct Noise(u64);

impl Noise {
    fn uniform(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        ((z >> 11) as f64 + 0.5) / (1_u64 << 53) as f64
    }

    /// Two standard normal values (Box-Muller).
    fn pair(&mut self) -> (f64, f64) {
        let (u, v) = (self.uniform(), self.uniform());
        let r = (-2.0 * u.ln()).sqrt();
        let angle = std::f64::consts::TAU * v;
        (r * angle.cos(), r * angle.sin())
    }
}

/// Adds uniform Gaussian noise of deviation `LEVELLING_FACTOR` times the
/// background's texture; returns that deviation (0 when the texture cannot
/// be measured, the image left as it is).
pub fn level(luminance: &mut Luminance) -> f32 {
    let Some(texture) = background_texture(luminance) else {
        return 0.0;
    };
    let sigma = f64::from(LEVELLING_FACTOR * texture);
    let mut noise = Noise(0x0A57_0517_C000_0014);
    for pair in luminance.pixels.chunks_mut(2) {
        let (a, b) = noise.pair();
        pair[0] += (a * sigma) as f32;
        if let Some(second) = pair.get_mut(1) {
            *second += (b * sigma) as f32;
        }
    }
    sigma as f32
}

/* -------------------------------- Writing -------------------------------- */

fn card(keyword: &str, value: &str, comment: &str) -> [u8; 80] {
    let text = if comment.is_empty() {
        format!("{keyword:<8}= {value:>20}")
    } else {
        format!("{keyword:<8}= {value:>20} / {comment}")
    };
    let mut out = [b' '; 80];
    for (slot, byte) in out.iter_mut().zip(text.bytes().filter(u8::is_ascii)) {
        *slot = byte;
    }
    out
}

/// The luminance as an uncompressed 16-bit FITS: the darkest 0.1 % at 500,
/// the brightest pixel at 64 500, the hints as cards. Returns the bytes and
/// the factor from the luminance's units to the file's.
#[must_use]
pub fn fits16(luminance: &Luminance, cards: &InputCards) -> (Vec<u8>, f32) {
    let finite: Vec<f32> = luminance
        .pixels
        .iter()
        .step_by(7)
        .copied()
        .filter(|v| v.is_finite())
        .collect();
    let mut sorted = finite;
    let low = if sorted.is_empty() {
        0.0
    } else {
        let at = sorted.len() / 1000;
        *sorted.select_nth_unstable_by(at, f32::total_cmp).1
    };
    let high = luminance
        .pixels
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(f32::MIN, f32::max);
    let factor = if high > low {
        64_000.0 / (high - low)
    } else {
        1.0
    };
    let mut header: Vec<[u8; 80]> = vec![
        card("SIMPLE", "T", "Astro Sync, image for the sky solver"),
        card("BITPIX", "16", ""),
        card("NAXIS", "2", ""),
        card("NAXIS1", &luminance.width.to_string(), ""),
        card("NAXIS2", &luminance.height.to_string(), ""),
        card("BZERO", "32768", ""),
        card("BSCALE", "1", ""),
    ];
    if let (Some(ra), Some(dec)) = (cards.ra_deg, cards.dec_deg) {
        header.push(card("RA", &format!("{ra:.6}"), "[deg] target"));
        header.push(card("DEC", &format!("{dec:.6}"), "[deg] target"));
    }
    if let Some(scale) = cards.pixel_scale_arcsec {
        header.push(card("PIXSCALE", &format!("{scale:.5}"), "[arcsec/px]"));
    }
    let mut end = [b' '; 80];
    end[..3].copy_from_slice(b"END");
    header.push(end);
    let mut bytes: Vec<u8> = header.concat();
    bytes.resize(bytes.len().div_ceil(2880) * 2880, b' ');
    bytes.reserve(luminance.pixels.len() * 2 + 2880);
    for v in &luminance.pixels {
        let scaled = if v.is_finite() {
            ((v - low) * factor + 500.0).round().clamp(0.0, 65_535.0)
        } else {
            500.0
        };
        let stored = (scaled as i32 - 32_768) as i16;
        bytes.extend_from_slice(&stored.to_be_bytes());
    }
    bytes.resize(bytes.len().div_ceil(2880) * 2880, 0);
    (bytes, factor)
}

/* --------------------------------- Hints --------------------------------- */

/// The scale of the image's pixels from its own header, else from a FITS of
/// the same size beside it (a stack's TIFF and its integration master).
#[must_use]
pub fn scale_hint(image: &Path, header: Option<&FitsHeader>, size: ImageSize) -> Option<f64> {
    if let Some(scale) = header.and_then(sensor_pixel_scale) {
        return Some(scale.arcsec_per_pixel);
    }
    let folder = image.parent()?;
    let mut siblings: Vec<PathBuf> = std::fs::read_dir(folder)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p != image
                && p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                    matches!(e.to_ascii_lowercase().as_str(), "fit" | "fits" | "fts")
                })
        })
        .collect();
    siblings.sort();
    siblings.into_iter().find_map(|path| {
        let header = crate::analysis::fits::read_primary_header_file(&path).ok()?;
        let same = header.get_i64("NAXIS1") == Some(i64::from(size.width))
            && header.get_i64("NAXIS2") == Some(i64::from(size.height));
        same.then(|| sensor_pixel_scale(&header))
            .flatten()
            .map(|s| s.arcsec_per_pixel)
    })
}

/// Writes `luminance` for the solver as `<work_dir>/input-<preparation>.fit`.
pub fn write_input(
    luminance: &Luminance,
    work_dir: &Path,
    preparation: Preparation,
    cards: &InputCards,
) -> std::io::Result<SolverInput> {
    let mut written = luminance.clone();
    let added = match preparation {
        Preparation::Levelled => level(&mut written),
        Preparation::Plain => 0.0,
    };
    let (bytes, factor) = fits16(&written, cards);
    let name = match preparation {
        Preparation::Levelled => "input-levelled.fit",
        Preparation::Plain => "input-plain.fit",
    };
    let path = work_dir.join(name);
    std::fs::write(&path, bytes)?;
    Ok(SolverInput {
        path,
        size: ImageSize {
            width: u32::try_from(written.width).unwrap_or(u32::MAX),
            height: u32::try_from(written.height).unwrap_or(u32::MAX),
        },
        added_noise: added * factor,
    })
}

#[cfg(test)]
pub(crate) mod tests;

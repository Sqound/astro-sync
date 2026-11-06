use super::*;
use crate::test_support::TempDir;
use std::io::Write;

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

/// A little-endian TIFF of `samples` 16-bit samples per pixel, one strip
/// per `rows_per_strip` rows, compressed with `compression` (1 or 32946),
/// with `predictor` (1 or 2): the layout `tifffile` writes for Astro Sync.
pub(crate) fn tiff16(
    width: usize,
    height: usize,
    samples: usize,
    pixels: &[u16],
    compression: u16,
    predictor: u16,
    rows_per_strip: usize,
) -> Vec<u8> {
    let mut strips = Vec::new();
    for first in (0..height).step_by(rows_per_strip) {
        let rows = rows_per_strip.min(height - first);
        let mut raw = Vec::new();
        for y in first..first + rows {
            let line = &pixels[y * width * samples..(y + 1) * width * samples];
            for (i, value) in line.iter().enumerate() {
                let stored = if predictor == 2 && i >= samples {
                    value.wrapping_sub(line[i - samples])
                } else {
                    *value
                };
                raw.extend_from_slice(&stored.to_le_bytes());
            }
        }
        strips.push(if compression == 1 { raw } else { zlib(&raw) });
    }
    let entries: Vec<(u16, u16, Vec<u32>)> = vec![
        (256, 4, vec![width as u32]),
        (257, 4, vec![height as u32]),
        (258, 3, vec![16; samples]),
        (259, 3, vec![u32::from(compression)]),
        (262, 3, vec![if samples == 3 { 2 } else { 1 }]),
        (273, 4, vec![0; strips.len()]),
        (277, 3, vec![samples as u32]),
        (278, 4, vec![rows_per_strip as u32]),
        (279, 4, strips.iter().map(|s| s.len() as u32).collect()),
        (284, 3, vec![1]),
        (317, 3, vec![u32::from(predictor)]),
    ];
    let size_of = |kind: u16| if kind == 3 { 2 } else { 4 };
    // Header, directory, then the arrays too long for an entry, then the
    // strips.
    let ifd_len = 2 + entries.len() * 12 + 4;
    let mut array_at = 8 + ifd_len;
    let mut array_offsets = Vec::new();
    for (_, kind, values) in &entries {
        if values.len() * size_of(*kind) > 4 {
            array_offsets.push(Some(array_at));
            array_at += values.len() * size_of(*kind);
        } else {
            array_offsets.push(None);
        }
    }
    let mut strip_offsets = Vec::new();
    let mut at = array_at;
    for strip in &strips {
        strip_offsets.push(at as u32);
        at += strip.len();
    }
    let mut bytes = b"II*\0".to_vec();
    bytes.extend_from_slice(&8_u32.to_le_bytes());
    bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    let mut array_bytes = Vec::new();
    for ((tag, kind, values), offset) in entries.iter().zip(&array_offsets) {
        let values = if *tag == 273 { &strip_offsets } else { values };
        bytes.extend_from_slice(&tag.to_le_bytes());
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes.extend_from_slice(&(values.len() as u32).to_le_bytes());
        let mut encoded = Vec::new();
        for v in values {
            if size_of(*kind) == 2 {
                encoded.extend_from_slice(&(*v as u16).to_le_bytes());
            } else {
                encoded.extend_from_slice(&v.to_le_bytes());
            }
        }
        if let Some(offset) = offset {
            bytes.extend_from_slice(&(*offset as u32).to_le_bytes());
            array_bytes.extend_from_slice(&encoded);
        } else {
            encoded.resize(4, 0);
            bytes.extend_from_slice(&encoded);
        }
    }
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(&array_bytes);
    for strip in strips {
        bytes.extend_from_slice(&strip);
    }
    bytes
}

/// A 16-bit grey PNG, rows filtered with `Sub` (1) and `Paeth` (4) in turn.
fn png16(width: usize, height: usize, pixels: &[u16]) -> Vec<u8> {
    let line_of = |y: usize| -> Vec<u8> {
        pixels[y * width..(y + 1) * width]
            .iter()
            .flat_map(|v| v.to_be_bytes())
            .collect()
    };
    let mut raw = Vec::new();
    for y in 0..height {
        let filter = if y % 2 == 0 { 1_u8 } else { 4 };
        raw.push(filter);
        let line = line_of(y);
        let previous = if y == 0 {
            vec![0; line.len()]
        } else {
            line_of(y - 1)
        };
        for i in 0..line.len() {
            let left = if i >= 2 { line[i - 2] } else { 0 };
            let corner = if i >= 2 { previous[i - 2] } else { 0 };
            let predicted = if filter == 1 {
                left
            } else {
                paeth(left, previous[i], corner)
            };
            raw.push(line[i].wrapping_sub(predicted));
        }
    }
    let chunk = |kind: &[u8], data: &[u8]| {
        let mut c = (data.len() as u32).to_be_bytes().to_vec();
        c.extend_from_slice(kind);
        c.extend_from_slice(data);
        c.extend_from_slice(&[0, 0, 0, 0]);
        c
    };
    let mut ihdr = (width as u32).to_be_bytes().to_vec();
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[16, 0, 0, 0, 0]);
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend(chunk(b"IHDR", &ihdr));
    bytes.extend(chunk(b"IDAT", &zlib(&raw)));
    bytes.extend(chunk(b"IEND", &[]));
    bytes
}

/// A FITS file of float planes as Siril writes a master, with the optics.
pub(crate) fn fits_float(width: usize, height: usize, planes: &[Vec<f32>]) -> Vec<u8> {
    let mut cards = vec![
        card("SIMPLE", "T", ""),
        card("BITPIX", "-32", ""),
        card("NAXIS", if planes.len() > 1 { "3" } else { "2" }, ""),
        card("NAXIS1", &width.to_string(), ""),
        card("NAXIS2", &height.to_string(), ""),
    ];
    if planes.len() > 1 {
        cards.push(card("NAXIS3", &planes.len().to_string(), ""));
    }
    cards.push(card("FOCALLEN", "190.", ""));
    cards.push(card("XPIXSZ", "3.76", ""));
    let mut end = [b' '; 80];
    end[..3].copy_from_slice(b"END");
    cards.push(end);
    let mut bytes = cards.concat();
    bytes.resize(bytes.len().div_ceil(2880) * 2880, b' ');
    for plane in planes {
        for v in plane {
            bytes.extend_from_slice(&v.to_be_bytes());
        }
    }
    bytes.resize(bytes.len().div_ceil(2880) * 2880, 0);
    bytes
}

#[test]
fn a_deflate_tiff_as_astro_sync_writes_it_is_read_bottom_row_first() {
    let dir = TempDir::new("solve-input-tiff");
    let (width, height) = (5, 4);
    // Pixel (x, y) of the raster: R = 100 x, G = 1000 y, B = 7.
    let mut pixels = Vec::new();
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&[100 * x as u16, 1000 * y as u16, 7]);
        }
    }
    for (compression, predictor) in [(32946, 1), (32946, 2), (1, 1)] {
        let path = dir
            .path()
            .join(format!("stack-{compression}-{predictor}.tif"));
        std::fs::write(
            &path,
            tiff16(width, height, 3, &pixels, compression, predictor, 3),
        )
        .unwrap();
        let (luminance, header) = read_luminance(&path).unwrap();
        assert!(header.is_none());
        assert_eq!((luminance.width, luminance.height), (width, height));
        for y in 0..height {
            for x in 0..width {
                // FITS row 0 is the raster's last row.
                let raster_y = height - 1 - y;
                let expected = (100.0 * x as f32 + 1000.0 * raster_y as f32 + 7.0) / 3.0;
                let got = luminance.pixels[y * width + x];
                assert!(
                    (got - expected).abs() < 1e-3,
                    "{compression}/{predictor} ({x},{y}): {got} != {expected}"
                );
            }
        }
    }
}

#[test]
fn a_png_and_a_colour_fits_are_read_too() {
    let dir = TempDir::new("solve-input-png-fits");
    let (width, height) = (6, 3);
    let grey: Vec<u16> = (0..width * height).map(|i| (i * 1111) as u16).collect();
    let png = dir.path().join("final.png");
    std::fs::write(&png, png16(width, height, &grey)).unwrap();
    let (luminance, _) = read_luminance(&png).unwrap();
    for y in 0..height {
        for x in 0..width {
            let raster = grey[(height - 1 - y) * width + x];
            assert_eq!(luminance.pixels[y * width + x], f32::from(raster));
        }
    }

    // A Siril colour master: three float planes, the rows kept as they are.
    let plane = |k: f32| {
        (0..width * height)
            .map(|i| k * i as f32)
            .collect::<Vec<_>>()
    };
    let fits = dir.path().join("integration.fit");
    std::fs::write(
        &fits,
        fits_float(width, height, &[plane(1.0), plane(2.0), plane(3.0)]),
    )
    .unwrap();
    let (luminance, header) = read_luminance(&fits).unwrap();
    assert!((luminance.pixels[7] - 2.0 * 7.0).abs() < 1e-5);
    assert_eq!(header.unwrap().get_f64("FOCALLEN"), Some(190.0));
}

#[test]
fn the_file_for_the_solver_is_an_uncompressed_16_bit_fits_with_the_hints() {
    let luminance = Luminance {
        width: 40,
        height: 30,
        pixels: (0..1200).map(|i| (i % 97) as f32 * 0.001).collect(),
    };
    let cards = InputCards {
        ra_deg: Some(38.1781),
        dec_deg: Some(61.45),
        pixel_scale_arcsec: Some(4.0817),
    };
    let (bytes, _) = fits16(&luminance, &cards);
    assert_eq!(bytes.len() % 2880, 0);
    let (header, offset) = parse_primary_header_with_data_offset(&bytes).unwrap();
    assert_eq!(header.get_i64("BITPIX"), Some(16));
    assert_eq!(header.get_i64("NAXIS"), Some(2));
    assert_eq!(header.get_i64("NAXIS1"), Some(40));
    assert_eq!(header.get_i64("NAXIS2"), Some(30));
    assert_eq!(header.get_f64("BZERO"), Some(32768.0));
    assert_eq!(header.get_f64("RA"), Some(38.1781));
    assert_eq!(header.get_f64("DEC"), Some(61.45));
    assert_eq!(header.get_f64("PIXSCALE"), Some(4.0817));
    // No compression: the data unit is two bytes a pixel, padded.
    assert!(header.get("ZIMAGE").is_none());
    assert!(bytes.len() - offset >= 1200 * 2);
    assert!(bytes.len() - offset < 1200 * 2 + 2880);
    // The darkest pixel at 500, the brightest at 64 500.
    let value = |i: usize| {
        i32::from(i16::from_be_bytes([
            bytes[offset + 2 * i],
            bytes[offset + 2 * i + 1],
        ])) + 32768
    };
    assert_eq!(value(0), 500);
    assert_eq!(value(96), 64_500);
}

/// A deep stack in miniature: a nebula, three stars, almost no noise.
fn deep_field(width: usize, height: usize) -> Luminance {
    let mut pixels = Vec::with_capacity(width * height);
    let mut noise = Noise(7);
    for y in 0..height {
        for x in 0..width {
            let nebula = 50.0 * ((x as f64 / 40.0).sin() * (y as f64 / 55.0).cos()).abs();
            pixels.push((1000.0 + nebula + noise.pair().0) as f32);
        }
    }
    for (sx, sy) in [(30_i32, 40_i32), (100, 70), (150, 20)] {
        for dy in -3_i32..=3 {
            for dx in -3_i32..=3 {
                let r2 = f64::from(dx * dx + dy * dy);
                let at = (sy + dy) as usize * width + (sx + dx) as usize;
                pixels[at] += (20_000.0 * (-r2 / 2.0).exp()) as f32;
            }
        }
    }
    Luminance {
        width,
        height,
        pixels,
    }
}

#[test]
fn levelling_adds_four_times_the_background_texture_and_always_the_same() {
    let field = deep_field(192, 128);
    let texture = background_texture(&field).unwrap();
    // The stars are clipped out of the tiles: the texture is the nebula's and
    // the noise's, far below a star.
    assert!(texture > 0.5 && texture < 30.0, "{texture}");
    let (mut a, mut b) = (field.clone(), field.clone());
    let sigma = level(&mut a);
    assert!((sigma - 4.0 * texture).abs() < 1e-3, "{sigma} {texture}");
    level(&mut b);
    assert_eq!(a.pixels, b.pixels, "the same image, the same noise");
    // The noise added has the deviation said, around zero.
    let differences: Vec<f64> = a
        .pixels
        .iter()
        .zip(&field.pixels)
        .map(|(x, y)| f64::from(x - y))
        .collect();
    let n = differences.len() as f64;
    let mean = differences.iter().sum::<f64>() / n;
    let deviation = (differences.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / n).sqrt();
    assert!(
        (deviation / f64::from(sigma) - 1.0).abs() < 0.03,
        "{deviation} {sigma}"
    );
    assert!(mean.abs() < f64::from(sigma) * 0.02);
}

#[test]
fn the_scale_comes_from_the_header_or_from_the_integration_beside_a_stack() {
    let dir = TempDir::new("solve-input-scale");
    let stacked = dir.path().join("stacked");
    std::fs::create_dir_all(&stacked).unwrap();
    let tiff = stacked.join("IC_1805_2026-08-09_raw.tif");
    std::fs::write(&tiff, tiff16(6, 3, 1, &[1; 18], 32946, 1, 3)).unwrap();
    let size = ImageSize {
        width: 6,
        height: 3,
    };
    assert_eq!(scale_hint(&tiff, None, size), None);
    // Another size beside it: not this image's master.
    std::fs::write(
        stacked.join("other_integration.fit"),
        fits_float(5, 3, &[vec![0.0; 15]]),
    )
    .unwrap();
    assert_eq!(scale_hint(&tiff, None, size), None);
    std::fs::write(
        stacked.join("nofilter_180s_g100_integration.fit"),
        fits_float(6, 3, &[vec![0.0; 18]]),
    )
    .unwrap();
    let scale = scale_hint(&tiff, None, size).unwrap();
    // 206.265 x 3.76 µm / 190 mm.
    assert!((scale - 4.0819).abs() < 1e-3, "{scale}");
}

#[test]
fn a_jpeg_or_a_cut_file_is_left_to_the_solver() {
    let dir = TempDir::new("solve-input-unsupported");
    let jpeg = dir.path().join("phone.jpg");
    std::fs::write(&jpeg, crate::raster::samples::jpeg(40, 30)).unwrap();
    assert!(matches!(
        read_luminance(&jpeg),
        Err(DecodeError::Unsupported(_))
    ));
    let cut = dir.path().join("cut.fits");
    std::fs::write(&cut, crate::raster::samples::fits(30, 20, &[])).unwrap();
    assert!(matches!(read_luminance(&cut), Err(DecodeError::Corrupt(_))));
}

/// The fixed path on a real image: `ASTRO_SYNC_SOLVE_IMAGE`,
/// `ASTRO_SYNC_ASTAP`, `ASTRO_SYNC_ASTAP_DATABASE`, the target's
/// `ASTRO_SYNC_SOLVE_RA` / `ASTRO_SYNC_SOLVE_DEC` (degrees); the answer of
/// ASTAP stays in `ASTRO_SYNC_SOLVE_WORK`. Run by hand:
/// `cargo test -p astro-sync-core real_image -- --ignored --nocapture`.
#[test]
#[ignore = "needs ASTAP, its database and a real image"]
fn real_image_solves_through_the_fixed_path() {
    use crate::sky_solution::{SolveHints, SolveJob, SolverTool, solve_with_astap};
    let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name}"));
    let image = PathBuf::from(var("ASTRO_SYNC_SOLVE_IMAGE"));
    let work = PathBuf::from(var("ASTRO_SYNC_SOLVE_WORK"));
    let tool = SolverTool {
        path: PathBuf::from(var("ASTRO_SYNC_ASTAP")),
        version: None,
        database: Some(PathBuf::from(var("ASTRO_SYNC_ASTAP_DATABASE"))),
    };
    let hints = SolveHints {
        ra_deg: std::env::var("ASTRO_SYNC_SOLVE_RA")
            .ok()
            .and_then(|v| v.parse().ok()),
        dec_deg: std::env::var("ASTRO_SYNC_SOLVE_DEC")
            .ok()
            .and_then(|v| v.parse().ok()),
        pixel_scale_arcsec: None,
    };
    let job = SolveJob {
        image: &image,
        hints,
        work_dir: &work,
        timeout: std::time::Duration::from_secs(180),
        cancel: None,
        solved_at: "2026-10-23T00:00:00Z",
    };
    let started = std::time::Instant::now();
    let solved = solve_with_astap(&tool, &job);
    println!("elapsed: {:.1} s", started.elapsed().as_secs_f64());
    println!("{solved:#?}");
    if let Ok(ini) = std::fs::read_to_string(work.join("astap.ini")) {
        println!("--- astap.ini\n{ini}");
    }
    solved.unwrap();
}

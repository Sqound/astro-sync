//! The size of an image (TIFF, JPEG, PNG, or a FITS pose), read from its header only,
//! and the text a TIFF carries in its description.
//!
//! A sky solution says the pixels it was computed on (`width`, `height`,
//! `sky-solution.schema.json`); the service refuses one whose proportions
//! differ from the stored image's. A header solution (`source: header`) is
//! only taken when the header was written for this very image, uncropped:
//! its `NAXIS1` × `NAXIS2` must be the image's size. No pixel is read, and
//! no image library is added: the three headers are a few fixed fields, and
//! a FITS pose gives its size in `NAXIS1` × `NAXIS2`.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::analysis::fits::read_primary_header_file;

/// Width and height in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageSize {
    pub width: u32,
    pub height: u32,
}

/// The size of the image at `path` (by its first bytes, not its extension),
/// or `None` for another format or an unreadable header.
#[must_use]
pub fn image_size(path: &Path) -> Option<ImageSize> {
    let mut file = File::open(path).ok()?;
    let mut head = [0_u8; 32];
    let read = file.read(&mut head).ok()?;
    let head = &head[..read];
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        png_size(head)
    } else if head.starts_with(&[0xFF, 0xD8]) {
        jpeg_size(&mut file)
    } else if head.starts_with(b"II*\0") || head.starts_with(b"MM\0*") {
        tiff_read(&mut file)
            .map(|tiff| tiff.size)
            .filter(|s| s.width > 0)
    } else if is_fits(head) {
        fits_size(path)
    } else {
        None
    }
}

/// Whether the first bytes are a FITS primary header (`SIMPLE  =`).
#[must_use]
pub fn is_fits(head: &[u8]) -> bool {
    head.starts_with(b"SIMPLE  =")
}

/// Whether the file at `path` is a FITS file, by its first bytes.
#[must_use]
pub fn is_fits_file(path: &Path) -> bool {
    let mut head = [0_u8; 9];
    File::open(path)
        .and_then(|mut file| file.read_exact(&mut head))
        .is_ok()
        && is_fits(&head)
}

/// The size of a FITS image: `NAXIS1` × `NAXIS2` of its primary header (a
/// colour cube's third axis is not a dimension of the picture). No pixel is
/// read. `None` for a header without a two-dimensional picture.
fn fits_size(path: &Path) -> Option<ImageSize> {
    let header = read_primary_header_file(path).ok()?;
    if header.get_i64("NAXIS")? < 2 {
        return None;
    }
    let width = u32::try_from(header.get_i64("NAXIS1")?).ok()?;
    let height = u32::try_from(header.get_i64("NAXIS2")?).ok()?;
    (width > 0 && height > 0).then_some(ImageSize { width, height })
}

/// The description (tag 270) of the TIFF at `path`, where some programs
/// write the FITS header of the image they exported.
#[must_use]
pub fn tiff_description(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    tiff_read(&mut file)?.description
}

fn png_size(head: &[u8]) -> Option<ImageSize> {
    // Signature (8), IHDR length (4), "IHDR" (4), width (4), height (4).
    if head.len() < 24 || &head[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(head[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(head[20..24].try_into().ok()?);
    (width > 0 && height > 0).then_some(ImageSize { width, height })
}

fn jpeg_size(file: &mut File) -> Option<ImageSize> {
    file.seek(SeekFrom::Start(2)).ok()?;
    let mut bytes = Vec::new();
    // The frame header comes before the image data: a few hundred kilobytes
    // at most even with a large embedded preview.
    file.take(4 * 1024 * 1024).read_to_end(&mut bytes).ok()?;
    let mut at = 0;
    while at + 4 <= bytes.len() {
        if bytes[at] != 0xFF {
            at += 1;
            continue;
        }
        let marker = bytes[at + 1];
        if marker == 0xFF || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            at += if marker == 0xFF { 1 } else { 2 };
            continue;
        }
        let length = usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
        let start_of_frame = matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if start_of_frame {
            let segment = bytes.get(at + 4..at + 2 + length)?;
            let height = u32::from(u16::from_be_bytes([*segment.get(1)?, *segment.get(2)?]));
            let width = u32::from(u16::from_be_bytes([*segment.get(3)?, *segment.get(4)?]));
            return (width > 0 && height > 0).then_some(ImageSize { width, height });
        }
        if marker == 0xDA {
            return None;
        }
        at += 2 + length;
    }
    None
}

struct Tiff {
    size: ImageSize,
    description: Option<String>,
}

fn tiff_read(file: &mut File) -> Option<Tiff> {
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut header = [0_u8; 8];
    file.read_exact(&mut header).ok()?;
    let little = &header[..2] == b"II";
    let u16_of = |b: &[u8]| {
        let pair = [b[0], b[1]];
        if little {
            u16::from_le_bytes(pair)
        } else {
            u16::from_be_bytes(pair)
        }
    };
    let u32_of = |b: &[u8]| {
        let quad = [b[0], b[1], b[2], b[3]];
        if little {
            u32::from_le_bytes(quad)
        } else {
            u32::from_be_bytes(quad)
        }
    };
    let ifd = u64::from(u32_of(&header[4..8]));
    file.seek(SeekFrom::Start(ifd)).ok()?;
    let mut count = [0_u8; 2];
    file.read_exact(&mut count).ok()?;
    let count = usize::from(u16_of(&count));
    let mut entries = vec![0_u8; count * 12];
    file.read_exact(&mut entries).ok()?;
    let (mut width, mut height) = (0, 0);
    let mut description: Option<(u64, usize)> = None;
    for entry in entries.chunks_exact(12) {
        let tag = u16_of(&entry[0..2]);
        let kind = u16_of(&entry[2..4]);
        let n = u32_of(&entry[4..8]);
        let value = match kind {
            3 => u32::from(u16_of(&entry[8..10])),
            _ => u32_of(&entry[8..12]),
        };
        match tag {
            256 => width = value,
            257 => height = value,
            270 if kind == 2 => description = Some((u64::from(value), n as usize)),
            _ => {}
        }
    }
    let description = match description {
        // Four bytes or fewer sit in the entry itself: not a header.
        Some((offset, length)) if length > 4 && length < 1024 * 1024 => {
            file.seek(SeekFrom::Start(offset)).ok()?;
            let mut text = vec![0_u8; length];
            file.read_exact(&mut text).ok()?;
            Some(
                String::from_utf8_lossy(&text)
                    .trim_end_matches('\0')
                    .to_string(),
            )
        }
        _ => None,
    };
    Some(Tiff {
        size: ImageSize { width, height },
        description,
    })
}

/// Test helpers: the smallest files of each format with a given size.
#[cfg(test)]
pub(crate) mod samples {
    /// A PNG signature and IHDR (no image data: enough for the header).
    pub fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
        bytes
    }

    /// A JPEG start, an APP0 segment, then a baseline frame header.
    pub fn jpeg(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        bytes.extend_from_slice(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        bytes.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08]);
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        bytes.extend_from_slice(&[0xFF, 0xD9]);
        bytes
    }

    /// A FITS primary header (no pixel) of a two-dimensional picture, with
    /// the extra `cards` (already formatted values) after the size.
    pub fn fits(width: u32, height: u32, cards: &[(&str, &str)]) -> Vec<u8> {
        let mut all = vec![
            ("SIMPLE".to_string(), "T".to_string()),
            ("BITPIX".to_string(), "16".to_string()),
            ("NAXIS".to_string(), "2".to_string()),
            ("NAXIS1".to_string(), width.to_string()),
            ("NAXIS2".to_string(), height.to_string()),
        ];
        all.extend(
            cards
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string())),
        );
        let mut bytes = Vec::new();
        for (keyword, value) in &all {
            let mut card = format!("{keyword:<8}= {value:>20}").into_bytes();
            card.resize(80, b' ');
            bytes.extend_from_slice(&card);
        }
        let mut end = b"END".to_vec();
        end.resize(80, b' ');
        bytes.extend_from_slice(&end);
        bytes.resize(bytes.len().div_ceil(2880) * 2880, b' ');
        bytes
    }

    /// A little-endian TIFF with its width, height and, when given, a
    /// description.
    pub fn tiff(width: u32, height: u32, description: Option<&str>) -> Vec<u8> {
        let text = description.map(|d| format!("{d}\0").into_bytes());
        let entries: u16 = if text.is_some() { 3 } else { 2 };
        let ifd_len = 2 + usize::from(entries) * 12 + 4;
        let text_offset = (8 + ifd_len) as u32;
        let mut bytes = b"II*\0".to_vec();
        bytes.extend_from_slice(&8_u32.to_le_bytes());
        bytes.extend_from_slice(&entries.to_le_bytes());
        let mut entry = |tag: u16, kind: u16, count: u32, value: u32| {
            bytes.extend_from_slice(&tag.to_le_bytes());
            bytes.extend_from_slice(&kind.to_le_bytes());
            bytes.extend_from_slice(&count.to_le_bytes());
            bytes.extend_from_slice(&value.to_le_bytes());
        };
        entry(256, 4, 1, width);
        entry(257, 4, 1, height);
        if let Some(text) = &text {
            entry(270, 2, text.len() as u32, text_offset);
        }
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        if let Some(text) = text {
            bytes.extend_from_slice(&text);
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn the_three_formats_give_their_size() {
        let dir = TempDir::new("raster-sizes");
        let cases = [
            ("a.png", samples::png(4144, 2822)),
            ("b.jpg", samples::jpeg(6000, 4000)),
            ("c.tif", samples::tiff(3008, 3008, None)),
        ];
        let expected = [(4144, 2822), (6000, 4000), (3008, 3008)];
        for ((name, bytes), (width, height)) in cases.iter().zip(expected) {
            let path = dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            assert_eq!(
                image_size(&path),
                Some(ImageSize { width, height }),
                "{name}"
            );
        }
        let other = dir.path().join("d.txt");
        std::fs::write(&other, b"not an image").unwrap();
        assert_eq!(image_size(&other), None);
    }

    #[test]
    fn a_fits_pose_gives_its_size_from_its_header() {
        let dir = TempDir::new("raster-fits");
        let path = dir.path().join("img-0028.fits");
        std::fs::write(&path, samples::fits(3072, 2080, &[("BZERO", "32768")])).unwrap();
        assert_eq!(
            image_size(&path),
            Some(ImageSize {
                width: 3072,
                height: 2080
            })
        );
        assert!(is_fits_file(&path));
        // A header that never closes, or a missing picture, is not a size.
        let open = dir.path().join("open.fits");
        std::fs::write(&open, b"SIMPLE  =                    T").unwrap();
        assert_eq!(image_size(&open), None);
        let empty = dir.path().join("empty.fits");
        std::fs::write(&empty, samples::fits(0, 0, &[])).unwrap();
        assert_eq!(image_size(&empty), None);
    }

    #[test]
    fn a_tiff_description_is_read() {
        let dir = TempDir::new("raster-description");
        let path = dir.path().join("final.tif");
        std::fs::write(&path, samples::tiff(10, 20, Some("CTYPE1  = 'RA---TAN'"))).unwrap();
        assert_eq!(
            tiff_description(&path).as_deref(),
            Some("CTYPE1  = 'RA---TAN'")
        );
        std::fs::write(&path, samples::tiff(10, 20, None)).unwrap();
        assert_eq!(tiff_description(&path), None);
    }
}

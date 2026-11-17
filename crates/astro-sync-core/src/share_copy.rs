//! « Exporter pour partager »: the copy of an image with its band, written
//! where the person chooses. The band is drawn by the window on a copy made
//! on request; this module is the only door the copy goes through, and it
//! keeps two promises: the copy carries no metadata (a PNG reduced to the
//! chunks that draw it), and it never replaces the file it comes from. The
//! files sent to the space, and the PNG and TIFF of a render, are written by
//! other code and never by this one.

use std::fs;
use std::io::Write;
use std::path::Path;

use thiserror::Error;

pub(crate) const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// The chunks that draw the picture. Every other chunk (text, EXIF, time,
/// physical size, colour profile) is metadata and is left out.
const KEPT: [&[u8; 4]; 5] = [b"IHDR", b"PLTE", b"tRNS", b"IDAT", b"IEND"];

#[derive(Debug, Error)]
pub enum ShareCopyError {
    #[error("the copy is not a PNG image")]
    NotPng,
    #[error("the copy must be saved as a PNG file")]
    NotPngName,
    #[error("the copy would replace the image it comes from")]
    WouldReplaceOriginal,
    #[error("the copy could not be written: {0}")]
    Io(#[from] std::io::Error),
}

impl ShareCopyError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotPng => "share_copy_not_png",
            Self::NotPngName => "share_copy_not_png_name",
            Self::WouldReplaceOriginal => "share_copy_replaces_original",
            Self::Io(_) => "share_copy_io",
        }
    }
}

/// The PNG `bytes` reduced to the chunks that draw it.
pub fn strip_png(bytes: &[u8]) -> Result<Vec<u8>, ShareCopyError> {
    if bytes.len() < 8 || bytes[..8] != PNG_SIGNATURE {
        return Err(ShareCopyError::NotPng);
    }
    let mut out = PNG_SIGNATURE.to_vec();
    let mut at = 8;
    let mut ended = false;
    while at < bytes.len() {
        if at + 12 > bytes.len() {
            return Err(ShareCopyError::NotPng);
        }
        let length =
            u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]) as usize;
        let end = at
            .checked_add(12)
            .and_then(|n| n.checked_add(length))
            .filter(|end| *end <= bytes.len())
            .ok_or(ShareCopyError::NotPng)?;
        let kind = &bytes[at + 4..at + 8];
        if KEPT.iter().any(|kept| kept.as_slice() == kind) {
            out.extend_from_slice(&bytes[at..end]);
        }
        ended = kind == b"IEND";
        at = end;
        if ended {
            break;
        }
    }
    if !ended {
        return Err(ShareCopyError::NotPng);
    }
    Ok(out)
}

/// Writes the copy `bytes` (metadata removed) at `destination`. Refused: a
/// destination that is not a `.png`, and the file `source` itself (however
/// it is spelled).
pub fn write_share_copy(
    destination: &Path,
    source: &Path,
    bytes: &[u8],
) -> Result<(), ShareCopyError> {
    let is_png = destination
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("png"));
    if !is_png {
        return Err(ShareCopyError::NotPngName);
    }
    if let (Ok(a), Ok(b)) = (fs::canonicalize(destination), fs::canonicalize(source)) {
        if a == b {
            return Err(ShareCopyError::WouldReplaceOriginal);
        }
    }
    let clean = strip_png(bytes)?;
    let mut file = fs::File::create(destination)?;
    file.write_all(&clean)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests;

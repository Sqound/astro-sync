use super::*;
use std::fs;

/// A PNG chunk: length, type, data, and a CRC the module never checks (it
/// copies the chunks it keeps byte for byte).
fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&[0, 0, 0, 0]);
    out
}

fn png_with(extra: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
    let mut bytes = PNG_SIGNATURE.to_vec();
    bytes.extend(chunk(b"IHDR", &[0; 13]));
    for (kind, data) in extra {
        bytes.extend(chunk(kind, data));
    }
    bytes.extend(chunk(b"IDAT", &[1, 2, 3]));
    bytes.extend(chunk(b"IEND", &[]));
    bytes
}

fn kinds(png: &[u8]) -> Vec<String> {
    let mut at = 8;
    let mut out = Vec::new();
    while at + 8 <= png.len() {
        let length = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        out.push(String::from_utf8_lossy(&png[at + 4..at + 8]).into_owned());
        at += 12 + length;
    }
    out
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("share-copy-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn only_the_chunks_that_draw_the_picture_stay() {
    let noisy = png_with(&[
        (b"tEXt", b"Comment\0a place and a name"),
        (b"eXIf", b"GPS"),
        (b"iTXt", b"x"),
        (b"tIME", b"1234567"),
        (b"pHYs", b"123456789"),
        (b"tRNS", b"0"),
    ]);
    let clean = strip_png(&noisy).unwrap();
    assert_eq!(kinds(&clean), ["IHDR", "tRNS", "IDAT", "IEND"]);
}

#[test]
fn a_clean_png_is_returned_unchanged() {
    let clean = png_with(&[]);
    assert_eq!(strip_png(&clean).unwrap(), clean);
}

#[test]
fn what_is_not_a_png_is_refused() {
    assert!(strip_png(b"GIF89a").is_err());
    let mut cut = png_with(&[]);
    cut.truncate(cut.len() - 5);
    assert!(strip_png(&cut).is_err());
}

#[test]
fn the_copy_is_written_without_metadata() {
    let dir = scratch("write");
    let source = dir.join("render.png");
    fs::write(&source, b"the original").unwrap();
    let destination = dir.join("copy.png");
    let noisy = png_with(&[(b"tEXt", b"secret")]);
    write_share_copy(&destination, &source, &noisy).unwrap();
    assert_eq!(
        kinds(&fs::read(&destination).unwrap()),
        ["IHDR", "IDAT", "IEND"]
    );
    // Nothing else was touched.
    assert_eq!(fs::read(&source).unwrap(), b"the original");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn the_copy_never_replaces_the_image_it_comes_from() {
    let dir = scratch("same");
    let source = dir.join("render.png");
    fs::write(&source, b"the original").unwrap();
    let error = write_share_copy(&source, &source, &png_with(&[])).unwrap_err();
    assert!(matches!(error, ShareCopyError::WouldReplaceOriginal));
    // Another spelling of the same file: refused too.
    let alias = dir.join(".").join("render.png");
    assert!(write_share_copy(&alias, &source, &png_with(&[])).is_err());
    assert_eq!(fs::read(&source).unwrap(), b"the original");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn the_copy_is_a_png_and_never_a_tiff() {
    let dir = scratch("ext");
    let source = dir.join("render.png");
    fs::write(&source, b"x").unwrap();
    let tiff = dir.join("render.tif");
    assert!(matches!(
        write_share_copy(&tiff, &source, &png_with(&[])),
        Err(ShareCopyError::NotPngName)
    ));
    assert!(!tiff.exists());
    let _ = fs::remove_dir_all(dir);
}

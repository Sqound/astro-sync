use super::fixture::{Ifd, V, tiff};
use super::*;
use std::io::Cursor;

fn read(bytes: Vec<u8>) -> io::Result<RawMetadata> {
    let mut source = Source::new(Cursor::new(bytes))?;
    read_metadata(&mut source)
}

/// A main block + an EXIF block, the shape of a CR2, NEF, ARW, DNG.
fn simple(little: bool, extra_exif: Ifd, extra_main: Ifd) -> Vec<u8> {
    let mut main: Ifd = vec![
        (0x010F, V::Ascii("NIKON CORPORATION")),
        (0x0110, V::Ascii("NIKON D3300")),
        (0x8769, V::Ptr(1)),
    ];
    main.extend(extra_main);
    let mut exif: Ifd = vec![
        (0x829A, V::Rational(vec![(30, 1)])),
        (0x8827, V::Short(vec![800])),
        (0x9003, V::Ascii("2026:10:07 23:15:42")),
    ];
    exif.extend(extra_exif);
    tiff(little, 0x2A, &[main, exif])
}

/* ----------------------------------- Tests -------------------------------- */

#[test]
fn a_nikon_tiff_gives_its_exposure_iso_model_and_clock_in_both_byte_orders() {
    for little in [true, false] {
        let meta = read(simple(little, Vec::new(), Vec::new())).unwrap();
        assert_eq!(meta.exposure_seconds, Some(30.0), "little={little}");
        assert_eq!(meta.iso, Some(800.0));
        assert_eq!(meta.model.as_deref(), Some("NIKON D3300"));
        assert_eq!(meta.make.as_deref(), Some("NIKON CORPORATION"));
        let taken = meta.taken.unwrap();
        assert_eq!(
            (taken.year, taken.month, taken.day, taken.hour, taken.minute),
            (2026, 10, 7, 23, 15)
        );
        assert_eq!(meta.offset_seconds, None);
        assert_eq!(meta.temperature_c, None);
    }
}

#[test]
fn the_header_is_the_one_of_a_fits_frame_with_a_gain_that_is_the_iso() {
    let meta = read(simple(true, Vec::new(), Vec::new())).unwrap();
    let header = header_of(&meta, "DSC_0001.NEF", "nef");
    assert_eq!(header.get("INSTRUME"), Some("NIKON D3300"));
    assert_eq!(header.get_f64("EXPTIME"), Some(30.0));
    assert_eq!(header.get_f64("GAIN"), Some(800.0));
    assert_eq!(header.get_f64("ISOSPEED"), Some(800.0));
    assert_eq!(header.get("XBINNING"), Some("1"));
    assert_eq!(header.get("RAWFILE"), Some("NEF"));
    assert_eq!(header.get("BAYERPAT"), Some("RGGB"));
    assert_eq!(header.get("IMAGETYP"), None);
    let record = crate::analysis::fits::extract_acquisition_record(&header);
    assert_eq!(record.exposure_seconds, Some(30.0));
    assert_eq!(record.gain, Some(800.0));
    assert_eq!(record.binning.as_deref(), Some("1x1"));
    assert_eq!(record.camera.as_deref(), Some("NIKON D3300"));
}

#[test]
fn a_clock_without_a_zone_is_kept_as_written_and_said_so() {
    let meta = read(simple(true, Vec::new(), Vec::new())).unwrap();
    let header = header_of(&meta, "a.nef", "nef");
    assert_eq!(header.get("DATE-OBS"), Some("2026-10-07T23:15:42"));
    assert_eq!(header.get("DATE-TZ"), Some("UNKNOWN"));
    let record = crate::analysis::fits::extract_acquisition_record(&header);
    assert!(record.date_zone_unknown);
    assert_eq!(record.date_obs_utc.as_deref(), Some("2026-10-07T23:15:42Z"));
}

#[test]
fn an_offset_time_brings_the_clock_to_utc() {
    let meta = read(simple(
        true,
        vec![(0x9011, V::Ascii("+09:00")), (0x9291, V::Ascii("25"))],
        Vec::new(),
    ))
    .unwrap();
    assert_eq!(meta.offset_seconds, Some(9 * 3600));
    let header = header_of(&meta, "a.nef", "nef");
    // 23:15:42.25 at UTC+9 is 14:15:42.25 UTC.
    assert_eq!(header.get("DATE-OBS"), Some("2026-10-07T14:15:42.250"));
    assert_eq!(header.get("DATE-TZ"), None);
    let record = crate::analysis::fits::extract_acquisition_record(&header);
    assert!(!record.date_zone_unknown);
}

#[test]
fn the_offset_time_of_the_camera_serves_when_the_original_one_is_absent() {
    let meta = read(simple(true, vec![(0x9010, V::Ascii("-05:30"))], Vec::new())).unwrap();
    assert_eq!(meta.offset_seconds, Some(-(5 * 3600 + 1800)));
}

#[test]
fn a_gps_time_gives_the_zone_of_a_clock_that_names_none() {
    // The camera says 23:15:42; the GPS says 14:15:30 UTC the same day: the
    // clock is UTC+9 (the 12 s are the shutter and the fix).
    let main: Ifd = vec![
        (0x0110, V::Ascii("X")),
        (0x8769, V::Ptr(1)),
        (0x8825, V::Ptr(2)),
    ];
    let gps = vec![
        (0x001D, V::Ascii("2026:10:07")),
        (0x0007, V::Rational(vec![(14, 1), (15, 1), (30, 1)])),
    ];
    let exif: Ifd = vec![
        (0x829A, V::Rational(vec![(1, 2)])),
        (0x9003, V::Ascii("2026:10:07 23:15:42")),
    ];
    let meta = read(tiff(true, 0x2A, &[main, exif, gps])).unwrap();
    assert!(meta.gps_unix.is_some());
    let header = header_of(&meta, "a.cr2", "cr2");
    assert_eq!(header.get("DATE-OBS"), Some("2026-10-07T14:15:42"));
    assert_eq!(header.get("DATE-TZ"), None);
}

#[test]
fn a_gps_time_a_day_away_from_the_clock_is_not_a_zone() {
    let main: Ifd = vec![
        (0x0110, V::Ascii("X")),
        (0x8769, V::Ptr(1)),
        (0x8825, V::Ptr(2)),
    ];
    let gps = vec![
        (0x001D, V::Ascii("2026:10:05")),
        (0x0007, V::Rational(vec![(1, 1), (0, 1), (0, 1)])),
    ];
    let exif: Ifd = vec![(0x9003, V::Ascii("2026:10:07 23:15:42"))];
    let meta = read(tiff(true, 0x2A, &[main, exif, gps])).unwrap();
    let header = header_of(&meta, "a.cr2", "cr2");
    assert_eq!(header.get("DATE-TZ"), Some("UNKNOWN"));
}

#[test]
fn the_iso_falls_back_on_the_recommended_index_when_the_camera_says_65535() {
    let main: Ifd = vec![(0x0110, V::Ascii("X")), (0x8769, V::Ptr(1))];
    let exif: Ifd = vec![
        (0x8827, V::Short(vec![65_535])),
        (0x8832, V::Long(vec![3200])),
    ];
    let meta = read(tiff(true, 0x2A, &[main, exif])).unwrap();
    assert_eq!(meta.iso, Some(3200.0));
}

#[test]
fn a_panasonic_keeps_its_iso_in_the_main_block_and_an_olympus_has_its_own_magic() {
    let main: Ifd = vec![
        (0x010F, V::Ascii("Panasonic")),
        (0x0110, V::Ascii("DMC-GH1")),
        (0x0017, V::Short(vec![1600])),
        (0x8769, V::Ptr(1)),
    ];
    let exif: Ifd = vec![(0x829A, V::Rational(vec![(1, 50)]))];
    for magic in [0x55, 0x4F52, 0x5352] {
        let meta = read(tiff(true, magic, &[main.clone(), exif.clone()])).unwrap();
        assert_eq!(meta.iso, Some(1600.0), "magic {magic:#x}");
        assert_eq!(meta.exposure_seconds, Some(0.02));
    }
}

#[test]
fn a_canon_maker_note_gives_the_camera_temperature() {
    // Position 12 of the shot information holds the temperature plus 128.
    let mut shot = vec![0_u16; 16];
    shot[12] = 128 + 24;
    let main: Ifd = vec![
        (0x010F, V::Ascii("Canon")),
        (0x0110, V::Ascii("Canon EOS 6D")),
        (0x8769, V::Ptr(1)),
    ];
    let exif: Ifd = vec![(0x829A, V::Rational(vec![(300, 1)])), (0x927C, V::Note(2))];
    let note: Ifd = vec![(0x0004, V::Short(shot))];
    let meta = read(tiff(true, 0x2A, &[main, exif, note])).unwrap();
    assert_eq!(meta.temperature_c, Some(24.0));
    let header = header_of(&meta, "a.cr2", "cr2");
    assert_eq!(header.get_f64("CCD-TEMP"), Some(24.0));
}

#[test]
fn another_maker_has_no_temperature_and_an_unrecorded_one_is_not_zero_degrees() {
    let mut shot = vec![0_u16; 16];
    shot[12] = 0;
    let main: Ifd = vec![(0x010F, V::Ascii("Canon")), (0x8769, V::Ptr(1))];
    let exif: Ifd = vec![(0x927C, V::Note(2))];
    let note: Ifd = vec![(0x0004, V::Short(shot))];
    let meta = read(tiff(true, 0x2A, &[main, exif, note])).unwrap();
    assert_eq!(meta.temperature_c, None);

    let mut shot = vec![0_u16; 16];
    shot[12] = 150;
    let main: Ifd = vec![(0x010F, V::Ascii("NIKON")), (0x8769, V::Ptr(1))];
    let exif: Ifd = vec![(0x927C, V::Note(2))];
    let note: Ifd = vec![(0x0004, V::Short(shot))];
    let meta = read(tiff(true, 0x2A, &[main, exif, note])).unwrap();
    assert_eq!(meta.temperature_c, None);
}

#[test]
fn a_fujifilm_raf_is_read_through_the_exif_of_its_preview() {
    let inner = simple(false, Vec::new(), Vec::new());
    // JPEG: SOI, APP1 (Exif\0\0 + TIFF), EOI.
    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE1];
    let size = (2 + 6 + inner.len()) as u16;
    jpeg.extend(size.to_be_bytes());
    jpeg.extend(b"Exif\0\0");
    jpeg.extend(&inner);
    jpeg.extend([0xFF, 0xD9]);
    let mut raf = b"FUJIFILMCCD-RAW 0201".to_vec();
    raf.resize(84, 0);
    raf.extend(100_u32.to_be_bytes());
    raf.extend((jpeg.len() as u32).to_be_bytes());
    raf.resize(100, 0);
    raf.extend(&jpeg);
    let meta = read(raf).unwrap();
    assert_eq!(meta.exposure_seconds, Some(30.0));
    assert_eq!(meta.iso, Some(800.0));
}

#[test]
fn a_fujifilm_says_its_mosaic_in_the_cfa_header_of_the_raf() {
    // The CFA header: a count, then (tag, size, bytes) records, big endian.
    let record = |tag: u16, data: &[u8]| {
        let mut out = tag.to_be_bytes().to_vec();
        out.extend((data.len() as u16).to_be_bytes());
        out.extend(data);
        out
    };
    let build = |pattern: &[u8]| {
        let inner = simple(false, Vec::new(), Vec::new());
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE1];
        jpeg.extend(((2 + 6 + inner.len()) as u16).to_be_bytes());
        jpeg.extend(b"Exif\0\0");
        jpeg.extend(&inner);
        jpeg.extend([0xFF, 0xD9]);
        let mut cfa = 2_u32.to_be_bytes().to_vec();
        cfa.extend(record(0x130, &[12, 12]));
        cfa.extend(record(0x131, pattern));
        let mut raf = b"FUJIFILMCCD-RAW 0201".to_vec();
        raf.resize(84, 0);
        raf.extend(120_u32.to_be_bytes());
        raf.extend((jpeg.len() as u32).to_be_bytes());
        raf.extend((120 + jpeg.len() as u32).to_be_bytes());
        raf.extend((cfa.len() as u32).to_be_bytes());
        raf.resize(120, 0);
        raf.extend(&jpeg);
        raf.extend(&cfa);
        raf
    };
    let x_trans = read(build(&[1_u8; 36])).unwrap();
    assert_eq!(x_trans.mosaic, Some(Mosaic::XTrans));
    let header = header_of(&x_trans, "a.raf", "raf");
    assert_eq!(header.get("BAYERPAT"), Some("XTRANS"));
    let bayer = read(build(&[1, 0, 2, 1])).unwrap();
    assert_eq!(bayer.mosaic, Some(Mosaic::Bayer("GRBG".to_string())));
    assert_eq!(
        header_of(&bayer, "a.raf", "raf").get("BAYERPAT"),
        Some("GRBG")
    );
    // The other formats leave the mosaic to Siril.
    let meta = RawMetadata {
        make: Some("FUJIFILM".into()),
        model: Some("X-T2".into()),
        ..RawMetadata::default()
    };
    assert_eq!(
        header_of(&meta, "a.raf", "raf").get("INSTRUME"),
        Some("FUJIFILM X-T2")
    );
}

fn mp4_box(kind: &[u8; 4], content: &[u8]) -> Vec<u8> {
    let mut out = ((content.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend(kind);
    out.extend(content);
    out
}

#[test]
fn a_canon_cr3_is_read_through_the_boxes_of_its_uuid() {
    let main: Ifd = vec![
        (0x010F, V::Ascii("Canon")),
        (0x0110, V::Ascii("Canon EOS R6")),
        (0x0132, V::Ascii("2026:10:07 23:15:42")),
    ];
    let exif: Ifd = vec![
        (0x829A, V::Rational(vec![(180, 1)])),
        (0x8827, V::Short(vec![1600])),
        (0x9003, V::Ascii("2026:10:07 23:15:42")),
        (0x9011, V::Ascii("+09:00")),
    ];
    let mut shot = vec![0_u16; 16];
    shot[12] = 128 + 18;
    let note: Ifd = vec![(0x0004, V::Short(shot))];
    let mut uuid = CANON_UUID.to_vec();
    uuid.extend(mp4_box(b"CNCV", b"CanonCR3_001/00.09.00/00.00.00"));
    uuid.extend(mp4_box(b"CMT1", &tiff(true, 0x2A, &[main])));
    uuid.extend(mp4_box(b"CMT2", &tiff(true, 0x2A, &[exif])));
    uuid.extend(mp4_box(b"CMT3", &tiff(true, 0x2A, &[note])));
    let mut file = mp4_box(b"ftyp", b"crx \0\0\0\x01crx isom");
    file.extend(mp4_box(b"moov", &mp4_box(b"uuid", &uuid)));
    file.extend(mp4_box(b"mdat", &[0_u8; 64]));
    let meta = read(file).unwrap();
    assert_eq!(meta.model.as_deref(), Some("Canon EOS R6"));
    assert_eq!(meta.exposure_seconds, Some(180.0));
    assert_eq!(meta.iso, Some(1600.0));
    assert_eq!(meta.temperature_c, Some(18.0));
    let header = header_of(&meta, "IMG_0001.CR3", "cr3");
    assert_eq!(header.get("DATE-OBS"), Some("2026-10-07T14:15:42"));
}

#[test]
fn what_is_not_a_raw_file_is_refused() {
    assert!(read(b"SIMPLE  =                    T".to_vec()).is_err());
    assert!(read(vec![0_u8; 3]).is_err());
    assert!(read(Vec::new()).is_err());
    // A TIFF whose IFD points beyond the end of the file.
    let mut broken = simple(true, Vec::new(), Vec::new());
    broken.truncate(30);
    assert!(read(broken).is_err());
    // A JPEG.
    let jpeg = vec![
        0xFF, 0xD8, 0xFF, 0xE0, 0, 16, b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1,
    ];
    assert!(read(jpeg).is_err());
}

#[test]
fn a_clock_with_nothing_in_it_is_no_date() {
    let main: Ifd = vec![(0x0110, V::Ascii("X")), (0x8769, V::Ptr(1))];
    let exif: Ifd = vec![(0x9003, V::Ascii("    :  :     :  :  "))];
    let meta = read(tiff(true, 0x2A, &[main, exif])).unwrap();
    assert_eq!(meta.taken, None);
    let header = header_of(&meta, "a.arw", "arw");
    assert_eq!(header.get("DATE-OBS"), None);
    assert_eq!(header.get("DATE-TZ"), None);
}

#[test]
fn the_frame_type_comes_from_a_whole_word_of_the_file_name() {
    for (name, expected) in [
        ("Light_M31_300s_ISO800_0001.CR2", Some("Light")),
        ("M31-dark-300s-0003.nef", Some("Dark")),
        ("Flat_0001.ARW", Some("Flat")),
        ("BIAS-001.dng", Some("Bias")),
        ("Offset 12.dng", Some("Bias")),
        ("DarkFlat_5.cr3", Some("DarkFlat")),
        ("Flat_Dark_5.cr3", None),
        ("IMG_0001.CR2", None),
        // Not whole words: a name that merely holds the letters.
        ("Flatiron_0001.CR2", None),
        ("lighthouse.CR2", None),
        ("dark_light.cr2", None),
    ] {
        assert_eq!(frame_type_of_name(name), expected, "{name}");
    }
    let meta = RawMetadata::default();
    let header = header_of(&meta, "M31_Dark_0001.CR2", "cr2");
    assert_eq!(header.get("IMAGETYP"), Some("Dark"));
}

#[test]
fn the_extensions_are_the_nine_of_the_scanner() {
    for extension in EXTENSIONS {
        assert_eq!(
            crate::scanner::FileClass::classify(extension),
            crate::scanner::FileClass::Raw,
            "{extension}"
        );
        assert!(is_raw_extension(&extension.to_ascii_uppercase()));
    }
    assert!(!is_raw_extension("fits"));
}

/* ------------------------- The real files of the samples ------------------ */

/// What a file of the public samples says (a table copied from the dumps of
/// raw.pixls.us, one per file), checked when `ASTRO_SYNC_RAW_SAMPLES` names
/// the folder that holds them (not kept in the repository).
#[test]
fn the_public_samples_say_what_their_dumps_say() {
    let Ok(folder) = std::env::var("ASTRO_SYNC_RAW_SAMPLES") else {
        return;
    };
    // (file, model, exposure, iso, date of the clock)
    let expected: [(&str, &str, f64, f64, &str); 12] = [
        (
            "canon-450d.CR2",
            "Canon EOS 450D",
            1.0 / 2000.0,
            400.0,
            "2016-09-25T15:24:20",
        ),
        (
            "canon-6d.CR2",
            "Canon EOS 6D",
            1.3,
            100.0,
            "2017-01-08T11:18:13",
        ),
        (
            "nikon-d3300.NEF",
            "NIKON D3300",
            1.0 / 50.0,
            400.0,
            "2015-05-22T14:56:26",
        ),
        (
            "sony-a100.ARW",
            "SONY DSLR-A100",
            1.0 / 125.0,
            100.0,
            "2018-02-19T17:55:25",
        ),
        ("sony-a7m3.ARW", "SONY ILCE-7M3", 1.0 / 50.0, 400.0, ""),
        (
            "pentax-kx.DNG",
            "PENTAX K-x",
            1.0 / 20.0,
            2000.0,
            "2017-01-12T19:32:38",
        ),
        (
            "pentax-kx.PEF",
            "PENTAX K-x",
            1.0 / 20.0,
            2000.0,
            "2017-01-12T19:32:52",
        ),
        (
            "fuji-xe1.RAF",
            "FUJIFILM X-E1",
            1.0 / 38.0,
            500.0,
            "2019-04-22T07:23:07",
        ),
        (
            "olympus-em5.ORF",
            "OLYMPUS E-M5",
            5.0,
            200.0,
            "2017-02-08T18:41:12",
        ),
        (
            "panasonic-gh1.RW2",
            "Panasonic DMC-GH1",
            1.0 / 50.0,
            100.0,
            "2017-04-07T08:54:49",
        ),
        // The CR3 files carry OffsetTimeOriginal or GPS: UTC, not the clock.
        ("canon-850d.CR3", "Canon EOS 850D", 0.008, 100.0, ""),
        ("canon-r6m2.CR3", "Canon EOS R6m2", 0.02, 100.0, ""),
    ];
    for (file, model, exposure, iso, date) in expected {
        let path = std::path::Path::new(&folder).join(file);
        if !path.is_file() {
            continue;
        }
        let header = read_header_file(&path).unwrap_or_else(|e| panic!("{file}: {e}"));
        assert_eq!(header.get("INSTRUME"), Some(model), "{file}");
        let seen = header.get_f64("EXPTIME").unwrap();
        assert!(
            (seen - exposure).abs() < exposure * 0.01 + 1e-6,
            "{file}: {seen} s, expected {exposure}"
        );
        assert_eq!(header.get_f64("GAIN"), Some(iso), "{file}");
        // The mosaic: a Fujifilm X-E1 is an X-Trans, the others are left to Siril.
        let mosaic = header.get("BAYERPAT");
        assert_eq!(mosaic == Some("XTRANS"), file.starts_with("fuji"), "{file}");
        if !date.is_empty() {
            assert!(
                header.get("DATE-OBS").unwrap().starts_with(date),
                "{file}: {:?}",
                header.get("DATE-OBS")
            );
            assert_eq!(header.get("DATE-TZ"), Some("UNKNOWN"), "{file}");
        }
    }
}

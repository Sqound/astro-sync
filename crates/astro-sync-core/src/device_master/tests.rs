//! The recognition of a device's stacked image and the families it covers,
//! on headers built here (the cards of a real Seestar's `Stacked_*.fit`,
//! no pixel).

use super::*;
use crate::analysis::fits::parse_primary_header;
use crate::test_support::fits_bytes;

const SEESTAR: [(&str, &str); 14] = [
    ("SIMPLE", "T"),
    ("BITPIX", "16"),
    ("NAXIS", "3"),
    ("NAXIS1", "2160"),
    ("NAXIS2", "3840"),
    ("NAXIS3", "3"),
    ("BZERO", "32768"),
    ("CREATOR", "'ZWO Seestar S50 Pro'"),
    ("XBINNING", "1"),
    ("YBINNING", "1"),
    ("FILTER", "'IRCUT   '"),
    ("GAIN", "200"),
    ("EXPTIME", "30."),
    ("TOTALEXP", "12900."),
];

fn header(cards: &[(&str, &str)]) -> FitsHeader {
    parse_primary_header(&fits_bytes(cards, 0)).unwrap()
}

fn seestar(extra: &[(&str, &str)]) -> FitsHeader {
    let mut cards: Vec<(&str, &str)> = SEESTAR.to_vec();
    cards.extend_from_slice(extra);
    header(&cards)
}

const NAME: &str = "Stacked_430_M 45_30.0s_IRCUT_20261011-061931.fit";

#[test]
fn a_seestar_stack_is_recognised_with_what_its_header_says() {
    let info = recognise(&seestar(&[("STACKCNT", "430")]), NAME).unwrap();
    assert_eq!(info.kind, DeviceKind::Seestar);
    // The maker's name is dropped; nothing of the unit's identifier is kept.
    assert_eq!(info.device, "Seestar S50 Pro");
    assert_eq!(info.poses, 430);
    assert_eq!(info.exposure_seconds, Some(30.0));
    assert_eq!(info.total_seconds, Some(12900.0));
    assert_eq!(info.filter.as_deref(), Some("IRCUT"));
    assert_eq!(info.gain, Some(200.0));
    assert_eq!((info.width, info.height), (2160, 3840));
    assert_eq!(info.family, "IRCUT_30s_g200");
    // No field solution in these cards; with one, it is said.
    assert!(!info.solved);
    let solved = seestar(&[
        ("STACKCNT", "430"),
        ("CTYPE1", "'RA---TAN-SIP'"),
        ("CRVAL1", "56.7093844642"),
    ]);
    assert!(recognise(&solved, NAME).unwrap().solved);
}

#[test]
fn the_name_is_compared_without_case() {
    assert!(recognise(&seestar(&[("STACKCNT", "8")]), "stacked_8_M 45.FIT").is_some());
}

#[test]
fn what_is_not_a_known_device_stack_is_left_alone() {
    // A Live stack of the ASIAIR: named `Stacked…`, counted, no known device.
    let asiair = header(&[
        ("SIMPLE", "T"),
        ("BITPIX", "16"),
        ("NAXIS", "2"),
        ("STACKCNT", "40"),
        ("CREATOR", "'ZWO ASIAIR'"),
    ]);
    assert_eq!(recognise(&asiair, "Stacked_40.fit"), None);
    // A header without a writer.
    let anonymous = header(&[("SIMPLE", "T"), ("BITPIX", "16"), ("STACKCNT", "40")]);
    assert_eq!(recognise(&anonymous, "Stacked_40.fit"), None);
    // A Seestar sub-frame, not a stack: no stack count, and not named so.
    assert_eq!(recognise(&seestar(&[]), "Light_M 45_30.0s.fit"), None);
    assert_eq!(recognise(&seestar(&[]), NAME), None);
    // A stack of one frame is not a stack.
    assert_eq!(recognise(&seestar(&[("STACKCNT", "1")]), NAME), None);
    // The Seestar's stack named otherwise (a renamed copy).
    assert_eq!(recognise(&seestar(&[("STACKCNT", "430")]), "M45.fit"), None);
}

#[test]
fn only_the_linear_cube_a_seestar_writes_is_taken() {
    let eight_bit = header(&[
        ("SIMPLE", "T"),
        ("BITPIX", "8"),
        ("NAXIS", "3"),
        ("NAXIS3", "3"),
        ("CREATOR", "'ZWO Seestar S50 Pro'"),
        ("STACKCNT", "430"),
    ]);
    assert_eq!(recognise(&eight_bit, NAME), None);
    let mono = header(&[
        ("SIMPLE", "T"),
        ("BITPIX", "16"),
        ("NAXIS", "2"),
        ("CREATOR", "'ZWO Seestar S50 Pro'"),
        ("STACKCNT", "430"),
    ]);
    assert_eq!(recognise(&mono, NAME), None);
    let signed = header(&[
        ("SIMPLE", "T"),
        ("BITPIX", "16"),
        ("NAXIS", "3"),
        ("NAXIS3", "3"),
        ("BZERO", "0"),
        ("CREATOR", "'ZWO Seestar S50 Pro'"),
        ("STACKCNT", "430"),
    ]);
    assert_eq!(recognise(&signed, NAME), None);
}

fn image(name: &str, poses: u64, family: &str) -> DeviceImage {
    DeviceImage {
        relative_path: format!("M45/device/{name}"),
        file_name: name.to_string(),
        bytes: 49_775_040,
        info: DeviceImageInfo {
            kind: DeviceKind::Seestar,
            device: "Seestar S50 Pro".to_string(),
            poses,
            exposure_seconds: Some(if family.contains("30s") { 30.0 } else { 10.0 }),
            total_seconds: None,
            filter: Some("IRCUT".to_string()),
            gain: Some(200.0),
            width: 2160,
            height: 3840,
            solved: true,
            family: family.to_string(),
        },
    }
}

fn lights(label: &str, frames: usize) -> LightFamily {
    LightFamily {
        label: label.to_string(),
        frames,
    }
}

#[test]
fn an_image_that_is_alone_in_its_family_covers_it_and_is_taken() {
    let found = families(
        &[image("Stacked_430.fit", 430, "IRCUT_30s_g200")],
        &[lights("IRCUT_30s_g200", 430)],
    );
    assert_eq!(found.len(), 1);
    let family = &found[0];
    assert_eq!(family.coverage, Coverage::Covers);
    assert_eq!(family.default_source, Source::Device);
    assert_eq!(family.use_file.as_deref(), Some("Stacked_430.fit"));
    assert_eq!(family.lights, 430);
    assert_eq!(family.aside_poses, 0);
    assert!(family.restackable());
}

#[test]
fn the_real_night_the_thirty_second_family_is_covered_the_ten_second_one_is_split() {
    let images = [
        image("Stacked_28.fit", 28, "IRCUT_10s_g200"),
        image("Stacked_8.fit", 8, "IRCUT_10s_g200"),
        image("Stacked_430.fit", 430, "IRCUT_30s_g200"),
    ];
    let found = families(
        &images,
        &[lights("IRCUT_30s_g200", 430), lights("IRCUT_10s_g200", 36)],
    );
    // The biggest family first.
    assert_eq!(found[0].label, "IRCUT_30s_g200");
    assert_eq!(found[0].coverage, Coverage::Covers);
    assert_eq!(found[0].default_source, Source::Device);
    // The split family: the poses are here, so they are stacked again and the
    // device choice has no image ("Unavailable"); the images stay named,
    // the biggest first.
    let split = &found[1];
    assert_eq!(split.label, "IRCUT_10s_g200");
    assert_eq!(split.coverage, Coverage::Split);
    assert_eq!(split.images, ["Stacked_28.fit", "Stacked_8.fit"]);
    assert_eq!(split.default_source, Source::Restack);
    assert_eq!(split.use_file, None);
    assert_eq!(split.aside_poses, 0);
    assert_eq!(split.lights, 36);
}

#[test]
fn a_split_family_whose_poses_are_not_here_takes_the_biggest_image_and_names_what_it_leaves() {
    let images = [
        image("Stacked_8.fit", 8, "IRCUT_10s_g200"),
        image("Stacked_28.fit", 28, "IRCUT_10s_g200"),
    ];
    let found = families(&images, &[]);
    assert_eq!(found.len(), 1);
    let family = &found[0];
    assert_eq!(family.coverage, Coverage::Split);
    assert_eq!(family.default_source, Source::Device);
    assert_eq!(family.use_file.as_deref(), Some("Stacked_28.fit"));
    // The 8 poses of the other image are left aside, said.
    assert_eq!(family.aside_poses, 8);
    assert_eq!(family.lights, 0);
    assert!(!family.restackable());
}

#[test]
fn a_family_without_poses_here_and_one_image_has_only_the_image() {
    let found = families(&[image("Stacked_430.fit", 430, "IRCUT_30s_g200")], &[]);
    assert_eq!(found[0].default_source, Source::Device);
    assert!(!found[0].restackable());
}

#[test]
fn poses_without_a_device_image_make_no_device_family() {
    assert!(families(&[], &[lights("IRCUT_30s_g200", 430)]).is_empty());
}

#[test]
fn a_file_name_that_cannot_leave_the_folder_is_refused() {
    assert!(is_plain_file_name("Stacked_430_M 45.fit"));
    for bad in [
        "",
        ".",
        "..",
        "a/b.fit",
        "a\\b.fit",
        ".hidden.fit",
        "x\0.fit",
    ] {
        assert!(!is_plain_file_name(bad), "{bad:?}");
    }
}

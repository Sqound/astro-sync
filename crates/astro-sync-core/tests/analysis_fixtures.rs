//! Fixture-backed tests: a short, committed, synthetic PHD2 guide log and a
//! short, committed, synthetic ASIAIR Autorun log (both under
//! `tests/fixtures/`), plus 2-3 synthetic FITS headers built directly in this
//! file rather than committed as binary fixtures -- the FITS header format is
//! entirely mechanical (fixed 80-byte cards, 2880-byte blocks, an `END` card),
//! so a builder makes the exact bytes under test as explicit as a binary
//! fixture would, without a `.fits` file no diff tool renders usefully.
//!
//! Every timestamp and identifier in the fixtures is synthetic (year 2030,
//! generic "Mount" device name, no personal path or real place name).

use astro_sync_core::analysis::{asiair, fits, phd2};

const PHD2_FIXTURE: &str = include_str!("fixtures/phd2_guide_log_short.txt");
const ASIAIR_FIXTURE: &str = include_str!("fixtures/asiair_autorun_log_short.txt");

#[test]
fn the_committed_phd2_fixture_parses_into_one_session_with_its_events() {
    let log = phd2::parse_phd2_guide_log(
        PHD2_FIXTURE,
        &phd2::Phd2Options {
            utc_offset: "+00:00".to_string(),
        },
    )
    .expect("the committed fixture is well-formed");

    assert_eq!(log.sessions.len(), 1);
    let session = &log.sessions[0];
    assert_eq!(session.pixel_scale_arcsec_per_px, Some(1.85));
    assert_eq!(session.samples.len(), 6);
    assert_eq!(
        session.started_at_utc.as_deref(),
        Some("2030-03-01T22:00:00.000Z")
    );
    assert_eq!(
        session.ended_at_utc.as_deref(),
        Some("2030-03-01T22:41:12.000Z")
    );

    assert_eq!(session.settle_events.len(), 2);
    assert_eq!(
        session.settle_events[0].outcome,
        phd2::SettleOutcome::Started
    );
    assert_eq!(
        session.settle_events[1].outcome,
        phd2::SettleOutcome::Complete
    );

    assert_eq!(session.star_lost_events.len(), 1);
    assert_eq!(
        session.star_lost_events[0].status_text.as_deref(),
        Some("SNR too low")
    );

    // Every sample's arcsec offset traces back to a parsed raw-distance
    // pixel value times the log's own declared pixel scale -- the evidence
    // rule, checked at the level of one concrete number.
    let first = &session.samples[0];
    assert_eq!(first.ra_offset_arcsec, -0.120 * 1.85);
    assert_eq!(first.dec_offset_arcsec, 0.180 * 1.85);
}

#[test]
fn the_committed_asiair_fixture_parses_into_the_expected_event_counts() {
    let log = asiair::parse_asiair_autorun_log(
        ASIAIR_FIXTURE,
        &asiair::AsiairOptions {
            utc_offset: "+00:00".to_string(),
        },
    )
    .expect("the committed fixture is well-formed");

    assert_eq!(log.dither_events.len(), 2);
    assert_eq!(log.settle_events.len(), 1);
    assert_eq!(log.settle_timeout_events.len(), 1);
    assert_eq!(log.autofocus_events.len(), 1);
    assert_eq!(log.autofocus_events[0].temperature_c, Some(10.0));
    assert_eq!(log.autofocus_result_events.len(), 1);
    assert!(log.autofocus_result_events[0].succeeded);
    assert_eq!(log.meridian_flip_events.len(), 1);
}

/// Builds a minimal, valid, synthetic FITS primary header from a list of
/// `(keyword, value)` cards -- the same mechanical construction
/// `astro_sync_core::analysis::fits`'s own unit tests use (duplicated here
/// rather than shared, since that helper is a private `#[cfg(test)]` item
/// of a different compilation unit; see `README.md`).
fn build_fits_header_bytes(cards: &[(&str, &str)]) -> Vec<u8> {
    const CARD_LEN: usize = 80;
    const BLOCK_LEN: usize = 2880;
    let mut out = Vec::new();
    for (keyword, value) in cards {
        let card = format!("{keyword:<8}= {value}");
        let mut bytes = card.into_bytes();
        bytes.resize(CARD_LEN, b' ');
        out.extend_from_slice(&bytes);
    }
    let mut end_card = b"END".to_vec();
    end_card.resize(CARD_LEN, b' ');
    out.extend_from_slice(&end_card);
    let remainder = out.len() % BLOCK_LEN;
    if remainder != 0 {
        out.resize(out.len() + (BLOCK_LEN - remainder), b' ');
    }
    out
}

#[test]
fn three_synthetic_fits_headers_group_into_two_acquisition_families() {
    let frame_a = build_fits_header_bytes(&[
        ("EXPTIME", "300.0"),
        ("FILTER", "'Ha      '"),
        ("GAIN", "100"),
        ("CCD-TEMP", "-10.1"),
        ("DATE-OBS", "'2030-03-01T22:00:00.000'"),
        ("XBINNING", "1"),
        ("YBINNING", "1"),
    ]);
    // Same acquisition family as `frame_a`: the temperature only differs by
    // 0.3 degrees, rounding to the same whole-degree class.
    let frame_b = build_fits_header_bytes(&[
        ("EXPTIME", "300.0"),
        ("FILTER", "'Ha      '"),
        ("GAIN", "100"),
        ("CCD-TEMP", "-9.8"),
        ("DATE-OBS", "'2030-03-01T22:05:30.000'"),
        ("XBINNING", "1"),
        ("YBINNING", "1"),
    ]);
    // A different filter: its own family.
    let frame_c = build_fits_header_bytes(&[
        ("EXPTIME", "300.0"),
        ("FILTER", "'OIII    '"),
        ("GAIN", "100"),
        ("CCD-TEMP", "-10.0"),
        ("DATE-OBS", "'2030-03-01T22:11:00.000'"),
        ("XBINNING", "1"),
        ("YBINNING", "1"),
    ]);

    let records: Vec<(String, fits::FitsAcquisitionRecord)> = [
        ("images/frame-a.fits", &frame_a),
        ("images/frame-b.fits", &frame_b),
        ("images/frame-c.fits", &frame_c),
    ]
    .into_iter()
    .map(|(path, bytes)| {
        let header = fits::parse_primary_header(bytes).expect("synthetic header is well-formed");
        (path.to_string(), fits::extract_acquisition_record(&header))
    })
    .collect();

    let families = fits::group_into_families(&records);
    assert_eq!(families.len(), 2);

    let ha_family = families
        .iter()
        .find(|f| f.filter.as_deref() == Some("Ha"))
        .expect("an Ha family exists");
    assert_eq!(
        ha_family.frame_paths,
        vec!["images/frame-a.fits", "images/frame-b.fits"]
    );
    assert_eq!(ha_family.temperature_class_c, Some(-10));

    let oiii_family = families
        .iter()
        .find(|f| f.filter.as_deref() == Some("OIII"))
        .expect("an OIII family exists");
    assert_eq!(oiii_family.frame_paths, vec!["images/frame-c.fits"]);
}

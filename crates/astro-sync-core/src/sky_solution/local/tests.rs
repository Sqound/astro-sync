use super::*;
use crate::test_support::{TempDir, fits_bytes};
use std::fs;

/// A night of the working folder with the master of one target (NGC 7000:
/// pointing and optics in its header) and a render beside it.
fn night(root: &Path, night_id: &str, render: &[u8]) -> PathBuf {
    let folder = root.join(format!("2026-08-17 · {night_id}"));
    let target = folder.join("NGC 7000");
    fs::create_dir_all(target.join("stacked")).unwrap();
    fs::write(
        folder.join(".astro-sync-night.json"),
        format!(
            r#"{{"format":1,"nightId":"{night_id}","serviceUrl":"https://s","createdAt":"2026-08-17T10:00:00Z"}}"#
        ),
    )
    .unwrap();
    fs::write(
        target.join(".astro-sync-target.json"),
        r#"{"format":1,"targetId":"openngc-ngc7000"}"#,
    )
    .unwrap();
    fs::write(
        target.join("stacked/nofilter_180s_g100_integration.fit"),
        fits_bytes(
            &[
                ("NAXIS", "3"),
                ("ASFAMILY", "'nofilter_180s_g100'"),
                ("EXPTIME", "180.0"),
                ("GAIN", "100"),
                ("STACKCNT", "18"),
                ("RA", "314.27499"),
                ("DEC", "44.306946"),
                ("XPIXSZ", "3.76"),
                ("FOCALLEN", "190.0"),
            ],
            16,
        ),
    )
    .unwrap();
    fs::write(target.join("stacked/NGC_7000_raw.tif"), render).unwrap();
    folder
}

fn missing(path: &Path) -> MissingCapture {
    MissingCapture {
        id: "capture-1".into(),
        session_id: None,
        original_name: "NGC_7000_raw.tif".into(),
        sha256: file_sha256(path).unwrap(),
        byte_size: fs::metadata(path).unwrap().len(),
        main_image_id: None,
        main_image_source: None,
        night_date: None,
        gallery_label: None,
        thumbnail: None,
        targets: Vec::new(),
    }
}

#[test]
fn a_render_of_a_night_takes_the_pointing_and_the_scale_of_its_master() {
    let dir = TempDir::new("solve-local-night");
    let folder = night(dir.path(), "n-17", b"a render of a night");
    let render = folder.join("NGC 7000/stacked/NGC_7000_raw.tif");
    let found = local_results(dir.path(), &[missing(&render)]);
    assert_eq!(found.len(), 1);
    let local = found.values().next().unwrap();
    assert_eq!(local.path, render);
    assert_eq!(local.hints.ra_deg, Some(314.27499));
    assert_eq!(local.hints.dec_deg, Some(44.306946));
    let scale = local.hints.pixel_scale_arcsec.unwrap();
    assert!((scale - 206.264_806 * 3.76 / 190.0).abs() < 1e-6, "{scale}");
}

#[test]
fn the_image_of_a_plan_takes_the_master_of_the_nights_its_record_names() {
    let dir = TempDir::new("solve-local-plan");
    night(dir.path(), "n-17", b"night 17");
    night(dir.path(), "n-19", b"night 19");
    let plan = dir.path().join("NGC 7000 · p-1");
    fs::create_dir_all(plan.join("stacked")).unwrap();
    fs::write(
        plan.join(".astro-sync-plan.json"),
        r#"{"format":1,"projectId":"p-1","targetId":"openngc-ngc7000","name":"NGC 7000"}"#,
    )
    .unwrap();
    let image = plan.join("stacked/NGC_7000_plan_raw.tif");
    fs::write(&image, b"the image of the plan").unwrap();
    fs::write(
        plan.join("stacked/NGC_7000_plan.json"),
        r#"{"format":1,"masters":[{"nightId":"n-17"},{"nightId":"n-19"}]}"#,
    )
    .unwrap();
    let found = local_results(dir.path(), &[missing(&image)]);
    let local = found.values().next().expect("found by its digest");
    assert_eq!(local.path, image);
    assert!(local.hints.ra_deg.is_some() && local.hints.pixel_scale_arcsec.is_some());
}

#[test]
fn only_the_images_the_space_lacks_a_solution_for_are_found() {
    let dir = TempDir::new("solve-local-other");
    let folder = night(dir.path(), "n-17", b"a render of a night");
    let render = folder.join("NGC 7000/stacked/NGC_7000_raw.tif");
    let mut other = missing(&render);
    other.sha256 = "0".repeat(64);
    assert!(local_results(dir.path(), &[other]).is_empty());
    assert!(local_results(dir.path(), &[]).is_empty());
    assert!(local_results(&dir.path().join("nowhere"), &[missing(&render)]).is_empty());
}

#[test]
fn what_the_capture_knows_completes_what_the_master_says_never_half_a_pointing() {
    let master = SolveHints {
        ra_deg: Some(314.0),
        dec_deg: Some(44.0),
        pixel_scale_arcsec: Some(4.08),
    };
    let target = SolveHints {
        ra_deg: Some(10.0),
        dec_deg: Some(20.0),
        pixel_scale_arcsec: None,
    };
    // The frames' own pointing first, the target's where the master is silent.
    assert_eq!(master.completed_by(target), master);
    let silent = SolveHints {
        pixel_scale_arcsec: Some(4.08),
        ..SolveHints::default()
    };
    let completed = silent.completed_by(target);
    assert_eq!(
        (completed.ra_deg, completed.dec_deg),
        (Some(10.0), Some(20.0))
    );
    assert_eq!(completed.pixel_scale_arcsec, Some(4.08));
    let half = SolveHints {
        ra_deg: Some(1.0),
        ..SolveHints::default()
    };
    assert_eq!(half.completed_by(target).ra_deg, Some(10.0));
}

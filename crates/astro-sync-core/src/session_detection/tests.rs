//! Synthetic fixtures only (no real data in the repository): headers written
//! by `test_support::fits_bytes`, logs written as text.

use super::*;
use crate::local_work::WorkProgress;
use crate::test_support::{TempDir, fits_bytes};
use crate::time_util::{format_iso8601_utc_millis, parse_iso8601_utc_to_unix_seconds};
use std::fs;
use std::sync::Mutex;

/* -------------------------------- Fixtures -------------------------------- */

const SITE: (f64, f64) = (43.8803, 7.4290);

struct Tree {
    dir: TempDir,
}

impl Tree {
    fn new(label: &str) -> Self {
        Self {
            dir: TempDir::new(label),
        }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().join(relative)
    }

    fn write(&self, relative: &str, bytes: &[u8]) {
        let path = self.path(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn frame(&self, relative: &str, cards: &[(String, String)]) {
        let cards: Vec<(&str, &str)> = cards
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        self.write(relative, &fits_bytes(&cards, 16));
    }
}

fn unix(text: &str) -> f64 {
    parse_iso8601_utc_to_unix_seconds(text).unwrap()
}

fn quoted(text: &str) -> String {
    format!("'{text}'")
}

fn date_obs(t: f64) -> String {
    quoted(format_iso8601_utc_millis(t).trim_end_matches('Z'))
}

/// A light of the ASIAIR at the test site.
fn light(
    object: Option<&str>,
    t: f64,
    exposure: f64,
    position: Option<(f64, f64)>,
) -> Vec<(String, String)> {
    let mut cards = vec![
        ("IMAGETYP".to_string(), quoted("Light")),
        ("DATE-OBS".to_string(), date_obs(t)),
        ("EXPTIME".to_string(), format!("{exposure:.1}")),
        ("GAIN".to_string(), "100".to_string()),
        ("CCD-TEMP".to_string(), "-10.0".to_string()),
        ("INSTRUME".to_string(), quoted("ZWO ASI533MC Pro")),
        ("TELESCOP".to_string(), quoted("ZWO AM5N")),
        ("FOCALLEN".to_string(), "190".to_string()),
        ("SITELAT".to_string(), format!("{}", SITE.0)),
        ("SITELONG".to_string(), format!("{}", SITE.1)),
    ];
    if let Some(object) = object {
        cards.push(("OBJECT".to_string(), quoted(object)));
    }
    if let Some((ra, dec)) = position {
        cards.push(("RA".to_string(), format!("{ra:.4}")));
        cards.push(("DEC".to_string(), format!("{dec:.4}")));
    }
    cards
}

fn with(mut cards: Vec<(String, String)>, key: &str, value: &str) -> Vec<(String, String)> {
    cards.retain(|(k, _)| k != key);
    cards.push((key.to_string(), value.to_string()));
    cards
}

fn calibration(kind: &str, t: f64, exposure: f64, temperature: f64) -> Vec<(String, String)> {
    vec![
        ("IMAGETYP".to_string(), quoted(kind)),
        ("DATE-OBS".to_string(), date_obs(t)),
        ("EXPTIME".to_string(), format!("{exposure}")),
        ("GAIN".to_string(), "100".to_string()),
        ("CCD-TEMP".to_string(), format!("{temperature:.1}")),
        ("INSTRUME".to_string(), quoted("ZWO ASI533MC Pro")),
    ]
}

fn target(id: &str, designation: &str, ra_hours: f64, dec: f64, size: f64) -> CatalogueTarget {
    CatalogueTarget {
        id: id.to_string(),
        designation: designation.to_string(),
        name: None,
        aliases: Vec::new(),
        type_label: None,
        constellation_label: None,
        ra_hours: Some(ra_hours),
        dec_degrees: Some(dec),
        magnitude: None,
        size_label: None,
        size_arcmin: Some(size),
        image_path: None,
    }
}

fn catalogue() -> Vec<CatalogueTarget> {
    vec![
        target("t-ic1396", "IC 1396", 21.6517, 57.5, 170.0),
        target("t-ngc7000", "NGC 7000", 20.9881, 44.52, 120.0),
        target("t-ic1318", "IC 1318", 20.4339, 40.5, 240.0),
        target("t-m29", "M29", 20.3989, 38.5233, 7.0),
        target("t-sh2-103", "Sh2-103", 20.85, 30.6667, 180.0),
        target("t-ngc6960", "NGC 6960", 20.7606, 30.7083, 70.0),
        target("t-m65", "M65", 11.3155, 13.0922, 9.0),
        target("t-m66", "M66", 11.3375, 12.9917, 9.0),
    ]
}

/// Mu Cephei, a star inside IC 1396.
const MU_CEPHEI: (f64, f64) = (325.3667, 57.6528);

fn request(tree: &Tree, fits: &[&str], calibration: &[&str], logs: &[&str]) -> DetectRequest {
    DetectRequest {
        fits: fits.iter().map(|p| tree.path(p)).collect(),
        calibration: calibration.iter().map(|p| tree.path(p)).collect(),
        logs: logs.iter().map(|p| tree.path(p)).collect(),
        subfolders: true,
        scope: None,
        filter: None,
    }
}

fn place(id: &str, lat: f64, lon: f64) -> KnownPlace {
    KnownPlace {
        id: id.to_string(),
        name: id.to_string(),
        latitude: lat,
        longitude: lon,
        time_zone: None,
        utc_offset_minutes: None,
    }
}

fn advanced() -> UserData {
    UserData {
        account: AccountRights {
            advanced_planning: true,
            several_plans_per_night: true,
        },
        ..UserData::default()
    }
}

fn detect(request: &DetectRequest, user: &UserData) -> Proposal {
    detect_sessions(request, &catalogue(), user, &DetectOptions::default()).unwrap()
}

/// The night of 17/18 August: NGC 7000 then Mu Cephei (IC 1396), and the
/// night of 19/20 August: Mu Cephei again.
fn two_nights(tree: &Tree) {
    let start = unix("2026-08-17T21:00:00Z");
    for i in 0..6 {
        let t = start + f64::from(i) * 190.0;
        tree.frame(
            &format!("fits/NGC_7000/lights/n{i}.fit"),
            &light(Some("NGC 7000"), t, 180.0, Some((314.82, 44.52))),
        );
    }
    let start = unix("2026-08-17T22:30:00Z");
    for i in 0..8 {
        let t = start + f64::from(i) * 190.0;
        tree.frame(
            &format!("fits/Mu_Cephei/night1/lights/m{i}.fit"),
            &light(Some("Mu Cephei"), t, 180.0, Some(MU_CEPHEI)),
        );
    }
    let start = unix("2026-08-19T21:30:00Z");
    for i in 0..10 {
        let t = start + f64::from(i) * 190.0;
        tree.frame(
            &format!("fits/Mu_Cephei/night2/lights/m{i}.fit"),
            &light(Some("Mu Cephei"), t, 180.0, Some(MU_CEPHEI)),
        );
    }
}

/* --------------------------------- Reading -------------------------------- */

#[test]
fn the_folder_of_fits_is_required() {
    let error = read_locations(&DetectRequest::default(), &DetectOptions::default()).unwrap_err();
    assert_eq!(error.code(), "no_fits_folder");
}

#[test]
fn a_calibration_folder_decides_the_type_and_a_linked_file_is_read_once() {
    let tree = Tree::new("detect-read");
    let t = unix("2026-08-17T22:00:00Z");
    tree.frame(
        "fits/night/lights/a.fit",
        &light(Some("NGC 7000"), t, 180.0, None),
    );
    // The ASIAIR writes `Light` on darks taken in its « Light » mode.
    tree.frame(
        "cal/DARK/set/d1.fit",
        &with(
            calibration("Light", t + 30_000.0, 180.0, -10.0),
            "OBJECT",
            "'Pi Pegasi'",
        ),
    );
    tree.frame(
        "fits/night/stacks/Stacked3_NGC.fit",
        &light(Some("NGC 7000"), t, 180.0, None),
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(tree.path("cal/DARK/set"), tree.path("fits/night/dark"))
            .unwrap();
        std::os::unix::fs::symlink(tree.path("fits"), tree.path("fits/night/loop")).unwrap();
    }
    let seen = Mutex::new(Vec::<WorkProgress>::new());
    let sink = |p: &WorkProgress| seen.lock().unwrap().push(p.clone());
    let found = Mutex::new(Vec::<super::FoundSoFar>::new());
    let found_sink = |f: &super::FoundSoFar| found.lock().unwrap().push(f.clone());
    let options = DetectOptions {
        progress: Some(&sink),
        found: Some(&found_sink),
        ..DetectOptions::default()
    };
    let reading = read_locations(&request(&tree, &["fits"], &["cal"], &[]), &options).unwrap();
    // "Found so far", at the end: the stack is no frame of a night.
    let last_found = found.lock().unwrap().last().cloned().unwrap();
    assert_eq!(
        (
            last_found.lights,
            last_found.calibration_files,
            last_found.calibration_sets,
            last_found.calibration_kinds.clone(),
        ),
        (1, 1, 1, vec!["dark".to_string()])
    );
    assert_eq!((last_found.nights, last_found.targets), (1, 1));
    assert_eq!(
        reading.frames.len(),
        3,
        "the dark reached by the link is read once"
    );
    let dark = reading
        .frames
        .iter()
        .find(|f| f.path.ends_with("d1.fit"))
        .unwrap();
    assert_eq!(dark.kind, Some(FrameType::Dark));
    assert!(dark.kind_from_folder);
    // The FITS are walked first, but the dark lives in the calibration
    // folder only: it is read there.
    #[cfg(unix)]
    assert!(dark.in_calibration_location);
    assert!(dark.path.starts_with(tree.path("cal")));
    assert!(reading.frames.iter().any(|f| f.stacked));
    #[cfg(unix)]
    assert!(
        reading
            .links
            .iter()
            .any(|l| l.problem == crate::scanner::LinkProblem::Loop)
    );
    let all = seen.lock().unwrap();
    // The search for the files speaks first (how many met, in which folder,
    // no total: nothing can be measured yet), then the headers are read.
    let walks: Vec<_> = all
        .iter()
        .filter(|p| p.kind == crate::local_work::WorkKind::Scan)
        .collect();
    assert!(!walks.is_empty(), "the walk says where it is");
    assert!(walks.iter().all(|p| p.total == 0));
    assert!(walks.iter().all(|p| {
        p.current
            .as_deref()
            .is_some_and(|c| Path::new(c).is_absolute())
    }));
    assert_eq!(
        all.first().map(|p| p.kind),
        Some(crate::local_work::WorkKind::Scan)
    );
    let seen: Vec<_> = all
        .iter()
        .filter(|p| p.kind == crate::local_work::WorkKind::Headers)
        .collect();
    let last = seen.last().unwrap();
    assert_eq!((last.done, last.total), (3, 3));
    // The whole path of the file being read (the window shows it).
    assert!(
        seen[0]
            .current
            .as_deref()
            .is_some_and(|p| Path::new(p).is_absolute())
    );
}

#[test]
fn a_stop_ends_the_reading() {
    let tree = Tree::new("detect-cancel");
    tree.frame(
        "fits/a.fit",
        &light(Some("M65"), unix("2026-05-14T13:09:56Z"), 10.0, None),
    );
    let cancel = CancelFlag::new();
    cancel.cancel();
    let options = DetectOptions {
        cancel: Some(&cancel),
        ..DetectOptions::default()
    };
    let error = read_locations(&request(&tree, &["fits"], &[], &[]), &options).unwrap_err();
    assert_eq!(error.code(), "cancelled");
}

#[test]
fn without_sub_folders_only_the_top_files_are_read() {
    let tree = Tree::new("detect-flat");
    let t = unix("2026-08-17T22:00:00Z");
    tree.frame("fits/a.fit", &light(Some("NGC 7000"), t, 180.0, None));
    tree.frame(
        "fits/deeper/b.fit",
        &light(Some("NGC 7000"), t + 200.0, 180.0, None),
    );
    let mut wanted = request(&tree, &["fits"], &[], &[]);
    wanted.subfolders = false;
    let reading = read_locations(&wanted, &DetectOptions::default()).unwrap();
    assert_eq!(reading.frames.len(), 1);
}

/* -------------------------------- Grouping -------------------------------- */

#[test]
fn nights_sessions_targets_and_the_multi_night_group() {
    let tree = Tree::new("detect-group");
    two_nights(&tree);
    let proposal = detect(&request(&tree, &["fits"], &[], &[]), &advanced());
    assert_eq!(proposal.sessions.len(), 2);
    let first = &proposal.sessions[0];
    assert_eq!(first.evening_date, "2026-08-17");
    let designations: Vec<&str> = first
        .targets
        .iter()
        .map(|t| t.designation.as_str())
        .collect();
    assert_eq!(designations, ["NGC 7000", "IC 1396"]);
    let ic1396 = &first.targets[1];
    assert!(
        matches!(ic1396.source, TargetSource::Coordinates { .. }),
        "a star's OBJECT, matched by its position"
    );
    assert_eq!(ic1396.objects, ["Mu Cephei"]);
    assert_eq!(ic1396.frames, 8);
    assert_eq!(ic1396.acquisition[0].frames, 8);
    assert_eq!(proposal.sessions[1].evening_date, "2026-08-19");

    assert_eq!(proposal.multi_night.len(), 1);
    let group = &proposal.multi_night[0];
    assert_eq!(group.designation, "IC 1396");
    assert_eq!(group.nights.len(), 2);
    assert!(
        (group.goal_seconds - 18.0 * 180.0).abs() < 1e-6,
        "the goal is the total acquired"
    );
    assert_eq!(group.status, "finished");
    assert!(!group.locked);
    let line = proposal
        .lines
        .iter()
        .find(|l| l.kind == LineKind::MultiNight)
        .unwrap();
    assert!(line.checked && !line.locked);
    assert_eq!(proposal.reading.lights, 24);
}

#[test]
fn a_night_crossing_midnight_is_one_night_and_noon_cuts_it() {
    let tree = Tree::new("detect-noon");
    // 23:50 and 00:10 UTC: one night; the next evening is another.
    for (i, t) in [
        "2026-08-17T23:50:00Z",
        "2026-08-18T00:10:00Z",
        "2026-08-18T21:00:00Z",
    ]
    .iter()
    .enumerate()
    {
        tree.frame(
            &format!("fits/{i}.fit"),
            &light(Some("NGC 7000"), unix(t), 60.0, None),
        );
    }
    let proposal = detect(&request(&tree, &["fits"], &[], &[]), &advanced());
    let evenings: Vec<&str> = proposal
        .sessions
        .iter()
        .map(|s| s.evening_date.as_str())
        .collect();
    assert_eq!(evenings, ["2026-08-17", "2026-08-18"]);
}

#[test]
fn a_drifting_pointing_with_one_object_stays_one_target() {
    let tree = Tree::new("detect-drift");
    let start = unix("2026-08-14T21:30:00Z");
    // « Sadr »: the frames sit between IC 1318 and the cluster M29.
    for i in 0..10 {
        let ra = 305.4 + f64::from(i) * 0.05;
        let dec = 39.9 + f64::from(i % 3) * 0.2;
        tree.frame(
            &format!("fits/Sadr/{i}.fit"),
            &light(
                Some("Sadr"),
                start + f64::from(i) * 190.0,
                180.0,
                Some((ra, dec)),
            ),
        );
    }
    let proposal = detect(&request(&tree, &["fits"], &[], &[]), &advanced());
    let targets = &proposal.sessions[0].targets;
    assert_eq!(targets.len(), 1);
    assert_eq!(
        targets[0].designation, "IC 1318",
        "the nebula that fills the frame, not M29"
    );
}

#[test]
fn the_folder_names_the_target_and_the_clock_follows_the_place() {
    let tree = Tree::new("detect-vespera");
    let start = unix("2026-05-14T13:09:56Z");
    for i in 0..20 {
        tree.frame(
            &format!("fits/2026-05-14_13-07-00_observation_M65/01-images-initial/img-{i:04}.fits"),
            &[
                (
                    "DATE-OBS".to_string(),
                    date_obs(start + f64::from(i) * 10.0),
                ),
                ("EXPOSURE".to_string(), "10.".to_string()),
                ("INSTRUME".to_string(), quoted("vespera2-d83addf3001a")),
                ("FOCAL".to_string(), "250".to_string()),
                ("BAYERPAT".to_string(), quoted("GBRG")),
            ],
        );
    }
    let wanted = request(&tree, &["fits"], &[], &[]);
    let mut user = advanced();
    user.places = vec![
        place("p-tokyo", 35.6812, 139.7671),
        place("p-france", SITE.0, SITE.1),
    ];

    user.default_place_id = Some("p-tokyo".into());
    let tokyo = detect(&wanted, &user);
    let session = &tokyo.sessions[0];
    assert_eq!(session.targets[0].designation, "M65");
    assert!(matches!(
        session.targets[0].source,
        TargetSource::Folder { .. }
    ));
    let clock = session.clock.as_ref().unwrap();
    assert!(
        !clock.suspect,
        "22:09 in Tokyo: the night is consistent in UTC"
    );
    assert!(clock.samples[0].sun_altitude_deg < -20.0);
    assert_eq!(session.shift_hours, 0);
    let place_line = tokyo
        .lines
        .iter()
        .find(|l| l.kind == LineKind::Place)
        .unwrap();
    assert!(
        place_line.ask,
        "the frames name no place: asked before creating"
    );

    user.default_place_id = Some("p-france".into());
    let france = detect(&wanted, &user);
    let session = &france.sessions[0];
    let clock = session.clock.as_ref().unwrap();
    assert!(clock.suspect);
    assert_eq!(clock.reasons[0].code, ClockReasonCode::SunUp);
    let shift = clock.proposed_shift_hours.unwrap();
    assert_eq!(session.shift_hours, shift);
    assert_eq!(
        session.start,
        format_iso8601_utc_millis(start + f64::from(shift) * 3600.0)
    );
    let line = france
        .lines
        .iter()
        .find(|l| l.reason == "clock.shift")
        .unwrap();
    assert!(line.checked, "the clock correction is ticked by default");

    user.default_place_id = None;
    let nowhere = detect(&wanted, &user);
    assert_eq!(
        nowhere.sessions[0].clock.as_ref().unwrap().place,
        PlaceUsed::None
    );
}

#[test]
fn a_header_altitude_that_disagrees_makes_the_clock_suspect() {
    let tree = Tree::new("detect-altaz");
    let start = unix("2026-08-17T22:30:00Z");
    for i in 0..4 {
        tree.frame(
            &format!("fits/{i}.fit"),
            &with(
                light(
                    Some("Mu Cephei"),
                    start + f64::from(i) * 190.0,
                    180.0,
                    Some(MU_CEPHEI),
                ),
                "OBJCTALT",
                "5.0",
            ),
        );
    }
    let proposal = detect(&request(&tree, &["fits"], &[], &[]), &advanced());
    let clock = proposal.sessions[0].clock.as_ref().unwrap();
    assert!(
        clock
            .reasons
            .iter()
            .any(|r| r.code == ClockReasonCode::AltitudeDisagrees)
    );
}

#[test]
fn an_unknown_object_is_a_new_target_and_nothing_is_to_solve() {
    let tree = Tree::new("detect-solve");
    let t = unix("2026-05-14T13:10:00Z");
    let bare = |t: f64| {
        vec![
            ("DATE-OBS".to_string(), date_obs(t)),
            ("EXPTIME".to_string(), "10".to_string()),
            ("SITELAT".to_string(), "35.68".to_string()),
            ("SITELONG".to_string(), "139.77".to_string()),
        ]
    };
    for i in 0..3 {
        tree.frame(
            &format!("fits/misc/{i}.fit"),
            &bare(t + f64::from(i) * 12.0),
        );
    }
    tree.frame(
        "fits/comet/c.fit",
        &with(bare(t + 100.0), "OBJECT", "'Comète 2026 A1'"),
    );
    let wanted = request(&tree, &["fits"], &[], &[]);
    let proposal = detect(&wanted, &advanced());
    let targets = &proposal.sessions[0].targets;
    let unsolved = targets
        .iter()
        .find(|t| t.source == TargetSource::ToSolve)
        .unwrap();
    assert_eq!(unsolved.frames, 3);
    assert!(
        unsolved.solve_frame.is_some(),
        "one frame of the group is marked"
    );
    assert!(
        targets
            .iter()
            .any(|t| t.source == TargetSource::UnknownObject)
    );
    let line = proposal
        .lines
        .iter()
        .find(|l| l.reason == "target.toSolve")
        .unwrap();
    assert!(!line.checked);

    // A solver that exists here is asked for one frame only.
    let calls = Mutex::new(0);
    let solve = |_: &Path| {
        *calls.lock().unwrap() += 1;
        Ok((169.733, 13.092))
    };
    let options = DetectOptions {
        solver: Some(&solve),
        ..DetectOptions::default()
    };
    let solved = detect_sessions(&wanted, &catalogue(), &advanced(), &options).unwrap();
    assert_eq!(*calls.lock().unwrap(), 1);
    let m65 = solved.sessions[0]
        .targets
        .iter()
        .find(|t| t.designation == "M65")
        .unwrap();
    assert!(matches!(m65.source, TargetSource::PlateSolved { .. }));
    assert_eq!(m65.frames, 3);

    // A solver that cannot solve says why, and the group stays to be
    // solved; a solution on no target of the catalogue keeps its position.
    let failing = |_: &Path| Err(crate::sky_solution::SolveFailure::TooFewStars);
    let options = DetectOptions {
        solver: Some(&failing),
        ..DetectOptions::default()
    };
    let failed = detect_sessions(&wanted, &catalogue(), &advanced(), &options).unwrap();
    let still = failed.sessions[0]
        .targets
        .iter()
        .find(|t| t.source == TargetSource::ToSolve)
        .unwrap();
    assert_eq!(
        still.solve_failure,
        Some(crate::sky_solution::SolveFailure::TooFewStars)
    );
    assert_eq!(still.solved_position, None);
    let off_list = |_: &Path| Ok((331.29, 46.48));
    let options = DetectOptions {
        solver: Some(&off_list),
        ..DetectOptions::default()
    };
    let placed = detect_sessions(&wanted, &catalogue(), &advanced(), &options).unwrap();
    let still = placed.sessions[0]
        .targets
        .iter()
        .find(|t| t.source == TargetSource::ToSolve)
        .unwrap();
    assert_eq!(still.solved_position, Some((331.29, 46.48)));
    assert_eq!(still.solve_failure, None);
}

#[test]
fn calibration_is_attached_by_exposure_temperature_gain_and_date() {
    let tree = Tree::new("detect-calibration");
    let night = unix("2026-08-14T21:30:00Z");
    for i in 0..3 {
        tree.frame(
            &format!("fits/Sadr/{i}.fit"),
            &light(Some("IC 1318"), night + f64::from(i) * 190.0, 180.0, None),
        );
    }
    let set = |folder: &str, kind: &str, day: &str, exposure: f64, temperature: f64| {
        for i in 0..2 {
            tree.frame(
                &format!("cal/{folder}/{i}.fit"),
                &calibration(
                    kind,
                    unix(&format!("{day}T07:00:00Z")) + f64::from(i) * 10.0,
                    exposure,
                    temperature,
                ),
            );
        }
    };
    set("DARK/180-cold", "Dark", "2026-08-08", 180.0, -10.0);
    set("DARK/180-warm", "Dark", "2026-08-15", 180.0, 5.0);
    set("DARK/90", "Dark", "2026-08-15", 90.0, -10.0);
    set("FLAT/1p6", "Flat", "2026-08-13", 1.61, -10.0);
    set("FLAT/600ms", "Flat", "2026-08-08", 0.6, -10.0);
    set("DARK_FLAT/1p6", "Dark", "2026-08-13", 1.6, -10.0);
    set("DARK_FLAT/600ms", "Dark", "2026-08-08", 0.6, -10.0);
    let proposal = detect(&request(&tree, &["fits"], &["cal"], &[]), &advanced());
    let attached = &proposal.sessions[0].calibration;
    let folder_of = |kind: FrameType| {
        let id = &attached.iter().find(|a| a.kind == kind).unwrap().set_id;
        proposal
            .calibration_sets
            .iter()
            .find(|s| &s.id == id)
            .unwrap()
            .folder
            .clone()
    };
    assert!(
        folder_of(FrameType::Dark).ends_with("180-cold"),
        "same exposure and temperature"
    );
    assert!(
        folder_of(FrameType::Flat).ends_with("FLAT/1p6"),
        "the nearest in time"
    );
    assert!(
        folder_of(FrameType::DarkFlat).ends_with("DARK_FLAT/1p6"),
        "the flats' exposure"
    );
    assert_eq!(attached.len(), 3);
    assert_eq!(proposal.set_aside.calibration_unattached.len(), 8);
}

fn phd2_log(begins_local: &str, seconds: u32) -> String {
    let mut lines = vec![
        "PHD2 version 2.6.13, Log version 2.5. Log enabled at 2026-08-17 22:50:00".to_string(),
        String::new(),
        format!("Guiding Begins at {begins_local}"),
        String::new(),
        "Pixel scale = 2.00 arc-sec/px, Binning = 1, Focal length = 250 mm".to_string(),
        "Frame,Time,mount,dx,dy,RARawDistance,DECRawDistance,RAGuideDistance,DECGuideDistance,RADuration,RADirection,DECDuration,DECDirection,XStep,YStep,StarMass,SNR,ErrorCode".to_string(),
    ];
    for (frame, elapsed) in (0..seconds).step_by(20).enumerate() {
        lines.push(format!(
            "{},{elapsed}.000,Mount,0,0,0.1,0.1,0,0,0,,0,,0,0,30000,12,0",
            frame + 1
        ));
    }
    lines.join("\n")
}

#[test]
fn a_shared_log_goes_to_both_targets_and_a_morning_log_to_none() {
    let tree = Tree::new("detect-logs");
    two_nights(&tree);
    // PHD2 in local time (+02:00), from before NGC 7000 to the end of the
    // first Mu Cephei frames.
    tree.write(
        "logs/2026-08-17_18/PHD2_GuideLog_2026-08-17_225500.txt",
        phd2_log("2026-08-17 22:55:00", 7200).as_bytes(),
    );
    // ASIAIR: dithers after the Mu Cephei frames only.
    let mut asiair = vec!["Log enabled at 2026/08/18 00:29:00".to_string()];
    for i in 0..8 {
        let t = unix("2026-08-17T22:30:00Z") + f64::from(i) * 190.0 + 182.0 + 7200.0;
        let local = format_iso8601_utc_millis(t);
        asiair.push(format!(
            "{} {} [Guide] Dither",
            local[..10].replace('-', "/"),
            &local[11..19]
        ));
    }
    tree.write(
        "logs/2026-08-17_18/Autorun_Log_2026-08-18_002900.txt",
        asiair.join("\n").as_bytes(),
    );
    tree.write(
        "logs/2026-08-17_18/Autorun_Log_2026-08-18_002900_CHN.txt",
        b"translation",
    );
    // The morning's darks, with dithers in the « Light » mode.
    let mut morning = vec!["Log enabled at 2026/08/18 09:40:00".to_string()];
    for i in 0..4 {
        let t = unix("2026-08-18T07:40:00Z") + f64::from(i) * 190.0;
        tree.frame(
            &format!("cal/DARK/morning/d{i}.fit"),
            &with(
                calibration("Light", t, 180.0, -10.0),
                "OBJECT",
                "'Pi Pegasi'",
            ),
        );
        let local = format_iso8601_utc_millis(t + 182.0 + 7200.0);
        morning.push(format!(
            "{} {} [Guide] Dither",
            local[..10].replace('-', "/"),
            &local[11..19]
        ));
    }
    tree.write(
        "logs/setup/Autorun_Log_2026-08-18_094000.txt",
        morning.join("\n").as_bytes(),
    );

    let proposal = detect(&request(&tree, &["fits"], &["cal"], &["logs"]), &advanced());
    let night = &proposal.sessions[0];
    let log_of = |name: &str| {
        proposal
            .logs
            .iter()
            .find(|l| l.path.ends_with(name))
            .unwrap()
    };
    let phd2 = log_of("PHD2_GuideLog_2026-08-17_225500.txt");
    assert_eq!(phd2.utc_offset.as_deref(), Some("+02:00"));
    let attached = night.logs.iter().find(|a| a.log_id == phd2.id).unwrap();
    assert_eq!(
        attached.targets,
        ["t-ic1396", "t-ngc7000"],
        "one log, two targets"
    );
    let asiair = log_of("Autorun_Log_2026-08-18_002900.txt");
    let attached = night.logs.iter().find(|a| a.log_id == asiair.id).unwrap();
    assert_eq!(attached.targets, ["t-ic1396"]);
    let morning = log_of("Autorun_Log_2026-08-18_094000.txt");
    assert!(
        morning.sessions.is_empty(),
        "a log of calibration is tied to no night"
    );
    assert_eq!(proposal.set_aside.logs_unattached.len(), 1);
    assert_eq!(
        proposal.set_aside.other_files.len(),
        1,
        "the _CHN rendition"
    );
}

#[test]
fn a_log_tie_prefers_a_whole_hour() {
    use crate::analysis::log_offset::{LogClock, match_log};
    // Guiding samples all night long: +1:30 and +2:00 hold the same frames.
    let frames: Vec<(f64, f64)> = (0..5)
        .map(|i| {
            (
                10_000.0 + f64::from(i) * 200.0,
                10_180.0 + f64::from(i) * 200.0,
            )
        })
        .collect();
    let samples: Vec<f64> = (0..2000)
        .map(|i| 10_000.0 - 9_000.0 + f64::from(i) * 10.0 + 7200.0)
        .collect();
    let found = match_log(LogClock::Phd2, &samples, &frames, None).unwrap();
    assert_eq!(found.shift_seconds % 3600, 0);
    assert_eq!(found.frames.len(), 5);
    let forced = match_log(LogClock::Phd2, &samples, &frames, Some((7200, 7200))).unwrap();
    assert_eq!(forced.shift_seconds, 7200);
}

/* -------------------------------- Matching -------------------------------- */

#[test]
fn places_within_ten_kilometres_the_nearest_ticked() {
    let tree = Tree::new("detect-places");
    two_nights(&tree);
    let wanted = request(&tree, &["fits"], &[], &[]);
    let mut user = advanced();
    user.places = vec![
        place("p-far", SITE.0 + 0.06, SITE.1),
        place("p-near", SITE.0 + 0.01, SITE.1),
        place("p-away", SITE.0 + 1.0, SITE.1),
    ];
    let proposal = detect(&wanted, &user);
    let found = &proposal.places[0];
    assert_eq!(found.action, PlaceAction::UseNearest);
    assert_eq!(found.chosen.as_deref(), Some("p-near"));
    assert_eq!(found.candidates.len(), 2, "within 10 km only");

    user.places.clear();
    let proposal = detect(&wanted, &user);
    assert_eq!(proposal.places[0].action, PlaceAction::Create);
}

#[test]
fn equipment_is_matched_by_name_and_focal_length() {
    let tree = Tree::new("detect-equipment");
    two_nights(&tree);
    let mut user = advanced();
    user.equipment = vec![
        KnownEquipment {
            id: "cam".into(),
            name: "ASI533MC Pro".into(),
            kind: "camera".into(),
            ..KnownEquipment::default()
        },
        KnownEquipment {
            id: "mount".into(),
            name: "AM5N".into(),
            kind: "mount".into(),
            ..KnownEquipment::default()
        },
        KnownEquipment {
            id: "scope".into(),
            name: "Askar".into(),
            kind: "telescope".into(),
            focal_length_mm: Some(192.0),
            ..KnownEquipment::default()
        },
    ];
    let proposal = detect(&request(&tree, &["fits"], &[], &[]), &user);
    assert_eq!(proposal.equipment.len(), 1);
    let item = &proposal.equipment[0];
    assert_eq!(item.camera.as_ref().unwrap().equipment_id, "cam");
    assert_eq!(item.mount.as_ref().unwrap().equipment_id, "mount");
    assert_eq!(item.telescope.as_ref().unwrap().by, "focal-length");
}

#[test]
fn the_filters_the_frames_name_are_matched_to_the_filters_of_mon_materiel() {
    // The filters are part of the equipment a night is created with, when "My
    // equipment" holds them.
    let tree = Tree::new("detect-equipment-filters");
    for (i, filter) in ["Ha", "OIII", "Ha"].iter().enumerate() {
        let t = unix("2026-08-17T21:00:00Z") + i as f64 * 400.0;
        tree.frame(
            &format!("src/n1/l{i}.fit"),
            &with(
                light(Some("NGC 7000"), t, 300.0, None),
                "FILTER",
                &quoted(filter),
            ),
        );
    }
    let mut user = advanced();
    user.equipment = vec![
        KnownEquipment {
            id: "f-ha".into(),
            name: "Ha 7 nm".into(),
            kind: "filter".into(),
            ..KnownEquipment::default()
        },
        KnownEquipment {
            id: "f-uv".into(),
            name: "UV/IR Cut".into(),
            kind: "filter".into(),
            ..KnownEquipment::default()
        },
    ];
    let proposal = detect(&request(&tree, &["src"], &[], &[]), &user);
    let item = &proposal.equipment[0];
    assert_eq!(item.filters, vec!["Ha".to_string(), "OIII".to_string()]);
    // « Ha » is a filter of the space, « OIII » is not (nothing is invented).
    let matched: Vec<&str> = item
        .filter_matches
        .iter()
        .map(|m| m.equipment_id.as_str())
        .collect();
    assert_eq!(matched, vec!["f-ha"]);
}

#[test]
fn a_night_without_filter_asks_unless_the_session_declares_one() {
    let tree = Tree::new("detect-filter-ask");
    two_nights(&tree);
    let wanted = request(&tree, &["fits"], &[], &[]);
    let mut user = advanced();
    let proposal = detect(&wanted, &user);
    assert!(
        proposal.sessions.iter().all(|s| s.filter_ask),
        "no FILTER anywhere: every night asks, nothing is supposed"
    );
    assert!(proposal.sessions[0].targets[0].filters.is_empty());

    user.sessions = vec![KnownSession {
        id: "sess-17".into(),
        evening_date: "2026-08-17".into(),
        targets: vec![
            KnownSlot {
                target_id: Some("t-ngc7000".into()),
                filter: Some("UV/IR Cut".into()),
                ..KnownSlot::default()
            },
            KnownSlot {
                target_id: Some("t-ic1396".into()),
                filter: Some("UV/IR Cut".into()),
                ..KnownSlot::default()
            },
        ],
        ..KnownSession::default()
    }];
    let proposal = detect(&wanted, &user);
    let first = &proposal.sessions[0];
    assert!(!first.filter_ask, "the existing session declares it");
    assert_eq!(first.targets[0].filters, vec!["UV/IR Cut".to_string()]);
    assert!(
        proposal.sessions[1].filter_ask,
        "the other night still asks"
    );
}

#[test]
fn existing_sessions_and_plans_are_never_duplicated() {
    let tree = Tree::new("detect-existing");
    two_nights(&tree);
    let wanted = request(&tree, &["fits"], &[], &[]);
    let mut user = advanced();
    user.sessions = vec![KnownSession {
        id: "sess-17".into(),
        evening_date: "2026-08-17".into(),
        targets: vec![KnownSlot {
            target_id: Some("t-ngc7000".into()),
            ..KnownSlot::default()
        }],
        ..KnownSession::default()
    }];
    user.plans = vec![KnownPlan {
        id: "plan-19".into(),
        name: "M33 – RVB".into(),
        evening_date: "2026-08-19".into(),
        status: "pending".into(),
        target_ids: vec!["t-m33".into()],
        ..KnownPlan::default()
    }];
    let proposal = detect(&wanted, &user);
    let first = &proposal.sessions[0];
    assert_eq!(
        first.existing,
        SessionMatch::Existing {
            session_id: "sess-17".into()
        }
    );
    assert_eq!(first.targets[0].existing, TargetMatch::InSession);
    assert_eq!(
        first.targets[1].existing,
        TargetMatch::AddToSession,
        "its slot is free"
    );
    let replace = proposal
        .lines
        .iter()
        .find(|l| l.reason == "plan.replace")
        .unwrap();
    assert!(replace.checked, "« Remplacer » is ticked by default");
    assert!(replace.text.contains("M33 – RVB"));

    // The same target planned: reused, set to executed.
    user.plans[0].target_ids = vec!["t-ic1396".into()];
    let proposal = detect(&wanted, &user);
    assert!(matches!(
        proposal.sessions[1].existing,
        SessionMatch::ReusePlan { .. }
    ));

    // A slot taken by another target: asked, never settled alone.
    user.sessions[0].targets[0].start = Some("2026-08-17T22:00:00Z".into());
    user.sessions[0].targets[0].end = Some("2026-08-17T23:30:00Z".into());
    user.sessions[0].targets[0].target_id = Some("t-other".into());
    let proposal = detect(&wanted, &user);
    let mu = &proposal.sessions[0].targets[1];
    assert_eq!(mu.existing, TargetMatch::Overlap);
    let line = proposal
        .lines
        .iter()
        .find(|l| l.reason == "target.overlap")
        .unwrap();
    assert!(line.ask && !line.checked);
}

#[test]
fn a_free_account_keeps_one_plan_a_night_and_a_locked_group() {
    let tree = Tree::new("detect-free");
    two_nights(&tree);
    let user = UserData {
        plans: vec![KnownPlan {
            id: "done".into(),
            name: "Already done".into(),
            evening_date: "2026-08-19".into(),
            status: "executed".into(),
            session_id: Some("elsewhere".into()),
            ..KnownPlan::default()
        }],
        ..UserData::default()
    };
    let proposal = detect(&request(&tree, &["fits"], &[], &[]), &user);
    assert!(matches!(
        proposal.sessions[1].existing,
        SessionMatch::PlanExists { .. }
    ));
    let line = proposal
        .lines
        .iter()
        .find(|l| l.reason == "plan.freeRuleExisting")
        .unwrap();
    assert!(!line.checked && line.locked);
    let group = proposal
        .lines
        .iter()
        .find(|l| l.kind == LineKind::MultiNight)
        .unwrap();
    assert!(group.locked && !group.checked);
    assert_eq!(group.reason, "multiNight.locked");
    assert!(proposal.notes.iter().any(|n| n.key == "account.free"));
    assert_eq!(
        proposal
            .lines
            .iter()
            .filter(|l| l.reason == "session.create")
            .count(),
        1
    );
}

#[test]
fn a_proposal_follows_the_offer_the_account_holds_now() {
    use super::RightsRefresh;
    let tree = Tree::new("detect-refresh-rights");
    two_nights(&tree);
    // Computed on the free account: the multi-night plan is shown locked.
    let mut proposal = detect(&request(&tree, &["fits"], &[], &[]), &UserData::default());
    let group_line = |p: &Proposal| {
        p.lines
            .iter()
            .find(|l| l.kind == LineKind::MultiNight)
            .cloned()
            .unwrap()
    };
    assert!(group_line(&proposal).locked && !group_line(&proposal).checked);
    assert_eq!(proposal.account, AccountRights::default());

    // The same rights: nothing to do.
    assert_eq!(
        proposal.refresh_rights(AccountRights::default()),
        RightsRefresh::Unchanged
    );

    // The offer is granted after the detection: the plan opens, ticked by
    // default, as if the detection had run with it.
    let granted = AccountRights {
        advanced_planning: true,
        several_plans_per_night: true,
    };
    let group_id = proposal.multi_night[0].id.clone();
    assert_eq!(
        proposal.refresh_rights(granted),
        RightsRefresh::Updated {
            unlocked: vec![format!("multiNight.create:{group_id}")]
        }
    );
    let line = group_line(&proposal);
    assert!(!line.locked && line.checked);
    assert_eq!(line.reason, "multiNight.create");
    assert!(
        proposal
            .notes
            .iter()
            .all(|n| !n.key.starts_with("account."))
    );
    assert!(!proposal.multi_night[0].locked);
    // It is the proposal a detection under the offer gives.
    let fresh = detect(&request(&tree, &["fits"], &[], &[]), &advanced());
    assert_eq!(proposal.lines, fresh.lines);
    assert_eq!(proposal.notes, fresh.notes);

    // The offer ends: the plan closes again.
    assert_eq!(
        proposal.refresh_rights(AccountRights::default()),
        RightsRefresh::Updated { unlocked: vec![] }
    );
    assert!(group_line(&proposal).locked);
}

#[test]
fn a_change_of_plans_per_night_needs_the_plans_read_again() {
    use super::RightsRefresh;
    let tree = Tree::new("detect-refresh-stale");
    two_nights(&tree);
    let user = UserData {
        plans: vec![KnownPlan {
            id: "done".into(),
            name: "Already done".into(),
            evening_date: "2026-08-19".into(),
            status: "executed".into(),
            session_id: Some("elsewhere".into()),
            ..KnownPlan::default()
        }],
        ..UserData::default()
    };
    let mut proposal = detect(&request(&tree, &["fits"], &[], &[]), &user);
    // "One plan per night" decided the night of the 19th: only a new
    // detection knows what becomes of it.
    assert_eq!(
        proposal.refresh_rights(advanced().account),
        RightsRefresh::Stale
    );
}

#[test]
fn the_sending_of_a_session_sees_its_range_and_its_targets() {
    let tree = Tree::new("detect-scope");
    two_nights(&tree);
    let mut wanted = request(&tree, &["fits"], &[], &[]);
    wanted.scope = Some(SessionScope {
        from: "2026-08-17T20:00:00Z".into(),
        to: "2026-08-18T04:00:00Z".into(),
        targets: vec![ScopeTarget {
            id: "t-ic1396".into(),
            designation: "IC 1396".into(),
            ..ScopeTarget::default()
        }],
    });
    let proposal = detect(&wanted, &advanced());
    let scoped = proposal.scoped.unwrap();
    assert_eq!(scoped.targets[0].files.len(), 8);
    assert_eq!(scoped.unassigned.len(), 1);
    assert_eq!(scoped.unassigned[0].designation, "NGC 7000");
    assert_eq!(scoped.unassigned[0].files.len(), 6);
    assert_eq!(scoped.other_nights, 10);
    assert_eq!(proposal.sessions.len(), 1);
}

#[test]
fn a_proposal_reads_back_as_it_was_written() {
    let tree = Tree::new("detect-serde");
    two_nights(&tree);
    let proposal = detect(&request(&tree, &["fits"], &[], &[]), &advanced());
    let json = serde_json::to_string(&proposal).unwrap();
    let back: Proposal = serde_json::from_str(&json).unwrap();
    assert_eq!(back, proposal);
}

/* -------------------------------- Organise -------------------------------- */

/// Two nights of a monochrome camera (Ha, OIII) sharing one set of darks.
fn mono(tree: &Tree) -> Proposal {
    for (night, start) in [
        ("n1", "2026-08-17T21:00:00Z"),
        ("n2", "2026-08-19T21:00:00Z"),
    ] {
        for (i, filter) in ["Ha", "OIII", "Ha"].iter().enumerate() {
            let t = unix(start) + i as f64 * 400.0;
            tree.frame(
                &format!("src/{night}/l{i}.fit"),
                &with(
                    light(Some("NGC 7000"), t, 300.0, None),
                    "FILTER",
                    &quoted(filter),
                ),
            );
        }
    }
    for i in 0..2 {
        tree.frame(
            &format!("src/DARK/d{i}.fit"),
            &calibration(
                "Dark",
                unix("2026-08-18T08:00:00Z") + f64::from(i) * 10.0,
                300.0,
                -10.0,
            ),
        );
    }
    let mut wanted = request(tree, &["src"], &[], &[]);
    wanted.calibration = vec![tree.path("src/DARK")];
    detect(&wanted, &advanced())
}

/// The nights' folders as the result page names them: the place and the
/// session created on the service (`YYYY-MM-DD · <place> · <id court>`).
const N17: &str = "2026-08-17 · Maison · 3e7a91";
const N19: &str = "2026-08-19 · Maison · 5b2f00";

fn names(proposal: &Proposal) -> std::collections::BTreeMap<String, super::NightNaming> {
    proposal
        .sessions
        .iter()
        .map(|s| {
            let id = if s.evening_date == "2026-08-17" {
                "3e7a91c0-0000-4000-8000-000000000001"
            } else {
                "5b2f00aa-0000-4000-8000-000000000002"
            };
            (
                s.id.clone(),
                super::NightNaming {
                    place: Some("Maison".into()),
                    session_id: Some(id.into()),
                    folder: None,
                },
            )
        })
        .collect()
}

fn digest(path: &Path) -> String {
    crate::scanner::file_sha256(path).unwrap()
}

#[test]
fn leave_in_place_does_nothing_and_a_dry_run_writes_nothing() {
    let tree = Tree::new("organise-plan");
    let proposal = mono(&tree);
    let destination = tree.path("organised");
    let plan = plan_organise(&proposal, &destination, OrganiseMode::LeaveInPlace, &[]).unwrap();
    assert!(plan.actions.is_empty());
    assert_eq!(
        organise(&plan, &OrganiseOptions::default()).unwrap(),
        OrganiseOutcome::default()
    );
    let plan = plan_organise_named(
        &proposal,
        &destination,
        OrganiseMode::Move,
        &[],
        &names(&proposal),
    )
    .unwrap();
    assert_eq!(plan.files, 6 + 2 * 2, "the darks go into each night");
    assert!(!destination.exists(), "the dry run writes nothing");
    assert!(plan.summary.starts_with("Move 10 files"));
    // Without the sessions created, the night's folder still has the
    // working folder's form: its date and a short id.
    let night = Path::new(&plan.actions[0].destination)
        .strip_prefix(&destination)
        .unwrap()
        .components()
        .next()
        .unwrap()
        .as_os_str()
        .to_string_lossy()
        .into_owned();
    assert!(
        night.starts_with("2026-08-1") && night.contains(" · "),
        "{night}"
    );
    // The free space of the destination's disk (its first existing parent).
    assert!(plan.free_bytes.is_some_and(|b| b > 0));
}

#[test]
fn biases_are_organised_in_bias_and_both_names_are_read() {
    assert_eq!(
        FrameType::of_calibration_folder("bias"),
        Some(FrameType::Bias)
    );
    assert_eq!(
        FrameType::of_calibration_folder("biases"),
        Some(FrameType::Bias)
    );
    let tree = Tree::new("organise-bias");
    let t = unix("2026-08-17T21:00:00Z");
    tree.frame("src/n1/l0.fit", &light(Some("NGC 7000"), t, 300.0, None));
    tree.frame(
        "src/BIAS/b0.fit",
        &calibration("Bias", unix("2026-08-18T08:00:00Z"), 0.001, -10.0),
    );
    let mut wanted = request(&tree, &["src/n1"], &[], &[]);
    wanted.calibration = vec![tree.path("src/BIAS")];
    let proposal = detect(&wanted, &advanced());
    let destination = tree.path("organised");
    let plan = plan_organise(&proposal, &destination, OrganiseMode::Copy, &[]).unwrap();
    assert!(
        plan.actions
            .iter()
            .any(|a| a.destination.contains("/calibration/bias/b0.fit")),
        "{:?}",
        plan.actions
    );
}

#[test]
fn copy_builds_the_tree_with_a_filter_level_and_calibration_in_each_night() {
    let tree = Tree::new("organise-copy");
    let proposal = mono(&tree);
    let destination = tree.path("organised");
    let plan = plan_organise_named(
        &proposal,
        &destination,
        OrganiseMode::Copy,
        &[],
        &names(&proposal),
    )
    .unwrap();
    let outcome = organise(&plan, &OrganiseOptions::default()).unwrap();
    assert_eq!(outcome.done, 10);
    for relative in [
        &format!("{N17}/NGC 7000/lights/Ha/l0.fit"),
        &format!("{N17}/NGC 7000/lights/OIII/l1.fit"),
        &format!("{N19}/NGC 7000/lights/Ha/l2.fit"),
        &format!("{N17}/calibration/darks/d0.fit"),
        &format!("{N19}/calibration/darks/d1.fit"),
    ] {
        assert!(destination.join(relative).is_file(), "{relative}");
    }
    assert!(
        tree.path("src/n1/l0.fit").is_file(),
        "a copy leaves the source"
    );
    // Run again: already there, nothing overwritten.
    let again = plan_organise_named(
        &proposal,
        &destination,
        OrganiseMode::Copy,
        &[],
        &names(&proposal),
    )
    .unwrap();
    assert!(
        again
            .actions
            .iter()
            .all(|a| a.kind == OrganiseActionKind::AlreadyThere)
    );
}

#[test]
fn a_different_file_at_its_place_is_never_overwritten() {
    let tree = Tree::new("organise-conflict");
    let proposal = mono(&tree);
    let destination = tree.path("organised");
    tree.write(
        &format!("organised/{N17}/NGC 7000/lights/Ha/l0.fit"),
        b"someone else's",
    );
    let plan = plan_organise_named(
        &proposal,
        &destination,
        OrganiseMode::Copy,
        &[],
        &names(&proposal),
    )
    .unwrap();
    assert_eq!(plan.conflicts.len(), 1);
    assert_eq!(plan.conflicts[0].reason, "destination-exists");
    organise(&plan, &OrganiseOptions::default()).unwrap();
    assert_eq!(
        fs::read(destination.join(format!("{N17}/NGC 7000/lights/Ha/l0.fit"))).unwrap(),
        b"someone else's"
    );
}

#[test]
fn move_checks_each_copy_then_removes_the_source() {
    let tree = Tree::new("organise-move");
    let proposal = mono(&tree);
    let before = digest(&tree.path("src/DARK/d0.fit"));
    let destination = tree.path("organised");
    let plan = plan_organise_named(
        &proposal,
        &destination,
        OrganiseMode::Move,
        &[],
        &names(&proposal),
    )
    .unwrap();
    let outcome = organise(&plan, &OrganiseOptions::default()).unwrap();
    assert_eq!(outcome.verified, 10);
    assert_eq!(
        outcome.removed_sources, 8,
        "a shared dark is removed after its last copy"
    );
    assert!(!tree.path("src/DARK/d0.fit").exists());
    for night in [N17, N19] {
        assert_eq!(
            digest(&destination.join(night).join("calibration/darks/d0.fit")),
            before
        );
    }
}

#[test]
fn move_stops_at_the_first_failure_and_keeps_the_rest() {
    let tree = Tree::new("organise-stop");
    let proposal = mono(&tree);
    let destination = tree.path("organised");
    let plan = plan_organise_named(
        &proposal,
        &destination,
        OrganiseMode::Move,
        &[],
        &names(&proposal),
    )
    .unwrap();
    // The second file disappears between the summary and the run.
    let second = PathBuf::from(&plan.actions[1].source);
    fs::remove_file(&second).unwrap();
    let outcome = organise(&plan, &OrganiseOptions::default()).unwrap();
    assert!(outcome.stopped);
    assert_eq!(outcome.failed.len(), 1);
    assert_eq!(outcome.done, 1);
    for action in &plan.actions[2..] {
        assert!(
            Path::new(&action.source).exists(),
            "untouched after the failure"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_points_at_the_source() {
    let tree = Tree::new("organise-link");
    let proposal = mono(&tree);
    let destination = tree.path("organised");
    let plan = plan_organise_named(
        &proposal,
        &destination,
        OrganiseMode::Symlink,
        &[],
        &names(&proposal),
    )
    .unwrap();
    assert!(!plan.windows_no_link);
    organise(&plan, &OrganiseOptions::default()).unwrap();
    let link = destination.join(format!("{N17}/NGC 7000/lights/Ha/l0.fit"));
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::canonicalize(&link).unwrap(),
        fs::canonicalize(tree.path("src/n1/l0.fit")).unwrap()
    );
}

#[cfg(unix)]
#[test]
fn files_already_linked_are_planned_as_in_place_links_and_a_run_creates_none() {
    let tree = Tree::new("organise-linked-again");
    let proposal = mono(&tree);
    let destination = tree.path("organised");
    let ask =
        |mode| plan_organise_named(&proposal, &destination, mode, &[], &names(&proposal)).unwrap();
    let first = ask(OrganiseMode::Symlink);
    let total = first.actions.len();
    assert!(first.actions.iter().all(|a| a.in_place_as.is_none()));
    organise(&first, &OrganiseOptions::default()).unwrap();
    // The treatments had linked everything: the plan says so, per file.
    let again = ask(OrganiseMode::Symlink);
    assert_eq!(again.actions.len(), total);
    assert!(again.actions.iter().all(|a| {
        a.kind == OrganiseActionKind::AlreadyThere && a.in_place_as == Some(InPlaceAs::Link)
    }));
    let outcome = organise(&again, &OrganiseOptions::default()).unwrap();
    assert_eq!(outcome.done, 0);
    assert_eq!(outcome.already_there, total);
    assert!(outcome.failed.is_empty());
    // A real copy is in place as a file of its own.
    let copied = tree.path("organised-copy");
    let copy = plan_organise_named(
        &proposal,
        &copied,
        OrganiseMode::Copy,
        &[],
        &names(&proposal),
    )
    .unwrap();
    organise(&copy, &OrganiseOptions::default()).unwrap();
    let again = plan_organise_named(
        &proposal,
        &copied,
        OrganiseMode::Copy,
        &[],
        &names(&proposal),
    )
    .unwrap();
    assert!(
        again
            .actions
            .iter()
            .all(|a| a.in_place_as == Some(InPlaceAs::File))
    );
}

#[cfg(unix)]
#[test]
fn the_sessions_created_become_night_folders_of_the_working_folder_with_links() {
    let tree = Tree::new("treat-prepare");
    let proposal = mono(&tree);
    let created = super::BulkCreateOutcome {
        sessions: proposal
            .sessions
            .iter()
            .zip([
                "3e7a91c0-0000-4000-8000-000000000001",
                "5b2f00aa-0000-4000-8000-000000000002",
            ])
            .map(|(s, id)| super::CreatedSession {
                detected_id: s.id.clone(),
                session_id: id.into(),
                evening_date: s.evening_date.clone(),
                action: "created".into(),
                plan_action: "created".into(),
                plan_id: None,
                designations: vec!["NGC 7000".into()],
                frames: 3,
                shift_hours: 0,
                added_targets: Vec::new(),
            })
            .collect(),
        ..super::BulkCreateOutcome::default()
    };
    let root = tree.path("work");
    let target = proposal.sessions[0].targets[0].target_id.clone().unwrap();
    let nights = super::prepare_treatments(
        &proposal,
        &created,
        &[target],
        &root,
        &[],
        "http://127.0.0.1:9130",
        "2026-10-16T12:00:00.000Z",
    )
    .unwrap();
    assert_eq!(nights.len(), 2);
    let first = &nights[0];
    let identity = crate::working_folder::read_night_identity(&first.night_folder).unwrap();
    assert_eq!(identity.night_id, created.sessions[0].session_id);
    let lights = first
        .night_folder
        .join(&first.targets[0].folder_name)
        .join("lights");
    let linked: Vec<_> = walkdir::WalkDir::new(&lights)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.path_is_symlink())
        .collect();
    assert_eq!(linked.len(), 3, "the three frames of the night, linked");
    assert!(first.night_folder.join("calibration/darks/d0.fit").exists());
    assert!(tree.path("src/n1/l0.fit").is_file(), "the original stays");
    // Again: nothing more is linked, nothing is overwritten.
    let again = super::prepare_treatments(
        &proposal,
        &created,
        &[],
        &root,
        &[],
        "http://127.0.0.1:9130",
        "2026-10-16T12:00:00.000Z",
    )
    .unwrap();
    assert_eq!(again[0].linked, 0);
    assert_eq!(again[0].night_folder, first.night_folder);
}

#[cfg(unix)]
#[test]
fn the_filter_read_from_the_folder_names_reaches_the_target_folder_of_the_night() {
    let tree = Tree::new("treat-filter");
    let mut proposal = mono(&tree);
    proposal.sessions[0].targets[0].filters.clear();
    proposal.sessions[0].targets[0].filter_hint = Some("Dualband".into());
    let created = super::BulkCreateOutcome {
        sessions: vec![super::CreatedSession {
            detected_id: proposal.sessions[0].id.clone(),
            session_id: "3e7a91c0-0000-4000-8000-000000000001".into(),
            evening_date: proposal.sessions[0].evening_date.clone(),
            action: "created".into(),
            plan_action: "created".into(),
            plan_id: None,
            designations: vec!["NGC 7000".into()],
            frames: 3,
            shift_hours: 0,
            added_targets: Vec::new(),
        }],
        ..super::BulkCreateOutcome::default()
    };
    let target = proposal.sessions[0].targets[0].target_id.clone().unwrap();
    let nights = super::prepare_treatments(
        &proposal,
        &created,
        &[target],
        &tree.path("work"),
        &[],
        "http://127.0.0.1:9130",
        "2026-10-16T12:00:00.000Z",
    )
    .unwrap();
    let folder = nights[0]
        .night_folder
        .join(&nights[0].targets[0].folder_name);
    assert_eq!(
        crate::working_folder::read_target_filter(&folder).as_deref(),
        Some("Dualband"),
        "the stacking and the plans know the filter the headers do not name"
    );
    assert!(tree.path("src/n1/l0.fit").is_file(), "the original stays");
}

/* -------------------------------- Service --------------------------------- */

#[test]
fn the_readers_of_the_service_are_tolerant() {
    use serde_json::json;
    let (places, default) = service::places_of(&json!({
        "places": [
            { "id": "p1", "name": "Col", "latitude": 43.88, "longitude": 7.43, "timeZone": "Europe/Paris" },
            { "id": "broken" }
        ],
        "defaultPlaceId": "p1"
    }));
    assert_eq!(places.len(), 1);
    assert_eq!(places[0].time_zone.as_deref(), Some("Europe/Paris"));
    assert_eq!(default.as_deref(), Some("p1"));

    let equipment = service::equipment_of(&json!({
        "equipment": [
            { "id": "e1", "type": "telescope", "name": "Askar", "specifications": { "focalLength": 250 } },
            { "id": "e2", "type": "camera", "name": "Old", "archivedAt": "2026-01-01T00:00:00Z" }
        ],
        "defaults": [ { "id": "d1", "type": "telescope", "name": "Seestar S50" } ]
    }));
    let ids: Vec<&str> = equipment.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["e1", "d1"]);
    assert_eq!(equipment[0].focal_length_mm, Some(250.0));

    let plans = service::plans_of(&json!({
        "plans": [
            { "id": "pl1", "status": "pending", "payload": { "date": "2026-07-31", "name": "M33 – RVB", "items": [ { "id": "i1", "targetId": "t-m33" } ] } }
        ]
    }));
    assert_eq!(plans[0].evening_date, "2026-07-31");
    // A plan the device closed is not planned any more.
    let closed = service::plans_of(&json!({
        "plans": [
            { "id": "pl2", "status": "active", "payload": { "date": "2026-08-19", "name": "Trompe", "closedAs": "archived", "items": [] } }
        ]
    }));
    assert!(!closed[0].is_pending());
    assert_eq!(plans[0].target_ids, ["t-m33"]);
    assert!(plans[0].is_pending());

    let session = service::session_of(
        "s1",
        Some("pl1".into()),
        "2026-08-17T21:00:00Z",
        &json!({ "night": { "targetIds": { "value": ["t-ngc7000"] } } }),
    );
    assert_eq!(session.evening_date, "2026-08-17");
    assert_eq!(session.targets[0].target_id.as_deref(), Some("t-ngc7000"));

    // The detail's targets carry their slots (the times observed, else
    // the plan's slot); a removed target holds none.
    let session = service::session_of(
        "s2",
        Some("pl2".into()),
        "2026-08-17T21:00:00Z",
        &json!({ "targets": [
            { "targetId": "ngc7000", "label": "NGC 7000", "state": "planned",
              "startedAt": null, "finishedAt": null,
              "slot": { "start": "2026-08-17T21:00:00.000Z", "end": "2026-08-17T22:30:00.000Z" } },
            { "targetId": "m31", "label": "M 31", "state": "added",
              "startedAt": "2026-08-17T23:00:00.000Z", "finishedAt": "2026-08-18T01:00:00.000Z", "slot": null },
            { "targetId": "m33", "state": "removed" }
        ] }),
    );
    let slots: Vec<_> = session
        .targets
        .iter()
        .map(|t| {
            (
                t.target_id.as_deref().unwrap(),
                t.start.as_deref(),
                t.end.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        slots,
        [
            (
                "ngc7000",
                Some("2026-08-17T21:00:00.000Z"),
                Some("2026-08-17T22:30:00.000Z")
            ),
            (
                "m31",
                Some("2026-08-17T23:00:00.000Z"),
                Some("2026-08-18T01:00:00.000Z")
            ),
        ]
    );

    let free = service::account_of(None, true);
    assert!(!free.advanced_planning && !free.several_plans_per_night);
    let private = service::account_of(None, false);
    assert!(private.advanced_planning && private.several_plans_per_night);
}

/* ----------------------- The image a device stacked ----------------------- */

/// A Seestar-like stacked image of NGC 7000 taken at `t` (its `DATE-OBS`).
fn device_stack(t: f64, poses: u32) -> Vec<(String, String)> {
    [
        ("SIMPLE", "T".to_string()),
        ("BITPIX", "16".to_string()),
        ("NAXIS", "3".to_string()),
        ("NAXIS1", "2160".to_string()),
        ("NAXIS2", "3840".to_string()),
        ("NAXIS3", "3".to_string()),
        ("BZERO", "32768".to_string()),
        ("CREATOR", quoted("ZWO Seestar S50 Pro")),
        ("STACKCNT", poses.to_string()),
        ("EXPTIME", "300.0".to_string()),
        ("TOTALEXP", format!("{}", f64::from(poses) * 300.0)),
        ("FILTER", quoted("IRCUT")),
        ("GAIN", "100".to_string()),
        ("OBJECT", quoted("NGC 7000")),
        ("DATE-OBS", date_obs(t)),
        ("SITELAT", format!("{}", SITE.0)),
        ("SITELONG", format!("{}", SITE.1)),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

#[test]
fn the_image_a_known_device_stacked_goes_to_its_target_and_is_no_longer_set_aside() {
    let tree = Tree::new("detect-device");
    let start = unix("2026-08-17T21:00:00Z");
    for i in 0..3 {
        tree.frame(
            &format!("src/lights/l{i}.fit"),
            &light(Some("NGC 7000"), start + f64::from(i) * 400.0, 300.0, None),
        );
    }
    // The device's own stack of the night, another of a night with no pose
    // here, and the Live stack of an ASIAIR (a header without a known device).
    tree.frame(
        "src/stacked_seestar/Stacked_3_NGC 7000.fit",
        &device_stack(start + 800.0, 3),
    );
    tree.frame(
        "src/stacked_seestar/Stacked_9_NGC 7000.fit",
        &device_stack(unix("2026-08-25T21:00:00Z"), 9),
    );
    tree.frame(
        "src/live/Stacked_40_NGC 7000.fit",
        &light(Some("NGC 7000"), start, 180.0, None),
    );
    let proposal = detect(&request(&tree, &["src"], &[], &[]), &advanced());

    assert_eq!(proposal.sessions.len(), 1);
    let target = &proposal.sessions[0].targets[0];
    assert_eq!(target.frames, 3);
    assert_eq!(
        target.device_images,
        vec![
            tree.path("src/stacked_seestar/Stacked_3_NGC 7000.fit")
                .display()
                .to_string()
        ]
    );
    // Not a pose: nothing of it is counted, nothing is in `files`.
    assert_eq!(proposal.reading.lights, 3);
    assert!(target.files.iter().all(|f| !f.contains("Stacked")));
    // Set aside: the Live stack, and the image no night here takes.
    let mut aside = proposal.set_aside.stacks.clone();
    aside.sort();
    assert_eq!(aside.len(), 2, "{aside:?}");
    assert!(aside[0].ends_with("Stacked_40_NGC 7000.fit"));
    assert!(aside[1].ends_with("Stacked_9_NGC 7000.fit"));
}

#[test]
fn ranger_les_fichiers_puts_the_device_image_in_the_device_folder_of_the_target() {
    let tree = Tree::new("organise-device");
    let start = unix("2026-08-17T21:00:00Z");
    for i in 0..3 {
        tree.frame(
            &format!("src/lights/l{i}.fit"),
            &light(Some("NGC 7000"), start + f64::from(i) * 400.0, 300.0, None),
        );
    }
    tree.frame(
        "src/stacked_seestar/Stacked_3_NGC 7000.fit",
        &device_stack(start + 800.0, 3),
    );
    let proposal = detect(&request(&tree, &["src"], &[], &[]), &advanced());
    let destination = tree.path("organised");
    for (mode, kind) in [
        (OrganiseMode::Copy, OrganiseActionKind::Copy),
        (OrganiseMode::Symlink, OrganiseActionKind::Link),
    ] {
        let plan = plan_organise(&proposal, &destination, mode, &[]).unwrap();
        assert_eq!(plan.files, 4, "three poses and the image");
        let image = plan
            .actions
            .iter()
            .find(|action| action.source.ends_with("Stacked_3_NGC 7000.fit"))
            .expect("the image is in the plan");
        assert_eq!(image.kind, kind);
        let place = Path::new(&image.destination);
        assert_eq!(place.parent().unwrap().file_name().unwrap(), "device");
        assert_eq!(
            place
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .file_name()
                .unwrap(),
            "NGC 7000",
            "{place:?}"
        );
        // The poses stay in `lights/`.
        assert!(
            plan.actions
                .iter()
                .filter(|a| a.source.contains("/l"))
                .all(|a| a.destination.contains("/lights/"))
        );
    }
    // Copied, then found in its folder by the night's detection.
    let plan = plan_organise(&proposal, &destination, OrganiseMode::Copy, &[]).unwrap();
    let outcome = organise(&plan, &OrganiseOptions::default()).unwrap();
    assert_eq!(outcome.done, 4);
    let copied = walkdir::WalkDir::new(&destination)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.path().ends_with("device/Stacked_3_NGC 7000.fit"))
        .count();
    assert_eq!(copied, 1);
}

#[test]
fn the_sessions_created_link_the_device_image_beside_the_poses() {
    let tree = Tree::new("treat-device");
    let start = unix("2026-08-17T21:00:00Z");
    for i in 0..3 {
        tree.frame(
            &format!("src/lights/l{i}.fit"),
            &light(Some("NGC 7000"), start + f64::from(i) * 400.0, 300.0, None),
        );
    }
    tree.frame(
        "src/stacked_seestar/Stacked_3_NGC 7000.fit",
        &device_stack(start + 800.0, 3),
    );
    let proposal = detect(&request(&tree, &["src"], &[], &[]), &advanced());
    let session = &proposal.sessions[0];
    let created = super::BulkCreateOutcome {
        sessions: vec![super::CreatedSession {
            detected_id: session.id.clone(),
            session_id: "3e7a91c0-0000-4000-8000-000000000001".into(),
            evening_date: session.evening_date.clone(),
            action: "created".into(),
            plan_action: "created".into(),
            plan_id: None,
            designations: vec!["NGC 7000".into()],
            frames: 3,
            shift_hours: 0,
            added_targets: Vec::new(),
        }],
        ..super::BulkCreateOutcome::default()
    };
    let target_id = session.targets[0].target_id.clone().unwrap();
    let nights = super::prepare_treatments(
        &proposal,
        &created,
        &[target_id],
        &tree.path("work"),
        &[],
        "http://127.0.0.1:9130",
        "2026-10-16T12:00:00.000Z",
    )
    .unwrap();
    let folder = nights[0]
        .night_folder
        .join(&nights[0].targets[0].folder_name);
    let image = folder.join("device/Stacked_3_NGC 7000.fit");
    assert!(image.is_file(), "the image is linked in device/");
    // A symbolic link; on Windows without developer mode, a hard link.
    assert!(
        cfg!(windows)
            || std::fs::symlink_metadata(&image)
                .unwrap()
                .file_type()
                .is_symlink()
    );
    assert!(
        tree.path("src/stacked_seestar/Stacked_3_NGC 7000.fit")
            .is_file(),
        "the original stays"
    );
    // And the night's detection finds it where it was put.
    let detection = crate::detection::detect_night(
        &nights[0].night_folder,
        &crate::detection::DetectionInput {
            evening_date: Some(session.evening_date.clone()),
            night_started_at: None,
            longitude: None,
            targets: vec![crate::detection::DetectionTarget {
                target_id: session.targets[0].target_id.clone().unwrap(),
                designation: "NGC 7000".to_string(),
                aliases: Vec::new(),
                ticked: true,
                online_final: false,
                stacking_planned: false,
            }],
        },
        &crate::detection::DetectOptions::default(),
    )
    .unwrap();
    let device = detection.targets[0]
        .device
        .as_ref()
        .expect("found in device/");
    assert_eq!(device.images.len(), 1);
    assert_eq!(detection.targets[0].light_frames, 3);
}

/* ------------------------- Flats by filter ------------------------- */

/// A night of four filters with `per_filter` sky flats of each, their
/// exposures all different (a sky flat's exposure follows the light).
fn four_filter_night(tree: &Tree, per_filter: u32) {
    let night = unix("2026-08-14T21:30:00Z");
    for (f, filter) in ["L", "R", "G", "B"].iter().enumerate() {
        for i in 0..2 {
            tree.frame(
                &format!("fits/Sadr/{filter}{i}.fit"),
                &with(
                    light(
                        Some("IC 1318"),
                        night + (f as f64 * 2.0 + f64::from(i)) * 190.0,
                        180.0,
                        None,
                    ),
                    "FILTER",
                    &quoted(filter),
                ),
            );
        }
        for i in 0..per_filter {
            tree.frame(
                &format!("cal/flats/{filter}{i}.fit"),
                &with(
                    calibration(
                        "Flat",
                        unix("2026-08-14T17:00:00Z") + f64::from(i) * 20.0,
                        2.0 + f64::from(i) * 0.37 + f as f64 * 5.0,
                        -10.0,
                    ),
                    "FILTER",
                    &quoted(filter),
                ),
            );
        }
    }
}

#[test]
fn sky_flats_of_different_exposures_make_one_set_per_filter() {
    let tree = Tree::new("detect-flats-by-filter");
    four_filter_night(&tree, 11);
    let proposal = detect(&request(&tree, &["fits"], &["cal"], &[]), &advanced());
    let flats: Vec<_> = proposal
        .calibration_sets
        .iter()
        .filter(|s| s.kind == Some(FrameType::Flat))
        .collect();
    assert_eq!(flats.len(), 4, "one set per filter: {flats:?}");
    assert!(flats.iter().all(|s| s.frames == 11));
    let attached = proposal.sessions[0]
        .calibration
        .iter()
        .filter(|a| a.kind == FrameType::Flat)
        .count();
    assert_eq!(attached, 4, "every filter has its flats attached");
    assert!(
        proposal.set_aside.calibration_unattached.is_empty(),
        "no flat is left aside: {:?}",
        proposal.set_aside.calibration_unattached
    );
}

#[test]
fn a_bias_half_a_degree_away_is_attached_and_one_three_degrees_away_is_not() {
    let tree = Tree::new("detect-bias-temperature");
    let night = unix("2026-08-14T21:30:00Z");
    for i in 0..2 {
        tree.frame(
            &format!("fits/Sadr/{i}.fit"),
            &with(
                light(Some("IC 1318"), night + f64::from(i) * 190.0, 180.0, None),
                "CCD-TEMP",
                "-5.0",
            ),
        );
    }
    // -5.0 and -4.5 are one set (the rounding to 2 °C split them), -2.0 is
    // another and does not fit lights taken at -5.
    for (i, temperature) in [-5.0, -5.0, -4.5, -4.5].iter().enumerate() {
        tree.frame(
            &format!("cal/bias/b{i}.fit"),
            &calibration(
                "Bias",
                unix("2026-08-14T08:00:00Z") + i as f64 * 5.0,
                0.001,
                *temperature,
            ),
        );
    }
    for i in 0..2 {
        tree.frame(
            &format!("cal/bias-warm/w{i}.fit"),
            &calibration(
                "Bias",
                unix("2026-08-14T08:10:00Z") + f64::from(i) * 5.0,
                0.001,
                -2.0,
            ),
        );
    }
    let proposal = detect(&request(&tree, &["fits"], &["cal"], &[]), &advanced());
    let bias: Vec<_> = proposal
        .calibration_sets
        .iter()
        .filter(|s| s.kind == Some(FrameType::Bias))
        .collect();
    assert_eq!(bias.len(), 2, "{bias:?}");
    let cold = bias.iter().find(|s| s.frames == 4).expect("one cold set");
    let warm = bias.iter().find(|s| s.frames == 2).expect("one warm set");
    let attached: Vec<&str> = proposal.sessions[0]
        .calibration
        .iter()
        .filter(|a| a.kind == FrameType::Bias)
        .map(|a| a.set_id.as_str())
        .collect();
    assert_eq!(attached, vec![cold.id.as_str()]);
    assert!(
        cold.files
            .iter()
            .all(|f| !proposal.set_aside.calibration_unattached.contains(f)),
        "the cold bias is not aside"
    );
    assert!(
        warm.files
            .iter()
            .all(|f| proposal.set_aside.calibration_unattached.contains(f)),
        "the bias three degrees away is left aside"
    );
}

#[test]
fn the_flats_of_a_filter_are_organised_in_their_own_folder() {
    let tree = Tree::new("organise-flats-by-filter");
    four_filter_night(&tree, 3);
    let proposal = detect(&request(&tree, &["fits"], &["cal"], &[]), &advanced());
    let plan = plan_organise(&proposal, &tree.path("organised"), OrganiseMode::Copy, &[]).unwrap();
    for filter in ["L", "R", "G", "B"] {
        let folder = format!("/calibration/flats/{filter}/");
        let in_folder = plan
            .actions
            .iter()
            .filter(|a| a.destination.contains(&folder))
            .count();
        assert_eq!(in_folder, 3, "{filter}: {:?}", plan.actions);
    }
    assert!(
        plan.actions
            .iter()
            .filter(|a| a.destination.contains("/calibration/flats/"))
            .all(|a| a
                .destination
                .split("/calibration/flats/")
                .nth(1)
                .unwrap()
                .contains('/')),
        "no flat is left loose beside the filter folders"
    );
}

/* ------------------- What is missing, what is not read ------------------ */

/// A mono night of 32 s lights in the V filter, with the calibration given.
fn v_night(tree: &Tree, darks: &[f64], flat_filters: &[&str]) {
    let start = unix("2026-08-11T21:10:00Z");
    for i in 0..4 {
        tree.frame(
            &format!("fits/IC1805/l{i}.fit"),
            &with(
                light(Some("IC 1805"), start + f64::from(i) * 40.0, 32.0, None),
                "FILTER",
                &quoted("V"),
            ),
        );
    }
    for (kind, exposure) in darks.iter().enumerate() {
        for i in 0..3 {
            tree.frame(
                &format!("cal/darks/{kind}-{i}.fit"),
                &calibration(
                    "Dark",
                    unix("2026-08-11T08:00:00Z") + f64::from(i) * 10.0,
                    *exposure,
                    -10.0,
                ),
            );
        }
    }
    for filter in flat_filters {
        for i in 0..3 {
            tree.frame(
                &format!("cal/flats/{filter}-{i}.fit"),
                &with(
                    calibration(
                        "Flat",
                        unix("2026-08-11T08:30:00Z") + f64::from(i) * 10.0,
                        1.5,
                        -10.0,
                    ),
                    "FILTER",
                    &quoted(filter),
                ),
            );
        }
    }
}

#[test]
fn darks_that_do_not_fit_and_a_flat_less_filter_are_warnings() {
    let tree = Tree::new("notes-warning");
    v_night(&tree, &[30.0], &["L", "R", "B"]);
    let proposal = detect(&request(&tree, &["fits"], &["cal"], &[]), &advanced());
    let notes = &proposal.sessions[0].calibration_notes;
    assert_eq!(
        notes,
        &vec![
            CalibrationNote::NoDark {
                exposure_seconds: 32.0,
                count: 3,
                found_seconds: vec![30.0],
            },
            CalibrationNote::NoFlat {
                filter: Some("V".to_string())
            },
        ]
    );
    assert!(notes.iter().all(CalibrationNote::is_warning));
    assert_eq!(
        notes[0].sentence(),
        "No dark of 32 s for these frames (3 darks of 30 s not attached)."
    );
    assert_eq!(notes[1].sentence(), "No flat for the V filter.");
    // They stay aside, as before: the notes say why.
    assert_eq!(proposal.set_aside.calibration_unattached.len(), 12);
}

#[test]
fn no_dark_and_no_flat_at_all_are_information() {
    let tree = Tree::new("notes-information");
    v_night(&tree, &[], &[]);
    let proposal = detect(&request(&tree, &["fits"], &[], &[]), &advanced());
    let notes = &proposal.sessions[0].calibration_notes;
    assert_eq!(
        notes,
        &vec![
            CalibrationNote::NoDarkAtAll {
                exposure_seconds: 32.0
            },
            CalibrationNote::NoFlatAtAll {
                filter: Some("V".to_string())
            },
        ]
    );
    assert!(notes.iter().all(|note| !note.is_warning()));
}

#[test]
fn a_night_with_its_darks_and_flats_has_no_note() {
    let tree = Tree::new("notes-none");
    v_night(&tree, &[32.0], &["V"]);
    let proposal = detect(&request(&tree, &["fits"], &["cal"], &[]), &advanced());
    assert!(
        proposal.sessions[0].calibration_notes.is_empty(),
        "{:?}",
        proposal.sessions[0].calibration_notes
    );
    // And the darks of the calibration folder of another camera are no dark
    // of these poses: they do not turn the information into a warning.
    let tree = Tree::new("notes-other-camera");
    v_night(&tree, &[], &[]);
    for i in 0..2 {
        tree.frame(
            &format!("cal/darks/other-{i}.fit"),
            &with(
                calibration("Dark", unix("2026-08-11T08:00:00Z"), 32.0, -10.0),
                "INSTRUME",
                &quoted("QHY268M"),
            ),
        );
    }
    let proposal = detect(&request(&tree, &["fits"], &["cal"], &[]), &advanced());
    assert!(matches!(
        proposal.sessions[0].calibration_notes[0],
        CalibrationNote::NoDarkAtAll { .. }
    ));
}

#[test]
fn videos_raw_files_and_the_text_files_of_a_capture_are_counted_and_not_read() {
    let tree = Tree::new("unread-files");
    tree.frame(
        "folder/l0.fit",
        &light(Some("IC 1396"), unix("2026-08-11T21:10:00Z"), 60.0, None),
    );
    tree.write("folder/Moon/2026-01-01-Copernicus.avi", b"video");
    tree.write(
        "folder/Moon/2026-01-01-Copernicus.avi.txt",
        b"[ZWO ASI294MC Pro]\nExposure = 0.02 Sec\n",
    );
    tree.write("folder/Moon/Tycho.SER", b"video");
    tree.write("folder/Moon/Tycho.txt", b"[ZWO ASI294MC Pro]\n");
    tree.write("folder/Moon/notes.txt", b"nothing about a video");
    for name in [
        "a.cr2", "b.NEF", "c.arw", "d.dng", "e.raf", "f.pef", "g.CR3",
    ] {
        tree.write(&format!("folder/raw/{name}"), b"raw");
    }
    let proposal = detect(&request(&tree, &["folder"], &[], &[]), &advanced());
    let aside = &proposal.set_aside;
    assert_eq!(aside.videos.len(), 2);
    assert_eq!(aside.video_captures.len(), 2, "{:?}", aside.video_captures);
    assert_eq!(aside.raw_files.len(), 7);
    // The unrelated text file stays an « other file »; the captures do not.
    assert_eq!(aside.other_files.len(), 1);
    assert!(aside.other_files[0].ends_with("notes.txt"));
    // Nothing of it is read: one FITS and one text file.
    assert_eq!(proposal.reading.files_read, 2);
    assert!(!proposal.nothing_readable());
    let notes = proposal.unread_note_sentences();
    assert_eq!(
        notes
            .iter()
            .map(|n| (n.warning, n.text.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (false, "2 videos not processed, with their 2 capture files"),
            (false, "7 RAW files not read"),
        ]
    );
}

#[test]
fn a_folder_where_nothing_is_readable_says_so_only_when_something_was_not_read() {
    let tree = Tree::new("nothing-readable");
    for i in 0..7 {
        tree.write(&format!("moon/{i}.cr2"), b"raw");
    }
    tree.write("moon/a.avi", b"video");
    tree.write("moon/b.avi", b"video");
    let proposal = detect(&request(&tree, &["moon"], &[], &[]), &advanced());
    assert_eq!(proposal.reading.files_read, 0);
    assert!(proposal.nothing_readable());
    let notes = proposal.unread_note_sentences();
    assert_eq!(notes.len(), 1);
    assert!(notes[0].warning);
    assert_eq!(
        notes[0].text,
        "No readable frame in this folder. Astro Sync reads FITS and XISF frames: 7 RAW files are not read here, 2 videos are not processed here."
    );
    // A real empty folder says nothing more than before.
    fs::create_dir_all(tree.path("empty")).unwrap();
    let proposal = detect(&request(&tree, &["empty"], &[], &[]), &advanced());
    assert!(!proposal.nothing_readable());
    assert!(proposal.unread_note_sentences().is_empty());
}

#[test]
fn a_smart_telescope_that_calibrates_on_board_is_not_told_it_lacks_darks_or_flats() {
    let tree = Tree::new("notes-seestar");
    let start = unix("2026-10-10T20:30:00Z");
    for i in 0..4 {
        tree.frame(
            &format!("fits/lights/Light_{i}.fit"),
            &with(
                with(
                    light(Some("M45"), start + f64::from(i) * 12.0, 10.0, None),
                    "CREATOR",
                    &quoted("ZWO Seestar S50 Pro"),
                ),
                "FILTER",
                &quoted("IRCUT"),
            ),
        );
    }
    let proposal = detect(&request(&tree, &["fits"], &[], &[]), &advanced());
    assert!(
        proposal.sessions[0].calibration_notes.is_empty(),
        "{:?}",
        proposal.sessions[0].calibration_notes
    );
    // The same poses of another camera are told.
    let tree = Tree::new("notes-no-seestar");
    v_night(&tree, &[], &[]);
    let proposal = detect(&request(&tree, &["fits"], &[], &[]), &advanced());
    assert_eq!(proposal.sessions[0].calibration_notes.len(), 2);
}

/* ------------------------- Camera RAW files ------------------- */

use crate::raw_header::fixture::{Shot, write_raw};

fn tokyo(with_offset: bool) -> UserData {
    UserData {
        places: vec![KnownPlace {
            utc_offset_minutes: with_offset.then_some(540),
            ..place("tokyo", 35.68, 139.69)
        }],
        default_place_id: Some("tokyo".to_string()),
        ..advanced()
    }
}

fn raw_shot(tree: &Tree, relative: &str, taken: &'static str, offset: Option<&'static str>) {
    let mut shot = Shot::new(taken, 30, 800);
    shot.offset = offset;
    write_raw(&tree.path(relative), &shot);
}

#[test]
fn camera_raw_files_are_poses_and_a_session_is_proposed_from_them() {
    let tree = Tree::new("detect-raw-session");
    for index in 0..4 {
        raw_shot(
            &tree,
            &format!("raw/night/lights/DSC_{index:04}.NEF"),
            "2026:10:07 23:15:00",
            Some("+09:00"),
        );
    }
    for index in 0..2 {
        raw_shot(
            &tree,
            &format!("cal/darks/Dark_{index}.NEF"),
            "2026:10:08 05:00:00",
            Some("+09:00"),
        );
    }
    let proposal = detect(&request(&tree, &["raw"], &["cal"], &[]), &tokyo(true));
    assert_eq!(proposal.reading.lights, 4);
    assert_eq!(proposal.reading.files_read, 6);
    // Read, not set aside: no "RAW not read".
    assert!(proposal.set_aside.raw_files.is_empty());
    assert!(proposal.unread_note_sentences().is_empty());
    assert!(!proposal.nothing_readable());
    assert_eq!(proposal.sessions.len(), 1);
    let session = &proposal.sessions[0];
    assert_eq!(session.evening_date, "2026-10-07");
    assert_eq!(session.targets[0].frames, 4);
    // The target's exposure is the total: four poses of 30 s.
    assert_eq!(session.targets[0].exposure_seconds, 120.0);
    assert_eq!(session.targets[0].instrument, "NIKON D3300");
    // The darks of the same exposure and ISO are attached.
    assert!(!session.calibration.is_empty(), "{:?}", session.calibration);
    assert_eq!(
        proposal
            .calibration_sets
            .iter()
            .map(|set| set.frames)
            .sum::<usize>(),
        2
    );
    assert!(proposal.set_aside.calibration_unattached.is_empty());
}

#[test]
fn a_camera_clock_without_a_zone_is_one_night_whatever_the_side_of_midnight() {
    // 23:30 on the 7th and 03:00 on the 8th, at a place east of Greenwich:
    // read as UTC, the second would belong to the next evening.
    for with_offset in [true, false] {
        let tree = Tree::new("detect-raw-wall-clock");
        for (index, taken) in [
            "2026:10:07 23:30:00",
            "2026:10:08 00:30:00",
            "2026:10:08 03:00:00",
        ]
        .into_iter()
        .enumerate()
        {
            raw_shot(&tree, &format!("raw/lights/DSC_{index}.NEF"), taken, None);
        }
        let proposal = detect(&request(&tree, &["raw"], &[], &[]), &tokyo(with_offset));
        assert_eq!(proposal.sessions.len(), 1, "with_offset={with_offset}");
        assert_eq!(proposal.sessions[0].evening_date, "2026-10-07");
        assert_eq!(proposal.sessions[0].targets[0].frames, 3);
        // Brought to UTC by the place: 23:30 at UTC+9 is 14:30.
        if with_offset {
            assert!(
                proposal.sessions[0].start.starts_with("2026-10-07T14:30"),
                "{}",
                proposal.sessions[0].start
            );
        }
        // The camera clock is consistent with the place: no clock finding.
        assert_eq!(proposal.sessions[0].shift_hours, 0);
    }
}

#[test]
fn a_raw_file_nothing_can_read_stays_an_information_line_and_is_not_counted_as_read() {
    let tree = Tree::new("detect-raw-odd");
    raw_shot(
        &tree,
        "raw/lights/ok.NEF",
        "2026:10:07 23:15:00",
        Some("+09:00"),
    );
    tree.write("raw/lights/odd.CR3", b"not a raw file");
    let proposal = detect(&request(&tree, &["raw"], &[], &[]), &tokyo(true));
    assert_eq!(proposal.reading.files_read, 1);
    assert_eq!(proposal.set_aside.raw_files.len(), 1);
    assert!(proposal.set_aside.raw_files[0].ends_with("odd.CR3"));
    assert!(!proposal.nothing_readable());
    let notes = proposal.unread_note_sentences();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].text, "1 RAW file not read");
}

/* ---------------------- The band read from the names ---------------------- */

/// Three lights of one night (no `FILTER`) in `folder`, from `start`.
fn lights_in(tree: &Tree, folder: &str, start: &str, names: &str) {
    let start = unix(start);
    for i in 0..3 {
        tree.frame(
            &format!("{folder}/{names}{i}.fit"),
            &light(
                Some("NGC 7000"),
                start + f64::from(i) * 190.0,
                180.0,
                Some((314.82, 44.52)),
            ),
        );
    }
}

fn guess_of<'a>(proposal: &'a Proposal, evening: &str) -> Option<&'a band::BandGuess> {
    proposal
        .sessions
        .iter()
        .find(|s| s.evening_date == evening)
        .unwrap_or_else(|| panic!("no night {evening}"))
        .filter_guess
        .as_ref()
}

#[test]
fn a_night_folder_that_names_its_band_preselects_it() {
    let tree = Tree::new("detect-band-folders");
    // The three nights of a real archive, shaped as it is (names only).
    lights_in(
        &tree,
        "fits/2026-08-09_10_180s_dualband/lights",
        "2026-08-09T21:00:00Z",
        "a",
    );
    lights_in(
        &tree,
        "fits/2026-08-10_11_90s_RGB/lights",
        "2026-08-10T21:00:00Z",
        "a",
    );
    lights_in(
        &tree,
        "fits/2026-08-12_120s_UVIR_cut/lights",
        "2026-08-12T21:00:00Z",
        "a",
    );
    // A fourth night whose names say nothing: the question stays.
    lights_in(&tree, "fits/Voile/lights", "2026-08-14T21:00:00Z", "a");
    let proposal = detect(&request(&tree, &["fits"], &[], &[]), &advanced());
    assert!(proposal.sessions.iter().all(|s| s.filter_ask));

    let first = guess_of(&proposal, "2026-08-09").unwrap();
    assert_eq!(first.band, band::NameBand::Dualband);
    assert_eq!(first.origin, band::GuessOrigin::Folder);
    assert_eq!(first.source, "180s_dualband");
    let second = guess_of(&proposal, "2026-08-10").unwrap();
    assert_eq!(second.band, band::NameBand::Broadband);
    assert_eq!(second.source, "90s_RGB");
    let third = guess_of(&proposal, "2026-08-12").unwrap();
    assert_eq!(third.band, band::NameBand::Broadband);
    assert_eq!(third.source, "120s_UVIR_cut");
    assert!(
        guess_of(&proposal, "2026-08-14").is_none(),
        "nothing says it"
    );
    // The guess travels in the proposal, read back as it was written.
    let json = serde_json::to_string(&proposal).unwrap();
    assert!(json.contains("\"filterGuess\""));
    assert_eq!(serde_json::from_str::<Proposal>(&json).unwrap(), proposal);
}

#[test]
fn the_same_folder_given_to_the_three_locations_still_preselects_the_bands() {
    let tree = Tree::new("detect-band-same-folder");
    // Three nights of one target shaped as a real archive (names only): the
    // band in the night's folder, darks reached by a link to a library
    // outside the folder, a log beside the lights.
    let nights = [
        ("2026-08-07_08_180s_dualband", "2026-08-07T21:00:00Z"),
        ("2026-08-10_11_90s_RGB", "2026-08-10T21:00:00Z"),
        ("2026-08-12_13_180s_dualband", "2026-08-12T21:00:00Z"),
    ];
    let t = unix("2026-08-11T10:00:00Z");
    tree.frame(
        "DARK/2026-08-11_180s/frames/d1.fit",
        &calibration("Dark", t, 180.0, -10.0),
    );
    for (night, start) in nights {
        lights_in(&tree, &format!("Voile/{night}/lights"), start, "Light_");
        tree.write(
            &format!("Voile/{night}/logs/Autorun_Log.txt"),
            b"2026/08/07 21:00:00 [Autorun|Begin] Voile\n",
        );
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            tree.path("DARK/2026-08-11_180s/frames"),
            tree.path(&format!("Voile/{night}/dark")),
        )
        .unwrap();
    }
    let expected = [
        ("2026-08-07", band::NameBand::Dualband),
        ("2026-08-10", band::NameBand::Broadband),
        ("2026-08-12", band::NameBand::Dualband),
    ];
    for (fits, calibration, logs) in [
        (&["Voile"][..], &["Voile"][..], &["Voile"][..]),
        (&["Voile"][..], &[][..], &[][..]),
        (&["Voile"][..], &["Voile", "DARK"][..], &[][..]),
    ] {
        let proposal = detect(&request(&tree, fits, calibration, logs), &advanced());
        for (evening, band) in expected {
            let guess = guess_of(&proposal, evening)
                .unwrap_or_else(|| panic!("no band for {evening} with {calibration:?}"));
            assert_eq!(guess.band, band, "{evening} with {calibration:?}");
            assert_eq!(guess.origin, band::GuessOrigin::Folder);
        }
        let reading = read_locations(
            &request(&tree, fits, calibration, logs),
            &DetectOptions::default(),
        )
        .unwrap();
        let lights: Vec<_> = reading
            .frames
            .iter()
            .filter(|f| f.kind == Some(FrameType::Light))
            .collect();
        assert_eq!(lights.len(), 9, "with {calibration:?}");
        assert!(lights.iter().all(|f| !f.in_calibration_location));
        // The dark is read once, a dark, wherever it was met.
        #[cfg(unix)]
        {
            let darks: Vec<_> = reading
                .frames
                .iter()
                .filter(|f| f.path.ends_with("d1.fit"))
                .collect();
            assert_eq!(darks.len(), 1);
            assert_eq!(darks[0].kind, Some(FrameType::Dark));
            assert_eq!(
                darks[0].in_calibration_location,
                calibration.contains(&"DARK")
            );
        }
    }
}

#[test]
fn the_file_names_then_a_session_note_say_the_band_when_the_folders_do_not() {
    let tree = Tree::new("detect-band-files-notes");
    lights_in(
        &tree,
        "fits/night-a/lights",
        "2026-08-09T21:00:00Z",
        "Light_Ha-OIII_",
    );
    lights_in(
        &tree,
        "fits/night-b/lights",
        "2026-08-10T21:00:00Z",
        "Light_",
    );
    tree.write(
        "fits/night-b/SESSION.md",
        "# SH2-190 — 10/11 août 2026 — dual-band 180 s\n\n- 100 lights\n".as_bytes(),
    );
    // A README at the root of the location is the archive's, not a night's.
    lights_in(
        &tree,
        "fits/night-c/lights",
        "2026-08-12T21:00:00Z",
        "Light_",
    );
    tree.write("fits/README.md", b"# My RGB archive\n");
    // A note above the frames of a night with no folder word counts too.
    lights_in(
        &tree,
        "fits/night-d/lights",
        "2026-08-14T21:00:00Z",
        "Light_",
    );
    tree.write("fits/night-d/README.md", b"# UV/IR-cut night\n");
    let proposal = detect(&request(&tree, &["fits"], &[], &[]), &advanced());

    let a = guess_of(&proposal, "2026-08-09").unwrap();
    assert_eq!(
        (a.band, a.origin),
        (band::NameBand::Dualband, band::GuessOrigin::File)
    );
    assert!(a.source.starts_with("Light_Ha-OIII_"));
    let b = guess_of(&proposal, "2026-08-10").unwrap();
    assert_eq!(
        (b.band, b.origin),
        (band::NameBand::Dualband, band::GuessOrigin::Note)
    );
    assert_eq!(b.note_file.as_deref(), Some("SESSION.md"));
    assert_eq!(b.source, "SH2-190 — 10/11 août 2026 — dual-band 180 s");
    assert!(
        guess_of(&proposal, "2026-08-12").is_none(),
        "the root's README is not a night's note"
    );
    let d = guess_of(&proposal, "2026-08-14").unwrap();
    assert_eq!(d.band, band::NameBand::Broadband);
    assert_eq!(d.note_file.as_deref(), Some("README.md"));
}

#[test]
fn a_header_filter_or_a_session_that_declares_one_leaves_no_guess() {
    let tree = Tree::new("detect-band-no-guess");
    lights_in(
        &tree,
        "fits/2026-08-17_dualband/lights",
        "2026-08-17T21:00:00Z",
        "a",
    );
    // The second night's frames name their filter: nothing to ask or guess.
    let start = unix("2026-08-19T21:00:00Z");
    for i in 0..3 {
        tree.frame(
            &format!("fits/2026-08-19_RGB/lights/b{i}.fit"),
            &with(
                light(Some("NGC 7000"), start + f64::from(i) * 190.0, 180.0, None),
                "FILTER",
                "'UV/IR Cut'",
            ),
        );
    }
    let mut user = advanced();
    let proposal = detect(&request(&tree, &["fits"], &[], &[]), &user);
    assert!(guess_of(&proposal, "2026-08-17").is_some());
    assert!(!proposal.sessions[1].filter_ask);
    assert!(guess_of(&proposal, "2026-08-19").is_none());

    user.sessions = vec![KnownSession {
        id: "sess-17".into(),
        evening_date: "2026-08-17".into(),
        targets: vec![KnownSlot {
            designation: Some("NGC 7000".into()),
            filter: Some("Dual-band".into()),
            ..KnownSlot::default()
        }],
        ..KnownSession::default()
    }];
    let proposal = detect(&request(&tree, &["fits"], &[], &[]), &user);
    assert!(!proposal.sessions[0].filter_ask);
    assert!(
        guess_of(&proposal, "2026-08-17").is_none(),
        "the session declares it: no guess to show"
    );
}

/// The filters a night's targets are created with: the headers', else the
/// one the folder names says.
fn filters_of(proposal: &Proposal, evening: &str) -> Vec<String> {
    proposal
        .sessions
        .iter()
        .find(|s| s.evening_date == evening)
        .unwrap_or_else(|| panic!("no night {evening}"))
        .targets
        .iter()
        .flat_map(|t| {
            let mut filters = t.filters.clone();
            filters.extend(t.filter_hint.clone());
            filters
        })
        .collect()
}

#[test]
fn a_light_without_a_filter_card_takes_the_filter_its_folder_names() {
    let tree = Tree::new("detect-filter-from-names");
    lights_in(
        &tree,
        "fits/2026-08-09_10_180s_dualband/lights",
        "2026-08-09T21:00:00Z",
        "a",
    );
    lights_in(
        &tree,
        "fits/2026-08-10_11_90s_RGB/lights",
        "2026-08-10T21:00:00Z",
        "a",
    );
    lights_in(
        &tree,
        "fits/2026-08-12_120s_UVIR_cut/lights",
        "2026-08-12T21:00:00Z",
        "a",
    );
    lights_in(&tree, "fits/Voile/lights", "2026-08-14T21:00:00Z", "a");
    let proposal = detect(&request(&tree, &["fits"], &[], &[]), &advanced());
    assert_eq!(filters_of(&proposal, "2026-08-09"), vec!["Dualband"]);
    assert_eq!(filters_of(&proposal, "2026-08-10"), vec!["RGB"]);
    assert_eq!(filters_of(&proposal, "2026-08-12"), vec!["UV/IR cut"]);
    assert!(
        filters_of(&proposal, "2026-08-14").is_empty(),
        "nothing says it: no filter is made up"
    );
    // A name is a preselection, not an answer: the question is still asked.
    assert!(proposal.sessions.iter().all(|s| s.filter_ask));
    // The filter reaches the service's body, as the target's filter.
    let body = create::bulk_body(&create::BulkCreateRequest::as_proposed(proposal)).body;
    let filters: Vec<(String, Option<String>)> = body["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|session| {
            (
                session["eveningDate"].as_str().unwrap().to_string(),
                session["targets"][0]["filter"].as_str().map(str::to_string),
            )
        })
        .collect();
    assert!(filters.contains(&("2026-08-09".into(), Some("Dualband".into()))));
    assert!(filters.contains(&("2026-08-12".into(), Some("UV/IR cut".into()))));
    assert!(filters.contains(&("2026-08-14".into(), None)));
}

#[test]
fn the_header_filter_wins_over_the_names_and_the_option_over_both() {
    let tree = Tree::new("detect-filter-precedence");
    let start = unix("2026-08-19T21:00:00Z");
    for i in 0..3 {
        tree.frame(
            &format!("fits/2026-08-19_RGB/lights/b{i}.fit"),
            &with(
                light(Some("NGC 7000"), start + f64::from(i) * 190.0, 180.0, None),
                "FILTER",
                "'UV/IR Cut'",
            ),
        );
    }
    lights_in(
        &tree,
        "fits/2026-08-17_dualband/lights",
        "2026-08-17T21:00:00Z",
        "a",
    );
    let mut asked = request(&tree, &["fits"], &[], &[]);
    let proposal = detect(&asked, &advanced());
    assert_eq!(filters_of(&proposal, "2026-08-19"), vec!["UV/IR Cut"]);
    assert_eq!(filters_of(&proposal, "2026-08-17"), vec!["Dualband"]);

    asked.filter = Some("  L-eNhance ".to_string());
    let proposal = detect(&asked, &advanced());
    assert_eq!(filters_of(&proposal, "2026-08-19"), vec!["L-eNhance"]);
    assert_eq!(filters_of(&proposal, "2026-08-17"), vec!["L-eNhance"]);

    asked.filter = Some("   ".to_string());
    let proposal = detect(&asked, &advanced());
    assert_eq!(filters_of(&proposal, "2026-08-17"), vec!["Dualband"]);
}

/* ------------------- A folder of logs of its own (links) ------------------ */

#[cfg(unix)]
#[test]
fn a_folder_of_logs_of_its_own_is_walked_through_sub_folders_and_links() {
    use std::os::unix::fs::symlink;
    let tree = Tree::new("detect-logs-own");
    two_nights(&tree);
    // Shaped like an archive: `LOGS/<night>` holds the logs of a night, and
    // a night's folder reaches them through a `logs` link.
    tree.write(
        "store/LOGS/2026-08-17_18_NGC7000_MuCephei/PHD2_GuideLog_2026-08-17_225500.txt",
        phd2_log("2026-08-17 22:55:00", 7200).as_bytes(),
    );
    tree.write(
        "store/LOGS/2026-08-19_20_MuCephei/PHD2_GuideLog_2026-08-19_232500.txt",
        phd2_log("2026-08-19 23:25:00", 7200).as_bytes(),
    );
    // Not a night of these frames.
    tree.write(
        "store/LOGS/2026-08-07_08_Voile/PHD2_GuideLog_2026-08-07_223803.txt",
        phd2_log("2026-08-07 22:38:00", 7200).as_bytes(),
    );
    let attached_to = |proposal: &Proposal, evening: &str| -> Vec<String> {
        let night = proposal
            .sessions
            .iter()
            .find(|s| s.evening_date == evening)
            .unwrap();
        night
            .logs
            .iter()
            .map(|a| {
                let log = proposal.logs.iter().find(|l| l.id == a.log_id).unwrap();
                log.path.rsplit('/').next().unwrap().to_string()
            })
            .collect()
    };

    // 1. The logs folder given alone: the sub-folders are walked.
    let proposal = detect(
        &request(&tree, &["fits"], &[], &["store/LOGS"]),
        &advanced(),
    );
    assert_eq!(proposal.logs.len(), 3, "{:?}", proposal.logs);
    assert_eq!(
        attached_to(&proposal, "2026-08-17"),
        ["PHD2_GuideLog_2026-08-17_225500.txt"]
    );
    assert_eq!(
        attached_to(&proposal, "2026-08-19"),
        ["PHD2_GuideLog_2026-08-19_232500.txt"]
    );
    assert_eq!(proposal.set_aside.logs_unattached.len(), 1);

    // 2. A logs folder made of links to the nights' folders, itself reached
    //    through a link.
    fs::create_dir_all(tree.path("links")).unwrap();
    for night in [
        "2026-08-17_18_NGC7000_MuCephei",
        "2026-08-19_20_MuCephei",
        "2026-08-07_08_Voile",
    ] {
        symlink(
            tree.path(&format!("store/LOGS/{night}")),
            tree.path(&format!("links/{night}")),
        )
        .unwrap();
    }
    symlink(tree.path("links"), tree.path("logs_root")).unwrap();
    let proposal = detect(&request(&tree, &["fits"], &[], &["logs_root"]), &advanced());
    assert_eq!(proposal.logs.len(), 3);
    assert_eq!(attached_to(&proposal, "2026-08-17").len(), 1);
    assert_eq!(attached_to(&proposal, "2026-08-19").len(), 1);

    // 3. Both at once: the nights' `logs` links and the folder of logs hold
    //    the same files, each is read once.
    symlink(
        tree.path("store/LOGS/2026-08-17_18_NGC7000_MuCephei"),
        tree.path("fits/Mu_Cephei/night1/logs"),
    )
    .unwrap();
    symlink(
        tree.path("store/LOGS/2026-08-19_20_MuCephei"),
        tree.path("fits/Mu_Cephei/night2/logs"),
    )
    .unwrap();
    let proposal = detect(
        &request(&tree, &["fits"], &[], &["store/LOGS"]),
        &advanced(),
    );
    assert_eq!(proposal.logs.len(), 3, "each log once");
    assert_eq!(attached_to(&proposal, "2026-08-17").len(), 1);
    assert_eq!(attached_to(&proposal, "2026-08-19").len(), 1);

    // 4. The tick "sub-folders" is one for the three locations: off, a
    //    logs folder that keeps its logs in per-night sub-folders gives none.
    let mut flat = request(
        &tree,
        &["fits/Mu_Cephei/night1/lights"],
        &[],
        &["store/LOGS"],
    );
    flat.subfolders = false;
    let reading = read_locations(&flat, &DetectOptions::default()).unwrap();
    assert!(reading.logs.is_empty());
}

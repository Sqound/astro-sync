//! The search, the night's list, the header of a FITS, the index
//! kept, a new target -- against a fake service and invented headers (no
//! real file, place or person).

use super::*;
use crate::analysis::fits::parse_primary_header;
use crate::test_support::{TempDir, fits_bytes};
use std::cell::RefCell;

/* ------------------------------ The fake service ------------------------- */

/// Three rows of the catalogue in the compact notation (invented values
/// close to the sky's, enough to place them).
fn catalogue() -> Vec<Value> {
    vec![
        json!([
            "t-m31",
            "M31",
            "Galaxie d'Andromède",
            "M31",
            ["NGC 224"],
            0.7123,
            41.2689,
            [190.0, 60.0],
            3.4,
            0,
            0,
            "190 × 60′",
            "/img/m31.jpg",
            null
        ]),
        json!([
            "t-ngc7000",
            "NGC 7000",
            "Nébuleuse de l'Amérique du Nord",
            "NGC 7000",
            [],
            20.9784,
            44.33,
            [120.0, 100.0],
            4.0,
            1,
            1,
            "120 × 100′",
            null,
            "/img/ngc7000-drawn.svg"
        ]),
        json!([
            "t-ic5070",
            "IC 5070",
            "Nébuleuse du Pélican",
            "IC 5070",
            [],
            20.8508,
            44.35,
            [60.0, 50.0],
            8.0,
            1,
            1,
            "60 × 50′",
            "/img/ic5070.jpg",
            null
        ]),
    ]
}

fn index_page(rows: &[Value], total: usize, revision: &str) -> Value {
    json!({
        "v": 1,
        "revision": revision,
        "total": total,
        "offset": 0,
        "limit": 8,
        "sort": "planner",
        "types": [["Galaxie", "Galaxie"], ["Nébuleuse en émission", "Nébuleuse en émission"]],
        "constellations": [["And", "Andromède"], ["Cyg", "Cygne"]],
        "fields": ["id", "designation", "name", "title", "aliases", "ra", "dec", "sizeArcmin", "magnitude", "type", "constellation", "sizeLabel", "photoUrl", "visualUrl", "aFutureColumn"],
        "rows": rows,
    })
}

#[derive(Default)]
struct FakeService {
    calls: RefCell<Vec<String>>,
    night: RefCell<Vec<String>>,
    revision: RefCell<String>,
}

impl FakeService {
    fn with_night(ids: &[&str]) -> Self {
        let service = Self::default();
        *service.night.borrow_mut() = ids.iter().map(|s| (*s).to_string()).collect();
        *service.revision.borrow_mut() = "rev-1".to_string();
        service
    }
    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}

impl CatalogueTransport for FakeService {
    fn target_index(
        &self,
        _: &str,
        _: Option<&str>,
        query: &[(&'static str, String)],
    ) -> Result<Value, GuiBridgeError> {
        let get = |key: &str| {
            query
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        self.calls.borrow_mut().push(format!(
            "INDEX q={} sort={} offset={} limit={}",
            get("q"),
            get("sort"),
            get("offset"),
            get("limit")
        ));
        let q = get("q").to_lowercase().replace(' ', "");
        let rows: Vec<Value> = catalogue()
            .into_iter()
            .filter(|row| {
                q.is_empty() || row.to_string().to_lowercase().replace(' ', "").contains(&q)
            })
            .collect();
        let total = rows.len();
        let offset: usize = get("offset").parse().unwrap_or(0);
        let limit: usize = get("limit").parse().unwrap_or(8);
        let page: Vec<Value> = rows.into_iter().skip(offset).take(limit).collect();
        Ok(index_page(&page, total, &self.revision.borrow()))
    }

    fn create_personal_target(
        &self,
        _: &str,
        _: &str,
        body: &Value,
    ) -> Result<Value, GuiBridgeError> {
        self.calls
            .borrow_mut()
            .push(format!("POST personal {body}"));
        Ok(json!({ "target": { "id": "p-comet", "source": "personal", "name": body["name"] } }))
    }
}

impl NightServiceTransport for FakeService {
    fn session_detail(&self, _: &str, _: &str, id: &str) -> Result<Value, GuiBridgeError> {
        self.calls.borrow_mut().push(format!("GET {id}"));
        Ok(json!({ "night": { "targetIds": { "value": *self.night.borrow(), "source": "plan" } } }))
    }
    fn create_session(&self, _: &str, _: &str, _: &Value) -> Result<Value, GuiBridgeError> {
        unreachable!()
    }
    fn patch_night(
        &self,
        _: &str,
        _: &str,
        id: &str,
        changes: &Value,
    ) -> Result<Value, GuiBridgeError> {
        self.calls
            .borrow_mut()
            .push(format!("PATCH {id} {changes}"));
        let ids: Vec<String> = serde_json::from_value(changes["targetIds"].clone()).unwrap();
        *self.night.borrow_mut() = ids.clone();
        Ok(json!({ "night": { "targetIds": { "value": ids, "source": "night" } } }))
    }
    fn finish_session(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &Value,
    ) -> Result<Value, GuiBridgeError> {
        unreachable!()
    }
}

/* --------------------------------- Search -------------------------------- */

#[test]
fn a_designation_is_written_the_catalogue_s_way_and_two_characters_are_needed() {
    assert_eq!(search_query(" m 31 ").as_deref(), Some("M31"));
    assert_eq!(search_query("m031").as_deref(), Some("M31"));
    assert_eq!(search_query("ngc7000").as_deref(), Some("NGC 7000"));
    assert_eq!(search_query("NGC-7000").as_deref(), Some("NGC 7000"));
    assert_eq!(search_query("ic 1396 a").as_deref(), Some("IC 1396A"));
    assert_eq!(search_query("sh2 155").as_deref(), Some("Sh2-155"));
    assert_eq!(search_query("ldn1622").as_deref(), Some("LDN 1622"));
    assert_eq!(search_query("abell39").as_deref(), Some("Abell 39"));
    assert_eq!(search_query("b33").as_deref(), Some("B33"));
    assert_eq!(
        search_query("  nébuleuse   de  Californie ").as_deref(),
        Some("nébuleuse de Californie")
    );
    assert_eq!(search_query("andromeda").as_deref(), Some("andromeda"));
    assert_eq!(search_query("m"), None);
    assert_eq!(search_query("  "), None);
}

#[test]
fn the_index_page_is_read_by_its_fields_with_the_drawn_view_when_there_is_no_photo() {
    let page = parse_index_page(&index_page(&catalogue(), 3, "rev-1")).unwrap();
    assert_eq!(page.total, 3);
    assert_eq!(page.revision.as_deref(), Some("rev-1"));
    let ngc = &page.targets[1];
    assert_eq!(ngc.designation, "NGC 7000");
    assert_eq!(ngc.name.as_deref(), Some("Nébuleuse de l'Amérique du Nord"));
    assert_eq!(ngc.type_label.as_deref(), Some("Nébuleuse en émission"));
    assert_eq!(ngc.constellation_label.as_deref(), Some("Cygne"));
    assert_eq!(ngc.size_label.as_deref(), Some("120 × 100′"));
    assert_eq!(ngc.size_arcmin, Some(120.0));
    assert_eq!(ngc.image_path.as_deref(), Some("/img/ngc7000-drawn.svg"));
    assert_eq!(page.targets[0].aliases, ["NGC 224"]);
    assert_eq!(page.targets[0].image_path.as_deref(), Some("/img/m31.jpg"));
    assert!(parse_index_page(&json!({ "rows": [] })).is_err());
}

#[test]
fn the_search_sends_the_normalised_text_by_the_planner_s_order_and_asks_nothing_below_two_characters()
 {
    let service = FakeService::with_night(&[]);
    let found = search_targets(&service, "s", None, "ngc7000", "fr", 0, 8).unwrap();
    assert_eq!(found.query, "NGC 7000");
    assert_eq!(found.total, 1);
    assert_eq!(found.targets[0].id, "t-ngc7000");
    assert_eq!(
        service.calls(),
        ["INDEX q=NGC 7000 sort=planner offset=0 limit=8"]
    );
    let nothing = search_targets(&service, "s", None, "n", "fr", 0, 8).unwrap();
    assert_eq!(nothing.total, 0);
    assert_eq!(service.calls().len(), 1);
    // "Show more targets" stops at 24.
    search_targets(&service, "s", None, "neb", "fr", 99, 99).unwrap();
    assert_eq!(
        service.calls()[1],
        "INDEX q=neb sort=planner offset=24 limit=24"
    );
}

/* ----------------------------- The night's list --------------------------- */

#[test]
fn adding_keeps_the_night_s_targets_and_a_repeat_or_a_target_not_there_sends_nothing() {
    let service = FakeService::with_night(&["t-ic1396"]);
    let ids = change_night_targets(
        &service,
        "s",
        "t",
        "night-7",
        &NightTargetChange::Add("t-ngc7000".to_string()),
    )
    .unwrap();
    assert_eq!(ids, ["t-ic1396", "t-ngc7000"]);
    assert_eq!(
        service.calls(),
        [
            "GET night-7",
            r#"PATCH night-7 {"targetIds":["t-ic1396","t-ngc7000"]}"#
        ]
    );
    // Added again: nothing is sent.
    change_night_targets(
        &service,
        "s",
        "t",
        "night-7",
        &NightTargetChange::Add("t-ngc7000".to_string()),
    )
    .unwrap();
    assert_eq!(service.calls().len(), 3);
    // Taken back (just added): the others stay.
    let ids = change_night_targets(
        &service,
        "s",
        "t",
        "night-7",
        &NightTargetChange::UndoAdd("t-ngc7000".to_string()),
    )
    .unwrap();
    assert_eq!(ids, ["t-ic1396"]);
    assert_eq!(
        service.calls()[4],
        r#"PATCH night-7 {"targetIds":["t-ic1396"]}"#
    );
    // A target not in the night: nothing to take back, nothing sent.
    change_night_targets(
        &service,
        "s",
        "t",
        "night-7",
        &NightTargetChange::UndoAdd("t-m31".to_string()),
    )
    .unwrap();
    assert_eq!(service.calls().len(), 6);
}

#[test]
fn the_change_is_read_from_the_window_s_json() {
    let add: NightTargetChange =
        serde_json::from_value(json!({ "kind": "add", "targetId": "t-m31" })).unwrap();
    assert_eq!(add, NightTargetChange::Add("t-m31".to_string()));
    let undo: NightTargetChange =
        serde_json::from_value(json!({ "kind": "undoAdd", "targetId": "t-m31" })).unwrap();
    assert_eq!(undo, NightTargetChange::UndoAdd("t-m31".to_string()));
}

/* ------------------------------ The FITS header -------------------------- */

fn header(cards: &[(&str, &str)]) -> FitsHeader {
    parse_primary_header(&fits_bytes(cards, 0)).unwrap()
}

/// Fixture 1: `OBJECT` alone, written with a space (« M 31 »).
fn object_only() -> Vec<(&'static str, &'static str)> {
    vec![
        ("IMAGETYP", "'LIGHT'"),
        ("OBJECT", "'M 31    '"),
        ("EXPTIME", "120."),
    ]
}

/// Fixture 2: `OBJCTRA`/`OBJCTDEC` only (NINA, ASIAIR), a field of
/// 2,1° × 1,6° from the scale and the size.
fn objctra_only() -> Vec<(&'static str, &'static str)> {
    vec![
        ("IMAGETYP", "'LIGHT'"),
        ("OBJCTRA", "'20 56 00'"),
        ("OBJCTDEC", "'+44 20 00'"),
        ("XPIXSZ", "3.76"),
        ("FOCALLEN", "400."),
        ("NAXIS1", "3880"),
        ("NAXIS2", "2954"),
    ]
}

/// Fixture 3: a solved image (`CRVAL1`/`CRVAL2`), its `OBJECT` and `RA`/
/// `DEC` too: the solution wins.
fn solved() -> Vec<(&'static str, &'static str)> {
    vec![
        ("OBJECT", "'NGC7000'"),
        ("CTYPE1", "'RA---TAN'"),
        ("CTYPE2", "'DEC--TAN'"),
        ("CRVAL1", "314.676"),
        ("CRVAL2", "44.33"),
        ("RA", "10.0"),
        ("DEC", "41.0"),
    ]
}

#[test]
fn the_three_fixtures_say_their_object_their_position_and_their_field() {
    let one = header_target(&header(&object_only()));
    assert_eq!(one.object.as_deref(), Some("M 31"));
    assert_eq!(one.position_source, None);
    assert_eq!(one.field_degrees, [1.0, 1.0]);
    assert!(!one.field_from_header);

    let two = header_target(&header(&objctra_only()));
    assert_eq!(two.object, None);
    assert_eq!(two.position_source, Some(PositionSource::Objctra));
    assert!((two.ra_degrees.unwrap() - 314.0).abs() < 1e-9);
    assert!((two.dec_degrees.unwrap() - (44.0 + 20.0 / 60.0)).abs() < 1e-9);
    assert!(two.field_from_header);
    assert!(
        (two.field_degrees[0] - 2.1).abs() < 0.05,
        "{:?}",
        two.field_degrees
    );
    assert!(
        (two.field_degrees[1] - 1.6).abs() < 0.05,
        "{:?}",
        two.field_degrees
    );

    let three = header_target(&header(&solved()));
    assert_eq!(three.position_source, Some(PositionSource::Crval));
    assert!((three.ra_degrees.unwrap() - 314.676).abs() < 1e-9);
}

#[test]
fn ra_dec_in_degrees_or_sexagesimal_and_a_crval_without_projection_is_ignored() {
    let nina = header_target(&header(&[("RA", "10.68"), ("DEC", "41.27")]));
    assert_eq!(nina.position_source, Some(PositionSource::RaDec));
    assert!((nina.ra_degrees.unwrap() - 10.68).abs() < 1e-9);
    let text = header_target(&header(&[("RA", "'00:42:44'"), ("DEC", "'+41:16:09'")]));
    assert!((text.ra_degrees.unwrap() - (0.0 + 42.0 / 60.0 + 44.0 / 3600.0) * 15.0).abs() < 1e-9);
    let pixel = header_target(&header(&[("CRVAL1", "1.0"), ("CRVAL2", "1.0")]));
    assert_eq!(pixel.position_source, None);
    let wrong = header_target(&header(&[
        ("OBJCTRA", "'25 00 00'"),
        ("OBJCTDEC", "'+10 00 00'"),
    ]));
    assert_eq!(wrong.position_source, None);
}

#[test]
fn the_field_holds_the_targets_whose_centre_falls_in_it_the_nearest_first() {
    let page = parse_index_page(&index_page(&catalogue(), 3, "r")).unwrap();
    let field = candidates_in_field(&page.targets, 314.0, 44.333, [2.1, 1.6]);
    let ids: Vec<&str> = field.iter().map(|c| c.target.id.as_str()).collect();
    assert_eq!(ids, ["t-ngc7000", "t-ic5070"]);
    assert!(field[0].distance_degrees < field[1].distance_degrees);
    assert!(candidates_in_field(&page.targets, 100.0, -20.0, [2.0, 2.0]).is_empty());
}

#[test]
fn the_decision_found_hesitates_candidates_unknown_or_nothing() {
    let page = parse_index_page(&index_page(&catalogue(), 3, "r")).unwrap();
    let m31 = page.targets[0].clone();
    let ngc = page.targets[1].clone();
    let at = |ra: f64, dec: f64, object: Option<&str>| HeaderTarget {
        object: object.map(str::to_string),
        ra_degrees: Some(ra),
        dec_degrees: Some(dec),
        position_source: Some(PositionSource::Objctra),
        field_degrees: [2.1, 1.6],
        field_from_header: true,
    };
    let field = candidates_in_field(&page.targets, 314.0, 44.333, [2.1, 1.6]);

    let found = decide(
        "a.fit".into(),
        at(10.68, 41.27, Some("M 31")),
        Some(m31.clone()),
        vec![],
    );
    assert_eq!(found.kind, FitsGuessKind::Found);
    assert_eq!(found.named.unwrap().id, "t-m31");

    // `OBJECT = M31`, but the position falls on NGC 7000.
    let torn = decide(
        "b.fit".into(),
        at(314.0, 44.333, Some("M31")),
        Some(m31),
        field.clone(),
    );
    assert_eq!(torn.kind, FitsGuessKind::Hesitates);
    assert_eq!(torn.candidates[0].target.id, "t-ngc7000");

    let only = decide("c.fit".into(), at(314.0, 44.333, None), None, field.clone());
    assert_eq!(only.kind, FitsGuessKind::Candidates);
    assert_eq!(only.candidates.len(), 2);

    let unknown = decide(
        "d.fit".into(),
        at(314.0, 44.333, Some("Comète C/2026 T1")),
        None,
        field,
    );
    assert_eq!(unknown.kind, FitsGuessKind::UnknownObject);
    assert_eq!(unknown.candidates.len(), 2);

    let mute = HeaderTarget {
        object: None,
        ra_degrees: None,
        dec_degrees: None,
        position_source: None,
        field_degrees: [1.0, 1.0],
        field_from_header: false,
    };
    assert_eq!(
        decide("e.fit".into(), mute, None, vec![]).kind,
        FitsGuessKind::Nothing
    );

    // A name without a position decides alone.
    let name_only = header_target(&header(&object_only()));
    assert_eq!(
        decide("f.fit".into(), name_only, Some(ngc), vec![]).kind,
        FitsGuessKind::Found
    );
}

#[test]
fn a_file_is_read_on_this_computer_and_the_index_is_kept_and_read_again_when_its_revision_changes()
{
    let dir = TempDir::new("fits-target");
    let index_path = dir.path().join("data").join(INDEX_FILE);
    let service = FakeService::with_night(&[]);

    // Fixture 1: the name settles it, no index asked.
    let object = dir.path().join("M31_Light_120s_0042.fit");
    std::fs::write(&object, fits_bytes(&object_only(), 2880)).unwrap();
    let guess = guess_fits_target(&service, "s", None, &object, "fr", &index_path).unwrap();
    assert_eq!(guess.kind, FitsGuessKind::Found);
    assert_eq!(guess.file_name, "M31_Light_120s_0042.fit");
    assert_eq!(
        service.calls(),
        ["INDEX q=M31 sort=planner offset=0 limit=8"]
    );
    assert!(!index_path.exists());

    // Fixture 2: coordinates only, the index walked then kept.
    let coordinates = dir.path().join("Lights_0007.fit");
    std::fs::write(&coordinates, fits_bytes(&objctra_only(), 2880)).unwrap();
    let guess = guess_fits_target(&service, "s", None, &coordinates, "fr", &index_path).unwrap();
    assert_eq!(guess.kind, FitsGuessKind::Candidates);
    assert_eq!(guess.candidates[0].target.designation, "NGC 7000");
    assert!(index_path.exists());
    let walked = service.calls().len();

    // Again: one row asked, the revision is the same, the kept index serves.
    guess_fits_target(&service, "s", None, &coordinates, "fr", &index_path).unwrap();
    assert_eq!(service.calls().len(), walked + 1);
    assert!(
        service
            .calls()
            .last()
            .unwrap()
            .ends_with("offset=0 limit=1")
    );

    // A new revision: walked again.
    *service.revision.borrow_mut() = "rev-2".to_string();
    guess_fits_target(&service, "s", None, &coordinates, "fr", &index_path).unwrap();
    assert_eq!(
        load_index(&index_path).unwrap().revision.as_deref(),
        Some("rev-2")
    );

    // Fixture 3: the solution and the name agree.
    let solved_file = dir.path().join("NGC7000_solved.fits");
    std::fs::write(&solved_file, fits_bytes(&solved(), 2880)).unwrap();
    let guess = guess_fits_target(&service, "s", None, &solved_file, "fr", &index_path).unwrap();
    assert_eq!(guess.kind, FitsGuessKind::Found);
    assert_eq!(guess.named.unwrap().id, "t-ngc7000");

    // Not a FITS: said so.
    let text = dir.path().join("notes.txt");
    std::fs::write(&text, "nothing").unwrap();
    assert!(matches!(
        guess_fits_target(&service, "s", None, &text, "fr", &index_path),
        Err(FitsGuessFailure::Unreadable(_))
    ));
}

/* ---------------------------- A new target ------------------------------- */

#[test]
fn a_new_target_needs_a_name_one_of_the_seven_labels_and_both_coordinates_or_none() {
    let spec = |name: &str, label: &str, ra: Option<f64>, dec: Option<f64>| PersonalTargetCreate {
        name: name.to_string(),
        label: label.to_string(),
        ra_hours: ra,
        dec_degrees: dec,
    };
    assert_eq!(
        personal_target_body(&spec(" Comète C/2026 T1 ", "Grand champ", None, None)).unwrap(),
        json!({ "name": "Comète C/2026 T1", "label": "Grand champ" })
    );
    assert_eq!(
        personal_target_body(&spec("X", "Galaxie", Some(0.7), Some(41.3))).unwrap(),
        json!({ "name": "X", "label": "Galaxie", "raHours": 0.7, "decDegrees": 41.3 })
    );
    assert_eq!(
        personal_target_body(&spec(" ", "Galaxie", None, None)),
        Err(PersonalTargetError::BadName)
    );
    assert_eq!(
        personal_target_body(&spec("X", "C/2026 T1", None, None)),
        Err(PersonalTargetError::BadLabel("C/2026 T1".to_string()))
    );
    assert_eq!(
        personal_target_body(&spec("X", "Amas", Some(1.0), None)),
        Err(PersonalTargetError::BadCoordinates)
    );
    assert_eq!(
        personal_target_body(&spec("X", "Amas", Some(24.0), Some(0.0))),
        Err(PersonalTargetError::BadCoordinates)
    );
}

#[test]
fn a_new_target_is_created_once_then_added_to_the_night() {
    let service = FakeService::with_night(&["t-ic1396"]);
    let created = create_personal_target(
        &service,
        "s",
        "t",
        &PersonalTargetCreate {
            name: "Comète C/2026 T1".to_string(),
            label: "Grand champ".to_string(),
            ra_hours: None,
            dec_degrees: None,
        },
        Some("night-7"),
    )
    .unwrap();
    assert_eq!(created.target["id"], "p-comet");
    assert_eq!(created.night_target_ids.unwrap(), ["t-ic1396", "p-comet"]);
    let calls = service.calls();
    assert_eq!(calls.len(), 3);
    assert!(calls[0].starts_with("POST personal"));
    assert_eq!(
        calls[2],
        r#"PATCH night-7 {"targetIds":["t-ic1396","p-comet"]}"#
    );
}

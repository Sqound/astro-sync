//! The targets of a night chosen in Astro Sync: the catalogue searched in the
//! application, a target read from the header of a capture's FITS, a new
//! target outside the catalogue, and the night's list of targets changed by
//! the service's own routes.
//!
//! - [`search_targets`]: `GET /v1/target-index?q=…&sort=planner` -- the
//!   compact columnar view of the shared catalogue (a few kilobytes for eight
//!   cards where `GET /v1/targets` sends tens), with the text normalised
//!   first ([`search_query`]: « m 31 » = « M31 », « ngc7000 » = « NGC 7000 »).
//! - [`change_night_targets`]: `GET /v1/sessions/{id}` then `PATCH
//!   /v1/sessions/{id}/night` with `targetIds` = the night's targets plus the
//!   new one (or minus one just added in the same opening of the wizard: the
//!   window offers nothing else, a target already in the night is managed in
//!   the Journal on the website).
//! - [`header_target`] and [`guess_fits_target`]: the header **only**, read
//!   on this computer (nothing of the file leaves): `OBJECT`, then the
//!   position in the order `CRVAL1`/`CRVAL2` (a solved image), `RA`/`DEC`
//!   (NINA, degrees), `OBJCTRA`/`OBJCTDEC` (sexagesimal); the field from the
//!   header's pixel scale × `NAXIS1`/`NAXIS2`, else 1°. The name is resolved
//!   by the search, the position by the catalogue's index kept in Astro
//!   Sync's data ([`CatalogueIndex`], read again when its `revision`
//!   changes).
//! - [`create_personal_target`]: `POST /v1/targets/personal` -- name, one of
//!   the seven labels of the service, optional coordinates -- then the
//!   target added to the night.

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::analysis::acquisition_steps::header_pixel_scale;
use crate::analysis::fits::{self, FitsHeader};
use crate::analysis::sky_night::parse_angle;
use crate::analysis::wcs::angular_distance_deg;
use crate::detection::normalize_object;
use crate::gui_bridge::{GuiBridgeError, HttpPortalGuiTransport};
use crate::night_service::NightServiceTransport;
use crate::service_wait::RequestFactory;

/// Cards per search page (eight cards at most).
pub const SEARCH_PAGE: u32 = 8;
/// "Show more targets" stops here.
pub const SEARCH_MAXIMUM: u32 = 24;
/// A page of the index walk (the route's own maximum).
pub const INDEX_PAGE: u32 = 1000;
/// The candidates of a field shown at most.
pub const FIELD_CANDIDATES: usize = 8;
/// The field assumed when the header gives no scale (degrees).
pub const DEFAULT_FIELD_DEGREES: f64 = 1.0;
/// The file of the catalogue's index in Astro Sync's data.
pub const INDEX_FILE: &str = "catalogue-index.json";

/// The seven labels of a personal target (`personal-target.schema.json`,
/// the public gallery's generic tags), as the website's form offers them.
pub const PERSONAL_LABELS: [&str; 7] = [
    "Galaxie",
    "Nébuleuse",
    "Amas",
    "Lune",
    "Planète",
    "Soleil",
    "Grand champ",
];

/* --------------------------------- Search -------------------------------- */

/// The text sent to the catalogue: trimmed, inner spaces collapsed, and a
/// designation written the catalogue's way (« m 31 » → « M31 », « ngc7000 »
/// → « NGC 7000 », « ic-1396 » → « IC 1396 », « sh2 155 » → « Sh2-155 »).
/// `None` below two characters (the window waits).
#[must_use]
pub fn search_query(text: &str) -> Option<String> {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() < 2 {
        return None;
    }
    Some(designation_form(&collapsed).unwrap_or(collapsed))
}

fn designation_form(text: &str) -> Option<String> {
    let lowered = text.to_lowercase();
    let squeezed: String = lowered
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '-' | '_' | '.'))
        .collect();
    let split = |prefix: &str| -> Option<(String, String)> {
        let rest = squeezed.strip_prefix(prefix)?;
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            return None;
        }
        let tail = &rest[digits.len()..];
        let number = digits.trim_start_matches('0');
        let number = if number.is_empty() { "0" } else { number };
        Some((number.to_string(), tail.to_string()))
    };
    // The longer prefixes first: « sh2 » before « s », « ldn » before « l ».
    if let Some((n, tail)) = split("sh2")
        && tail.is_empty()
    {
        return Some(format!("Sh2-{n}"));
    }
    for (prefix, written) in [
        ("ngc", "NGC "),
        ("ic", "IC "),
        ("ldn", "LDN "),
        ("lbn", "LBN "),
    ] {
        if let Some((n, tail)) = split(prefix)
            && tail.chars().all(|c| c.is_ascii_alphabetic())
            && tail.len() <= 1
        {
            return Some(format!("{written}{n}{}", tail.to_uppercase()));
        }
    }
    if let Some((n, tail)) = split("abell")
        && tail.is_empty()
    {
        return Some(format!("Abell {n}"));
    }
    for prefix in ["m", "b"] {
        if let Some((n, tail)) = split(prefix)
            && tail.is_empty()
        {
            return Some(format!("{}{n}", prefix.to_uppercase()));
        }
    }
    None
}

/// One target of the catalogue, as a card shows it (the phone's
/// `TargetCard`): the words are the service's, in the language asked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogueTarget {
    pub id: String,
    pub designation: String,
    /// The common name ("Nébuleuse de l'Amérique du Nord", French catalogue name), often absent.
    pub name: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    /// "Emission nebula".
    pub type_label: Option<String>,
    /// "Cygnus".
    pub constellation_label: Option<String>,
    /// Right ascension in hours, declination in degrees (J2000).
    pub ra_hours: Option<f64>,
    pub dec_degrees: Option<f64>,
    pub magnitude: Option<f64>,
    /// « 120 × 100′ ».
    pub size_label: Option<String>,
    /// The larger apparent size, arcminutes.
    pub size_arcmin: Option<f64>,
    /// The preview: the catalogue's photo, else its drawn view; a path on
    /// the service (`/…`).
    pub image_path: Option<String>,
}

/// One page of the index, read by its `fields`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IndexPage {
    pub revision: Option<String>,
    pub total: u64,
    pub targets: Vec<CatalogueTarget>,
}

/// Reads a page of `GET /v1/target-index`: each column by its name in
/// `fields`, a column unknown or absent ignored (the notation is meant to
/// grow).
pub fn parse_index_page(value: &Value) -> Result<IndexPage, GuiBridgeError> {
    let bad = |what: &str| GuiBridgeError::InvalidResponse(format!("target index: {what}"));
    let fields: Vec<&str> = value
        .get("fields")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("no fields"))?
        .iter()
        .map(|f| f.as_str().unwrap_or(""))
        .collect();
    let column = |name: &str| fields.iter().position(|f| *f == name);
    let dictionary = |name: &str| -> Vec<Option<String>> {
        value
            .get(name)
            .and_then(Value::as_array)
            .map(|pairs| {
                pairs
                    .iter()
                    .map(|pair| pair.get(1).and_then(Value::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let types = dictionary("types");
    let constellations = dictionary("constellations");
    let (id, designation, name, aliases) = (
        column("id").ok_or_else(|| bad("no id column"))?,
        column("designation").ok_or_else(|| bad("no designation column"))?,
        column("name"),
        column("aliases"),
    );
    let (ra, dec, size, magnitude, kind, constellation, size_label, photo, visual) = (
        column("ra"),
        column("dec"),
        column("sizeArcmin"),
        column("magnitude"),
        column("type"),
        column("constellation"),
        column("sizeLabel"),
        column("photoUrl"),
        column("visualUrl"),
    );
    let rows = value
        .get("rows")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("no rows"))?;
    let mut targets = Vec::with_capacity(rows.len());
    for row in rows {
        let cell = |index: Option<usize>| index.and_then(|i| row.get(i)).filter(|v| !v.is_null());
        let text = |index: Option<usize>| {
            cell(index)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let number = |index: Option<usize>| cell(index).and_then(Value::as_f64);
        let label = |index: Option<usize>, dictionary: &[Option<String>]| {
            cell(index)
                .and_then(Value::as_u64)
                .and_then(|i| dictionary.get(usize::try_from(i).ok()?).cloned().flatten())
        };
        let (Some(id), Some(designation)) = (text(Some(id)), text(Some(designation))) else {
            continue;
        };
        targets.push(CatalogueTarget {
            id,
            designation,
            name: text(name),
            aliases: cell(aliases)
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            type_label: label(kind, &types),
            constellation_label: label(constellation, &constellations),
            ra_hours: number(ra),
            dec_degrees: number(dec),
            magnitude: number(magnitude),
            size_label: text(size_label),
            size_arcmin: cell(size)
                .and_then(Value::as_array)
                .and_then(|sizes| sizes.iter().filter_map(Value::as_f64).reduce(f64::max)),
            image_path: text(photo).or_else(|| text(visual)),
        });
    }
    Ok(IndexPage {
        revision: value
            .get("revision")
            .and_then(Value::as_str)
            .map(str::to_string),
        total: value.get("total").and_then(Value::as_u64).unwrap_or(0),
        targets,
    })
}

/// What the search answers: the text sent, how many targets match, the
/// cards of this page.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetSearch {
    pub query: String,
    pub total: u64,
    pub targets: Vec<CatalogueTarget>,
}

/// The catalogue and the personal targets, as the service serves them.
pub trait CatalogueTransport {
    /// `GET /v1/target-index` with these parameters (the token, when
    /// there is one, only adds favourites the window ignores).
    fn target_index(
        &self,
        base_url: &str,
        token: Option<&str>,
        query: &[(&'static str, String)],
    ) -> Result<Value, GuiBridgeError>;
    /// `POST /v1/targets/personal`, sent once.
    fn create_personal_target(
        &self,
        base_url: &str,
        token: &str,
        body: &Value,
    ) -> Result<Value, GuiBridgeError>;
}

fn route(base_url: &str, path: &str) -> String {
    format!("{}/api{path}", base_url.trim_end_matches('/'))
}

impl CatalogueTransport for HttpPortalGuiTransport {
    fn target_index(
        &self,
        base_url: &str,
        token: Option<&str>,
        query: &[(&'static str, String)],
    ) -> Result<Value, GuiBridgeError> {
        let url = route(base_url, "/v1/target-index");
        let token = token.map(str::to_owned);
        let query = query.to_vec();
        let build: RequestFactory = Arc::new(move |client| {
            let request = client.get(url.clone()).query(&query);
            match &token {
                Some(token) => request.bearer_auth(token),
                None => request,
            }
        });
        self.read_json(&build)
    }

    fn create_personal_target(
        &self,
        base_url: &str,
        token: &str,
        body: &Value,
    ) -> Result<Value, GuiBridgeError> {
        let url = route(base_url, "/v1/targets/personal");
        self.write_json_once(base_url, &|client| {
            client.post(url.clone()).bearer_auth(token).json(body)
        })
    }
}

/// The catalogue searched: `limit` cards from `offset`, best first for a
/// night (`sort=planner`). A text under two characters asks nothing.
pub fn search_targets<T: CatalogueTransport>(
    transport: &T,
    base_url: &str,
    token: Option<&str>,
    text: &str,
    locale: &str,
    offset: u32,
    limit: u32,
) -> Result<TargetSearch, GuiBridgeError> {
    let Some(query) = search_query(text) else {
        return Ok(TargetSearch {
            query: text.trim().to_string(),
            total: 0,
            targets: Vec::new(),
        });
    };
    let limit = limit.clamp(1, SEARCH_MAXIMUM);
    let offset = offset.min(SEARCH_MAXIMUM);
    let page = parse_index_page(&transport.target_index(
        base_url,
        token,
        &[
            ("q", query.clone()),
            ("sort", "planner".to_string()),
            ("locale", locale.to_string()),
            ("offset", offset.to_string()),
            ("limit", limit.to_string()),
        ],
    )?)?;
    Ok(TargetSearch {
        query,
        total: page.total,
        targets: page.targets,
    })
}

/* ----------------------------- The night's list --------------------------- */

/// A change of the night's targets from the wizard.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "targetId")]
pub enum NightTargetChange {
    /// A target of the catalogue or of the space, added.
    Add(String),
    /// A target added in this opening of the wizard, taken back (the window
    /// offers it for nothing else).
    UndoAdd(String),
}

/// The night's targets as the service holds them (`night.targetIds.value`,
/// the night's own list or the plan's).
#[must_use]
pub fn night_target_ids(detail: &Value) -> Vec<String> {
    detail
        .pointer("/night/targetIds/value")
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The list after the change; `None` when nothing changes (already there,
/// or not there to take back).
#[must_use]
pub fn changed_target_ids(current: &[String], change: &NightTargetChange) -> Option<Vec<String>> {
    match change {
        NightTargetChange::Add(id) => (!current.contains(id)).then(|| {
            let mut next = current.to_vec();
            next.push(id.clone());
            next
        }),
        NightTargetChange::UndoAdd(id) => current
            .contains(id)
            .then(|| current.iter().filter(|t| *t != id).cloned().collect()),
    }
}

/// The night's targets changed by the service's own routes: read, then
/// `PATCH …/night` with the whole list (the other targets kept as they
/// are). Answers the list in force afterwards.
pub fn change_night_targets<T: NightServiceTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
    session_id: &str,
    change: &NightTargetChange,
) -> Result<Vec<String>, GuiBridgeError> {
    let detail = transport.session_detail(base_url, token, session_id)?;
    let current = night_target_ids(&detail);
    let Some(next) = changed_target_ids(&current, change) else {
        return Ok(current);
    };
    let answer =
        transport.patch_night(base_url, token, session_id, &json!({ "targetIds": next }))?;
    let written = night_target_ids(&answer);
    Ok(if written.is_empty() && !next.is_empty() {
        next
    } else {
        written
    })
}

/* ------------------------------ The FITS header -------------------------- */

/// Where the position of a header came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PositionSource {
    /// `CRVAL1`/`CRVAL2` of a solved image.
    Crval,
    /// `RA`/`DEC` (NINA writes degrees).
    RaDec,
    /// `OBJCTRA`/`OBJCTDEC`, sexagesimal (NINA, ASIAIR).
    Objctra,
}

/// What a header says of its target.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeaderTarget {
    /// `OBJECT` as written.
    pub object: Option<String>,
    pub ra_degrees: Option<f64>,
    pub dec_degrees: Option<f64>,
    pub position_source: Option<PositionSource>,
    /// Width and height of the image on the sky, degrees.
    pub field_degrees: [f64; 2],
    /// `false`: the default field (no scale or no size in the header).
    pub field_from_header: bool,
}

fn hours_text(text: &str) -> Option<f64> {
    // A number alone is degrees for `RA`, hours for `OBJCTRA`: the callers
    // say which; this reads sexagesimal hours (« 00 42 44.3 », « 0:42:44 »,
    // « 0h42m44s »).
    let cleaned = text.trim().replace(['h', 'H'], " ");
    let hours = parse_angle(&cleaned)?;
    (0.0..24.0).contains(&hours).then_some(hours * 15.0)
}

fn declination_text(text: &str) -> Option<f64> {
    parse_angle(text).filter(|d| (-90.0..=90.0).contains(d))
}

fn ra_in_range(ra: f64) -> Option<f64> {
    ra.is_finite().then(|| ra.rem_euclid(360.0))
}

/// The header's own words: `OBJECT`, the position (first of `CRVAL`, `RA`/
/// `DEC`, `OBJCTRA`/`OBJCTDEC`), the field.
#[must_use]
pub fn header_target(header: &FitsHeader) -> HeaderTarget {
    let object = header
        .get("OBJECT")
        .map(str::trim)
        .filter(|o| !o.is_empty())
        .map(str::to_string);
    let crval = || {
        // A CRVAL without a sky projection (`CTYPE1 = 'RA---…'`) is not a
        // position on the sky.
        let ctype = header.get("CTYPE1")?.trim().to_ascii_uppercase();
        if !ctype.starts_with("RA") {
            return None;
        }
        let ra = ra_in_range(header.get_f64("CRVAL1")?)?;
        let dec = header
            .get_f64("CRVAL2")
            .filter(|d| (-90.0..=90.0).contains(d))?;
        Some((ra, dec, PositionSource::Crval))
    };
    let ra_dec = || {
        let ra = header
            .get_f64("RA")
            .and_then(ra_in_range)
            .or_else(|| header.get("RA").and_then(hours_text))?;
        let dec = header
            .get_f64("DEC")
            .filter(|d| (-90.0..=90.0).contains(d))
            .or_else(|| header.get("DEC").and_then(declination_text))?;
        Some((ra, dec, PositionSource::RaDec))
    };
    let objctra = || {
        let ra = header.get("OBJCTRA").and_then(hours_text)?;
        let dec = header.get("OBJCTDEC").and_then(declination_text)?;
        Some((ra, dec, PositionSource::Objctra))
    };
    let position = crval().or_else(ra_dec).or_else(objctra);
    let scale = header_pixel_scale(header);
    let size = header
        .get_i64("NAXIS1")
        .zip(header.get_i64("NAXIS2"))
        .filter(|(w, h)| *w > 0 && *h > 0);
    let (field_degrees, field_from_header) = match (scale, size) {
        (Some(scale), Some((w, h))) => {
            #[allow(clippy::cast_precision_loss)]
            let field = [scale * w as f64 / 3600.0, scale * h as f64 / 3600.0];
            (field, true)
        }
        _ => ([DEFAULT_FIELD_DEGREES, DEFAULT_FIELD_DEGREES], false),
    };
    HeaderTarget {
        object,
        ra_degrees: position.map(|p| p.0),
        dec_degrees: position.map(|p| p.1),
        position_source: position.map(|p| p.2),
        field_degrees,
        field_from_header,
    }
}

/// The header of a capture's file (FITS or XISF), its bytes only.
pub fn read_capture_header(path: &Path) -> std::io::Result<FitsHeader> {
    let xisf = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("xisf"));
    if xisf {
        crate::xisf::read_header_file(path)
    } else {
        fits::read_primary_header_file(path)
    }
}

/// A target whose centre falls in the field, and how far from its middle.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldCandidate {
    pub target: CatalogueTarget,
    pub distance_degrees: f64,
}

/// The radius of a field: half its larger side (the rotation of the image
/// is unknown, a centre inside this circle is "in the field").
#[must_use]
pub fn field_radius(field: [f64; 2]) -> f64 {
    field[0].max(field[1]) / 2.0
}

/// The targets of `targets` whose centre falls in the field, the nearest
/// first, [`FIELD_CANDIDATES`] at most.
#[must_use]
pub fn candidates_in_field(
    targets: &[CatalogueTarget],
    ra_degrees: f64,
    dec_degrees: f64,
    field: [f64; 2],
) -> Vec<FieldCandidate> {
    let radius = field_radius(field);
    let mut found: Vec<FieldCandidate> = targets
        .iter()
        .filter_map(|target| {
            let distance = angular_distance_deg(
                ra_degrees,
                dec_degrees,
                target.ra_hours? * 15.0,
                target.dec_degrees?,
            );
            (distance <= radius).then(|| FieldCandidate {
                target: target.clone(),
                distance_degrees: distance,
            })
        })
        .collect();
    found.sort_by(|a, b| a.distance_degrees.total_cmp(&b.distance_degrees));
    found.truncate(FIELD_CANDIDATES);
    found
}

/// Does a target answer to this `OBJECT` (designation, alias or common
/// name, without case or spaces)?
#[must_use]
pub fn names_object(target: &CatalogueTarget, object: &str) -> bool {
    let wanted = normalize_object(object);
    std::iter::once(&target.designation)
        .chain(target.aliases.iter())
        .chain(target.name.iter())
        .any(|name| normalize_object(name) == wanted)
}

/// What the window shows after reading a header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FitsGuessKind {
    /// The name, and the position when there is one, agree: one card.
    Found,
    /// The name and the position disagree: the name's card first, then the
    /// field's ("The header hesitates").
    Hesitates,
    /// No name: the targets of the field, the nearest first.
    Candidates,
    /// A name the catalogue does not know ("Create "..." as a new
    /// target"), with the field's targets when there is a position.
    UnknownObject,
    /// Neither name nor position.
    Nothing,
}

/// The reading of one file.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FitsGuess {
    pub file_name: String,
    pub header: HeaderTarget,
    pub kind: FitsGuessKind,
    /// The target its `OBJECT` names, when the catalogue knows it.
    pub named: Option<CatalogueTarget>,
    pub candidates: Vec<FieldCandidate>,
}

/// The decision, from what was read and found (pure).
#[must_use]
pub fn decide(
    file_name: String,
    header: HeaderTarget,
    named: Option<CatalogueTarget>,
    field: Vec<FieldCandidate>,
) -> FitsGuess {
    let position = header.ra_degrees.zip(header.dec_degrees);
    let agrees = |target: &CatalogueTarget| match (position, target.ra_hours, target.dec_degrees) {
        (Some((ra, dec)), Some(t_ra), Some(t_dec)) => {
            let slack = target.size_arcmin.unwrap_or(0.0) / 120.0;
            angular_distance_deg(ra, dec, t_ra * 15.0, t_dec)
                <= field_radius(header.field_degrees) + slack
        }
        // No position on one side: the name alone decides.
        _ => true,
    };
    let (kind, candidates) = match (&header.object, &named, position) {
        (Some(_), Some(target), _) if agrees(target) => (FitsGuessKind::Found, Vec::new()),
        (Some(_), Some(target), _) => (
            FitsGuessKind::Hesitates,
            field
                .into_iter()
                .filter(|c| c.target.id != target.id)
                .collect(),
        ),
        (Some(_), None, _) => (FitsGuessKind::UnknownObject, field),
        (None, _, Some(_)) => (FitsGuessKind::Candidates, field),
        (None, _, None) => (FitsGuessKind::Nothing, Vec::new()),
    };
    FitsGuess {
        file_name,
        header,
        kind,
        named: if matches!(kind, FitsGuessKind::Found | FitsGuessKind::Hesitates) {
            named
        } else {
            None
        },
        candidates,
    }
}

/* ------------------------------ The index kept --------------------------- */

/// The catalogue's index kept in Astro Sync's data: what the position of a
/// header is matched against, read again only when its revision changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogueIndex {
    pub revision: Option<String>,
    pub locale: String,
    pub targets: Vec<CatalogueTarget>,
}

/// The index kept, `None` when absent or unreadable.
#[must_use]
pub fn load_index(path: &Path) -> Option<CatalogueIndex> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Writes the index (a temporary file, then renamed: never half a file).
pub fn save_index(path: &Path, index: &CatalogueIndex) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let partial = path.with_extension("json.partial");
    std::fs::write(
        &partial,
        serde_json::to_vec(index).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(partial, path)
}

/// The index in force: the one kept when the service's revision (one row
/// asked) and the language are the same, else walked again page by page
/// (`sort=designation`, stable); a walk whose revision changes midway
/// starts over once.
pub fn refresh_index<T: CatalogueTransport>(
    transport: &T,
    base_url: &str,
    token: Option<&str>,
    locale: &str,
    kept: Option<CatalogueIndex>,
) -> Result<(CatalogueIndex, bool), GuiBridgeError> {
    let page = |offset: u64, limit: u32| -> Result<IndexPage, GuiBridgeError> {
        parse_index_page(&transport.target_index(
            base_url,
            token,
            &[
                ("sort", "designation".to_string()),
                ("locale", locale.to_string()),
                ("offset", offset.to_string()),
                ("limit", limit.to_string()),
            ],
        )?)
    };
    let probe = page(0, 1)?;
    if let Some(kept) = kept
        && kept.locale == locale
        && probe.revision.is_some()
        && kept.revision == probe.revision
    {
        return Ok((kept, false));
    }
    for _attempt in 0..2 {
        let mut targets = Vec::new();
        let mut revision = None;
        let mut offset = 0_u64;
        let mut changed = false;
        loop {
            let next = page(offset, INDEX_PAGE)?;
            if offset == 0 {
                revision.clone_from(&next.revision);
            } else if next.revision != revision {
                changed = true;
                break;
            }
            let count = next.targets.len() as u64;
            targets.extend(next.targets);
            offset += u64::from(INDEX_PAGE);
            if count == 0 || offset >= next.total {
                break;
            }
        }
        if !changed {
            return Ok((
                CatalogueIndex {
                    revision,
                    locale: locale.to_string(),
                    targets,
                },
                true,
            ));
        }
    }
    Err(GuiBridgeError::InvalidResponse(
        "the catalogue changed twice while it was read".to_string(),
    ))
}

/// "Find the target with a FITS": the header read, the name resolved by the search, the
/// field matched against the index (refreshed when needed, then kept at
/// `index_path`) when the name does not settle it.
pub fn guess_fits_target<T: CatalogueTransport>(
    transport: &T,
    base_url: &str,
    token: Option<&str>,
    file: &Path,
    locale: &str,
    index_path: &Path,
) -> Result<FitsGuess, FitsGuessFailure> {
    let header =
        read_capture_header(file).map_err(|e| FitsGuessFailure::Unreadable(e.to_string()))?;
    let read = header_target(&header);
    let file_name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let named = match &read.object {
        Some(object) => search_targets(transport, base_url, token, object, locale, 0, SEARCH_PAGE)?
            .targets
            .into_iter()
            .find(|target| names_object(target, object)),
        None => None,
    };
    let settled = decide(file_name.clone(), read.clone(), named.clone(), Vec::new());
    if settled.kind == FitsGuessKind::Found || settled.kind == FitsGuessKind::Nothing {
        return Ok(settled);
    }
    let Some((ra, dec)) = read.ra_degrees.zip(read.dec_degrees) else {
        return Ok(settled);
    };
    let (index, fresh) = refresh_index(transport, base_url, token, locale, load_index(index_path))?;
    if fresh {
        // A copy that cannot be written is read again next time: no harm.
        let _ = save_index(index_path, &index);
    }
    let field = candidates_in_field(&index.targets, ra, dec, read.field_degrees);
    Ok(decide(file_name, read, named, field))
}

#[derive(Debug, Error)]
pub enum FitsGuessFailure {
    #[error("the file's header cannot be read: {0}")]
    Unreadable(String),
    #[error(transparent)]
    Service(#[from] GuiBridgeError),
}

/* ---------------------------- A new target ------------------------------- */

/// "New target": what the window's form sends.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalTargetCreate {
    pub name: String,
    /// One of [`PERSONAL_LABELS`].
    pub label: String,
    pub ra_hours: Option<f64>,
    pub dec_degrees: Option<f64>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PersonalTargetError {
    #[error("the name must have 1 to 80 characters")]
    BadName,
    #[error("the label must be one of the seven of the service, got {0:?}")]
    BadLabel(String),
    #[error("the right ascension (0–24 h) and the declination (−90–90°) go together")]
    BadCoordinates,
}

/// The body of `POST /v1/targets/personal`, checked before anything leaves.
pub fn personal_target_body(spec: &PersonalTargetCreate) -> Result<Value, PersonalTargetError> {
    let name = spec.name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(PersonalTargetError::BadName);
    }
    if !PERSONAL_LABELS.contains(&spec.label.as_str()) {
        return Err(PersonalTargetError::BadLabel(spec.label.clone()));
    }
    let mut body = json!({ "name": name, "label": spec.label });
    match (spec.ra_hours, spec.dec_degrees) {
        (None, None) => {}
        (Some(ra), Some(dec)) if (0.0..24.0).contains(&ra) && (-90.0..=90.0).contains(&dec) => {
            body["raHours"] = json!(ra);
            body["decDegrees"] = json!(dec);
        }
        _ => return Err(PersonalTargetError::BadCoordinates),
    }
    Ok(body)
}

#[derive(Debug, Error)]
pub enum PersonalTargetFailure {
    #[error(transparent)]
    Invalid(#[from] PersonalTargetError),
    #[error(transparent)]
    Service(#[from] GuiBridgeError),
}

/// What "Create and add to the night" answers: the target created, and the
/// night's targets afterwards (`None`: no night was named).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalTargetCreated {
    pub target: Value,
    pub night_target_ids: Option<Vec<String>>,
}

/// The new target created in the space, then added to the night when one
/// is named.
pub fn create_personal_target<T: CatalogueTransport + NightServiceTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
    spec: &PersonalTargetCreate,
    session_id: Option<&str>,
) -> Result<PersonalTargetCreated, PersonalTargetFailure> {
    let body = personal_target_body(spec)?;
    let answer = CatalogueTransport::create_personal_target(transport, base_url, token, &body)?;
    let target = answer.get("target").cloned().unwrap_or(answer);
    let id = target
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| GuiBridgeError::InvalidResponse("no target id".to_string()))?
        .to_string();
    let night_target_ids = match session_id {
        Some(session) => Some(change_night_targets(
            transport,
            base_url,
            token,
            session,
            &NightTargetChange::Add(id),
        )?),
        None => None,
    };
    Ok(PersonalTargetCreated {
        target,
        night_target_ids,
    })
}

#[cfg(test)]
mod tests;

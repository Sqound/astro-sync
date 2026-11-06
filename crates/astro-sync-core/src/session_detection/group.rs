//! Step 2: nights, places, sessions, targets, the rebuilt plans, the
//! multi-night groups, the calibration and the logs attached.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::analysis::asiair::{AsiairOptions, parse_asiair_autorun_log};
use crate::analysis::log_offset::{LogClock, LogMatch, format_offset, match_log};
use crate::analysis::nina::{NinaOptions, parse_nina_log};
use crate::analysis::phd2::{Phd2Options, parse_phd2_guide_log};
use crate::analysis::wcs::angular_distance_deg;
use crate::catalogue_targets::{CatalogueTarget, field_radius};
use crate::detection::{FrameType, LogKind, NightWindow};
use crate::sky_solution::SolveFailure;
use crate::time_util::{format_iso8601_utc_millis, parse_iso8601_utc_to_unix_seconds};

use super::clock::{self, ClockInput, PlaceUsed};
use super::read::{FrameRecord, Reading};
use super::{
    AcquisitionRow, AttachedCalibration, AttachedLog, CALIBRATION_MAX_DAYS, CalibrationNote,
    CalibrationSet, DetectError, DetectOptions, DetectRequest, DetectedLogFile, DetectedSession,
    DetectedTarget, EquipmentProposal, KnownPlace, MULTI_NIGHT_GAP_DAYS, MultiNightGroup,
    MultiNightNight, PLACE_RADIUS_KM, PlaceProposal, Proposal, ReadingSummary, ScopedDetection,
    ScopedTarget, SetAside, TargetSource, UnassignedGroup, UserData, distance_km,
};

/// What [`super::propose`] completes: everything found, nothing matched yet.
#[derive(Debug, Clone, Default)]
pub struct Grouping {
    pub proposal: Proposal,
}

impl Grouping {
    /// The evenings the sessions span (`YYYY-MM-DD`), for a caller that
    /// reads the person's sessions and plans of those dates only.
    #[must_use]
    pub fn date_range(&self) -> Option<(String, String)> {
        let dates: BTreeSet<&str> = self
            .proposal
            .sessions
            .iter()
            .flat_map(|s| [s.evening_date.as_str(), s.written_evening_date.as_str()])
            .collect();
        Some(((*dates.first()?).to_string(), (*dates.last()?).to_string()))
    }
}

pub(super) fn iso(unix: f64) -> String {
    format_iso8601_utc_millis(unix)
}

/// Lower case letters and digits only: « NGC_7000 » = « ngc 7000 ».
pub(super) fn loose(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn names(target: &CatalogueTarget, text: &str) -> bool {
    let wanted = loose(text);
    wanted.chars().count() >= 2
        && std::iter::once(&target.designation)
            .chain(target.aliases.iter())
            .chain(target.name.iter())
            .any(|name| loose(name) == wanted)
}

fn by_name<'c>(catalogue: &'c [CatalogueTarget], text: &str) -> Option<&'c CatalogueTarget> {
    catalogue.iter().find(|target| names(target, text))
}

/// How far a target's centre may be from the frame's: half the field, and
/// half the target's own size.
fn reach(target: &CatalogueTarget, field: [f64; 2]) -> f64 {
    field_radius(field) + target.size_arcmin.unwrap_or(0.0) / 120.0
}

fn distance_to(target: &CatalogueTarget, ra: f64, dec: f64) -> Option<f64> {
    Some(angular_distance_deg(
        ra,
        dec,
        target.ra_hours? * 15.0,
        target.dec_degrees?,
    ))
}

/// The target of the catalogue that best fills the frame: within reach, the
/// one whose extent (up to the field's) goes furthest past the distance
/// between the centres. A large nebula framed whole beats a small object
/// near the middle of it ("Voile complet" is the Cygnus Loop, not one of
/// its arcs; « Sadr » is IC 1318, not the cluster M29 beside the star); a
/// small galaxy framed at the centre beats its neighbours.
fn nearest(
    catalogue: &[CatalogueTarget],
    ra: f64,
    dec: f64,
    field: [f64; 2],
) -> Option<(&CatalogueTarget, f64)> {
    let radius = field_radius(field);
    catalogue
        .iter()
        .filter_map(|t| {
            let d = distance_to(t, ra, dec)?;
            let extent = (t.size_arcmin.unwrap_or(0.0) / 120.0).min(radius);
            (d <= reach(t, field)).then_some((t, d, extent - d))
        })
        .max_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(t, d, _)| (t, d))
}

/// The mean position of frames (on the sphere: right ascensions near 0° and
/// 360° average well).
fn mean_position(frames: &[&FrameRecord]) -> Option<(f64, f64)> {
    let (mut x, mut y, mut z, mut n) = (0.0, 0.0, 0.0, 0);
    for frame in frames {
        let Some((ra, dec)) = frame.ra_degrees.zip(frame.dec_degrees) else {
            continue;
        };
        let (ra, dec) = (ra.to_radians(), dec.to_radians());
        x += dec.cos() * ra.cos();
        y += dec.cos() * ra.sin();
        z += dec.sin();
        n += 1;
    }
    (n > 0).then(|| {
        (
            y.atan2(x).to_degrees().rem_euclid(360.0),
            z.atan2(x.hypot(y)).to_degrees(),
        )
    })
}

/// What a folder's name may say of its target: the name after Vespera's
/// `observation_`, then runs of one to three words (`M_27`, `NGC_7000`).
fn folder_candidates(folder: &str) -> Vec<String> {
    let mut out = Vec::new();
    for marker in ["observation_", "observation-"] {
        if let Some(index) = folder.find(marker) {
            out.push(folder[index + marker.len()..].to_string());
        }
    }
    out.push(folder.to_string());
    let words: Vec<&str> = folder.split(['_', ' ']).filter(|w| !w.is_empty()).collect();
    for size in (1..=3).rev() {
        for window in words.windows(size) {
            out.push(window.join(" "));
        }
    }
    out
}

/// A target resolved for one frame.
#[derive(Debug, Clone, PartialEq)]
struct Resolved {
    key: String,
    target_id: Option<String>,
    designation: String,
    name: Option<String>,
    ra_degrees: Option<f64>,
    dec_degrees: Option<f64>,
    source: TargetSource,
}

impl Resolved {
    fn of(target: &CatalogueTarget, source: TargetSource) -> Self {
        Self {
            key: target.id.clone(),
            target_id: Some(target.id.clone()),
            designation: target.designation.clone(),
            name: target.name.clone(),
            ra_degrees: target.ra_hours.map(|h| h * 15.0),
            dec_degrees: target.dec_degrees,
            source,
        }
    }
}

/// `OBJECT` in the catalogue, else the coordinates, else the folder's name,
/// else `OBJECT` as a new target, else "plate solving to do".
fn resolve(frame: &FrameRecord, catalogue: &[CatalogueTarget]) -> Resolved {
    let position = frame.ra_degrees.zip(frame.dec_degrees);
    let by_position = || {
        let (ra, dec) = position?;
        nearest(catalogue, ra, dec, frame.field_degrees)
    };
    if let Some(object) = &frame.object
        && let Some(named) = by_name(catalogue, object)
    {
        let agrees = match position {
            Some((ra, dec)) => {
                distance_to(named, ra, dec).is_none_or(|d| d <= reach(named, frame.field_degrees))
            }
            None => true,
        };
        if agrees {
            return Resolved::of(named, TargetSource::Object);
        }
        if let Some((target, distance)) = by_position() {
            return Resolved::of(
                target,
                TargetSource::Coordinates {
                    distance_degrees: distance,
                },
            );
        }
        return Resolved::of(named, TargetSource::Object);
    }
    if let Some((target, distance)) = by_position() {
        return Resolved::of(
            target,
            TargetSource::Coordinates {
                distance_degrees: distance,
            },
        );
    }
    for folder in frame.folders.iter().rev() {
        for candidate in folder_candidates(folder) {
            if let Some(target) = by_name(catalogue, &candidate) {
                return Resolved::of(
                    target,
                    TargetSource::Folder {
                        folder: folder.clone(),
                    },
                );
            }
        }
    }
    if let Some(object) = &frame.object {
        return Resolved {
            key: format!("object:{}", loose(object)),
            target_id: None,
            designation: object.clone(),
            name: None,
            ra_degrees: frame.ra_degrees,
            dec_degrees: frame.dec_degrees,
            source: TargetSource::UnknownObject,
        };
    }
    // Vespera names its folders `…_observation_<target>`: a target's name by
    // its own convention, even outside the catalogue.
    for folder in frame.folders.iter().rev() {
        if let Some(index) = folder.find("observation_") {
            let name = folder[index + "observation_".len()..].trim().to_string();
            if !name.is_empty() {
                return Resolved {
                    key: format!("folder:{}", loose(&name)),
                    target_id: None,
                    designation: name,
                    name: None,
                    ra_degrees: None,
                    dec_degrees: None,
                    source: TargetSource::Folder {
                        folder: folder.clone(),
                    },
                };
            }
        }
    }
    let parent = frame
        .path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    Resolved {
        key: format!("unsolved:{parent}"),
        target_id: None,
        designation: frame
            .folders
            .last()
            .cloned()
            .unwrap_or_else(|| "?".to_string()),
        name: None,
        ra_degrees: None,
        dec_degrees: None,
        source: TargetSource::ToSolve,
    }
}

/// The known place within reach of a site, the nearest.
pub(super) fn place_near(places: &[KnownPlace], site: (f64, f64)) -> Option<&KnownPlace> {
    places
        .iter()
        .map(|p| (p, distance_km(site.0, site.1, p.latitude, p.longitude)))
        .filter(|(_, d)| *d <= PLACE_RADIUS_KM)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(p, _)| p)
}

/// The evening of an instant: civil noon to civil noon when the place's
/// offset is known, else mean solar noon at the longitude (the rule of
/// 2026-09-27), else UTC.
fn evening_of(unix: f64, site: Option<(f64, f64)>, places: &[KnownPlace]) -> String {
    let offset = site
        .and_then(|site| place_near(places, site))
        .and_then(|p| p.utc_offset_minutes);
    if let Some(minutes) = offset {
        return iso(unix + f64::from(minutes) * 60.0 - 12.0 * 3600.0)[..10].to_string();
    }
    NightWindow {
        start: 0.0,
        longitude: site.map_or(0.0, |s| s.1),
    }
    .evening_of(unix)
}

/// What a frame's target is resolved from: `OBJECT`, the position (to
/// 0.05°), the folders.
type ResolveKey = (Option<String>, Option<(i64, i64)>, Vec<String>);

#[derive(Clone, Copy)]
struct Light<'a> {
    frame: &'a FrameRecord,
    start: f64,
    end: f64,
}

fn round_to(value: f64, step: f64) -> i64 {
    (value / step).round() as i64
}

fn civil_days(date: &str) -> Option<i64> {
    Some((parse_iso8601_utc_to_unix_seconds(&format!("{date}T00:00:00Z"))? / 86_400.0) as i64)
}

fn most_common<'a>(values: impl Iterator<Item = &'a str>) -> String {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for value in values {
        *counts.entry(value).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(v, _)| v.to_string())
        .unwrap_or_default()
}

fn acquisition_rows(lights: &[Light<'_>]) -> Vec<AcquisitionRow> {
    let mut rows: BTreeMap<(String, i64, i64), AcquisitionRow> = BTreeMap::new();
    for light in lights {
        let f = light.frame;
        let key = (
            f.filter.clone().unwrap_or_default(),
            f.exposure_seconds.map_or(-1, |e| round_to(e, 0.001)),
            f.gain.map_or(-1, |g| round_to(g, 1.0)),
        );
        let row = rows.entry(key).or_insert_with(|| AcquisitionRow {
            filter: f.filter.clone(),
            exposure_seconds: f.exposure_seconds,
            gain: f.gain,
            frames: 0,
            total_seconds: 0.0,
        });
        row.frames += 1;
        row.total_seconds += f.exposure_seconds.unwrap_or(0.0);
    }
    rows.into_values().collect()
}

struct SessionDraft<'a> {
    evening: String,
    site: Option<(f64, f64)>,
    lights: Vec<Light<'a>>,
}

/// Groups the lights of one evening by place: sites within
/// [`PLACE_RADIUS_KM`] are one place; lights without a site join the only
/// place of the evening, or make a session of their own.
fn sessions_of_evening<'a>(evening: &str, lights: Vec<Light<'a>>) -> Vec<SessionDraft<'a>> {
    let mut drafts: Vec<SessionDraft<'a>> = Vec::new();
    let mut without_site = Vec::new();
    for light in lights {
        let Some(site) = light.frame.site else {
            without_site.push(light);
            continue;
        };
        let near = drafts.iter_mut().find(|d| {
            d.site
                .is_some_and(|s| distance_km(s.0, s.1, site.0, site.1) <= PLACE_RADIUS_KM)
        });
        match near {
            Some(draft) => draft.lights.push(light),
            None => drafts.push(SessionDraft {
                evening: evening.to_string(),
                site: Some(site),
                lights: vec![light],
            }),
        }
    }
    if !without_site.is_empty() {
        if drafts.len() == 1 {
            drafts[0].lights.extend(without_site);
        } else {
            drafts.push(SessionDraft {
                evening: evening.to_string(),
                site: None,
                lights: without_site,
            });
        }
    }
    for draft in &mut drafts {
        draft.lights.sort_by(|a, b| a.start.total_cmp(&b.start));
    }
    drafts.sort_by(|a, b| a.lights[0].start.total_cmp(&b.lights[0].start));
    drafts
}

/// The place proposals: one per site (within [`PLACE_RADIUS_KM`] of each
/// other), and one for the sessions whose frames name no place.
fn place_of(places: &mut Vec<PlaceProposal>, site: Option<(f64, f64)>, session: &str) -> String {
    let found = places
        .iter_mut()
        .find(|p| match (site, p.latitude.zip(p.longitude)) {
            (Some(a), Some(b)) => distance_km(a.0, a.1, b.0, b.1) <= PLACE_RADIUS_KM,
            (None, None) => true,
            _ => false,
        });
    if let Some(place) = found {
        place.sessions.push(session.to_string());
        return place.id.clone();
    }
    let id = if site.is_some() {
        format!(
            "place-{}",
            places.iter().filter(|p| p.latitude.is_some()).count() + 1
        )
    } else {
        "place-none".to_string()
    };
    places.push(PlaceProposal {
        id: id.clone(),
        latitude: site.map(|s| s.0),
        longitude: site.map(|s| s.1),
        sessions: vec![session.to_string()],
        ..PlaceProposal::default()
    });
    id
}

/// The instrument a multi-night group is kept to: the camera and the focal
/// length, as the equipment found writes them.
fn equipment_label(item: &EquipmentProposal) -> String {
    let camera = item.instrume.as_deref().unwrap_or("?");
    match item.focal_length_mm {
        Some(focal) => format!("{camera} · {focal:.0} mm"),
        None => camera.to_string(),
    }
}

/// The `FILTER` name of a frame, kept once with the equipment it was seen on.
fn note_filter(item: &mut EquipmentProposal, frame: &FrameRecord) {
    if let Some(filter) = frame.filter.as_deref().map(str::trim)
        && !filter.is_empty()
        && !item.filters.iter().any(|known| known == filter)
    {
        item.filters.push(filter.to_string());
    }
}

fn equipment_of(
    equipment: &mut Vec<EquipmentProposal>,
    frame: &FrameRecord,
    session: &str,
) -> String {
    // Focal lengths within 2 % are one telescope (the ASIAIR writes 189 or
    // 190 mm for the same one).
    let found = equipment.iter_mut().find(|e| {
        e.telescop == frame.telescop
            && e.instrume == frame.instrume
            && match (e.focal_length_mm, frame.focal_length_mm) {
                (Some(a), Some(b)) => (a - b).abs() <= 0.02 * a.max(b),
                (a, b) => a.is_none() && b.is_none(),
            }
    });
    if let Some(item) = found {
        if !item.sessions.iter().any(|s| s == session) {
            item.sessions.push(session.to_string());
        }
        note_filter(item, frame);
        return item.id.clone();
    }
    let id = format!("eq-{}", equipment.len() + 1);
    let mut item = EquipmentProposal {
        id: id.clone(),
        telescop: frame.telescop.clone(),
        instrume: frame.instrume.clone(),
        focal_length_mm: frame.focal_length_mm,
        sessions: vec![session.to_string()],
        ..EquipmentProposal::default()
    };
    note_filter(&mut item, frame);
    equipment.push(item);
    id
}

/// The place the clock is checked at: the header's, else the known place
/// matched, else the default place.
fn clock_place(site: Option<(f64, f64)>, user: &UserData) -> Option<((f64, f64), PlaceUsed)> {
    if let Some(site) = site {
        return Some((site, PlaceUsed::Header));
    }
    let id = user.default_place_id.as_deref()?;
    let place = user.places.iter().find(|p| p.id == id)?;
    Some((
        (place.latitude, place.longitude),
        PlaceUsed::DefaultPlace {
            place_id: place.id.clone(),
        },
    ))
}

/// The reading with the clock of its RAW frames brought to UTC. A camera
/// clock whose time zone the file does not say is the wall clock of the
/// place where the night was shot: with the place of the frame (its own,
/// else the default place), the clock is taken as that place's civil time
/// when its offset is known, else as its mean solar time (the rule of the
/// nights). Without any place it is left as written (UTC). The clock check
/// that follows can still propose a shift of whole hours.
fn with_clocks_in_utc(reading: &Reading, user: &UserData) -> Reading {
    let default_site = clock_place(None, user).map(|(site, _)| site);
    let mut adjusted = reading.clone();
    for frame in &mut adjusted.frames {
        if !frame.date_zone_unknown {
            continue;
        }
        frame.date_zone_unknown = false;
        let Some(site) = frame.site.or(default_site) else {
            continue;
        };
        let seconds = place_near(&user.places, site)
            .and_then(|p| p.utc_offset_minutes)
            .map_or(site.1 / 15.0 * 3600.0, |minutes| f64::from(minutes) * 60.0);
        frame.date_obs = frame.date_obs.map(|taken| taken - seconds);
    }
    adjusted
}

#[allow(clippy::too_many_lines)]
/// The filters of a target for the proposal: the one the person gave for the
/// run, else those the `FILTER` cards of its lights name.
fn target_filters(request: &DetectRequest, lights: &[Light<'_>]) -> BTreeSet<String> {
    if let Some(given) = request
        .filter
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        return BTreeSet::from([given.to_string()]);
    }
    lights
        .iter()
        .filter_map(|l| l.frame.filter.clone())
        .collect()
}

/// The filter the names around the lights of a target say, when the headers
/// name none: the most common name among its lights (a tie goes to the first
/// in alphabetical order, so the proposal does not depend on the file order).
fn target_filter_hint(lights: &[Light<'_>]) -> Option<String> {
    let mut counted: BTreeMap<&str, usize> = BTreeMap::new();
    for light in lights {
        if let Some(name) = light
            .frame
            .band_guess
            .as_ref()
            .and_then(|guess| guess.filter_name.as_deref())
        {
            *counted.entry(name).or_default() += 1;
        }
    }
    counted
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
        .map(|(name, _)| name.to_string())
}

pub(super) fn group(
    reading: &Reading,
    request: &DetectRequest,
    catalogue: &[CatalogueTarget],
    user: &UserData,
    options: &DetectOptions<'_>,
) -> Result<Grouping, DetectError> {
    let adjusted;
    let reading = if reading.frames.iter().any(|frame| frame.date_zone_unknown) {
        adjusted = with_clocks_in_utc(reading, user);
        &adjusted
    } else {
        reading
    };
    let mut set_aside = SetAside {
        unreadable: reading.unreadable.clone(),
        stacks: reading.stacks.clone(),
        other_files: reading.other_files.clone(),
        videos: reading.videos.clone(),
        video_captures: reading.video_captures.clone(),
        raw_files: reading.raw_files.clone(),
        ..SetAside::default()
    };
    let mut lights: Vec<Light<'_>> = Vec::new();
    let mut calibration: Vec<&FrameRecord> = Vec::new();
    // The images a known device stacked: not poses, attached to their
    // target below; what no target takes is set aside.
    let mut device_frames: Vec<&FrameRecord> = Vec::new();
    for frame in &reading.frames {
        if frame.device.is_some() {
            device_frames.push(frame);
            continue;
        }
        if frame.stacked {
            set_aside.stacks.push(frame.display());
            continue;
        }
        match (frame.kind, frame.date_obs) {
            (Some(FrameType::Light), Some(start)) => lights.push(Light {
                frame,
                start,
                end: start + frame.exposure_seconds.unwrap_or(0.0),
            }),
            (Some(FrameType::Light), None) => set_aside.without_date.push(frame.display()),
            (Some(_), _) => calibration.push(frame),
            (None, _) => set_aside.calibration_unattached.push(frame.display()),
        }
    }

    // The detection of a declared session: its range only.
    let mut other_nights = 0;
    if let Some(scope) = &request.scope {
        let from = parse_iso8601_utc_to_unix_seconds(&scope.from)
            .ok_or_else(|| DetectError::BadScope(scope.from.clone()))?;
        let to = parse_iso8601_utc_to_unix_seconds(&scope.to)
            .ok_or_else(|| DetectError::BadScope(scope.to.clone()))?;
        lights.retain(|light| {
            let inside = light.start >= from - 3600.0 && light.start <= to + 3600.0;
            if !inside {
                other_nights += 1;
            }
            inside
        });
    }

    // Nights, then places within a night.
    let default_site = clock_place(None, user).map(|(site, _)| site);
    let mut evenings: BTreeMap<String, Vec<Light<'_>>> = BTreeMap::new();
    for light in &lights {
        let site = light.frame.site.or(default_site);
        evenings
            .entry(evening_of(light.start, site, &user.places))
            .or_default()
            .push(*light);
    }
    let mut drafts: Vec<SessionDraft<'_>> = Vec::new();
    for (evening, lights) in evenings {
        drafts.extend(sessions_of_evening(&evening, lights));
    }

    // Targets, resolved once per kind of frame.
    let mut cache: HashMap<ResolveKey, Resolved> = HashMap::new();
    let mut resolve_cached = |frame: &FrameRecord| {
        let key = (
            frame.object.clone(),
            frame
                .ra_degrees
                .zip(frame.dec_degrees)
                .map(|(r, d)| (round_to(r, 0.05), round_to(d, 0.05))),
            frame.folders.clone(),
        );
        cache
            .entry(key)
            .or_insert_with(|| resolve(frame, catalogue))
            .clone()
    };

    let mut proposal = Proposal::default();
    let mut light_targets: Vec<(usize, String, Light<'_>)> = Vec::new();
    let mut evening_counts: BTreeMap<String, usize> = BTreeMap::new();
    for draft in &drafts {
        let n = evening_counts.entry(draft.evening.clone()).or_default();
        *n += 1;
        let session_id = format!("s-{}-{n}", draft.evening);
        let place = place_of(&mut proposal.places, draft.site, &session_id);
        let mut equipment: Vec<String> = Vec::new();
        // The frames that write the same `OBJECT` in a night are one target,
        // resolved at their mean position: a pointing that drifts, a
        // meridian flip or a second framing never splits it.
        let mut by_object: BTreeMap<&str, Vec<&FrameRecord>> = BTreeMap::new();
        for light in &draft.lights {
            if let Some(object) = light.frame.object.as_deref() {
                by_object.entry(object).or_default().push(light.frame);
            }
        }
        let representatives: HashMap<&str, FrameRecord> = by_object
            .into_iter()
            .map(|(object, frames)| {
                let mut representative = frames[0].clone();
                let position = mean_position(&frames);
                representative.ra_degrees = position.map(|p| p.0);
                representative.dec_degrees = position.map(|p| p.1);
                (object, representative)
            })
            .collect();
        let mut by_target: BTreeMap<String, (Resolved, Vec<Light<'_>>)> = BTreeMap::new();
        let mut frame_equipment: HashMap<*const FrameRecord, String> = HashMap::new();
        for light in &draft.lights {
            let id = equipment_of(&mut proposal.equipment, light.frame, &session_id);
            frame_equipment.insert(std::ptr::from_ref(light.frame), id.clone());
            if !equipment.contains(&id) {
                equipment.push(id);
            }
            let frame = light
                .frame
                .object
                .as_deref()
                .and_then(|o| representatives.get(o))
                .unwrap_or(light.frame);
            let resolved = resolve_cached(frame);
            by_target
                .entry(resolved.key.clone())
                .or_insert_with(|| (resolved, Vec::new()))
                .1
                .push(*light);
        }

        // One frame of a group without a target is solved, when a solver
        // exists here; else it is marked.
        let unsolved: Vec<String> = by_target
            .keys()
            .filter(|k| k.starts_with("unsolved:"))
            .cloned()
            .collect();
        let mut solve_frames: HashMap<String, String> = HashMap::new();
        let mut solve_failures: HashMap<String, SolveFailure> = HashMap::new();
        let mut solved_positions: HashMap<String, (f64, f64)> = HashMap::new();
        for key in unsolved {
            let first = by_target[&key].1[0].frame;
            let answer = options.solver.map(|solve| solve(&first.path));
            let solved = match &answer {
                Some(Ok((ra, dec))) => {
                    let found = nearest(catalogue, *ra, *dec, first.field_degrees);
                    if found.is_none() {
                        solved_positions.insert(key.clone(), (*ra, *dec));
                    }
                    found
                }
                Some(Err(failure)) => {
                    solve_failures.insert(key.clone(), failure.clone());
                    None
                }
                None => None,
            };
            if let Some((target, distance)) = solved {
                let (_, frames) = by_target.remove(&key).expect("key listed above");
                let resolved = Resolved::of(
                    target,
                    TargetSource::PlateSolved {
                        frame: first.display(),
                        distance_degrees: distance,
                    },
                );
                by_target
                    .entry(resolved.key.clone())
                    .or_insert_with(|| (resolved, Vec::new()))
                    .1
                    .extend(frames);
            } else {
                solve_frames.insert(key, first.display());
            }
        }

        let session_index = proposal.sessions.len();
        let mut targets = Vec::new();
        let mut band_guesses = Vec::new();
        for (key, (resolved, mut lights)) in by_target {
            lights.sort_by(|a, b| a.start.total_cmp(&b.start));
            band_guesses.extend(lights.iter().filter_map(|l| l.frame.band_guess.clone()));
            for light in &lights {
                light_targets.push((session_index, key.clone(), *light));
            }
            let filters = target_filters(request, &lights);
            let objects: BTreeSet<String> = lights
                .iter()
                .filter_map(|l| l.frame.object.clone())
                .collect();
            let instruments: Vec<String> = lights
                .iter()
                .map(|l| {
                    let id = &frame_equipment[&std::ptr::from_ref(l.frame)];
                    proposal
                        .equipment
                        .iter()
                        .find(|e| &e.id == id)
                        .map_or_else(|| l.frame.instrument(), equipment_label)
                })
                .collect();
            targets.push(DetectedTarget {
                key: key.clone(),
                target_id: resolved.target_id.clone(),
                designation: resolved.designation.clone(),
                name: resolved.name.clone(),
                source: resolved.source.clone(),
                objects: objects.into_iter().collect(),
                start: iso(lights[0].start),
                end: iso(lights.iter().map(|l| l.end).fold(f64::MIN, f64::max)),
                frames: lights.len(),
                exposure_seconds: lights
                    .iter()
                    .map(|l| l.frame.exposure_seconds.unwrap_or(0.0))
                    .sum(),
                filter_hint: filters
                    .is_empty()
                    .then(|| target_filter_hint(&lights))
                    .flatten(),
                filters: filters.into_iter().collect(),
                acquisition: acquisition_rows(&lights),
                instrument: most_common(instruments.iter().map(String::as_str)),
                solve_frame: solve_frames.get(&key).cloned(),
                solve_failure: solve_failures.get(&key).cloned(),
                solved_position: solved_positions.get(&key).copied(),
                device_images: Vec::new(),
                files: lights.iter().map(|l| l.frame.display()).collect(),
                file_times: lights.iter().map(|l| iso(l.start)).collect(),
                file_filters: if lights.iter().any(|l| l.frame.filter.is_some()) {
                    lights
                        .iter()
                        .map(|l| l.frame.filter.clone().unwrap_or_default())
                        .collect()
                } else {
                    Vec::new()
                },
                existing: super::TargetMatch::New,
            });
        }
        targets.sort_by(|a, b| a.start.cmp(&b.start));

        // The camera's clock, checked against the sky.
        let clock = clock_place(draft.site, user).map(|(site, used)| {
            let inputs: Vec<ClockInput> = light_targets
                .iter()
                .filter(|(s, _, _)| *s == session_index)
                .map(|(_, key, light)| {
                    let target = targets.iter().find(|t| &t.key == key);
                    let catalogue_position = target
                        .and_then(|t| t.target_id.as_deref())
                        .and_then(|id| catalogue.iter().find(|c| c.id == id))
                        .and_then(|c| Some((c.ra_hours? * 15.0, c.dec_degrees?)));
                    ClockInput {
                        mid: (light.start + light.end) / 2.0,
                        position: light
                            .frame
                            .ra_degrees
                            .zip(light.frame.dec_degrees)
                            .or(catalogue_position),
                        altitude_deg: light.frame.altitude_deg,
                        azimuth_deg: light.frame.azimuth_deg,
                        lst_hours: light.frame.lst_hours,
                    }
                })
                .collect();
            clock::evaluate(&inputs, site, used)
        });
        let clock = match clock {
            Some(finding) => finding,
            None => Some(clock::without_place()),
        };
        let shift_hours = clock
            .as_ref()
            .and_then(|c| c.proposed_shift_hours)
            .unwrap_or(0);
        let shift = f64::from(shift_hours) * 3600.0;
        let start = draft.lights[0].start;
        let end = draft.lights.iter().map(|l| l.end).fold(f64::MIN, f64::max);
        let site_for_night = draft.site.or(default_site);
        if shift_hours != 0 {
            for target in &mut targets {
                let times = [&mut target.start, &mut target.end]
                    .into_iter()
                    .chain(target.file_times.iter_mut());
                for time in times {
                    if let Some(t) = parse_iso8601_utc_to_unix_seconds(time) {
                        *time = iso(t + shift);
                    }
                }
            }
        }
        proposal.sessions.push(DetectedSession {
            id: session_id,
            evening_date: evening_of(start + shift, site_for_night, &user.places),
            written_evening_date: draft.evening.clone(),
            place,
            equipment,
            start: iso(start + shift),
            end: iso(end + shift),
            targets,
            clock,
            shift_hours,
            filter_guess: super::band::guess_for_night(&band_guesses),
            ..DetectedSession::default()
        });
    }

    // The device images go to the target and the night they say.
    for frame in device_frames {
        let taken = frame.date_obs.is_some_and(|start| {
            let evening = evening_of(start, frame.site.or(default_site), &user.places);
            let key = resolve_cached(frame).key;
            proposal
                .sessions
                .iter_mut()
                .filter(|session| session.written_evening_date == evening)
                .find_map(|session| session.targets.iter_mut().find(|target| target.key == key))
                .map(|target| target.device_images.push(frame.display()))
                .is_some()
        });
        if !taken {
            set_aside.stacks.push(frame.display());
        }
    }

    proposal.multi_night = multi_night_groups(&proposal.sessions);
    proposal.calibration_sets = calibration_sets(&calibration);
    attach_calibration(&mut proposal, &drafts, &mut set_aside);
    let offsets: BTreeSet<i32> = lights
        .iter()
        .filter_map(|l| l.frame.site)
        .filter_map(|site| place_near(&user.places, site)?.utc_offset_minutes)
        .collect();
    // The clocks a log may be in: the place's when it is known, else within
    // four hours of the mean solar time of the frames' longitude (no civil
    // time zone is further from it).
    let within = if offsets.len() == 1 {
        offsets
            .first()
            .map(|m| (i64::from(*m) * 60, i64::from(*m) * 60))
    } else {
        let longitudes: Vec<f64> = lights
            .iter()
            .filter_map(|l| l.frame.site.map(|s| s.1))
            .collect();
        let mean = (!longitudes.is_empty())
            .then(|| longitudes.iter().sum::<f64>() / longitudes.len() as f64);
        mean.map(|lon| {
            let solar = (lon / 15.0 * 2.0).round() as i64 * 1800;
            (solar - 4 * 3600, solar + 4 * 3600)
        })
    };
    attach_logs(
        &mut proposal,
        reading,
        &light_targets,
        &calibration,
        within,
        &mut set_aside,
    );

    if let Some(scope) = &request.scope {
        proposal.scoped = Some(scoped(
            scope,
            &proposal.sessions,
            &light_targets,
            other_nights,
        ));
    }

    let targets: BTreeSet<&str> = proposal
        .sessions
        .iter()
        .flat_map(|s| s.targets.iter().map(|t| t.key.as_str()))
        .collect();
    let nights: BTreeSet<&str> = proposal
        .sessions
        .iter()
        .map(|s| s.evening_date.as_str())
        .collect();
    proposal.reading = ReadingSummary {
        files_read: reading.files_read,
        lights: lights.len(),
        calibration_frames: calibration.len(),
        log_files: reading.logs.len(),
        nights: nights.len(),
        targets: targets.len(),
        links: reading.links.clone(),
    };
    proposal.set_aside = set_aside;
    Ok(Grouping { proposal })
}

/// The same target with the same instrument on nights at most
/// [`MULTI_NIGHT_GAP_DAYS`] apart.
fn multi_night_groups(sessions: &[DetectedSession]) -> Vec<MultiNightGroup> {
    let mut by_key: BTreeMap<(String, String), Vec<(&DetectedSession, &DetectedTarget)>> =
        BTreeMap::new();
    for session in sessions {
        for target in &session.targets {
            if target.key.starts_with("unsolved:") {
                continue;
            }
            by_key
                .entry((target.key.clone(), target.instrument.clone()))
                .or_default()
                .push((session, target));
        }
    }
    let mut groups = Vec::new();
    for ((key, instrument), mut nights) in by_key {
        nights.sort_by(|a, b| a.0.evening_date.cmp(&b.0.evening_date));
        let mut chain: Vec<(&DetectedSession, &DetectedTarget)> = Vec::new();
        let mut flush = |chain: &mut Vec<(&DetectedSession, &DetectedTarget)>| {
            let distinct: BTreeSet<&str> =
                chain.iter().map(|(s, _)| s.evening_date.as_str()).collect();
            if distinct.len() >= 2 {
                let first = chain[0].1;
                groups.push(MultiNightGroup {
                    id: String::new(),
                    target_key: key.clone(),
                    target_id: first.target_id.clone(),
                    designation: first.designation.clone(),
                    instrument: instrument.clone(),
                    nights: chain
                        .iter()
                        .map(|(s, t)| MultiNightNight {
                            session_id: s.id.clone(),
                            evening_date: s.evening_date.clone(),
                            frames: t.frames,
                            seconds: t.exposure_seconds,
                        })
                        .collect(),
                    goal_seconds: chain.iter().map(|(_, t)| t.exposure_seconds).sum(),
                    frames: chain.iter().map(|(_, t)| t.frames).sum(),
                    status: "finished".to_string(),
                    locked: false,
                });
            }
            chain.clear();
        };
        for night in nights {
            let gap = chain.last().and_then(|last| {
                Some(civil_days(&night.0.evening_date)? - civil_days(&last.0.evening_date)?)
            });
            if gap.is_some_and(|g| g > MULTI_NIGHT_GAP_DAYS) {
                flush(&mut chain);
            }
            chain.push(night);
        }
        flush(&mut chain);
    }
    for (index, group) in groups.iter_mut().enumerate() {
        group.id = format!("mn-{}", index + 1);
    }
    groups
}

/// How wide a set of bias frames may be in temperature, and how far the set's
/// mean temperature may be from a night's lights for the set to fit them (°C).
const BIAS_TEMPERATURE_SLACK: f64 = 1.0;

/// The cluster of each temperature: the temperatures sorted, a cluster
/// started at the first and holding every temperature within `width` of it.
/// Returns the index of the cluster of each input value (in input order).
fn temperature_clusters(values: &[f64], width: f64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|a, b| values[*a].total_cmp(&values[*b]));
    let mut clusters = vec![0; values.len()];
    let mut current = 0;
    let mut start = f64::NAN;
    for index in order {
        if start.is_nan() {
            start = values[index];
        } else if values[index] - start > width {
            current += 1;
            start = values[index];
        }
        clusters[index] = current;
    }
    clusters
}

/// Calibration frames of one kind, folder, exposure, gain, temperature and
/// filter taken together.
///
/// Two kinds are grouped by what matters to them: a **flat** set ignores the
/// exposure (a sky flat's exposure follows the light, so it varies by
/// construction) and the temperature, and keeps the filter and the gain; a
/// **bias** set ignores the rounding of the temperature (frames within
/// [`BIAS_TEMPERATURE_SLACK`] of each other stay together, `-5 °C` and
/// `-4.5 °C` included) and keeps the gain. Darks and dark-flats are keyed as
/// before.
fn calibration_sets(frames: &[&FrameRecord]) -> Vec<CalibrationSet> {
    type Key = (FrameType, String, i64, i64, i64, String, String);
    let mut sets: BTreeMap<Key, Vec<&FrameRecord>> = BTreeMap::new();
    // The temperature clusters of the bias frames, within what else the key
    // holds (folder, exposure, gain, filter, instrument).
    let mut bias_groups: BTreeMap<(String, i64, i64, String, String), Vec<usize>> = BTreeMap::new();
    for (at, frame) in frames.iter().enumerate() {
        if frame.kind == Some(FrameType::Bias) && frame.temperature_c.is_some() {
            let group = (
                frame
                    .path
                    .parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                frame.exposure_seconds.map_or(-1, |e| round_to(e, 0.001)),
                frame.gain.map_or(-1, |g| round_to(g, 1.0)),
                frame.filter.clone().unwrap_or_default(),
                frame.instrume.clone().unwrap_or_default(),
            );
            bias_groups.entry(group).or_default().push(at);
        }
    }
    let mut bias_cluster: Vec<Option<usize>> = vec![None; frames.len()];
    for members in bias_groups.values() {
        let temperatures: Vec<f64> = members
            .iter()
            .filter_map(|at| frames[*at].temperature_c)
            .collect();
        for (at, cluster) in members
            .iter()
            .zip(temperature_clusters(&temperatures, BIAS_TEMPERATURE_SLACK))
        {
            bias_cluster[*at] = Some(cluster);
        }
    }
    for (at, frame) in frames.iter().enumerate() {
        let Some(kind) = frame.kind else { continue };
        let folder = frame
            .path
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let (exposure, temperature) = match kind {
            FrameType::Flat => (-1, i64::MIN),
            FrameType::Bias => (
                frame.exposure_seconds.map_or(-1, |e| round_to(e, 0.001)),
                bias_cluster[at].map_or(i64::MIN, |c| c as i64),
            ),
            _ => (
                frame.exposure_seconds.map_or(-1, |e| round_to(e, 0.001)),
                frame.temperature_c.map_or(i64::MIN, |t| round_to(t, 2.0)),
            ),
        };
        let key = (
            kind,
            folder,
            exposure,
            frame.gain.map_or(-1, |g| round_to(g, 1.0)),
            temperature,
            frame.filter.clone().unwrap_or_default(),
            frame.instrume.clone().unwrap_or_default(),
        );
        sets.entry(key).or_default().push(frame);
    }
    let mut out: Vec<CalibrationSet> = sets
        .into_iter()
        .map(|((kind, folder, ..), frames)| {
            let dates: Vec<f64> = frames.iter().filter_map(|f| f.date_obs).collect();
            let mean = |values: Vec<f64>| {
                (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
            };
            CalibrationSet {
                id: String::new(),
                kind: Some(kind),
                kind_from_folder: frames.iter().any(|f| f.kind_from_folder),
                folder,
                frames: frames.len(),
                exposure_seconds: mean(frames.iter().filter_map(|f| f.exposure_seconds).collect()),
                gain: frames.iter().find_map(|f| f.gain),
                temperature_c: mean(frames.iter().filter_map(|f| f.temperature_c).collect()),
                filter: frames.iter().find_map(|f| f.filter.clone()),
                instrument: frames.iter().find_map(|f| f.instrume.clone()),
                start: dates.iter().copied().reduce(f64::min).map(iso),
                end: dates.iter().copied().reduce(f64::max).map(iso),
                files: frames.iter().map(|f| f.display()).collect(),
            }
        })
        .collect();
    out.sort_by(|a, b| (a.kind, &a.start).cmp(&(b.kind, &b.start)));
    for (index, set) in out.iter_mut().enumerate() {
        set.id = format!("cal-{}", index + 1);
    }
    out
}

/// Days between a set's frames and the night's lights (0 when they overlap).
fn days_away(set: &CalibrationSet, span: (f64, f64)) -> Option<f64> {
    let start = parse_iso8601_utc_to_unix_seconds(set.start.as_deref()?)?;
    let end = parse_iso8601_utc_to_unix_seconds(set.end.as_deref()?)?;
    let gap = if end < span.0 {
        span.0 - end
    } else if start > span.1 {
        start - span.1
    } else {
        0.0
    };
    Some(gap / 86_400.0)
}

fn same(a: Option<f64>, b: Option<f64>, slack: f64) -> Option<bool> {
    Some((a? - b?).abs() <= slack)
}

/// The calibration of each night: darks of the lights' exposure, gain and
/// temperature; flats of their gain and filter; dark-flats of the flats'
/// exposure; bias of the gain and temperature; the nearest in time, at most
/// [`CALIBRATION_MAX_DAYS`] away.
fn attach_calibration(
    proposal: &mut Proposal,
    drafts: &[SessionDraft<'_>],
    set_aside: &mut SetAside,
) {
    let sets = &proposal.calibration_sets;
    let mut used: BTreeSet<String> = BTreeSet::new();
    // Per night: each family of poses and whether a dark and a flat were
    // attached to it (the notes below).
    let mut results: Vec<Vec<(&FrameRecord, bool, bool)>> = Vec::new();
    for (session, draft) in proposal.sessions.iter_mut().zip(drafts) {
        let span = (
            draft.lights[0].start,
            draft.lights.iter().map(|l| l.end).fold(f64::MIN, f64::max),
        );
        // The families of the night's lights.
        let mut families: Vec<&FrameRecord> = Vec::new();
        for light in &draft.lights {
            let f = light.frame;
            let known = families.iter().any(|g| {
                same(g.exposure_seconds, f.exposure_seconds, 0.001).unwrap_or(true)
                    && g.gain == f.gain
                    && g.filter == f.filter
                    && g.instrume == f.instrume
            });
            if !known {
                families.push(f);
            }
        }
        let fits = |set: &CalibrationSet, light: &FrameRecord, matched: &mut Vec<String>| {
            if let (Some(a), Some(b)) = (&set.instrument, &light.instrume)
                && a != b
            {
                return false;
            }
            if same(set.gain, light.gain, 0.5) == Some(false) {
                return false;
            }
            if set.gain.is_some() && light.gain.is_some() {
                matched.push("gain".into());
            }
            true
        };
        let best = |kind: FrameType,
                    test: &dyn Fn(&CalibrationSet, &mut Vec<String>) -> bool|
         -> Option<(usize, AttachedCalibration)> {
            sets.iter()
                .enumerate()
                .filter(|(_, s)| s.kind == Some(kind))
                .filter_map(|(i, s)| {
                    let mut matched = Vec::new();
                    if !test(s, &mut matched) {
                        return None;
                    }
                    let days = days_away(s, span)?;
                    (days <= CALIBRATION_MAX_DAYS).then(|| {
                        matched.push("date".into());
                        (
                            i,
                            AttachedCalibration {
                                set_id: s.id.clone(),
                                kind,
                                days_away: (days * 10.0).round() / 10.0,
                                matched_on: matched,
                            },
                        )
                    })
                })
                .min_by(|a, b| a.1.days_away.total_cmp(&b.1.days_away))
        };
        let mut attached: Vec<AttachedCalibration> = Vec::new();
        let push = |found: Option<(usize, AttachedCalibration)>,
                    attached: &mut Vec<AttachedCalibration>|
         -> Option<usize> {
            let (index, item) = found?;
            if !attached.iter().any(|a| a.set_id == item.set_id) {
                attached.push(item);
            }
            Some(index)
        };
        let mut family_results: Vec<(&FrameRecord, bool, bool)> = Vec::new();
        for light in &families {
            let temperature_within = |slack: f64| {
                move |set: &CalibrationSet, matched: &mut Vec<String>| match same(
                    set.temperature_c,
                    light.temperature_c,
                    slack,
                ) {
                    Some(false) => false,
                    Some(true) => {
                        matched.push("temperature".into());
                        true
                    }
                    None => true,
                }
            };
            let temperature = temperature_within(2.5);
            let bias_temperature = temperature_within(BIAS_TEMPERATURE_SLACK);
            let dark = best(FrameType::Dark, &|s, m| {
                fits(s, light, m)
                    && temperature(s, m)
                    && match (s.exposure_seconds, light.exposure_seconds) {
                        (Some(a), Some(b)) if (a - b).abs() <= (0.01 * b).max(0.5) => {
                            m.push("exposure".into());
                            true
                        }
                        _ => false,
                    }
            });
            let has_dark = push(dark, &mut attached).is_some();
            let flat = best(FrameType::Flat, &|s, m| {
                fits(s, light, m)
                    && match (&s.filter, &light.filter) {
                        (Some(a), Some(b)) if a == b => {
                            m.push("filter".into());
                            true
                        }
                        (Some(_), Some(_)) => false,
                        _ => true,
                    }
            });
            let flat_index = push(flat, &mut attached);
            family_results.push((light, has_dark, flat_index.is_some()));
            if let Some(index) = flat_index {
                let flat_exposure = sets[index].exposure_seconds;
                let dark_flat = best(FrameType::DarkFlat, &|s, m| {
                    fits(s, light, m)
                        && temperature(s, m)
                        && match (s.exposure_seconds, flat_exposure) {
                            (Some(a), Some(b)) if (a - b).abs() <= (0.05 * b).max(0.01) => {
                                m.push("exposure".into());
                                true
                            }
                            _ => false,
                        }
                });
                push(dark_flat, &mut attached);
            }
            let bias = best(FrameType::Bias, &|s, m| {
                fits(s, light, m) && bias_temperature(s, m)
            });
            push(bias, &mut attached);
        }
        attached.sort_by(|a, b| a.kind.cmp(&b.kind));
        used.extend(attached.iter().map(|a| a.set_id.clone()));
        session.calibration = attached;
        results.push(family_results);
    }
    for (session, family_results) in proposal.sessions.iter_mut().zip(results) {
        session.calibration_notes = calibration_notes(sets, &family_results);
    }
    for set in &proposal.calibration_sets {
        if !used.contains(&set.id) {
            set_aside
                .calibration_unattached
                .extend(set.files.iter().cloned());
        }
    }
}

/// What the families of a night's poses lack in calibration: for each family
/// without an attached dark, the darks that exist (of the instrument) and do
/// not fit, or none at all; the same for the flats of its filter. One note
/// per cause, however many families share it.
fn calibration_notes(
    sets: &[CalibrationSet],
    families: &[(&FrameRecord, bool, bool)],
) -> Vec<CalibrationNote> {
    let of_instrument = |set: &CalibrationSet, light: &FrameRecord| !matches!((&set.instrument, &light.instrume), (Some(a), Some(b)) if a != b);
    let mut notes: Vec<CalibrationNote> = Vec::new();
    for (light, has_dark, has_flat) in families {
        if !has_dark && let Some(exposure) = light.exposure_seconds {
            let darks: Vec<&CalibrationSet> = sets
                .iter()
                .filter(|s| s.kind == Some(FrameType::Dark) && of_instrument(s, light))
                .collect();
            let note = if darks.is_empty() {
                CalibrationNote::NoDarkAtAll {
                    exposure_seconds: exposure,
                }
            } else {
                let mut found: Vec<f64> = darks
                    .iter()
                    .filter_map(|s| s.exposure_seconds)
                    .map(|e| (e * 1000.0).round() / 1000.0)
                    .collect();
                found.sort_by(f64::total_cmp);
                found.dedup();
                CalibrationNote::NoDark {
                    exposure_seconds: exposure,
                    count: darks.iter().map(|s| s.frames).sum(),
                    found_seconds: found,
                }
            };
            // A smart telescope calibrates on board: no dark is no omission.
            let silent = darks.is_empty() && light.calibrates_itself();
            if !silent && !notes.contains(&note) {
                notes.push(note);
            }
        }
        if !has_flat {
            let any_flat = sets
                .iter()
                .any(|s| s.kind == Some(FrameType::Flat) && of_instrument(s, light));
            let filter = light.filter.clone();
            let note = if any_flat {
                CalibrationNote::NoFlat { filter }
            } else {
                CalibrationNote::NoFlatAtAll { filter }
            };
            let silent = !any_flat && light.calibrates_itself();
            if !silent && !notes.contains(&note) {
                notes.push(note);
            }
        }
    }
    notes
}

/// The instant every line of a log starts with (`2026/08/07 22:46:00`,
/// `2026-08-07 22:46:00`, `2026-08-07T22:46:00`), read as UTC.
fn line_instants(contents: &str) -> Vec<f64> {
    contents
        .lines()
        .filter_map(|line| {
            let head = line.get(..19)?;
            let b = head.as_bytes();
            if !(b[4] == b'/' || b[4] == b'-') || !(b[10] == b' ' || b[10] == b'T') {
                return None;
            }
            let iso = format!(
                "{}-{}-{}T{}Z",
                &head[..4],
                &head[5..7],
                &head[8..10],
                &head[11..19]
            );
            parse_iso8601_utc_to_unix_seconds(&iso)
        })
        .collect()
}

/// A log whose events say too little (a night without dither): the frames
/// inside the span of its lines, at the place's clock when it is known,
/// else at the whole hour that holds the most (the nearest to UTC on a
/// tie).
fn span_match(
    lines: &[f64],
    frames: &[(f64, f64)],
    within: Option<(i64, i64)>,
) -> Option<LogMatch> {
    let first = lines.iter().copied().reduce(f64::min)?;
    let last = lines.iter().copied().reduce(f64::max)?;
    let inside = |shift: i64| -> Vec<usize> {
        let (from, to) = (first - shift as f64, last - shift as f64);
        frames
            .iter()
            .enumerate()
            .filter(|(_, (start, _))| *start >= from && *start <= to)
            .map(|(i, _)| i)
            .collect()
    };
    let (low, high) = within.unwrap_or((-14 * 3600, 14 * 3600));
    let centre = (low + high) / 2;
    let mut shifts: Vec<i64> = (-14..=14)
        .map(|h| h * 3600)
        .chain([low, high])
        .filter(|shift| (low..=high).contains(shift))
        .collect();
    shifts.sort_unstable();
    shifts.dedup();
    shifts.sort_by_key(|shift| ((shift - centre).abs(), *shift < centre));
    let mut best: Option<LogMatch> = None;
    for shift in shifts {
        let frames = inside(shift);
        if !frames.is_empty() && best.as_ref().is_none_or(|b| frames.len() > b.frames.len()) {
            best = Some(LogMatch {
                shift_seconds: shift,
                frames,
            });
        }
    }
    best
}

/// The instants of a log, read as UTC (its clock is found afterwards).
fn log_instants(kind: LogKind, contents: &str) -> Vec<f64> {
    let parse = |t: &str| parse_iso8601_utc_to_unix_seconds(t);
    match kind {
        LogKind::Phd2 => parse_phd2_guide_log(contents, &Phd2Options::default())
            .map(|log| {
                log.sessions
                    .iter()
                    .flat_map(|s| s.samples.iter().filter_map(|x| parse(&x.timestamp_utc)))
                    .collect()
            })
            .unwrap_or_default(),
        LogKind::Asiair => parse_asiair_autorun_log(contents, &AsiairOptions::default())
            .map(|log| {
                log.dither_events
                    .iter()
                    .map(|e| &e.timestamp_utc)
                    .chain(log.settle_events.iter().map(|e| &e.timestamp_utc))
                    .chain(log.settle_timeout_events.iter().map(|e| &e.timestamp_utc))
                    .chain(log.autofocus_events.iter().map(|e| &e.timestamp_utc))
                    .chain(log.autofocus_result_events.iter().map(|e| &e.timestamp_utc))
                    .chain(log.meridian_flip_events.iter().map(|e| &e.timestamp_utc))
                    .filter_map(|t| parse(t))
                    .collect()
            })
            .unwrap_or_default(),
        LogKind::Nina => parse_nina_log(contents, &NinaOptions::default())
            .map(|log| log.exposures.iter().map(|e| e.unix_seconds).collect())
            .unwrap_or_default(),
        LogKind::Unknown => Vec::new(),
    }
}

/// Each log tied to the lights its instants fall on, its clock found
/// (`crate::analysis::log_offset::match_log`): a log shared by two targets
/// of a night is attached to both.
fn attach_logs(
    proposal: &mut Proposal,
    reading: &Reading,
    lights: &[(usize, String, Light<'_>)],
    calibration: &[&FrameRecord],
    within: Option<(i64, i64)>,
    set_aside: &mut SetAside,
) {
    // The calibration frames take part: a log of the morning's darks lines
    // up with them, not with the night's lights at some other clock.
    let mut spans: Vec<(f64, f64)> = lights.iter().map(|(_, _, l)| (l.start, l.end)).collect();
    spans.extend(calibration.iter().filter_map(|f| {
        let start = f.date_obs?;
        Some((start, start + f.exposure_seconds.unwrap_or(0.0)))
    }));
    let is_light = |frame: usize| frame < lights.len();
    for (index, log) in reading.logs.iter().enumerate() {
        let id = format!("log-{}", index + 1);
        let path = log.path.display().to_string();
        let contents = std::fs::read(&log.path)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        let instants = log_instants(log.kind, &contents);
        let clock = match log.kind {
            LogKind::Phd2 => LogClock::Phd2,
            LogKind::Asiair => LogClock::Asiair,
            _ => LogClock::Nina,
        };
        // Mostly lights, two at least: else a log of calibration or set-up.
        let of_lights = |m: &LogMatch| {
            let lit = m.frames.iter().filter(|f| is_light(**f)).count();
            lit >= 2 && lit * 2 > m.frames.len()
        };
        let lines = line_instants(&contents);
        // A log known only by the span of its lines must cover most of the
        // lights of the nights it touches (a short log of the morning's darks
        // sits inside a night at some other clock).
        let covers = |m: &LogMatch| {
            let touched: BTreeSet<usize> = m
                .frames
                .iter()
                .filter(|f| is_light(**f))
                .map(|f| lights[*f].0)
                .collect();
            let total = lights
                .iter()
                .filter(|(s, _, _)| touched.contains(s))
                .count();
            let lit = m.frames.iter().filter(|f| is_light(**f)).count();
            lit * 2 >= total
        };
        let found = match_log(clock, &instants, &spans, within)
            .filter(of_lights)
            .or_else(|| {
                span_match(&lines, &spans, within)
                    .filter(of_lights)
                    .filter(covers)
            })
            .map(|mut m| {
                m.frames.retain(|f| is_light(*f));
                m
            });
        let kind = match log.kind {
            LogKind::Phd2 => "phd2",
            LogKind::Asiair => "asiair",
            LogKind::Nina => "nina",
            LogKind::Unknown => "unknown",
        };
        let instants = if instants.is_empty() { lines } else { instants };
        let Some(found) = found else {
            set_aside.logs_unattached.push(path.clone());
            let first = instants.iter().copied().reduce(f64::min);
            let last = instants.iter().copied().reduce(f64::max);
            proposal.logs.push(DetectedLogFile {
                id,
                path,
                kind: kind.to_string(),
                start: first.map(iso),
                end: last.map(iso),
                ..DetectedLogFile::default()
            });
            continue;
        };
        let shift = found.shift_seconds as f64;
        let mut per_session: BTreeMap<usize, (BTreeSet<String>, usize)> = BTreeMap::new();
        for frame in &found.frames {
            let (session, key, _) = &lights[*frame];
            let entry = per_session.entry(*session).or_default();
            entry.0.insert(key.clone());
            entry.1 += 1;
        }
        let mut sessions = Vec::new();
        for (session, (targets, matched)) in per_session {
            let session = &mut proposal.sessions[session];
            sessions.push(session.id.clone());
            session.logs.push(AttachedLog {
                log_id: id.clone(),
                targets: targets.into_iter().collect(),
                matched_frames: matched,
            });
        }
        proposal.logs.push(DetectedLogFile {
            id,
            path,
            kind: kind.to_string(),
            utc_offset: Some(format_offset(found.shift_seconds)),
            start: instants
                .iter()
                .copied()
                .reduce(f64::min)
                .map(|t| iso(t - shift)),
            end: instants
                .iter()
                .copied()
                .reduce(f64::max)
                .map(|t| iso(t - shift)),
            matched_frames: found.frames.len(),
            sessions,
        });
    }
}

/// The detection sheet of a declared session: its targets, each with its
/// frames; what matches none of them; what has neither `OBJECT` nor
/// coordinates.
fn scoped(
    scope: &super::SessionScope,
    sessions: &[DetectedSession],
    lights: &[(usize, String, Light<'_>)],
    other_nights: usize,
) -> ScopedDetection {
    let mut out = ScopedDetection {
        targets: scope
            .targets
            .iter()
            .map(|t| ScopedTarget {
                id: t.id.clone(),
                designation: t.designation.clone(),
                files: Vec::new(),
            })
            .collect(),
        other_nights,
        ..ScopedDetection::default()
    };
    let mut unassigned: BTreeMap<String, UnassignedGroup> = BTreeMap::new();
    for (session, key, light) in lights {
        let frame = light.frame;
        let target = sessions[*session].targets.iter().find(|t| &t.key == key);
        let matches = |wanted: &super::ScopeTarget| {
            let by_id = target.and_then(|t| t.target_id.as_deref()) == Some(wanted.id.as_str());
            let named = |text: &str| {
                let text = loose(text);
                std::iter::once(&wanted.designation)
                    .chain(wanted.aliases.iter())
                    .any(|n| loose(n) == text)
            };
            let by_name = target.is_some_and(|t| named(&t.designation))
                || frame.object.as_deref().is_some_and(named);
            let by_position = match (
                frame.ra_degrees.zip(frame.dec_degrees),
                wanted.ra_hours.zip(wanted.dec_degrees),
            ) {
                (Some((ra, dec)), Some((h, d))) => {
                    angular_distance_deg(ra, dec, h * 15.0, d)
                        <= field_radius(frame.field_degrees) + 0.5
                }
                _ => false,
            };
            by_id || by_name || by_position
        };
        if let Some(index) = scope.targets.iter().position(matches) {
            out.targets[index].files.push(frame.display());
        } else if frame.object.is_none() && frame.ra_degrees.is_none() {
            out.without_object.push(frame.display());
        } else {
            let group = unassigned
                .entry(key.clone())
                .or_insert_with(|| UnassignedGroup {
                    key: key.clone(),
                    designation: target.map(|t| t.designation.clone()).unwrap_or_default(),
                    target_id: target.and_then(|t| t.target_id.clone()),
                    files: Vec::new(),
                });
            group.files.push(frame.display());
        }
    }
    out.unassigned = unassigned.into_values().collect();
    out
}

//! "Create…": the proposal as the person ticked it, turned into the body of the
//! service's `POST /v1/sessions/bulk` (`session-bulk.schema.json`), and the
//! service's answer turned back into what the result screen draws. Pure but for
//! [`create_sessions`], the one call. Nothing outside `ticked` is sent; the
//! identifiers the service makes derive from `importId`, itself derived from
//! the body: sending the same choices again (a retry after a cut connection)
//! replays, never duplicates.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::catalogue_targets::{PersonalTargetCreate, personal_target_body};
use crate::gui_bridge::{GuiBridgeError, HttpPortalGuiTransport};
use crate::service_wait::RequestFactory;
use crate::time_util::{format_iso8601_utc_millis, parse_iso8601_utc_to_unix_seconds};

use super::{
    DetectedSession, DetectedTarget, MultiNightGroup, PlaceAction, Proposal, SessionMatch,
    TargetMatch, TargetSource,
};

/// The answer the person gave for one place of the proposal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum PlaceAnswer {
    /// One of "My places" (the nearest ticked by default).
    #[serde(rename_all = "camelCase")]
    Known {
        place_id: String,
        known_place_id: String,
    },
    /// "Create this place": a new place at the frames' coordinates.
    #[serde(rename_all = "camelCase")]
    Create {
        place_id: String,
        name: String,
        latitude: f64,
        longitude: f64,
        time_zone: Option<String>,
    },
}

/// What a line that asks (two equal choices) was answered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineAnswer {
    pub line_id: String,
    /// `add` or `skip`; a line without answer is not added.
    pub answer: String,
}

/// The whole hours the person chose for a session whose clock is suspect
/// (applied to the session, never to the files; 0: none).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionShift {
    pub session_id: String,
    pub hours: i32,
}

/// "Create…": the proposal as the person ticked it. Nothing outside
/// `ticked` is written; the detected ids are the proposal's own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkCreateRequest {
    pub proposal: Proposal,
    /// The ids of the `ProposalLine`s left ticked.
    pub ticked: Vec<String>,
    #[serde(default)]
    pub places: Vec<PlaceAnswer>,
    #[serde(default)]
    pub answers: Vec<LineAnswer>,
    #[serde(default)]
    pub shifts: Vec<SessionShift>,
}

/// The name a place created from the frames takes when the person gives none
/// and the caller has no word of its own for it (the command line takes its
/// word from the dictionaries, `detect.place.defaultName`). A word, never the
/// position of the frames: a place named by its coordinates would say where
/// the person lives wherever its name is read.
pub const DEFAULT_PLACE_NAME: &str = "Observing site";

impl BulkCreateRequest {
    /// [`Self::as_proposed_named`] with [`DEFAULT_PLACE_NAME`].
    #[must_use]
    pub fn as_proposed(proposal: Proposal) -> Self {
        Self::as_proposed_named(proposal, DEFAULT_PLACE_NAME)
    }

    /// The proposal as it comes, the lines ticked by default, the nearest
    /// place, the proposed clock shift: what the command line sends when the
    /// person gives no choice of their own. A place to create is named
    /// `place_name`.
    #[must_use]
    pub fn as_proposed_named(proposal: Proposal, place_name: &str) -> Self {
        let ticked = proposal
            .lines
            .iter()
            .filter(|l| l.checked && !l.locked)
            .map(|l| l.id.clone())
            .collect();
        let shifts = proposal
            .sessions
            .iter()
            .filter(|s| s.shift_hours != 0)
            .map(|s| SessionShift {
                session_id: s.id.clone(),
                hours: s.shift_hours,
            })
            .collect();
        // "Create this place", ticked by the engine: the place is created, under
        // a neutral name, instead of the night going without a place in
        // silence A place the frames do not name
        // cannot be created: its nights are said to lack one (`bulk_body`).
        let places = proposal
            .places
            .iter()
            .filter(|place| place.action == PlaceAction::Create)
            .filter(|place| {
                let id = format!("place.create:{}", place.id);
                proposal
                    .lines
                    .iter()
                    .any(|l| l.id == id && l.checked && !l.locked)
            })
            .filter_map(|place| {
                let (latitude, longitude) = (place.latitude?, place.longitude?);
                Some(PlaceAnswer::Create {
                    place_id: place.id.clone(),
                    name: place_name.to_string(),
                    latitude,
                    longitude,
                    time_zone: None,
                })
            })
            .collect();
        Self {
            proposal,
            ticked,
            places,
            answers: Vec::new(),
            shifts,
        }
    }
}

/// What became of one session of the proposal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedSession {
    /// The proposal's id (`DetectedSession::id`).
    pub detected_id: String,
    /// The session on the service.
    pub session_id: String,
    pub evening_date: String,
    /// `created` ("Imported by Astro Sync"), `updated` (an existing
    /// session that received poses or a target).
    pub action: String,
    /// `created`, `reused` (a planned plan, now "Executed"), `replaced`,
    /// `none` (an existing session).
    pub plan_action: String,
    pub plan_id: Option<String>,
    pub designations: Vec<String>,
    pub frames: usize,
    pub shift_hours: i32,
    /// The targets added to an existing session ("Sh2-103 added to the
    /// session of 11 August").
    #[serde(default)]
    pub added_targets: Vec<String>,
}

/// A multi-night plan created (born "Completed", goal = the total acquired).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedMultiNight {
    pub group_id: String,
    pub plan_id: String,
    pub designation: String,
    pub session_ids: Vec<String>,
    pub goal_seconds: f64,
}

/// An element the service, or Astro Sync before sending, did not write, and
/// why (the service's code, or `target_unknown` / `no_target` here).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedElement {
    /// `session`, `target`, `multi-night`, `place`.
    pub kind: String,
    /// The proposal's id of the element.
    pub id: String,
    pub reason: String,
}

/// What was created or updated on the service.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkCreateOutcome {
    pub sessions: Vec<CreatedSession>,
    pub multi_night: Vec<CreatedMultiNight>,
    /// The ids of the places created.
    pub places_created: Vec<String>,
    /// What was not written, with its reason (partial results).
    #[serde(default)]
    pub skipped: Vec<SkippedElement>,
    /// The same choices were already sent: nothing new was written.
    #[serde(default)]
    pub replayed: bool,
}

/// The body and what Astro Sync left out before sending.
#[derive(Debug, Clone, PartialEq)]
pub struct BulkBody {
    pub body: Value,
    pub skipped: Vec<SkippedElement>,
}

fn shifted(iso: &str, seconds: f64) -> String {
    if seconds == 0.0 {
        return iso.to_string();
    }
    parse_iso8601_utc_to_unix_seconds(iso)
        .map(|t| format_iso8601_utc_millis(t + seconds))
        .unwrap_or_else(|| iso.to_string())
}

/// The exposure of one frame: the row of the rebuilt plan that gathered the
/// most time.
fn frame_exposure(target: &DetectedTarget) -> Option<f64> {
    target
        .acquisition
        .iter()
        .filter(|r| r.exposure_seconds.is_some())
        .max_by(|a, b| a.total_seconds.total_cmp(&b.total_seconds))
        .and_then(|r| r.exposure_seconds)
        .filter(|e| *e > 0.0)
}

fn target_body(target: &DetectedTarget, offset: f64) -> Option<Value> {
    let target_id = target.target_id.as_ref()?;
    // The header's filter, else the one the folders name; the service keeps
    // a filter name of 40 characters at most.
    let filter = match target.filters.as_slice() {
        [one] => Some(one.chars().take(40).collect::<String>()),
        [] => target
            .filter_hint
            .as_ref()
            .map(|name| name.chars().take(40).collect::<String>()),
        _ => None,
    };
    Some(json!({
        "targetId": target_id,
        "designation": target.designation,
        "name": shown_name(target),
        "startedAt": shifted(&target.start, offset),
        "finishedAt": shifted(&target.end, offset),
        "frameCount": target.frames,
        "exposureSeconds": frame_exposure(target),
        "filter": filter,
    }))
}

/// A designation folded to be compared: « Sh2-190 », « SH2 190 » and « sh2190 »
/// are one name; « NGC 0224 » and « NGC224 » too.
fn folded(text: &str) -> String {
    let mut out = String::new();
    let mut after_letter = false;
    for c in text.chars().filter(|c| c.is_alphanumeric()) {
        if c == '0' && after_letter {
            continue;
        }
        after_letter = c.is_alphabetic();
        out.extend(c.to_uppercase());
    }
    out
}

/// The name a target is created under: the catalogue's common name, followed
/// by the name the frames call it when that is none of its names -- « Heart
/// Nebula · SH2-190 » for lights whose `OBJECT` says SH2-190 and a catalogue
/// that knows the object as IC 1805. The designation stays the catalogue's;
/// the person's own word is not lost between their folders and the pages.
fn shown_name(target: &DetectedTarget) -> Option<String> {
    let known = [Some(&target.designation), target.name.as_ref()]
        .into_iter()
        .flatten()
        .map(|name| folded(name))
        .collect::<Vec<_>>();
    let own = target
        .objects
        .iter()
        .map(|object| object.trim())
        .find(|object| !object.is_empty() && !known.contains(&folded(object)));
    let name = match (target.name.as_deref().map(str::trim), own) {
        (Some(common), Some(own)) if !common.is_empty() => format!("{common} · {own}"),
        (_, Some(own)) => own.to_string(),
        (common, None) => return common.filter(|c| !c.is_empty()).map(str::to_string),
    };
    Some(name.chars().take(300).collect())
}

fn equipment_of(proposal: &Proposal, session: &DetectedSession) -> Option<Value> {
    let item = proposal
        .equipment
        .iter()
        .find(|e| session.equipment.contains(&e.id))?;
    let mut equipment = serde_json::Map::new();
    equipment.insert("mode".into(), json!("imaging"));
    for (key, matched) in [
        ("cameraId", &item.camera),
        ("telescopeId", &item.telescope),
        ("mountId", &item.mount),
    ] {
        if let Some(m) = matched {
            equipment.insert(key.into(), json!(m.equipment_id));
        }
    }
    // The filters the frames name that "My equipment" holds.
    if !item.filter_matches.is_empty() {
        let ids: Vec<&str> = item
            .filter_matches
            .iter()
            .map(|m| m.equipment_id.as_str())
            .collect();
        equipment.insert("filterIds".into(), json!(ids));
    }
    (equipment.len() > 1).then_some(Value::Object(equipment))
}

/// A ULID-shaped key from the bytes of the body: the same choices, the same
/// key (the service's idempotency).
fn import_id_of(body: &Value) -> String {
    const CROCKFORD: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let digest = Sha256::digest(crate::canonical_json::canonical_json(body).as_bytes());
    let mut value: u128 = 0;
    for byte in &digest[..16] {
        value = (value << 8) | u128::from(*byte);
    }
    let mut out = [0u8; 26];
    for slot in out.iter_mut().rev() {
        *slot = CROCKFORD[(value % 32) as usize];
        value /= 32;
    }
    // The first character of a ULID is 0 to 7.
    out[0] = CROCKFORD[(out[0] as char).to_digit(32).unwrap_or(0) as usize % 8];
    String::from_utf8_lossy(&out).into_owned()
}

/// The body of `POST /v1/sessions/bulk` for the request.
#[must_use]
pub fn bulk_body(request: &BulkCreateRequest) -> BulkBody {
    let proposal = &request.proposal;
    let ticked: BTreeSet<&str> = request.ticked.iter().map(String::as_str).collect();
    let answer = |line_id: &str| {
        request
            .answers
            .iter()
            .find(|a| a.line_id == line_id)
            .map(|a| a.answer.as_str())
    };
    let mut skipped = Vec::new();
    let mut places = Vec::new();
    let mut place_of: BTreeMap<&str, Value> = BTreeMap::new();
    for place in &proposal.places {
        let given = request.places.iter().find(|a| match a {
            PlaceAnswer::Known { place_id, .. } | PlaceAnswer::Create { place_id, .. } => {
                place_id == &place.id
            }
        });
        match given {
            Some(PlaceAnswer::Known { known_place_id, .. }) => {
                place_of.insert(&place.id, json!({ "placeId": known_place_id }));
            }
            Some(PlaceAnswer::Create {
                name,
                latitude,
                longitude,
                time_zone,
                ..
            }) => {
                places.push(json!({
                    "ref": place.id,
                    "name": name,
                    "latitude": latitude,
                    "longitude": longitude,
                    "timeZone": time_zone,
                }));
                place_of.insert(&place.id, json!({ "ref": place.id }));
            }
            None => {
                if let Some(chosen) = &place.chosen {
                    place_of.insert(&place.id, json!({ "placeId": chosen }));
                }
            }
        }
    }

    let mut sessions = Vec::new();
    let mut sent: BTreeSet<&str> = BTreeSet::new();
    for session in &proposal.sessions {
        let (reason, plan) = match &session.existing {
            SessionMatch::Create => ("session.create", Some(json!({ "action": "create" }))),
            SessionMatch::Existing { .. } => ("session.existing", None),
            SessionMatch::ReusePlan { plan_id, .. } => (
                "plan.reuse",
                Some(json!({ "action": "reuse", "planId": plan_id })),
            ),
            SessionMatch::ReplacePlan { plan_id, .. } => (
                "plan.replace",
                Some(json!({ "action": "replace", "planId": plan_id })),
            ),
            SessionMatch::PlanExists { .. } => continue,
        };
        if !ticked.contains(format!("{reason}:{}", session.id).as_str()) {
            continue;
        }
        let chosen = request
            .shifts
            .iter()
            .find(|s| s.session_id == session.id)
            .map_or(0, |s| s.hours);
        let offset = f64::from(chosen - session.shift_hours) * 3600.0;
        let mut targets = Vec::new();
        for target in &session.targets {
            let line = |reason: &str| format!("{reason}:{}/{}", session.id, target.key);
            let wanted = match target.existing {
                TargetMatch::InSession => false,
                TargetMatch::AddToSession => ticked.contains(line("target.addToSession").as_str()),
                TargetMatch::Overlap => answer(&line("target.overlap")) == Some("add"),
                TargetMatch::New => match target.source {
                    TargetSource::ToSolve => false,
                    TargetSource::UnknownObject => {
                        ticked.contains(line("target.createPersonal").as_str())
                    }
                    _ => ticked.contains(line("target.detected").as_str()),
                },
            };
            if !wanted {
                continue;
            }
            match target_body(target, offset) {
                Some(body) => targets.push(body),
                None => skipped.push(SkippedElement {
                    kind: "target".into(),
                    id: format!("{}/{}", session.id, target.key),
                    reason: "target_unknown".into(),
                }),
            }
        }
        // A night whose place was not decided (the frames name none) is
        // created without one: said, never in silence.
        if !matches!(session.existing, SessionMatch::Existing { .. })
            && !place_of.contains_key(session.place.as_str())
            && !skipped
                .iter()
                .any(|s| s.kind == "place" && s.id == session.place)
        {
            skipped.push(SkippedElement {
                kind: "place".into(),
                id: session.place.clone(),
                reason: "place_choose".into(),
            });
        }
        let existing = match &session.existing {
            SessionMatch::Existing { session_id } => Some(session_id.clone()),
            _ => None,
        };
        if existing.is_none() && targets.is_empty() {
            skipped.push(SkippedElement {
                kind: "session".into(),
                id: session.id.clone(),
                reason: "no_target".into(),
            });
            continue;
        }
        let mut entry = json!({
            "ref": session.id,
            "eveningDate": session.evening_date,
            "startedAt": shifted(&session.start, offset),
            "finishedAt": shifted(&session.end, offset),
            "place": place_of.get(session.place.as_str()).cloned().unwrap_or(Value::Null),
            "equipment": equipment_of(proposal, session),
            "targets": targets,
        });
        if let Some(id) = existing {
            entry["sessionId"] = json!(id);
        }
        if let Some(plan) = plan {
            entry["plan"] = plan;
        }
        sent.insert(&session.id);
        sessions.push(entry);
    }

    let mut multi = Vec::new();
    for group in &proposal.multi_night {
        if group.locked || !ticked.contains(format!("multiNight.create:{}", group.id).as_str()) {
            continue;
        }
        let nights: Vec<_> = group
            .nights
            .iter()
            .filter(|n| sent.contains(n.session_id.as_str()))
            .collect();
        let Some(target_id) = &group.target_id else {
            skipped.push(skipped_group(group, "target_unknown"));
            continue;
        };
        if nights.len() < 2 {
            skipped.push(skipped_group(group, "nights_missing"));
            continue;
        }
        multi.push(json!({
            "ref": group.id,
            "targetId": target_id,
            "designation": group.designation,
            "sessionRefs": nights.iter().map(|n| n.session_id.clone()).collect::<Vec<_>>(),
            "goalSeconds": nights.iter().map(|n| n.seconds).sum::<f64>(),
        }));
    }

    let mut body = json!({
        "places": places,
        "sessions": sessions,
        "multiNight": multi,
    });
    body["importId"] = json!(import_id_of(&body));
    BulkBody { body, skipped }
}

fn skipped_group(group: &MultiNightGroup, reason: &str) -> SkippedElement {
    SkippedElement {
        kind: "multi-night".into(),
        id: group.id.clone(),
        reason: reason.into(),
    }
}

fn texts(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The service's answer, as the result screen draws it.
#[must_use]
pub fn outcome_of(
    request: &BulkCreateRequest,
    built: &BulkBody,
    answer: &Value,
) -> BulkCreateOutcome {
    let proposal = &request.proposal;
    let mut outcome = BulkCreateOutcome {
        skipped: built.skipped.clone(),
        replayed: answer["replayed"].as_bool().unwrap_or(false),
        ..BulkCreateOutcome::default()
    };
    for place in answer["places"].as_array().into_iter().flatten() {
        match place["placeId"].as_str() {
            Some(id) if place["created"].as_bool() == Some(true) => {
                outcome.places_created.push(id.to_string());
            }
            Some(_) => {}
            None => outcome.skipped.push(SkippedElement {
                kind: "place".into(),
                id: place["ref"].as_str().unwrap_or_default().into(),
                reason: place["reason"].as_str().unwrap_or("refused").into(),
            }),
        }
    }
    let sent: BTreeMap<&str, &Value> = built.body["sessions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| Some((s["ref"].as_str()?, s)))
        .collect();
    for made in answer["sessions"].as_array().into_iter().flatten() {
        let reference = made["ref"].as_str().unwrap_or_default();
        let action = made["action"].as_str().unwrap_or_default();
        let Some(session_id) = made["sessionId"].as_str().filter(|_| action != "skipped") else {
            outcome.skipped.push(SkippedElement {
                kind: "session".into(),
                id: reference.into(),
                reason: made["reason"].as_str().unwrap_or("refused").into(),
            });
            continue;
        };
        let Some(detected) = proposal.sessions.iter().find(|s| s.id == reference) else {
            continue;
        };
        let added: Vec<String> = sent
            .get(reference)
            .map(|s| {
                s["targets"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|t| t["designation"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let existing = action != "created";
        outcome.sessions.push(CreatedSession {
            detected_id: reference.into(),
            session_id: session_id.into(),
            evening_date: detected.evening_date.clone(),
            action: if existing { "updated" } else { "created" }.into(),
            plan_action: made["planAction"].as_str().unwrap_or("none").into(),
            plan_id: made["planId"].as_str().map(str::to_string),
            designations: detected
                .targets
                .iter()
                .filter(|t| {
                    existing && t.existing == TargetMatch::InSession
                        || added.contains(&t.designation)
                })
                .map(|t| t.designation.clone())
                .collect(),
            frames: detected
                .targets
                .iter()
                .filter(|t| t.existing == TargetMatch::InSession || added.contains(&t.designation))
                .map(|t| t.frames)
                .sum(),
            shift_hours: request
                .shifts
                .iter()
                .find(|s| s.session_id == reference)
                .map_or(0, |s| s.hours),
            added_targets: if existing && !texts(&made["addedTargets"]).is_empty() {
                added
            } else {
                Vec::new()
            },
        });
    }
    for made in answer["multiNight"].as_array().into_iter().flatten() {
        let reference = made["ref"].as_str().unwrap_or_default();
        let group = proposal.multi_night.iter().find(|g| g.id == reference);
        match (made["projectId"].as_str(), group) {
            (Some(project_id), Some(group)) if made["action"] == "created" => {
                let goal = built.body["multiNight"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|m| m["ref"] == reference)
                    .and_then(|m| m["goalSeconds"].as_f64())
                    .unwrap_or(group.goal_seconds);
                outcome.multi_night.push(CreatedMultiNight {
                    group_id: reference.into(),
                    plan_id: project_id.into(),
                    designation: group.designation.clone(),
                    session_ids: texts(&made["sessionIds"]),
                    goal_seconds: goal,
                });
            }
            _ => outcome.skipped.push(SkippedElement {
                kind: "multi-night".into(),
                id: reference.into(),
                reason: made["reason"].as_str().unwrap_or("refused").into(),
            }),
        }
    }
    outcome
}

/// The one call: `POST /v1/sessions/bulk` (idempotent: a retry replays),
/// after the personal targets the person ticked (`GET` then `POST
/// /v1/targets/personal`, the service's own mechanism).
pub trait BulkCreateTransport {
    fn create_bulk(
        &self,
        base_url: &str,
        token: &str,
        body: &Value,
    ) -> Result<Value, GuiBridgeError>;
    /// `GET /v1/targets/personal`: `{ targets: [{ id, name, … }] }`.
    fn personal_targets(&self, base_url: &str, token: &str) -> Result<Value, GuiBridgeError>;
    /// `POST /v1/targets/personal`, sent once: `{ target: { id, … } }`.
    fn create_personal_target(
        &self,
        base_url: &str,
        token: &str,
        body: &Value,
    ) -> Result<Value, GuiBridgeError>;
}

impl BulkCreateTransport for HttpPortalGuiTransport {
    fn create_bulk(
        &self,
        base_url: &str,
        token: &str,
        body: &Value,
    ) -> Result<Value, GuiBridgeError> {
        let url = format!("{}/api/v1/sessions/bulk", base_url.trim_end_matches('/'));
        let token = token.to_owned();
        let body = body.clone();
        let build: RequestFactory =
            Arc::new(move |client| client.post(url.clone()).bearer_auth(&token).json(&body));
        self.read_json(&build)
    }

    fn personal_targets(&self, base_url: &str, token: &str) -> Result<Value, GuiBridgeError> {
        let url = format!("{}/api/v1/targets/personal", base_url.trim_end_matches('/'));
        let token = token.to_owned();
        let build: RequestFactory =
            Arc::new(move |client| client.get(url.clone()).bearer_auth(&token));
        self.read_json(&build)
    }

    fn create_personal_target(
        &self,
        base_url: &str,
        token: &str,
        body: &Value,
    ) -> Result<Value, GuiBridgeError> {
        let url = format!("{}/api/v1/targets/personal", base_url.trim_end_matches('/'));
        self.write_json_once(base_url, &|client| {
            client.post(url.clone()).bearer_auth(token).json(body)
        })
    }
}

/// The tag of the public gallery a personal target made here carries: the
/// frames say nothing of what the object is, so the most general of the
/// seven (the person can change it in the target library).
const PERSONAL_LABEL: &str = "Grand champ";

/// The targets named by `OBJECT` that the catalogue does not know and the
/// person left ticked ("create "..." as a new target") are created as
/// personal targets of the space, once each (one already held under the same
/// name is reused: a retry never doubles it), and take their id in the
/// proposal. One the service refuses keeps no id: the body skips it as
/// `target_unknown`.
fn with_personal_targets<T: BulkCreateTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
    request: &BulkCreateRequest,
) -> BulkCreateRequest {
    let ticked: BTreeSet<&str> = request.ticked.iter().map(String::as_str).collect();
    let wanted = |session: &DetectedSession, target: &DetectedTarget| {
        target.target_id.is_none()
            && matches!(target.source, TargetSource::UnknownObject)
            && ticked
                .contains(format!("target.createPersonal:{}/{}", session.id, target.key).as_str())
    };
    let designations: BTreeSet<String> = request
        .proposal
        .sessions
        .iter()
        .flat_map(|s| s.targets.iter().filter(|t| wanted(s, t)))
        .map(|t| t.designation.clone())
        .collect();
    if designations.is_empty() {
        return request.clone();
    }
    let held = transport
        .personal_targets(base_url, token)
        .ok()
        .and_then(|answer| answer.get("targets").and_then(Value::as_array).cloned())
        .unwrap_or_default();
    let mut ids: BTreeMap<String, String> = BTreeMap::new();
    for designation in designations {
        let same = |entry: &&Value| {
            entry["name"]
                .as_str()
                .is_some_and(|name| name.trim().eq_ignore_ascii_case(designation.trim()))
        };
        let id = held
            .iter()
            .find(same)
            .and_then(|entry| entry["id"].as_str().map(str::to_string))
            .or_else(|| {
                let body = personal_target_body(&PersonalTargetCreate {
                    name: designation.clone(),
                    label: PERSONAL_LABEL.into(),
                    ra_hours: None,
                    dec_degrees: None,
                })
                .ok()?;
                let answer = transport
                    .create_personal_target(base_url, token, &body)
                    .ok()?;
                answer["target"]["id"]
                    .as_str()
                    .or_else(|| answer["id"].as_str())
                    .map(str::to_string)
            });
        if let Some(id) = id {
            ids.insert(designation, id);
        }
    }
    let mut prepared = request.clone();
    let mut keys: BTreeMap<String, String> = BTreeMap::new();
    for session in &mut prepared.proposal.sessions {
        let snapshot = session.clone();
        for target in &mut session.targets {
            if !wanted(&snapshot, target) {
                continue;
            }
            if let Some(id) = ids.get(&target.designation) {
                target.target_id = Some(id.clone());
                keys.insert(target.key.clone(), id.clone());
            }
        }
    }
    for group in &mut prepared.proposal.multi_night {
        if group.target_id.is_none() {
            group.target_id = keys.get(&group.target_key).cloned();
        }
    }
    prepared
}

/// "Create…": the personal targets, the body, the call, the answer read.
pub fn create_sessions<T: BulkCreateTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
    request: &BulkCreateRequest,
) -> Result<BulkCreateOutcome, GuiBridgeError> {
    let request = &with_personal_targets(transport, base_url, token, request);
    let built = bulk_body(request);
    let nothing = built.body["sessions"].as_array().is_none_or(Vec::is_empty)
        && built.body["places"].as_array().is_none_or(Vec::is_empty);
    if nothing {
        return Ok(BulkCreateOutcome {
            skipped: built.skipped,
            ..BulkCreateOutcome::default()
        });
    }
    let answer = transport.create_bulk(base_url, token, &built.body)?;
    Ok(outcome_of(request, &built, &answer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue_targets::PERSONAL_LABELS;
    use crate::session_detection::{
        AcquisitionRow, EquipmentMatch, EquipmentProposal, MultiNightNight, PlaceProposal,
        ProposalLine,
    };

    fn target(
        key: &str,
        id: Option<&str>,
        existing: TargetMatch,
        start: &str,
        end: &str,
    ) -> DetectedTarget {
        DetectedTarget {
            key: key.into(),
            target_id: id.map(str::to_string),
            designation: key.to_uppercase(),
            source: if id.is_some() {
                TargetSource::Object
            } else {
                TargetSource::UnknownObject
            },
            start: start.into(),
            end: end.into(),
            frames: 40,
            exposure_seconds: 7200.0,
            acquisition: vec![AcquisitionRow {
                exposure_seconds: Some(180.0),
                frames: 40,
                total_seconds: 7200.0,
                ..AcquisitionRow::default()
            }],
            existing,
            ..DetectedTarget::default()
        }
    }

    fn line(id: &str, checked: bool) -> ProposalLine {
        ProposalLine {
            id: id.into(),
            checked,
            ..ProposalLine::default()
        }
    }

    fn proposal() -> Proposal {
        Proposal {
            places: vec![
                PlaceProposal {
                    id: "place-1".into(),
                    chosen: Some("maison".into()),
                    ..PlaceProposal::default()
                },
                PlaceProposal {
                    id: "place-2".into(),
                    latitude: Some(44.0),
                    longitude: Some(7.0),
                    ..PlaceProposal::default()
                },
            ],
            equipment: vec![EquipmentProposal {
                id: "eq-1".into(),
                camera: Some(EquipmentMatch {
                    equipment_id: "cam".into(),
                    name: "ZWO ASI533MC Pro".into(),
                    by: "name".into(),
                }),
                ..EquipmentProposal::default()
            }],
            sessions: vec![
                DetectedSession {
                    id: "s-07".into(),
                    evening_date: "2026-08-07".into(),
                    place: "place-1".into(),
                    equipment: vec!["eq-1".into()],
                    start: "2026-08-07T21:00:00.000Z".into(),
                    end: "2026-08-08T01:00:00.000Z".into(),
                    shift_hours: -9,
                    targets: vec![
                        target(
                            "ic1396",
                            Some("ic1396"),
                            TargetMatch::New,
                            "2026-08-07T21:00:00.000Z",
                            "2026-08-08T01:00:00.000Z",
                        ),
                        target(
                            "object:foo",
                            None,
                            TargetMatch::New,
                            "2026-08-07T22:00:00.000Z",
                            "2026-08-07T23:00:00.000Z",
                        ),
                    ],
                    ..DetectedSession::default()
                },
                DetectedSession {
                    id: "s-11".into(),
                    evening_date: "2026-08-11".into(),
                    place: "place-2".into(),
                    start: "2026-08-11T21:00:00.000Z".into(),
                    end: "2026-08-12T02:00:00.000Z".into(),
                    existing: SessionMatch::Existing {
                        session_id: "sess-11".into(),
                    },
                    targets: vec![
                        target(
                            "ic1805",
                            Some("ic1805"),
                            TargetMatch::InSession,
                            "2026-08-11T21:00:00.000Z",
                            "2026-08-11T23:00:00.000Z",
                        ),
                        target(
                            "sh2-103",
                            Some("sh2-103"),
                            TargetMatch::AddToSession,
                            "2026-08-11T23:10:00.000Z",
                            "2026-08-12T02:00:00.000Z",
                        ),
                        target(
                            "m31",
                            Some("m31"),
                            TargetMatch::Overlap,
                            "2026-08-11T22:00:00.000Z",
                            "2026-08-11T23:00:00.000Z",
                        ),
                    ],
                    ..DetectedSession::default()
                },
                DetectedSession {
                    id: "s-12".into(),
                    evening_date: "2026-08-12".into(),
                    place: "place-1".into(),
                    start: "2026-08-12T21:00:00.000Z".into(),
                    end: "2026-08-13T01:00:00.000Z".into(),
                    existing: SessionMatch::ReplacePlan {
                        plan_id: "plan-m33".into(),
                        plan_name: "M33 – RVB".into(),
                    },
                    targets: vec![target(
                        "ic1396",
                        Some("ic1396"),
                        TargetMatch::New,
                        "2026-08-12T21:00:00.000Z",
                        "2026-08-13T01:00:00.000Z",
                    )],
                    ..DetectedSession::default()
                },
            ],
            multi_night: vec![MultiNightGroup {
                id: "mn-ic1396".into(),
                target_id: Some("ic1396".into()),
                designation: "IC 1396".into(),
                nights: vec![
                    MultiNightNight {
                        session_id: "s-07".into(),
                        seconds: 7200.0,
                        ..MultiNightNight::default()
                    },
                    MultiNightNight {
                        session_id: "s-12".into(),
                        seconds: 7200.0,
                        ..MultiNightNight::default()
                    },
                ],
                goal_seconds: 14400.0,
                ..MultiNightGroup::default()
            }],
            lines: vec![
                line("session.create:s-07", true),
                line("target.detected:s-07/ic1396", true),
                line("target.createPersonal:s-07/object:foo", true),
                line("clock.shift:s-07", true),
                line("session.existing:s-11", true),
                line("target.inSession:s-11/ic1805", true),
                line("target.addToSession:s-11/sh2-103", true),
                line("target.overlap:s-11/m31", false),
                line("plan.replace:s-12", true),
                line("target.detected:s-12/ic1396", true),
                line("multiNight.create:mn-ic1396", true),
            ],
            ..Proposal::default()
        }
    }

    fn request() -> BulkCreateRequest {
        let mut request = BulkCreateRequest::as_proposed(proposal());
        request.places.push(PlaceAnswer::Create {
            place_id: "place-2".into(),
            name: "Col".into(),
            latitude: 44.0,
            longitude: 7.0,
            time_zone: Some("Europe/Paris".into()),
        });
        request
    }

    #[test]
    fn the_night_is_created_with_the_equipment_found_filters_included() {
        let mut asked = request();
        asked.proposal.equipment[0].telescope = Some(EquipmentMatch {
            equipment_id: "scope".into(),
            name: "Askar 50P".into(),
            by: "focal-length".into(),
        });
        asked.proposal.equipment[0].filter_matches = vec![EquipmentMatch {
            equipment_id: "f-ha".into(),
            name: "Ha 7 nm".into(),
            by: "name".into(),
        }];
        let built = bulk_body(&asked);
        let equipment = &built.body["sessions"][0]["equipment"];
        assert_eq!(equipment["cameraId"], "cam");
        assert_eq!(equipment["telescopeId"], "scope");
        assert_eq!(equipment["filterIds"], serde_json::json!(["f-ha"]));
        assert_eq!(equipment["mode"], "imaging");
    }

    #[test]
    fn the_body_holds_what_is_ticked_and_nothing_else() {
        let built = bulk_body(&request());
        let body = &built.body;
        assert_eq!(body["places"][0]["ref"], "place-2");
        let sessions = body["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 3);
        // The proposed shift kept: the times are the proposal's.
        assert_eq!(sessions[0]["startedAt"], "2026-08-07T21:00:00.000Z");
        assert_eq!(sessions[0]["plan"]["action"], "create");
        assert_eq!(sessions[0]["place"]["placeId"], "maison");
        assert_eq!(sessions[0]["equipment"]["cameraId"], "cam");
        assert_eq!(sessions[0]["targets"].as_array().unwrap().len(), 1);
        assert_eq!(sessions[0]["targets"][0]["exposureSeconds"], 180.0);
        // The existing session: only the target to add; the overlap without answer is not.
        assert_eq!(sessions[1]["sessionId"], "sess-11");
        assert_eq!(sessions[1]["place"]["ref"], "place-2");
        let added: Vec<_> = sessions[1]["targets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["targetId"].clone())
            .collect();
        assert_eq!(added, vec![json!("sh2-103")]);
        assert_eq!(
            sessions[2]["plan"],
            json!({ "action": "replace", "planId": "plan-m33" })
        );
        assert_eq!(
            body["multiNight"][0]["sessionRefs"],
            json!(["s-07", "s-12"])
        );
        assert_eq!(body["multiNight"][0]["goalSeconds"], 14400.0);
        assert_eq!(
            built.skipped,
            vec![SkippedElement {
                kind: "target".into(),
                id: "s-07/object:foo".into(),
                reason: "target_unknown".into()
            }]
        );
        // The same choices, the same key; another choice, another key.
        assert_eq!(bulk_body(&request()).body["importId"], body["importId"]);
        let key = body["importId"].as_str().unwrap();
        assert_eq!(key.len(), 26);
        assert!(('0'..='7').contains(&key.chars().next().unwrap()));
        let mut other = request();
        other.answers.push(LineAnswer {
            line_id: "target.overlap:s-11/m31".into(),
            answer: "add".into(),
        });
        let changed = bulk_body(&other);
        assert_ne!(changed.body["importId"], body["importId"]);
        assert_eq!(
            changed.body["sessions"][1]["targets"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    fn night_in_an_unknown_place(action: PlaceAction, ticked: bool) -> Proposal {
        let mut proposal = Proposal {
            places: vec![PlaceProposal {
                id: "place-9".into(),
                latitude: Some(36.7265),
                longitude: Some(139.994),
                action,
                chosen: None,
                sessions: vec!["s-10".into()],
                ..PlaceProposal::default()
            }],
            sessions: vec![DetectedSession {
                id: "s-10".into(),
                evening_date: "2026-10-10".into(),
                place: "place-9".into(),
                start: "2026-10-10T13:26:00.000Z".into(),
                end: "2026-10-10T20:02:00.000Z".into(),
                targets: vec![target(
                    "openngc-mel022",
                    Some("openngc-mel022"),
                    TargetMatch::New,
                    "2026-10-10T13:26:00.000Z",
                    "2026-10-10T20:02:00.000Z",
                )],
                ..DetectedSession::default()
            }],
            lines: vec![
                line("place.create:place-9", ticked),
                line("session.create:s-10", true),
                line("target.detected:s-10/openngc-mel022", true),
            ],
            ..Proposal::default()
        };
        proposal.sessions[0].existing = SessionMatch::Create;
        proposal
    }

    /// The designation the frames give a target when the catalogue knows it by
    /// another is kept beside the catalogue's name, not lost.
    #[test]
    fn the_name_the_frames_give_a_target_stays_beside_the_catalogues() {
        let named = |designation: &str, name: Option<&str>, objects: &[&str]| {
            let mut t = target("k", Some("id"), TargetMatch::New, "a", "b");
            t.designation = designation.into();
            t.name = name.map(str::to_string);
            t.objects = objects.iter().map(|o| (*o).to_string()).collect();
            shown_name(&t)
        };
        // SH2-190 on the lights, IC 1805 for the catalogue.
        assert_eq!(
            named("IC 1805", Some("Heart Nebula"), &["SH2-190"]).as_deref(),
            Some("Heart Nebula · SH2-190")
        );
        assert_eq!(
            named("IC 1805", None, &["SH2-190"]).as_deref(),
            Some("SH2-190")
        );
        // Words of the person, over a catalogue's.
        assert_eq!(
            named("Sh2-103", Some("Veil Nebula"), &["Voile complet"]).as_deref(),
            Some("Veil Nebula · Voile complet")
        );
        // The catalogue's own names, written another way, add nothing.
        assert_eq!(
            named(
                "NGC 224",
                Some("Andromeda"),
                &["NGC0224", "ngc 224", "andromeda"]
            )
            .as_deref(),
            Some("Andromeda")
        );
        assert_eq!(named("M 31", None, &["M31", "m 31"]), None);
        assert_eq!(named("IC 1805", None, &[]), None);
        // The service keeps 300 characters of a name.
        let long = "x".repeat(400);
        assert_eq!(
            named("IC 1805", None, &[long.as_str()])
                .unwrap()
                .chars()
                .count(),
            300
        );
    }

    /// "Unknown place: create this place" ticked is done -- the place
    /// is created under a neutral name, never its coordinates, and the night
    /// is in it.
    #[test]
    fn a_ticked_place_to_create_is_created_under_a_neutral_name_and_the_night_is_in_it() {
        let request =
            BulkCreateRequest::as_proposed(night_in_an_unknown_place(PlaceAction::Create, true));
        let built = bulk_body(&request);
        assert_eq!(built.body["places"].as_array().unwrap().len(), 1);
        assert_eq!(built.body["places"][0]["name"], DEFAULT_PLACE_NAME);
        assert!(!built.body["places"][0]["name"].to_string().contains('°'));
        assert_eq!(built.body["places"][0]["ref"], "place-9");
        assert_eq!(built.body["sessions"][0]["place"]["ref"], "place-9");
        assert!(built.skipped.is_empty(), "{:?}", built.skipped);
        // The caller's own word (the dictionary of the person's language), or
        // the person's name for it.
        let named = bulk_body(&BulkCreateRequest::as_proposed_named(
            night_in_an_unknown_place(PlaceAction::Create, true),
            "Lieu d’observation",
        ));
        assert_eq!(named.body["places"][0]["name"], "Lieu d’observation");
    }

    /// The place the person unticked is not created; the night goes without it
    /// and says so.
    #[test]
    fn a_place_left_unticked_or_unnamed_is_said_not_dropped_in_silence() {
        for (action, ticked) in [(PlaceAction::Create, false), (PlaceAction::Choose, false)] {
            let built = bulk_body(&BulkCreateRequest::as_proposed(night_in_an_unknown_place(
                action, ticked,
            )));
            assert!(built.body["places"].as_array().unwrap().is_empty());
            assert!(built.body["sessions"][0]["place"].is_null());
            assert!(
                built
                    .skipped
                    .iter()
                    .any(|s| s.kind == "place" && s.id == "place-9" && s.reason == "place_choose"),
                "{:?}",
                built.skipped
            );
        }
    }

    #[test]
    fn a_shift_other_than_the_proposed_one_moves_the_times_and_an_unticked_night_is_not_sent() {
        let mut request = request();
        request.shifts = vec![SessionShift {
            session_id: "s-07".into(),
            hours: -8,
        }];
        request.ticked.retain(|id| id != "plan.replace:s-12");
        let built = bulk_body(&request);
        assert_eq!(
            built.body["sessions"][0]["startedAt"],
            "2026-08-07T22:00:00.000Z"
        );
        assert_eq!(
            built.body["sessions"][0]["targets"][0]["finishedAt"],
            "2026-08-08T02:00:00.000Z"
        );
        assert_eq!(built.body["sessions"].as_array().unwrap().len(), 2);
        // One night left of the group: no multi-night plan, said why.
        assert!(built.body["multiNight"].as_array().unwrap().is_empty());
        assert!(
            built
                .skipped
                .iter()
                .any(|s| s.id == "mn-ic1396" && s.reason == "nights_missing")
        );
    }

    struct Answering(Value);
    impl BulkCreateTransport for Answering {
        fn create_bulk(&self, _: &str, _: &str, body: &Value) -> Result<Value, GuiBridgeError> {
            assert!(body["importId"].is_string());
            Ok(self.0.clone())
        }
        fn personal_targets(&self, _: &str, _: &str) -> Result<Value, GuiBridgeError> {
            Ok(json!({ "targets": [] }))
        }
        fn create_personal_target(
            &self,
            _: &str,
            _: &str,
            body: &Value,
        ) -> Result<Value, GuiBridgeError> {
            Ok(json!({ "target": { "id": "personal-foo", "name": body["name"] } }))
        }
    }

    #[test]
    fn the_answer_becomes_the_result_rows_with_what_was_skipped() {
        let answer = json!({
            "importId": "x", "replayed": false,
            "places": [{ "ref": "place-2", "placeId": "astro-sync-1", "created": true }],
            "sessions": [
                { "ref": "s-07", "sessionId": "a", "planId": "p7", "action": "created", "planAction": "created", "addedTargets": [], "replayed": false },
                { "ref": "s-11", "sessionId": "sess-11", "planId": null, "action": "updated", "planAction": "none", "addedTargets": ["sh2-103"], "replayed": false },
                { "ref": "s-12", "sessionId": null, "planId": "plan-m33", "action": "skipped", "planAction": "none", "addedTargets": [], "replayed": false, "reason": "plan_not_pending" }
            ],
            "multiNight": [{ "ref": "mn-ic1396", "projectId": null, "sessionIds": [], "action": "skipped", "replayed": false, "reason": "nights_missing" }]
        });
        let outcome = create_sessions(&Answering(answer), "http://x", "t", &request()).unwrap();
        assert_eq!(outcome.places_created, vec!["astro-sync-1".to_string()]);
        assert_eq!(outcome.sessions.len(), 2);
        let first = &outcome.sessions[0];
        assert_eq!(
            (
                first.action.as_str(),
                first.plan_action.as_str(),
                first.shift_hours
            ),
            ("created", "created", -9)
        );
        assert_eq!(
            first.designations,
            vec!["IC1396".to_string(), "OBJECT:FOO".to_string()]
        );
        let updated = &outcome.sessions[1];
        assert_eq!(updated.action, "updated");
        assert_eq!(updated.added_targets, vec!["SH2-103".to_string()]);
        assert_eq!(
            updated.designations,
            vec!["IC1805".to_string(), "SH2-103".to_string()]
        );
        assert_eq!(updated.frames, 80);
        let reasons: Vec<_> = outcome
            .skipped
            .iter()
            .map(|s| (s.id.as_str(), s.reason.as_str()))
            .collect();
        assert_eq!(
            reasons,
            vec![
                ("s-12", "plan_not_pending"),
                ("mn-ic1396", "nights_missing")
            ]
        );
    }

    /// Records the calls of the personal-target mechanism.
    struct Personal {
        held: Vec<Value>,
        created: std::cell::RefCell<Vec<Value>>,
        bulk: std::cell::RefCell<Option<Value>>,
        refuse: bool,
    }
    impl BulkCreateTransport for Personal {
        fn create_bulk(&self, _: &str, _: &str, body: &Value) -> Result<Value, GuiBridgeError> {
            *self.bulk.borrow_mut() = Some(body.clone());
            Ok(
                json!({ "importId": "x", "replayed": false, "places": [], "sessions": [], "multiNight": [] }),
            )
        }
        fn personal_targets(&self, _: &str, _: &str) -> Result<Value, GuiBridgeError> {
            Ok(json!({ "targets": self.held }))
        }
        fn create_personal_target(
            &self,
            _: &str,
            _: &str,
            body: &Value,
        ) -> Result<Value, GuiBridgeError> {
            if self.refuse {
                return Err(GuiBridgeError::InvalidResponse("refused".into()));
            }
            self.created.borrow_mut().push(body.clone());
            Ok(json!({ "target": { "id": "personal-foo", "name": body["name"] } }))
        }
    }

    fn personal(held: Vec<Value>, refuse: bool) -> Personal {
        Personal {
            held,
            created: Default::default(),
            bulk: Default::default(),
            refuse,
        }
    }

    #[test]
    fn a_target_outside_the_catalogue_is_created_as_a_personal_target_before_the_sending() {
        let transport = personal(Vec::new(), false);
        create_sessions(&transport, "http://x", "t", &request()).unwrap();
        let created = transport.created.borrow();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0]["name"], "OBJECT:FOO");
        assert!(PERSONAL_LABELS.contains(&created[0]["label"].as_str().unwrap()));
        let body = transport.bulk.borrow().clone().unwrap();
        let targets = &body["sessions"][0]["targets"];
        assert!(
            targets
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["targetId"] == "personal-foo")
        );
    }

    #[test]
    fn a_personal_target_the_space_already_holds_is_reused_not_created_again() {
        let held = vec![json!({ "id": "personal-held", "name": "object:foo" })];
        let transport = personal(held, false);
        create_sessions(&transport, "http://x", "t", &request()).unwrap();
        assert!(transport.created.borrow().is_empty());
        let body = transport.bulk.borrow().clone().unwrap();
        assert!(body.to_string().contains("personal-held"));
    }

    #[test]
    fn a_personal_target_the_service_refuses_stays_skipped_as_unknown() {
        let transport = personal(Vec::new(), true);
        let outcome = create_sessions(&transport, "http://x", "t", &request()).unwrap();
        assert!(
            outcome
                .skipped
                .iter()
                .any(|s| s.id == "s-07/object:foo" && s.reason == "target_unknown")
        );
    }
}

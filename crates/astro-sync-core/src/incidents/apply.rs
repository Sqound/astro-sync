//! The writing of the list ("Add N incidents to the session"): two calls,
//! in this order.
//!
//! 1. **The additions**, `POST /v1/sessions/{id}/events` in batches of at most
//!    [`MAX_ADDS_PER_BATCH`]. They are **idempotent**: the `eventId` of an
//!    addition is derived from the session and the detection key
//!    ([`event_id_for`]), so sending the same list again after a cut writes
//!    nothing twice (the service answers `duplicate`). A pause is written as
//!    the pair `pause` + `resume` (`session-events.md`): the `resume` closes
//!    it, both dated at their own instant.
//! 2. **The replacements**, `PUT /v1/sessions/{id}/rework`
//!    (`eventReplacements`, `If-Match` with the `reworkRevision` read just
//!    before), all or nothing. A refusal (`session_rework_conflict`,
//!    `night_not_finished`, `event_not_replaceable`, …) is an *answer*
//!    ([`ApplyOutcome::replacements_refused`]): the additions already made
//!    stay told. The original of a replaced signal stays in the journal,
//!    hidden.
//!
//! **The key space of the identifiers.** An `eventId` is 26 characters of
//! Crockford base 32. A ULID that a client generates starts, for as long as
//! the clock reads a year before 10889, with `0`; the ones the service
//! derives for a debriefing start with `0` as well (a night anchor of this
//! century). The identifiers of this module start with
//! [`EVENT_ID_PREFIX`] (`7`: the first three bits of a ULID at their
//! highest, a year no clock reaches) and go on with 125 bits of a SHA-256
//! over `astro-sync/incidents/v1`, the session and the key: they cannot meet
//! a generated one, and are still valid ULIDs.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

use super::IncidentKind;
use super::declared::IncidentsTransport;
use crate::gui_bridge::GuiBridgeError;
use crate::time_util::parse_iso8601_utc_to_unix_seconds;

/// The first character of every identifier of this module (see the module
/// documentation).
pub const EVENT_ID_PREFIX: char = '7';
/// The most events one `POST /v1/sessions/{id}/events` takes.
pub const MAX_ADDS_PER_BATCH: usize = 500;
/// The most replacements one rework takes.
pub const MAX_REPLACEMENTS: usize = 100;
/// What follows the person's own text in a replaced signal
/// ("Your text · detected: ..."); [`merged_text_with`] takes another one.
pub const DETECTED_SEPARATOR: &str = " \u{b7} detected: ";

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

fn derive(purpose: &str, session_id: &str, key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"astro-sync/incidents/v1\0");
    hasher.update(purpose.as_bytes());
    hasher.update(b"\0");
    hasher.update(session_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(key.as_bytes());
    let digest = hasher.finalize();
    let mut head = [0_u8; 16];
    head.copy_from_slice(&digest[..16]);
    let value = u128::from_be_bytes(head) & ((1_u128 << 125) - 1);
    let mut id = String::with_capacity(26);
    id.push(EVENT_ID_PREFIX);
    for position in (0..25).rev() {
        id.push(CROCKFORD[((value >> (5 * position)) & 0x1F) as usize] as char);
    }
    id
}

/// The `eventId` of the event a detection key becomes in a session:
/// deterministic, so a second sending is a no-op for the service.
#[must_use]
pub fn event_id_for(session_id: &str, key: &str) -> String {
    derive("event", session_id, key)
}

/// The `eventId` of the `resume` that closes the pause of `key`.
fn resume_event_id_for(session_id: &str, key: &str) -> String {
    derive("resume", session_id, key)
}

/// An incident to add to the session (what the person kept, edited or not).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedIncident {
    /// The detection key ([`super::DetectedIncident::id`]), or one the window
    /// gives to an incident the person added by hand.
    pub key: String,
    pub event_type: IncidentKind,
    /// ISO 8601 UTC: the `recordedAt` of the event (of the `pause`).
    pub at: String,
    /// The end of a pause: the `recordedAt` of its `resume`. Refused on any
    /// other type.
    pub until: Option<String>,
    pub payload: Value,
}

/// A declared signal replaced by a detection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Replacement {
    /// The `eventId` of the declared signal (the pause, for a pause).
    pub event_id: String,
    /// The type of the original: the replacement has the same one.
    pub event_type: IncidentKind,
    pub at: String,
    pub until: Option<String>,
    pub payload: Value,
}

/// What the person chose to write.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Apply {
    pub session_id: String,
    #[serde(default)]
    pub adds: Vec<AddedIncident>,
    #[serde(default)]
    pub replacements: Vec<Replacement>,
}

/// An addition the service stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedEvent {
    pub key: String,
    pub event_id: String,
    /// The `resume` that closes an added pause.
    pub resume_event_id: Option<String>,
}

/// An addition the service refused, alone (the others are stored).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rejection {
    pub key: String,
    pub code: String,
    pub message: String,
}

/// Why the replacements were all refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplacementsRefused {
    /// The code of the service (`session_rework_conflict`,
    /// `night_not_finished`, `event_not_replaceable`,
    /// `event_already_replaced`, `invalid_request`, …), or `unreachable`,
    /// `invalid_response`, `sign_in_ended`, `service_error` when the service
    /// did not answer.
    pub code: String,
    pub message: String,
}

/// What the two calls came to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyOutcome {
    pub added: Vec<AddedEvent>,
    /// The keys the session already held (a first sending was cut, or the
    /// list was sent twice): nothing was written for them.
    pub duplicate: Vec<String>,
    pub rejected: Vec<Rejection>,
    /// The `eventId` of the declared signals the rework replaced.
    pub replaced: Vec<String>,
    pub replacements_refused: Option<ReplacementsRefused>,
}

#[derive(Debug, Error)]
pub enum ApplyError {
    /// The request is wrong; nothing was sent.
    #[error("invalid request: {0}")]
    Invalid(String),
    /// The service could not be reached or answered something unreadable
    /// while the additions were sent. What was answered before is in
    /// `partial`; a second sending is harmless (idempotent).
    #[error("the service did not answer the additions: {source}")]
    Service {
        source: GuiBridgeError,
        partial: Box<ApplyOutcome>,
    },
}

fn instant(text: &str) -> Option<f64> {
    parse_iso8601_utc_to_unix_seconds(text).filter(|t| t.is_finite())
}

fn check_span(
    what: &str,
    kind: IncidentKind,
    at: &str,
    until: Option<&str>,
    payload: &Value,
) -> Result<(), ApplyError> {
    let invalid = |reason: &str| Err(ApplyError::Invalid(format!("{what}: {reason}")));
    let Some(start) = instant(at) else {
        return invalid("`at` is not an ISO 8601 UTC instant");
    };
    if !payload.is_object() {
        return invalid("the payload is not an object");
    }
    match until {
        None => Ok(()),
        Some(_) if kind != IncidentKind::Pause => invalid("only a pause has an `until`"),
        Some(until) => match instant(until) {
            None => invalid("`until` is not an ISO 8601 UTC instant"),
            Some(end) if end < start => invalid("`until` is before `at`"),
            Some(_) => Ok(()),
        },
    }
}

fn validate(request: &Apply) -> Result<(), ApplyError> {
    if request.session_id.trim().is_empty() {
        return Err(ApplyError::Invalid("no session".to_string()));
    }
    if request.replacements.len() > MAX_REPLACEMENTS {
        return Err(ApplyError::Invalid(format!(
            "at most {MAX_REPLACEMENTS} replacements at once, got {}",
            request.replacements.len()
        )));
    }
    for add in &request.adds {
        if add.key.trim().is_empty() {
            return Err(ApplyError::Invalid("an addition has no key".to_string()));
        }
        check_span(
            &add.key,
            add.event_type,
            &add.at,
            add.until.as_deref(),
            &add.payload,
        )?;
    }
    for replacement in &request.replacements {
        if replacement.event_id.trim().is_empty() {
            return Err(ApplyError::Invalid(
                "a replacement names no event".to_string(),
            ));
        }
        if !replacement.event_type.replaceable() {
            return Err(ApplyError::Invalid(format!(
                "{}: a {} cannot be replaced",
                replacement.event_id,
                replacement.event_type.as_str()
            )));
        }
        check_span(
            &replacement.event_id,
            replacement.event_type,
            &replacement.at,
            replacement.until.as_deref(),
            &replacement.payload,
        )?;
    }
    Ok(())
}

/// One addition, as the events it becomes.
struct Prepared {
    key: String,
    event_id: String,
    resume_event_id: Option<String>,
    events: Vec<Value>,
}

fn prepare(session_id: &str, adds: &[AddedIncident]) -> Vec<Prepared> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut prepared = Vec::new();
    for add in adds {
        if !seen.insert(add.key.as_str()) {
            continue;
        }
        let event_id = event_id_for(session_id, &add.key);
        let mut events = vec![json!({
            "eventId": event_id,
            "type": add.event_type.as_str(),
            "recordedAt": add.at,
            "payload": add.payload,
        })];
        let mut resume_event_id = None;
        if add.event_type == IncidentKind::Pause
            && let Some(until) = &add.until
        {
            let id = resume_event_id_for(session_id, &add.key);
            events.push(json!({
                "eventId": id,
                "type": "resume",
                "recordedAt": until,
                "payload": {},
            }));
            resume_event_id = Some(id);
        }
        prepared.push(Prepared {
            key: add.key.clone(),
            event_id,
            resume_event_id,
            events,
        });
    }
    prepared
}

/// What the service said of the events of the batches sent so far.
#[derive(Default)]
struct Answers {
    accepted: HashSet<String>,
    duplicate: HashSet<String>,
    rejected: HashMap<String, (String, String)>,
}

impl Answers {
    fn read(&mut self, answer: &Value) {
        let ids = |name: &str| -> Vec<String> {
            answer
                .get(name)
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        self.accepted.extend(ids("accepted"));
        self.duplicate.extend(ids("duplicate"));
        if let Some(list) = answer.get("rejected").and_then(Value::as_array) {
            for item in list {
                let text = |name: &str| {
                    item.get(name)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string()
                };
                self.rejected
                    .insert(text("eventId"), (text("code"), text("message")));
            }
        }
    }

    /// The outcome for the additions of `prepared` whose batch was answered.
    fn outcome(&self, prepared: &[Prepared]) -> ApplyOutcome {
        let mut outcome = ApplyOutcome::default();
        for item in prepared {
            let ids: Vec<&String> = std::iter::once(&item.event_id)
                .chain(item.resume_event_id.iter())
                .collect();
            if let Some((code, message)) = ids.iter().find_map(|id| self.rejected.get(*id)) {
                outcome.rejected.push(Rejection {
                    key: item.key.clone(),
                    code: code.clone(),
                    message: message.clone(),
                });
            } else if ids.iter().any(|id| self.accepted.contains(*id)) {
                outcome.added.push(AddedEvent {
                    key: item.key.clone(),
                    event_id: item.event_id.clone(),
                    resume_event_id: item.resume_event_id.clone(),
                });
            } else if ids.iter().all(|id| self.duplicate.contains(*id)) {
                outcome.duplicate.push(item.key.clone());
            } else {
                outcome.rejected.push(Rejection {
                    key: item.key.clone(),
                    code: "not_acknowledged".to_string(),
                    message: "the service did not account for this event".to_string(),
                });
            }
        }
        outcome
    }
}

fn refusal_of(error: &GuiBridgeError) -> ReplacementsRefused {
    let (code, message) = match error {
        GuiBridgeError::Refused { code, message } => (code.as_str(), message.clone()),
        GuiBridgeError::Transport(_) | GuiBridgeError::Wait(_) => {
            ("unreachable", error.to_string())
        }
        GuiBridgeError::InvalidResponse(_) => ("invalid_response", error.to_string()),
        GuiBridgeError::SignInEnded(_) => ("sign_in_ended", error.to_string()),
        _ => ("service_error", error.to_string()),
    };
    ReplacementsRefused {
        code: code.to_string(),
        message,
    }
}

/// Writes what the person chose: the additions, then the replacements.
///
/// # Errors
///
/// [`ApplyError::Invalid`] before anything is sent; [`ApplyError::Service`]
/// when the additions could not be sent (what was answered is in `partial`,
/// a new call is safe). A refused rework is **not** an error: see
/// [`ApplyOutcome::replacements_refused`].
pub fn apply<T: IncidentsTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
    request: &Apply,
) -> Result<ApplyOutcome, ApplyError> {
    validate(request)?;
    let session_id = request.session_id.as_str();

    // (1) The additions, idempotent, in batches that never cut a pause from
    // its resume.
    let prepared = prepare(session_id, &request.adds);
    let mut answers = Answers::default();
    let mut answered = 0;
    let mut batch: Vec<Value> = Vec::new();
    let mut batch_items = 0;
    let mut chunks: Vec<(Vec<Value>, usize)> = Vec::new();
    for item in &prepared {
        if !batch.is_empty() && batch.len() + item.events.len() > MAX_ADDS_PER_BATCH {
            chunks.push((std::mem::take(&mut batch), batch_items));
            batch_items = 0;
        }
        batch.extend(item.events.iter().cloned());
        batch_items += 1;
    }
    if !batch.is_empty() {
        chunks.push((batch, batch_items));
    }
    for (events, items) in chunks {
        match transport.append_events(base_url, token, session_id, &events) {
            Ok(answer) => {
                answers.read(&answer);
                answered += items;
            }
            Err(source) => {
                return Err(ApplyError::Service {
                    source,
                    partial: Box::new(answers.outcome(&prepared[..answered])),
                });
            }
        }
    }
    let mut outcome = answers.outcome(&prepared[..answered]);

    // (2) The replacements, all or nothing.
    if request.replacements.is_empty() {
        return Ok(outcome);
    }
    let revision = match transport.rework_revision(base_url, token, session_id) {
        Ok(revision) => revision,
        Err(error) => {
            outcome.replacements_refused = Some(refusal_of(&error));
            return Ok(outcome);
        }
    };
    let replacements: Vec<Value> = request
        .replacements
        .iter()
        .map(|r| {
            let mut event = json!({
                "type": r.event_type.as_str(),
                "at": r.at,
                "payload": r.payload,
            });
            if let Some(until) = &r.until {
                event["until"] = json!(until);
            }
            json!({ "eventId": r.event_id, "event": event })
        })
        .collect();
    let body = json!({ "eventReplacements": replacements });
    match transport.put_rework(base_url, token, session_id, revision, &body) {
        Ok(_) => {
            outcome.replaced = request
                .replacements
                .iter()
                .map(|r| r.event_id.clone())
                .collect();
        }
        Err(error) => outcome.replacements_refused = Some(refusal_of(&error)),
    }
    Ok(outcome)
}

fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    if max == 0 {
        return String::new();
    }
    // The characters, not the bytes: never in the middle of a code point.
    let mut kept: String = text.chars().take(max - 1).collect();
    kept.push('\u{2026}');
    kept
}

/// The text of a replaced signal: the person's own text first, the detected
/// text after it, at most `max` characters (the schema's limit: 300 for an
/// incident, 200 for the reason of a pause, 500 for a note). An empty side is
/// left out with its separator; a text too long is cut at a character
/// boundary and ends with `…`, so the person's text is the last to lose a
/// character.
#[must_use]
pub fn merged_text(user: &str, detected: &str, max: usize) -> String {
    merged_text_with(user, detected, DETECTED_SEPARATOR, max)
}

/// [`merged_text`] with the separator of the person's language.
#[must_use]
pub fn merged_text_with(user: &str, detected: &str, separator: &str, max: usize) -> String {
    let (user, detected) = (user.trim(), detected.trim());
    let whole = match (user.is_empty(), detected.is_empty()) {
        (true, _) => detected.to_string(),
        (false, true) => user.to_string(),
        (false, false) => format!("{user}{separator}{detected}"),
    };
    cut(&whole, max)
}

fn text_of<'a>(payload: &'a Value, field: &str) -> &'a str {
    payload.get(field).and_then(Value::as_str).unwrap_or("")
}

/// The payload of the signal that replaces a declared one: the detected
/// payload, with the person's text kept in front of the detected text
/// ([`merged_text`]) in the one field that carries text (`reason` of a
/// pause: 200, `description` of an incident: 300, `notes` of a condition:
/// 300, `text` of a note: 500), and what only the person said (the
/// equipment of an incident, the fields of a condition the detection leaves
/// empty) kept.
#[must_use]
pub fn merged_payload(kind: IncidentKind, user: &Value, detected: &Value) -> Value {
    let (field, max) = match kind {
        IncidentKind::Pause => ("reason", 200),
        IncidentKind::EquipmentIssue => ("description", 300),
        IncidentKind::Condition => ("notes", 300),
        IncidentKind::Note => ("text", 500),
        _ => return detected.clone(),
    };
    let mut merged = detected.clone();
    if let (Some(target), Some(own)) = (merged.as_object_mut(), user.as_object()) {
        for (name, value) in own {
            if name != field && !target.contains_key(name) {
                target.insert(name.clone(), value.clone());
            }
        }
    }
    let text = merged_text(text_of(user, field), text_of(detected, field), max);
    if !text.is_empty() {
        merged[field] = json!(text);
    }
    merged
}

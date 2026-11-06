//! The signals already in the session (`GET /v1/sessions/{id}/events`), and
//! the one list they make with the detections (one merged list, sorted by
//! time).
//!
//! The read is the **effective** one: an event a rework replaced is hidden by
//! the service (`includeReplaced` is never asked) and is not read here even
//! if one slips through. A `pause` and the `resume` that follows it are one
//! interval; the `resume` is never a line. A type the list does not show, or
//! one a newer service stores, is skipped, never an error.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

use super::{DetectedIncident, IncidentKind, event_id_for};
use crate::gui_bridge::{GuiBridgeError, HttpPortalGuiTransport};
use crate::night_service::route;
use crate::service_wait::RequestFactory;
use crate::time_util::parse_iso8601_utc_to_unix_seconds;

/// Two incidents of one kind this close (or overlapping) seem to go together
/// (ten minutes, to be tuned on the real nights).
pub const PAIR_PROXIMITY_SECONDS: f64 = 600.0;
/// The most a page of events holds (the service's cap).
const PAGE_LIMIT: u32 = 200;
/// The pages read before giving up (a session holds at most a few thousand
/// events).
const MAX_PAGES: usize = 200;

/// The routes of the list, by a device token, behind a trait so that tests
/// never touch the network.
pub trait IncidentsTransport {
    /// `GET /v1/sessions/{id}/events?since=&limit=`: a page, as the service
    /// writes it (`session-event-ingest.schema.json`, `eventPage`).
    fn events_page(
        &self,
        base_url: &str,
        token: &str,
        session_id: &str,
        since: Option<&str>,
    ) -> Result<Value, GuiBridgeError>;
    /// `POST /v1/sessions/{id}/events` with `{ events }`: the per-event
    /// classification (`accepted`, `duplicate`, `rejected`). Harmless to
    /// repeat.
    fn append_events(
        &self,
        base_url: &str,
        token: &str,
        session_id: &str,
        events: &[Value],
    ) -> Result<Value, GuiBridgeError>;
    /// The `reworkRevision` of the session (`GET /v1/sessions/{id}`), what
    /// `If-Match` of the next rework must say.
    fn rework_revision(
        &self,
        base_url: &str,
        token: &str,
        session_id: &str,
    ) -> Result<u64, GuiBridgeError>;
    /// `PUT /v1/sessions/{id}/rework` with `If-Match: "<revision>"`. Sent
    /// once: a second sending after a success would be refused (the revision
    /// moved).
    fn put_rework(
        &self,
        base_url: &str,
        token: &str,
        session_id: &str,
        revision: u64,
        body: &Value,
    ) -> Result<Value, GuiBridgeError>;
}

impl IncidentsTransport for HttpPortalGuiTransport {
    fn events_page(
        &self,
        base_url: &str,
        token: &str,
        session_id: &str,
        since: Option<&str>,
    ) -> Result<Value, GuiBridgeError> {
        let url = route(base_url, &format!("/v1/sessions/{session_id}/events"));
        let token = token.to_owned();
        let mut query: Vec<(&'static str, String)> = vec![("limit", PAGE_LIMIT.to_string())];
        if let Some(since) = since {
            query.push(("since", since.to_string()));
        }
        let build: RequestFactory =
            Arc::new(move |client| client.get(url.clone()).bearer_auth(&token).query(&query));
        self.read_json(&build)
    }

    fn append_events(
        &self,
        base_url: &str,
        token: &str,
        session_id: &str,
        events: &[Value],
    ) -> Result<Value, GuiBridgeError> {
        let url = route(base_url, &format!("/v1/sessions/{session_id}/events"));
        let token = token.to_owned();
        let body = serde_json::json!({ "events": events });
        let build: RequestFactory =
            Arc::new(move |client| client.post(url.clone()).bearer_auth(&token).json(&body));
        // Idempotent by `eventId`: sent again freely while the service wakes.
        self.read_json(&build)
    }

    fn rework_revision(
        &self,
        base_url: &str,
        token: &str,
        session_id: &str,
    ) -> Result<u64, GuiBridgeError> {
        let url = route(base_url, &format!("/v1/sessions/{session_id}"));
        let token = token.to_owned();
        let build: RequestFactory =
            Arc::new(move |client| client.get(url.clone()).bearer_auth(&token));
        let detail: Value = self.read_json(&build)?;
        detail
            .pointer("/session/reworkRevision")
            .or_else(|| detail.get("reworkRevision"))
            .and_then(Value::as_u64)
            .ok_or_else(|| GuiBridgeError::InvalidResponse("no reworkRevision".to_string()))
    }

    fn put_rework(
        &self,
        base_url: &str,
        token: &str,
        session_id: &str,
        revision: u64,
        body: &Value,
    ) -> Result<Value, GuiBridgeError> {
        let url = route(base_url, &format!("/v1/sessions/{session_id}/rework"));
        let token = token.to_owned();
        let body = body.clone();
        let if_match = format!("\"{revision}\"");
        self.write_json_once(base_url, &move |client| {
            client
                .put(url.clone())
                .bearer_auth(&token)
                .header("If-Match", if_match.clone())
                .json(&body)
        })
    }
}

/// What a rework did to a stored event (`rework`, `session-event-ingest.schema.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ReworkMark {
    /// An event a rework added (or the replacement of one).
    pub added: bool,
    pub target_id: Option<String>,
    /// The event this one replaces.
    pub replaces: Option<String>,
    pub replaced_by: Option<String>,
    pub removed: bool,
}

/// A signal of the session, as the service stores it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeclaredEvent {
    pub event_id: String,
    #[serde(rename = "type")]
    pub event_type: IncidentKind,
    pub recorded_at: String,
    pub payload: Value,
    /// `declared` or `astro-sync` (set by the service); none: a service that
    /// predates the field.
    pub source: Option<String>,
    /// `observed`, `planned` or `reconstructed`.
    pub timing: Option<String>,
    pub rework: Option<ReworkMark>,
    /// The target the event is posed on, when it says one: the rework's
    /// placement, else the payload's.
    pub target_id: Option<String>,
    /// A pause: the instant of the `resume` that closed it.
    pub until: Option<String>,
    pub resume_event_id: Option<String>,
}

impl DeclaredEvent {
    /// Written by Astro Sync (a detection added before), not by a person.
    #[must_use]
    pub fn by_astro_sync(&self) -> bool {
        self.source.as_deref() == Some("astro-sync")
    }

    /// The text the event carries, for a line of the list.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        ["reason", "description", "text", "notes"]
            .iter()
            .find_map(|field| self.payload.get(*field).and_then(Value::as_str))
    }
}

/// The signals read, and whether the read was cut.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeclaredEvents {
    /// By time: the pauses (with their end), the conditions, the equipment
    /// issues, the notes, and the four kinds Astro Sync detects.
    pub events: Vec<DeclaredEvent>,
    /// The service said there was more and the cursor could not go further
    /// (more events at one instant than a page holds): the list may be short.
    pub truncated: bool,
}

fn unix_of(text: &str) -> Option<f64> {
    parse_iso8601_utc_to_unix_seconds(text).filter(|t| t.is_finite())
}

fn string_of(value: &Value, field: &str) -> Option<String> {
    value.get(field).and_then(Value::as_str).map(str::to_string)
}

/// A stored event that is effective (a replaced or removed one is not).
fn is_hidden(event: &Value) -> bool {
    let rework = event.get("rework");
    rework
        .and_then(|r| r.get("replacedBy"))
        .is_some_and(|v| !v.is_null())
        || rework
            .and_then(|r| r.get("removed"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

/// The signals of a session from the pages of its events (pure: the pages
/// are the ones [`read_declared_events`] read).
#[must_use]
pub fn declared_from_events(raw: &[Value]) -> Vec<DeclaredEvent> {
    let mut ordered: Vec<(f64, &Value)> = raw
        .iter()
        .filter(|event| !is_hidden(event))
        .filter_map(|event| {
            let at = unix_of(event.get("recordedAt")?.as_str()?)?;
            Some((at, event))
        })
        .collect();
    ordered.sort_by(|a, b| {
        a.0.total_cmp(&b.0).then_with(|| {
            string_of(a.1, "eventId")
                .unwrap_or_default()
                .cmp(&string_of(b.1, "eventId").unwrap_or_default())
        })
    });

    let mut events: Vec<DeclaredEvent> = Vec::new();
    // The pause a `resume` would close, by session order.
    let mut open: Option<usize> = None;
    for (_, event) in ordered {
        let Some(event_id) = string_of(event, "eventId") else {
            continue;
        };
        let event_type = string_of(event, "type").unwrap_or_default();
        let recorded_at = string_of(event, "recordedAt").unwrap_or_default();
        if event_type == "resume" {
            if let Some(index) = open.take() {
                events[index].until = Some(recorded_at);
                events[index].resume_event_id = Some(event_id);
            }
            continue;
        }
        let Some(kind) = IncidentKind::parse(&event_type) else {
            continue;
        };
        let rework: Option<ReworkMark> = event
            .get("rework")
            .and_then(|r| serde_json::from_value(r.clone()).ok());
        let payload = event.get("payload").cloned().unwrap_or(Value::Null);
        let target_id = match &rework {
            Some(mark) => mark.target_id.clone(),
            None => string_of(&payload, "targetId"),
        };
        events.push(DeclaredEvent {
            event_id,
            event_type: kind,
            recorded_at,
            payload,
            source: string_of(event, "source"),
            timing: string_of(event, "timing"),
            rework,
            target_id,
            until: None,
            resume_event_id: None,
        });
        if kind == IncidentKind::Pause {
            open = Some(events.len() - 1);
        }
    }
    events
}

/// Reads the signals of a session, page after page (`since` is inclusive: a
/// row can come twice and is kept once).
///
/// # Errors
///
/// What the service or the network answered when a page could not be read.
pub fn read_declared_events<T: IncidentsTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
    session_id: &str,
) -> Result<DeclaredEvents, GuiBridgeError> {
    let mut raw: Vec<Value> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut since: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let page = transport.events_page(base_url, token, session_id, since.as_deref())?;
        let mut fresh = 0;
        for event in page
            .get("events")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(id) = event.get("eventId").and_then(Value::as_str)
                && seen.insert(id.to_string())
            {
                raw.push(event.clone());
                fresh += 1;
            }
        }
        if !page
            .get("hasMore")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Ok(DeclaredEvents {
                events: declared_from_events(&raw),
                truncated: false,
            });
        }
        match string_of(&page, "nextSince") {
            Some(next) if fresh > 0 => since = Some(next),
            // The cursor cannot go further: say the read is short.
            _ => break,
        }
    }
    Ok(DeclaredEvents {
        events: declared_from_events(&raw),
        truncated: true,
    })
}

/// Whose line it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowOrigin {
    /// Declared by a person during the execution (or a rework).
    Declared,
    /// Detected: a proposal, or an event Astro Sync already added.
    Detected,
}

/// One line of the merged list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergedRow {
    pub origin: RowOrigin,
    pub kind: IncidentKind,
    /// The `eventId` of a stored event, the detection key of a proposal.
    pub id: String,
    pub at: String,
    pub until: Option<String>,
    pub text: Option<String>,
    /// A detected line that is already in the session (Astro Sync added it
    /// before): nothing to propose.
    pub already_added: bool,
    /// The line of the other origin and the same kind that seems to go with
    /// this one (within [`PAIR_PROXIMITY_SECONDS`], or overlapping).
    pub near: Option<String>,
}

/// The distance between two intervals (zero when they overlap), in seconds.
fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0.max(b.0) - a.1.min(b.1)).max(0.0)
}

fn interval(at: &str, until: Option<&str>) -> Option<(f64, f64)> {
    let start = unix_of(at)?;
    Some((start, until.and_then(unix_of).unwrap_or(start).max(start)))
}

/// The declared signals and the detections, in one list by time. A detection
/// that Astro Sync already wrote to the session (its derived `eventId` is
/// there) is not proposed again: the stored line stands for it, as
/// `Detected` and `already_added`. A proposal and a declared signal of the
/// same kind close to each other point at one another through `near`.
#[must_use]
pub fn merge_rows(
    session_id: &str,
    declared: &[DeclaredEvent],
    detected: &[DetectedIncident],
) -> Vec<MergedRow> {
    let stored: HashSet<&str> = declared.iter().map(|e| e.event_id.as_str()).collect();
    let mut rows: Vec<MergedRow> = declared
        .iter()
        .map(|event| MergedRow {
            origin: if event.by_astro_sync() {
                RowOrigin::Detected
            } else {
                RowOrigin::Declared
            },
            kind: event.event_type,
            id: event.event_id.clone(),
            at: event.recorded_at.clone(),
            until: event.until.clone(),
            text: event.text().map(str::to_string),
            already_added: event.by_astro_sync(),
            near: None,
        })
        .collect();
    for incident in detected {
        if stored.contains(event_id_for(session_id, &incident.id).as_str()) {
            continue;
        }
        rows.push(MergedRow {
            origin: RowOrigin::Detected,
            kind: incident.event_type,
            id: incident.id.clone(),
            at: incident.start.clone(),
            until: incident.end.clone(),
            text: Some(incident.description.clone()),
            already_added: false,
            near: None,
        });
    }
    // The pairs: a declared line (a person's) and a proposal of one kind.
    let spans: Vec<Option<(f64, f64)>> = rows
        .iter()
        .map(|row| interval(&row.at, row.until.as_deref()))
        .collect();
    for i in 0..rows.len() {
        if rows[i].already_added {
            continue;
        }
        let best = (0..rows.len())
            .filter(|&j| {
                j != i
                    && rows[j].kind == rows[i].kind
                    && rows[j].origin != rows[i].origin
                    && !rows[j].already_added
            })
            .filter_map(|j| {
                let gap = distance(spans[i]?, spans[j]?);
                (gap <= PAIR_PROXIMITY_SECONDS).then_some((gap, j))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        rows[i].near = best.map(|(_, j)| rows[j].id.clone());
    }
    let mut keyed: Vec<(f64, MergedRow)> = rows
        .into_iter()
        .map(|row| (unix_of(&row.at).unwrap_or(f64::MAX), row))
        .collect();
    keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)));
    keyed.into_iter().map(|(_, row)| row).collect()
}

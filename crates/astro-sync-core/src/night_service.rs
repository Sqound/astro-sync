//! A night of the space read and created by Astro Sync (the
//! "Nights" screen and its "Create a night" window: a night created by
//! `POST /v1/sessions`, `PATCH …/night`).
//!
//! - [`NightServiceTransport::session_detail`]: `GET /v1/sessions/{id}` by a
//!   device token -- the structured night (`night`: title, place, target
//!   ids) and the stored debriefing, passed to the window as the service
//!   wrote them (`session.schema.json`, `detail`). The window reads only the
//!   fields it shows, so a field the service adds later breaks nothing.
//! - [`create_night`]: a direct night (no plan) created by `POST
//!   /v1/sessions` with a fresh `clientSessionId` (the service answers the
//!   same night to a repeat), then named and placed by `PATCH
//!   /v1/sessions/{id}/night`, and finished by `POST …/finish` when an end
//!   hour is given (the service refuses a `finishedAt` on an open night).
//!   Nothing is written on disk: the night's folder is created at the
//!   wizard's "Folder" step.
//!
//! Every request here is harmless to repeat (idempotent by
//! `clientSessionId`, a `PATCH` of the same fields, a finish that never moves
//! a recorded end), so each goes through the patient read of the transport.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use thiserror::Error;

use crate::gui_bridge::{GuiBridgeError, HttpPortalGuiTransport};
use crate::service_wait::RequestFactory;
use crate::time_util::{civil_from_days, days_from_civil};

/// What the "Create a night" window sends (the form's fields, word for word: a
/// date, a saved place, optional hours in UTC, an optional name).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NightCreate {
    /// The evening of the night, `YYYY-MM-DD`.
    pub evening_date: String,
    /// `HH:MM` in UTC, optional; before 12:00 it is the next morning's.
    pub start: Option<String>,
    /// `HH:MM` in UTC, optional; before the start, it is the next morning.
    pub end: Option<String>,
    /// The place's name (a place of the space).
    pub place: String,
    /// The night's name; none: the screens title "Night of Wed. 7 Oct." (French in the app).
    pub title: Option<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum NightCreateError {
    #[error("the evening date must read AAAA-MM-JJ, got {0:?}")]
    BadDate(String),
    #[error("an hour must read HH:MM, got {0:?}")]
    BadHour(String),
    #[error("the place must not be empty")]
    NoPlace,
}

/// The three bodies of a creation: `POST /v1/sessions`, `PATCH …/night`,
/// and `POST …/finish` when the end is known.
#[derive(Debug, Clone, PartialEq)]
pub struct NightCreateRequests {
    pub create: Value,
    pub patch: Value,
    pub finish: Option<Value>,
}

fn parse_date(text: &str) -> Option<(i64, u32, u32)> {
    let mut parts = text.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || text.len() != 10 || !(1..=12).contains(&month) {
        return None;
    }
    let days = days_from_civil(year, month, day);
    // A day past the month's end (« 2026-02-30 ») does not round-trip.
    (civil_from_days(days) == (year, month, day)).then_some((year, month, day))
}

fn parse_hour(text: &str) -> Option<(u32, u32)> {
    let (hours, minutes) = text.split_once(':')?;
    if hours.len() != 2 || minutes.len() != 2 {
        return None;
    }
    let hours: u32 = hours.parse().ok()?;
    let minutes: u32 = minutes.parse().ok()?;
    (hours < 24 && minutes < 60).then_some((hours, minutes))
}

fn instant(days: i64, (hours, minutes): (u32, u32)) -> String {
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:00.000Z")
}

/// The bodies of a creation, checked before anything leaves. Without a start
/// hour the night starts at noon UTC on its evening date: the same calendar
/// day wherever the person reads it, until the frames say better. A start
/// hour before noon is the next morning's (a night begun after midnight UTC),
/// and the end is read against the day the night really started.
pub fn creation_requests(
    spec: &NightCreate,
    client_session_id: &str,
) -> Result<NightCreateRequests, NightCreateError> {
    let (year, month, day) = parse_date(&spec.evening_date)
        .ok_or_else(|| NightCreateError::BadDate(spec.evening_date.clone()))?;
    let place = spec.place.trim();
    if place.is_empty() {
        return Err(NightCreateError::NoPlace);
    }
    let hour = |value: &Option<String>| -> Result<Option<(u32, u32)>, NightCreateError> {
        match value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            None => Ok(None),
            Some(text) => parse_hour(text)
                .map(Some)
                .ok_or_else(|| NightCreateError::BadHour(text.to_string())),
        }
    };
    let start = hour(&spec.start)?;
    let end = hour(&spec.end)?;
    let evening = days_from_civil(year, month, day);
    let start_at = start.unwrap_or((12, 0));
    // Before noon UTC an hour cannot be the evening's own: it is the
    // morning after.
    let start_day = evening + i64::from(start.is_some_and(|(hours, _)| hours < 12));
    let mut patch = json!({ "place": { "name": place } });
    if let Some(title) = spec
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        patch["title"] = json!(title);
    }
    let finish = end.map(|end| {
        let next_morning = end <= start_at;
        json!({ "finishedAt": instant(start_day + i64::from(next_morning), end) })
    });
    Ok(NightCreateRequests {
        create: json!({
            "startedAt": instant(start_day, start_at),
            "clientSessionId": client_session_id,
        }),
        patch,
        finish,
    })
}

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A ULID (`session.schema.json`'s `clientSessionId`): the time in
/// milliseconds, then 80 random bits from the operating system.
#[must_use]
pub fn new_client_session_id(unix_millis: u64, random: [u8; 10]) -> String {
    let mut value: u128 = u128::from(unix_millis & 0xFFFF_FFFF_FFFF) << 80;
    for (index, byte) in random.iter().enumerate() {
        value |= u128::from(*byte) << (8 * (9 - index));
    }
    (0..26)
        .rev()
        .map(|position| CROCKFORD[((value >> (5 * position)) & 0x1F) as usize] as char)
        .collect()
}

/// A fresh `clientSessionId`: now, and random bytes from the operating
/// system (the clock alone if they cannot be read).
#[must_use]
pub fn fresh_client_session_id() -> String {
    let mut random = [0u8; 10];
    let _ = getrandom::getrandom(&mut random);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default();
    new_client_session_id(millis, random)
}

/// The service's routes of a night, by a device token.
pub trait NightServiceTransport {
    fn session_detail(
        &self,
        base_url: &str,
        token: &str,
        session_id: &str,
    ) -> Result<Value, GuiBridgeError>;
    fn create_session(
        &self,
        base_url: &str,
        token: &str,
        body: &Value,
    ) -> Result<Value, GuiBridgeError>;
    fn patch_night(
        &self,
        base_url: &str,
        token: &str,
        session_id: &str,
        changes: &Value,
    ) -> Result<Value, GuiBridgeError>;
    fn finish_session(
        &self,
        base_url: &str,
        token: &str,
        session_id: &str,
        body: &Value,
    ) -> Result<Value, GuiBridgeError>;
}

/// `<service>/api<path>`: the address of a route of the service.
pub(crate) fn route(base_url: &str, path: &str) -> String {
    format!("{}/api{path}", base_url.trim_end_matches('/'))
}

impl NightServiceTransport for HttpPortalGuiTransport {
    fn session_detail(
        &self,
        base_url: &str,
        token: &str,
        session_id: &str,
    ) -> Result<Value, GuiBridgeError> {
        let url = route(base_url, &format!("/v1/sessions/{session_id}"));
        let token = token.to_owned();
        let build: RequestFactory =
            Arc::new(move |client| client.get(url.clone()).bearer_auth(&token));
        self.read_json(&build)
    }

    fn create_session(
        &self,
        base_url: &str,
        token: &str,
        body: &Value,
    ) -> Result<Value, GuiBridgeError> {
        let url = route(base_url, "/v1/sessions");
        let token = token.to_owned();
        let body = body.clone();
        let build: RequestFactory =
            Arc::new(move |client| client.post(url.clone()).bearer_auth(&token).json(&body));
        self.read_json(&build)
    }

    fn patch_night(
        &self,
        base_url: &str,
        token: &str,
        session_id: &str,
        changes: &Value,
    ) -> Result<Value, GuiBridgeError> {
        let url = route(base_url, &format!("/v1/sessions/{session_id}/night"));
        let token = token.to_owned();
        let body = changes.clone();
        let build: RequestFactory =
            Arc::new(move |client| client.patch(url.clone()).bearer_auth(&token).json(&body));
        self.read_json(&build)
    }

    fn finish_session(
        &self,
        base_url: &str,
        token: &str,
        session_id: &str,
        body: &Value,
    ) -> Result<Value, GuiBridgeError> {
        let url = route(base_url, &format!("/v1/sessions/{session_id}/finish"));
        let token = token.to_owned();
        let body = body.clone();
        let build: RequestFactory =
            Arc::new(move |client| client.post(url.clone()).bearer_auth(&token).json(&body));
        self.read_json(&build)
    }
}

#[derive(Debug, Error)]
pub enum CreateNightFailure {
    #[error(transparent)]
    Invalid(#[from] NightCreateError),
    #[error(transparent)]
    Service(#[from] GuiBridgeError),
}

/// "Create this night": the night created, named, placed (and finished when
/// its end is known); answers the night's detail as `GET /v1/sessions/{id}`
/// gives it.
pub fn create_night<T: NightServiceTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
    spec: &NightCreate,
    client_session_id: &str,
) -> Result<Value, CreateNightFailure> {
    let requests = creation_requests(spec, client_session_id)?;
    let created = transport.create_session(base_url, token, &requests.create)?;
    let id = created
        .pointer("/session/id")
        .and_then(Value::as_str)
        .ok_or_else(|| GuiBridgeError::InvalidResponse("no session id".to_string()))?
        .to_string();
    if let Some(finish) = &requests.finish {
        transport.finish_session(base_url, token, &id, finish)?;
    }
    transport.patch_night(base_url, token, &id, &requests.patch)?;
    Ok(transport.session_detail(base_url, token, &id)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn spec() -> NightCreate {
        NightCreate {
            evening_date: "2026-10-07".to_string(),
            start: Some("21:00".to_string()),
            end: Some("04:00".to_string()),
            place: " Col des Étoiles ".to_string(),
            title: Some("Night of 7 Oct".to_string()),
        }
    }

    #[test]
    fn the_window_s_form_gives_the_three_bodies_and_an_end_before_the_start_is_the_next_morning() {
        let requests = creation_requests(&spec(), "01JABCDEFGHJKMNPQRSTVWXYZ0").unwrap();
        assert_eq!(
            requests.create,
            json!({ "startedAt": "2026-10-07T21:00:00.000Z", "clientSessionId": "01JABCDEFGHJKMNPQRSTVWXYZ0" })
        );
        assert_eq!(
            requests.patch,
            json!({ "place": { "name": "Col des Étoiles" }, "title": "Night of 7 Oct" })
        );
        assert_eq!(
            requests.finish,
            Some(json!({ "finishedAt": "2026-10-08T04:00:00.000Z" }))
        );
    }

    #[test]
    fn a_start_before_noon_is_the_next_morning_and_the_end_follows_the_real_start_day() {
        // « 02:00 » on the evening of the 7th is the morning of the 8th.
        let mut after_midnight = spec();
        after_midnight.start = Some("02:00".to_string());
        after_midnight.end = Some("05:00".to_string());
        let requests = creation_requests(&after_midnight, "x").unwrap();
        assert_eq!(requests.create["startedAt"], "2026-10-08T02:00:00.000Z");
        assert_eq!(
            requests.finish,
            Some(json!({ "finishedAt": "2026-10-08T05:00:00.000Z" }))
        );
        // Without an end the night stays open.
        after_midnight.end = None;
        let requests = creation_requests(&after_midnight, "x").unwrap();
        assert_eq!(requests.create["startedAt"], "2026-10-08T02:00:00.000Z");
        assert_eq!(requests.finish, None);
        // An evening start with an end past midnight is unchanged.
        let mut evening = spec();
        evening.start = Some("22:00".to_string());
        evening.end = Some("03:00".to_string());
        let requests = creation_requests(&evening, "x").unwrap();
        assert_eq!(requests.create["startedAt"], "2026-10-07T22:00:00.000Z");
        assert_eq!(
            requests.finish,
            Some(json!({ "finishedAt": "2026-10-08T03:00:00.000Z" }))
        );
        // Noon itself is still the evening's own day.
        let mut noon = spec();
        noon.start = Some("12:00".to_string());
        noon.end = None;
        let requests = creation_requests(&noon, "x").unwrap();
        assert_eq!(requests.create["startedAt"], "2026-10-07T12:00:00.000Z");
    }

    #[test]
    fn without_hours_the_night_starts_at_noon_utc_and_stays_open_and_a_bad_field_is_refused() {
        let mut bare = spec();
        bare.start = None;
        bare.end = Some(String::new());
        bare.title = Some("  ".to_string());
        let requests = creation_requests(&bare, "x").unwrap();
        assert_eq!(requests.create["startedAt"], "2026-10-07T12:00:00.000Z");
        assert_eq!(requests.finish, None);
        assert_eq!(
            requests.patch,
            json!({ "place": { "name": "Col des Étoiles" } })
        );

        let mut wrong = spec();
        wrong.evening_date = "2026-02-30".to_string();
        assert!(matches!(
            creation_requests(&wrong, "x"),
            Err(NightCreateError::BadDate(_))
        ));
        let mut wrong = spec();
        wrong.start = Some("25:00".to_string());
        assert!(matches!(
            creation_requests(&wrong, "x"),
            Err(NightCreateError::BadHour(_))
        ));
        let mut wrong = spec();
        wrong.place = " ".to_string();
        assert_eq!(
            creation_requests(&wrong, "x"),
            Err(NightCreateError::NoPlace)
        );
    }

    #[test]
    fn a_client_session_id_is_a_ulid() {
        let id = new_client_session_id(1_759_870_800_000, [0xFF; 10]);
        assert_eq!(id.len(), 26);
        assert!(id.bytes().all(|b| CROCKFORD.contains(&b)));
        assert!(id.ends_with("ZZZZZZZZZZZZZZZZ"));
        assert_eq!(new_client_session_id(0, [0; 10]), "0".repeat(26));
    }

    #[derive(Default)]
    struct FakeService {
        calls: RefCell<Vec<(String, Value)>>,
    }

    impl NightServiceTransport for FakeService {
        fn session_detail(&self, _: &str, _: &str, id: &str) -> Result<Value, GuiBridgeError> {
            self.calls
                .borrow_mut()
                .push((format!("GET {id}"), Value::Null));
            Ok(json!({ "session": { "id": id } }))
        }
        fn create_session(&self, _: &str, _: &str, body: &Value) -> Result<Value, GuiBridgeError> {
            self.calls
                .borrow_mut()
                .push(("POST".to_string(), body.clone()));
            Ok(json!({ "session": { "id": "night-9" }, "created": true }))
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
                .push((format!("PATCH {id}"), changes.clone()));
            Ok(Value::Null)
        }
        fn finish_session(
            &self,
            _: &str,
            _: &str,
            id: &str,
            body: &Value,
        ) -> Result<Value, GuiBridgeError> {
            self.calls
                .borrow_mut()
                .push((format!("FINISH {id}"), body.clone()));
            Ok(Value::Null)
        }
    }

    #[test]
    fn a_night_is_created_finished_then_named_and_read_back() {
        let service = FakeService::default();
        let detail = create_night(&service, "https://s.example.test", "t", &spec(), "id").unwrap();
        assert_eq!(detail["session"]["id"], "night-9");
        let calls: Vec<String> = service
            .calls
            .borrow()
            .iter()
            .map(|(c, _)| c.clone())
            .collect();
        assert_eq!(
            calls,
            ["POST", "FINISH night-9", "PATCH night-9", "GET night-9"]
        );
        let mut wrong = spec();
        wrong.place = String::new();
        assert!(matches!(
            create_night(&service, "s", "t", &wrong, "id"),
            Err(CreateNightFailure::Invalid(NightCreateError::NoPlace))
        ));
    }
}

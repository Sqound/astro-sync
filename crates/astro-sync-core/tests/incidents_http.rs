//! "Incident list" on the real HTTP transport, against the fake service: the
//! events read by `GET /v1/sessions/{id}/events`, the additions by
//! `POST …/events`, the replacements by `PUT …/rework` with the `If-Match`
//! of the `reworkRevision` read just before, and a refusal of the rework
//! kept as an answer. Nothing leaves the machine.

#[path = "support/fake_service.rs"]
mod fake_service;

use std::time::Duration;

use serde_json::{Value, json};

use astro_sync_core::gui_bridge::HttpPortalGuiTransport;
use astro_sync_core::incidents::{
    AddedIncident, Apply, IncidentKind, Replacement, apply, event_id_for, read_declared_events,
};
use astro_sync_core::service_wait::{WaitHooks, WakePolicy};
use fake_service::{FakeService, Reply, Wake, error_reply};

const SESSION: &str = "4d2b7e10-8a6c-4f53-9b1d-7e0a3c5f2b84";

fn fast() -> WakePolicy {
    WakePolicy {
        total_budget: Duration::from_millis(4_000),
        attempt_timeout: Duration::from_millis(800),
        write_timeout: Duration::from_millis(1_000),
        chunk_timeout: Duration::from_millis(1_500),
        retry_pause: Duration::from_millis(40),
        notice_after: Duration::from_millis(120),
        connect_timeout: Duration::from_millis(500),
    }
}

fn transport() -> HttpPortalGuiTransport {
    HttpPortalGuiTransport::with_options(fast(), WaitHooks::silent()).unwrap()
}

fn routes(rework: Reply) -> impl Fn(&fake_service::Recorded) -> Reply + Send + Sync + 'static {
    move |request| {
        let path = request.path.split('?').next().unwrap_or_default();
        match (request.method.as_str(), path) {
            ("GET", p) if p == format!("/api/v1/sessions/{SESSION}/events") => Reply::Json(
                200,
                json!({
                    "sessionId": SESSION,
                    "events": [
                        {
                            "eventId": "E2", "sessionId": SESSION, "type": "pause",
                            "recordedAt": "2026-08-14T22:14:00.000Z",
                            "receivedAt": "2026-08-15T07:00:00.000Z",
                            "originDeviceId": null, "source": "declared",
                            "payload": { "reason": "Flip" }
                        },
                        {
                            "eventId": "E3", "sessionId": SESSION, "type": "resume",
                            "recordedAt": "2026-08-14T22:26:00.000Z",
                            "receivedAt": "2026-08-15T07:00:00.000Z",
                            "originDeviceId": null, "payload": {}
                        }
                    ],
                    "nextSince": null,
                    "hasMore": false
                })
                .to_string(),
            ),
            ("POST", p) if p == format!("/api/v1/sessions/{SESSION}/events") => {
                let ids: Vec<Value> = request.json()["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|e| e["eventId"].clone())
                    .collect();
                Reply::Json(
                    200,
                    json!({ "sessionId": SESSION, "accepted": ids, "duplicate": [], "rejected": [] })
                        .to_string(),
                )
            }
            ("GET", p) if p == format!("/api/v1/sessions/{SESSION}") => Reply::Json(
                200,
                json!({ "session": { "id": SESSION, "reworkRevision": 4 }, "supplements": [] })
                    .to_string(),
            ),
            ("PUT", p) if p == format!("/api/v1/sessions/{SESSION}/rework") => rework.clone(),
            ("GET", "/api/v1/service") => fake_service::identity_reply(),
            _ => error_reply(404, "not_found", "Not found."),
        }
    }
}

fn request() -> Apply {
    Apply {
        session_id: SESSION.to_string(),
        adds: vec![AddedIncident {
            key: "pause:2026-08-14T22:13:38Z".to_string(),
            event_type: IncidentKind::Pause,
            at: "2026-08-14T22:13:38Z".to_string(),
            until: Some("2026-08-14T22:24:58Z".to_string()),
            payload: json!({ "reason": "Meridian flip", "cause": "meridian_flip" }),
        }],
        replacements: vec![Replacement {
            event_id: "E2".to_string(),
            event_type: IncidentKind::Pause,
            at: "2026-08-14T22:13:38Z".to_string(),
            until: Some("2026-08-14T22:24:58Z".to_string()),
            payload: json!({ "reason": "Flip" }),
        }],
    }
}

#[test]
fn the_declared_signals_are_read_with_the_device_bearer_and_a_pause_gets_its_resume() {
    let service = FakeService::start(Wake::Instant, routes(Reply::Json(200, "{}".to_string())));
    let read =
        read_declared_events(&transport(), &service.base_url(), "device-token", SESSION).unwrap();
    assert_eq!(read.events.len(), 1);
    assert_eq!(
        read.events[0].until.as_deref(),
        Some("2026-08-14T22:26:00.000Z")
    );
    let asked: Vec<_> = service
        .requests()
        .into_iter()
        .filter(|r| r.reached_app && r.path.contains("/events"))
        .collect();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].method, "GET");
    assert!(asked[0].path.contains("limit=200"), "{}", asked[0].path);
    assert!(!asked[0].path.contains("includeReplaced"));
    assert_eq!(
        asked[0].header("authorization"),
        Some("Bearer device-token")
    );
}

#[test]
fn the_additions_go_first_then_the_rework_with_the_revision_as_if_match() {
    let service = FakeService::start(Wake::Instant, routes(Reply::Json(200, "{}".to_string())));
    let outcome = apply(
        &transport(),
        &service.base_url(),
        "device-token",
        &request(),
    )
    .unwrap();
    assert_eq!(outcome.added.len(), 1);
    assert_eq!(outcome.replaced, ["E2"]);
    assert!(outcome.replacements_refused.is_none());

    let sent: Vec<_> = service
        .requests()
        .into_iter()
        .filter(|r| r.reached_app && r.path.starts_with("/api/v1/sessions/"))
        .collect();
    let order: Vec<String> = sent
        .iter()
        .map(|r| format!("{} {}", r.method, r.path.split('?').next().unwrap()))
        .collect();
    assert_eq!(
        order,
        [
            format!("POST /api/v1/sessions/{SESSION}/events"),
            format!("GET /api/v1/sessions/{SESSION}"),
            format!("PUT /api/v1/sessions/{SESSION}/rework"),
        ]
    );
    // The events: the envelope's four members, a deterministic identifier.
    let events = sent[0].json();
    let pause_id = event_id_for(SESSION, "pause:2026-08-14T22:13:38Z");
    assert_eq!(events["events"][0]["eventId"], pause_id);
    assert_eq!(events["events"][1]["type"], "resume");
    // The rework: `If-Match: "<reworkRevision>"`, quoted, as read.
    assert_eq!(sent[2].header("if-match"), Some("\"4\""));
    assert_eq!(sent[2].header("authorization"), Some("Bearer device-token"));
    assert_eq!(
        sent[2].json(),
        json!({ "eventReplacements": [{
            "eventId": "E2",
            "event": {
                "type": "pause",
                "at": "2026-08-14T22:13:38Z",
                "until": "2026-08-14T22:24:58Z",
                "payload": { "reason": "Flip" }
            }
        }] })
    );
}

#[test]
fn a_conflict_of_the_rework_is_an_answer_and_the_addition_stays_told() {
    let service = FakeService::start(
        Wake::Instant,
        routes(error_reply(
            409,
            "session_rework_conflict",
            "The session was reworked since.",
        )),
    );
    let outcome = apply(
        &transport(),
        &service.base_url(),
        "device-token",
        &request(),
    )
    .unwrap();
    assert_eq!(outcome.added.len(), 1);
    assert!(outcome.replaced.is_empty());
    let refused = outcome
        .replacements_refused
        .expect("a refusal is an answer");
    assert_eq!(refused.code, "session_rework_conflict");
    assert_eq!(refused.message, "The session was reworked since.");
}

//! « Création des sessions… » by packets, through the real HTTP client
//! (`HttpPortalGuiTransport`) against a fake service: the nights leave ten at
//! a time, a `503` `service_starting` with `Retry-After` is waited for that
//! long and the same packet sent again (never the packet before it), and a
//! `401` stops at once as the end of the sign-in it is.

#[path = "support/fake_service.rs"]
mod fake_service;

use std::cell::{Cell, RefCell};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use astro_sync_core::gui_bridge::{GuiBridgeError, HttpPortalGuiTransport};
use astro_sync_core::service_wait::{WaitHooks, WakePolicy};
use astro_sync_core::session_detection::create::packets::{
    NIGHTS_PER_PACKET, PacketEvent, PacketRun, SentPackets, create_sessions_by_packets,
};
use astro_sync_core::session_detection::{
    AcquisitionRow, BulkCreateRequest, DetectedSession, DetectedTarget, PlaceProposal, Proposal,
    ProposalLine, TargetSource,
};
use astro_sync_core::space::SignInEnd;
use fake_service::{FakeService, Recorded, Reply, Wake, error_reply};
use serde_json::{Value, json};

fn request(count: usize) -> BulkCreateRequest {
    let mut proposal = Proposal {
        places: vec![PlaceProposal {
            id: "home".into(),
            chosen: Some("place-home".into()),
            ..PlaceProposal::default()
        }],
        ..Proposal::default()
    };
    for index in 0..count {
        let id = format!("s-{index:03}");
        let day = format!("2026-03-{:02}", index + 1);
        proposal.sessions.push(DetectedSession {
            id: id.clone(),
            evening_date: day.clone(),
            place: "home".into(),
            start: format!("{day}T20:00:00.000Z"),
            end: format!("{day}T23:00:00.000Z"),
            targets: vec![DetectedTarget {
                key: "m42".into(),
                target_id: Some("m42".into()),
                designation: "M42".into(),
                source: TargetSource::Object,
                start: format!("{day}T20:00:00.000Z"),
                end: format!("{day}T23:00:00.000Z"),
                frames: 5,
                acquisition: vec![AcquisitionRow {
                    exposure_seconds: Some(30.0),
                    frames: 5,
                    total_seconds: 150.0,
                    ..AcquisitionRow::default()
                }],
                ..DetectedTarget::default()
            }],
            ..DetectedSession::default()
        });
        for line in [
            format!("session.create:{id}"),
            format!("target.detected:{id}/m42"),
        ] {
            proposal.lines.push(ProposalLine {
                id: line,
                checked: true,
                ..ProposalLine::default()
            });
        }
    }
    let ticked = proposal.lines.iter().map(|l| l.id.clone()).collect();
    BulkCreateRequest {
        proposal,
        ticked,
        places: Vec::new(),
        answers: Vec::new(),
        shifts: Vec::new(),
    }
}

/// The service's answer to a packet: every session created.
fn created(request: &Recorded) -> Reply {
    let body = request.json();
    let sessions: Vec<Value> = body["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            json!({
                "ref": s["ref"],
                "sessionId": format!("sess-{}", s["ref"].as_str().unwrap()),
                "action": "created",
                "planAction": "created",
                "replayed": false,
            })
        })
        .collect();
    Reply::Json(
        200,
        json!({ "importId": body["importId"], "replayed": false, "places": [], "sessions": sessions, "multiNight": [] })
            .to_string(),
    )
}

fn transport() -> HttpPortalGuiTransport {
    HttpPortalGuiTransport::with_options(
        WakePolicy {
            attempt_timeout: Duration::from_secs(5),
            ..WakePolicy::default()
        },
        WaitHooks::silent(),
    )
    .unwrap()
}

fn bulk_bodies(service: &FakeService) -> Vec<Value> {
    service
        .requests()
        .iter()
        .filter(|r| r.method == "POST" && r.path == "/api/v1/sessions/bulk" && r.reached_app)
        .map(Recorded::json)
        .collect()
}

#[test]
fn eleven_nights_leave_in_two_packets_and_a_busy_service_is_waited_for() {
    let calls = AtomicUsize::new(0);
    let service = FakeService::start(Wake::Instant, move |request| {
        if request.path != "/api/v1/sessions/bulk" {
            return error_reply(404, "not_found", "no such route");
        }
        // The second call (the second packet's first try) meets a busy service.
        if calls.fetch_add(1, Ordering::SeqCst) == 1 {
            return Reply::JsonWith(
                503,
                vec![
                    ("Retry-After".into(), "7".into()),
                    ("Cache-Control".into(), "no-store".into()),
                ],
                r#"{"error":{"code":"service_starting","message":"The service is starting. Try again in a moment.","retryAt":"2026-11-20T00:00:10.000Z"}}"#.into(),
            );
        }
        created(request)
    });
    let events = RefCell::new(Vec::new());
    let slept = Cell::new(Duration::ZERO);
    let memory = SentPackets::new();
    let outcome = {
        let mut on_event = |event| events.borrow_mut().push(event);
        let sleep = |d: Duration| slept.set(slept.get() + d);
        let mut run = PacketRun {
            on_event: &mut on_event,
            sleep: &sleep,
            memory: Some(&memory),
            size: NIGHTS_PER_PACKET,
        };
        create_sessions_by_packets(
            &transport(),
            &service.base_url(),
            "token",
            &request(11),
            &mut run,
            &mut |_| Ok::<(), GuiBridgeError>(()),
        )
        .unwrap()
    };
    assert_eq!(outcome.sessions.len(), 11);
    let bodies = bulk_bodies(&service);
    let sizes: Vec<usize> = bodies
        .iter()
        .map(|b| b["sessions"].as_array().unwrap().len())
        .collect();
    // 10, then the last night twice (the 503, then through).
    assert_eq!(sizes, vec![10, 1, 1]);
    assert_eq!(bodies[1], bodies[2]);
    assert_ne!(bodies[0]["importId"], bodies[1]["importId"]);
    // The `Retry-After` of 7 s is longer than the first step (5 s): 7 s.
    assert_eq!(slept.get(), Duration::from_secs(7));
    let waits: Vec<u64> = events
        .borrow()
        .iter()
        .filter_map(|e| match e {
            PacketEvent::Waiting { seconds } => Some(*seconds),
            PacketEvent::Sending { .. } => None,
        })
        .collect();
    assert_eq!(waits, vec![7, 6, 5, 4, 3, 2, 1]);
    assert_eq!(memory.len(), 2);
}

#[test]
fn a_401_ends_the_sign_in_at_the_first_packet_and_nothing_is_asked_again() {
    let service = FakeService::start(Wake::Instant, |request| {
        if request.path == "/api/v1/sessions/bulk" {
            return error_reply(401, "device_detached", "This device was detached.");
        }
        error_reply(404, "not_found", "no such route")
    });
    let slept = Mutex::new(Duration::ZERO);
    let sleep = |d: Duration| *slept.lock().unwrap() += d;
    let mut run = PacketRun {
        on_event: &mut |_| {},
        sleep: &sleep,
        memory: None,
        size: NIGHTS_PER_PACKET,
    };
    let result = create_sessions_by_packets(
        &transport(),
        &service.base_url(),
        "token",
        &request(25),
        &mut run,
        &mut |_| Ok::<(), GuiBridgeError>(()),
    );
    assert!(matches!(
        result,
        Err(GuiBridgeError::SignInEnded(SignInEnd::DeviceDetached))
    ));
    assert_eq!(bulk_bodies(&service).len(), 1);
    assert_eq!(*slept.lock().unwrap(), Duration::ZERO);
}

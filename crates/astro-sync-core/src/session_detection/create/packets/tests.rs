use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, VecDeque};

use super::*;
use crate::session_detection::create::{PlaceAnswer, create_sessions};
use crate::session_detection::{
    AcquisitionRow, DetectedSession, DetectedTarget, MultiNightGroup, MultiNightNight,
    PlaceProposal, Proposal, ProposalLine, TargetSource,
};
use crate::space::SignInEnd;

/* ------------------------------ The proposal ------------------------------ */

fn line(id: &str) -> ProposalLine {
    ProposalLine {
        id: id.into(),
        checked: true,
        ..ProposalLine::default()
    }
}

fn night_id(index: usize) -> String {
    format!("s-{index:03}")
}

/// `count` nights (one target each) of one known place, the nights from
/// `created_from` on at a place to create; `groups` join nights (indexes) in
/// a multi-night plan.
fn request(count: usize, groups: &[&[usize]], created_from: Option<usize>) -> BulkCreateRequest {
    let mut proposal = Proposal {
        places: vec![
            PlaceProposal {
                id: "home".into(),
                chosen: Some("place-home".into()),
                ..PlaceProposal::default()
            },
            PlaceProposal {
                id: "field".into(),
                latitude: Some(10.0),
                longitude: Some(20.0),
                ..PlaceProposal::default()
            },
        ],
        ..Proposal::default()
    };
    for index in 0..count {
        let id = night_id(index);
        let day = format!("2026-{:02}-{:02}", 1 + index / 28, 1 + index % 28);
        proposal.sessions.push(DetectedSession {
            id: id.clone(),
            evening_date: day.clone(),
            place: if created_from.is_some_and(|from| index >= from) {
                "field".into()
            } else {
                "home".into()
            },
            start: format!("{day}T20:00:00.000Z"),
            end: format!("{day}T23:00:00.000Z"),
            targets: vec![DetectedTarget {
                key: "m31".into(),
                target_id: Some("m31".into()),
                designation: "M31".into(),
                source: TargetSource::Object,
                start: format!("{day}T20:00:00.000Z"),
                end: format!("{day}T23:00:00.000Z"),
                frames: 10,
                acquisition: vec![AcquisitionRow {
                    exposure_seconds: Some(60.0),
                    frames: 10,
                    total_seconds: 600.0,
                    ..AcquisitionRow::default()
                }],
                ..DetectedTarget::default()
            }],
            ..DetectedSession::default()
        });
        proposal.lines.push(line(&format!("session.create:{id}")));
        proposal
            .lines
            .push(line(&format!("target.detected:{id}/m31")));
    }
    for (rank, nights) in groups.iter().enumerate() {
        let id = format!("mn-{rank}");
        proposal.multi_night.push(MultiNightGroup {
            id: id.clone(),
            target_id: Some("m31".into()),
            designation: "M31".into(),
            nights: nights
                .iter()
                .map(|n| MultiNightNight {
                    session_id: night_id(*n),
                    seconds: 600.0,
                    ..MultiNightNight::default()
                })
                .collect(),
            ..MultiNightGroup::default()
        });
        proposal
            .lines
            .push(line(&format!("multiNight.create:{id}")));
    }
    let ticked = proposal.lines.iter().map(|l| l.id.clone()).collect();
    BulkCreateRequest {
        proposal,
        ticked,
        places: created_from
            .map(|_| {
                vec![PlaceAnswer::Create {
                    place_id: "field".into(),
                    name: "Observing site".into(),
                    latitude: 10.0,
                    longitude: 20.0,
                    time_zone: None,
                }]
            })
            .unwrap_or_default(),
        answers: Vec::new(),
        shifts: Vec::new(),
    }
}

fn sizes(packets: &[Packet]) -> Vec<usize> {
    packets.iter().map(|p| p.nights.len()).collect()
}

/* -------------------------------- The cut --------------------------------- */

#[test]
fn the_nights_are_cut_into_packets_of_ten() {
    for (count, expected) in [
        (1, vec![1]),
        (9, vec![9]),
        (10, vec![10]),
        (11, vec![10, 1]),
        (40, vec![10, 10, 10, 10]),
    ] {
        let body = bulk_body(&request(count, &[], None)).body;
        let packets = packets_of(&body, NIGHTS_PER_PACKET);
        assert_eq!(sizes(&packets), expected, "{count} nights");
        // Every night once, in the order of the body.
        let all: Vec<String> = packets.iter().flat_map(|p| p.nights.clone()).collect();
        assert_eq!(all, (0..count).map(night_id).collect::<Vec<_>>());
    }
}

#[test]
fn ten_nights_or_fewer_are_the_body_of_before_byte_for_byte() {
    for count in [1, 9, 10] {
        let built = bulk_body(&request(count, &[[0usize, 1].as_slice()], Some(0)));
        let packets = packets_of(&built.body, NIGHTS_PER_PACKET);
        assert_eq!(packets.len(), 1);
        let body = packet_body(&packets[0], &BTreeMap::new());
        assert_eq!(body, built.body, "{count} nights");
        assert_eq!(body["importId"], built.body["importId"]);
    }
}

#[test]
fn a_multi_night_plan_astride_two_packets_travels_with_all_its_nights() {
    // Nights 9 and 12 (indexes 8 and 11) in one plan: the plan and both
    // nights go in the first packet, the nights in between after them.
    let body = bulk_body(&request(12, &[[8usize, 11].as_slice()], None)).body;
    let packets = packets_of(&body, NIGHTS_PER_PACKET);
    assert_eq!(sizes(&packets), vec![10, 2]);
    let first: BTreeSet<String> = packets[0].nights.iter().cloned().collect();
    assert!(first.contains(&night_id(8)) && first.contains(&night_id(11)));
    assert_eq!(packets[1].nights, vec![night_id(9), night_id(10)]);
    assert_eq!(packets[0].multi_night.len(), 1);
    assert!(packets[1].multi_night.is_empty());
    // Every plan names only sessions of its own packet.
    for packet in &packets {
        let refs: BTreeSet<&str> = packet
            .sessions
            .iter()
            .filter_map(|s| s["ref"].as_str())
            .collect();
        for group in &packet.multi_night {
            for night in group["sessionRefs"].as_array().unwrap() {
                assert!(refs.contains(night.as_str().unwrap()));
            }
        }
    }
}

#[test]
fn a_plan_larger_than_a_packet_is_never_cut() {
    let nights: Vec<usize> = (2..15).collect();
    let body = bulk_body(&request(20, &[nights.as_slice()], None)).body;
    let packets = packets_of(&body, NIGHTS_PER_PACKET);
    // Nights 1 and 2 alone, the 13 nights of the plan, then the last 5.
    assert_eq!(sizes(&packets), vec![2, 13, 5]);
    assert_eq!(packets[1].multi_night.len(), 1);
    // Two plans sharing a night are one block.
    let body = bulk_body(&request(
        12,
        &[[0usize, 9].as_slice(), [9usize, 11].as_slice()],
        None,
    ))
    .body;
    let packets = packets_of(&body, NIGHTS_PER_PACKET);
    let first: BTreeSet<String> = packets[0].nights.iter().cloned().collect();
    for night in [0, 9, 11] {
        assert!(first.contains(&night_id(night)));
    }
    assert_eq!(packets[0].multi_night.len(), 2);
}

#[test]
fn the_sessions_of_one_night_stay_together() {
    let entry = |reference: &str| json!({ "ref": reference, "place": null });
    let body = json!({
        "places": [],
        "sessions": [entry("a"), entry("b"), entry("b#2"), entry("c")],
        "multiNight": [],
    });
    let packets = packets_of(&body, 2);
    assert_eq!(
        packets.iter().map(|p| p.sessions.len()).collect::<Vec<_>>(),
        vec![3, 1]
    );
    // Two nights per packet: « b » and « b#2 » are one night.
    assert_eq!(packets[0].nights, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(packets[1].nights, vec!["c".to_string()]);
}

#[test]
fn a_place_is_created_with_its_first_packet_and_named_by_its_id_after() {
    // The nights from the fifteenth on are at the place to create.
    let body = bulk_body(&request(25, &[], Some(14))).body;
    let packets = packets_of(&body, NIGHTS_PER_PACKET);
    assert_eq!(sizes(&packets), vec![10, 10, 5]);
    assert!(packets[0].places.is_empty());
    assert_eq!(packets[1].places.len(), 1);
    assert!(packets[2].places.is_empty());

    let known = BTreeMap::from([("field".to_string(), Some("astro-sync-field".to_string()))]);
    let second = packet_body(&packets[1], &known);
    assert_eq!(second["sessions"][9]["place"], json!({ "ref": "field" }));
    let third = packet_body(&packets[2], &known);
    for session in third["sessions"].as_array().unwrap() {
        assert_eq!(session["place"], json!({ "placeId": "astro-sync-field" }));
    }
    // A place the service refused: the later nights go without one.
    let refused = BTreeMap::from([("field".to_string(), None)]);
    let third = packet_body(&packets[2], &refused);
    assert!(third["sessions"][0]["place"].is_null());
    // The known place of the first packets is left as it is.
    assert_eq!(
        packet_body(&packets[0], &known)["sessions"][0]["place"],
        json!({ "placeId": "place-home" })
    );
}

/* ------------------------------ The sending ------------------------------- */

/// A service that derives every id from the packet's `importId` and the
/// element's `ref` (as `session-bulk-service.js` does): a packet sent again
/// replays. `script` says what the next calls answer before they succeed.
#[derive(Default)]
struct Service {
    calls: Cell<usize>,
    bodies: RefCell<Vec<Value>>,
    script: RefCell<VecDeque<PacketFailure>>,
    /// Every call of the packet of this `importId` is cut.
    refuse_import: RefCell<Option<String>>,
    written: RefCell<BTreeSet<String>>,
}

impl Service {
    fn derived(body: &Value, kind: &str, reference: &str) -> String {
        format!(
            "{kind}-{}-{reference}",
            &body["importId"].as_str().unwrap()[..8]
        )
    }
}

impl BulkCreateTransport for Service {
    fn create_bulk(&self, _: &str, _: &str, _: &Value) -> Result<Value, GuiBridgeError> {
        unreachable!("the packets go through send_packet")
    }
    fn send_packet(&self, _: &str, _: &str, body: &Value) -> Result<Value, PacketFailure> {
        self.calls.set(self.calls.get() + 1);
        self.bodies.borrow_mut().push(body.clone());
        if let Some(failure) = self.script.borrow_mut().pop_front() {
            return Err(failure);
        }
        if self.refuse_import.borrow().as_deref() == body["importId"].as_str() {
            return Err(PacketFailure::of(GuiBridgeError::Transport("cut".into())));
        }
        let mut replayed = true;
        let mut write = |id: String| {
            let new = self.written.borrow_mut().insert(id.clone());
            replayed &= !new;
            id
        };
        let places: Vec<Value> = body["places"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                let r = p["ref"].as_str().unwrap();
                json!({ "ref": r, "placeId": write(Self::derived(body, "place", r)), "created": true })
            })
            .collect();
        let sessions: Vec<Value> = body["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                let r = s["ref"].as_str().unwrap();
                json!({
                    "ref": r,
                    "sessionId": write(Self::derived(body, "session", r)),
                    "action": "created",
                    "planAction": "created",
                })
            })
            .collect();
        let multi: Vec<Value> = body["multiNight"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| {
                let r = m["ref"].as_str().unwrap();
                json!({
                    "ref": r,
                    "projectId": write(Self::derived(body, "project", r)),
                    "action": "created",
                    "sessionIds": m["sessionRefs"].as_array().unwrap().iter()
                        .map(|n| Self::derived(body, "session", n.as_str().unwrap()))
                        .collect::<Vec<_>>(),
                })
            })
            .collect();
        Ok(json!({
            "replayed": replayed,
            "places": places,
            "sessions": sessions,
            "multiNight": multi,
        }))
    }
    fn personal_targets(&self, _: &str, _: &str) -> Result<Value, GuiBridgeError> {
        Ok(json!({ "targets": [] }))
    }
    fn create_personal_target(&self, _: &str, _: &str, _: &Value) -> Result<Value, GuiBridgeError> {
        unreachable!()
    }
}

fn session_writes(service: &Service) -> usize {
    service
        .written
        .borrow()
        .iter()
        .filter(|id| id.starts_with("session-"))
        .count()
}

struct Seen {
    events: Vec<PacketEvent>,
    slept: Duration,
}

fn run_with(
    service: &Service,
    request: &BulkCreateRequest,
    memory: Option<&SentPackets>,
) -> (Result<BulkCreateOutcome, GuiBridgeError>, Seen) {
    let events = RefCell::new(Vec::new());
    let slept = Cell::new(Duration::ZERO);
    let result = {
        let mut on_event = |event| events.borrow_mut().push(event);
        let sleep = |d: Duration| slept.set(slept.get() + d);
        let mut run = PacketRun {
            on_event: &mut on_event,
            sleep: &sleep,
            memory,
            size: NIGHTS_PER_PACKET,
        };
        create_sessions_by_packets(service, "https://s", "t", request, &mut run, &mut |_| {
            Ok::<(), GuiBridgeError>(())
        })
    };
    (
        result,
        Seen {
            events: events.into_inner(),
            slept: slept.get(),
        },
    )
}

fn sendings(seen: &Seen) -> Vec<(usize, usize, usize)> {
    seen.events
        .iter()
        .filter_map(|e| match e {
            PacketEvent::Sending {
                from, to, total, ..
            } => Some((*from, *to, *total)),
            PacketEvent::Waiting { .. } => None,
        })
        .collect()
}

#[test]
fn forty_nights_leave_in_four_packets_one_after_the_other() {
    let service = Service::default();
    let request = request(40, &[[8usize, 12].as_slice()], Some(30));
    let (result, seen) = run_with(&service, &request, None);
    let outcome = result.unwrap();
    assert_eq!(service.calls.get(), 4);
    assert_eq!(outcome.sessions.len(), 40);
    assert_eq!(outcome.multi_night.len(), 1);
    assert_eq!(outcome.places_created.len(), 1);
    assert!(!outcome.replayed);
    assert_eq!(
        sendings(&seen),
        vec![(1, 10, 40), (11, 20, 40), (21, 30, 40), (31, 40, 40)]
    );
    // The place to create left once, with the packet of its first night; the
    // last packet names it by the id the service answered.
    let bodies = service.bodies.borrow();
    let created: Vec<usize> = bodies
        .iter()
        .map(|b| b["places"].as_array().unwrap().len())
        .collect();
    assert_eq!(created, vec![0, 0, 0, 1]);
    // Every body is one the service accepts: at most 200 sessions, the plan's
    // nights in its own call, a fresh `importId` each.
    let ids: BTreeSet<&str> = bodies
        .iter()
        .filter_map(|b| b["importId"].as_str())
        .collect();
    assert_eq!(ids.len(), 4);
    // The wrapper sends the same packets.
    let again = Service::default();
    let outcome = create_sessions(&again, "https://s", "t", &request).unwrap();
    assert_eq!(again.calls.get(), 4);
    assert_eq!(outcome.sessions.len(), 40);
    assert_eq!(*again.bodies.borrow(), *bodies);
}

#[test]
fn a_busy_service_is_asked_again_after_its_retry_after_and_a_packet_through_never_leaves_again() {
    let service = Service::default();
    // The second packet meets a 503 `service_starting`, `Retry-After: 12`, then
    // one without `Retry-After`.
    let busy = |retry_after| {
        PacketFailure::answered(
            GuiBridgeError::Refused {
                code: "service_starting".into(),
                message: "The service is starting. Try again in a moment.".into(),
            },
            Some(503),
            retry_after,
        )
    };
    let request = request(15, &[], None);
    // The first packet goes through in a first run (kept), so the failures
    // scripted after it meet the second packet.
    let memory = SentPackets::new();
    let one = BulkCreateRequest {
        proposal: Proposal {
            sessions: request.proposal.sessions[..10].to_vec(),
            ..request.proposal.clone()
        },
        ..request.clone()
    };
    let (first, _) = run_with(&service, &one, Some(&memory));
    first.unwrap();
    assert_eq!(service.calls.get(), 1);
    assert_eq!(memory.len(), 1);

    service
        .script
        .borrow_mut()
        .extend([busy(Some(12)), busy(None)]);
    let (result, seen) = run_with(&service, &request, Some(&memory));
    let outcome = result.unwrap();
    // The first packet was not sent again: three calls for the second.
    assert_eq!(service.calls.get(), 4);
    assert_eq!(outcome.sessions.len(), 15);
    // 12 s asked (more than the first step, 5 s), then the second step, 10 s.
    assert_eq!(seen.slept, Duration::from_secs(22));
    let waits: Vec<u64> = seen
        .events
        .iter()
        .filter_map(|e| match e {
            PacketEvent::Waiting { seconds } => Some(*seconds),
            PacketEvent::Sending { .. } => None,
        })
        .collect();
    assert_eq!(waits.len(), 22);
    assert_eq!(&waits[..3], &[12, 11, 10]);
    assert_eq!(waits[11], 1);
    assert_eq!(waits[12], 10);
    assert_eq!(*waits.last().unwrap(), 1);
    // The sending is said again after each wait.
    assert_eq!(sendings(&seen), vec![(11, 15, 15); 3]);
    assert_eq!(session_writes(&service), 15);
}

#[test]
fn a_service_that_stays_busy_is_asked_six_times_then_the_failure_is_said() {
    let service = Service::default();
    service.script.borrow_mut().extend((0..10).map(|_| {
        PacketFailure::answered(
            GuiBridgeError::Refused {
                code: "service_starting".into(),
                message: String::new(),
            },
            Some(503),
            Some(10),
        )
    }));
    let (result, seen) = run_with(&service, &request(3, &[], None), None);
    assert!(matches!(
        result,
        Err(GuiBridgeError::Refused { ref code, .. }) if code == "service_starting"
    ));
    assert_eq!(service.calls.get(), 6);
    // 10, 10, 20, 40, 60 s.
    assert_eq!(seen.slept, Duration::from_secs(140));
}

#[test]
fn a_401_stops_at_once_and_stays_the_end_of_the_sign_in() {
    for failure in [
        PacketFailure::answered(
            GuiBridgeError::SignInEnded(SignInEnd::DeviceDetached),
            Some(401),
            None,
        ),
        PacketFailure::answered(
            GuiBridgeError::Refused {
                code: "authentication_required".into(),
                message: String::new(),
            },
            Some(401),
            Some(10),
        ),
    ] {
        let service = Service::default();
        service.script.borrow_mut().push_back(failure);
        let (result, seen) = run_with(&service, &request(25, &[], None), None);
        assert!(result.is_err());
        assert_eq!(service.calls.get(), 1);
        assert_eq!(seen.slept, Duration::ZERO);
        assert_eq!(session_writes(&service), 0);
    }
}

#[test]
fn what_is_asked_again_and_what_is_not() {
    let refused = |code: &str| GuiBridgeError::Refused {
        code: code.into(),
        message: String::new(),
    };
    let retryable = |error, status| PacketFailure::answered(error, status, None).retryable;
    assert!(retryable(refused("service_starting"), Some(503)));
    assert!(retryable(refused("rate_limited"), Some(429)));
    assert!(retryable(refused("invalid_response"), Some(502)));
    assert!(retryable(
        GuiBridgeError::Transport("no network".into()),
        None
    ));
    assert!(retryable(
        GuiBridgeError::Wait(ServiceWaitError::StartTimeout { waited_secs: 100 }),
        None
    ));
    assert!(!retryable(refused("authentication_required"), Some(401)));
    assert!(!retryable(refused("device_detached"), Some(401)));
    assert!(!retryable(refused("internal_error"), Some(500)));
    assert!(!retryable(refused("invalid_request"), Some(400)));
    assert!(!retryable(
        GuiBridgeError::Wait(ServiceWaitError::Cancelled),
        None
    ));
    // Without a status (a fake transport): the code says it.
    assert!(PacketFailure::of(refused("service_starting")).retryable);
    assert!(!PacketFailure::of(refused("plan_limit")).retryable);
}

#[test]
fn an_import_cut_halfway_resumes_without_a_double() {
    let request = request(40, &[[18usize, 23].as_slice()], Some(5));
    let service = Service::default();
    let memory = SentPackets::new();
    // The network is lost at the third packet, past every new try.
    let third = {
        let built = bulk_body(&request);
        let packets = packets_of(&built.body, NIGHTS_PER_PACKET);
        // The ids of the earlier packets are those the service answers.
        let mut known = BTreeMap::new();
        known.insert(
            "field".to_string(),
            Some({
                let first = packet_body(&packets[0], &BTreeMap::new());
                Service::derived(&first, "place", "field")
            }),
        );
        packet_body(&packets[2], &known)["importId"]
            .as_str()
            .unwrap()
            .to_string()
    };
    *service.refuse_import.borrow_mut() = Some(third);
    let (result, _) = run_with(&service, &request, Some(&memory));
    assert!(matches!(result, Err(GuiBridgeError::Transport(_))));
    assert_eq!(service.calls.get(), 2 + 6);
    assert_eq!(memory.len(), 2);
    assert_eq!(session_writes(&service), 20);

    // « Créer » again, the network back: the two packets through are not
    // sent again.
    *service.refuse_import.borrow_mut() = None;
    let (result, _) = run_with(&service, &request, Some(&memory));
    let outcome = result.unwrap();
    assert_eq!(service.calls.get(), 8 + 2);
    assert_eq!(outcome.sessions.len(), 40);
    assert_eq!(session_writes(&service), 40);

    // The application closed and opened again: nothing kept, every packet
    // sent again, each one a replay -- not one session more.
    let (result, _) = run_with(&service, &request, None);
    let outcome = result.unwrap();
    assert_eq!(service.calls.get(), 10 + 4);
    assert!(outcome.replayed);
    assert_eq!(outcome.sessions.len(), 40);
    assert_eq!(session_writes(&service), 40);
    assert_eq!(service.written.borrow().len(), 40 + 1 + 1);
}

#[test]
fn the_memory_is_per_space() {
    let service = Service::default();
    let memory = SentPackets::new();
    let request = request(3, &[], None);
    let (first, _) = run_with(&service, &request, Some(&memory));
    first.unwrap();
    // The same choices for another account are sent.
    let events = RefCell::new(Vec::new());
    let mut on_event = |e| events.borrow_mut().push(e);
    let mut run = PacketRun {
        on_event: &mut on_event,
        sleep: &|_| {},
        memory: Some(&memory),
        size: NIGHTS_PER_PACKET,
    };
    create_sessions_by_packets(
        &service,
        "https://s",
        "other",
        &request,
        &mut run,
        &mut |_| Ok::<(), GuiBridgeError>(()),
    )
    .unwrap();
    assert_eq!(service.calls.get(), 2);
    assert_eq!(memory.len(), 2);
}

#[test]
fn each_packet_is_handed_over_before_the_next_leaves() {
    let service = Service::default();
    let request = request(25, &[], None);
    let seen = RefCell::new(Vec::new());
    let mut run = PacketRun {
        on_event: &mut |_| {},
        sleep: &|_| {},
        memory: None,
        size: NIGHTS_PER_PACKET,
    };
    let result = create_sessions_by_packets(
        &service,
        "https://s",
        "t",
        &request,
        &mut run,
        &mut |part| {
            seen.borrow_mut()
                .push((part.sessions.len(), service.calls.get()));
            if seen.borrow().len() == 2 {
                return Err(GuiBridgeError::InvalidResponse("disk full".into()));
            }
            Ok(())
        },
    );
    assert!(result.is_err());
    assert_eq!(*seen.borrow(), vec![(10, 1), (10, 2)]);
    assert_eq!(service.calls.get(), 2);
}

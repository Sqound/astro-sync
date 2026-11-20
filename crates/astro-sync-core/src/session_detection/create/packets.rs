//! « Création des sessions… » by packets: the body [`super::bulk_body`] builds
//! for the whole import is sent as several `POST /v1/sessions/bulk` of
//! [`NIGHTS_PER_PACKET`] nights each, one after the other, so that no call
//! lasts long enough to be abandoned and sent again while the service is
//! still writing the first copy (the service writes about 117 rows per night,
//! one after the other: 40 nights in one call could pass the minute).
//!
//! - **What travels together.** A night travels whole (a night with several
//!   sessions has one entry per session, `s-07`, `s-07#2`: they stay in one
//!   packet), and a multi-night plan travels with all its nights (the service
//!   only knows the `sessionRefs` of its own call): nights joined by a plan
//!   form one block, placed in the packet of its first night; a block larger
//!   than a packet is a packet of its own. A place to create goes with the
//!   first packet that names it; the packets after it name it by the
//!   `placeId` the service answered (`null` when it refused it, as the
//!   service itself does for a session whose place it refused).
//! - **Never twice.** Each packet carries its own `importId`, derived from its
//!   bytes ([`super::import_id_of`]), and the service derives every
//!   identifier from it: sending a packet again replays it, never doubles it.
//!   An import of [`NIGHTS_PER_PACKET`] nights or fewer is one packet whose
//!   body is the body of before, byte for byte, with the same `importId`.
//!   Within one run of the application, a packet that went through is kept
//!   ([`SentPackets`]) and never sent again, even by a new « Créer »; after
//!   the application was closed, the packets that went through are sent
//!   again and replayed by the service.
//! - **When the service is busy.** A packet whose call failed because the
//!   service could not answer (`503` `service_starting`, `429`, a gateway
//!   error, no network, no answer in time) is sent again after a growing
//!   wait ([`PACKET_RETRY_WAITS_SECONDS`]), never shorter than the
//!   `Retry-After` the service asked for; the wait is counted down second by
//!   second ([`PacketEvent::Waiting`]). A refusal (`401`, a `4xx`, a `500`)
//!   stops at once: a `401` stays the end of the sign-in it always was.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::gui_bridge::GuiBridgeError;
use crate::service_wait::ServiceWaitError;

use super::{
    BulkBody, BulkCreateOutcome, BulkCreateRequest, BulkCreateTransport, bulk_body, import_id_of,
    outcome_of, with_personal_targets,
};

/// How many nights one `POST /v1/sessions/bulk` carries at most (a block of
/// nights joined by a multi-night plan may carry more: it is never cut).
pub const NIGHTS_PER_PACKET: usize = 10;

/// The waits before each new try of a packet the service could not take
/// (then the packet's failure is said): about two minutes and a quarter in
/// all, each one at least the `Retry-After` asked.
pub const PACKET_RETRY_WAITS_SECONDS: [u64; 5] = [5, 10, 20, 40, 60];

/// The longest `Retry-After` honoured for one wait.
pub const LONGEST_PACKET_WAIT_SECONDS: u64 = 300;

/// One call of the import: its part of the body, and the nights it carries
/// (the proposal's session ids, in order).
#[derive(Debug, Clone, PartialEq)]
pub struct Packet {
    pub places: Vec<Value>,
    pub sessions: Vec<Value>,
    pub multi_night: Vec<Value>,
    pub nights: Vec<String>,
}

/// The night an entry of `sessions` belongs to: its `ref` without the
/// `#2`, `#3` that [`super::bulk_body`] gives the second session of a night.
fn night_of(entry: &Value) -> String {
    let reference = entry["ref"].as_str().unwrap_or_default();
    match reference.rsplit_once('#') {
        Some((night, rank)) if !rank.is_empty() && rank.chars().all(|c| c.is_ascii_digit()) => {
            night.to_string()
        }
        _ => reference.to_string(),
    }
}

fn root(parent: &mut [usize], index: usize) -> usize {
    let mut top = index;
    while parent[top] != top {
        top = parent[top];
    }
    let mut at = index;
    while parent[at] != top {
        let next = parent[at];
        parent[at] = top;
        at = next;
    }
    top
}

/// The body of the whole import cut into packets of `size` nights at most
/// (see the module documentation for what travels together), in the order
/// of the body. Always one packet at least.
#[must_use]
pub fn packets_of(body: &Value, size: usize) -> Vec<Packet> {
    let size = size.max(1);
    let list = |key: &str| body[key].as_array().cloned().unwrap_or_default();
    let (places, sessions, multi) = (list("places"), list("sessions"), list("multiNight"));

    let mut nights: Vec<String> = Vec::new();
    let mut night_of_entry = Vec::with_capacity(sessions.len());
    for entry in &sessions {
        let night = night_of(entry);
        let index = nights.iter().position(|n| *n == night).unwrap_or_else(|| {
            nights.push(night);
            nights.len() - 1
        });
        night_of_entry.push(index);
    }

    // The nights joined by a plan: one block, named by its first night.
    let mut parent: Vec<usize> = (0..nights.len()).collect();
    let nights_of_group = |group: &Value| -> Vec<usize> {
        group["sessionRefs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(|r| nights.iter().position(|n| n == r))
            .collect()
    };
    for group in &multi {
        let joined = nights_of_group(group);
        for pair in joined.windows(2) {
            let (a, b) = (root(&mut parent, pair[0]), root(&mut parent, pair[1]));
            if a != b {
                parent[a.max(b)] = a.min(b);
            }
        }
    }
    let mut blocks: Vec<Vec<usize>> = Vec::new();
    let mut block_of_root: BTreeMap<usize, usize> = BTreeMap::new();
    for night in 0..nights.len() {
        let top = root(&mut parent, night);
        let block = *block_of_root.entry(top).or_insert_with(|| {
            blocks.push(Vec::new());
            blocks.len() - 1
        });
        blocks[block].push(night);
    }

    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    for block in blocks {
        if !current.is_empty() && current.len() + block.len() > size {
            groups.push(std::mem::take(&mut current));
        }
        current.extend(block);
    }
    if !current.is_empty() || groups.is_empty() {
        groups.push(current);
    }
    for group in &mut groups {
        group.sort_unstable();
    }
    let packet_of_night: BTreeMap<usize, usize> = groups
        .iter()
        .enumerate()
        .flat_map(|(packet, group)| group.iter().map(move |night| (*night, packet)))
        .collect();

    let mut packets: Vec<Packet> = groups
        .iter()
        .map(|group| Packet {
            places: Vec::new(),
            sessions: Vec::new(),
            multi_night: Vec::new(),
            nights: group.iter().map(|n| nights[*n].clone()).collect(),
        })
        .collect();
    for (entry, night) in sessions.iter().zip(&night_of_entry) {
        packets[packet_of_night[night]].sessions.push(entry.clone());
    }
    for group in &multi {
        let packet = nights_of_group(group)
            .first()
            .map_or(0, |night| packet_of_night[night]);
        packets[packet].multi_night.push(group.clone());
    }
    // A place goes with the first packet whose sessions name it (in the
    // order of the body's places, as before).
    for place in &places {
        let reference = place["ref"].as_str().unwrap_or_default();
        let packet = packets
            .iter()
            .position(|p| {
                p.sessions
                    .iter()
                    .any(|s| s["place"]["ref"].as_str() == Some(reference))
            })
            .unwrap_or(0);
        packets[packet].places.push(place.clone());
    }
    packets
}

/// The body of `packet`, its references to the places of an earlier packet
/// replaced by the `placeId` the service answered for them (`null`: refused),
/// and its own `importId`.
#[must_use]
pub fn packet_body(packet: &Packet, known_places: &BTreeMap<String, Option<String>>) -> Value {
    let own: Vec<&str> = packet
        .places
        .iter()
        .filter_map(|p| p["ref"].as_str())
        .collect();
    let sessions: Vec<Value> = packet
        .sessions
        .iter()
        .map(|entry| {
            let mut entry = entry.clone();
            if let Some(reference) = entry["place"]["ref"].as_str().map(str::to_string)
                && !own.contains(&reference.as_str())
            {
                entry["place"] = match known_places.get(&reference) {
                    Some(Some(place_id)) => json!({ "placeId": place_id }),
                    _ => Value::Null,
                };
            }
            entry
        })
        .collect();
    let mut body = json!({
        "places": packet.places,
        "sessions": sessions,
        "multiNight": packet.multi_night,
    });
    body["importId"] = json!(import_id_of(&body));
    body
}

/// Why a packet did not go through, and whether to ask again.
#[derive(Debug)]
pub struct PacketFailure {
    pub error: GuiBridgeError,
    /// The service could not take it now (busy, starting, out of reach):
    /// the same packet may be sent again.
    pub retryable: bool,
    /// The `Retry-After` the service sent, in seconds.
    pub retry_after_seconds: Option<u64>,
}

/// The codes of an answer that says « not now, nothing was done ».
const BUSY_CODES: [&str; 3] = ["service_starting", "service_unavailable", "rate_limited"];

impl PacketFailure {
    /// A failure known only by its error (no status, no `Retry-After`).
    #[must_use]
    pub fn of(error: GuiBridgeError) -> Self {
        let retryable = match &error {
            GuiBridgeError::Transport(_) => true,
            GuiBridgeError::Wait(ServiceWaitError::StartTimeout { .. }) => true,
            GuiBridgeError::Refused { code, .. } => BUSY_CODES.contains(&code.as_str()),
            _ => false,
        };
        Self {
            error,
            retryable,
            retry_after_seconds: None,
        }
    }

    /// A failure with the status and the `Retry-After` of the answer, when
    /// one came: a `429`, `502`, `503`, `504` is the service busy; a `401` and
    /// every other refusal are not asked again.
    #[must_use]
    pub fn answered(error: GuiBridgeError, status: Option<u16>, retry_after: Option<u64>) -> Self {
        let mut failure = Self::of(error);
        if let Some(status) = status {
            failure.retryable = matches!(status, 429 | 502..=504)
                && !matches!(failure.error, GuiBridgeError::SignInEnded(_));
        }
        failure.retry_after_seconds = retry_after;
        failure
    }
}

/// What the packets are doing, for the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketEvent {
    /// The packet `index` (from 1) of `packets` leaves, with the nights
    /// `from` to `to` (from 1) of `total`.
    Sending {
        index: usize,
        packets: usize,
        from: usize,
        to: usize,
        total: usize,
    },
    /// The service is busy: a new try in `seconds` (said each second).
    Waiting { seconds: u64 },
}

/// The packets that went through in this run of the application, by space and
/// `importId`: a new « Créer » after a failure halfway never sends them again.
pub struct SentPackets {
    answers: Mutex<BTreeMap<String, Value>>,
}

impl Default for SentPackets {
    fn default() -> Self {
        Self::new()
    }
}

impl SentPackets {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            answers: Mutex::new(BTreeMap::new()),
        }
    }

    fn key(base_url: &str, token: &str, import_id: &str) -> String {
        let digest = Sha256::digest(token.as_bytes());
        let account: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
        format!("{}|{account}|{import_id}", base_url.trim_end_matches('/'))
    }

    fn get(&self, key: &str) -> Option<Value> {
        self.answers.lock().ok()?.get(key).cloned()
    }

    fn keep(&self, key: String, answer: &Value) {
        if let Ok(mut answers) = self.answers.lock() {
            answers.insert(key, answer.clone());
        }
    }

    /// How many packets are kept (tests).
    #[must_use]
    pub fn len(&self) -> usize {
        self.answers.lock().map_or(0, |a| a.len())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// How a run of packets waits and what it says.
pub struct PacketRun<'a> {
    pub on_event: &'a mut dyn FnMut(PacketEvent),
    /// Waits one step of a countdown (`std::thread::sleep`; tests: nothing).
    pub sleep: &'a dyn Fn(Duration),
    pub memory: Option<&'a SentPackets>,
    pub size: usize,
}

fn real_sleep(duration: Duration) {
    std::thread::sleep(duration);
}

impl PacketRun<'_> {
    /// Packets of [`NIGHTS_PER_PACKET`], real waits, nothing said, nothing
    /// kept: the command line.
    pub fn quiet(on_event: &mut dyn FnMut(PacketEvent)) -> PacketRun<'_> {
        PacketRun {
            on_event,
            sleep: &real_sleep,
            memory: None,
            size: NIGHTS_PER_PACKET,
        }
    }
}

/// One packet: kept from this run, else sent, again while the service is
/// busy.
fn send_packet<T: BulkCreateTransport>(
    transport: &T,
    base_url: &str,
    token: &str,
    body: &Value,
    run: &mut PacketRun<'_>,
    sending: PacketEvent,
) -> Result<Value, GuiBridgeError> {
    let import_id = body["importId"].as_str().unwrap_or_default();
    let key = SentPackets::key(base_url, token, import_id);
    if let Some(answer) = run.memory.and_then(|m| m.get(&key)) {
        return Ok(answer);
    }
    let mut tried = 0;
    loop {
        (run.on_event)(sending);
        match transport.send_packet(base_url, token, body) {
            Ok(answer) => {
                if let Some(memory) = run.memory {
                    memory.keep(key, &answer);
                }
                return Ok(answer);
            }
            Err(failure) if failure.retryable && tried < PACKET_RETRY_WAITS_SECONDS.len() => {
                let step = PACKET_RETRY_WAITS_SECONDS[tried];
                tried += 1;
                let wait = failure.retry_after_seconds.map_or(step, |asked| {
                    asked.min(LONGEST_PACKET_WAIT_SECONDS).max(step)
                });
                for seconds in (1..=wait).rev() {
                    (run.on_event)(PacketEvent::Waiting { seconds });
                    (run.sleep)(Duration::from_secs(1));
                }
            }
            Err(failure) => return Err(failure.error),
        }
    }
}

/// « Créer … » by packets: the personal targets, the body cut into packets,
/// each packet sent in its turn ([`send_packet`]) and its answer handed to
/// `after` before the next leaves (step 4 records the nights created there,
/// so a failure further on keeps them). The outcome is the packets' outcomes
/// put together, with what Astro Sync left out before sending; `replayed`
/// when every packet was a replay.
pub fn create_sessions_by_packets<T, E>(
    transport: &T,
    base_url: &str,
    token: &str,
    request: &BulkCreateRequest,
    run: &mut PacketRun<'_>,
    after: &mut dyn FnMut(&BulkCreateOutcome) -> Result<(), E>,
) -> Result<BulkCreateOutcome, E>
where
    T: BulkCreateTransport,
    E: From<GuiBridgeError>,
{
    let request = &with_personal_targets(transport, base_url, token, request);
    let built = bulk_body(request);
    let nothing = built.body["sessions"].as_array().is_none_or(Vec::is_empty)
        && built.body["places"].as_array().is_none_or(Vec::is_empty);
    let mut outcome = BulkCreateOutcome {
        skipped: built.skipped.clone(),
        ..BulkCreateOutcome::default()
    };
    if nothing {
        return Ok(outcome);
    }
    let packets = packets_of(&built.body, run.size);
    let total: usize = packets.iter().map(|p| p.nights.len()).sum();
    let mut known_places: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut done = 0;
    let mut replayed = true;
    for (index, packet) in packets.iter().enumerate() {
        let body = packet_body(packet, &known_places);
        let sending = PacketEvent::Sending {
            index: index + 1,
            packets: packets.len(),
            from: done + 1,
            to: done + packet.nights.len(),
            total,
        };
        let answer = send_packet(transport, base_url, token, &body, run, sending)?;
        for place in answer["places"].as_array().into_iter().flatten() {
            if let Some(reference) = place["ref"].as_str() {
                known_places.insert(
                    reference.to_string(),
                    place["placeId"].as_str().map(str::to_string),
                );
            }
        }
        let part = outcome_of(
            request,
            &BulkBody {
                body,
                skipped: Vec::new(),
            },
            &answer,
        );
        after(&part)?;
        replayed &= part.replayed;
        outcome.sessions.extend(part.sessions);
        outcome.multi_night.extend(part.multi_night);
        outcome.places_created.extend(part.places_created);
        outcome.skipped.extend(part.skipped);
        done += packet.nights.len();
    }
    outcome.replayed = replayed;
    Ok(outcome)
}

#[cfg(test)]
mod tests;

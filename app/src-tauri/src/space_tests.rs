//! The desktop app's half of the hosted sign-in, driven through Tauri's own
//! IPC on its mock runtime -- the same command wrappers the window calls, the
//! real client, and the fake service that replays the exchanges the contract
//! records (`crates/astro-sync-core/tests/support/contract_replay.rs`):
//!
//! - what the service announces decides the screen (a private server keeps
//!   its pairing);
//! - the request and the verification of a code reach the page in the shape it
//!   reads -- a wrong code with its tries left, a wait with its `Retry-After`
//!   -- and **the token never reaches the page**;
//! - a sign-in the service ends (`device_detached`, `space_deleted`) sends the
//!   page back to the sign-in screen with the reason, forgets the token, and
//!   **loses nothing else**: the folders associated with sessions and the
//!   files waiting in the upload queue stay.
//!
//! The app's own files live under the platform's config directory and the
//! device token normally goes through the OS keyring: neither is ever touched
//! (see `wake_tests`, whose harness this one shares: `HOME` is redirected to a
//! throwaway directory, the keyring is `keyring`'s mock, and the token is read
//! from the legacy file it falls back to).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use astro_sync_core::association::{self, Association};
use astro_sync_core::dry_run::build_dry_run_report;
use astro_sync_core::scanner::scan_folder;
use astro_sync_core::upload_queue::{QueueState, UploadQueue};

use crate::wake_tests::fake_service;
use crate::wake_tests::{fast, harness, serialized, unique_dir, Harness};
use contract_replay::{exchange, reply_of, Replay};
use fake_service::{FakeService, Wake};

#[path = "../../../crates/astro-sync-core/tests/support/contract_replay.rs"]
mod contract_replay;

const TOKEN_OF_ASTRO_SYNC: &str = "QJR3D-0HcwGBdSXJ8IpOIuINTTIPAl2OHFyn1hIeyUQ";
const CODE_OF_ASTRO_SYNC: &str = "730418";
/// The example bearer of the recorded `space-current` / `space-devices`
/// exchanges.
const RECORDED_BEARER: &str = "wl2zxC3Dt4HJgMfhEiU4pUwq0oeW9ODmY8T3moyhKm4";
const ANSWER: Duration = Duration::from_secs(5);
/// The space `signed_in_to` links: the queue is seen from it.
const SPACE_OF_ASTRO_SYNC: &str = "4ec83832-fbb7-4f8d-91ca-8c7eefd8ab71";

fn json_of(value: &serde_json::Value) -> String {
    value.to_string()
}

/// A computer signed in to a space by e-mail code, the way the files hold it
/// after a sign-in: the address of the service, the space record and the
/// token. With the mock keyring the token is read (and consumed) from the
/// legacy file: call this before each command that needs it.
fn signed_in_to(harness: &Harness, portal_url: &str) {
    harness.pair_with(portal_url);
    std::fs::write(
        harness.config_dir.join("space.json"),
        serde_json::json!({
            "link": {
                "portalUrl": portal_url,
                "space": {
                    "id": SPACE_OF_ASTRO_SYNC,
                    "email": "vous@exemple.fr",
                    "webUrl": "https://portal.example.test"
                },
                "device": {
                    "id": "98dc0200-d02c-44a3-899b-129fa99f3e0c",
                    "name": "Astro Sync \u{2014} MacBook Pro"
                }
            }
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(harness.config_dir.join("device-token"), TOKEN_OF_ASTRO_SYNC).unwrap();
}

fn forget_files(harness: &Harness) {
    std::fs::create_dir_all(&harness.config_dir).unwrap();
    for name in [
        "settings.json",
        "space.json",
        "device-token",
        "installation-id",
        "capabilities-cache.json",
        "associations.json",
        "upload-queue.sqlite",
    ] {
        let _ = std::fs::remove_file(harness.config_dir.join(name));
    }
}

/// The queue of the test, seen from the space `signed_in_to` links.
fn space_queue(harness: &Harness) -> UploadQueue {
    UploadQueue::open(&harness.config_dir.join("upload-queue.sqlite"))
        .unwrap()
        .for_space(Some(SPACE_OF_ASTRO_SYNC.to_string()))
}

/// A folder associated with a session, and a file waiting in the queue of the
/// space.
fn a_night_waiting_to_be_sent(harness: &Harness) -> std::path::PathBuf {
    let folder = unique_dir("night");
    std::fs::write(folder.join("frame.tif"), vec![9_u8; 250]).unwrap();
    let association = Association::existing("session-1", &folder).unwrap();
    association::save_association(&harness.config_dir.join("associations.json"), &association)
        .unwrap();
    let queue = space_queue(harness);
    queue
        .enqueue_from_dry_run(&build_dry_run_report(&scan_folder(&folder).unwrap()))
        .unwrap();
    folder
}

fn queue_states(harness: &Harness) -> Vec<QueueState> {
    space_queue(harness)
        .list()
        .unwrap()
        .into_iter()
        .map(|entry| entry.state)
        .collect()
}

#[test]
fn the_page_signs_in_with_an_address_and_a_code_and_never_receives_the_token() {
    let _serial = serialized();
    let harness = harness(fast());
    forget_files(&harness);
    let replay = Replay::start(&[
        "hosted-service-announces-email-code",
        "request-code-address-with-space",
        "verify-code-astro-sync",
    ]);
    let portal = replay.base_url();

    // What the service announces.
    let announced = harness
        .invoke(
            "service_announcement",
            serde_json::json!({"portalUrl": portal}),
        )
        .answer(ANSWER)
        .unwrap();
    assert_eq!(announced["method"], "email-code");
    assert_eq!(announced["termsUrl"], "https://www.example.test/terms");

    // Nothing is signed in yet.
    let status = harness
        .invoke("space_status", serde_json::json!({}))
        .answer(ANSWER)
        .unwrap();
    assert_eq!(status["paired"], false);
    assert_eq!(status["space"], serde_json::Value::Null);

    // The code.
    let requested = harness
        .invoke(
            "space_request_code",
            serde_json::json!({"portalUrl": portal, "email": "vous@exemple.fr", "language": "fr"}),
        )
        .answer(ANSWER)
        .unwrap();
    assert_eq!(requested["email"], "vous@exemple.fr");
    assert_eq!(requested["resendAfterSeconds"], 60);
    assert_eq!(requested["expiresInSeconds"], 600);

    // The verification.
    let outcome = harness
        .invoke(
            "space_verify_code",
            serde_json::json!({"portalUrl": portal, "email": "vous@exemple.fr", "code": CODE_OF_ASTRO_SYNC}),
        )
        .answer(ANSWER)
        .unwrap();
    assert_eq!(outcome["created"], false);
    assert_eq!(outcome["status"]["paired"], true);
    assert_eq!(outcome["status"]["space"]["email"], "vous@exemple.fr");
    assert_eq!(
        outcome["status"]["space"]["webUrl"],
        "https://portal.example.test"
    );
    assert_eq!(
        outcome["status"]["device"]["name"],
        "Astro Sync \u{2014} MacBook Pro"
    );
    replay.assert_played_exactly();

    // The service and the name are kept exactly as a pairing keeps them.
    let settings: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(harness.config_dir.join("settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(settings["portalUrl"], portal.as_str());
    assert_eq!(settings["deviceLabel"], "Astro Sync \u{2014} MacBook Pro");
    // The identifier of the installation was made once and kept.
    let installation = std::fs::read_to_string(harness.config_dir.join("installation-id")).unwrap();
    assert!(installation.starts_with("astrosync-"));

    // The token reached no place the page reads: not the answers, not the
    // events, not a file of the sign-in.
    for shown in [
        json_of(&requested),
        json_of(&outcome),
        json_of(&serde_json::Value::Array(
            harness.service_events.lock().unwrap().clone(),
        )),
        std::fs::read_to_string(harness.config_dir.join("settings.json")).unwrap(),
        std::fs::read_to_string(harness.config_dir.join("space.json")).unwrap(),
    ] {
        assert!(
            !shown.contains(TOKEN_OF_ASTRO_SYNC),
            "the token leaked: {shown}"
        );
        assert!(
            !shown.contains(CODE_OF_ASTRO_SYNC),
            "the code leaked: {shown}"
        );
    }
    drop(harness.app);
}

#[test]
fn a_private_server_announces_its_pairing_and_keeps_it() {
    let _serial = serialized();
    let harness = harness(fast());
    forget_files(&harness);
    for announcement in [
        "private-server-announces-pairing",
        "older-private-server-announces-nothing",
    ] {
        let replay = Replay::start(&[announcement]);
        let announced = harness
            .invoke(
                "service_announcement",
                serde_json::json!({"portalUrl": replay.base_url()}),
            )
            .answer(ANSWER)
            .unwrap();
        assert_eq!(announced["method"], "pairing", "{announcement}");
        replay.assert_played_exactly();
    }
    // The pairing itself is `device_pair`, untouched (`wake_tests`).
    drop(harness.app);
}

#[test]
fn a_refused_code_reaches_the_page_with_what_the_screen_counts_down() {
    let _serial = serialized();
    let harness = harness(fast());
    forget_files(&harness);

    for (exchange_id, code, details) in [
        (
            "verify-code-incorrect",
            "sign_in_code_incorrect",
            serde_json::json!({"attemptsRemaining": 4}),
        ),
        (
            "verify-code-fifth-failure-locks",
            "sign_in_locked",
            serde_json::json!({"retryAfterSeconds": 900, "retryAt": "2026-09-29T21:35:00.000Z"}),
        ),
        (
            "verify-code-expired",
            "sign_in_code_expired",
            serde_json::Value::Null,
        ),
        (
            "verify-too-many-failures-from-origin",
            "sign_in_rate_limited",
            serde_json::json!({"retryAfterSeconds": 1500, "retryAt": "2026-09-29T22:05:00.000Z"}),
        ),
    ] {
        let replay = Replay::start(&[exchange_id]);
        let error = harness
            .invoke(
                "space_verify_code",
                serde_json::json!({"portalUrl": replay.base_url(), "email": "vous@exemple.fr", "code": "481207"}),
            )
            .answer(ANSWER)
            .expect_err("a refused code is an error the page can translate");
        assert_eq!(error["code"], code, "{exchange_id}");
        if details.is_null() {
            assert!(error.get("details").is_none(), "{exchange_id}: {error}");
        } else {
            assert_eq!(error["details"], details, "{exchange_id}");
        }
        assert!(
            !json_of(&error).contains("481207"),
            "the code is not echoed back"
        );
        replay.assert_played_exactly();
        assert!(
            !harness.config_dir.join("device-token").exists(),
            "{exchange_id}: no token without a verified code"
        );
    }
    drop(harness.app);
}

#[test]
fn a_service_that_ends_the_sign_in_brings_the_page_back_and_loses_nothing() {
    let _serial = serialized();
    for (exchange_id, reason) in [
        ("call-from-detached-device", "device_detached"),
        ("call-after-deletion", "space_deleted"),
    ] {
        let harness = harness(fast());
        forget_files(&harness);
        let night = a_night_waiting_to_be_sent(&harness);
        let ends = exchange(exchange_id);
        let service = FakeService::start(Wake::Instant, move |request| {
            if request.path == "/api/v1/service" {
                fake_service::identity_reply()
            } else {
                reply_of(&ends)
            }
        });
        signed_in_to(&harness, &service.base_url());

        // A call the person makes: the answer says why, and the page is told.
        let error = harness
            .invoke(
                "list_sessions",
                serde_json::json!({"since": null, "limit": null}),
            )
            .answer(ANSWER)
            .expect_err("the sign-in is over");
        assert_eq!(error["code"], reason);
        let events = harness.space_events.lock().unwrap().clone();
        assert_eq!(
            events,
            [serde_json::json!({"kind": "signed_out", "reason": reason})],
            "the page is sent back to the sign-in screen"
        );

        // What is on disk: no token, the reason, the address to offer again.
        assert!(
            !harness.config_dir.join("device-token").exists(),
            "the token is forgotten"
        );
        let status = harness
            .invoke("space_status", serde_json::json!({}))
            .answer(ANSWER)
            .unwrap();
        assert_eq!(status["paired"], false);
        assert_eq!(status["ended"]["reason"], reason);
        // What this computer knew of the space is forgotten, its address too.
        assert!(status["ended"].get("email").is_none(), "{status}");
        let record = std::fs::read_to_string(harness.config_dir.join("space.json")).unwrap();
        assert!(
            !record.contains("vous@exemple.fr") && !record.contains("4ec83832"),
            "{record}"
        );
        assert_eq!(status["space"], serde_json::Value::Null);
        assert_eq!(status["serviceUrl"], service.base_url().as_str());

        // Nothing else was lost.
        let associations =
            association::load_store(&harness.config_dir.join("associations.json")).unwrap();
        assert_eq!(associations.associations.len(), 1);
        assert_eq!(associations.associations[0].folder, night);
        assert_eq!(queue_states(&harness), [QueueState::Queued]);
        let _ = std::fs::remove_dir_all(&night);
        drop(harness.app);
    }
}

#[test]
fn an_upload_run_stops_on_a_sign_in_the_service_ended_and_the_file_stays_queued() {
    let _serial = serialized();
    let harness = harness(fast());
    forget_files(&harness);
    let night = a_night_waiting_to_be_sent(&harness);
    let detached = exchange("call-from-detached-device");
    let service = FakeService::start(Wake::Instant, move |request| {
        if request.path == "/api/v1/service" {
            fake_service::identity_reply()
        } else {
            reply_of(&detached)
        }
    });
    signed_in_to(&harness, &service.base_url());

    let processed = harness
        .invoke(
            "upload_all",
            serde_json::json!({"sessionId": null, "planId": null, "targetIds": []}),
        )
        .answer(ANSWER)
        .expect("the run ends, it does not fail");

    assert_eq!(processed, 0);
    let upload_events = harness.upload_events.lock().unwrap().clone();
    assert!(
        upload_events
            .iter()
            .any(|event| event["kind"] == "Failed" && event["code"] == "device_detached"),
        "the page reads the stable code: {upload_events:?}"
    );
    assert_eq!(
        harness.space_events.lock().unwrap().clone(),
        [serde_json::json!({"kind": "signed_out", "reason": "device_detached"})]
    );
    assert_eq!(
        queue_states(&harness),
        [QueueState::Queued],
        "a sign-in that ends must not lose the pending upload"
    );
    assert!(!harness.config_dir.join("device-token").exists());
    let _ = std::fs::remove_dir_all(&night);
    drop(harness.app);
}

#[test]
fn the_startup_capability_refresh_learns_of_a_detachment_and_does_not_serve_the_old_profile() {
    let _serial = serialized();
    let harness = harness(fast());
    forget_files(&harness);
    let detached = exchange("call-from-detached-device");
    let service = FakeService::start(Wake::Instant, move |request| {
        if request.path == "/api/v1/service" {
            fake_service::identity_reply()
        } else {
            reply_of(&detached)
        }
    });
    // A profile of that space is cached.
    std::fs::write(
        harness.config_dir.join("capabilities-cache.json"),
        r#"{"capabilities":{"planner.multiple":true,"planner.mosaic":true,"pointing.solve":true,"sync.plan_limit":1000,"gallery.private":true,"portal.invitations":true,"capture.variant_limit":12,"capture.count_limit":9007199254740991,"capture.public":true,"analysis.logs":true,"analysis.progression":true,"storage.max_file_bytes":104857600,"storage.quota_bytes":53687091200,"history.retention_days":730},"fetchedAtUtc":"2026-09-27T00:00:00Z"}"#,
    )
    .unwrap();
    signed_in_to(&harness, &service.base_url());

    let state = harness
        .invoke("capabilities", serde_json::json!({}))
        .answer(ANSWER)
        .unwrap();

    assert_eq!(state["paired"], false, "nothing is paired any more");
    assert_eq!(state["capabilities"], serde_json::Value::Null);
    assert!(
        !harness.config_dir.join("capabilities-cache.json").exists(),
        "the profile of the space this computer left is not served for another"
    );
    assert_eq!(harness.space_events.lock().unwrap().len(), 1);
    drop(harness.app);
}

/// The state `capabilities` gives the interface carries the payment that
/// failed -- the recorded answer of `GET /v1/entitlements`
/// (`entitlements-payment-failed`) -- and keeps it for the next launch even
/// when the service cannot be reached; a payment that resumed clears it.
#[test]
fn the_capabilities_state_carries_a_payment_that_failed_and_keeps_it_offline() {
    let _serial = serialized();
    let harness = harness(fast());
    forget_files(&harness);
    let failed = exchange("entitlements-payment-failed");
    let free = exchange("entitlements-free-account");
    let hold = Arc::new(AtomicBool::new(true));
    let held = Arc::clone(&hold);
    let service = FakeService::start(Wake::Instant, move |request| match request.path.as_str() {
        "/api/v1/service" => fake_service::identity_reply(),
        "/api/v1/capabilities" => fake_service::capabilities_reply(),
        "/api/v1/entitlements" => {
            if held.load(Ordering::SeqCst) {
                reply_of(&failed)
            } else {
                reply_of(&free)
            }
        }
        _ => fake_service::error_reply(404, "not_found", "no such route"),
    });
    let _ = std::fs::remove_file(harness.config_dir.join("capabilities-cache.json"));
    signed_in_to(&harness, &service.base_url());

    let state = harness
        .invoke("capabilities", serde_json::json!({}))
        .answer(ANSWER)
        .unwrap();
    assert_eq!(
        state["standing"],
        serde_json::json!({
            "kind": "paymentFailed",
            "since": "2026-09-30T12:00:00.000Z",
            "readOnlyUntil": "2026-12-29T12:00:00.000Z",
        })
    );

    // The service cannot be reached: the cached standing is what is said.
    let unreachable = FakeService::start(Wake::Held(Duration::from_secs(30)), |_| {
        fake_service::identity_reply()
    });
    signed_in_to(&harness, &unreachable.base_url());
    let state = harness
        .invoke("capabilities", serde_json::json!({}))
        .answer(ANSWER)
        .unwrap();
    assert_eq!(state["standing"]["kind"], "paymentFailed");

    // The payment resumed: the next reading clears it.
    hold.store(false, Ordering::SeqCst);
    signed_in_to(&harness, &service.base_url());
    let state = harness
        .invoke("capabilities", serde_json::json!({}))
        .answer(ANSWER)
        .unwrap();
    assert_eq!(state["standing"], serde_json::Value::Null);
    drop(harness.app);
}

#[test]
fn the_page_reads_the_space_renames_this_computer_and_detaches_it() {
    let _serial = serialized();
    let harness = harness(fast());
    forget_files(&harness);
    let night = a_night_waiting_to_be_sent(&harness);
    // The recorded answers whose token and device are the ones the later
    // exchanges use.
    let replay = Replay::start(&[
        "current-space-for-device",
        "rename-device",
        "detach-this-device",
    ]);
    signed_in_to(&harness, &replay.base_url());
    let token = RECORDED_BEARER;
    let record = |device_id: &str, name: &str| {
        serde_json::json!({
            "link": {
                "portalUrl": replay.base_url(),
                "space": {"id": "4ec83832-fbb7-4f8d-91ca-8c7eefd8ab71", "email": "vous@exemple.fr", "webUrl": "https://portal.example.test"},
                "device": {"id": device_id, "name": name}
            }
        })
        .to_string()
    };
    let arrange = || {
        std::fs::write(harness.config_dir.join("device-token"), token).unwrap();
        std::fs::write(
            harness.config_dir.join("space.json"),
            record("6daf99d9-1c14-4206-bae2-3cec9456a714", "iPhone 16 Pro"),
        )
        .unwrap();
    };

    // Read the space (one short, silent attempt).
    arrange();
    let refreshed = harness
        .invoke("space_refresh", serde_json::json!({}))
        .answer(ANSWER)
        .unwrap();
    assert_eq!(refreshed["paired"], true);
    assert_eq!(refreshed["space"]["webUrl"], "https://portal.example.test");
    assert!(
        harness.service_events().is_empty(),
        "a refresh announces nothing"
    );

    // Rename.
    arrange();
    let renamed = harness
        .invoke(
            "space_rename",
            serde_json::json!({"name": "iPhone de terrain"}),
        )
        .answer(ANSWER)
        .unwrap();
    assert_eq!(renamed["serviceUpdated"], true);
    assert_eq!(renamed["status"]["device"]["name"], "iPhone de terrain");

    // Detach: the service is told, the token and the link are forgotten, the
    // queue and the associations stay.
    arrange();
    let detached = harness
        .invoke("space_detach", serde_json::json!({}))
        .answer(ANSWER)
        .unwrap();
    assert_eq!(detached["serviceNotified"], true);
    assert_eq!(detached["status"]["paired"], false);
    assert_eq!(
        detached["status"]["ended"],
        serde_json::Value::Null,
        "leaving is not being thrown out"
    );
    assert!(!harness.config_dir.join("device-token").exists());
    assert!(!harness.config_dir.join("space.json").exists());
    assert_eq!(queue_states(&harness), [QueueState::Queued]);
    assert_eq!(
        association::load_store(&harness.config_dir.join("associations.json"))
            .unwrap()
            .associations
            .len(),
        1
    );
    replay.assert_played_exactly();
    let _ = std::fs::remove_dir_all(&night);
    drop(harness.app);
}

/// A new sign-in of the same installation in the same space replaces the
/// token: the old one answers `401 device_authentication_required`. The page
/// is asked to sign in again, and nothing is erased.
#[test]
fn a_token_a_newer_sign_in_replaced_brings_the_page_back_and_erases_nothing() {
    let _serial = serialized();
    let harness = harness(fast());
    forget_files(&harness);
    let night = a_night_waiting_to_be_sent(&harness);
    let service = FakeService::start(Wake::Instant, |request| {
        if request.path == "/api/v1/service" {
            fake_service::identity_reply()
        } else {
            fake_service::error_reply(
                401,
                "device_authentication_required",
                "Device authentication is required.",
            )
        }
    });
    signed_in_to(&harness, &service.base_url());

    let error = harness
        .invoke(
            "list_sessions",
            serde_json::json!({"since": null, "limit": null}),
        )
        .answer(ANSWER)
        .expect_err("the token is no longer the one in force");

    assert_eq!(error["code"], "device_authentication_required");
    assert_eq!(
        harness.space_events.lock().unwrap().clone(),
        [serde_json::json!({"kind": "signed_out", "reason": "connection_replaced"})],
        "the page is asked to sign in again"
    );
    let status = harness
        .invoke("space_status", serde_json::json!({}))
        .answer(ANSWER)
        .unwrap();
    assert_eq!(status["paired"], false);
    assert_eq!(status["ended"]["reason"], "connection_replaced");
    // Nothing is erased: the associated folder and the waiting file stay.
    assert_eq!(queue_states(&harness), [QueueState::Queued]);
    assert_eq!(
        association::load_store(&harness.config_dir.join("associations.json"))
            .unwrap()
            .associations
            .len(),
        1
    );
    let _ = std::fs::remove_dir_all(&night);
    drop(harness.app);
}

/// On a private server the same answer is an ordinary revoked device: it is
/// not a sign-in by e-mail code ending, so the page is not sent anywhere and
/// no record is written -- and the file it left waiting is not marked failed.
#[test]
fn a_private_servers_revoked_device_does_not_end_a_sign_in() {
    let _serial = serialized();
    let harness = harness(fast());
    forget_files(&harness);
    let night = a_night_waiting_to_be_sent(&harness);
    let service = FakeService::start(Wake::Instant, |request| {
        if request.path == "/api/v1/service" {
            fake_service::identity_reply()
        } else {
            fake_service::error_reply(
                401,
                "device_authentication_required",
                "Device authentication is required.",
            )
        }
    });
    // A pairing: an address and a token, and no space on record.
    harness.pair_with(&service.base_url());

    let error = harness
        .invoke(
            "list_sessions",
            serde_json::json!({"since": null, "limit": null}),
        )
        .answer(ANSWER)
        .expect_err("the server does not know this device");
    assert_eq!(error["code"], "device_authentication_required");

    harness.pair_with(&service.base_url());
    let processed = harness
        .invoke(
            "upload_all",
            serde_json::json!({"sessionId": null, "planId": null, "targetIds": []}),
        )
        .answer(ANSWER)
        .expect("the run ends, it does not fail");
    assert_eq!(processed, 0);

    assert!(
        harness.space_events.lock().unwrap().is_empty(),
        "a private server's revoked device is none of the e-mail sign-in's business"
    );
    assert!(
        !harness.config_dir.join("space.json").exists(),
        "no record is written for a pairing"
    );
    assert_eq!(
        queue_states(&harness),
        [QueueState::Queued],
        "an authentication problem is not the file's fault"
    );
    let _ = std::fs::remove_dir_all(&night);
    drop(harness.app);
}

#[test]
fn a_refresh_that_cannot_reach_the_service_keeps_what_is_on_disk_silently() {
    let _serial = serialized();
    let harness = harness(fast());
    forget_files(&harness);
    let closed = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port())
    };
    signed_in_to(&harness, &closed);

    let status = harness
        .invoke("space_refresh", serde_json::json!({}))
        .answer(ANSWER)
        .expect("the screen keeps what it has");

    assert_eq!(status["paired"], true);
    assert_eq!(status["space"]["email"], "vous@exemple.fr");
    assert!(harness.space_events.lock().unwrap().is_empty());
    assert!(harness.service_events().is_empty());
    drop(harness.app);
}

#[test]
fn the_queue_the_page_reads_is_the_linked_space_s_and_what_came_before_is_apart() {
    let _serial = serialized();
    let harness = harness(fast());
    forget_files(&harness);
    // A file sent before the queue knew its spaces (to a space since deleted).
    let night = unique_dir("night-before");
    let file = night.join("final").join("sh2-103.tif");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, vec![7_u8; 120]).unwrap();
    let unscoped = UploadQueue::open(&harness.config_dir.join("upload-queue.sqlite")).unwrap();
    let result = astro_sync_core::upload_queue::ResultFile {
        path: file.clone(),
        sha256: "sh2-103".into(),
        size: 120,
        kind: astro_sync_core::upload_queue::ResultKind::Final,
        target: Default::default(),
    };
    unscoped
        .enqueue_results(std::slice::from_ref(&result))
        .unwrap();
    unscoped
        .mark_completed("sh2-103", "capture-of-the-deleted-space")
        .unwrap();
    drop(unscoped);
    signed_in_to(&harness, "https://portal.example.test");

    let status = harness
        .invoke("queue_status", serde_json::json!({}))
        .answer(ANSWER)
        .unwrap();
    assert_eq!(status["spaceId"], SPACE_OF_ASTRO_SYNC);
    assert_eq!(status["entries"], serde_json::json!([]));
    assert_eq!(status["beforeLink"][0]["sha256"], "sh2-103");

    // The same file, imported again for this space: it is to send, not
    // « already known ».
    let queue = space_queue(&harness);
    let outcome = queue
        .enqueue_results(std::slice::from_ref(&result))
        .unwrap();
    assert_eq!(outcome.inserted, 1);
    queue.mark_completed("sh2-103", "capture-here").unwrap();
    // Sent here, it is « already sent » the next time, and can be sent again.
    let again = queue
        .enqueue_results(std::slice::from_ref(&result))
        .unwrap();
    assert_eq!(
        again.already_sent[0].capture_id.as_deref(),
        Some("capture-here")
    );
    let resent = harness
        .invoke("upload_resend", serde_json::json!({"sha256": "sh2-103"}))
        .answer(ANSWER)
        .unwrap();
    assert_eq!(resent, serde_json::json!(true));
    assert_eq!(queue_states(&harness), [QueueState::Queued]);

    // « Effacer l'historique » purges what came before the link only.
    let forgotten = harness
        .invoke("queue_forget_before_link", serde_json::json!({}))
        .answer(ANSWER)
        .unwrap();
    assert_eq!(forgotten, serde_json::json!(1));
    let status = harness
        .invoke("queue_status", serde_json::json!({}))
        .answer(ANSWER)
        .unwrap();
    assert_eq!(status["beforeLink"], serde_json::json!([]));
    assert_eq!(status["entries"][0]["sha256"], "sh2-103");
    let _ = std::fs::remove_dir_all(&night);
    drop(harness.app);
}

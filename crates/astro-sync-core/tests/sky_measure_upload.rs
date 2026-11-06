//! The night's measured sky background goes to the service: the real HTTP
//! transport against the fake service -- one `POST` to
//! `/api/v1/place-sky-measures` with the device's bearer and the body
//! `analysis::sky_night::upload_body` builds, the stored value read back,
//! and a refusal the contract names (`place_not_found`) kept with its code
//! and its sentence.

#[path = "support/fake_service.rs"]
mod fake_service;

use std::time::Duration;

use astro_sync_core::analysis::sky_night::{
    NightSkyMeasure, SKY_METHOD_VERSION, Site, upload_body,
};
use astro_sync_core::gui_bridge::{
    GuiBridgeError, HttpPortalGuiTransport, upload_place_sky_measure,
};
use astro_sync_core::refusal;
use astro_sync_core::service_wait::{WaitHooks, WakePolicy};
use fake_service::{FakeService, Reply, Wake, error_reply};

fn fast() -> WakePolicy {
    WakePolicy {
        total_budget: Duration::from_millis(4_000),
        attempt_timeout: Duration::from_millis(500),
        write_timeout: Duration::from_millis(600),
        chunk_timeout: Duration::from_millis(1_500),
        retry_pause: Duration::from_millis(40),
        notice_after: Duration::from_millis(120),
        connect_timeout: Duration::from_millis(500),
    }
}

fn night() -> NightSkyMeasure {
    NightSkyMeasure {
        night: "2026-10-10".into(),
        sqm: 21.04,
        pose_count: 14,
        method_version: SKY_METHOD_VERSION.into(),
        observed_at: "2026-10-11T05:11:00.000Z".into(),
        site: Site {
            latitude: 45.0,
            longitude: -30.0,
        },
    }
}

const STORED: &str = r#"{"measure":{"placeId":"col-des-melezes","night":"2026-10-10","sqm":21.04,"poseCount":14,"methodVersion":"astro-sync-sky/1","observedAt":"2026-10-11T05:11:00.000Z","sessionId":null,"updatedAt":"2026-10-11T09:00:00.000Z"},"replaced":true}"#;

#[test]
fn the_night_value_is_sent_once_with_the_device_bearer_and_read_back() {
    let service = FakeService::start(Wake::Instant, |request| {
        if request.method == "POST" && request.path == "/api/v1/place-sky-measures" {
            return Reply::Json(200, STORED.to_string());
        }
        error_reply(404, "not_found", "Not found.")
    });
    let transport = HttpPortalGuiTransport::with_options(fast(), WaitHooks::silent()).unwrap();
    let body = upload_body(&night(), None);
    let uploaded =
        upload_place_sky_measure(&transport, &service.base_url(), "device-token", &body).unwrap();
    assert!(uploaded.replaced);
    assert_eq!(uploaded.measure.place_id, "col-des-melezes");
    assert_eq!(uploaded.measure.sqm, 21.04);
    assert_eq!(uploaded.measure.session_id, None);

    let sent: Vec<_> = service
        .requests()
        .into_iter()
        .filter(|request| request.reached_app && request.method == "POST")
        .collect();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].header("authorization"), Some("Bearer device-token"));
    assert_eq!(
        sent[0].json(),
        body,
        "exactly the night's one value, nothing else"
    );
}

#[test]
fn a_place_the_account_does_not_know_is_refused_with_its_own_sentence() {
    let service = FakeService::start(Wake::Instant, |_| {
        error_reply(
            422,
            "place_not_found",
            "No place of this account is within 500 m.",
        )
    });
    let transport = HttpPortalGuiTransport::with_options(fast(), WaitHooks::silent()).unwrap();
    let error = upload_place_sky_measure(
        &transport,
        &service.base_url(),
        "device-token",
        &upload_body(&night(), Some("0b9f3c1e-6d0a-4f5e-9a3b-2c1d0e9f8a7b")),
    )
    .unwrap_err();
    assert!(matches!(&error, GuiBridgeError::Refused { code, .. } if code == "place_not_found"));
    assert_eq!(error.code(), "place_not_found");
    assert!(
        refusal::plain_sentence("place_not_found")
            .unwrap()
            .contains("500 m")
    );
    // An address that is not a service is refused before anything leaves.
    assert!(matches!(
        upload_place_sky_measure(&transport, "not-a-url", "t", &serde_json::json!({})),
        Err(GuiBridgeError::InvalidPortalUrl(_))
    ));
}

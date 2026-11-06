//! Astro Sync signs in to the hosted service with an e-mail address, then
//! the code of six digits received by e-mail, as the application `sync`.
//! These tests run the real client against a fake service that **replays the
//! exchanges the contract records** (`contracts/fixtures/api/`, see
//! `support/contract_replay.rs`): every recorded answer of the routes Astro
//! Sync calls -- the successes and
//! every refusal -- is played, and every request Astro Sync sends is checked
//! against the contract.
//!
//! What is checked, in order: what the service announces (a private server
//! keeps its pairing); the request of a code and its verification, exchange
//! by exchange; that a write is never sent twice (a code is single-use) and
//! that a service that is starting is waited for; the whole journey (sign in,
//! keep the token, read the space, rename, detach); and a sign-in the service
//! ends, which brings the person back to the sign-in screen **without losing
//! a folder association or a pending upload**.

#[path = "support/contract_replay.rs"]
mod contract_replay;
#[path = "support/fake_service.rs"]
mod fake_service;

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use astro_sync_core::association::{self, Association};
use astro_sync_core::dry_run::build_dry_run_report;
use astro_sync_core::gui_bridge::{
    self, GuiBridgeError, GuiSettings, HttpPortalGuiTransport, PortalGuiTransport, QueueDriveEvent,
};
use astro_sync_core::scanner::scan_folder;
use astro_sync_core::service_wait::{
    ServiceEvent, ServiceWaitError, WaitControl, WaitHooks, WakePolicy,
};
use astro_sync_core::space::{
    self, CodeRequested, DeviceIdentity, SignInAnnouncement, SignInEnd, SignedIn, SpaceClient,
    SpaceCreation, SpaceError, SpaceFiles, TokenStore,
};
use astro_sync_core::standing::Standing;
use astro_sync_core::upload_queue::{QueueState, UploadQueue};
use astro_sync_core::uploader::{HttpUploadTransport, UploadError, UploadTarget, upload_entry};
use contract_replay::{Replay, exchange, reply_of};
use fake_service::{FakeService, Reply, UploadModel, Wake, error_reply, identity_reply};

/* --------------------------------- helpers -------------------------------- */

/// The example bearer of the contract's recorded exchanges (`space-current`,
/// `space-devices`): the token those requests carry.
const RECORDED_BEARER: &str = "wl2zxC3Dt4HJgMfhEiU4pUwq0oeW9ODmY8T3moyhKm4";

/// The same policy shape as the default one, on a scale of hundreds of
/// milliseconds so no test waits for a real start.
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

type Events = Arc<Mutex<Vec<ServiceEvent>>>;

fn client_with_events(base_url: &str) -> (SpaceClient, Events) {
    let events: Events = Arc::default();
    let sink = Arc::clone(&events);
    let hooks = WaitHooks::new(WaitControl::default().token(), move |event| {
        sink.lock().unwrap().push(event);
    });
    (
        SpaceClient::with_options(base_url, fast(), hooks).unwrap(),
        events,
    )
}

fn client(base_url: &str) -> SpaceClient {
    SpaceClient::with_options(base_url, fast(), WaitHooks::silent()).unwrap()
}

fn this_computer() -> DeviceIdentity {
    DeviceIdentity {
        installation_id: "astrosync-mac-7f3a9c2e1b4d".to_string(),
        name: "Astro Sync \u{2014} MacBook Pro".to_string(),
        model: None,
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let unique = format!(
            "{}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        );
        let path = std::env::temp_dir().join(format!(
            "astro-sync-space-{label}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn files(&self) -> SpaceFiles {
        SpaceFiles {
            settings: self.0.join("settings.json"),
            device_token: self.0.join("device-token"),
            record: self.0.join("space.json"),
            installation_id: self.0.join("installation-id"),
        }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Default)]
struct MemoryStore {
    token: RefCell<Option<String>>,
}

impl TokenStore for MemoryStore {
    fn save(&self, token: &str) -> Result<(), GuiBridgeError> {
        *self.token.borrow_mut() = Some(token.to_string());
        Ok(())
    }

    fn clear(&self) -> Result<(), GuiBridgeError> {
        *self.token.borrow_mut() = None;
        Ok(())
    }
}

fn recorded_signed_in(id: &str) -> SignedIn {
    serde_json::from_value(exchange(id).response_body).expect("a recorded sign-in")
}

fn recorded_code_requested(id: &str) -> CodeRequested {
    serde_json::from_value(exchange(id).response_body).expect("a recorded code request")
}

fn refused(error: SpaceError) -> space::Refusal {
    match error {
        SpaceError::Refused(refusal) => refusal,
        other => panic!("expected a refusal of the service, got {other:?}"),
    }
}

/* --------------------------- what the service announces ------------------- */

#[test]
fn every_recorded_announcement_is_read_as_the_screen_to_show() {
    let cases = [
        (
            "hosted-service-announces-email-code",
            SignInAnnouncement::EmailCode {
                code_length: 6,
                space_creation: SpaceCreation::Open,
                terms_url: Some("https://www.example.test/terms".to_string()),
                privacy_url: Some("https://www.example.test/privacy".to_string()),
                terms_url_template: None,
                privacy_url_template: None,
            },
        ),
        (
            "hosted-service-announces-legal-pages-per-language",
            SignInAnnouncement::EmailCode {
                code_length: 6,
                space_creation: SpaceCreation::Open,
                terms_url: Some("https://www.example.test/en/terms/".to_string()),
                privacy_url: Some("https://www.example.test/en/privacy/".to_string()),
                terms_url_template: Some("https://www.example.test/{lang}/terms/".to_string()),
                privacy_url_template: Some("https://www.example.test/{lang}/privacy/".to_string()),
            },
        ),
        (
            "hosted-service-without-legal-links",
            SignInAnnouncement::EmailCode {
                code_length: 6,
                space_creation: SpaceCreation::Open,
                terms_url: None,
                privacy_url: None,
                terms_url_template: None,
                privacy_url_template: None,
            },
        ),
        // A private server signing in by e-mail code: the same screens, with
        // the words of a portal of one person and no creation of a space.
        (
            "private-server-announces-closed-email-code",
            SignInAnnouncement::EmailCode {
                code_length: 6,
                space_creation: SpaceCreation::Closed,
                terms_url: None,
                privacy_url: None,
                terms_url_template: None,
                privacy_url_template: None,
            },
        ),
        // A private server keeps its pairing: the code the server shows.
        (
            "private-server-announces-pairing",
            SignInAnnouncement::Pairing,
        ),
        ("service-without-private-api", SignInAnnouncement::None),
        // An announcement that is absent reads as a private server.
        (
            "older-private-server-announces-nothing",
            SignInAnnouncement::Pairing,
        ),
    ];
    for (id, expected) in cases {
        let replay = Replay::start(&[id]);
        let announced = client(&replay.base_url())
            .announcement()
            .unwrap_or_else(|error| panic!("{id}: {error}"));
        assert_eq!(announced, expected, "{id}");
        replay.assert_played_exactly();
    }
}

#[test]
fn something_that_is_not_an_astro_portal_is_told_so() {
    let service = FakeService::start(Wake::Instant, |_| {
        Reply::Json(200, r#"{"hello":"world"}"#.to_string())
    });
    let error = client(&service.base_url()).announcement().unwrap_err();
    assert!(matches!(error, SpaceError::NotAnAstroPortal), "{error:?}");
    assert_eq!(error.code(), "not_astro_service");

    let gone = FakeService::start(Wake::Instant, |_| error_reply(404, "not_found", "no"));
    let error = client(&gone.base_url()).announcement().unwrap_err();
    assert_eq!(error.code(), "not_astro_service");
}

/* ---------------------------- the request of a code ----------------------- */

/// Every exchange of `sign-in-code-request.json`, and how Astro Sync reads it.
#[test]
fn every_recorded_code_request_is_played_and_read_as_the_contract_says() {
    for (id, typed, expected) in [
        (
            "request-code-address-without-space",
            "vous@exemple.fr",
            "request-code-address-without-space",
        ),
        (
            "request-code-address-with-space",
            "vous@exemple.fr",
            "request-code-address-with-space",
        ),
        (
            // The service names the address the message goes to.
            "request-code-normalizes-address",
            "  Jean.Dupont+Astro@Exemple.FR ",
            "request-code-normalizes-address",
        ),
        (
            // Composed Unicode, the domain in punycode, no final dot: the
            // service does it, and Astro Sync keeps the address it answers.
            "request-code-international-domain",
            "you@Bücher.example.",
            "request-code-international-domain",
        ),
    ] {
        let replay = Replay::start(&[id]);
        let requested = client(&replay.base_url())
            .request_code(typed, "fr", &this_computer())
            .unwrap_or_else(|error| panic!("{id}: {error}"));
        assert_eq!(requested, recorded_code_requested(expected), "{id}");
        assert_eq!(requested.code_length, 6);
        assert_eq!(requested.expires_in_seconds, 600);
        assert_eq!(requested.resend_after_seconds, 60);
        assert_eq!(
            replay.requests_to("POST", "/api/v1/sign-in/code").len(),
            1,
            "{id}: a code is asked for once"
        );
        replay.assert_played_exactly();
    }

    // Each refusal: the contract's code, its extra fields, its `Retry-After`,
    // and a request sent exactly once.
    for (id, code, retry_after) in [
        (
            "request-code-resend-too-soon",
            "sign_in_rate_limited",
            Some(42),
        ),
        (
            "request-code-too-many-for-address",
            "sign_in_rate_limited",
            Some(1440),
        ),
        (
            "request-code-too-many-from-origin",
            "sign_in_rate_limited",
            Some(1800),
        ),
        ("request-code-while-locked", "sign_in_locked", Some(892)),
        (
            "request-code-delivery-failed",
            "email_delivery_failed",
            None,
        ),
        ("request-code-not-configured", "sign_in_unavailable", None),
        // The e-mail budget of the whole service is spent: `retryAt`.
        (
            "request-code-service-budget-spent",
            "sign_in_unavailable",
            Some(900),
        ),
        ("request-code-invalid-address", "email_invalid", None),
        // A control or invisible character: refused, never removed.
        ("request-code-invisible-character", "email_invalid", None),
        (
            "request-code-on-private-server",
            "sign_in_wrong_service",
            None,
        ),
        // A private server signing in by e-mail code: not its owner's
        // address, or not installed yet. Plain answers, no wait.
        ("request-code-not-owner", "not_owner", None),
        ("request-code-not-initialized", "not_initialized", None),
    ] {
        let replay = Replay::start(&[id]);
        let error = client(&replay.base_url())
            .request_code("vous@exemple.fr", "fr", &this_computer())
            .unwrap_err();
        assert_eq!(error.code(), code, "{id}: {error:?}");
        let refusal = refused(error);
        assert_eq!(refusal.retry_after_seconds, retry_after, "{id}");
        assert_eq!(
            refusal.retry_at.is_some(),
            retry_after.is_some(),
            "{id}: `retryAt` comes with the waits"
        );
        assert_eq!(
            replay.requests_to("POST", "/api/v1/sign-in/code").len(),
            1,
            "{id}: a refusal is not sent again"
        );
        replay.assert_played_exactly();
    }
}

#[test]
fn an_address_that_is_not_one_never_leaves_the_computer() {
    let replay = Replay::start(&[]);
    for typed in ["vous-exemple.fr", "vous@", ""] {
        let error = client(&replay.base_url())
            .request_code(typed, "fr", &this_computer())
            .unwrap_err();
        assert!(matches!(error, SpaceError::InvalidEmail), "{typed:?}");
        assert_eq!(error.code(), "email_invalid");
        let error = client(&replay.base_url())
            .verify_code(typed, "481207", &this_computer())
            .unwrap_err();
        assert!(matches!(error, SpaceError::InvalidEmail), "{typed:?}");
    }
    assert!(replay.service().requests().is_empty(), "nothing was sent");
}

/// `400 email_invalid`: the service refuses an address with a control or
/// invisible character and never removes it. The address of the recorded
/// exchange, and the usual ones a paste brings, are refused before anything
/// is sent.
#[test]
fn an_address_with_an_invisible_or_control_character_never_leaves_the_computer() {
    let recorded = exchange("request-code-invisible-character");
    let recorded_address = recorded.request_body["email"].as_str().unwrap().to_string();
    assert!(recorded_address.contains('\u{7}'), "{recorded_address:?}");
    let replay = Replay::start(&[]);
    for typed in [
        recorded_address.as_str(),
        "vous\u{200B}@exemple.fr",
        "vous@exemple.fr\u{202E}",
        "\u{FEFF}vous@exemple.fr",
    ] {
        let error = client(&replay.base_url())
            .request_code(typed, "fr", &this_computer())
            .unwrap_err();
        assert!(matches!(error, SpaceError::InvalidEmail), "{typed:?}");
        assert_eq!(error.code(), "email_invalid");
        let error = client(&replay.base_url())
            .verify_code(typed, "481207", &this_computer())
            .unwrap_err();
        assert!(matches!(error, SpaceError::InvalidEmail), "{typed:?}");
    }
    assert!(replay.service().requests().is_empty(), "nothing was sent");
}

/// `503 sign_in_unavailable` with `retryAt`: the e-mail budget of the whole
/// service is spent, the sending of codes is suspended until then -- said,
/// with the wait and the time the service gave. Without `retryAt` the
/// settings are incomplete: the message as it was.
#[test]
fn a_suspended_sending_of_codes_says_so_and_when_it_ends() {
    let replay = Replay::start(&["request-code-service-budget-spent"]);
    let error = client(&replay.base_url())
        .request_code("vous@exemple.fr", "fr", &this_computer())
        .unwrap_err();
    replay.assert_played_exactly();
    assert_eq!(error.code(), "sign_in_unavailable");
    let sentence = error.to_string();
    assert!(
        sentence.contains("Sending codes is paused for the moment"),
        "{sentence}"
    );
    assert!(sentence.contains("in 15 minutes"), "{sentence}");
    assert!(sentence.contains("2026-09-29 21:45 UTC"), "{sentence}");
    let refusal = refused(error);
    assert_eq!(
        refusal.retry_at.as_deref(),
        Some("2026-09-29T21:45:00.000Z")
    );
    assert_eq!(refusal.retry_after_seconds, Some(900));

    let replay = Replay::start(&["request-code-not-configured"]);
    let error = client(&replay.base_url())
        .request_code("vous@exemple.fr", "fr", &this_computer())
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Signing in by e-mail is not available on this service right now."
    );
}

/// A private server signing in by e-mail code refuses another address than
/// its owner's, and every address before its installation: each is said in
/// the words of a portal of one person, never as a wait, and never with the
/// service's own English.
#[test]
fn a_private_portal_that_is_not_yours_or_not_installed_yet_says_so_plainly() {
    for (id, code, sentence) in [
        (
            "request-code-not-owner",
            "not_owner",
            "This portal belongs to someone else. Use the e-mail address chosen when it was set up.",
        ),
        (
            "request-code-not-initialized",
            "not_initialized",
            "This portal does not have an owner yet. Finish setting it up from a browser.",
        ),
    ] {
        let replay = Replay::start(&[id]);
        let error = client(&replay.base_url())
            .request_code("vous@exemple.fr", "fr", &this_computer())
            .unwrap_err();
        replay.assert_played_exactly();
        assert_eq!(error.code(), code, "{id}");
        assert_eq!(error.to_string(), sentence, "{id}");
        let refusal = refused(error);
        assert_eq!(refusal.code, code, "{id}");
        assert!(
            refusal.retry_at.is_none() && refusal.retry_after_seconds.is_none(),
            "{id}: nothing to wait for"
        );
    }
}

/// The lock lasts 15 minutes, then 1 hour, then 24 hours: what is said is
/// what the service said -- its `Retry-After` and its `retryAt` -- and the
/// verification is sent exactly once.
#[test]
fn a_lock_of_a_quarter_hour_an_hour_or_a_day_is_said_as_the_service_said_it() {
    for (seconds, retry_at, said_wait, said_time) in [
        (
            900_u64,
            "2026-09-29T21:35:00.000Z",
            "in 15 minutes",
            "21:35 UTC",
        ),
        (3600, "2026-09-29T22:20:00.000Z", "in 1 hour", "22:20 UTC"),
        (
            86_400,
            "2026-09-30T21:20:00.000Z",
            "in 24 hours",
            "2026-09-30 21:20 UTC",
        ),
    ] {
        let body = format!(
            r#"{{"error":{{"code":"sign_in_locked","message":"Too many attempts."}},"retryAt":"{retry_at}"}}"#
        );
        let service = FakeService::start(Wake::Instant, move |request| {
            if request.method == "GET" {
                return identity_reply();
            }
            Reply::JsonWith(
                429,
                vec![("retry-after".to_string(), seconds.to_string())],
                body.clone(),
            )
        });
        let error = client(&service.base_url())
            .verify_code("vous@exemple.fr", "481207", &this_computer())
            .unwrap_err();
        assert_eq!(error.code(), "sign_in_locked");
        let sentence = error.to_string();
        assert!(sentence.contains(said_wait), "{sentence}");
        assert!(sentence.contains(said_time), "{sentence}");
        assert!(!sentence.contains("Too many attempts"), "{sentence}");
        let refusal = refused(error);
        assert_eq!(refusal.retry_after_seconds, Some(seconds));
        assert_eq!(refusal.retry_at.as_deref(), Some(retry_at));
        assert_eq!(
            service.app_hits("POST", "/api/v1/sign-in/verify"),
            1,
            "a refusal is not sent again"
        );
    }
}

/// `409 device_limit_reached` at the verification: the address is proven and
/// the code used, the space keeps its ceiling of devices. Said: detach one
/// from "My devices", then ask for a new code -- and the verification is
/// never sent twice.
#[test]
fn a_space_at_its_ceiling_of_devices_says_to_detach_one_and_that_the_code_was_used() {
    let service = FakeService::start(Wake::Instant, |request| {
        if request.method == "GET" {
            return identity_reply();
        }
        error_reply(
            409,
            "device_limit_reached",
            "The space has its ceiling of active devices.",
        )
    });
    let error = client(&service.base_url())
        .verify_code("vous@exemple.fr", "481207", &this_computer())
        .unwrap_err();
    assert_eq!(error.code(), "device_limit_reached");
    let sentence = error.to_string();
    assert!(sentence.contains("Detach a device"), "{sentence}");
    assert!(sentence.contains("“My devices”"), "{sentence}");
    assert!(
        sentence.contains("mobile application or on the web"),
        "{sentence}"
    );
    assert!(
        sentence.contains("this one has already been used"),
        "{sentence}"
    );
    assert!(!sentence.contains("ceiling"), "{sentence}");
    assert_eq!(service.app_hits("POST", "/api/v1/sign-in/verify"), 1);
}

#[test]
fn a_code_that_is_not_six_digits_never_leaves_the_computer() {
    let replay = Replay::start(&[]);
    for typed in ["48120", "4812071", "48120a", ""] {
        let error = client(&replay.base_url())
            .verify_code("vous@exemple.fr", typed, &this_computer())
            .unwrap_err();
        assert!(matches!(error, SpaceError::InvalidCode), "{typed:?}");
        assert_eq!(error.code(), "sign_in_code_malformed");
    }
    assert!(replay.service().requests().is_empty(), "nothing was sent");
}

/* --------------------------------- the code ------------------------------- */

/// Every exchange of `sign-in-verify.json` (and of the planned devices file)
/// that ends in a verification, played by the application `sync`.
#[test]
fn every_recorded_verification_is_played_and_read_as_the_contract_says() {
    // Successes. The token, the space and the device are the recorded ones.
    for (id, created, device_name) in [
        ("verify-code-app-creates-space", true, "iPhone 16 Pro"),
        ("verify-code-app-existing-space", false, "iPad Air"),
        (
            // The recorded answer to Astro Sync itself.
            "verify-code-astro-sync",
            false,
            "Astro Sync \u{2014} MacBook Pro",
        ),
        // A phone that left a space is attached to none: never refused.
        ("sign-in-with-phone-without-space", false, "iPhone 16 Pro"),
        // Astro Sync on a computer already signed in to another space: no
        // refusal, no limit -- it only sends.
        (
            "astro-sync-in-two-spaces",
            false,
            "Astro Sync \u{2014} MacBook Pro",
        ),
    ] {
        let replay = Replay::start(&[id]);
        let signed_in = client(&replay.base_url())
            .verify_code("vous@exemple.fr", "481207", &this_computer())
            .unwrap_or_else(|error| panic!("{id}: {error}"));
        assert_eq!(signed_in, recorded_signed_in(id), "{id}");
        assert_eq!(signed_in.created, created, "{id}");
        assert_eq!(signed_in.device.name, device_name, "{id}");
        assert_eq!(signed_in.space.email, "vous@exemple.fr");
        assert_eq!(signed_in.space.web_url, "https://portal.example.test");
        assert_eq!(
            replay.requests_to("POST", "/api/v1/sign-in/verify").len(),
            1,
            "{id}: a code is presented once"
        );
        replay.assert_played_exactly();
    }

    // Refusals.
    for (id, code, attempts, retry_after) in [
        (
            "verify-code-incorrect",
            "sign_in_code_incorrect",
            Some(4),
            None,
        ),
        (
            "verify-code-fifth-failure-locks",
            "sign_in_locked",
            None,
            Some(900),
        ),
        ("verify-while-locked", "sign_in_locked", None, Some(600)),
        ("verify-code-expired", "sign_in_code_expired", None, None),
        (
            // Compared five times, even the right one: "This code has expired."
            "verify-code-compared-five-times",
            "sign_in_code_expired",
            None,
            None,
        ),
        (
            "verify-code-already-used",
            "sign_in_code_expired",
            None,
            None,
        ),
        ("verify-invalid-address", "email_invalid", None, None),
        ("verify-app-without-device", "invalid_request", None, None),
        (
            "verify-too-many-failures-from-origin",
            "sign_in_rate_limited",
            None,
            Some(1500),
        ),
        (
            "verify-on-private-server",
            "sign_in_wrong_service",
            None,
            None,
        ),
    ] {
        let replay = Replay::start(&[id]);
        let error = client(&replay.base_url())
            .verify_code("vous@exemple.fr", "481207", &this_computer())
            .unwrap_err();
        assert_eq!(error.code(), code, "{id}: {error:?}");
        let refusal = refused(error);
        assert_eq!(refusal.attempts_remaining, attempts, "{id}");
        assert_eq!(refusal.retry_after_seconds, retry_after, "{id}");
        assert_eq!(
            replay.requests_to("POST", "/api/v1/sign-in/verify").len(),
            1,
            "{id}: a refusal is not sent again"
        );
        replay.assert_played_exactly();
    }
}

#[test]
fn a_verification_that_never_received_an_answer_is_not_replayed() {
    // The service did its work, then the answer was lost: the code may have
    // been used. The contract: "not replayed blindly; the person asks for a
    // new one".
    let service = FakeService::start(Wake::Instant, |request| {
        if request.method == "GET" {
            identity_reply()
        } else {
            Reply::DropAfterProcessing
        }
    });
    let error = client(&service.base_url())
        .verify_code("vous@exemple.fr", "481207", &this_computer())
        .unwrap_err();
    assert!(
        matches!(error, SpaceError::Wait(ServiceWaitError::NotConfirmed)),
        "{error:?}"
    );
    assert_eq!(error.code(), "service_write_unconfirmed");
    assert_eq!(
        service.raw_hits("POST", "/api/v1/sign-in/verify"),
        1,
        "the code was presented once, whatever happened"
    );
}

/* ---------------------------- the service is starting --------------------- */

#[test]
fn a_service_that_is_asleep_is_woken_first_and_the_code_is_presented_once() {
    let replay = Replay::start_with(
        &["verify-code-astro-sync"],
        Wake::Held(Duration::from_millis(1_300)),
        "hosted-service-announces-email-code",
    );
    let (client, events) = client_with_events(&replay.base_url());

    let signed_in = client
        .verify_code("vous@exemple.fr", "730418", &this_computer())
        .expect("the wake is shorter than the budget");

    assert_eq!(signed_in.device.name, "Astro Sync \u{2014} MacBook Pro");
    assert_eq!(
        *events.lock().unwrap(),
        [ServiceEvent::Starting, ServiceEvent::Ready]
    );
    assert_eq!(
        replay.requests_to("POST", "/api/v1/sign-in/verify").len(),
        1,
        "the code reached the application exactly once"
    );
    replay.assert_played_exactly();
}

#[test]
fn the_applications_own_service_starting_is_waited_out_and_the_write_sent_again() {
    // `503 service_starting`: "nothing was checked, sent or used". The code
    // stays valid and the same write is sent again once the service is ready.
    for (script, called) in [
        (
            ["verify-service-starting", "verify-code-astro-sync"],
            "verify",
        ),
        (
            [
                "request-code-service-starting",
                "request-code-address-without-space",
            ],
            "request",
        ),
    ] {
        let replay = Replay::start(&script);
        let (client, events) = client_with_events(&replay.base_url());
        if called == "verify" {
            let signed_in = client
                .verify_code("vous@exemple.fr", "730418", &this_computer())
                .unwrap();
            assert!(!signed_in.created);
        } else {
            let requested = client
                .request_code("vous@exemple.fr", "fr", &this_computer())
                .unwrap();
            assert_eq!(requested.resend_after_seconds, 60);
        }
        assert_eq!(
            *events.lock().unwrap(),
            [ServiceEvent::Starting, ServiceEvent::Ready],
            "{called}: the wait is announced once, then over"
        );
        replay.assert_played_exactly();
    }
}

#[test]
fn a_service_that_never_finishes_starting_ends_the_wait_within_its_budget() {
    let starting = exchange("verify-service-starting");
    let service = FakeService::start(Wake::Instant, move |request| {
        if request.method == "GET" {
            identity_reply()
        } else {
            reply_of(&starting)
        }
    });
    let (client, events) = client_with_events(&service.base_url());

    let started = std::time::Instant::now();
    let error = client
        .verify_code("vous@exemple.fr", "481207", &this_computer())
        .unwrap_err();

    assert_eq!(error.code(), "service_start_timeout", "{error:?}");
    assert!(
        started.elapsed() < Duration::from_millis(6_000),
        "never forever: {:?}",
        started.elapsed()
    );
    assert_eq!(
        *events.lock().unwrap(),
        [ServiceEvent::Starting, ServiceEvent::Stopped]
    );
    let attempts = service.raw_hits("POST", "/api/v1/sign-in/verify");
    assert!(
        (2..=40).contains(&attempts),
        "a bounded number of tries, paced by what the service asked: {attempts}"
    );
}

#[test]
fn the_person_can_cancel_the_wait_for_a_service_that_is_starting() {
    let starting = exchange("request-code-service-starting");
    let service = FakeService::start(Wake::Instant, move |request| {
        if request.method == "GET" {
            identity_reply()
        } else {
            reply_of(&starting)
        }
    });
    let control = WaitControl::default();
    let hooks = WaitHooks::new(control.token(), |_| {});
    let client = SpaceClient::with_options(
        &service.base_url(),
        WakePolicy {
            retry_pause: Duration::from_millis(400),
            total_budget: Duration::from_secs(30),
            ..fast()
        },
        hooks,
    )
    .unwrap();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        control.cancel_all();
    });

    let started = std::time::Instant::now();
    let error = client
        .request_code("vous@exemple.fr", "fr", &this_computer())
        .unwrap_err();
    canceller.join().unwrap();

    assert_eq!(error.code(), "cancelled", "{error:?}");
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn a_service_that_cannot_be_reached_at_all_fails_at_once() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let (client, events) = client_with_events(&format!("http://127.0.0.1:{port}"));

    let started = std::time::Instant::now();
    let error = client
        .verify_code("vous@exemple.fr", "481207", &this_computer())
        .unwrap_err();

    assert_eq!(error.code(), "network_error", "{error:?}");
    assert!(started.elapsed() < Duration::from_millis(1_500));
    assert!(events.lock().unwrap().is_empty());
}

/* -------------------------------- the space ------------------------------- */

#[test]
fn the_space_of_a_token_and_the_end_of_a_sign_in_are_read_as_recorded() {
    // The token of the recorded request is the one the fake service expects:
    // it is sent as `Authorization: Bearer <token>`.
    let token = RECORDED_BEARER;
    let replay = Replay::start(&["current-space-for-device"]);
    let current = client(&replay.base_url()).current_space(token).unwrap();
    assert_eq!(current.space.email, "vous@exemple.fr");
    assert_eq!(current.space.web_url, "https://portal.example.test");
    assert_eq!(current.device.unwrap().name, "iPhone 16 Pro");
    replay.assert_played_exactly();

    // A token the service does not know any more: the device was detached.
    let replay = Replay::start(&["current-space-without-credential"]);
    let error = client(&replay.base_url()).current_space(token).unwrap_err();
    assert_eq!(error.sign_in_end(), Some(SignInEnd::DeviceDetached));
    assert_eq!(error.code(), "device_detached");
    replay.assert_played_exactly();

    // The space was deleted.
    let replay = Replay::start(&["call-after-deletion"]);
    let error = client(&replay.base_url()).current_space(token).unwrap_err();
    assert_eq!(error.sign_in_end(), Some(SignInEnd::SpaceDeleted));
    assert_eq!(error.code(), "space_deleted");
    replay.assert_played_exactly();

    // A private server has no such route.
    let replay = Replay::start(&["current-space-on-private-server"]);
    let error = client(&replay.base_url()).current_space(token).unwrap_err();
    assert!(error.sign_in_end().is_none(), "{error:?}");
    replay.assert_played_exactly();
}

#[test]
fn the_planned_routes_of_the_devices_are_written_against_their_recorded_shapes() {
    let token = RECORDED_BEARER;
    let device_id = "6daf99d9-1c14-4206-bae2-3cec9456a714";

    let replay = Replay::start(&["rename-device"]);
    let renamed = client(&replay.base_url())
        .rename_device(token, device_id, "iPhone de terrain")
        .unwrap();
    assert_eq!(renamed.name, "iPhone de terrain");
    assert_eq!(renamed.id, device_id);
    replay.assert_played_exactly();

    let replay = Replay::start(&["detach-this-device"]);
    client(&replay.base_url())
        .detach_device(token, device_id)
        .unwrap();
    replay.assert_played_exactly();

    // Astro Sync is never limited, but should a service ever answer so, the
    // refusal is read as what it is.
    let replay = Replay::start(&["detach-this-device-limited"]);
    let error = client(&replay.base_url())
        .detach_device(token, device_id)
        .unwrap_err();
    assert_eq!(refused(error).code, "device_change_limited");
    replay.assert_played_exactly();

    // A device that was detached from elsewhere.
    let detached = exchange("call-from-detached-device");
    let service = FakeService::start(Wake::Instant, move |request| {
        if request.method == "GET" {
            identity_reply()
        } else {
            reply_of(&detached)
        }
    });
    let error = client(&service.base_url())
        .detach_device(token, device_id)
        .unwrap_err();
    assert_eq!(error.sign_in_end(), Some(SignInEnd::DeviceDetached));
}

/* ---------- every recorded exchange is either played or set aside ---------- */

/// The exchanges of the contract that Astro Sync plays above, and the ones it
/// never makes, each with the reason. A new exchange in the contract fails
/// this test until someone decides which it is.
#[test]
fn every_recorded_exchange_is_either_played_or_set_aside_for_a_reason() {
    let played: BTreeSet<&str> = [
        // Announcements.
        "hosted-service-announces-email-code",
        "hosted-service-announces-legal-pages-per-language",
        "hosted-service-without-legal-links",
        "private-server-announces-closed-email-code",
        "private-server-announces-pairing",
        "service-without-private-api",
        "older-private-server-announces-nothing",
        // The request of a code.
        "request-code-address-without-space",
        "request-code-address-with-space",
        "request-code-normalizes-address",
        "request-code-invalid-address",
        "request-code-international-domain",
        "request-code-invisible-character",
        "request-code-service-budget-spent",
        "request-code-resend-too-soon",
        "request-code-too-many-for-address",
        "request-code-too-many-from-origin",
        "request-code-while-locked",
        "request-code-delivery-failed",
        "request-code-service-starting",
        "request-code-not-configured",
        "request-code-on-private-server",
        "request-code-not-owner",
        "request-code-not-initialized",
        // The verification.
        "verify-code-app-creates-space",
        "verify-code-app-existing-space",
        "verify-code-astro-sync",
        "verify-code-incorrect",
        "verify-code-fifth-failure-locks",
        "verify-while-locked",
        "verify-code-expired",
        "verify-code-compared-five-times",
        "verify-code-already-used",
        "verify-invalid-address",
        "verify-app-without-device",
        "verify-too-many-failures-from-origin",
        "verify-service-starting",
        "verify-on-private-server",
        "sign-in-with-phone-without-space",
        "astro-sync-in-two-spaces",
        // The space and its devices.
        "current-space-for-device",
        "current-space-without-credential",
        "current-space-on-private-server",
        "call-after-deletion",
        "rename-device",
        "detach-this-device",
        "detach-this-device-limited",
        "call-from-detached-device",
        // What the billing of the space says: read for one thing, a payment
        // that failed (`the_standing_of_the_space_is_read_from_every_recorded_entitlements_answer`).
        // The answer of a private server is the contract's; the others are
        // Astro Sync's own (`tests/fixtures/standing-answers.json`).
        "entitlements-free-account",
        "entitlements-paid-account",
        "entitlements-administrator-space",
        "entitlements-private-server",
        "entitlements-monthly-subscription",
        "entitlements-offered-subscription",
        "entitlements-ended-subscription",
        "entitlements-payment-failed",
    ]
    .into_iter()
    .collect();
    let set_aside: BTreeSet<(&str, &str)> = [
        (
            "request-code-malformed",
            "a request Astro Sync never makes: it always sends a JSON object",
        ),
        (
            "verify-code-web",
            "the web page's verification: a session and cookies, no device token",
        ),
        (
            "verify-link-app",
            "the link of the message opens no Astro Sync: on a computer it shows the code",
        ),
        (
            "verify-link-invalid",
            "the link of the message opens no Astro Sync",
        ),
        (
            "verify-link-other-address",
            "the link of the message opens no Astro Sync: Astro Sync never sends a link token",
        ),
        (
            "verify-link-without-address",
            "the link of the message opens no Astro Sync: Astro Sync never sends a link token",
        ),
        (
            "verify-link-while-locked",
            "the link of the message opens no Astro Sync: on a computer it shows the code, so a lock never meets it",
        ),
        (
            "current-space-for-web-session",
            "a read by a web session, not by a device token",
        ),
        (
            "current-space-with-a-demo-mode",
            "the demo mode of a demonstration space (`demoModeAllowed`) is not Astro Sync's: it has no demo mode and ignores the flag",
        ),
        (
            "list-devices",
            "\u{201c}My devices\u{201d} is the phone's and the web's; Astro Sync lists nothing",
        ),
        (
            "detach-lost-device",
            "removing another device is the job of \u{201c}My devices\u{201d}; Astro Sync lists nothing",
        ),
        (
            "link-page-visited",
            "the page a link opens is the service's, in a browser: Astro Sync serves and reads nothing of it",
        ),
        (
            "link-page-on-private-server",
            "the page a link opens is the service's, in a browser: Astro Sync serves and reads nothing of it",
        ),
        (
            "delete-space",
            "deleting a space is the phone's and the web's; Astro Sync only sends",
        ),
        (
            "delete-space-without-confirmation",
            "deleting a space is the phone's and the web's; Astro Sync only sends",
        ),
        (
            "delete-plan",
            "the trash of the plans is the phone's and the web's; Astro Sync deletes, restores and erases no plan",
        ),
        (
            "delete-plan-in-progress",
            "the trash of the plans is the phone's and the web's; Astro Sync deletes, restores and erases no plan",
        ),
        (
            "trash-project",
            "the trash of the plans is the phone's and the web's; Astro Sync deletes, restores and erases no plan",
        ),
        (
            "list-deleted",
            "the trash of the plans is the phone's and the web's; Astro Sync deletes, restores and erases no plan",
        ),
        (
            "list-deleted-device-refused",
            "the trash of the plans is the phone's and the web's; Astro Sync deletes, restores and erases no plan",
        ),
        (
            "restore-plan",
            "the trash of the plans is the phone's and the web's; Astro Sync deletes, restores and erases no plan",
        ),
        (
            "restore-plan-night-taken",
            "the trash of the plans is the phone's and the web's; Astro Sync deletes, restores and erases no plan",
        ),
        (
            "restore-project",
            "the trash of the plans is the phone's and the web's; Astro Sync deletes, restores and erases no plan",
        ),
        (
            "erase-plan",
            "the trash of the plans is the phone's and the web's; Astro Sync deletes, restores and erases no plan",
        ),
        (
            "erase-live-plan",
            "the trash of the plans is the phone's and the web's; Astro Sync deletes, restores and erases no plan",
        ),
        (
            "erase-project",
            "the trash of the plans is the phone's and the web's; Astro Sync deletes, restores and erases no plan",
        ),
        (
            "list-plans-names-deleted",
            "Astro Sync reads the plans afresh at each detection and keeps no copy: a deleted plan is simply no longer listed, and `deletedPlanIds` (additive) is for the phones' copies",
        ),
    ]
    .into_iter()
    .collect();
    let set_aside_ids: BTreeSet<&str> = set_aside.iter().map(|(id, _)| *id).collect();
    assert!(
        played.is_disjoint(&set_aside_ids),
        "an exchange cannot be both played and set aside"
    );
    let recorded: BTreeSet<String> = contract_replay::all_exchanges()
        .into_iter()
        .map(|exchange| exchange.id)
        .collect();
    let decided: BTreeSet<String> = played
        .iter()
        .chain(set_aside_ids.iter())
        .map(|id| (*id).to_string())
        .collect();
    let undecided: Vec<&String> = recorded.difference(&decided).collect();
    assert!(
        undecided.is_empty(),
        "recorded exchanges Astro Sync neither plays nor sets aside: {undecided:?}"
    );
    let unknown: Vec<&String> = decided.difference(&recorded).collect();
    assert!(
        unknown.is_empty(),
        "exchanges named here that the contract does not record: {unknown:?}"
    );
}

/* ---------------------- what the billing of the space says ---------------- */

/// A payment that failed puts the space in read-only from the day the hold
/// started -- everything stays, nothing new is accepted. The
/// service says it in `GET /v1/entitlements`; Astro Sync reads that route
/// for this alone, so every answer it records is played -- the answer of a
/// private server, which the contract records, and the others, which are
/// Astro Sync's own and hold only what it reads
/// (`tests/fixtures/standing-answers.json`) -- and the space says nothing
/// unless its subscription is on hold.
#[test]
fn the_standing_of_the_space_is_read_from_every_recorded_entitlements_answer() {
    let transport = HttpPortalGuiTransport::with_options(fast(), WaitHooks::silent()).unwrap();
    let recorded: Vec<_> = contract_replay::all_exchanges()
        .into_iter()
        .filter(|exchange| {
            exchange.file == "standing-answers.json" || exchange.id == "entitlements-private-server"
        })
        .collect();
    assert!(
        recorded.len() >= 8,
        "the recorded answers of the entitlements: {}",
        recorded.len()
    );
    for exchange in recorded {
        let bearer = exchange
            .request_headers
            .iter()
            .find(|(name, _)| name == "authorization")
            .and_then(|(_, value)| value.strip_prefix("Bearer "))
            .unwrap_or_else(|| panic!("{} carries no bearer", exchange.id))
            .to_string();
        let replay = Replay::start(&[exchange.id.as_str()]);
        let standing = transport
            .fetch_standing(&replay.base_url(), &bearer)
            .unwrap_or_else(|error| panic!("{}: {error:?}", exchange.id));
        replay.assert_played_exactly();
        if exchange.id == "entitlements-payment-failed" {
            // Ten days into a hold that started on 30/09/2026: read-only until
            // 29/12/2026, the 90 days the rules give.
            assert_eq!(
                standing,
                Some(Standing::PaymentFailed {
                    since: "2026-09-30T12:00:00.000Z".to_string(),
                    read_only_until: "2026-12-29T12:00:00.000Z".to_string(),
                })
            );
        } else {
            // A free space, a purchase, an offered or a monthly subscription, an
            // administrator, a private server, and a subscription that ended
            // (its refusal at the upload already says so): nothing to say.
            assert_eq!(standing, None, "{}", exchange.id);
        }
    }
}

/// What Astro Sync records of the answers of the route is what it reads of
/// them and nothing more: a list of purchases (their kind, their state, the
/// two dates of a hold), whether a subscription is in force, whether the
/// space is an administrator's.
#[test]
fn the_standing_answers_hold_only_what_astro_sync_reads() {
    let recorded: Vec<_> = contract_replay::all_exchanges()
        .into_iter()
        .filter(|exchange| exchange.file == "standing-answers.json")
        .collect();
    assert!(
        recorded.len() >= 7,
        "the standing answers of Astro Sync: {}",
        recorded.len()
    );
    for exchange in recorded {
        let body = exchange.response_body.as_object().expect("an object");
        for key in body.keys() {
            assert!(
                ["purchases", "subscription", "serviceAdministrator"].contains(&key.as_str()),
                "{}: {key} is not read by Astro Sync",
                exchange.id
            );
        }
        for purchase in body["purchases"].as_array().expect("a list of purchases") {
            for key in purchase.as_object().expect("a purchase").keys() {
                assert!(
                    ["kind", "state", "endedAt", "readOnlyUntil"].contains(&key.as_str()),
                    "{}: {key} of a purchase is not read by Astro Sync",
                    exchange.id
                );
            }
        }
    }
}

/// A private server has no such route, or refuses it: the read is a refusal
/// as it is (the refresh of the capabilities believes it and says nothing),
/// and the end of a sign-in stays the end of a sign-in.
#[test]
fn an_entitlements_read_that_is_refused_is_a_refusal() {
    let transport = HttpPortalGuiTransport::with_options(fast(), WaitHooks::silent()).unwrap();
    let service = FakeService::start(Wake::Instant, move |request| {
        if request.path == "/api/v1/entitlements" {
            error_reply(404, "not_found", "No such route.")
        } else {
            identity_reply()
        }
    });
    assert!(matches!(
        transport.fetch_standing(&service.base_url(), RECORDED_BEARER),
        Err(GuiBridgeError::Refused { .. })
    ));

    let service = FakeService::start(Wake::Instant, move |request| {
        if request.path == "/api/v1/entitlements" {
            error_reply(
                401,
                "authentication_required",
                "A device token is required.",
            )
        } else {
            identity_reply()
        }
    });
    assert!(matches!(
        transport.fetch_standing(&service.base_url(), RECORDED_BEARER),
        Err(GuiBridgeError::Refused { .. } | GuiBridgeError::SignInEnded(_))
    ));
}

/* -------------------------------- the whole journey ----------------------- */

/// An address, a code, the token kept like a pairing's, the space read, the
/// computer renamed, then detached -- every request checked against the
/// contract, and the token nowhere but in the store.
#[test]
fn the_whole_journey_signs_in_keeps_the_token_renames_and_detaches() {
    let dir = TempDir::new("journey");
    let files = dir.files();
    let store = MemoryStore::default();
    let replay = Replay::start(&[
        "hosted-service-announces-email-code",
        "request-code-address-without-space",
        // The recorded answer whose token and device the later exchanges use.
        "verify-code-app-creates-space",
        "current-space-for-device",
        "rename-device",
        "detach-this-device",
    ]);
    let client = client(&replay.base_url());
    let service_url = replay.base_url();

    // 1. What the service announces.
    assert!(matches!(
        client.announcement().unwrap(),
        SignInAnnouncement::EmailCode { .. }
    ));

    // 2. The code, then the verification.
    let device =
        DeviceIdentity::for_installation(&files, "Astro Sync \u{2014} MacBook Pro").unwrap();
    let requested = client
        .request_code("vous@exemple.fr", "fr", &device)
        .unwrap();
    assert_eq!(requested.email, "vous@exemple.fr");
    let signed_in = client
        .verify_code("vous@exemple.fr", "481207", &device)
        .unwrap();
    assert!(signed_in.created);
    let token = signed_in.device_token.expose().to_string();
    assert!(
        !format!("{signed_in:?}").contains(&token),
        "the token is in no Debug output"
    );

    // 3. Kept like a pairing's: the store, the address, the name.
    let status = space::complete_sign_in(&files, &store, &service_url, &signed_in).unwrap();
    assert_eq!(store.token.borrow().as_deref(), Some(token.as_str()));
    assert!(status.paired);
    assert_eq!(status.space.as_ref().unwrap().email, "vous@exemple.fr");
    let settings = gui_bridge::load_settings(&files.settings).unwrap();
    assert_eq!(settings.portal_url.as_deref(), Some(service_url.as_str()));
    assert_eq!(settings.device_label.as_deref(), Some("iPhone 16 Pro"));
    for file in [&files.settings, &files.record, &files.installation_id] {
        assert!(
            !std::fs::read_to_string(file).unwrap().contains(&token),
            "{}: the token is in no file of the sign-in",
            file.display()
        );
    }

    // 4. Read the space again.
    let refreshed = space::refresh_space(&client, &files, &store, &token).unwrap();
    assert_eq!(
        refreshed.space.unwrap().web_url,
        "https://portal.example.test"
    );

    // 5. Rename.
    let renamed =
        space::rename_computer(&client, &files, &store, &token, "iPhone de terrain").unwrap();
    assert!(renamed.service_updated);
    assert_eq!(renamed.status.device.unwrap().name, "iPhone de terrain");
    assert_eq!(
        gui_bridge::load_settings(&files.settings)
            .unwrap()
            .device_label
            .as_deref(),
        Some("iPhone de terrain")
    );

    // 6. Detach.
    let detached = space::detach_computer(&client, &files, &store, &token).unwrap();
    assert!(detached.service_notified);
    assert!(store.token.borrow().is_none(), "the token is forgotten");
    assert!(!detached.status.paired);
    assert_eq!(detached.status.space, None);
    assert_eq!(
        detached.status.ended, None,
        "leaving is not being thrown out"
    );

    replay.assert_played_exactly();
    // Astro Sync never attaches a device: that route is a phone's.
    assert!(
        replay
            .service()
            .requests()
            .iter()
            .all(|request| request.path != "/api/v1/sign-in/attach"),
        "Astro Sync never calls POST /api/v1/sign-in/attach"
    );
    // The identity of the installation stays: it is this installation's.
    assert!(files.installation_id.exists());
}

#[test]
fn the_installation_is_announced_as_the_same_one_at_every_sign_in() {
    let dir = TempDir::new("installation");
    let files = dir.files();
    let first = DeviceIdentity::for_installation(&files, "Astro Sync").unwrap();
    let second = DeviceIdentity::for_installation(&files, "Astro Sync").unwrap();
    assert_eq!(first.installation_id, second.installation_id);
    assert!(first.installation_id.len() >= 16);
}

#[test]
fn a_rename_the_service_does_not_answer_yet_is_kept_on_this_computer() {
    // `PATCH /v1/devices/{id}` is a route of the space devices.
    let dir = TempDir::new("rename-unanswered");
    let files = dir.files();
    let store = MemoryStore::default();
    let signed_in = recorded_signed_in("verify-code-astro-sync");
    let service = FakeService::start(Wake::Instant, |request| {
        if request.method == "GET" {
            identity_reply()
        } else {
            Reply::Text(405, String::new())
        }
    });
    space::complete_sign_in(&files, &store, &service.base_url(), &signed_in).unwrap();

    let outcome = space::rename_computer(
        &client(&service.base_url()),
        &files,
        &store,
        signed_in.device_token.expose(),
        "Bureau",
    )
    .unwrap();

    assert!(!outcome.service_updated);
    assert_eq!(outcome.status.device.unwrap().name, "Bureau");
    assert!(outcome.status.paired, "nothing else changed");
}

#[test]
fn detaching_is_carried_out_here_even_when_the_service_cannot_be_told() {
    let dir = TempDir::new("detach-offline");
    let files = dir.files();
    let store = MemoryStore::default();
    let signed_in = recorded_signed_in("verify-code-astro-sync");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let unreachable = format!("http://127.0.0.1:{port}");
    space::complete_sign_in(&files, &store, &unreachable, &signed_in).unwrap();

    let outcome = space::detach_computer(
        &client(&unreachable),
        &files,
        &store,
        signed_in.device_token.expose(),
    )
    .unwrap();

    assert!(!outcome.service_notified);
    assert!(!outcome.status.paired);
    assert!(store.token.borrow().is_none());
}

#[test]
fn cancelling_the_wait_cancels_the_detachment() {
    let dir = TempDir::new("detach-cancel");
    let files = dir.files();
    let store = MemoryStore::default();
    let signed_in = recorded_signed_in("verify-code-astro-sync");
    let service = FakeService::start(Wake::Held(Duration::from_secs(30)), |_| identity_reply());
    space::complete_sign_in(&files, &store, &service.base_url(), &signed_in).unwrap();
    let control = WaitControl::default();
    let client = SpaceClient::with_options(
        &service.base_url(),
        WakePolicy {
            attempt_timeout: Duration::from_millis(3_000),
            total_budget: Duration::from_millis(6_000),
            ..fast()
        },
        WaitHooks::new(control.token(), |_| {}),
    )
    .unwrap();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        control.cancel_all();
    });

    let error = space::detach_computer(&client, &files, &store, signed_in.device_token.expose())
        .unwrap_err();
    canceller.join().unwrap();

    assert_eq!(error.code(), "cancelled");
    assert!(store.token.borrow().is_some(), "still signed in");
    assert!(space::space_status(&files, true).unwrap().space.is_some());
}

/* ---------------- a sign-in the service ends: nothing is lost -------------- */

struct Night {
    folder: PathBuf,
    associations: PathBuf,
    queue: PathBuf,
}

/// A folder associated with a session, and a file waiting in the upload
/// queue.
fn a_night_waiting_to_be_sent(dir: &TempDir) -> Night {
    let folder = dir.path().join("night");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("frame.tif"), vec![7_u8; 300]).unwrap();
    let associations = dir.path().join("associations.json");
    let association = Association::existing("session-1", &folder).unwrap();
    association::save_association(&associations, &association).unwrap();
    let queue_path = dir.path().join("upload-queue.sqlite");
    let queue = UploadQueue::open(&queue_path).unwrap();
    let report = build_dry_run_report(&scan_folder(&folder).unwrap());
    queue.enqueue_from_dry_run(&report).unwrap();
    Night {
        folder,
        associations,
        queue: queue_path,
    }
}

#[test]
fn a_service_that_ends_the_sign_in_brings_back_the_sign_in_screen_and_loses_nothing() {
    for (exchange_id, end) in [
        ("call-from-detached-device", SignInEnd::DeviceDetached),
        ("call-after-deletion", SignInEnd::SpaceDeleted),
    ] {
        let dir = TempDir::new("ended");
        let files = dir.files();
        let store = MemoryStore::default();
        let night = a_night_waiting_to_be_sent(&dir);
        let signed_in = recorded_signed_in("verify-code-astro-sync");
        let ends = exchange(exchange_id);
        // Whatever route it is asked, the service says this computer is out.
        let service = FakeService::start(Wake::Instant, move |request| {
            if request.method == "GET" && request.path == "/api/v1/service" {
                identity_reply()
            } else {
                reply_of(&ends)
            }
        });
        space::complete_sign_in(&files, &store, &service.base_url(), &signed_in).unwrap();
        let token = signed_in.device_token.expose();

        // The queue: the upload stops, and the file is NOT marked failed.
        let queue = UploadQueue::open(&night.queue).unwrap();
        let transport =
            HttpUploadTransport::with_options(service.base_url(), fast(), WaitHooks::silent())
                .unwrap();
        let entry = queue.next_pending().unwrap().unwrap();
        let error = upload_entry(
            &transport,
            &queue,
            token,
            &entry,
            &UploadTarget::default(),
            |_, _| {},
        )
        .unwrap_err();
        assert!(
            matches!(&error, UploadError::SignInEnded(seen) if *seen == end),
            "{error:?}"
        );
        assert!(!error.is_terminal());
        assert_eq!(error.code(), end.code());
        assert_eq!(
            queue.list().unwrap()[0].state,
            QueueState::Queued,
            "a sign-in that ends must not lose the pending upload"
        );

        // "Upload all" says why it stopped, with the code the screen keys on.
        let mut events = Vec::new();
        let processed = gui_bridge::drive_queue(
            &transport,
            &queue,
            token,
            &UploadTarget::default(),
            &gui_bridge::DriveOptions::interactive(),
            || true,
            |event| events.push(event),
        )
        .unwrap();
        assert_eq!(processed, 0);
        assert!(
            events.iter().any(|event| matches!(
                event,
                QueueDriveEvent::Failed { code, .. } if code == end.code()
            )),
            "{events:?}"
        );
        assert_eq!(queue.list().unwrap()[0].state, QueueState::Queued);

        // Any other call that carries the token says the same.
        let sessions = gui_bridge::list_sessions(
            &HttpPortalGuiTransport::with_options(fast(), WaitHooks::silent()).unwrap(),
            &service.base_url(),
            token,
            None,
            None,
        )
        .unwrap_err();
        assert!(matches!(&sessions, GuiBridgeError::SignInEnded(seen) if *seen == end));
        assert_eq!(sessions.code(), end.code());

        // The sign-in ends: the token is forgotten, the reason kept.
        let status = space::end_sign_in(&files, &store, end).unwrap();
        assert!(store.token.borrow().is_none());
        assert!(!status.paired);
        assert_eq!(status.ended.as_ref().unwrap().reason, end);
        // The space is forgotten: the token, the space, its address. What is
        // left says why, and when.
        assert_eq!(status.space, None);
        let record = std::fs::read_to_string(&files.record).unwrap();
        assert!(
            !record.contains("vous@exemple.fr") && !record.contains("4ec83832"),
            "nothing of the space stays on disk: {record}"
        );
        // The address of the service is kept for the next sign-in.
        assert_eq!(status.service_url, service.base_url());

        // Nothing else was lost: the associated folder, the waiting file.
        let associations = association::load_store(&night.associations).unwrap();
        assert_eq!(associations.associations.len(), 1);
        assert_eq!(associations.associations[0].folder, night.folder);
        assert_eq!(
            UploadQueue::open(&night.queue).unwrap().list().unwrap()[0].state,
            QueueState::Queued
        );

        // Signing in again picks everything up where it was left: the same
        // file is sent, with the new token.
        let model = Arc::new(UploadModel::default());
        let app_model = Arc::clone(&model);
        let awake = FakeService::start(Wake::Instant, move |request| {
            app_model
                .reply(request, 100)
                .unwrap_or_else(|| error_reply(404, "not_found", "no such route"))
        });
        let again = recorded_signed_in("verify-code-app-existing-space");
        let back = space::complete_sign_in(&files, &store, &awake.base_url(), &again).unwrap();
        assert!(back.paired);
        assert_eq!(back.ended, None, "the sentence goes once signed in again");
        let transport =
            HttpUploadTransport::with_options(awake.base_url(), fast(), WaitHooks::silent())
                .unwrap();
        let queue = UploadQueue::open(&night.queue).unwrap();
        let entry = queue.next_pending().unwrap().unwrap();
        upload_entry(
            &transport,
            &queue,
            again.device_token.expose(),
            &entry,
            &UploadTarget::default(),
            |_, _| {},
        )
        .unwrap();
        assert_eq!(queue.list().unwrap()[0].state, QueueState::Completed);
        assert_eq!(model.captures_created(), 1);
    }
}

#[test]
fn a_refresh_that_finds_the_device_detached_ends_the_sign_in() {
    let dir = TempDir::new("refresh-detached");
    let files = dir.files();
    let store = MemoryStore::default();
    let signed_in = recorded_signed_in("verify-code-astro-sync");
    let detached = exchange("call-from-detached-device");
    let service = FakeService::start(Wake::Instant, move |request| {
        if request.path == "/api/v1/service" {
            identity_reply()
        } else {
            reply_of(&detached)
        }
    });
    space::complete_sign_in(&files, &store, &service.base_url(), &signed_in).unwrap();

    let error = space::refresh_space(
        &client(&service.base_url()),
        &files,
        &store,
        signed_in.device_token.expose(),
    )
    .unwrap_err();

    assert_eq!(error.sign_in_end(), Some(SignInEnd::DeviceDetached));
    assert!(store.token.borrow().is_none());
    let status = space::space_status(&files, false).unwrap();
    assert_eq!(
        status.ended.unwrap().reason,
        SignInEnd::DeviceDetached,
        "what the sign-in screen says"
    );
}

#[test]
fn a_refresh_that_cannot_reach_the_service_changes_nothing() {
    let dir = TempDir::new("refresh-offline");
    let files = dir.files();
    let store = MemoryStore::default();
    let signed_in = recorded_signed_in("verify-code-astro-sync");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let unreachable = format!("http://127.0.0.1:{port}");
    space::complete_sign_in(&files, &store, &unreachable, &signed_in).unwrap();

    let error = space::refresh_space(
        &client(&unreachable),
        &files,
        &store,
        signed_in.device_token.expose(),
    )
    .unwrap_err();

    assert_eq!(error.code(), "network_error");
    assert!(store.token.borrow().is_some(), "still signed in");
    let status = space::space_status(&files, true).unwrap();
    assert_eq!(status.space.unwrap().email, "vous@exemple.fr");
}

/* ------------- a space that is gone, and a refusal nobody knows yet -------- */

#[test]
fn a_space_that_is_no_longer_available_is_a_known_refusal_with_nothing_to_retry() {
    // `403` `space_unavailable` on the verification: the code is right, but
    // the space of this address is no longer available (the person was
    // disabled). A refusal of the contract, not an error to retry.
    let service = FakeService::start(Wake::Instant, |request| {
        if request.method == "GET" {
            identity_reply()
        } else {
            error_reply(
                403,
                "space_unavailable",
                "This space is no longer available.",
            )
        }
    });

    let error = client(&service.base_url())
        .verify_code("vous@exemple.fr", "481207", &this_computer())
        .unwrap_err();

    assert_eq!(error.code(), "space_unavailable", "{error:?}");
    let refusal = refused(error);
    assert_eq!(
        (refusal.status, refusal.code.as_str()),
        (403, "space_unavailable")
    );
    assert_eq!(
        service.raw_hits("POST", "/api/v1/sign-in/verify"),
        1,
        "a refusal is not sent again"
    );
    let sentence = SpaceError::Refused(refusal).to_string();
    assert!(sentence.contains("is no longer available"), "{sentence}");
    assert!(
        !sentence.contains("Try again") && !sentence.contains("This space"),
        "nothing to retry, and never the service's own English: {sentence}"
    );
}

#[test]
fn a_refusal_nobody_knows_yet_is_the_generic_sentence_never_a_raw_error() {
    for (status, code) in [(403, "a_refusal_from_the_future"), (400, "another_one")] {
        let service = FakeService::start(Wake::Instant, move |request| {
            if request.method == "GET" {
                identity_reply()
            } else {
                error_reply(status, code, "The service's own English text.")
            }
        });
        let error = client(&service.base_url())
            .verify_code("vous@exemple.fr", "481207", &this_computer())
            .unwrap_err();
        assert_eq!(error.code(), "portal_refused", "{error:?}");
        let sentence = error.to_string();
        assert!(
            !sentence.contains(code) && !sentence.contains("English"),
            "{sentence}"
        );
        assert!(sentence.contains("update Astro Sync"), "{sentence}");
    }
    // An answer that is not even the v1 envelope (a gateway page, a 500 of
    // the platform) says the same.
    let service = FakeService::start(Wake::Instant, |request| {
        if request.method == "GET" {
            identity_reply()
        } else {
            Reply::Text(500, "<html>boom</html>".to_string())
        }
    });
    let error = client(&service.base_url())
        .request_code("vous@exemple.fr", "fr", &this_computer())
        .unwrap_err();
    assert_eq!(error.code(), "portal_refused");
    assert!(!error.to_string().contains("boom"), "{error}");
}

/* ---------- what the space devices say of Astro Sync ---------------- */

#[test]
fn astro_sync_is_never_refused_nor_limited_and_a_service_that_did_so_anyway_is_the_generic_sentence()
 {
    // Astro Sync (`application: "sync"`) is never asked to attach a device and
    // never answered `device_attached_elsewhere` or `device_change_limited`.
    // Were a service to say so anyway, it is a refusal nobody knows how to
    // explain here: the generic sentence, no raw error, and the attachment
    // token of the answer stays where it is.
    for (code, body) in [
        (
            "device_attached_elsewhere",
            r#"{"error":{"code":"device_attached_elsewhere","message":"This device is attached to another space."},"attachment":{"token":"h344OawrYXPkmdF3eptvJXYHfK7sKJCB4MBfHGiqPfE","expiresAt":"2026-09-29T21:25:30.000Z","currentSpaceEmail":"v…@exemple.fr"}}"#,
        ),
        (
            "device_change_limited",
            r#"{"error":{"code":"device_change_limited","message":"This device already left a space in the last 24 hours."},"retryAt":"2026-09-30T09:30:00.000Z","currentSpaceEmail":"v…@exemple.fr"}"#,
        ),
    ] {
        let service = FakeService::start(Wake::Instant, move |request| {
            if request.method == "GET" {
                identity_reply()
            } else {
                Reply::Json(409, body.to_string())
            }
        });
        let error = client(&service.base_url())
            .verify_code("vous@exemple.fr", "730418", &this_computer())
            .unwrap_err();

        assert_eq!(error.code(), "portal_refused", "{code}: {error:?}");
        let sentence = error.to_string();
        assert!(sentence.contains("update Astro Sync"), "{code}: {sentence}");
        for hidden in [code, "h344Oaw", "v…@", "English", "This device"] {
            assert!(
                !sentence.contains(hidden),
                "{code}: the sentence shows {hidden:?}: {sentence}"
            );
        }
        assert_eq!(
            service.raw_hits("POST", "/api/v1/sign-in/verify"),
            1,
            "{code}: not sent again"
        );
        assert_eq!(
            service.raw_hits("POST", "/api/v1/sign-in/attach"),
            0,
            "{code}: Astro Sync never attaches"
        );
    }
}

#[test]
fn a_token_a_newer_sign_in_replaced_asks_to_sign_in_again_and_erases_nothing() {
    // A new sign-in of the same installation in the same space replaces the
    // token: the old one answers `401 device_authentication_required`.
    let dir = TempDir::new("replaced");
    let files = dir.files();
    let store = MemoryStore::default();
    let night = a_night_waiting_to_be_sent(&dir);
    let signed_in = recorded_signed_in("verify-code-astro-sync");
    let service = FakeService::start(Wake::Instant, |request| {
        if request.path == "/api/v1/service" {
            identity_reply()
        } else {
            error_reply(
                401,
                "device_authentication_required",
                "Device authentication is required.",
            )
        }
    });
    space::complete_sign_in(&files, &store, &service.base_url(), &signed_in).unwrap();
    let token = signed_in.device_token.expose();
    let end = SignInEnd::ConnectionReplaced;

    // The upload stops and the file is not marked failed: it is not at fault.
    let queue = UploadQueue::open(&night.queue).unwrap();
    let transport =
        HttpUploadTransport::with_options(service.base_url(), fast(), WaitHooks::silent()).unwrap();
    let entry = queue.next_pending().unwrap().unwrap();
    let error = upload_entry(
        &transport,
        &queue,
        token,
        &entry,
        &UploadTarget::default(),
        |_, _| {},
    )
    .unwrap_err();
    assert!(
        matches!(&error, UploadError::SignInEnded(seen) if *seen == end),
        "{error:?}"
    );
    assert!(!error.is_terminal());
    assert_eq!(error.code(), "device_authentication_required");
    assert_eq!(queue.list().unwrap()[0].state, QueueState::Queued);

    // Any other call that carries the token says the same.
    let sessions = gui_bridge::list_sessions(
        &HttpPortalGuiTransport::with_options(fast(), WaitHooks::silent()).unwrap(),
        &service.base_url(),
        token,
        None,
        None,
    )
    .unwrap_err();
    assert!(matches!(&sessions, GuiBridgeError::SignInEnded(seen) if *seen == end));

    // Reading the space ends the sign-in: the token is forgotten, the reason
    // kept -- and nothing else is erased.
    let error =
        space::refresh_space(&client(&service.base_url()), &files, &store, token).unwrap_err();
    assert_eq!(error.sign_in_end(), Some(end));
    assert!(store.token.borrow().is_none());
    let status = space::space_status(&files, false).unwrap();
    assert_eq!(status.ended.unwrap().reason, end);
    assert_eq!(status.space, None);
    let associations = association::load_store(&night.associations).unwrap();
    assert_eq!(associations.associations.len(), 1);
    assert_eq!(
        UploadQueue::open(&night.queue).unwrap().list().unwrap()[0].state,
        QueueState::Queued
    );
    // The sentence says what to do, and that nothing is lost.
    let sentence = SpaceError::SignInEnded(end).to_string();
    assert!(
        sentence.contains("Sign in again") && sentence.contains("are kept"),
        "{sentence}"
    );
}

#[test]
fn a_private_servers_revoked_device_is_not_this_sign_in_ending() {
    // On a private server `device_authentication_required` is an ordinary
    // revoked device: it ends a sign-in by e-mail code only when there is one
    // (a space on record). The two other answers always do.
    let dir = TempDir::new("private-revoked");
    let files = dir.files();
    let store = MemoryStore::default();

    // A pairing: no space on record.
    gui_bridge::save_settings(
        &files.settings,
        &GuiSettings {
            portal_url: Some("https://private.example.test".to_string()),
            device_label: Some("Bureau".to_string()),
            locale: None,
        },
    )
    .unwrap();
    assert!(!space::ends_this_sign_in(&files, SignInEnd::ConnectionReplaced).unwrap());
    assert!(space::ends_this_sign_in(&files, SignInEnd::DeviceDetached).unwrap());
    assert!(space::ends_this_sign_in(&files, SignInEnd::SpaceDeleted).unwrap());

    // Signed in to a space: it does.
    space::complete_sign_in(
        &files,
        &store,
        "https://portal.example.test",
        &recorded_signed_in("verify-code-astro-sync"),
    )
    .unwrap();
    assert!(space::ends_this_sign_in(&files, SignInEnd::ConnectionReplaced).unwrap());

    // And a private server's revoked device never marks a waiting file failed.
    let night = a_night_waiting_to_be_sent(&dir);
    let private = FakeService::start(Wake::Instant, |request| {
        if request.path == "/api/v1/service" {
            reply_of(&exchange("private-server-announces-pairing"))
        } else {
            error_reply(
                401,
                "device_authentication_required",
                "Device authentication is required.",
            )
        }
    });
    let queue = UploadQueue::open(&night.queue).unwrap();
    let entry = queue.next_pending().unwrap().unwrap();
    let transport =
        HttpUploadTransport::with_options(private.base_url(), fast(), WaitHooks::silent()).unwrap();
    let error = upload_entry(
        &transport,
        &queue,
        "a-revoked-token",
        &entry,
        &UploadTarget::default(),
        |_, _| {},
    )
    .unwrap_err();
    assert!(!error.is_terminal(), "{error:?}");
    assert_eq!(queue.list().unwrap()[0].state, QueueState::Queued);
}

/* ------------------------- a private server is unchanged ------------------- */

#[test]
fn a_private_server_keeps_its_pairing_and_is_not_asked_for_an_address() {
    // Announced as `pairing`: the interface shows the code field, not the
    // address. The pairing itself is `gui_bridge::claim_device`, untouched.
    let service = FakeService::start(Wake::Instant, |request| {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/api/v1/service") => reply_of(&exchange("private-server-announces-pairing")),
            ("POST", "/api/v1/devices/claim") => {
                Reply::Json(201, r#"{"deviceToken":"paired-token"}"#.to_string())
            }
            _ => reply_of(&exchange("request-code-on-private-server")),
        }
    });
    assert_eq!(
        client(&service.base_url()).announcement().unwrap(),
        SignInAnnouncement::Pairing
    );
    // The routes of the hosted service do not exist there.
    let error = client(&service.base_url())
        .request_code("vous@exemple.fr", "fr", &this_computer())
        .unwrap_err();
    assert_eq!(error.code(), "sign_in_wrong_service");
    // And the pairing works exactly as before.
    let transport = HttpPortalGuiTransport::with_options(fast(), WaitHooks::silent()).unwrap();
    let token = gui_bridge::claim_device(&transport, &service.base_url(), "abcd1234", "Bureau")
        .expect("pairing with a private server is unchanged");
    assert_eq!(token, "paired-token");
    assert_eq!(service.raw_hits("POST", "/api/v1/sign-in/verify"), 0);

    // A pairing keeps no space: the status shows no e-mail address, only that
    // a device is paired.
    let dir = TempDir::new("private");
    let files = dir.files();
    gui_bridge::save_settings(
        &files.settings,
        &GuiSettings {
            portal_url: Some(service.base_url()),
            device_label: Some("Bureau".to_string()),
            locale: None,
        },
    )
    .unwrap();
    let status = space::space_status(&files, true).unwrap();
    assert!(status.paired);
    assert_eq!(status.space, None);
    assert_eq!(status.ended, None);
}

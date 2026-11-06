//! A fake service that **replays the exchanges the contract records**
//! (`contracts/fixtures/api/*.json`): each one a request and
//! the answer the service gives to it. Astro Sync replays them against a
//! fake service.
//!
//! [`Replay`] is scripted with the ids of the exchanges to play, in order. For
//! each request Astro Sync sends it
//!
//! - answers with the recorded response, verbatim (status, `Retry-After`,
//!   body), so a refusal is exactly the contract's refusal;
//! - checks the request against the recorded one: the same method and path,
//!   a JSON `Content-Type` where the recorded one has it, the very
//!   `Authorization` where the recorded one has it and none where it has
//!   none -- and, since a recorded request is one example among many (the
//!   identifiers, the names and the codes vary), a body that satisfies the
//!   schema of what Astro Sync sends (`sign-in.schema.json`): only known
//!   fields, `device.application` = `sync`, a valid installation identifier,
//!   a six-digit code. What does not match is recorded as a *violation* the
//!   test asserts empty.
//!
//! The pings that wake a sleeping service (`GET /api/v1/service`) are
//! answered with an announcement of the hosted service unless the script
//! itself names that route next.
//!
//! Shared by path (`#[path = ".../contract_replay.rs"] mod contract_replay;`,
//! next to `fake_service`) by every test binary that needs it.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use super::fake_service::{FakeService, Recorded, Reply, Wake};

/// One recorded exchange.
#[derive(Debug, Clone)]
pub struct Exchange {
    pub id: String,
    /// The fixture file it comes from (`sign-in-verify.json`).
    pub file: String,
    /// `built` (the service answers it today) or `planned` (not yet).
    pub status: String,
    pub given: Value,
    pub method: String,
    pub path: String,
    pub request_headers: Vec<(String, String)>,
    pub request_body: Value,
    pub status_code: u16,
    pub response_headers: Vec<(String, String)>,
    pub response_body: Value,
}

/// The contract package: the copy beside this repository's crates
/// (`contracts/`).
pub fn contracts_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts")
}

pub fn fixtures_dir() -> PathBuf {
    contracts_root().join("fixtures/api")
}

fn headers_of(value: &Value) -> Vec<(String, String)> {
    value
        .as_object()
        .map(|map| {
            map.iter()
                // A header of several values (`set-cookie`) is a web session,
                // not something Astro Sync reads.
                .filter_map(|(name, value)| {
                    Some((name.to_ascii_lowercase(), value.as_str()?.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The answers of `GET /v1/entitlements` Astro Sync plays, reduced to what it
/// reads (`tests/fixtures/standing-answers.json`, in the format of the
/// contract's recorded files). They are Astro Sync's own, so that the contract
/// package it copies holds the answer of a private server alone. Resolved
/// from the manifest directory of whichever crate includes this file (the
/// core's tests and the application's both sit two levels under the root).
pub fn standing_answers_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/astro-sync-core/tests/fixtures/standing-answers.json")
}

fn exchanges_of(path: &Path) -> Vec<Exchange> {
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let document: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("{} is not JSON: {error}", path.display()));
    let file = path.file_name().unwrap().to_string_lossy().into_owned();
    let status = document["status"].as_str().unwrap_or_default().to_string();
    document["exchanges"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|exchange| Exchange {
            id: exchange["id"].as_str().unwrap().to_string(),
            file: file.clone(),
            status: status.clone(),
            given: exchange["given"].clone(),
            method: exchange["request"]["method"].as_str().unwrap().to_string(),
            path: exchange["request"]["path"].as_str().unwrap().to_string(),
            request_headers: headers_of(&exchange["request"]["headers"]),
            request_body: exchange["request"]["body"].clone(),
            status_code: u16::try_from(exchange["response"]["status"].as_u64().unwrap()).unwrap(),
            response_headers: headers_of(&exchange["response"]["headers"]),
            response_body: exchange["response"]["body"].clone(),
        })
        .collect()
}

/// Every exchange of every recorded file, in file then document order, then
/// the standing answers of Astro Sync's own.
pub fn all_exchanges() -> Vec<Exchange> {
    let dir = fixtures_dir();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("read {}: {error}", dir.display()))
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    files.sort();
    files.push(standing_answers_path());
    files.iter().flat_map(|path| exchanges_of(path)).collect()
}

pub fn exchange(id: &str) -> Exchange {
    all_exchanges()
        .into_iter()
        .find(|exchange| exchange.id == id)
        .unwrap_or_else(|| panic!("no recorded exchange {id:?}"))
}

/// What the service answers, exactly as recorded.
pub fn reply_of(exchange: &Exchange) -> Reply {
    let body = if exchange.response_body.is_null() {
        String::new()
    } else {
        exchange.response_body.to_string()
    };
    Reply::JsonWith(
        exchange.status_code,
        exchange.response_headers.clone(),
        body,
    )
}

/* ------------------------------ what Astro Sync sends --------------------- */

fn is_installation_id(value: &str) -> bool {
    (16..=128).contains(&value.len())
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        })
}

/// What is wrong with a `device` object Astro Sync sent
/// (`sign-in.schema.json` `$defs/device`), empty when nothing.
pub fn device_violations(device: &Value) -> Vec<String> {
    let mut problems = Vec::new();
    let Some(object) = device.as_object() else {
        return vec!["device is not an object".to_string()];
    };
    for key in object.keys() {
        if !["application", "installationId", "name", "platform", "model"].contains(&key.as_str()) {
            problems.push(format!("device.{key} is not a field of the contract"));
        }
    }
    if device["application"] != "sync" {
        problems.push(format!(
            "device.application must be \"sync\", got {}",
            device["application"]
        ));
    }
    if !device["installationId"]
        .as_str()
        .is_some_and(is_installation_id)
    {
        problems.push(format!(
            "device.installationId is not 16 to 128 characters of A-Z a-z 0-9 _ -: {}",
            device["installationId"]
        ));
    }
    match device["name"].as_str() {
        Some(name) if !name.is_empty() && name.chars().count() <= 120 => {}
        other => problems.push(format!(
            "device.name must be 1 to 120 characters, got {other:?}"
        )),
    }
    if !["ios", "android", "macos", "windows", "linux"]
        .iter()
        .any(|platform| device["platform"] == *platform)
    {
        problems.push(format!(
            "device.platform is not known: {}",
            device["platform"]
        ));
    }
    // (No `if let ... &&`: this file is shared with a crate of edition 2021.)
    if device.get("model").is_some_and(|model| {
        !model
            .as_str()
            .is_some_and(|model| !model.is_empty() && model.chars().count() <= 120)
    }) {
        problems.push("device.model must be 1 to 120 characters".to_string());
    }
    problems
}

fn is_normalised_email(value: &str) -> bool {
    value.matches('@').count() == 1
        && !value
            .chars()
            .any(|character| character.is_whitespace() || character.is_uppercase())
        && value.len() <= 254
        && value
            .split_once('@')
            .is_some_and(|(local, domain)| !local.is_empty() && domain.contains('.'))
}

/// What is wrong with a `POST /v1/sign-in/code` body Astro Sync sent.
pub fn code_request_violations(body: &Value) -> Vec<String> {
    let mut problems = Vec::new();
    let Some(object) = body.as_object() else {
        return vec!["the body is not a JSON object".to_string()];
    };
    for key in object.keys() {
        if !["email", "language", "device"].contains(&key.as_str()) {
            problems.push(format!("{key} is not a field of the request"));
        }
    }
    if !body["email"].as_str().is_some_and(is_normalised_email) {
        problems.push(format!(
            "email is not a normalised address: {}",
            body["email"]
        ));
    }
    if let Some(language) = body
        .get("language")
        .filter(|language| !["fr", "en", "ja"].iter().any(|known| *language == known))
    {
        problems.push(format!("language is not fr, en or ja: {language}"));
    }
    problems.extend(device_violations(&body["device"]));
    problems
}

/// What is wrong with a `POST /v1/sign-in/verify` body Astro Sync sent (with
/// a code, as the application: never a link, never `client: web`).
pub fn verify_request_violations(body: &Value) -> Vec<String> {
    let mut problems = Vec::new();
    let Some(object) = body.as_object() else {
        return vec!["the body is not a JSON object".to_string()];
    };
    for key in object.keys() {
        if !["email", "code", "client", "device"].contains(&key.as_str()) {
            problems.push(format!("{key} is not a field of the request"));
        }
    }
    if !body["email"].as_str().is_some_and(is_normalised_email) {
        problems.push(format!(
            "email is not a normalised address: {}",
            body["email"]
        ));
    }
    if !body["code"]
        .as_str()
        .is_some_and(|code| code.len() == 6 && code.chars().all(|digit| digit.is_ascii_digit()))
    {
        problems.push("code must be six digits".to_string());
    }
    if body["client"] != "app" {
        problems.push(format!("client must be \"app\", got {}", body["client"]));
    }
    problems.extend(device_violations(&body["device"]));
    problems
}

/// What is wrong with a `PATCH /v1/devices/{id}` body Astro Sync sent.
pub fn rename_request_violations(body: &Value) -> Vec<String> {
    let mut problems = Vec::new();
    match (body.as_object(), body["name"].as_str()) {
        (Some(object), Some(name))
            if object.len() == 1 && !name.is_empty() && name.chars().count() <= 120 => {}
        _ => problems.push(format!(
            "the body must be {{\"name\": 1 to 120 characters}}, got {body}"
        )),
    }
    problems
}

/// What is wrong with a request, compared to the exchange the script plays
/// next: `Err` when it is not even the same route (the script is then not
/// consumed), `Ok` with what else does not match.
fn violations_of_request(request: &Recorded, exchange: &Exchange) -> Result<Vec<String>, String> {
    let mut problems = Vec::new();
    let recorded_path = exchange.path.as_str();
    if request.method != exchange.method || request.path != recorded_path {
        return Err(format!(
            "{}: Astro Sync sent {} {} where the contract records {} {}",
            exchange.id, request.method, request.path, exchange.method, recorded_path
        ));
    }
    for (name, value) in &exchange.request_headers {
        match name.as_str() {
            "content-type" => {
                let sent = request.header("content-type").unwrap_or_default();
                if !sent
                    .to_ascii_lowercase()
                    .starts_with(&value.to_ascii_lowercase())
                {
                    problems.push(format!(
                        "{}: content-type is {sent:?}, not {value:?}",
                        exchange.id
                    ));
                }
            }
            "authorization" => {
                if request.header("authorization") != Some(value.as_str()) {
                    problems.push(format!(
                        "{}: the Authorization header is not the recorded one",
                        exchange.id
                    ));
                }
            }
            _ => {}
        }
    }
    let recorded_names: Vec<&str> = exchange
        .request_headers
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    // A `401` is also what a credential the service no longer knows is
    // answered ("no credential, or a revoked one"): the recorded request
    // carries none, the one Astro Sync sends may carry the revoked one.
    if !recorded_names.contains(&"authorization")
        && exchange.status_code != 401
        && request.header("authorization").is_some()
    {
        problems.push(format!(
            "{}: a credential was sent where the contract records none",
            exchange.id
        ));
    }
    let body = request.json();
    problems.extend(
        match (request.method.as_str(), request.path.as_str()) {
            ("POST", "/api/v1/sign-in/code") => code_request_violations(&body),
            ("POST", "/api/v1/sign-in/verify") => verify_request_violations(&body),
            ("PATCH", path) if path.starts_with("/api/v1/devices/") => {
                rename_request_violations(&body)
            }
            _ => Vec::new(),
        }
        .into_iter()
        .map(|problem| format!("{}: {problem}", exchange.id)),
    );
    Ok(problems)
}

/* ---------------------------------- the service --------------------------- */

const SERVICE_PATH: &str = "/api/v1/service";

/// The fake service. See the module documentation.
pub struct Replay {
    service: FakeService,
    script: Arc<Mutex<VecDeque<Exchange>>>,
    violations: Arc<Mutex<Vec<String>>>,
}

impl Replay {
    /// An awake service that plays `script` (exchange ids, in order) and
    /// answers the wake-up pings with the hosted service's announcement.
    pub fn start(script: &[&str]) -> Self {
        Self::start_with(script, Wake::Instant, "hosted-service-announces-email-code")
    }

    pub fn start_with(script: &[&str], wake: Wake, announcement: &str) -> Self {
        let queue: VecDeque<Exchange> = script.iter().map(|id| exchange(id)).collect();
        let script = Arc::new(Mutex::new(queue));
        let violations: Arc<Mutex<Vec<String>>> = Arc::default();
        let ping = exchange(announcement);
        let (app_script, app_violations) = (Arc::clone(&script), Arc::clone(&violations));
        let service = FakeService::start(wake, move |request| {
            let mut script = app_script.lock().unwrap();
            let ping_is_scripted = script
                .front()
                .is_some_and(|next| next.method == "GET" && next.path == SERVICE_PATH);
            if request.method == "GET" && request.path == SERVICE_PATH && !ping_is_scripted {
                return reply_of(&ping);
            }
            let Some(next) = script.pop_front() else {
                app_violations.lock().unwrap().push(format!(
                    "unscripted request: {} {}",
                    request.method, request.path
                ));
                return Reply::Json(
                    500,
                    r#"{"error":{"code":"unscripted","message":"nothing left to replay"}}"#
                        .to_string(),
                );
            };
            match violations_of_request(request, &next) {
                Ok(problems) => {
                    app_violations.lock().unwrap().extend(problems);
                    reply_of(&next)
                }
                Err(different_route) => {
                    app_violations.lock().unwrap().push(different_route);
                    Reply::Json(500, r#"{"error":{"code":"unexpected_route","message":"not the scripted exchange"}}"#.to_string())
                }
            }
        });
        Self {
            service,
            script,
            violations,
        }
    }

    pub fn base_url(&self) -> String {
        self.service.base_url()
    }

    pub fn service(&self) -> &FakeService {
        &self.service
    }

    /// What did not match the contract, so far.
    pub fn violations(&self) -> Vec<String> {
        self.violations.lock().unwrap().clone()
    }

    /// The exchanges not played yet.
    pub fn remaining(&self) -> Vec<String> {
        self.script
            .lock()
            .unwrap()
            .iter()
            .map(|exchange| exchange.id.clone())
            .collect()
    }

    /// Everything scripted was played, and nothing sent broke the contract.
    pub fn assert_played_exactly(&self) {
        assert_eq!(
            self.violations(),
            Vec::<String>::new(),
            "requests that break the contract"
        );
        assert_eq!(
            self.remaining(),
            Vec::<String>::new(),
            "exchanges never played"
        );
    }

    /// The requests that reached the application on this route.
    pub fn requests_to(&self, method: &str, path: &str) -> Vec<Recorded> {
        self.service
            .requests()
            .into_iter()
            .filter(|request| {
                request.reached_app && request.method == method && request.path == path
            })
            .collect()
    }
}

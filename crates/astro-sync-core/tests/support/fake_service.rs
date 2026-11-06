//! A throwaway HTTP service for the "the service is asleep" tests.
//!
//! It imitates a service that sleeps when nobody uses it: the *first*
//! request wakes it, the gateway in front keeps every connection open while
//! it starts (tens of seconds for a real one; a few hundred milliseconds
//! here), and once it is ready every answer is immediate. A request the
//! client abandons while it is held never reaches the application --
//! exactly like a request the gateway dropped -- so [`FakeService::app_hits`] counts what the *application*
//! received, which is what "never two creations for one action" is about.
//!
//! It is std-only, listens on a port the operating system picks, and stops
//! when dropped. This file is shared by path
//! (`#[path = ".../fake_service.rs"] mod fake_service;`) by every test
//! binary that needs it, in `astro-sync-core`, `astro-sync-cli` and the
//! Tauri crate.

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// How the fake gateway behaves until the service is "ready" (measured from
/// the first request it ever receives).
#[derive(Debug, Clone, Copy)]
pub enum Wake {
    /// Ready immediately: an awake service.
    Instant,
    /// The gateway holds every connection open for this long after the first
    /// request, then lets the request through.
    Held(Duration),
    /// The gateway answers `503` with a plain-text body (no application
    /// error envelope) for this long after the first request.
    GatewayUnavailable(Duration),
}

/// One request as the fake saw it.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    /// Header names lower-cased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// `false` for a request the fake gateway held and the client abandoned,
    /// or answered with its own gateway `503`.
    pub reached_app: bool,
    pub at: Instant,
}

impl Recorded {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or(serde_json::Value::Null)
    }
}

/// What the application does with a request that reached it.
#[derive(Debug, Clone)]
pub enum Reply {
    Json(u16, String),
    /// A JSON answer with headers of its own (`Retry-After`, ...): what the
    /// contract's recorded exchanges carry. An empty body is sent as such.
    JsonWith(u16, Vec<(String, String)>, String),
    Text(u16, String),
    /// Never answers; the connection stays open until the client leaves.
    Hang,
    /// The application did its work, then the connection is closed without
    /// an answer: the response is lost.
    DropAfterProcessing,
    /// Answers after a pause.
    After(Duration, Box<Reply>),
}

type App = dyn Fn(&Recorded) -> Reply + Send + Sync;

struct Shared {
    stop: AtomicBool,
    first_request: Mutex<Option<Instant>>,
    log: Mutex<Vec<Recorded>>,
    wake: Wake,
    app: Box<App>,
}

pub struct FakeService {
    addr: SocketAddr,
    shared: Arc<Shared>,
    accept: Option<JoinHandle<()>>,
}

impl FakeService {
    pub fn start(wake: Wake, app: impl Fn(&Recorded) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind an OS-assigned port");
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            first_request: Mutex::new(None),
            log: Mutex::new(Vec::new()),
            wake,
            app: Box::new(app),
        });
        let accept_shared = Arc::clone(&shared);
        let accept = thread::spawn(move || {
            while !accept_shared.stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let shared = Arc::clone(&accept_shared);
                        thread::spawn(move || serve(stream, shared));
                    }
                    Err(_) => thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        Self {
            addr,
            shared,
            accept: Some(accept),
        }
    }

    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// Every request seen, in arrival order (held-and-abandoned and gateway
    /// `503` ones included).
    pub fn requests(&self) -> Vec<Recorded> {
        self.shared.log.lock().unwrap().clone()
    }

    /// Requests that reached the application, filtered by method and path
    /// prefix.
    pub fn app_hits(&self, method: &str, path_prefix: &str) -> usize {
        self.requests()
            .iter()
            .filter(|request| {
                request.reached_app
                    && request.method == method
                    && request.path.starts_with(path_prefix)
            })
            .count()
    }

    /// Every request, reached the application or not.
    pub fn raw_hits(&self, method: &str, path_prefix: &str) -> usize {
        self.requests()
            .iter()
            .filter(|request| request.method == method && request.path.starts_with(path_prefix))
            .count()
    }
}

impl Drop for FakeService {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.accept.take() {
            let _ = handle.join();
        }
    }
}

fn is_timeout(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn read_request(stream: &mut TcpStream, shared: &Shared) -> Option<Recorded> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 16 * 1024];
    let head_end = loop {
        if let Some(position) = find(&buffer, b"\r\n\r\n") {
            break position + 4;
        }
        match stream.read(&mut chunk) {
            Ok(0) => return None,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
            Err(error) if is_timeout(&error) => {
                if shared.stop.load(Ordering::SeqCst) {
                    return None;
                }
            }
            Err(_) => return None,
        }
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next()?.split(' ');
    let method = request_line.next()?.to_string();
    let path = request_line.next()?.to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect();
    let content_length: usize = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    while buffer.len() < head_end + content_length {
        match stream.read(&mut chunk) {
            Ok(0) => return None,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
            Err(error) if is_timeout(&error) => {
                if shared.stop.load(Ordering::SeqCst) {
                    return None;
                }
            }
            Err(_) => return None,
        }
    }
    Some(Recorded {
        method,
        path,
        headers,
        body: buffer[head_end..head_end + content_length].to_vec(),
        reached_app: false,
        at: Instant::now(),
    })
}

fn client_gone(stream: &TcpStream) -> bool {
    let mut one = [0_u8; 1];
    match stream.peek(&mut one) {
        Ok(0) => true,
        Ok(_) => false,
        Err(error) => !is_timeout(&error),
    }
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Status",
    }
}

fn write_response(stream: &mut TcpStream, status: u16, content_type: &str, body: &str) -> bool {
    write_response_with(stream, status, content_type, &[], body)
}

fn write_response_with(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    headers: &[(String, String)],
    body: &str,
) -> bool {
    let extra: String = headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect();
    let response = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\n{extra}Content-Length: {}\r\nConnection: keep-alive\r\n\r\n{body}",
        status_text(status),
        body.len()
    );
    stream.write_all(response.as_bytes()).is_ok()
}

/// Returns `false` when the connection must be closed.
fn answer(stream: &mut TcpStream, reply: Reply, shared: &Shared) -> bool {
    match reply {
        Reply::Json(status, body) => write_response(stream, status, "application/json", &body),
        Reply::JsonWith(status, headers, body) => {
            write_response_with(stream, status, "application/json", &headers, &body)
        }
        Reply::Text(status, body) => write_response(stream, status, "text/plain", &body),
        Reply::DropAfterProcessing => false,
        Reply::Hang => {
            while !shared.stop.load(Ordering::SeqCst) && !client_gone(stream) {
                thread::sleep(Duration::from_millis(10));
            }
            false
        }
        Reply::After(pause, inner) => {
            let until = Instant::now() + pause;
            while Instant::now() < until {
                if shared.stop.load(Ordering::SeqCst) || client_gone(stream) {
                    return false;
                }
                thread::sleep(Duration::from_millis(10));
            }
            answer(stream, *inner, shared)
        }
    }
}

fn serve(mut stream: TcpStream, shared: Arc<Shared>) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(20)));
    let _ = stream.set_nodelay(true);
    loop {
        let Some(mut request) = read_request(&mut stream, &shared) else {
            return;
        };
        let now = Instant::now();
        let first = *shared.first_request.lock().unwrap().get_or_insert(now);
        match shared.wake {
            Wake::Instant => {}
            Wake::Held(delay) => {
                let ready = first + delay;
                while Instant::now() < ready {
                    if shared.stop.load(Ordering::SeqCst) {
                        return;
                    }
                    if client_gone(&stream) {
                        shared.log.lock().unwrap().push(request);
                        return;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
            }
            Wake::GatewayUnavailable(delay) => {
                if now < first + delay {
                    shared.log.lock().unwrap().push(request);
                    let sent = write_response(
                        &mut stream,
                        503,
                        "text/plain",
                        "upstream connect error or disconnect/reset before headers",
                    );
                    if !sent {
                        return;
                    }
                    continue;
                }
            }
        }
        request.reached_app = true;
        shared.log.lock().unwrap().push(request.clone());
        let reply = (shared.app)(&request);
        if !answer(&mut stream, reply, &shared) {
            return;
        }
    }
}

/* ------------------------- canned application bodies ---------------------- */

pub fn identity_reply() -> Reply {
    Reply::Json(
        200,
        r#"{"service":"astro-portal","apiVersion":1,"version":"0.0.0-test"}"#.to_string(),
    )
}

pub fn sessions_reply() -> Reply {
    Reply::Json(
        200,
        r#"{"sessions":[{"id":"s-1","planId":null,"status":"open","startedAt":"2026-09-29T20:00:00Z","finishedAt":null,"createdAt":"2026-09-29T20:00:00Z","updatedAt":"2026-09-29T20:00:00Z","eventCount":0,"supplementCount":0,"pendingSupplementCount":0}],"nextSince":null,"hasMore":false}"#
            .to_string(),
    )
}

pub fn capabilities_reply() -> Reply {
    Reply::Json(
        200,
        r#"{"planner.multiple":true,"planner.mosaic":true,"pointing.solve":true,"sync.plan_limit":1000,"gallery.private":true,"portal.invitations":true,"capture.variant_limit":12,"capture.count_limit":9007199254740991,"capture.public":true,"analysis.logs":true,"analysis.progression":true,"storage.max_file_bytes":104857600,"storage.quota_bytes":53687091200,"history.retention_days":730}"#
            .to_string(),
    )
}

pub fn error_reply(status: u16, code: &str, message: &str) -> Reply {
    Reply::Json(
        status,
        format!(r#"{{"error":{{"code":"{code}","message":"{message}"}}}}"#),
    )
}

/// A faithful, small model of `api/v1/uploads.md`: one upload session per
/// content hash, offset-guarded chunks (`409` with the server's real offset
/// on a mismatch), idempotent create and finalize.
#[derive(Default)]
pub struct UploadModel {
    state: Mutex<UploadState>,
}

#[derive(Default)]
struct UploadState {
    by_hash: HashMap<String, UploadSessionState>,
    creations: usize,
    captures: usize,
}

struct UploadSessionState {
    id: String,
    byte_size: u64,
    chunk_size: u64,
    data: Vec<u8>,
    capture_id: Option<String>,
}

impl UploadSessionState {
    fn envelope(&self) -> String {
        let capture = match &self.capture_id {
            Some(id) => format!(r#","captureId":"{id}""#),
            None => String::new(),
        };
        format!(
            r#"{{"uploadId":"{}","offset":{},"byteSize":{},"chunkSize":{},"completed":{}{}}}"#,
            self.id,
            self.data.len(),
            self.byte_size,
            self.chunk_size,
            self.capture_id.is_some(),
            capture
        )
    }
}

impl UploadModel {
    /// Distinct upload sessions ever created (the *effect* count).
    pub fn sessions_created(&self) -> usize {
        self.state.lock().unwrap().creations
    }

    pub fn captures_created(&self) -> usize {
        self.state.lock().unwrap().captures
    }

    /// The bytes the server holds for a hash.
    pub fn bytes_for(&self, sha256: &str) -> Option<Vec<u8>> {
        self.state
            .lock()
            .unwrap()
            .by_hash
            .get(sha256)
            .map(|session| session.data.clone())
    }

    /// `None` for a request that is not an upload route.
    pub fn reply(&self, request: &Recorded, chunk_size: u64) -> Option<Reply> {
        use sha2::{Digest, Sha256};
        let mut state = self.state.lock().unwrap();
        if request.method == "POST" && request.path == "/api/v1/uploads" {
            let body = request.json();
            let sha256 = body["sha256"].as_str().unwrap_or_default().to_string();
            let byte_size = body["byteSize"].as_u64().unwrap_or(0);
            if let Some(existing) = state.by_hash.get(&sha256) {
                return Some(Reply::Json(200, existing.envelope()));
            }
            state.creations += 1;
            let id = format!("upload-{}", state.creations);
            let session = UploadSessionState {
                id,
                byte_size,
                chunk_size,
                data: Vec::new(),
                capture_id: None,
            };
            let envelope = session.envelope();
            state.by_hash.insert(sha256, session);
            return Some(Reply::Json(201, envelope));
        }
        let id = request.path.strip_prefix("/api/v1/uploads/")?.to_string();
        let (id, finalize) = match id.strip_suffix("/finalize") {
            Some(id) => (id.to_string(), true),
            None => (id, false),
        };
        let Some((sha256, session)) = state
            .by_hash
            .iter_mut()
            .find(|(_, session)| session.id == id)
            .map(|(sha, session)| (sha.clone(), session))
        else {
            return Some(error_reply(404, "upload_not_found", "unknown upload"));
        };
        match (request.method.as_str(), finalize) {
            ("GET", false) => Some(Reply::Json(200, session.envelope())),
            ("PUT", false) => {
                if session.capture_id.is_some() {
                    return Some(Reply::Json(200, session.envelope()));
                }
                let declared: u64 = request
                    .header("upload-offset")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(u64::MAX);
                if declared != session.data.len() as u64 {
                    return Some(Reply::Json(
                        409,
                        format!(
                            r#"{{"error":{{"code":"upload_offset_mismatch","message":"offset"}},"offset":{}}}"#,
                            session.data.len()
                        ),
                    ));
                }
                session.data.extend_from_slice(&request.body);
                Some(Reply::Json(200, session.envelope()))
            }
            ("POST", true) => {
                if session.data.len() as u64 != session.byte_size {
                    return Some(Reply::Json(
                        409,
                        format!(
                            r#"{{"error":{{"code":"upload_incomplete","message":"incomplete"}},"offset":{}}}"#,
                            session.data.len()
                        ),
                    ));
                }
                let digest = format!("{:x}", Sha256::digest(&session.data));
                if digest != sha256 {
                    session.data.clear();
                    return Some(Reply::Json(
                        422,
                        r#"{"error":{"code":"upload_hash_mismatch","message":"hash"},"offset":0}"#
                            .to_string(),
                    ));
                }
                let newly_captured = session.capture_id.is_none();
                let capture_id = format!("capture-of-{}", session.id);
                session.capture_id.get_or_insert(capture_id);
                let envelope = session.envelope();
                let body = format!(
                    r#"{},"capture":{{}},"deduplicated":false}}"#,
                    &envelope[..envelope.len() - 1]
                );
                if newly_captured {
                    state.captures += 1;
                }
                Some(Reply::Json(200, body))
            }
            _ => Some(error_reply(404, "not_found", "no such route")),
        }
    }
}

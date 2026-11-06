//! Patience when the hosted service is asleep.
//!
//! A service can sleep when nobody uses it. After such a sleep the *first*
//! request is answered late -- the service starts and the gateway in front
//! of it keeps the connection open meanwhile -- and everything after answers
//! in under a second. Astro Sync is the patient party: it says the service is starting and waits, instead
//! of saying the service does not answer.
//!
//! [`PatientHttp`] is the one HTTP client both transports
//! ([`crate::uploader::HttpUploadTransport`],
//! [`crate::gui_bridge::HttpPortalGuiTransport`]) send through. It tells
//! three situations apart, because they call for three different behaviors:
//!
//! - **The service is starting**: a request that gets no answer within its
//!   time limit (`NoAnswer`), or is answered by the gateway itself with
//!   `502`/`503`/`504` and no application error body. The wait is announced
//!   ([`ServiceEvent::Starting`]) and the request is sent again, until an
//!   answer arrives, within [`WakePolicy::total_budget`] -- never forever.
//! - **The service is unreachable**: name not found, connection refused, a
//!   TLS failure, no network at all. The error is immediate and nothing is
//!   waited on, as before.
//! - **The application answered**: any other status, including a `503` that
//!   carries the application's own error envelope (`{"error":{"code":..}}`,
//!   e.g. `authentication_unavailable`). That is a real answer; retrying it
//!   would only delay a real refusal.
//!
//! Sending a request again is only safe when a second copy is harmless, so
//! the two ways to send are separate on purpose:
//!
//! - [`PatientHttp::send_safe`] retries. It is for reads, and for the
//!   writes whose contract makes a repeat harmless (the resumable uploads:
//!   idempotent create and finalize, offset-guarded chunks).
//! - [`PatientHttp::wake`] then [`PatientHttp::send_once`] is for every other
//!   write (the single-use pairing code, the version-creating analysis
//!   upload). The service is first woken by a read that has no effect (the
//!   public identity route, [`SERVICE_IDENTITY_PATH`]), and the write is
//!   then sent exactly once. If it gets no confirmation the caller is told
//!   plainly that it may have gone through ([`ServiceWaitError::NotConfirmed`]);
//!   it is never sent a second time behind the person's back.

use reqwest::blocking::{Client, RequestBuilder};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use thiserror::Error;

/// The public, credential-free identity route (`GET /v1/service`: "no
/// credential, no database"), used to wake the service before a write that
/// must be sent only once. Any answer that is not the gateway's own
/// gateway error proves the service is up.
pub const SERVICE_IDENTITY_PATH: &str = "/api/v1/service";

/// How often a waiting call looks at the cancel flag and the clock.
const POLL: Duration = Duration::from_millis(50);

/// A service that answered this recently is awake: a later silence is a slow
/// answer, not a sleeping service, and is not announced as "starting".
const AWAKE_WINDOW: Duration = Duration::from_secs(60);

/// Within this window a write need not wake the service again first.
const WAKE_SKIP_WINDOW: Duration = Duration::from_secs(30);

/// How long each kind of call may wait. The default values
/// ([`WakePolicy::default`]) outlast a slow start with room to spare; tests
/// use the same shape on a scale of hundreds of milliseconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakePolicy {
    /// The longest a call keeps waiting for a service that is starting,
    /// counted from its first attempt, before it gives up
    /// ([`ServiceWaitError::StartTimeout`]). Never less than the call's own
    /// time limit.
    pub total_budget: Duration,
    /// One attempt of a small call (reads, create, finalize): longer than
    /// a slow start, so the very first request is normally the one
    /// that is answered.
    pub attempt_timeout: Duration,
    /// The time limit of a small write sent once (the pairing claim).
    pub write_timeout: Duration,
    /// The time limit of a call that carries a large body (a file chunk, an
    /// analysis report), which needs time to travel on a slow link.
    pub chunk_timeout: Duration,
    /// The pause between two attempts after the gateway answered
    /// `502`/`503`/`504`.
    pub retry_pause: Duration,
    /// Silence longer than this from a service not known to be awake shows
    /// the "starting" state; a normal answer never gets there.
    pub notice_after: Duration,
    /// How long to wait for a connection to be established. The gateway
    /// accepts connections while the service starts, so a connection that
    /// cannot be made is not a sleeping service.
    pub connect_timeout: Duration,
}

impl Default for WakePolicy {
    fn default() -> Self {
        Self {
            total_budget: Duration::from_secs(100),
            attempt_timeout: Duration::from_secs(60),
            write_timeout: Duration::from_secs(30),
            chunk_timeout: Duration::from_secs(120),
            retry_pause: Duration::from_secs(2),
            notice_after: Duration::from_secs(4),
            connect_timeout: Duration::from_secs(15),
        }
    }
}

impl WakePolicy {
    /// For a refresh nobody is waiting on and that has a fallback (the
    /// cached capability profile): one short attempt, never announced. The
    /// request still reaches the service, which starts waking; the caller
    /// simply does not hold up the application for it.
    #[must_use]
    pub fn background_refresh() -> Self {
        let short = Duration::from_secs(5);
        Self {
            total_budget: short,
            attempt_timeout: short,
            write_timeout: short,
            chunk_timeout: short,
            retry_pause: Duration::from_secs(1),
            notice_after: Duration::from_secs(3600),
            connect_timeout: Duration::from_secs(5),
        }
    }
}

/// What a caller can watch while a call waits for the service. Serialized
/// (`{"kind":"starting"}`) as the payload of the Tauri event the interface
/// listens to. Events are balanced: `Starting` is announced at most once per
/// call, and only a call that announced it ends with `Ready` or `Stopped`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ServiceEvent {
    /// The service has not answered yet and is starting: "The service is
    /// starting. It can take a minute." (shown in the person's language)
    Starting,
    /// The wait is over: the service answered.
    Ready,
    /// The wait is over without an answer (gave up, or was cancelled).
    Stopped,
}

/// The switch a person uses to cancel every wait in progress ("Annuler" in
/// the interface). Cancelling applies to the calls waiting *now*: a token
/// taken afterwards is unaffected, so a cancel can never leak into the next
/// thing the person starts.
#[derive(Debug, Clone, Default)]
pub struct WaitControl {
    generation: Arc<AtomicU64>,
}

impl WaitControl {
    /// A token for one call: cancelled by any [`WaitControl::cancel_all`]
    /// that happens after it was taken.
    #[must_use]
    pub fn token(&self) -> CancelToken {
        CancelToken {
            generation: Some(Arc::clone(&self.generation)),
            seen: self.generation.load(Ordering::SeqCst),
        }
    }

    /// Cancels every wait in progress.
    pub fn cancel_all(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
}

/// See [`WaitControl::token`].
#[derive(Debug, Clone)]
pub struct CancelToken {
    generation: Option<Arc<AtomicU64>>,
    seen: u64,
}

impl CancelToken {
    /// A token that is never cancelled (the command line: Ctrl-C ends the
    /// process instead).
    #[must_use]
    pub fn never() -> Self {
        Self {
            generation: None,
            seen: 0,
        }
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.generation
            .as_ref()
            .is_some_and(|generation| generation.load(Ordering::SeqCst) != self.seen)
    }
}

type EventSink = Arc<dyn Fn(ServiceEvent) + Send + Sync>;

/// What a caller hands to a transport: where to report the waiting state,
/// and how to cancel it.
#[derive(Clone)]
pub struct WaitHooks {
    token: CancelToken,
    on_event: EventSink,
}

impl WaitHooks {
    /// No reporting, no cancelling.
    #[must_use]
    pub fn silent() -> Self {
        Self {
            token: CancelToken::never(),
            on_event: Arc::new(|_| {}),
        }
    }

    pub fn new(
        token: CancelToken,
        on_event: impl Fn(ServiceEvent) + Send + Sync + 'static,
    ) -> Self {
        Self {
            token,
            on_event: Arc::new(on_event),
        }
    }

    /// Reporting only, no cancelling.
    pub fn with_events(on_event: impl Fn(ServiceEvent) + Send + Sync + 'static) -> Self {
        Self::new(CancelToken::never(), on_event)
    }

    fn emit(&self, event: ServiceEvent) {
        (self.on_event)(event);
    }

    fn cancelled(&self) -> bool {
        self.token.is_cancelled()
    }
}

/// How a wait for the service ended when it did not end with an answer.
/// Each has a stable [`code`](ServiceWaitError::code) the interface turns
/// into a plain sentence in the person's language; the `Display` text below
/// is the same sentence in English for the command line.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ServiceWaitError {
    /// The service was still starting when the total budget ran out.
    #[error(
        "The service had not finished starting after {waited_secs} seconds. Please wait a moment, then try again."
    )]
    StartTimeout { waited_secs: u64 },
    /// The person cancelled the wait.
    #[error("Waiting was cancelled. You can try again whenever you like.")]
    Cancelled,
    /// A write was sent once (never resent) and no confirmation came back:
    /// it may or may not have been recorded.
    #[error(
        "The service did not confirm that it received your request. It may have gone through, so please check before trying again."
    )]
    NotConfirmed,
}

impl ServiceWaitError {
    /// The stable code an interface looks up as `errors.<code>`.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::StartTimeout { .. } => "service_start_timeout",
            Self::Cancelled => "cancelled",
            Self::NotConfirmed => "service_write_unconfirmed",
        }
    }
}

/// Why a call ended without an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpFailure {
    /// The service cannot be reached at all; the text is the technical
    /// detail, for logs -- never for a person.
    Unreachable(String),
    /// See [`ServiceWaitError`].
    Wait(ServiceWaitError),
}

/// A complete answer: status and body, read in full inside the attempt so
/// the time limit covers the body too.
#[derive(Debug, Clone)]
pub struct HttpAnswer {
    pub status: u16,
    pub body: Vec<u8>,
    /// The `Retry-After` header, when the service sent one as a number of
    /// seconds (the only form the contract uses); an HTTP date is ignored.
    pub retry_after_seconds: Option<u64>,
}

impl HttpAnswer {
    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    #[must_use]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Builds one copy of a request. Called once per attempt, so it must own
/// what it needs (an abandoned attempt can outlive the caller's borrow).
pub type RequestFactory = Arc<dyn Fn(&Client) -> RequestBuilder + Send + Sync>;

/// `502`/`503`/`504` answered by the gateway itself while the service
/// starts, as opposed to by the application: the application's own errors
/// always carry the `{"error":{"code":..}}` envelope, the gateway's never
/// do.
fn is_gateway_page(answer: &HttpAnswer) -> bool {
    matches!(answer.status, 502..=504) && !carries_error_envelope(&answer.body)
}

fn carries_error_envelope(body: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("error")?.get("code")?.as_str().map(|_| ()))
        .is_some()
}

enum AttemptFailure {
    /// A connection could not be made: name not found, refused, TLS, no
    /// network, or the connection attempt itself timed out.
    Connect(String),
    /// The request was sent and nothing came back in time.
    NoAnswer(String),
    /// The connection broke after it was made (reset, closed early).
    Interrupted,
    /// Anything else (a malformed request, a redirect loop): not a wait.
    Other(String),
}

fn describe(error: &reqwest::Error) -> String {
    let mut text = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(inner) = source {
        text.push_str(": ");
        text.push_str(&inner.to_string());
        source = inner.source();
    }
    text
}

fn classify(error: &reqwest::Error) -> AttemptFailure {
    let detail = describe(error);
    // A connect timeout is both `is_connect` and `is_timeout`: it is a
    // connection that could not be made, not a request left unanswered.
    if error.is_connect() {
        AttemptFailure::Connect(detail)
    } else if error.is_timeout() {
        AttemptFailure::NoAnswer(detail)
    } else if error.is_request() || error.is_body() {
        AttemptFailure::Interrupted
    } else {
        AttemptFailure::Other(detail)
    }
}

fn attempt_blocking(
    client: &Client,
    build: &dyn Fn(&Client) -> RequestBuilder,
    timeout: Duration,
) -> Result<HttpAnswer, AttemptFailure> {
    let response = build(client)
        .timeout(timeout)
        .send()
        .map_err(|error| classify(&error))?;
    let status = response.status().as_u16();
    let retry_after_seconds = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok());
    let body = response.bytes().map_err(|error| classify(&error))?.to_vec();
    Ok(HttpAnswer {
        status,
        body,
        retry_after_seconds,
    })
}

/// Announces the "starting" state at most once per call.
#[derive(Default)]
struct Notice {
    shown: bool,
}

impl Notice {
    fn show(&mut self, hooks: &WaitHooks) {
        if !self.shown {
            self.shown = true;
            hooks.emit(ServiceEvent::Starting);
        }
    }
}

/// The patient HTTP client. See the module documentation.
pub struct PatientHttp {
    client: Client,
    policy: WakePolicy,
    hooks: WaitHooks,
    answered: Mutex<Option<Instant>>,
}

impl PatientHttp {
    /// The `Err` text is the technical reason the client could not be built.
    pub fn new(policy: WakePolicy, hooks: WaitHooks) -> Result<Self, String> {
        // No client-wide time limit: every request states its own, and a
        // long-lived transfer must not inherit a small-call limit.
        // This client only ever talks to the portal, so it states what the
        // application is on every request (see `client_identity`).
        let client = Client::builder()
            .connect_timeout(policy.connect_timeout)
            .default_headers(crate::client_identity::headers())
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            client,
            policy,
            hooks,
            answered: Mutex::new(None),
        })
    }

    #[must_use]
    pub fn policy(&self) -> &WakePolicy {
        &self.policy
    }

    fn mark_answered(&self) {
        *self.answered.lock().unwrap() = Some(Instant::now());
    }

    fn answered_within(&self, window: Duration) -> bool {
        self.answered
            .lock()
            .unwrap()
            .is_some_and(|at| at.elapsed() < window)
    }

    /// Sends a request that is safe to send more than once, retrying while
    /// the service is starting (see the module documentation), and returns
    /// the first real answer -- of any status. `timeout` is one attempt's
    /// limit.
    pub fn send_safe(
        &self,
        timeout: Duration,
        build: &RequestFactory,
    ) -> Result<HttpAnswer, HttpFailure> {
        let started = Instant::now();
        // A call with a long limit (a big chunk) is never cut short by the
        // starting budget.
        let limit = self.policy.total_budget.max(timeout);
        let mut notice = Notice::default();
        let result = self.retry_while_starting(timeout, build, started, limit, &mut notice);
        if notice.shown {
            self.hooks.emit(if result.is_ok() {
                ServiceEvent::Ready
            } else {
                ServiceEvent::Stopped
            });
        }
        result
    }

    fn retry_while_starting(
        &self,
        timeout: Duration,
        build: &RequestFactory,
        started: Instant,
        limit: Duration,
        notice: &mut Notice,
    ) -> Result<HttpAnswer, HttpFailure> {
        loop {
            if self.hooks.cancelled() {
                return Err(HttpFailure::Wait(ServiceWaitError::Cancelled));
            }
            let elapsed = started.elapsed();
            if elapsed >= limit {
                return Err(HttpFailure::Wait(ServiceWaitError::StartTimeout {
                    waited_secs: elapsed.as_secs(),
                }));
            }
            let attempt_timeout = timeout.min(limit - elapsed);
            match self.attempt_watching(build, attempt_timeout, started, notice)? {
                Ok(answer) if is_gateway_page(&answer) => {
                    notice.show(&self.hooks);
                    self.pause(started, limit)?;
                }
                Ok(answer) => {
                    self.mark_answered();
                    return Ok(answer);
                }
                Err(AttemptFailure::Connect(detail) | AttemptFailure::Other(detail)) => {
                    return Err(HttpFailure::Unreachable(detail));
                }
                Err(AttemptFailure::NoAnswer(detail)) => {
                    if self.answered_within(AWAKE_WINDOW) {
                        // The service answered a moment ago: this is a slow
                        // or stuck answer, not a sleeping service. Same as
                        // before this module existed.
                        return Err(HttpFailure::Unreachable(detail));
                    }
                    notice.show(&self.hooks);
                }
                Err(AttemptFailure::Interrupted) => {
                    if !self.answered_within(AWAKE_WINDOW) {
                        notice.show(&self.hooks);
                    }
                    self.pause(started, limit)?;
                }
            }
        }
    }

    /// One attempt, run on its own thread so the caller keeps watching the
    /// clock and the cancel flag: the attempt in flight is abandoned when
    /// the person cancels (it ends by itself at its own time limit, and only
    /// ever carries a request that is safe to have sent).
    fn attempt_watching(
        &self,
        build: &RequestFactory,
        timeout: Duration,
        started: Instant,
        notice: &mut Notice,
    ) -> Result<Result<HttpAnswer, AttemptFailure>, HttpFailure> {
        let (sender, receiver) = mpsc::channel();
        let client = self.client.clone();
        let build = Arc::clone(build);
        thread::Builder::new()
            .name("astro-sync-request".to_string())
            .spawn(move || {
                let _ = sender.send(attempt_blocking(&client, &*build, timeout));
            })
            .map_err(|error| HttpFailure::Unreachable(error.to_string()))?;
        loop {
            match receiver.recv_timeout(POLL) {
                Ok(outcome) => return Ok(outcome),
                Err(RecvTimeoutError::Timeout) => {
                    if self.hooks.cancelled() {
                        return Err(HttpFailure::Wait(ServiceWaitError::Cancelled));
                    }
                    if started.elapsed() >= self.policy.notice_after
                        && !self.answered_within(AWAKE_WINDOW)
                    {
                        notice.show(&self.hooks);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Ok(Err(AttemptFailure::Interrupted));
                }
            }
        }
    }

    /// Waits [`WakePolicy::retry_pause`], noticing a cancel within one poll.
    fn pause(&self, started: Instant, limit: Duration) -> Result<(), HttpFailure> {
        self.pause_for(self.policy.retry_pause, started, limit)
    }

    /// Waits `duration` (never past `limit`, counted from `started`),
    /// noticing a cancel within one poll.
    fn pause_for(
        &self,
        duration: Duration,
        started: Instant,
        limit: Duration,
    ) -> Result<(), HttpFailure> {
        let until = Instant::now() + duration;
        while Instant::now() < until && started.elapsed() < limit {
            if self.hooks.cancelled() {
                return Err(HttpFailure::Wait(ServiceWaitError::Cancelled));
            }
            thread::sleep(POLL.min(until.saturating_duration_since(Instant::now())));
        }
        Ok(())
    }

    /// Wakes the service with a read that has no effect (the public
    /// identity route), waiting for it as [`PatientHttp::send_safe`] does.
    /// Skipped when the service answered a moment ago. To be called before
    /// every [`PatientHttp::send_once`].
    pub fn wake(&self, base_url: &str) -> Result<(), HttpFailure> {
        if self.answered_within(WAKE_SKIP_WINDOW) {
            return Ok(());
        }
        let url = format!(
            "{}{}",
            base_url.trim_end_matches('/'),
            SERVICE_IDENTITY_PATH
        );
        let build: RequestFactory = Arc::new(move |client| client.get(url.clone()));
        self.send_safe(self.policy.attempt_timeout, &build)
            .map(|_| ())
    }

    /// Sends a write exactly once -- never again, whatever happens -- and
    /// returns the answer, of any status. Only the gateway's own gateway
    /// errors, a request left unanswered and a connection that broke after
    /// it was made are reported as [`ServiceWaitError::NotConfirmed`], since
    /// the write may have been recorded; a connection that could not be
    /// made at all is [`HttpFailure::Unreachable`], since nothing was sent.
    ///
    /// The caller must have woken the service first ([`PatientHttp::wake`]):
    /// this call does not wait for a starting service and cannot be
    /// cancelled midway, because abandoning a write in flight would not
    /// stop it from taking effect.
    pub fn send_once(
        &self,
        timeout: Duration,
        build: &(dyn Fn(&Client) -> RequestBuilder + Sync),
    ) -> Result<HttpAnswer, HttpFailure> {
        // A plain thread rather than the caller's own: the blocking client
        // must never run inside an asynchronous runtime's worker.
        let outcome = thread::scope(|scope| {
            scope
                .spawn(|| attempt_blocking(&self.client, build, timeout))
                .join()
        })
        .unwrap_or(Err(AttemptFailure::Interrupted));
        match outcome {
            Ok(answer) if is_gateway_page(&answer) => {
                Err(HttpFailure::Wait(ServiceWaitError::NotConfirmed))
            }
            Ok(answer) => {
                self.mark_answered();
                Ok(answer)
            }
            Err(AttemptFailure::Connect(detail) | AttemptFailure::Other(detail)) => {
                Err(HttpFailure::Unreachable(detail))
            }
            Err(AttemptFailure::NoAnswer(_) | AttemptFailure::Interrupted) => {
                Err(HttpFailure::Wait(ServiceWaitError::NotConfirmed))
            }
        }
    }

    /// [`PatientHttp::wake`] then [`PatientHttp::send_once`], for a write the
    /// *application itself* may answer "I am not ready yet, nothing was
    /// done" (the `503` `service_starting` of the sign-in routes: the
    /// service is up but cannot reach its database yet). `not_ready` says
    /// which answer that is. Such an answer is a promise that nothing was
    /// checked, sent or used, so the write is sent again after the pause the
    /// service asked for (`Retry-After`, never longer than five times the
    /// policy's own pause), within [`WakePolicy::total_budget`]; the wait is
    /// announced like any other and can be cancelled. Every other answer,
    /// including a refusal, is returned as it is: a write that got an answer
    /// is never sent twice.
    pub fn send_once_while_starting(
        &self,
        base_url: &str,
        timeout: Duration,
        build: &(dyn Fn(&Client) -> RequestBuilder + Sync),
        not_ready: &dyn Fn(&HttpAnswer) -> bool,
    ) -> Result<HttpAnswer, HttpFailure> {
        let started = Instant::now();
        let limit = self.policy.total_budget.max(timeout);
        let mut notice = Notice::default();
        let result = self.send_once_until_ready(
            base_url,
            timeout,
            build,
            not_ready,
            started,
            limit,
            &mut notice,
        );
        if notice.shown {
            self.hooks.emit(if result.is_ok() {
                ServiceEvent::Ready
            } else {
                ServiceEvent::Stopped
            });
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn send_once_until_ready(
        &self,
        base_url: &str,
        timeout: Duration,
        build: &(dyn Fn(&Client) -> RequestBuilder + Sync),
        not_ready: &dyn Fn(&HttpAnswer) -> bool,
        started: Instant,
        limit: Duration,
        notice: &mut Notice,
    ) -> Result<HttpAnswer, HttpFailure> {
        loop {
            self.wake(base_url)?;
            if self.hooks.cancelled() {
                return Err(HttpFailure::Wait(ServiceWaitError::Cancelled));
            }
            let answer = self.send_once(timeout, build)?;
            if !not_ready(&answer) {
                return Ok(answer);
            }
            notice.show(&self.hooks);
            if started.elapsed() >= limit {
                return Err(HttpFailure::Wait(ServiceWaitError::StartTimeout {
                    waited_secs: started.elapsed().as_secs(),
                }));
            }
            let asked = answer
                .retry_after_seconds
                .map_or(self.policy.retry_pause, Duration::from_secs);
            let pause = asked
                .min(self.policy.retry_pause * 5)
                .max(self.policy.retry_pause);
            self.pause_for(pause, started, limit)?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(status: u16, body: &str) -> HttpAnswer {
        HttpAnswer {
            status,
            body: body.as_bytes().to_vec(),
            retry_after_seconds: None,
        }
    }

    #[test]
    fn the_gateway_errors_are_told_from_the_applications_own() {
        assert!(is_gateway_page(&answer(503, "upstream connect error")));
        assert!(is_gateway_page(&answer(502, "")));
        assert!(is_gateway_page(&answer(
            504,
            "<html>Gateway Timeout</html>"
        )));
        assert!(is_gateway_page(&answer(503, r#"{"unrelated":true}"#)));

        let envelope = r#"{"error":{"code":"authentication_unavailable","message":"x"}}"#;
        assert!(!is_gateway_page(&answer(503, envelope)));
        assert!(!is_gateway_page(&answer(500, "boom")));
        assert!(!is_gateway_page(&answer(404, "")));
        assert!(!is_gateway_page(&answer(200, "")));
    }

    #[test]
    fn the_default_policy_outlasts_a_slow_start() {
        let policy = WakePolicy::default();
        let slow_start = Duration::from_secs(41);
        assert!(policy.total_budget >= Duration::from_secs(90));
        assert!(
            policy.attempt_timeout > slow_start,
            "the first request must be able to be the one that is answered"
        );
        assert!(policy.notice_after < Duration::from_secs(10));
    }

    #[test]
    fn a_token_is_cancelled_only_by_a_cancel_that_comes_after_it() {
        let control = WaitControl::default();
        let before = control.token();
        assert!(!before.is_cancelled());
        control.cancel_all();
        assert!(before.is_cancelled());
        assert!(!control.token().is_cancelled());
        assert!(!CancelToken::never().is_cancelled());
    }
}

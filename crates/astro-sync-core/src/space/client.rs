//! The HTTP side of the hosted sign-in: what the service announces, the
//! request and the verification of a code, the space of a token, the rename
//! and the detachment of a device.
//!
//! Which call may be sent again while the service wakes is decided on what
//! the contract says about repeating it, as everywhere in this crate
//! ([`crate::service_wait`]):
//!
//! - `GET /v1/service`, `GET /v1/space`: reads -- retried freely.
//! - `PATCH`/`DELETE /v1/devices/{id}`: naming the same name twice, or
//!   detaching a device that is already detached (answered `device_detached`,
//!   which is read as done), changes nothing more -- retried.
//! - `POST /v1/sign-in/code`, `POST /v1/sign-in/verify`: **never sent
//!   twice**. A second message is a second e-mail (and a `429` within 60
//!   seconds); a code serves once, so a verification that never received an
//!   answer "is not replayed blindly: the code may have been used"
//!   (`v1.md`). The service is woken first (`GET /v1/service`, until it
//!   answers), then the write is sent once. The one exception is the
//!   application's own `503` `service_starting` -- "nothing was checked, sent
//!   or used" -- after which the same write is sent again once the service
//!   says it is ready.

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::{
    DeviceIdentity, Refusal, SYNC_APPLICATION, Secret, SignInEnd, SpaceError, current_platform,
    message_language, normalize_code, normalize_device_name, normalize_email,
};
use crate::service_wait::{
    HttpAnswer, PatientHttp, RequestFactory, SERVICE_IDENTITY_PATH, WaitHooks, WakePolicy,
};

/// The API versions this Astro Sync speaks.
const SUPPORTED_API_VERSIONS: [u64; 1] = [1];

/// What `GET /v1/service` says about how a person signs in
/// (`service.schema.json` `$defs/signIn`).
///
/// Serialized as the interface reads it: `{ "method": "email-code", ... }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "method",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum SignInAnnouncement {
    /// An e-mail address, then a code of six digits: the hosted service, or a
    /// private server signing in by e-mail code.
    EmailCode {
        code_length: u8,
        /// Whether an address without a space gets one (the hosted service)
        /// or only the owner's address opens the portal (a private server).
        space_creation: SpaceCreation,
        /// The terms of use, once the service has them configured.
        terms_url: Option<String>,
        /// The privacy policy, once the service has it configured.
        privacy_url: Option<String>,
        /// Additive: the address template (`https://.../{lang}/terms/`)
        /// behind `terms_url`, when the service names a page per language.
        /// The interface replaces `{lang}` with the tag of the language it
        /// is shown in (`legalAddressFor`); `terms_url` is the English page.
        terms_url_template: Option<String>,
        /// The same, for the privacy policy.
        privacy_url_template: Option<String>,
    },
    /// A private server: the code the server shows. Also what an absent
    /// announcement means (a private server older than the field).
    Pairing,
    /// A service whose private API is switched off: nothing to attach to.
    None,
    /// A method newer than this Astro Sync: the person is told to update.
    Unsupported { announced: String },
}

/// Who may sign in by e-mail code (`service.schema.json` `$defs/signIn`,
/// `spaceCreation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SpaceCreation {
    /// The hosted service: an address without a space gets one.
    Open,
    /// A private server: one space, its owner's; any other address is
    /// refused (`not_owner`). The words of a portal of one person, never the
    /// creation of a space. `closed` is additive to the contract: an absent or
    /// unknown value reads as [`SpaceCreation::Open`].
    Closed,
}

impl SignInAnnouncement {
    /// Reads the body of `GET /v1/service`: the marker, an API version this
    /// client speaks, then `signIn`. An absent `signIn` is a private server
    /// older than the field, read as [`SignInAnnouncement::Pairing`]; fields
    /// this client does not know are ignored, as the contract asks.
    pub fn parse(body: &serde_json::Value) -> Result<Self, SpaceError> {
        let payload = body.as_object().ok_or(SpaceError::NotAnAstroPortal)?;
        let version_named = payload
            .get("version")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|version| !version.is_empty());
        let api_version = payload
            .get("apiVersion")
            .and_then(serde_json::Value::as_u64)
            .filter(|version| *version >= 1);
        if payload.get("service").and_then(serde_json::Value::as_str) != Some("astro-portal")
            || api_version.is_none()
            || !version_named
        {
            return Err(SpaceError::NotAnAstroPortal);
        }
        if !api_version.is_some_and(|version| SUPPORTED_API_VERSIONS.contains(&version)) {
            return Err(SpaceError::IncompatibleApiVersion);
        }
        let Some(sign_in) = payload.get("signIn") else {
            return Ok(Self::Pairing);
        };
        let method = sign_in
            .get("method")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        Ok(match method {
            "email-code" => Self::EmailCode {
                code_length: u8::try_from(super::CODE_LENGTH).expect("six fits in a byte"),
                space_creation: match sign_in
                    .get("spaceCreation")
                    .and_then(serde_json::Value::as_str)
                {
                    Some("closed") => SpaceCreation::Closed,
                    _ => SpaceCreation::Open,
                },
                terms_url: web_link(sign_in.get("termsUrl")),
                privacy_url: web_link(sign_in.get("privacyUrl")),
                terms_url_template: web_template(sign_in.get("termsUrlTemplate")),
                privacy_url_template: web_template(sign_in.get("privacyUrlTemplate")),
            },
            "pairing" => Self::Pairing,
            "none" => Self::None,
            other => Self::Unsupported {
                announced: other.to_string(),
            },
        })
    }
}

/// A link the interface may open: `http(s)` only, as the schema's pattern.
fn web_link(value: Option<&serde_json::Value>) -> Option<String> {
    let link = value?.as_str()?;
    (link.starts_with("https://") || link.starts_with("http://")).then(|| link.to_string())
}

/// An address template the interface may resolve: a web link that holds the
/// `{lang}` placeholder (`service.schema.json` `termsUrlTemplate`).
fn web_template(value: Option<&serde_json::Value>) -> Option<String> {
    web_link(value).filter(|link| link.contains("{lang}"))
}

/// `202` of `POST /v1/sign-in/code`: a message is on its way. Any earlier
/// code of the address stops being valid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeRequested {
    /// The address as the service normalised it, and sent the message to.
    pub email: String,
    pub code_length: u8,
    pub expires_at: String,
    pub expires_in_seconds: u64,
    /// When "Send the code again" unlocks.
    pub resend_at: String,
    pub resend_after_seconds: u64,
}

/// The space a token belongs to (`sign-in.schema.json` `$defs/space`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceInfo {
    pub id: String,
    pub email: String,
    /// The address of the space on the web, where one signs in with the same
    /// e-mail address.
    pub web_url: String,
}

/// The device the sign-in registered (`$defs/registeredDevice`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredDevice {
    pub id: String,
    pub name: String,
}

/// `200` of `POST /v1/sign-in/verify` for the application. Never serialized:
/// the token must not travel to the interface.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedIn {
    /// `true` when this verification created the space.
    pub created: bool,
    pub space: SpaceInfo,
    pub device: RegisteredDevice,
    pub device_token: Secret,
}

/// `200` of `GET /v1/space`: the space of the token and the calling device.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentSpace {
    pub space: SpaceInfo,
    pub device: Option<RegisteredDevice>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceBody<'a> {
    application: &'static str,
    installation_id: &'a str,
    name: &'a str,
    platform: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<&'a str>,
}

impl<'a> DeviceBody<'a> {
    fn of(device: &'a DeviceIdentity) -> Self {
        Self {
            application: SYNC_APPLICATION,
            installation_id: &device.installation_id,
            name: &device.name,
            platform: current_platform(),
            model: device.model.as_deref(),
        }
    }
}

#[derive(Serialize)]
struct CodeRequestBody<'a> {
    email: &'a str,
    language: &'static str,
    device: DeviceBody<'a>,
}

#[derive(Serialize)]
struct VerifyBody<'a> {
    email: &'a str,
    code: &'a str,
    client: &'static str,
    device: DeviceBody<'a>,
}

#[derive(Serialize)]
struct RenameBody<'a> {
    name: &'a str,
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: ErrorDetail,
    #[serde(default, rename = "attemptsRemaining")]
    attempts_remaining: Option<u8>,
    #[serde(default, rename = "retryAt")]
    retry_at: Option<String>,
}

#[derive(Deserialize)]
struct ErrorDetail {
    code: String,
    message: String,
}

/// Reads a non-success answer as what it is: the end of a sign-in, or a
/// refusal in the v1 envelope (`invalid_response` when there is none).
fn refusal_of(answer: &HttpAnswer) -> SpaceError {
    match serde_json::from_slice::<ErrorEnvelope>(&answer.body) {
        Ok(envelope) => {
            if answer.status == 401
                && let Some(end) = SignInEnd::from_code(&envelope.error.code)
            {
                return SpaceError::SignInEnded(end);
            }
            SpaceError::Refused(Refusal {
                status: answer.status,
                code: envelope.error.code,
                message: envelope.error.message,
                attempts_remaining: envelope.attempts_remaining,
                retry_at: envelope.retry_at,
                retry_after_seconds: answer.retry_after_seconds,
            })
        }
        Err(_) => SpaceError::Refused(Refusal {
            status: answer.status,
            code: "invalid_response".to_string(),
            message: answer.text(),
            attempts_remaining: None,
            retry_at: None,
            retry_after_seconds: answer.retry_after_seconds,
        }),
    }
}

/// The application's own `503` `service_starting`: the service is up but
/// cannot reach its database yet, nothing was checked, sent or used.
fn is_service_starting(answer: &HttpAnswer) -> bool {
    answer.status == 503
        && serde_json::from_slice::<ErrorEnvelope>(&answer.body)
            .is_ok_and(|envelope| envelope.error.code == "service_starting")
}

fn parse<T: serde::de::DeserializeOwned>(answer: &HttpAnswer) -> Result<T, SpaceError> {
    serde_json::from_slice(&answer.body)
        .map_err(|error| SpaceError::InvalidResponse(error.to_string()))
}

/// A device id is a UUID made by the service; it is put in a path, so
/// anything else is refused rather than trusted.
fn checked_device_id(id: &str) -> Result<&str, SpaceError> {
    if !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        Ok(id)
    } else {
        Err(SpaceError::InvalidResponse(
            "unexpected device identifier".to_string(),
        ))
    }
}

/// The client of the hosted sign-in for one service.
pub struct SpaceClient {
    http: PatientHttp,
    base_url: String,
}

impl SpaceClient {
    /// A client that waits as long as the default policy says and reports
    /// nothing.
    pub fn new(base_url: &str) -> Result<Self, SpaceError> {
        Self::with_options(base_url, WakePolicy::default(), WaitHooks::silent())
    }

    /// Like [`SpaceClient::new`], with an explicit waiting policy (tests
    /// shorten it) and the hooks through which a caller shows the "the
    /// service is starting" state and can cancel the wait.
    pub fn with_options(
        base_url: &str,
        policy: WakePolicy,
        hooks: WaitHooks,
    ) -> Result<Self, SpaceError> {
        let parsed = reqwest::Url::parse(base_url.trim())
            .map_err(|_| SpaceError::InvalidServiceUrl(base_url.to_string()))?;
        if parsed.scheme() != "http" && parsed.scheme() != "https" {
            return Err(SpaceError::InvalidServiceUrl(base_url.to_string()));
        }
        let http = PatientHttp::new(policy, hooks).map_err(SpaceError::Transport)?;
        Ok(Self {
            http,
            base_url: base_url.trim().trim_end_matches('/').to_string(),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Reads the service's own announcement of how a person signs in. Says
    /// "the service is starting" and waits if the service is asleep.
    pub fn announcement(&self) -> Result<SignInAnnouncement, SpaceError> {
        let url = self.url(SERVICE_IDENTITY_PATH);
        let build: RequestFactory = Arc::new(move |client: &Client| client.get(url.clone()));
        let answer = self
            .http
            .send_safe(self.http.policy().attempt_timeout, &build)?;
        if !answer.is_success() {
            // Whatever answered here is not an Astro portal: a portal never
            // fails this route, which reads nothing.
            return Err(SpaceError::NotAnAstroPortal);
        }
        let body: serde_json::Value =
            serde_json::from_slice(&answer.body).map_err(|_| SpaceError::NotAnAstroPortal)?;
        SignInAnnouncement::parse(&body)
    }

    /// `POST /v1/sign-in/code`: has a code of six digits sent to `email`.
    /// The answer is the same whether the address already has a space or
    /// not. `locale` picks the language of the message (`fr`, `en`, `ja`).
    pub fn request_code(
        &self,
        email: &str,
        locale: &str,
        device: &DeviceIdentity,
    ) -> Result<CodeRequested, SpaceError> {
        let email = normalize_email(email)?;
        let name = normalize_device_name(&device.name)?;
        let device = DeviceIdentity {
            name,
            ..device.clone()
        };
        let url = self.url("/api/v1/sign-in/code");
        let body = CodeRequestBody {
            email: &email,
            language: message_language(locale),
            device: DeviceBody::of(&device),
        };
        let answer = self.send_sign_in_write(&url, &body)?;
        if answer.is_success() {
            return parse(&answer);
        }
        Err(refusal_of(&answer))
    }

    /// `POST /v1/sign-in/verify` with the code, as the application: the
    /// service creates the space of an address that has none, registers this
    /// computer under `device.name` and returns its device token. Sent once
    /// (see the module documentation).
    pub fn verify_code(
        &self,
        email: &str,
        code: &str,
        device: &DeviceIdentity,
    ) -> Result<SignedIn, SpaceError> {
        let email = normalize_email(email)?;
        let code = normalize_code(code)?;
        let name = normalize_device_name(&device.name)?;
        let device = DeviceIdentity {
            name,
            ..device.clone()
        };
        let url = self.url("/api/v1/sign-in/verify");
        let body = VerifyBody {
            email: &email,
            code: &code,
            client: "app",
            device: DeviceBody::of(&device),
        };
        let answer = self.send_sign_in_write(&url, &body)?;
        if answer.is_success() {
            let signed_in: SignedIn = parse(&answer)?;
            let token = signed_in.device_token.expose();
            if token.is_empty() || token.chars().any(char::is_whitespace) {
                return Err(SpaceError::InvalidResponse(
                    "unexpected device token".to_string(),
                ));
            }
            return Ok(signed_in);
        }
        Err(refusal_of(&answer))
    }

    /// Wakes the service, then sends one `POST` of the sign-in exactly once
    /// (more only when the application answers `service_starting`).
    fn send_sign_in_write<B: Serialize + Sync>(
        &self,
        url: &str,
        body: &B,
    ) -> Result<HttpAnswer, SpaceError> {
        Ok(self.http.send_once_while_starting(
            &self.base_url,
            self.http.policy().write_timeout,
            &|client: &Client| client.post(url).json(body),
            &is_service_starting,
        )?)
    }

    /// `GET /v1/space`: the space of `token` and this device. A bearer the
    /// service no longer knows (`401` `authentication_required`, « no
    /// credential, or a revoked one ») is a device that was detached.
    pub fn current_space(&self, token: &str) -> Result<CurrentSpace, SpaceError> {
        let url = self.url("/api/v1/space");
        let token = token.to_owned();
        let build: RequestFactory =
            Arc::new(move |client: &Client| client.get(url.clone()).bearer_auth(&token));
        let answer = self
            .http
            .send_safe(self.http.policy().attempt_timeout, &build)?;
        if answer.is_success() {
            return parse(&answer);
        }
        match refusal_of(&answer) {
            SpaceError::Refused(refusal)
                if answer.status == 401 && refusal.code == "authentication_required" =>
            {
                Err(SpaceError::SignInEnded(SignInEnd::DeviceDetached))
            }
            other => Err(other),
        }
    }

    /// `PATCH /v1/devices/{id}`: renames this computer in "My devices".
    /// A route of the space devices (`v1.md` « Space devices »): a service
    /// may not answer it.
    pub fn rename_device(
        &self,
        token: &str,
        device_id: &str,
        name: &str,
    ) -> Result<RegisteredDevice, SpaceError> {
        let name = normalize_device_name(name)?;
        let url = self.url(&format!(
            "/api/v1/devices/{}",
            checked_device_id(device_id)?
        ));
        let token = token.to_owned();
        let build: RequestFactory = Arc::new(move |client: &Client| {
            client
                .patch(url.clone())
                .bearer_auth(&token)
                .json(&RenameBody { name: &name })
        });
        let answer = self
            .http
            .send_safe(self.http.policy().attempt_timeout, &build)?;
        if answer.is_success() {
            return parse::<RenamedDevice>(&answer).map(|device| RegisteredDevice {
                id: device.id,
                name: device.name,
            });
        }
        Err(refusal_of(&answer))
    }

    /// `DELETE /v1/devices/{id}`: detaches this computer from its space. A
    /// device that is already detached answers `device_detached`, which is
    /// the same outcome: done. A route of the space devices: a service
    /// may not answer it.
    pub fn detach_device(&self, token: &str, device_id: &str) -> Result<(), SpaceError> {
        let url = self.url(&format!(
            "/api/v1/devices/{}",
            checked_device_id(device_id)?
        ));
        let token = token.to_owned();
        let build: RequestFactory =
            Arc::new(move |client: &Client| client.delete(url.clone()).bearer_auth(&token));
        let answer = self
            .http
            .send_safe(self.http.policy().attempt_timeout, &build)?;
        if answer.is_success() {
            return Ok(());
        }
        Err(refusal_of(&answer))
    }
}

/// `PATCH /v1/devices/{id}`'s body: the device as "My devices" lists it;
/// only what this client keeps.
#[derive(Deserialize)]
struct RenamedDevice {
    id: String,
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn what_the_service_announces_is_read_as_the_screen_to_show() {
        let identity = |sign_in: Option<serde_json::Value>| {
            let mut body = json!({"service": "astro-portal", "apiVersion": 1, "version": "1.0.1"});
            if let Some(sign_in) = sign_in {
                body["signIn"] = sign_in;
            }
            body
        };
        assert_eq!(
            SignInAnnouncement::parse(&identity(Some(json!({
                "method": "email-code",
                "spaceCreation": "open",
                "codeLength": 6,
                "termsUrl": "https://www.example.test/terms",
                "privacyUrl": "https://www.example.test/privacy"
            }))))
            .unwrap(),
            SignInAnnouncement::EmailCode {
                code_length: 6,
                space_creation: SpaceCreation::Open,
                terms_url: Some("https://www.example.test/terms".to_string()),
                privacy_url: Some("https://www.example.test/privacy".to_string()),
                terms_url_template: None,
                privacy_url_template: None,
            }
        );
        // A private server signing in by e-mail code: only its owner's
        // address opens it. `closed` is additive: an absent or unknown value
        // reads as `open`.
        assert_eq!(
            SignInAnnouncement::parse(&identity(Some(json!({
                "method": "email-code",
                "spaceCreation": "closed",
                "codeLength": 6
            }))))
            .unwrap(),
            SignInAnnouncement::EmailCode {
                code_length: 6,
                space_creation: SpaceCreation::Closed,
                terms_url: None,
                privacy_url: None,
                terms_url_template: None,
                privacy_url_template: None,
            }
        );
        for open in [
            json!({"method": "email-code"}),
            json!({"method": "email-code", "spaceCreation": "later"}),
        ] {
            assert_eq!(
                SignInAnnouncement::parse(&identity(Some(open))).unwrap(),
                SignInAnnouncement::EmailCode {
                    code_length: 6,
                    space_creation: SpaceCreation::Open,
                    terms_url: None,
                    privacy_url: None,
                    terms_url_template: None,
                    privacy_url_template: None,
                }
            );
        }
        // A page per language: the English address any client opens, and the
        // templates the interface resolves; a template without the
        // placeholder, or that is not a web link, is dropped.
        assert_eq!(
            SignInAnnouncement::parse(&identity(Some(json!({
                "method": "email-code",
                "spaceCreation": "open",
                "codeLength": 6,
                "termsUrl": "https://www.example.test/en/terms/",
                "privacyUrl": "https://www.example.test/en/privacy/",
                "termsUrlTemplate": "https://www.example.test/{lang}/terms/",
                "privacyUrlTemplate": "https://www.example.test/privacy"
            }))))
            .unwrap(),
            SignInAnnouncement::EmailCode {
                code_length: 6,
                space_creation: SpaceCreation::Open,
                terms_url: Some("https://www.example.test/en/terms/".to_string()),
                privacy_url: Some("https://www.example.test/en/privacy/".to_string()),
                terms_url_template: Some("https://www.example.test/{lang}/terms/".to_string()),
                privacy_url_template: None,
            }
        );
        // The legal links are absent, never empty -- and only web links.
        assert_eq!(
            SignInAnnouncement::parse(&identity(Some(json!({
                "method": "email-code",
                "spaceCreation": "open",
                "codeLength": 6,
                "termsUrl": "javascript:alert(1)"
            }))))
            .unwrap(),
            SignInAnnouncement::EmailCode {
                code_length: 6,
                space_creation: SpaceCreation::Open,
                terms_url: None,
                privacy_url: None,
                terms_url_template: None,
                privacy_url_template: None,
            }
        );
        assert_eq!(
            SignInAnnouncement::parse(&identity(Some(json!({"method": "pairing"})))).unwrap(),
            SignInAnnouncement::Pairing
        );
        assert_eq!(
            SignInAnnouncement::parse(&identity(Some(json!({"method": "none"})))).unwrap(),
            SignInAnnouncement::None
        );
        // No announcement at all: a private server older than the field.
        assert_eq!(
            SignInAnnouncement::parse(&identity(None)).unwrap(),
            SignInAnnouncement::Pairing
        );
        // A method newer than this application.
        assert_eq!(
            SignInAnnouncement::parse(&identity(Some(json!({"method": "passkey"})))).unwrap(),
            SignInAnnouncement::Unsupported {
                announced: "passkey".to_string()
            }
        );
    }

    #[test]
    fn something_that_is_not_an_astro_portal_is_refused() {
        for body in [
            json!("nope"),
            json!([]),
            json!({"service": "other", "apiVersion": 1, "version": "1"}),
            json!({"service": "astro-portal", "apiVersion": "1", "version": "1"}),
            json!({"service": "astro-portal", "apiVersion": 0, "version": "1"}),
            json!({"service": "astro-portal", "apiVersion": 1, "version": ""}),
            json!({"service": "astro-portal", "apiVersion": 1}),
        ] {
            assert!(
                matches!(
                    SignInAnnouncement::parse(&body),
                    Err(SpaceError::NotAnAstroPortal)
                ),
                "{body}"
            );
        }
        assert!(matches!(
            SignInAnnouncement::parse(
                &json!({"service": "astro-portal", "apiVersion": 2, "version": "2.0.0"})
            ),
            Err(SpaceError::IncompatibleApiVersion)
        ));
    }

    #[test]
    fn the_announcement_serialises_as_the_interface_reads_it() {
        let email = serde_json::to_value(SignInAnnouncement::EmailCode {
            code_length: 6,
            space_creation: SpaceCreation::Open,
            terms_url: Some("https://t.example".to_string()),
            privacy_url: None,
            terms_url_template: None,
            privacy_url_template: None,
        })
        .unwrap();
        assert_eq!(
            email,
            json!({"method": "email-code", "codeLength": 6, "spaceCreation": "open", "termsUrl": "https://t.example", "privacyUrl": null, "termsUrlTemplate": null, "privacyUrlTemplate": null})
        );
        assert_eq!(
            serde_json::to_value(SignInAnnouncement::EmailCode {
                code_length: 6,
                space_creation: SpaceCreation::Closed,
                terms_url: None,
                privacy_url: None,
                terms_url_template: None,
                privacy_url_template: None,
            })
            .unwrap()["spaceCreation"],
            json!("closed")
        );
        assert_eq!(
            serde_json::to_value(SignInAnnouncement::Pairing).unwrap(),
            json!({"method": "pairing"})
        );
        assert_eq!(
            serde_json::to_value(SignInAnnouncement::Unsupported {
                announced: "passkey".to_string()
            })
            .unwrap(),
            json!({"method": "unsupported", "announced": "passkey"})
        );
    }

    #[test]
    fn a_device_id_is_never_put_in_a_path_unchecked() {
        assert!(checked_device_id("6daf99d9-1c14-4206-bae2-3cec9456a714").is_ok());
        for refused in ["", "../space", "a/b", "a b", "a?b", &"a".repeat(65)] {
            assert!(checked_device_id(refused).is_err(), "{refused:?}");
        }
    }

    #[test]
    fn a_service_that_is_not_ready_is_told_from_a_refusal() {
        let answer = |status: u16, body: &str| HttpAnswer {
            status,
            body: body.as_bytes().to_vec(),
            retry_after_seconds: None,
        };
        assert!(is_service_starting(&answer(
            503,
            r#"{"error":{"code":"service_starting","message":"x"},"retryAt":"2026-09-29T21:14:10.000Z"}"#
        )));
        assert!(!is_service_starting(&answer(
            503,
            r#"{"error":{"code":"sign_in_unavailable","message":"x"}}"#
        )));
        assert!(!is_service_starting(&answer(502, "bad gateway")));
        assert!(!is_service_starting(&answer(
            429,
            r#"{"error":{"code":"service_starting","message":"x"}}"#
        )));
    }
}

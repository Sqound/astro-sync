//! What is already online for a night: `GET /v1/sessions/{id}/online`
//! (`contracts/schemas/api/session-online.schema.json`) read by a device token. Per target of the night, its captures, each
//! with its images (the main one first, then the variants), each image with
//! its SHA-256 and whether a sky solution exists for it; and the analysis in
//! place. The pre-check ([`crate::precheck`]) and the resume
//! ([`crate::night_resume`]) read it before anything is sent; it is never
//! gated by a capability.
//!
//! The readers are tolerant (no `deny_unknown_fields`): a field the service
//! adds later does not break an installed Astro Sync.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::gui_bridge::{GuiBridgeError, HttpPortalGuiTransport};
use crate::service_wait::RequestFactory;

/// One image of a capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineImage {
    pub id: String,
    pub main: bool,
    pub sha256: String,
    pub byte_size: u64,
    pub original_name: String,
    /// `phone`, `web`, `astro-sync`, `stacking`, or none (older images).
    pub source: Option<String>,
    pub created_at: String,
    /// `present` or `missing`.
    pub sky_solution: String,
}

impl OnlineImage {
    #[must_use]
    pub fn has_sky_solution(&self) -> bool {
        self.sky_solution == "present"
    }
}

/// One capture of the night.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineCapture {
    pub id: String,
    pub target_ids: Vec<String>,
    pub source: Option<String>,
    /// `draft` or `finalized`.
    pub stage: String,
    /// `private` or `public`.
    pub visibility: String,
    pub uploaded: bool,
    pub created_at: String,
    pub main_image: Option<OnlineImage>,
    pub image_count: u64,
    pub images: Vec<OnlineImage>,
}

impl OnlineCapture {
    /// The variants it holds besides its main image.
    #[must_use]
    pub fn variant_count(&self) -> u64 {
        self.image_count.saturating_sub(1)
    }
}

/// One target of the night and its captures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineTarget {
    pub target_id: Option<String>,
    pub name: Option<String>,
    pub planned: bool,
    pub captures: Vec<OnlineCapture>,
}

/// The night's analysis in place (`present: false` without one).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineAnalysis {
    pub present: bool,
    pub sent_at: Option<String>,
    pub origin_device_id: Option<String>,
    pub device_name: Option<String>,
    pub report_version: Option<u32>,
    pub producer: Option<String>,
    pub step_count: u64,
    pub edited_step_count: u64,
}

/// `GET /v1/sessions/{id}/online`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NightOnline {
    pub session_id: String,
    pub targets: Vec<OnlineTarget>,
    pub analysis: OnlineAnalysis,
}

impl NightOnline {
    /// The target `target_id`, when the night or a capture names it.
    #[must_use]
    pub fn target(&self, target_id: &str) -> Option<&OnlineTarget> {
        self.targets
            .iter()
            .find(|t| t.target_id.as_deref() == Some(target_id))
    }

    /// The capture of `target_id` a new image becomes a variant of: its
    /// first capture (the night's capture of that target).
    #[must_use]
    pub fn capture_of(&self, target_id: &str) -> Option<&OnlineCapture> {
        self.target(target_id)?.captures.first()
    }

    /// Every image online, with the capture that holds it.
    pub fn images(&self) -> impl Iterator<Item = (&OnlineCapture, &OnlineImage)> {
        self.targets
            .iter()
            .flat_map(|t| t.captures.iter())
            .flat_map(|c| c.images.iter().map(move |i| (c, i)))
    }

    /// The image online with the digest `sha256`, if any.
    #[must_use]
    pub fn image_with(&self, sha256: &str) -> Option<(&OnlineCapture, &OnlineImage)> {
        self.images()
            .find(|(_, image)| image.sha256.eq_ignore_ascii_case(sha256))
    }

    /// A night with nothing online beyond its plan: no capture, no analysis.
    #[must_use]
    pub fn is_bare(&self) -> bool {
        !self.analysis.present && self.targets.iter().all(|t| t.captures.is_empty())
    }
}

/// The read, behind a trait so tests never touch the network.
pub trait NightOnlineTransport {
    fn night_online(
        &self,
        base_url: &str,
        token: &str,
        session_id: &str,
    ) -> Result<NightOnline, GuiBridgeError>;
}

impl NightOnlineTransport for HttpPortalGuiTransport {
    /// A read: retried freely while the service wakes.
    fn night_online(
        &self,
        base_url: &str,
        token: &str,
        session_id: &str,
    ) -> Result<NightOnline, GuiBridgeError> {
        let url = format!(
            "{}/api/v1/sessions/{session_id}/online",
            base_url.trim_end_matches('/')
        );
        let token = token.to_owned();
        let build: RequestFactory =
            Arc::new(move |client| client.get(url.clone()).bearer_auth(&token));
        self.read_json(&build)
    }
}

//! Before anything is sent (the "Processing" and "Summary" steps).
//!
//! - **What each image becomes** ([`plan_images`]): an image whose
//!   SHA-256 is already online is "already online, identical" and never sent
//!   again; an image for a target whose capture is online is a **variant**
//!   of that capture (`captureId`; private, never the main image by itself)
//!   when `capture.variant_limit` > 0, else it "will not be sent"; the
//!   stacking and a `final/` file aiming at the same target make **one**
//!   variant (the stacked image takes the place); several renders of the
//!   stacking are each a variant when variants are allowed, and one only
//!   otherwise (the person chooses it).
//! A **progression image** is the history of its
//!   target's capture -- the one this sending creates, or the one online --
//!   with the role `progression`: never a capture of its own, never a
//!   variant (it counts against neither `capture.count_limit` nor
//!   `capture.variant_limit`, only against the storage), and it leaves
//!   after that capture's image.
//! - **The service's own answer, before any byte** ([`check_with_service`],
//!   `POST /v1/uploads/check`): `present`, `accepted` or `refused` with its
//!   numbers.
//! - **The four limits** ([`check_quotas`], from `GET
//!   /v1/space/storage`): the total against the place left, each file
//!   against the per-file ceiling, the variants of a capture against
//!   `capture.variant_limit`, the new captures against `capture.count_limit`.
//!   One exceeded: **nothing is enqueued** ([`enqueue_checked`]), and every
//!   finding says which file and which limit, with its numbers. The service
//!   checks again at reception; a later refusal stays in the queue with its
//!   code, resumable ([`crate::uploader`]).
//! - **The treatments** ([`treatments`]): decided by the
//!   capabilities `analysis.logs` and `analysis.progression` (the
//!   progression only: the stacking is local and never gated) and the tools
//!   present -- never by an offer's name. A private server announces every
//!   capability and its own limits.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::gui_bridge::{Capabilities, GuiBridgeError, HttpPortalGuiTransport};
use crate::night_online::NightOnline;
use crate::pipelines::DEFAULT_PIPELINE;
use crate::service_wait::RequestFactory;
use crate::upload_queue::{
    EnqueueOutcome, EntryTarget, ResultFile, ResultKind, UploadQueue, UploadQueueError,
};
use crate::uploader::UploadMetadata;

/* ------------------------------ The service ------------------------------ */

/// `GET /v1/space/storage` (`space-storage.schema.json`). A number the
/// profile does not cap is the unlimited sentinel, never absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceStorage {
    pub used_bytes: u64,
    pub quota_bytes: u64,
    pub remaining_bytes: u64,
    pub max_file_bytes: u64,
    pub capture_count: u64,
    pub capture_count_limit: u64,
    pub variant_limit: u64,
    pub capabilities: Capabilities,
}

/// The numbers an answer of `POST /v1/uploads/check` carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckStorage {
    pub used_bytes: u64,
    pub quota_bytes: u64,
    pub max_file_bytes: u64,
    pub capture_count: u64,
    pub capture_count_limit: u64,
    pub variant_limit: u64,
    #[serde(default)]
    pub variant_count: Option<u64>,
}

/// `POST /v1/uploads/check`'s answer (`upload-check.schema.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "result",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum UploadCheck {
    /// The space already holds these bytes: nothing would be stored.
    Present {
        capture_id: String,
        image_id: String,
        main: bool,
        storage: CheckStorage,
    },
    /// The upload would be received, as a new capture or a variant.
    Accepted { adds: String, storage: CheckStorage },
    /// The upload would be refused, with the code the upload would answer.
    Refused {
        code: String,
        message: String,
        storage: CheckStorage,
    },
}

/// The two routes, behind a trait so tests never touch the network.
pub trait PrecheckTransport {
    fn space_storage(&self, base_url: &str, token: &str) -> Result<SpaceStorage, GuiBridgeError>;
    fn check_upload(
        &self,
        base_url: &str,
        token: &str,
        metadata: &UploadMetadata,
    ) -> Result<UploadCheck, GuiBridgeError>;
}

impl PrecheckTransport for HttpPortalGuiTransport {
    /// A read.
    fn space_storage(&self, base_url: &str, token: &str) -> Result<SpaceStorage, GuiBridgeError> {
        let url = format!("{}/api/v1/space/storage", base_url.trim_end_matches('/'));
        let token = token.to_owned();
        let build: RequestFactory =
            Arc::new(move |client| client.get(url.clone()).bearer_auth(&token));
        self.read_json(&build)
    }

    /// « Nothing is written » (`uploads.md`): repeated freely while the
    /// service wakes.
    fn check_upload(
        &self,
        base_url: &str,
        token: &str,
        metadata: &UploadMetadata,
    ) -> Result<UploadCheck, GuiBridgeError> {
        let url = format!("{}/api/v1/uploads/check", base_url.trim_end_matches('/'));
        let token = token.to_owned();
        let body = metadata.clone();
        let build: RequestFactory =
            Arc::new(move |client| client.post(url.clone()).bearer_auth(&token).json(&body));
        self.read_json(&build)
    }
}

/* ------------------------- What each image becomes ----------------------- */

/// An image of the night that could leave: a target's final image, a render
/// of the stacking (`render`: the pipeline's id), or a progression image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageCandidate {
    pub target_id: String,
    pub path: PathBuf,
    pub sha256: String,
    pub size: u64,
    pub kind: ResultKind,
    #[serde(default)]
    pub render: Option<String>,
    /// The place of the render among those the stacking was asked for, the
    /// first asked being 0: the first render without a curve is the main image
    /// of the capture. `None` for an image whose order is not known (a render
    /// of an older stacking): the natural render is then the main one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_rank: Option<u32>,
    /// The contrast of presentation a render carries: the render
    /// without it stays the main image of the capture, the one with the curve
    /// is its variant.
    #[serde(default, skip_serializing_if = "crate::stacking::Contrast::is_none")]
    pub contrast: crate::stacking::Contrast,
    /// The colour intensity and the softened stars a render carries: like
    /// the contrast, an image that carries any is a variant, never the main one.
    #[serde(default, flatten)]
    pub retouch: crate::stacking::Retouch,
    /// A progression image: the light frames its milestone stacks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_count: Option<u64>,
}

/// The role a progression image is sent with (`uploadMetadata.role`, the
/// service's `capture_variants.role`).
pub const PROGRESSION_ROLE: &str = "progression";

/// Why an image will not be sent (said before the sending).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotSent {
    /// A different image for a target whose capture is online, and the space
    /// keeps no variant (`capture.variant_limit` = 0): "will not be
    /// sent".
    VariantsNotAllowed,
    /// The stacking aims at the same target: one variant, the stacked image.
    ReplacedByStacking,
    /// Another render of the stacking, while the space keeps no variant: one
    /// render is chosen before the sending.
    OneRenderOnly,
    /// Another final image of the same target: the first one goes.
    OtherFinalImage,
    /// A progression image whose target has no capture, online or in this
    /// sending: it would have nothing to be the history of.
    NoCaptureForProgression,
    /// A stacked image heavier than the per-file ceiling of the space
    /// (`storage.max_file_bytes`): it cannot leave, and the person's final
    /// image of the target is sent instead, never nothing.
    TooLargeForSpace,
}

/// What an image becomes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Decision {
    /// Same SHA-256 online: "already online, identical", not sent.
    AlreadyOnline {
        capture_id: String,
        image_id: String,
    },
    /// A new capture of its target.
    NewCapture,
    /// A variant of the capture online.
    Variant {
        capture_id: String,
    },
    /// A variant of the capture the image `first_sha256` creates in the same
    /// sending (several renders on a target with nothing online).
    VariantOfNew {
        first_sha256: String,
    },
    NotSent {
        reason: NotSent,
    },
    /// A progression image of the capture online (its history).
    Progression {
        capture_id: String,
    },
    /// A progression image of the capture the image `first_sha256` creates
    /// in the same sending.
    ProgressionOfNew {
        first_sha256: String,
    },
}

impl Decision {
    /// Whether the image leaves.
    #[must_use]
    pub fn sends(&self) -> bool {
        matches!(
            self,
            Decision::NewCapture
                | Decision::Variant { .. }
                | Decision::VariantOfNew { .. }
                | Decision::Progression { .. }
                | Decision::ProgressionOfNew { .. }
        )
    }

    /// Whether the image leaves as a progression image.
    #[must_use]
    pub fn is_progression(&self) -> bool {
        matches!(
            self,
            Decision::Progression { .. } | Decision::ProgressionOfNew { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePlan {
    pub image: ImageCandidate,
    #[serde(flatten)]
    pub decision: Decision,
    /// What Astro Sync proposes for this image and its capture
    /// ([`crate::metadata::attach_proposals`]); nothing until attached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<crate::metadata::Proposals>,
    /// The render started from the image a device stacked: the stacker's
    /// provenance says so ([`crate::metadata::recorded_device_master`]), attached
    /// with the proposals; what the window says of its quality and its origin
    /// follows.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub from_device: bool,
}

/// The sort key of a stacked image: the main image of the capture sorts first.
/// A render with a curve is never first; among the others the stacking's own
/// order decides (the first asked is the main), the natural render being the
/// main one when the order is not known.
fn main_order(image: &ImageCandidate) -> (bool, u32) {
    let rank = image
        .render_rank
        .unwrap_or(u32::from(image.render.as_deref() != Some(DEFAULT_PIPELINE)));
    (!image.contrast.is_none() || !image.retouch.is_none(), rank)
}

/// Makes `main` the main image of every target that has a render of that
/// pipeline: its renders sort first, the others after them in the order they
/// had. A target with no such render is left as it was.
pub fn prefer_main_render(candidates: &mut [ImageCandidate], main: &str) {
    let targets: Vec<String> = candidates
        .iter()
        .filter(|c| c.kind == ResultKind::Stacked && c.render.as_deref() == Some(main))
        .map(|c| c.target_id.clone())
        .collect();
    for candidate in candidates.iter_mut() {
        if candidate.kind != ResultKind::Stacked || !targets.contains(&candidate.target_id) {
            continue;
        }
        candidate.render_rank = Some(if candidate.render.as_deref() == Some(main) {
            0
        } else {
            1 + candidate.render_rank.unwrap_or(u32::from(
                candidate.render.as_deref() != Some(DEFAULT_PIPELINE),
            ))
        });
    }
}

/// Decides, target by target, what each candidate becomes (see the module
/// documentation). `variant_limit` is `capture.variant_limit`.
#[must_use]
pub fn plan_images(
    online: &NightOnline,
    candidates: &[ImageCandidate],
    variant_limit: u64,
) -> Vec<ImagePlan> {
    plan_images_within(online, candidates, variant_limit, u64::MAX)
}

/// [`plan_images`], knowing the space's per-file ceiling (`max_file_bytes`):
/// a stacked image above it cannot leave and is set aside (`TooLargeForSpace`),
/// and a target whose stacked images all are too heavy keeps its final image,
/// sent as if nothing had been stacked -- a night is never left with nothing
/// to send because the stacking is bigger than the space accepts.
#[must_use]
pub fn plan_images_within(
    online: &NightOnline,
    candidates: &[ImageCandidate],
    variant_limit: u64,
    max_file_bytes: u64,
) -> Vec<ImagePlan> {
    let variants_allowed = variant_limit > 0;
    let mut plans: Vec<Option<Decision>> = vec![None; candidates.len()];

    for (i, image) in candidates.iter().enumerate() {
        if let Some((capture, online_image)) = online.image_with(&image.sha256) {
            plans[i] = Some(Decision::AlreadyOnline {
                capture_id: capture.id.clone(),
                image_id: online_image.id.clone(),
            });
        }
    }

    let mut targets: Vec<&str> = Vec::new();
    for image in candidates {
        if !targets.contains(&image.target_id.as_str()) {
            targets.push(&image.target_id);
        }
    }
    for &target in &targets {
        let pending: Vec<usize> = (0..candidates.len())
            .filter(|&i| {
                plans[i].is_none()
                    && candidates[i].target_id == target
                    && candidates[i].kind != ResultKind::Progression
            })
            .collect();
        let mut stacked: Vec<usize> = Vec::new();
        for &i in &pending {
            if candidates[i].kind != ResultKind::Stacked {
                continue;
            }
            if candidates[i].size > max_file_bytes {
                plans[i] = Some(Decision::NotSent {
                    reason: NotSent::TooLargeForSpace,
                });
            } else {
                stacked.push(i);
            }
        }
        // The main image of the capture is the first render asked for when
        // the stacking recorded the order of its renders, else the natural
        // render when it is among them, whatever its file name; the other
        // renders are its variants (stable: the others keep their order). An
        // image with the contrast of presentation, softened stars or stronger
        // colours is never the main one (the render stays the capture, the
        // finished image is its variant).
        stacked.sort_by_key(|&i| main_order(&candidates[i]));
        let finals: Vec<usize> = pending
            .iter()
            .copied()
            .filter(|&i| candidates[i].kind == ResultKind::Final)
            .collect();
        let mut going: Vec<usize> = Vec::new();
        if stacked.is_empty() {
            if let Some((&first, rest)) = finals.split_first() {
                going.push(first);
                for &other in rest {
                    plans[other] = Some(Decision::NotSent {
                        reason: NotSent::OtherFinalImage,
                    });
                }
            }
        } else {
            for &f in &finals {
                plans[f] = Some(Decision::NotSent {
                    reason: NotSent::ReplacedByStacking,
                });
            }
            for (n, &s) in stacked.iter().enumerate() {
                if n == 0 || variants_allowed {
                    going.push(s);
                } else {
                    plans[s] = Some(Decision::NotSent {
                        reason: NotSent::OneRenderOnly,
                    });
                }
            }
        }
        let capture = online.capture_of(target);
        let mut first_new: Option<String> = None;
        for i in going {
            plans[i] = Some(match (capture, &first_new) {
                (Some(capture), _) if variants_allowed => Decision::Variant {
                    capture_id: capture.id.clone(),
                },
                (Some(_), _) => Decision::NotSent {
                    reason: NotSent::VariantsNotAllowed,
                },
                (None, None) => {
                    first_new = Some(candidates[i].sha256.clone());
                    Decision::NewCapture
                }
                (None, Some(first)) => Decision::VariantOfNew {
                    first_sha256: first.clone(),
                },
            });
        }
    }

    // The progression images, once each target's capture is known.
    for &target in &targets {
        let capture = progression_capture(online, candidates, &plans, target);
        for i in 0..candidates.len() {
            if plans[i].is_none()
                && candidates[i].target_id == target
                && candidates[i].kind == ResultKind::Progression
            {
                plans[i] = Some(capture.clone().unwrap_or(Decision::NotSent {
                    reason: NotSent::NoCaptureForProgression,
                }));
            }
        }
    }

    candidates
        .iter()
        .cloned()
        .zip(plans)
        .map(|(image, decision)| ImagePlan {
            image,
            decision: decision.unwrap_or(Decision::NotSent {
                reason: NotSent::OtherFinalImage,
            }),
            metadata: None,
            from_device: false,
        })
        .collect()
}

/// The capture a progression image of `target` is the history of: the one
/// an image of this sending creates, else the one an image of the target
/// already online, or a variant, names, else the target's capture online.
fn progression_capture(
    online: &NightOnline,
    candidates: &[ImageCandidate],
    plans: &[Option<Decision>],
    target: &str,
) -> Option<Decision> {
    let of_target = || {
        (0..candidates.len()).filter(move |&i| {
            candidates[i].target_id == target && candidates[i].kind != ResultKind::Progression
        })
    };
    if let Some(i) = of_target().find(|&i| plans[i] == Some(Decision::NewCapture)) {
        return Some(Decision::ProgressionOfNew {
            first_sha256: candidates[i].sha256.clone(),
        });
    }
    let named = of_target().find_map(|i| match &plans[i] {
        Some(Decision::AlreadyOnline { capture_id, .. } | Decision::Variant { capture_id }) => {
            Some(capture_id.clone())
        }
        _ => None,
    });
    named
        .or_else(|| online.capture_of(target).map(|c| c.id.clone()))
        .map(|capture_id| Decision::Progression { capture_id })
}

/// After the service's answer: a progression image waiting for the capture
/// an image of this sending would create, when the service said that image
/// is already online, is the history of that capture online.
pub fn rebind_progressions(plans: &mut [ImagePlan]) {
    let online: BTreeMap<String, String> = plans
        .iter()
        .filter_map(|plan| match &plan.decision {
            Decision::AlreadyOnline { capture_id, .. } => {
                Some((plan.image.sha256.clone(), capture_id.clone()))
            }
            _ => None,
        })
        .collect();
    for plan in plans.iter_mut() {
        if let Decision::ProgressionOfNew { first_sha256 } = &plan.decision
            && let Some(capture_id) = online.get(first_sha256)
        {
            plan.decision = Decision::Progression {
                capture_id: capture_id.clone(),
            };
        }
    }
}

/* ------------------------------ The four limits -------------------------- */

/// A limit an image or the sending would pass, with its numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "limit",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum QuotaFinding {
    /// `storage.quota_bytes`: the total to send against the place left.
    StorageQuota {
        to_send_bytes: u64,
        remaining_bytes: u64,
        quota_bytes: u64,
    },
    /// `storage.max_file_bytes`: one file over the ceiling.
    FileTooLarge {
        path: PathBuf,
        size: u64,
        max_file_bytes: u64,
    },
    /// `capture.variant_limit`: a capture would hold more variants than
    /// allowed (`capture_id` none for a capture this sending creates).
    VariantLimit {
        target_id: String,
        capture_id: Option<String>,
        existing: u64,
        adding: u64,
        variant_limit: u64,
    },
    /// `capture.count_limit`: the new captures past the space's limit.
    CaptureCount {
        adding: u64,
        capture_count: u64,
        capture_count_limit: u64,
    },
    /// The service's own answer to `POST /v1/uploads/check`.
    Refused {
        path: PathBuf,
        code: String,
        storage: CheckStorage,
    },
}

impl QuotaFinding {
    /// The stable code of the limit, as the service names it.
    #[must_use]
    pub fn code(&self) -> &str {
        match self {
            QuotaFinding::StorageQuota { .. } => "storage_quota_reached",
            QuotaFinding::FileTooLarge { .. } => "file_too_large",
            QuotaFinding::VariantLimit { .. } => "capture_variant_limit_reached",
            QuotaFinding::CaptureCount { .. } => "capture_limit_reached",
            QuotaFinding::Refused { code, .. } => code,
        }
    }
}

/// The verdict of the "Summary" on a sending.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaCheck {
    /// Nothing exceeded: the sending may start.
    pub allowed: bool,
    /// The bytes of the images that would leave.
    pub to_send_bytes: u64,
    pub to_send_files: u64,
    pub new_captures: u64,
    pub findings: Vec<QuotaFinding>,
}

/// The four limits, checked on the images that would leave (pure).
#[must_use]
pub fn check_quotas(
    storage: &SpaceStorage,
    online: &NightOnline,
    plans: &[ImagePlan],
) -> QuotaCheck {
    let sending: Vec<&ImagePlan> = plans.iter().filter(|p| p.decision.sends()).collect();
    let to_send_bytes: u64 = sending.iter().map(|p| p.image.size).sum();
    let mut findings = Vec::new();

    if to_send_bytes > storage.remaining_bytes {
        findings.push(QuotaFinding::StorageQuota {
            to_send_bytes,
            remaining_bytes: storage.remaining_bytes,
            quota_bytes: storage.quota_bytes,
        });
    }
    for plan in &sending {
        if plan.image.size > storage.max_file_bytes {
            findings.push(QuotaFinding::FileTooLarge {
                path: plan.image.path.clone(),
                size: plan.image.size,
                max_file_bytes: storage.max_file_bytes,
            });
        }
    }

    // Variants per capture: online ones first, then those of a new capture.
    let mut adding: BTreeMap<(String, Option<String>), u64> = BTreeMap::new();
    for plan in &sending {
        match &plan.decision {
            Decision::Variant { capture_id } => {
                *adding
                    .entry((plan.image.target_id.clone(), Some(capture_id.clone())))
                    .or_insert(0) += 1;
            }
            Decision::VariantOfNew { .. } => {
                *adding
                    .entry((plan.image.target_id.clone(), None))
                    .or_insert(0) += 1;
            }
            _ => {}
        }
    }
    for ((target_id, capture_id), count) in adding {
        let existing = capture_id
            .as_deref()
            .and_then(|id| {
                online
                    .targets
                    .iter()
                    .flat_map(|t| t.captures.iter())
                    .find(|c| c.id == id)
            })
            .map_or(0, |c| c.variant_count());
        if existing + count > storage.variant_limit {
            findings.push(QuotaFinding::VariantLimit {
                target_id,
                capture_id,
                existing,
                adding: count,
                variant_limit: storage.variant_limit,
            });
        }
    }

    let new_captures = sending
        .iter()
        .filter(|p| p.decision == Decision::NewCapture)
        .count() as u64;
    if new_captures > 0 && storage.capture_count + new_captures > storage.capture_count_limit {
        findings.push(QuotaFinding::CaptureCount {
            adding: new_captures,
            capture_count: storage.capture_count,
            capture_count_limit: storage.capture_count_limit,
        });
    }

    QuotaCheck {
        allowed: findings.is_empty(),
        to_send_bytes,
        to_send_files: sending.len() as u64,
        new_captures,
        findings,
    }
}

/* ------------------------- The service, before bytes --------------------- */

/// The metadata `POST /v1/uploads/check` and the upload itself carry for a
/// plan (`captureId` for a variant or a progression image of an online
/// capture; the role and the frames of a progression image).
#[must_use]
pub fn metadata_of(plan: &ImagePlan, session_id: &str) -> UploadMetadata {
    UploadMetadata {
        session_id: Some(session_id.to_string()),
        plan_id: None,
        target_ids: vec![plan.image.target_id.clone()],
        original_name: plan.image.path.file_name().map_or_else(
            || plan.image.sha256.clone(),
            |n| n.to_string_lossy().into_owned(),
        ),
        sha256: plan.image.sha256.clone(),
        byte_size: plan.image.size,
        content_type: None,
        capture_id: match &plan.decision {
            Decision::Variant { capture_id } | Decision::Progression { capture_id } => {
                Some(capture_id.clone())
            }
            _ => None,
        },
        source: Some(source_of(plan.image.kind).to_string()),
        role: (plan.image.kind == ResultKind::Progression).then(|| PROGRESSION_ROLE.to_string()),
        progression_frames: plan
            .image
            .frame_count
            .filter(|_| plan.image.kind == ResultKind::Progression),
        project_id: None,
        combined_night_ids: None,
        totals_by_filter: None,
        plan_render: None,
        metadata_source: None,
        proposals: crate::metadata::Proposals::default(),
    }
}

fn source_of(kind: ResultKind) -> &'static str {
    match kind {
        ResultKind::Stacked | ResultKind::Plan => "stacking",
        ResultKind::Final | ResultKind::Progression => "astro-sync",
    }
}

/// Asks the service about every image that would leave, before a byte does:
/// an image it already holds becomes [`Decision::AlreadyOnline`] (not sent);
/// a refusal is returned as a [`QuotaFinding::Refused`]. (A variant of a
/// capture this sending creates is checked when it is sent: its capture does
/// not exist yet.)
pub fn check_with_service<T: PrecheckTransport>(
    transport: &T,
    portal_url: &str,
    token: &str,
    session_id: &str,
    plans: &mut [ImagePlan],
) -> Result<Vec<QuotaFinding>, GuiBridgeError> {
    let mut findings = Vec::new();
    for plan in plans.iter_mut() {
        if !matches!(
            plan.decision,
            Decision::NewCapture | Decision::Variant { .. } | Decision::Progression { .. }
        ) {
            continue;
        }
        match transport.check_upload(portal_url, token, &metadata_of(plan, session_id))? {
            UploadCheck::Present {
                capture_id,
                image_id,
                ..
            } => {
                plan.decision = Decision::AlreadyOnline {
                    capture_id,
                    image_id,
                };
            }
            UploadCheck::Accepted { .. } => {}
            UploadCheck::Refused { code, storage, .. } => findings.push(QuotaFinding::Refused {
                path: plan.image.path.clone(),
                code,
                storage,
            }),
        }
    }
    Ok(findings)
}

/// Enqueues the images that leave, each with its night, its target and, for
/// a variant or a progression image, its capture -- **only when `check`
/// allows it**: a limit passed enqueues nothing and returns the findings. The
/// progression images go last, after the image whose capture they are the
/// history of.
pub fn enqueue_checked(
    queue: &UploadQueue,
    session_id: &str,
    plans: &[ImagePlan],
    check: &QuotaCheck,
) -> Result<Result<EnqueueOutcome, Vec<QuotaFinding>>, UploadQueueError> {
    if !check.allowed {
        return Ok(Err(check.findings.clone()));
    }
    let (history, mut images): (Vec<&ImagePlan>, Vec<&ImagePlan>) = plans
        .iter()
        .filter(|plan| plan.decision.sends())
        .partition(|plan| plan.decision.is_progression());
    // The capture goes before the renders that are its variants, wherever the
    // natural render stands in the list of files.
    images.sort_by_key(|plan| matches!(plan.decision, Decision::VariantOfNew { .. }));
    let files: Vec<ResultFile> = images
        .into_iter()
        .chain(history)
        .map(|plan| ResultFile {
            path: plan.image.path.clone(),
            sha256: plan.image.sha256.clone(),
            size: plan.image.size,
            kind: plan.image.kind,
            target: EntryTarget {
                session_id: Some(session_id.to_string()),
                target_ids: vec![plan.image.target_id.clone()],
                variant_of: match &plan.decision {
                    Decision::Variant { capture_id } | Decision::Progression { capture_id } => {
                        Some(capture_id.clone())
                    }
                    Decision::VariantOfNew { first_sha256 }
                    | Decision::ProgressionOfNew { first_sha256 } => Some(format!(
                        "{}{first_sha256}",
                        crate::uploader::VARIANT_OF_SHA256
                    )),
                    _ => None,
                },
                source: Some(source_of(plan.image.kind).to_string()),
                metadata: plan.metadata.clone(),
                frame_count: plan
                    .image
                    .frame_count
                    .filter(|_| plan.image.kind == ResultKind::Progression),
                plan: None,
            },
        })
        .collect();
    queue.enqueue_results(&files).map(Ok)
}

/* ------------------------------- Treatments ------------------------------ */

/// The five treatments of the "Processing" step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Treatment {
    /// "Stacking from the FITS" (the pipeline).
    Stacking,
    /// "Sending the already processed final image".
    FinalImage,
    /// "Sky annotations" (ASTAP).
    SkyAnnotations,
    /// "Logs" (NINA, PHD2, ASIAIR readers).
    Logs,
    /// "Progression" (Siril).
    Progression,
}

/// Why a treatment cannot be ticked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "why",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Unavailable {
    /// The space's capabilities do not grant it (greyed, "See the offers").
    NotInSubscription { capability: String },
    /// Its tool is missing ("Settings › Dependencies").
    ToolMissing { tool: String },
    /// The night has nothing for it (no frame, no log, no final image).
    NothingToTreat,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreatmentState {
    pub treatment: Treatment,
    pub available: bool,
    pub unavailable: Option<Unavailable>,
}

/// The tools Settings › Dependencies found.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsPresent {
    pub stacking_pipeline: bool,
    pub astap: bool,
    pub siril: bool,
}

/// What the night's folder and what is online give each treatment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NightMaterial {
    pub light_frames: bool,
    /// An image a device stacked: the stacking renders it, so it is material
    /// for the stacking and the sky annotations, not for the progression
    /// (which reads poses).
    #[serde(default)]
    pub device_images: bool,
    pub logs: bool,
    /// A final image here or online.
    pub final_image: bool,
}

/// The five treatments, each available or not, and why. The
/// capability comes first, then the tool, then the material.
#[must_use]
pub fn treatments(
    capabilities: &Capabilities,
    tools: &ToolsPresent,
    material: &NightMaterial,
) -> Vec<TreatmentState> {
    let state = |treatment: Treatment,
                 gate: Option<(&str, bool)>,
                 tool: Option<(&str, bool)>,
                 has: bool| {
        let unavailable = match (gate, tool) {
            (Some((capability, false)), _) => Some(Unavailable::NotInSubscription {
                capability: capability.to_string(),
            }),
            (_, Some((tool, false))) => Some(Unavailable::ToolMissing {
                tool: tool.to_string(),
            }),
            _ if !has => Some(Unavailable::NothingToTreat),
            _ => None,
        };
        TreatmentState {
            treatment,
            available: unavailable.is_none(),
            unavailable,
        }
    };
    // The stacking is local and never gated: its tool only.
    let can_stack = tools.stacking_pipeline;
    vec![
        state(
            Treatment::Stacking,
            None,
            Some(("stacking-pipeline", tools.stacking_pipeline)),
            material.light_frames || material.device_images,
        ),
        state(Treatment::FinalImage, None, None, material.final_image),
        state(
            Treatment::SkyAnnotations,
            None,
            Some(("astap", tools.astap)),
            // A final image, or frames the stacking turns into one: the
            // sky solution is made after the image is sent, so the stacked
            // image is annotated like any other. Frames count only when
            // the stacking can run.
            material.final_image
                || ((material.light_frames || material.device_images) && can_stack),
        ),
        state(
            Treatment::Logs,
            Some(("analysis.logs", capabilities.analysis_logs)),
            None,
            material.logs,
        ),
        state(
            Treatment::Progression,
            Some(("analysis.progression", capabilities.analysis_progression)),
            Some(("siril", tools.siril)),
            material.light_frames,
        ),
    ]
}

/// Whether several renders of the stacking may be ticked (each a variant),
/// or one only.
#[must_use]
pub fn several_renders_allowed(storage: &SpaceStorage) -> bool {
    storage.variant_limit > 0
}

#[cfg(test)]
mod sky_annotation_tests {
    use super::*;

    fn capabilities(progression: bool) -> Capabilities {
        serde_json::from_value(serde_json::json!({
            "planner.multiple": true, "planner.mosaic": true, "pointing.solve": true,
            "sync.plan_limit": 0, "gallery.private": true, "portal.invitations": true,
            "capture.variant_limit": 0, "capture.count_limit": 3, "capture.public": true,
            "analysis.logs": true, "analysis.progression": progression,
            "storage.max_file_bytes": 100, "storage.quota_bytes": 1000,
            "history.retention_days": 0
        }))
        .unwrap()
    }

    fn tools(astap: bool, stacking_pipeline: bool) -> ToolsPresent {
        ToolsPresent {
            stacking_pipeline,
            astap,
            siril: true,
        }
    }

    fn sky(
        capabilities: &Capabilities,
        tools: &ToolsPresent,
        material: NightMaterial,
    ) -> TreatmentState {
        treatments(capabilities, tools, &material)
            .into_iter()
            .find(|state| state.treatment == Treatment::SkyAnnotations)
            .unwrap()
    }

    const FRAMES: NightMaterial = NightMaterial {
        light_frames: true,
        device_images: false,
        logs: false,
        final_image: false,
    };

    #[test]
    fn frames_to_stack_open_the_sky_annotations_without_a_final_image() {
        let state = sky(&capabilities(true), &tools(true, true), FRAMES);
        assert!(state.available);
        assert_eq!(state.unavailable, None);
    }

    #[test]
    fn no_frames_and_no_image_leave_nothing_to_annotate() {
        let state = sky(
            &capabilities(true),
            &tools(true, true),
            NightMaterial::default(),
        );
        assert_eq!(state.unavailable, Some(Unavailable::NothingToTreat));
    }

    #[test]
    fn frames_that_cannot_be_stacked_do_not_open_the_sky_annotations() {
        let no_pipeline = sky(&capabilities(true), &tools(true, false), FRAMES);
        assert_eq!(no_pipeline.unavailable, Some(Unavailable::NothingToTreat));
        // Without `analysis.progression` the frames are stacked all the same.
        assert!(sky(&capabilities(false), &tools(true, true), FRAMES).available);
        // A final image needs no stacking.
        let final_only = NightMaterial {
            final_image: true,
            ..NightMaterial::default()
        };
        assert!(sky(&capabilities(false), &tools(true, false), final_only).available);
    }

    #[test]
    fn the_stacking_is_never_gated_the_progression_keeps_its_capability() {
        let of = |capabilities: &Capabilities, treatment: Treatment| {
            treatments(capabilities, &tools(true, true), &FRAMES)
                .into_iter()
                .find(|state| state.treatment == treatment)
                .unwrap()
        };
        // A free account (no `analysis.progression`) stacks.
        assert!(of(&capabilities(false), Treatment::Stacking).available);
        assert!(of(&capabilities(true), Treatment::Stacking).available);
        // The progression, the intermediate stackings sent, keeps its gate.
        assert_eq!(
            of(&capabilities(false), Treatment::Progression).unavailable,
            Some(Unavailable::NotInSubscription {
                capability: "analysis.progression".to_string()
            })
        );
        assert!(of(&capabilities(true), Treatment::Progression).available);
    }

    #[test]
    fn a_missing_solver_still_says_so_before_the_material() {
        let state = sky(&capabilities(true), &tools(false, true), FRAMES);
        assert_eq!(
            state.unavailable,
            Some(Unavailable::ToolMissing {
                tool: "astap".to_string()
            })
        );
    }
}

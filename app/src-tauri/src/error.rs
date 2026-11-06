//! Every Tauri command below returns `Result<T, CommandError>` rather than
//! a raw string: `code` is a stable, machine-readable identifier the React
//! side looks up in its own locale dictionaries (`errors.<code>`), and
//! `message` is the core's own (English) `Display` text, shown as a
//! fallback for a portal-specific refusal reason that cannot be
//! pre-translated. This is what keeps error *presentation* entirely on the
//! localized frontend while the bridge stays a thin, un-opinionated
//! wrapper over the core's own error types.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub code: &'static str,
    pub message: String,
    /// What a screen needs beside the code, when there is something: the
    /// attempts left of a wrong sign-in code, when a wait ends. Absent
    /// otherwise, so every other error keeps its two-field shape.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl CommandError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    /// For a failure with no dedicated core error type (a Tauri API call,
    /// a path resolution failure): still a stable code so the frontend has
    /// *something* to key a generic "unexpected error" message on, plus
    /// the raw detail for troubleshooting.
    pub fn other(message: impl Into<String>) -> Self {
        Self::new("unknown_error", message)
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

impl From<astro_sync_core::gui_bridge::GuiBridgeError> for CommandError {
    fn from(error: astro_sync_core::gui_bridge::GuiBridgeError) -> Self {
        // The stable code lives with the error, in the core: the command
        // line and this bridge never disagree on it (`GuiBridgeError::code`).
        Self::new(error.code(), error.to_string())
    }
}

impl From<astro_sync_core::space::SpaceError> for CommandError {
    fn from(error: astro_sync_core::space::SpaceError) -> Self {
        // The refusals of the sign-in carry what a screen counts down or
        // tells: the attempts left, when the wait ends.
        let details = error.refusal().and_then(|refusal| {
            let mut details = serde_json::Map::new();
            if let Some(attempts) = refusal.attempts_remaining {
                details.insert("attemptsRemaining".to_string(), attempts.into());
            }
            if let Some(at) = &refusal.retry_at {
                details.insert("retryAt".to_string(), at.clone().into());
            }
            if let Some(seconds) = refusal.retry_after_seconds {
                details.insert("retryAfterSeconds".to_string(), seconds.into());
            }
            (!details.is_empty()).then_some(serde_json::Value::Object(details))
        });
        Self {
            code: error.code(),
            message: error.to_string(),
            details,
        }
    }
}

impl From<astro_sync_core::session_detection::DetectError> for CommandError {
    fn from(error: astro_sync_core::session_detection::DetectError) -> Self {
        Self::new(error.code(), error.to_string())
    }
}

impl From<astro_sync_core::session_detection::OrganiseError> for CommandError {
    fn from(error: astro_sync_core::session_detection::OrganiseError) -> Self {
        use astro_sync_core::session_detection::OrganiseError as E;
        let code = match &error {
            E::SymlinkUnavailable => "symlink_unavailable",
            E::LinkAcrossVolumes { .. } => "link_across_volumes",
            E::Io { .. } => "io_error",
        };
        Self::new(code, error.to_string())
    }
}

impl From<astro_sync_core::scanner::ScanError> for CommandError {
    fn from(error: astro_sync_core::scanner::ScanError) -> Self {
        use astro_sync_core::scanner::ScanError as E;
        let code = match &error {
            E::NotADirectory(_) => "folder_not_found",
            E::Io { .. } | E::Walk(_) => "io_error",
            E::Cancelled => "cancelled",
        };
        Self::new(code, error.to_string())
    }
}

impl From<astro_sync_core::package::PackageError> for CommandError {
    fn from(error: astro_sync_core::package::PackageError) -> Self {
        use astro_sync_core::package::PackageError as E;
        let code = match &error {
            E::EmptyReport => "empty_dry_run_report",
            E::OutputParentNotFound(_) => "parent_not_found",
            E::OutputInsideSource { .. } => "output_inside_source",
            E::PackageAlreadyExists(_) => "package_already_exists",
            E::InvalidAssetPath { .. } => "invalid_asset_path",
            E::Io { .. } => "io_error",
            E::AnalysisPathCollision(_) => "analysis_path_collision",
            E::ManifestUnreadable { .. } => "package_manifest_unreadable",
            E::ProgressionPathCollision(_) => "progression_path_collision",
            E::UnknownMilestoneLabel(_) => "unknown_milestone_label",
        };
        Self::new(code, error.to_string())
    }
}

impl From<astro_sync_core::upload_queue::UploadQueueError> for CommandError {
    fn from(error: astro_sync_core::upload_queue::UploadQueueError) -> Self {
        use astro_sync_core::upload_queue::UploadQueueError as E;
        let code = match &error {
            E::Open { .. } => "queue_open_error",
            E::Database(_) => "queue_error",
            E::NotFound(_) => "queue_entry_not_found",
            E::NotAResult { .. } => "not_a_result",
        };
        Self::new(code, error.to_string())
    }
}

impl From<astro_sync_core::uploader::UploadError> for CommandError {
    fn from(error: astro_sync_core::uploader::UploadError) -> Self {
        // `network_error`, `service_start_timeout`, `cancelled`: the failures
        // a person can act on have a code of their own; the rest stay
        // `upload_error` with the core's text.
        Self::new(error.code(), error.to_string())
    }
}

impl From<astro_sync_core::analysis::AnalysisError> for CommandError {
    fn from(error: astro_sync_core::analysis::AnalysisError) -> Self {
        use astro_sync_core::analysis::AnalysisError as E;
        let code = match &error {
            E::Scan(astro_sync_core::scanner::ScanError::Cancelled) => "cancelled",
            E::Scan(_) => "folder_not_found",
            E::Io { .. } => "io_error",
            E::Phd2 { .. } => "invalid_phd2_guide_log",
            E::Asiair { .. } => "invalid_asiair_autorun_log",
            E::Nina { .. } => "invalid_nina_log",
        };
        Self::new(code, error.to_string())
    }
}

impl From<astro_sync_core::local_settings::LocalSettingsError> for CommandError {
    fn from(error: astro_sync_core::local_settings::LocalSettingsError) -> Self {
        Self::new(error.code(), error.to_string())
    }
}

impl From<astro_sync_core::digest_cache::DigestCacheError> for CommandError {
    fn from(error: astro_sync_core::digest_cache::DigestCacheError) -> Self {
        Self::new("io_error", error.to_string())
    }
}

impl From<astro_sync_core::working_folder::WorkingFolderError> for CommandError {
    fn from(error: astro_sync_core::working_folder::WorkingFolderError) -> Self {
        Self::new(error.code(), error.to_string())
    }
}

impl From<astro_sync_core::detection::DetectionError> for CommandError {
    fn from(error: astro_sync_core::detection::DetectionError) -> Self {
        Self::new(error.code(), error.to_string())
    }
}

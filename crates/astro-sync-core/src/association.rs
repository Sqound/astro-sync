//! Linking a hosted session id to a local folder.
//!
//! Two ways to associate a folder: an existing folder is associated as-is
//! ([`Association::existing`]); a brand-new folder is only ever created inside
//! a parent directory the user picked ([`Association::propose_new`]) -- Astro
//! Sync never invents a location on its own. Associations persist as plain JSON
//! in a config file path the caller supplies explicitly; that file is never
//! written inside the associated folder itself (checked in
//! [`save_association`]), so it can never be mistaken for one of the user's own
//! captures or swept up by the read-only scanner.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::paths::is_within;
use crate::time_util::now_iso8601_utc;

/// One session-to-folder link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Association {
    pub session_id: String,
    pub folder: PathBuf,
    /// ISO 8601 UTC timestamp of when this association was recorded.
    pub created_at: String,
    /// Whether `folder` was created by this association (a brand-new,
    /// user-requested folder) rather than an existing one associated as-is.
    pub proposed: bool,
}

/// The on-disk shape of the config file: every association recorded so
/// far, keyed implicitly by `session_id` (at most one record per session).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssociationStore {
    pub associations: Vec<Association>,
}

impl AssociationStore {
    #[must_use]
    pub fn find(&self, session_id: &str) -> Option<&Association> {
        self.associations
            .iter()
            .find(|association| association.session_id == session_id)
    }
}

#[derive(Debug, Error)]
pub enum AssociationError {
    #[error("session id must not be empty")]
    EmptySessionId,
    #[error("existing folder does not exist or is not a directory: {0}")]
    FolderNotFound(PathBuf),
    #[error("parent directory does not exist or is not a directory: {0}")]
    ParentNotFound(PathBuf),
    #[error(
        "a folder named {name:?} already exists in {parent}; associate it as an existing folder instead"
    )]
    FolderAlreadyExists { parent: PathBuf, name: String },
    #[error(
        "the config file must not be stored inside the associated folder ({config} is inside {folder})"
    )]
    ConfigInsideFolder { config: PathBuf, folder: PathBuf },
    #[error("io error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("invalid config file JSON at {path}: {source}")]
    InvalidConfig {
        path: PathBuf,
        source: serde_json::Error,
    },
}

impl Association {
    /// Associates an existing folder as-is. `folder` must already exist;
    /// nothing is created, moved, or modified inside it.
    pub fn existing(
        session_id: impl Into<String>,
        folder: impl AsRef<Path>,
    ) -> Result<Self, AssociationError> {
        let session_id = require_non_empty_session_id(session_id)?;
        let folder = folder.as_ref();
        if !folder.is_dir() {
            return Err(AssociationError::FolderNotFound(folder.to_path_buf()));
        }
        Ok(Self {
            session_id,
            folder: folder.to_path_buf(),
            created_at: now_iso8601_utc(),
            proposed: false,
        })
    }

    /// Creates a brand-new folder named `folder_name` inside `parent` (which
    /// must already exist and be a directory the user chose) and associates
    /// it. Refuses if a folder with that name already exists there -- an
    /// existing folder must go through [`Association::existing`] instead,
    /// so "proposed" always means "just created for this association".
    pub fn propose_new(
        session_id: impl Into<String>,
        parent: impl AsRef<Path>,
        folder_name: &str,
    ) -> Result<Self, AssociationError> {
        let session_id = require_non_empty_session_id(session_id)?;
        let parent = parent.as_ref();
        if !parent.is_dir() {
            return Err(AssociationError::ParentNotFound(parent.to_path_buf()));
        }
        let folder = parent.join(folder_name);
        if folder.exists() {
            return Err(AssociationError::FolderAlreadyExists {
                parent: parent.to_path_buf(),
                name: folder_name.to_string(),
            });
        }
        fs::create_dir(&folder).map_err(|source| AssociationError::Io {
            path: folder.clone(),
            source,
        })?;
        Ok(Self {
            session_id,
            folder,
            created_at: now_iso8601_utc(),
            proposed: true,
        })
    }
}

fn require_non_empty_session_id(session_id: impl Into<String>) -> Result<String, AssociationError> {
    let session_id = session_id.into();
    if session_id.trim().is_empty() {
        return Err(AssociationError::EmptySessionId);
    }
    Ok(session_id)
}

/// Loads the association store from `config_path`. A missing file is not an
/// error: it simply means no association has been recorded yet.
pub fn load_store(config_path: &Path) -> Result<AssociationStore, AssociationError> {
    if !config_path.exists() {
        return Ok(AssociationStore::default());
    }
    let raw = fs::read_to_string(config_path).map_err(|source| AssociationError::Io {
        path: config_path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&raw).map_err(|source| AssociationError::InvalidConfig {
        path: config_path.to_path_buf(),
        source,
    })
}

/// Records `association` in the config file at `config_path`, replacing any
/// prior association for the same `session_id`. Refuses when `config_path`
/// would resolve to somewhere inside the associated folder.
pub fn save_association(
    config_path: &Path,
    association: &Association,
) -> Result<AssociationStore, AssociationError> {
    if is_within(&association.folder, config_path).unwrap_or(false) {
        return Err(AssociationError::ConfigInsideFolder {
            config: config_path.to_path_buf(),
            folder: association.folder.clone(),
        });
    }

    let mut store = load_store(config_path)?;
    store
        .associations
        .retain(|existing| existing.session_id != association.session_id);
    store.associations.push(association.clone());

    if let Some(parent) = config_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|source| AssociationError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let json = serde_json::to_string_pretty(&store).expect("association store serializes");
    fs::write(config_path, json).map_err(|source| AssociationError::Io {
        path: config_path.to_path_buf(),
        source,
    })?;
    Ok(store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;
    use std::fs;

    #[test]
    fn existing_rejects_an_empty_session_id() {
        let dir = TempDir::new("assoc-empty-session");
        assert!(matches!(
            Association::existing("  ", dir.path()),
            Err(AssociationError::EmptySessionId)
        ));
    }

    #[test]
    fn existing_associates_an_existing_folder_as_is() {
        let dir = TempDir::new("assoc-existing");
        let association = Association::existing("session-1", dir.path()).unwrap();
        assert_eq!(association.folder, dir.path());
        assert!(!association.proposed);
    }

    #[test]
    fn existing_rejects_a_folder_that_does_not_exist() {
        let dir = TempDir::new("assoc-missing");
        let missing = dir.path().join("does-not-exist");
        assert!(matches!(
            Association::existing("session-1", &missing),
            Err(AssociationError::FolderNotFound(path)) if path == missing
        ));
    }

    #[test]
    fn propose_new_creates_the_folder_inside_the_chosen_parent() {
        let parent = TempDir::new("assoc-propose-parent");
        let association =
            Association::propose_new("session-1", parent.path(), "2026-09-20-m31").unwrap();
        assert!(association.proposed);
        assert_eq!(association.folder, parent.path().join("2026-09-20-m31"));
        assert!(association.folder.is_dir());
    }

    #[test]
    fn propose_new_refuses_when_a_folder_with_that_name_already_exists() {
        let parent = TempDir::new("assoc-propose-exists");
        fs::create_dir(parent.path().join("already-there")).unwrap();
        assert!(matches!(
            Association::propose_new("session-1", parent.path(), "already-there"),
            Err(AssociationError::FolderAlreadyExists { .. })
        ));
    }

    #[test]
    fn propose_new_refuses_a_parent_that_does_not_exist() {
        let missing_parent = std::env::temp_dir().join("astro-sync-does-not-exist-parent");
        assert!(matches!(
            Association::propose_new("session-1", &missing_parent, "folder"),
            Err(AssociationError::ParentNotFound(_))
        ));
    }

    #[test]
    fn save_and_load_round_trip_and_replace_by_session_id() {
        let folder = TempDir::new("assoc-store-folder");
        let config_dir = TempDir::new("assoc-store-config");
        let config_path = config_dir.path().join("astro-sync.json");

        let first = Association::existing("session-1", folder.path()).unwrap();
        save_association(&config_path, &first).unwrap();

        let second_folder = TempDir::new("assoc-store-folder-2");
        let second = Association::existing("session-1", second_folder.path()).unwrap();
        let store = save_association(&config_path, &second).unwrap();

        assert_eq!(store.associations.len(), 1);
        assert_eq!(
            store.find("session-1").unwrap().folder,
            second_folder.path()
        );

        let reloaded = load_store(&config_path).unwrap();
        assert_eq!(reloaded.associations.len(), 1);
        assert_eq!(
            reloaded.find("session-1").unwrap().folder,
            second_folder.path()
        );
    }

    #[test]
    fn load_store_of_a_missing_config_file_is_an_empty_store() {
        let config_dir = TempDir::new("assoc-store-missing");
        let store = load_store(&config_dir.path().join("does-not-exist.json")).unwrap();
        assert!(store.associations.is_empty());
    }

    #[test]
    fn save_association_refuses_a_config_path_inside_the_associated_folder() {
        let folder = TempDir::new("assoc-config-inside");
        let association = Association::existing("session-1", folder.path()).unwrap();
        let config_path = folder.path().join("astro-sync.json");
        assert!(matches!(
            save_association(&config_path, &association),
            Err(AssociationError::ConfigInsideFolder { .. })
        ));
        assert!(!config_path.exists());
    }
}

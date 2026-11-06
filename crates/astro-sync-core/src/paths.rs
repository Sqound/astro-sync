//! Path helpers shared by the scanner and the package producer: turning a
//! filesystem path into the forward-slash relative form the open
//! session-package contract requires, checking whether one path is nested
//! inside another (the "output must never be inside the source folder"
//! rule), and validating a candidate package-relative path against the same
//! traversal rules as `manifest.schema.json`'s `relativePath` pattern
//! (`contracts/schemas/session-package/manifest.schema.json`).

use std::io;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

/// Converts a path into a forward-slash-separated string, regardless of the
/// host platform's own separator. Used for every path that ends up in a
/// dry-run report or a package manifest, since the contract requires
/// forward slashes.
#[must_use]
pub fn to_relative_slash_string(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Lexically resolves `.`/`..` components without touching the filesystem
/// (unlike [`Path::canonicalize`], which requires the path to exist).
fn normalize_lexically(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                result.pop();
            }
            Component::CurDir => {}
            other => result.push(other.as_os_str()),
        }
    }
    result
}

/// Returns whether `candidate` resolves to a location inside (or equal to)
/// `base`. `base` must already exist (it is canonicalized, resolving
/// symlinks); `candidate` does not need to exist yet, since this is also
/// used to reject an output location before it has been created. As much of
/// `candidate` as already exists is canonicalized too (resolving any
/// symlinked ancestor, e.g. macOS's `/var` -> `/private/var`), so a
/// system-temp-directory path compares correctly against a canonicalized
/// `base` even before the candidate itself has been created.
pub fn is_within(base: &Path, candidate: &Path) -> io::Result<bool> {
    let base = base.canonicalize()?;
    let candidate_absolute = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        std::env::current_dir()?.join(candidate)
    };
    let candidate_normalized = normalize_lexically(&candidate_absolute);
    let candidate_resolved = resolve_existing_prefix(&candidate_normalized)?;
    Ok(candidate_resolved.starts_with(&base))
}

/// Canonicalizes as much of `path` as actually exists on disk, then
/// reattaches the remaining, not-yet-existing trailing components
/// unresolved (a path segment that does not exist yet cannot itself be a
/// symlink). `path` must already be absolute and lexically normalized (no
/// `.`/`..` components), so walking up via [`Path::parent`] is meaningful.
fn resolve_existing_prefix(path: &Path) -> io::Result<PathBuf> {
    if let Ok(canonical) = path.canonicalize() {
        return Ok(canonical);
    }
    let mut trailing = Vec::new();
    let mut current = path;
    loop {
        let Some(parent) = current.parent() else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("no existing ancestor of {}", path.display()),
            ));
        };
        trailing.push(
            current
                .file_name()
                .expect("a path with a parent has a file name")
                .to_owned(),
        );
        if let Ok(mut resolved) = parent.canonicalize() {
            for component in trailing.into_iter().rev() {
                resolved.push(component);
            }
            return Ok(resolved);
        }
        current = parent;
    }
}

/// Why a relative asset path was rejected. Mirrors the checks
/// `manifest.schema.json`'s `relativePath` pattern performs at the schema
/// level; the package producer runs this independently before writing
/// anything, per the contract README's "two separate checks, both
/// mandatory" rule for importers -- the same discipline applies to the
/// producer that writes the paths in the first place.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PathValidationError {
    #[error("path is empty")]
    Empty,
    #[error("path must be relative, not absolute")]
    Absolute,
    #[error("path must not carry a drive letter")]
    DriveLetter,
    #[error("path segment {0:?} is empty")]
    EmptySegment(String),
    #[error("path segment {0:?} is a \".\" or \"..\" traversal")]
    Traversal(String),
}

/// Validates a package-relative path string against the same rules as
/// `manifest.schema.json`'s `relativePath` pattern: relative, forward-slash
/// separated, no leading slash, no drive letter, no `.`/`..` segment.
pub fn validate_relative_asset_path(path: &str) -> Result<(), PathValidationError> {
    if path.is_empty() {
        return Err(PathValidationError::Empty);
    }
    if path.starts_with('/') {
        return Err(PathValidationError::Absolute);
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(PathValidationError::DriveLetter);
    }
    for segment in path.split('/') {
        if segment.is_empty() {
            return Err(PathValidationError::EmptySegment(path.to_string()));
        }
        if segment == "." || segment == ".." {
            return Err(PathValidationError::Traversal(segment.to_string()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn is_within_holds_for_a_direct_child_and_the_base_itself() {
        let base = TempDir::new("paths-is-within-base");
        let child = base.path().join("child");
        std::fs::create_dir(&child).unwrap();
        assert!(is_within(base.path(), &child).unwrap());
        assert!(is_within(base.path(), base.path()).unwrap());
    }

    #[test]
    fn is_within_holds_even_for_a_not_yet_created_grandchild() {
        // Exercises the exact scenario the package producer relies on: the
        // candidate (the not-yet-created package output directory) is
        // nested under `base`, which on macOS resolves through the
        // `/var` -> `/private/var` symlink once canonicalized -- the
        // not-yet-existing candidate must still compare as "inside" `base`.
        let base = TempDir::new("paths-is-within-not-yet-created");
        let not_yet_created = base.path().join("astro-session-package").join("images");
        assert!(is_within(base.path(), &not_yet_created).unwrap());
    }

    #[test]
    fn is_within_is_false_for_an_unrelated_directory() {
        let base = TempDir::new("paths-is-within-base-2");
        let other = TempDir::new("paths-is-within-other");
        assert!(!is_within(base.path(), other.path()).unwrap());
    }

    #[test]
    fn accepts_a_well_formed_relative_path() {
        assert_eq!(
            validate_relative_asset_path("images/frame-0001.fits"),
            Ok(())
        );
        assert_eq!(validate_relative_asset_path("checksums.sha256"), Ok(()));
    }

    #[test]
    fn rejects_absolute_paths() {
        assert_eq!(
            validate_relative_asset_path("/etc/passwd"),
            Err(PathValidationError::Absolute)
        );
    }

    #[test]
    fn rejects_a_windows_drive_letter() {
        assert_eq!(
            validate_relative_asset_path("C:/observatory/secret.fits"),
            Err(PathValidationError::DriveLetter)
        );
    }

    #[test]
    fn rejects_parent_dir_traversal_anywhere_in_the_path() {
        assert_eq!(
            validate_relative_asset_path("images/../../etc/passwd"),
            Err(PathValidationError::Traversal("..".to_string()))
        );
        assert_eq!(
            validate_relative_asset_path(".."),
            Err(PathValidationError::Traversal("..".to_string()))
        );
    }

    #[test]
    fn rejects_a_current_dir_segment_and_an_empty_segment() {
        assert_eq!(
            validate_relative_asset_path("images/./frame.fits"),
            Err(PathValidationError::Traversal(".".to_string()))
        );
        assert_eq!(
            validate_relative_asset_path("images//frame.fits"),
            Err(PathValidationError::EmptySegment(
                "images//frame.fits".to_string()
            ))
        );
    }

    #[test]
    fn rejects_an_empty_path() {
        assert_eq!(
            validate_relative_asset_path(""),
            Err(PathValidationError::Empty)
        );
    }
}

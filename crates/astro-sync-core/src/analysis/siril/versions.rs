//! The compatible-version whitelist ("data, not code", point 1):
//! [`siril_versions.json`](siril_versions.json) is embedded at compile time
//! via [`include_str!`] and holds every
//! `siril --version`/`siril-cli --version` output this crate recognizes as
//! compatible with the progression orchestration. Adding, removing, or
//! reordering a version is a one-line JSON edit, never a `.rs` change.
//!
//! This module also holds [`extract_version`], the small, dependency-free
//! parser that pulls a version token out of whatever `siril --version`
//! actually prints (`"Siril 1.2.1\n"`, `"siril 1.4.4-git"`, ...): no regex
//! crate is added for this one narrow need, matching the same
//! zero-dependency stance `fits.rs` and `time_util.rs` already take.

use serde::Deserialize;

const SIRIL_VERSIONS_JSON: &str = include_str!("siril_versions.json");

#[derive(Debug, Deserialize)]
struct SirilVersionsFile {
    #[serde(rename = "compatibleVersions")]
    compatible_versions: Vec<String>,
}

/// The whitelist of compatible Siril versions, in the exact order
/// `siril_versions.json` lists them. Parsing a file this crate embeds and
/// commits itself can only fail if that file is malformed, which is a
/// build-time invariant this crate controls -- exactly the same
/// "always parses, or the crate itself is broken" assumption
/// `package::build_manifest`'s `.expect("manifest always serializes")`
/// already makes elsewhere.
#[must_use]
pub fn compatible_versions() -> Vec<String> {
    serde_json::from_str::<SirilVersionsFile>(SIRIL_VERSIONS_JSON)
        .expect("siril_versions.json is committed by this crate and must always parse")
        .compatible_versions
}

/// Whether `version` (as [`extract_version`] returns it) is in
/// [`compatible_versions`]. Exact string match: a version list entry names
/// one specific, verified release, never a range or prefix, so a person
/// reading `siril_versions.json` sees exactly what was tested.
#[must_use]
pub fn is_compatible(version: &str) -> bool {
    compatible_versions().iter().any(|known| known == version)
}

/// A character is part of a version token if it could plausibly appear
/// inside a dotted version number: digits, `.`, or a trailing
/// pre-release/build tag made of ASCII letters, digits, or `-`/`_` (covers
/// forms like `"1.4.4-git"` or `"1.2.0_rc1"` without pulling in a real
/// semver parser for a value this crate only ever compares for exact
/// equality).
fn is_version_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_'
}

/// Extracts the first whitespace-separated token in `text` that looks like
/// a dotted version number (starts with an ASCII digit and contains at
/// least one `.`), from whatever `siril --version`/`siril-cli --version`
/// printed to stdout and/or stderr -- real-world output looks like
/// `"Siril 1.2.1"`, `"siril-cli 1.4.4"`, or `"Siril ver. 1.2.1"`; this scans
/// words rather than anchoring on a fixed prefix so the exact banner
/// wording never needs to be hardcoded here. Returns `None` when no such
/// token is found (unrecognized output, or the process printed nothing
/// useful) -- callers treat that the same as the binary being absent.
#[must_use]
pub fn extract_version(text: &str) -> Option<String> {
    for word in text.split_ascii_whitespace() {
        let trimmed = word.trim_matches(|c: char| !is_version_token_char(c));
        if !trimmed.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        if !trimmed.contains('.') {
            continue;
        }
        if trimmed.chars().all(is_version_token_char) {
            return Some(trimmed.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_committed_whitelist_parses_and_is_not_empty() {
        let versions = compatible_versions();
        assert!(!versions.is_empty());
        assert!(versions.contains(&"1.2.1".to_string()));
        assert!(versions.contains(&"1.4.4".to_string()));
    }

    #[test]
    fn the_development_line_is_deliberately_excluded() {
        // See `siril_versions.json`'s own "notes" field: 1.5.x is the
        // rolling development line, not yet a confirmed-stable target.
        assert!(!compatible_versions().iter().any(|v| v.starts_with("1.5")));
    }

    #[test]
    fn is_compatible_matches_a_known_version_exactly() {
        assert!(is_compatible("1.2.1"));
        assert!(!is_compatible("1.2.1-git"));
        assert!(!is_compatible("0.9.9"));
    }

    #[test]
    fn extract_version_reads_the_common_siril_version_banner() {
        assert_eq!(extract_version("Siril 1.2.1\n"), Some("1.2.1".to_string()));
        assert_eq!(
            extract_version("siril-cli 1.4.4"),
            Some("1.4.4".to_string())
        );
        assert_eq!(
            extract_version("Siril ver. 1.2.1"),
            Some("1.2.1".to_string())
        );
    }

    #[test]
    fn extract_version_handles_a_prerelease_suffix() {
        assert_eq!(
            extract_version("Siril 1.4.5-git"),
            Some("1.4.5-git".to_string())
        );
    }

    #[test]
    fn extract_version_returns_none_for_unrecognized_output() {
        assert_eq!(extract_version(""), None);
        assert_eq!(extract_version("command not found"), None);
        assert_eq!(extract_version("Siril"), None);
    }
}

//! How Astro Sync says what it is to the Astro portal.
//!
//! Every request to the portal carries one header,
//! `X-PlanAstro-Client: astro-sync-macos/1.0.0+6aa28f767 (macOS 15.1)`: the
//! product (by platform), the version, the commit that was built, and the
//! system with its version when it can be read. The service uses it to know
//! which builds are still in use; it parses the value and ignores one that
//! is malformed, so this module only ever produces values that follow the
//! contract: every token made of `[0-9A-Za-z._-]`, at most 40 characters,
//! the whole header at most 200.
//!
//! Nothing personal goes in it: no identifier, no machine name, no language.
//! The header is added by the two clients that talk to the portal
//! ([`crate::service_wait::PatientHttp`] and the error-report transport) and
//! by no other: the downloads of tools and of the reference catalogue go to
//! third parties, which must learn nothing about the application.

use std::sync::OnceLock;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

use crate::build_info;
use crate::error_report::System;
use crate::space;

/// The name of the header, lower case as HTTP/2 wants it.
pub const HEADER_NAME: &str = "x-planastro-client";

/// The longest token (version, build, system version) the service accepts.
const MAX_TOKEN_CHARS: usize = 40;

/// The longest header value the service accepts.
const MAX_HEADER_CHARS: usize = 200;

/// The value for the running program, composed once.
static VALUE: OnceLock<String> = OnceLock::new();

/// Tells the identity which version the application is. The window calls it
/// at start-up with the version of `tauri.conf.json`; the first call wins,
/// and a program that never calls it (the command line, the tests) is
/// identified by the version of this crate. It must come before the first
/// request: the value is composed once and kept.
pub fn install(version: &str) {
    VALUE.get_or_init(|| compose_for_this_program(version));
}

/// The header value of the running program.
#[must_use]
pub fn value() -> &'static str {
    VALUE.get_or_init(|| compose_for_this_program(env!("CARGO_PKG_VERSION")))
}

/// The headers to give to `ClientBuilder::default_headers` of a client that
/// talks to the portal -- and only to it.
#[must_use]
pub fn headers() -> HeaderMap {
    let mut map = HeaderMap::new();
    // The value only holds visible ASCII by construction; were that ever
    // wrong, no header is better than a client that cannot be built.
    if let Ok(value) = HeaderValue::from_str(value()) {
        map.insert(HeaderName::from_static(HEADER_NAME), value);
    }
    map
}

fn compose_for_this_program(version: &str) -> String {
    let build = build_info::build_info(version).git_sha;
    let system = System::current();
    compose(
        space::current_platform(),
        version,
        Some(&build),
        system.os_version.as_deref(),
    )
}

/// The header value for a platform (`macos`, `windows`, anything else is
/// Linux), an application `version`, the built commit (none, or
/// [`build_info::UNKNOWN_SHA`], leaves the `+build` out) and the system with
/// its version when it is known (the system is named after the platform, so
/// the comment can never contradict the product).
#[must_use]
pub fn compose(
    platform: &str,
    version: &str,
    build: Option<&str>,
    system_version: Option<&str>,
) -> String {
    let (product, system_name) = match platform {
        "macos" => ("astro-sync-macos", "macOS"),
        "windows" => ("astro-sync-windows", "Windows"),
        _ => ("astro-sync-linux", "Linux"),
    };
    let version = token_prefix(version).unwrap_or_else(|| "0".to_string());
    let mut value = format!("{product}/{version}");
    if let Some(build) = build
        .filter(|build| *build != build_info::UNKNOWN_SHA)
        .and_then(whole_token)
    {
        value.push('+');
        value.push_str(&build);
    }
    match system_version.and_then(whole_token) {
        Some(system_version) => value.push_str(&format!(" ({system_name} {system_version})")),
        None => value.push_str(&format!(" ({system_name})")),
    }
    // Unreachable with the limits above; kept so the contract holds even if
    // they change.
    value.truncate(MAX_HEADER_CHARS);
    value
}

fn allowed(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

/// `text` when all of it is a valid token (1 to 40 allowed characters).
fn whole_token(text: &str) -> Option<String> {
    let valid =
        !text.is_empty() && text.chars().count() <= MAX_TOKEN_CHARS && text.chars().all(allowed);
    valid.then(|| text.to_string())
}

/// The longest valid start of `text`: `1.0.0+91` is `1.0.0`. A version is
/// cut where it stops being valid (the build metadata of a semantic version
/// is the commit, which is stated on its own) rather than dropped.
fn token_prefix(text: &str) -> Option<String> {
    let prefix: String = text
        .trim()
        .chars()
        .take_while(|c| allowed(*c))
        .take(MAX_TOKEN_CHARS)
        .collect();
    (!prefix.is_empty()).then_some(prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nominal_value_on_each_system() {
        assert_eq!(
            compose("macos", "1.0.0", Some("6aa28f767"), Some("15.1")),
            "astro-sync-macos/1.0.0+6aa28f767 (macOS 15.1)"
        );
        assert_eq!(
            compose("linux", "1.0.0", Some("6aa28f767"), Some("24.04")),
            "astro-sync-linux/1.0.0+6aa28f767 (Linux 24.04)"
        );
        assert_eq!(
            compose("windows", "1.0.0", None, None),
            "astro-sync-windows/1.0.0 (Windows)"
        );
    }

    #[test]
    fn an_unknown_platform_is_linux() {
        assert_eq!(
            compose("freebsd", "1.0.0", None, None),
            "astro-sync-linux/1.0.0 (Linux)"
        );
    }

    #[test]
    fn a_missing_or_unknown_build_leaves_out_the_plus() {
        for build in [None, Some(build_info::UNKNOWN_SHA), Some(""), Some("a b")] {
            assert_eq!(
                compose("macos", "1.0.0", build, Some("15.1")),
                "astro-sync-macos/1.0.0 (macOS 15.1)",
                "{build:?}"
            );
        }
    }

    #[test]
    fn a_system_without_a_readable_version_has_only_its_name() {
        for version in [None, Some(""), Some("15 1"), Some("15.1)")] {
            assert_eq!(
                compose("macos", "1.0.0", None, version),
                "astro-sync-macos/1.0.0 (macOS)",
                "{version:?}"
            );
        }
    }

    #[test]
    fn a_version_is_cut_where_it_stops_being_valid() {
        assert_eq!(
            compose("macos", "1.0.0+91", None, None),
            "astro-sync-macos/1.0.0 (macOS)"
        );
        assert_eq!(
            compose("macos", "1.0.0-beta.2 (x)", None, None),
            "astro-sync-macos/1.0.0-beta.2 (macOS)"
        );
        // Nothing valid at all: a placeholder, never an empty token.
        assert_eq!(
            compose("macos", " +x", None, None),
            "astro-sync-macos/0 (macOS)"
        );
    }

    #[test]
    fn forbidden_characters_never_reach_the_header() {
        let value = compose(
            "windows",
            "1.0\r\nX-Evil: 1",
            Some("abc\ndef"),
            Some("10;rm"),
        );
        assert_eq!(value, "astro-sync-windows/1.0 (Windows)");
        assert!(HeaderValue::from_str(&value).is_ok());
    }

    #[test]
    fn the_tokens_and_the_whole_are_bounded() {
        let long = "1".repeat(100);
        let forty = "2".repeat(40);
        let cut = compose("macos", &long, Some(&forty), Some(&forty));
        assert_eq!(
            cut,
            format!(
                "astro-sync-macos/{}+{forty} (macOS {forty})",
                "1".repeat(40)
            )
        );
        assert!(cut.chars().count() <= MAX_HEADER_CHARS);
        // A build or a system version that is too long is left out whole.
        let value = compose(
            "windows",
            "1.0.0",
            Some(&"3".repeat(41)),
            Some(&"4".repeat(41)),
        );
        assert_eq!(value, "astro-sync-windows/1.0.0 (Windows)");
    }

    #[test]
    fn the_value_of_this_program_follows_the_contract() {
        let value = value();
        assert!(value.chars().count() <= MAX_HEADER_CHARS, "{value}");
        let (product, rest) = value.split_once('/').unwrap();
        assert!(
            ["astro-sync-macos", "astro-sync-windows", "astro-sync-linux"].contains(&product),
            "{value}"
        );
        assert!(rest.starts_with(env!("CARGO_PKG_VERSION")), "{value}");
        assert!(rest.ends_with(')') && rest.contains(" ("), "{value}");
        let map = headers();
        assert_eq!(map.get(HEADER_NAME).unwrap().to_str().unwrap(), value);
    }
}

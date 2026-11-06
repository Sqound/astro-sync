//! Canonical JSON serialization: sorted object keys, no insignificant
//! whitespace, stable number formatting and string escaping.
//!
//! Two callers need exactly this: `astro-sync-cli`'s golden tests
//! (`crates/astro-sync-cli/tests/golden.rs`), which must compare the CLI's
//! `--json` stdout against the structures this crate produces directly with
//! nothing depending on field-declaration order or formatting whitespace;
//! and the Tauri GUI, which can use the same function to hash or diff a
//! report deterministically. Both get one shared implementation instead of
//! each growing its own key-sorting logic.
//!
//! `serde_json::Value`'s `Object` variant is a `BTreeMap` as long as
//! `serde_json`'s `preserve_order` feature is not enabled -- and it is not,
//! anywhere in this workspace (`Cargo.lock` carries no `indexmap`), so
//! Cargo has nothing to unify that feature on. Round-tripping any
//! `Serialize` value through `serde_json::Value` therefore already yields
//! keys sorted lexicographically at every nesting level; combining that
//! with `serde_json`'s ordinary compact writer (`to_string`, not
//! `to_string_pretty`) drops insignificant whitespace. `serde_json` already
//! formats numbers and escapes strings the same way on every call, so
//! nothing extra is needed for those.

use serde::Serialize;

/// Serializes `value` in a canonical, deterministic form: object keys
/// sorted, no insignificant whitespace.
///
/// # Panics
///
/// Panics if `value`'s `Serialize` implementation itself fails. None of
/// this crate's own report/outcome types ever do (the same assumption
/// `package::produce_package` already makes: see its own
/// `.expect("manifest always serializes")`), so this keeps the same
/// infallible-in-practice signature the plan calls for
/// (`canonical_json(value) -> String`) rather than pushing an error type
/// every caller would have to handle for a failure mode that cannot occur.
#[must_use]
pub fn canonical_json<T: Serialize + ?Sized>(value: &T) -> String {
    let as_value = serde_json::to_value(value).expect("value always serializes to JSON");
    serde_json::to_string(&as_value).expect("a serde_json::Value always serializes")
}

#[cfg(test)]
mod tests {
    use super::canonical_json;
    use serde::Serialize;
    use serde_json::json;

    #[derive(Serialize)]
    struct DeclaredOutOfOrder {
        zebra: u32,
        apple: u32,
        mango: u32,
    }

    #[test]
    fn sorts_object_keys_regardless_of_declaration_order() {
        let value = DeclaredOutOfOrder {
            zebra: 1,
            apple: 2,
            mango: 3,
        };
        assert_eq!(canonical_json(&value), r#"{"apple":2,"mango":3,"zebra":1}"#);
    }

    #[test]
    fn sorts_keys_at_every_nesting_level() {
        let value = json!({
            "outer_z": { "inner_z": 1, "inner_a": 2 },
            "outer_a": 3,
        });
        assert_eq!(
            canonical_json(&value),
            r#"{"outer_a":3,"outer_z":{"inner_a":2,"inner_z":1}}"#
        );
    }

    #[test]
    fn drops_insignificant_whitespace() {
        let value = json!({"a": [1, 2, 3], "b": "text"});
        let rendered = canonical_json(&value);
        assert!(!rendered.contains(' '));
        assert!(!rendered.contains('\n'));
    }

    #[test]
    fn two_serializations_of_the_same_value_are_byte_identical() {
        let value = json!({"b": 1, "a": [true, false, null], "c": "é\"\\"});
        assert_eq!(canonical_json(&value), canonical_json(&value));
    }

    #[test]
    fn escaping_is_stable_for_quotes_backslashes_and_non_ascii() {
        let value = json!({"text": "quote \" backslash \\ accent é"});
        assert_eq!(
            canonical_json(&value),
            r#"{"text":"quote \" backslash \\ accent é"}"#
        );
    }
}

//! The smallest JSON Schema (2020-12 subset) evaluator that reads the
//! contracts Astro Sync exchanges with the service: the keywords
//! `session-analysis.schema.json` and `session-online.schema.json` use --
//! `$ref` (same document), `type`, `required`, `properties`,
//! `additionalProperties: false`, `items`, `anyOf`, `oneOf`, `const`, `enum`,
//! `minimum`, `minLength`, `maxLength`, `pattern` (only the `^[0-9a-f]{64}$`
//! of a SHA-256). The service validates a report with the same subset
//! (`packages/portal/src/api/session-analysis-report-schema.js`), so a report
//! this evaluator accepts is a report the service accepts.
//!
//! Shared by path (`#[path = "support/schema_check.rs"] mod schema_check;`).

#![allow(dead_code)]

use serde_json::Value;
use std::path::PathBuf;

/// The schema file `name` of `contracts/schemas/api/`.
pub fn contract(name: &str) -> Value {
    // The copy beside the crates (`contracts/`).
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts");
    let path = root.join("schemas/api").join(name);
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

fn resolve<'a>(root: &'a Value, reference: &str) -> &'a Value {
    let pointer = reference
        .strip_prefix('#')
        .expect("same-document $ref only");
    root.pointer(pointer)
        .unwrap_or_else(|| panic!("cannot resolve {reference}"))
}

fn type_matches(value: &Value, kind: &str) -> bool {
    match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.as_f64().is_some_and(|n| n.fract() == 0.0),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => true,
    }
}

/// Every way `value` breaks `schema` (empty: it is valid).
pub fn errors(schema: &Value, value: &Value, root: &Value, path: &str) -> Vec<String> {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        // Another contract of the same folder (`capabilities.schema.json`,
        // `capture.schema.json#/$defs/uuid`): evaluated against its own root.
        if !reference.starts_with('#') {
            let (file, pointer) = reference.split_once('#').unwrap_or((reference, ""));
            let other = contract(file);
            let target = if pointer.is_empty() {
                other.clone()
            } else {
                other.pointer(pointer).unwrap().clone()
            };
            return errors(&target, value, &other, path);
        }
        return errors(resolve(root, reference), value, root, path);
    }
    let mut out = Vec::new();
    if let Some(branches) = schema.get("anyOf").and_then(Value::as_array)
        && !branches
            .iter()
            .any(|branch| errors(branch, value, root, path).is_empty())
    {
        out.push(format!("{path}: matches no anyOf branch"));
    }
    if let Some(branches) = schema.get("oneOf").and_then(Value::as_array) {
        let matched = branches
            .iter()
            .filter(|branch| errors(branch, value, root, path).is_empty())
            .count();
        if matched != 1 {
            out.push(format!("{path}: matches {matched} oneOf branches"));
        }
    }
    if let Some(constant) = schema.get("const")
        && constant != value
    {
        out.push(format!("{path}: must equal {constant}"));
    }
    if let Some(options) = schema.get("enum").and_then(Value::as_array)
        && !options.contains(value)
    {
        out.push(format!("{path}: {value} is not one of {options:?}"));
    }
    if let Some(kind) = schema.get("type") {
        let kinds: Vec<&str> = match kind {
            Value::String(k) => vec![k.as_str()],
            Value::Array(ks) => ks.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        if !kinds.iter().any(|k| type_matches(value, k)) {
            out.push(format!("{path}: {value} is not of type {kinds:?}"));
            return out;
        }
    }
    if let (Some(minimum), Some(number)) = (
        schema.get("minimum").and_then(Value::as_f64),
        value.as_f64(),
    ) && number < minimum
    {
        out.push(format!("{path}: {number} < {minimum}"));
    }
    if let Some(text) = value.as_str() {
        let length = text.chars().count() as u64;
        if schema
            .get("minLength")
            .and_then(Value::as_u64)
            .is_some_and(|min| length < min)
        {
            out.push(format!("{path}: too short"));
        }
        if schema
            .get("maxLength")
            .and_then(Value::as_u64)
            .is_some_and(|max| length > max)
        {
            out.push(format!("{path}: too long"));
        }
        if schema.get("pattern").and_then(Value::as_str) == Some("^[0-9a-f]{64}$")
            && !(text.len() == 64
                && text
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
        {
            out.push(format!("{path}: not a SHA-256"));
        }
    }
    if let Some(object) = value.as_object() {
        for key in schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !object.contains_key(key) {
                out.push(format!("{path}: missing {key}"));
            }
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
            for key in object.keys() {
                if !properties.is_some_and(|p| p.contains_key(key)) {
                    out.push(format!("{path}: unexpected {key}"));
                }
            }
        }
        for (key, subschema) in properties.into_iter().flatten() {
            if let Some(nested) = object.get(key) {
                out.extend(errors(subschema, nested, root, &format!("{path}.{key}")));
            }
        }
    }
    if let (Some(items), Some(array)) = (schema.get("items"), value.as_array()) {
        for (index, item) in array.iter().enumerate() {
            out.extend(errors(items, item, root, &format!("{path}[{index}]")));
        }
    }
    out
}

/// Asserts `value` is valid against `#/$defs/<def>` of the contract `name`
/// (or the whole document when `def` is empty).
pub fn assert_valid(name: &str, def: &str, value: &Value) {
    let root = contract(name);
    let schema = if def.is_empty() {
        root.clone()
    } else {
        root.pointer(&format!("/$defs/{def}")).unwrap().clone()
    };
    let found = errors(&schema, value, &root, "$");
    assert!(found.is_empty(), "{name}#{def}: {found:#?}");
}

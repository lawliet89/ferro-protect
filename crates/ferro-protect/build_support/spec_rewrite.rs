//! JSON Schema preprocessing for typify model generation.
//!
//! Used by `build.rs` at codegen time. The single public entry point is
//! [`rewrite`]; everything else is private composition.
//!
//! The pipeline is intentionally pure: same input always produces same
//! output, no I/O, no environment reads.

#![allow(clippy::doc_markdown)]

/// Apply the schema-only preprocessing pipeline.
///
/// Pure function: same input always produces same output.
pub fn rewrite(raw: serde_json::Value) -> serde_json::Value {
    let mut value = raw;
    preprocess_for_typify(&mut value);
    value
}

/// Mutate schema constructs typify/schemars cannot consume directly.
///
/// - `const: X` becomes `enum: [X]`
/// - `type: [T, "null"]` becomes `anyOf: [{ "type": T }, { "type": "null" }]`
/// - `allOf: [<single $ref>]` collapses to the referenced schema
/// - collision-prone string enums are relaxed to plain strings
/// - audio-detection enums with known live/spec drift are relaxed to strings
/// - `additionalProperties: false` is removed next to combinators
fn preprocess_for_typify(value: &mut serde_json::Value) {
    descend(value);
}

fn descend(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            expand_nullable_type_array(map);
            if let Some(const_val) = map.remove("const") {
                map.insert(
                    "enum".to_string(),
                    serde_json::Value::Array(vec![const_val]),
                );
            }
            flatten_singleton_all_of(map);
            strip_additional_properties_alongside_combinators(map);
            drop_enum_with_collision_prone_variants(map);
            drop_drifted_audio_detection_enum(map);
            for v in map.values_mut() {
                descend(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                descend(v);
            }
        }
        _ => {}
    }
}

fn expand_nullable_type_array(map: &mut serde_json::Map<String, serde_json::Value>) {
    let Some(serde_json::Value::Array(types)) = map.get("type").cloned() else {
        return;
    };
    if map.contains_key("anyOf") || map.contains_key("oneOf") {
        return;
    }

    let mut variants = Vec::with_capacity(types.len());
    let mut had_null = false;
    for ty in types {
        if ty.as_str() == Some("null") {
            had_null = true;
        }
        variants.push(serde_json::json!({ "type": ty }));
    }
    if had_null {
        map.remove("type");
        map.insert("anyOf".to_string(), serde_json::Value::Array(variants));
    }
}

/// `allOf: [<single $ref>]` is the 3.0-era idiom for "the referenced schema,
/// no modifications". typify chokes when combined with sibling constraints
/// (`additionalProperties: false`, `description`, etc.), so collapse it to a
/// plain `$ref` whenever the only meaningful sibling is `description`.
fn flatten_singleton_all_of(map: &mut serde_json::Map<String, serde_json::Value>) {
    let Some(serde_json::Value::Array(items)) = map.get("allOf") else {
        return;
    };
    if items.len() != 1 {
        return;
    }
    let serde_json::Value::Object(only) = &items[0] else {
        return;
    };
    if only.len() != 1 {
        return;
    }
    let Some(serde_json::Value::String(ref_path)) = only.get("$ref") else {
        return;
    };
    let ref_path = ref_path.clone();
    map.remove("allOf");
    map.remove("additionalProperties");
    map.insert("$ref".to_string(), serde_json::Value::String(ref_path));
}

/// Drop the `enum` constraint when two or more string values would sanitize
/// to the same Rust identifier (typify panics with `Failed to make unique
/// variant names`). Example: `["+", "-", "nc", "no", "com"]` -- both `+`
/// and `-` strip down to the empty string and collide.
///
/// Detecting at the spec level lets the field become a plain `String` in
/// generated code; we lose compile-time exhaustiveness but the field still
/// round-trips whatever the NVR returns.
fn drop_enum_with_collision_prone_variants(map: &mut serde_json::Map<String, serde_json::Value>) {
    let Some(serde_json::Value::Array(values)) = map.get("enum") else {
        return;
    };
    if map.get("type").and_then(|v| v.as_str()) != Some("string") {
        return;
    }
    let mut seen = std::collections::HashSet::new();
    for value in values {
        let Some(s) = value.as_str() else {
            return;
        };
        let sanitized: String = s.chars().filter(char::is_ascii_alphanumeric).collect();
        if !seen.insert(sanitized) {
            map.remove("enum");
            return;
        }
    }
}

/// Some Protect 6.2.x NVRs return `smoke_cmonx` for smart audio detection
/// even though the 6.2.83 spec only lists `alrmCmonx`. Keep the field as a
/// string so live responses remain readable across that spec/runtime drift.
fn drop_drifted_audio_detection_enum(map: &mut serde_json::Map<String, serde_json::Value>) {
    let Some(serde_json::Value::Array(values)) = map.get("enum") else {
        return;
    };
    if values
        .iter()
        .any(|value| value.as_str() == Some("alrmCmonx"))
    {
        map.remove("enum");
    }
}

/// Drop `additionalProperties: false` when the schema also uses a combinator
/// (`allOf`/`oneOf`/`anyOf`). typify's merge logic gives up on this combo and
/// our generated Rust types already enforce a closed shape implicitly.
fn strip_additional_properties_alongside_combinators(
    map: &mut serde_json::Map<String, serde_json::Value>,
) {
    let has_combinator =
        map.contains_key("allOf") || map.contains_key("oneOf") || map.contains_key("anyOf");
    if !has_combinator {
        return;
    }
    if matches!(
        map.get("additionalProperties"),
        Some(serde_json::Value::Bool(false))
    ) {
        map.remove("additionalProperties");
    }
}

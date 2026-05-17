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
    lift_inline_one_or_array_refs(&mut value);
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

/// Real-world Protect NVRs return `smoke_cmonx` for smart audio detection
/// even though the spec only lists `alrmCmonx`. The drift is persisted in
/// the per-camera `smartDetectSettings.audioTypes` user-config field rather
/// than in the `cameraFeatureFlags.smartDetectAudioTypes` capability list,
/// which is why a quick `curl ... | jq '.[].featureFlags.smartDetectAudioTypes'`
/// will *not* surface the problem -- you have to inspect
/// `smartDetectSettings.audioTypes` (or just run the `live_read_cameras_list`
/// integration test against your NVR, which is the authoritative check).
///
/// We can't simply rename `smoke_cmonx` to `alrmCmonx` on the wire because
/// that's a user-set value the NVR round-trips on PATCH. Dropping the enum
/// keeps the field a plain `String`, which serializes/deserializes whatever
/// the NVR happens to use.
///
/// Confirmed still required against firmware 7.1.60 in 2026-05 (see
/// PROGRESS.md "Investigated retiring drop_drifted_audio_detection_enum").
/// If you want to retry retiring this rule on a future firmware: comment
/// out the call site in `descend()`, run
/// `cargo test --features dangerous-tls -p ferro-protect --test live live_read_cameras`
/// against a real NVR, and check that no camera fails with
/// `unknown variant 'smoke_cmonx'`. Look in the cameras of users who have
/// historically configured smart-audio detection -- newly-added cameras
/// may not exhibit the drift.
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

/// Lift inline `anyOf: [{$ref: X}, {type: array, items: {$ref: X}}]` patterns
/// out into named top-level schemas with stable, distinct names.
///
/// Protect 7.1.60 added bulk-operation schemas (`deviceBulkReference` and
/// friends) that express "one entity ID or an array of entity IDs" inline
/// dozens of times. Typify, when generating a name for each inline anyOf,
/// derives it from the inner `$ref` -- producing `ViewerId` from a `viewerId`
/// ref, which then collides with the top-level `ViewerId` typify generates
/// for the `viewerId` schema itself. The result is duplicate definitions and
/// roughly 200 compile errors per spec bump that adds another such schema.
///
/// We sidestep the naming clash by synthesising a top-level
/// `<inner>OrArray` schema (e.g. `viewerIdOrArray`) for each unique inner
/// ref we see, and replacing the inline anyOf with a $ref to it. Each
/// resulting Rust type has a clear single source of truth and a name that
/// does not collide. Synthesis is idempotent: the same inner ref always
/// produces the same lifted schema, even if it appears in fifteen
/// different parent schemas.
fn lift_inline_one_or_array_refs(value: &mut serde_json::Value) {
    let mut synthesised: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    let Some(schemas) = value
        .pointer_mut("/components/schemas")
        .and_then(|v| v.as_object_mut())
    else {
        return;
    };

    // Walk every top-level schema body, replacing inline patterns and
    // recording the names we synthesise as we go.
    for schema_body in schemas.values_mut() {
        lift_descend(schema_body, &mut synthesised);
    }

    // Add synthesised schemas to components.schemas if they don't already
    // exist. We only insert; never overwrite, so a spec that already
    // happens to define `viewerIdOrArray` wins.
    for (name, schema) in synthesised {
        schemas.entry(name).or_insert(schema);
    }
}

fn lift_descend(
    value: &mut serde_json::Value,
    synthesised: &mut serde_json::Map<String, serde_json::Value>,
) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(inner_ref) = match_one_or_array_ref(map) {
                // Derive the schema name from the inner ref's last segment.
                // `#/components/schemas/viewerId` -> `viewerId` -> `viewerIdOrArray`.
                let inner_name = inner_ref
                    .rsplit('/')
                    .next()
                    .unwrap_or(&inner_ref)
                    .to_string();
                let lifted_name = format!("{inner_name}OrArray");
                let lifted_ref = format!("#/components/schemas/{lifted_name}");

                // Record the synthesised schema (idempotent on subsequent hits).
                synthesised
                    .entry(lifted_name)
                    .or_insert_with(|| build_one_or_array_schema(&inner_ref));

                // Replace the inline anyOf with a $ref to the lifted schema.
                map.clear();
                map.insert("$ref".to_string(), serde_json::Value::String(lifted_ref));
                return;
            }
            for v in map.values_mut() {
                lift_descend(v, synthesised);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                lift_descend(v, synthesised);
            }
        }
        _ => {}
    }
}

/// Return `Some(inner_ref)` when `map` is exactly
/// `anyOf: [{$ref: X}, {type: "array", items: {$ref: X}}]`
/// with the two refs identical and no other sibling keys anywhere in
/// the structure.
///
/// The exact-shape check matters: if we accept patterns with extra
/// sibling constraints (e.g. `minItems`, `uniqueItems`, descriptions
/// on either branch), the lifted top-level schema -- which we build
/// fresh from just the inner ref -- would silently drop those
/// constraints and change the spec's semantics. By refusing to match
/// anything but the exact minimal shape, we either lift cleanly or
/// don't lift at all; a future spec that uses a richer variant will
/// surface as a typify naming collision (the original symptom) and
/// can be addressed deliberately.
fn match_one_or_array_ref(map: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    // Outer object: must be exactly `{"anyOf": [...]}`.
    if map.len() != 1 {
        return None;
    }
    let serde_json::Value::Array(items) = map.get("anyOf")? else {
        return None;
    };
    if items.len() != 2 {
        return None;
    }

    // Branch 0: must be exactly `{"$ref": "..."}`.
    let single_branch = items[0].as_object()?;
    if single_branch.len() != 1 {
        return None;
    }
    let single_ref = single_branch.get("$ref")?.as_str()?;

    // Branch 1: must be exactly `{"type": "array", "items": {"$ref": "..."}}`.
    let array_branch = items[1].as_object()?;
    if array_branch.len() != 2 {
        return None;
    }
    if array_branch.get("type")?.as_str()? != "array" {
        return None;
    }
    let array_items = array_branch.get("items")?.as_object()?;
    if array_items.len() != 1 {
        return None;
    }
    let array_inner_ref = array_items.get("$ref")?.as_str()?;

    if single_ref != array_inner_ref {
        return None;
    }
    Some(single_ref.to_string())
}

fn build_one_or_array_schema(inner_ref: &str) -> serde_json::Value {
    serde_json::json!({
        "anyOf": [
            { "$ref": inner_ref },
            { "type": "array", "items": { "$ref": inner_ref } }
        ]
    })
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

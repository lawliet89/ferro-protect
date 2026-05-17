//! OpenAPI 3.1 -> 3.0 + progenitor-friendly rewrite pipeline.
//!
//! Used by `build.rs` at codegen time and by the snapshot test in
//! `tests/spec_rewrite_snapshot.rs` to detect changes in the pipeline's
//! output. The single public entry point is [`rewrite`]; everything else is
//! private composition.
//!
//! The pipeline is intentionally pure: same input always produces same
//! output, no I/O, no environment reads.

#![allow(clippy::doc_markdown)]

const HTTP_METHODS: &[&str] = &[
    "get", "put", "post", "delete", "patch", "head", "options", "trace",
];

/// Apply the full 3.1 -> 3.0 + progenitor-friendly rewrite pipeline.
///
/// Pure function: same input always produces same output.
pub fn rewrite(raw: serde_json::Value) -> serde_json::Value {
    let mut value = raw;
    convert_31_to_30(&mut value);
    value
}

/// Mutate a parsed OpenAPI 3.1 document into something the `openapiv3` crate
/// (which targets 3.0) accepts. The conversions applied:
///
/// - top-level `openapi` field bumped to `3.0.3`
/// - `type: [X, "null"]` shorthand becomes `type: X` + `nullable: true`
/// - `const: X` becomes `enum: [X]`
/// - numeric `exclusiveMinimum` / `exclusiveMaximum` (3.1) become the 3.0
///   `minimum` + `exclusiveMinimum: true` / `maximum` + `exclusiveMaximum: true`
/// - top-level `webhooks` field is removed (3.0 has no equivalent)
/// - operations without `operationId` get a synthesized one derived from method
///   and path (the Protect spec ships without them and progenitor refuses to
///   generate code otherwise)
fn convert_31_to_30(value: &mut serde_json::Value) {
    if let serde_json::Value::Object(map) = value {
        if let Some(serde_json::Value::String(v)) = map.get_mut("openapi") {
            if v.starts_with("3.1") {
                *v = "3.0.3".to_string();
            }
        }
        map.remove("webhooks");
        synthesize_operation_ids(map);
        sanitize_non_json_payloads(map);
    }
    descend(value);
}

/// Strip content types that progenitor's typify backend can't generate code
/// for. The affected operations are hand-implemented in their respective
/// phases (snapshot in phase 7, file upload in phase 8) using the raw HTTP
/// client. Removing the body here lets progenitor still emit an operation
/// method we can reuse, or omit it cleanly.
fn sanitize_non_json_payloads(root: &mut serde_json::Map<String, serde_json::Value>) {
    let Some(serde_json::Value::Object(paths)) = root.get_mut("paths") else {
        return;
    };
    let keys: Vec<String> = paths.keys().cloned().collect();
    for path in keys {
        let Some(serde_json::Value::Object(item)) = paths.get_mut(&path) else {
            continue;
        };
        for method in HTTP_METHODS {
            let Some(serde_json::Value::Object(op)) = item.get_mut(*method) else {
                continue;
            };
            if let Some(serde_json::Value::Object(body)) = op.get_mut("requestBody") {
                if let Some(serde_json::Value::Object(content)) = body.get_mut("content") {
                    content.remove("multipart/form-data");
                    if content.is_empty() {
                        op.remove("requestBody");
                    }
                }
            }
            if let Some(serde_json::Value::Object(responses)) = op.get_mut("responses") {
                for resp in responses.values_mut() {
                    let serde_json::Value::Object(resp_obj) = resp else {
                        continue;
                    };
                    if let Some(serde_json::Value::Object(content)) = resp_obj.get_mut("content") {
                        if let Some(image) = content.remove("image/jpeg") {
                            content.insert("application/octet-stream".to_string(), image);
                        }
                    }
                    // OpenAPI 3.0 requires `description` on every Response
                    // object; some upstream specs (e.g. 7.1.60 POST
                    // /v1/arm-profiles) ship 201 responses without one and
                    // openapiv3 refuses to parse them. Inject a placeholder
                    // so codegen proceeds; the value is never visible to
                    // library consumers.
                    resp_obj.entry("description").or_insert_with(|| {
                        serde_json::Value::String("(no description in spec)".to_string())
                    });
                }
                // The Protect spec uses `default` for the error response of
                // most operations. Progenitor counts `default` as a possible
                // success type and trips its "one success type" assertion,
                // so we don't want to leave it as-is. Two cases:
                //
                // 1. Op has no explicit 4xx/5xx code: rename `default` to
                //    `4XX` (valid 3.0 range) so progenitor classifies it as
                //    error-only and our `Error::Api` branch still gets the
                //    typed body.
                //
                // 2. Op already has an explicit 4xx/5xx code (new in 7.x):
                //    drop `default` entirely. Keeping it would give the op
                //    two different error body schemas in the same bucket,
                //    which trips progenitor's
                //    `response_types.len() <= 1` assertion. The hand-written
                //    `Error::from_progenitor` adaptor handles unknown body
                //    shapes anyway via its serde_json::Value probe.
                if responses.contains_key("default") {
                    let has_explicit_error = responses.keys().any(|k| {
                        let bytes = k.as_bytes();
                        !bytes.is_empty() && (bytes[0] == b'4' || bytes[0] == b'5')
                    });
                    if has_explicit_error {
                        responses.remove("default");
                    } else if let Some(default_resp) = responses.remove("default") {
                        responses.entry("4XX").or_insert(default_resp);
                    }
                }
            }
        }
    }
}

fn synthesize_operation_ids(root: &mut serde_json::Map<String, serde_json::Value>) {
    let Some(serde_json::Value::Object(paths)) = root.get_mut("paths") else {
        return;
    };
    let path_items: Vec<String> = paths.keys().cloned().collect();
    for path in path_items {
        let Some(serde_json::Value::Object(item)) = paths.get_mut(&path) else {
            continue;
        };
        for method in HTTP_METHODS {
            let Some(serde_json::Value::Object(op)) = item.get_mut(*method) else {
                continue;
            };
            if op.contains_key("operationId") {
                continue;
            }
            let id = operation_id_from(method, &path);
            op.insert("operationId".to_string(), serde_json::Value::String(id));
        }
    }
}

fn operation_id_from(method: &str, path: &str) -> String {
    let mut out = String::with_capacity(path.len() + method.len() + 1);
    out.push_str(method);
    for segment in path.split('/').filter(|s| !s.is_empty() && *s != "v1") {
        out.push('_');
        for ch in segment.chars() {
            if ch.is_ascii_alphanumeric() {
                out.push(ch.to_ascii_lowercase());
            } else if ch == '-' || ch == '_' {
                out.push('_');
            }
            // strip { and } from path-params; other chars dropped silently
        }
    }
    out
}

fn descend(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::Array(types)) = map.get("type").cloned() {
                let mut non_null = Vec::with_capacity(types.len());
                let mut has_null = false;
                for t in &types {
                    if t.as_str() == Some("null") {
                        has_null = true;
                    } else {
                        non_null.push(t.clone());
                    }
                }
                if non_null.len() == 1 {
                    map.insert("type".to_string(), non_null.into_iter().next().unwrap());
                    if has_null {
                        map.insert("nullable".to_string(), serde_json::Value::Bool(true));
                    }
                }
            }
            if let Some(const_val) = map.remove("const") {
                map.insert(
                    "enum".to_string(),
                    serde_json::Value::Array(vec![const_val]),
                );
            }
            if let Some(ex) = map.get("exclusiveMinimum").cloned() {
                if ex.is_number() {
                    map.insert("minimum".to_string(), ex);
                    map.insert(
                        "exclusiveMinimum".to_string(),
                        serde_json::Value::Bool(true),
                    );
                }
            }
            if let Some(ex) = map.get("exclusiveMaximum").cloned() {
                if ex.is_number() {
                    map.insert("maximum".to_string(), ex);
                    map.insert(
                        "exclusiveMaximum".to_string(),
                        serde_json::Value::Bool(true),
                    );
                }
            }
            collapse_nullable_combinator(map, "oneOf");
            collapse_nullable_combinator(map, "anyOf");
            flatten_singleton_all_of(map);
            strip_additional_properties_alongside_combinators(map);
            drop_enum_with_collision_prone_variants(map);
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

/// 3.1 expresses "nullable T" as `oneOf: [<T>, {type: "null"}]`. 3.0 has no
/// `type: "null"` so we rewrite to the parent's `nullable: true` instead, and
/// either drop the combinator (if only one non-null variant remains) or strip
/// the null variant in place (multi-variant unions stay).
fn collapse_nullable_combinator(map: &mut serde_json::Map<String, serde_json::Value>, key: &str) {
    let Some(serde_json::Value::Array(variants)) = map.get(key).cloned() else {
        return;
    };
    let mut non_null: Vec<serde_json::Value> = Vec::with_capacity(variants.len());
    let mut had_null = false;
    for v in variants {
        if let serde_json::Value::Object(ref obj) = v {
            if obj.len() == 1 && obj.get("type").and_then(|t| t.as_str()) == Some("null") {
                had_null = true;
                continue;
            }
        }
        non_null.push(v);
    }
    if !had_null {
        return;
    }
    map.insert("nullable".to_string(), serde_json::Value::Bool(true));
    if non_null.len() == 1 {
        let only = non_null.into_iter().next().unwrap();
        map.remove(key);
        if let serde_json::Value::Object(only_obj) = only {
            for (k, v) in only_obj {
                map.entry(k).or_insert(v);
            }
        }
    } else {
        map.insert(key.to_string(), serde_json::Value::Array(non_null));
    }
}

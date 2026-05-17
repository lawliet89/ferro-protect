//! Build-time codegen for the UniFi Protect OpenAPI client.
//!
//! Bumping the spec version is a one-line change here -- update `SPEC_VERSION`
//! (or run `scripts/update-spec` from the repo root, which edits this file
//! mechanically and re-runs the full check suite).
#![allow(clippy::doc_markdown)]

use std::path::PathBuf;

const SPEC_VERSION: &str = "6.2.83";
const SPEC_DIR: &str = "../../third_party/unifi-apis/unifi-protect";
const HTTP_METHODS: &[&str] = &[
    "get", "put", "post", "delete", "patch", "head", "options", "trace",
];

type BuildResult<T> = Result<T, Box<dyn std::error::Error>>;

fn main() {
    if let Err(e) = run() {
        eprintln!("\nferro-protect codegen failed: {e}\n");
        eprintln!("See UPGRADING.md for triage steps.\n");
        std::process::exit(1);
    }
}

fn run() -> BuildResult<()> {
    let spec_path = PathBuf::from(SPEC_DIR).join(format!("{SPEC_VERSION}.json"));
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", spec_path.display());

    let raw = std::fs::read_to_string(&spec_path).map_err(|e| {
        format!(
            "Could not read spec at {}: {e}\n  Did you run `git submodule update --init --recursive`?",
            spec_path.display()
        )
    })?;

    let mut spec_value: serde_json::Value = serde_json::from_str(&raw)?;
    convert_31_to_30(&mut spec_value);

    let spec: openapiv3::OpenAPI = serde_json::from_value(spec_value)
        .map_err(|e| format!("Spec failed to parse as OpenAPI 3.0 after down-conversion: {e}"))?;

    let mut generator = progenitor::Generator::default();
    let tokens = generator
        .generate_tokens(&spec)
        .map_err(|e| format!("progenitor codegen error: {e}"))?;
    let ast =
        syn::parse2(tokens).map_err(|e| format!("syn could not parse generated tokens: {e}"))?;
    let content = prettyplease::unparse(&ast);

    let out_dir = std::env::var("OUT_DIR")?;
    let out_path = PathBuf::from(out_dir).join("generated.rs");
    std::fs::write(&out_path, content)?;

    Ok(())
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
    rewrite(value);
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
                }
                // The Protect spec uses `default` for the error response of every
                // operation. Progenitor counts `default` as a possible success
                // type and trips its "one success type" assertion. Rename to
                // `4XX` (a valid 3.0 range) so progenitor classifies it as
                // error-only and our `Error::Api` branch still gets the typed
                // body via the generated error type.
                if let Some(default_resp) = responses.remove("default") {
                    responses.entry("4XX").or_insert(default_resp);
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

fn rewrite(value: &mut serde_json::Value) {
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
            for v in map.values_mut() {
                rewrite(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                rewrite(v);
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

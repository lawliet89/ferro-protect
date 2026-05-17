//! Build-time codegen for the UniFi Protect OpenAPI client.
//!
//! Bumping the spec version is a one-line change here -- update `SPEC_VERSION`
//! (or run `scripts/update-spec` from the repo root, which edits this file
//! mechanically and re-runs the full check suite).
#![allow(clippy::doc_markdown)]

use std::path::PathBuf;

#[path = "build_support/spec_rewrite.rs"]
mod spec_rewrite;

const SPEC_VERSION: &str = "6.2.83";
const SPEC_DIR: &str = "../../third_party/unifi-apis/unifi-protect";

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
    println!("cargo:rerun-if-changed=build_support/spec_rewrite.rs");
    println!("cargo:rerun-if-changed={}", spec_path.display());

    let raw = std::fs::read_to_string(&spec_path).map_err(|e| {
        format!(
            "Could not read spec at {}: {e}\n  Did you run `git submodule update --init --recursive`?",
            spec_path.display()
        )
    })?;

    let raw_value: serde_json::Value = serde_json::from_str(&raw)?;
    let rewritten = spec_rewrite::rewrite(raw_value);

    let spec: openapiv3::OpenAPI = serde_json::from_value(rewritten)
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

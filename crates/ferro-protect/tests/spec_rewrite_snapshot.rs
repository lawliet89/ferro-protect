#![forbid(unsafe_code)]
#![allow(clippy::pedantic, clippy::nursery)]

//! Snapshot the output of the OpenAPI spec rewrite pipeline so any change in
//! either the pipeline or the input spec turns into a reviewable git diff.
//! See `UPGRADING.md` ("When the snapshot test fails") for the review flow.

#[path = "../build_support/spec_rewrite.rs"]
mod spec_rewrite;

const SPEC_PATH: &str = "../../third_party/unifi-apis/unifi-protect/6.2.83.json";

#[test]
fn rewrite_output_matches_snapshot() {
    let raw_text = std::fs::read_to_string(SPEC_PATH)
        .expect("spec file present (run `git submodule update --init`)");
    let raw: serde_json::Value = serde_json::from_str(&raw_text).expect("spec is valid JSON");
    let rewritten = spec_rewrite::rewrite(raw);
    insta::assert_json_snapshot!(rewritten);
}

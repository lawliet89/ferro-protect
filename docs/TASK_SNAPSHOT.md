# Task: snapshot-test the OpenAPI rewrite pipeline

## Why

`build.rs` rewrites the OpenAPI 3.1 spec into a 3.0-compatible form before handing it to progenitor (3.1 → 3.0 nullables, synthesized operationIds, `default` → `4XX`, `allOf` flattening, `additionalProperties` stripping, multipart/image content-type rewrites). Today, if the rewrites produce wrong output, we find out either:
- via a build panic (loud, easy), or
- via subtly broken generated Rust code at runtime (silent, painful).

A snapshot test of the rewritten spec turns the second case into a loud `git diff`. Whenever the rewrites or the input spec change, the test fails and shows exactly what changed in the output. A maintainer then either accepts the diff (intentional change) or fixes the code (regression).

Scope: one new test file, one extracted module, one dev-dependency. No behavior change in `build.rs`.

## Action items

### 1. Extract the rewrite pipeline into a shared module

Move the rewrite functions currently inlined in `build.rs` into a new file:

```
crates/ferro-protect/build_support/spec_rewrite.rs
```

Expose a single public entry point:

```rust
/// Apply the full 3.1 → 3.0 + progenitor-friendly rewrite pipeline.
/// Pure function: same input always produces same output.
pub fn rewrite(raw: serde_json::Value) -> serde_json::Value { /* ... */ }
```

All existing internal rewrite steps (`convert_31_to_30`, `synthesize_operation_ids`, etc.) stay as private helpers inside this module. `rewrite()` composes them in the order `build.rs` currently uses.

### 2. Wire `build.rs` to use the extracted module

Replace the inlined rewrite logic in `build.rs` with:

```rust
#[path = "build_support/spec_rewrite.rs"]
mod spec_rewrite;

// ... later, where the pipeline used to run inline:
let rewritten = spec_rewrite::rewrite(raw);
```

`build.rs` must behave identically to before this change. Generated code in `$OUT_DIR/generated.rs` must be byte-identical. Verify by hashing it before and after.

### 3. Add `insta` as a dev-dependency

In `crates/ferro-protect/Cargo.toml`:

```toml
[dev-dependencies]
insta = { version = "1", features = ["json"] }
```

(Use whatever current `1.x` is at time of work; the `json` feature is needed for `assert_json_snapshot!`.)

### 4. Create the snapshot test

New file: `crates/ferro-protect/tests/spec_rewrite_snapshot.rs`

```rust
#![forbid(unsafe_code)]

// Re-use the same rewrite module that build.rs uses.
#[path = "../build_support/spec_rewrite.rs"]
mod spec_rewrite;

const SPEC_PATH: &str = "../../third_party/unifi-apis/unifi-protect/6.2.83.json";

#[test]
fn rewrite_output_matches_snapshot() {
    let raw_text = std::fs::read_to_string(SPEC_PATH)
        .expect("spec file present (run `git submodule update --init`)");
    let raw: serde_json::Value =
        serde_json::from_str(&raw_text).expect("spec is valid JSON");
    let rewritten = spec_rewrite::rewrite(raw);
    insta::assert_json_snapshot!(rewritten);
}
```

### 5. Generate the initial snapshot

Run once:

```
INSTA_UPDATE=auto cargo test --package ferro-protect rewrite_output_matches_snapshot
```

This creates the snapshot file:

```
crates/ferro-protect/tests/snapshots/spec_rewrite_snapshot__rewrite_output_matches_snapshot.snap
```

**Commit the snapshot file.** It is the test fixture; without it the test cannot pass.

### 6. Document the workflow in `UPGRADING.md`

Append a short section:

> ### When the snapshot test fails
>
> If `cargo test` reports `rewrite_output_matches_snapshot` failed, the spec rewrite pipeline produced different output than last time. This is expected when you bump the spec submodule or change a rewrite rule. Steps:
>
> 1. Run `cargo insta review` and inspect the diff.
> 2. **Read the diff carefully.** Changes you expected (e.g., new endpoints from a spec bump) are fine to accept. Changes you did *not* expect — new shapes you've never seen, lost fields, unexpected `oneOf` collapses — indicate the spec introduced a construct the rewrite pipeline doesn't fully handle. Fix the rewrite first, then re-run.
> 3. Accept the new snapshot with `cargo insta accept` (or the equivalent in `cargo insta review`).
> 4. Commit the updated `.snap` file alongside the spec bump.

### 7. Verify guardrails

Before committing:

- `cargo fmt --all -- --check` passes.
- `cargo clippy --all-targets --all-features -- -D warnings` passes.
- `cargo test --all` passes (including the new snapshot test).
- `cargo deny check` passes (`insta` is permissively licensed; no allow-list change expected).

### 8. PROGRESS.md entry and commit

Append a PROGRESS.md entry summarizing what landed. Commit message convention (consistent with prior commits):

```
chore(spec): add snapshot test for the rewrite pipeline
```

Body should mention: the goal (tripwire on rewrite output changes), the new module location, the new test, and a one-line pointer to the UPGRADING.md section that explains the review workflow.

## Acceptance criteria

- `build.rs` produces byte-identical `$OUT_DIR/generated.rs` to before this change.
- New test exists and passes.
- Snapshot file is committed.
- Deliberately mutating a rewrite rule (locally, not committed) causes the test to fail with a readable diff.
- UPGRADING.md tells future maintainers how to handle a failing snapshot.
- All guardrails (fmt, clippy, test, deny) green.

## Out of scope

- Summarizing the rewrite output into a smaller, more legible snapshot (e.g., "which rules fired and where"). Could be added later if the full-spec snapshot proves too noisy in PR reviews. For now the full snapshot is the simplest tripwire and matches insta's idiomatic usage.
- Snapshotting the *generated Rust code* (i.e., `$OUT_DIR/generated.rs`). That would also be useful but is a separate, larger task with different ergonomics — defer.

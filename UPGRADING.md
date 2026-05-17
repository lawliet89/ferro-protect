# Upgrading the UniFi Protect spec version

ferro-protect pins one OpenAPI spec version at a time. The version is the
`SPEC_VERSION` constant in [`crates/ferro-protect/build.rs`](crates/ferro-protect/build.rs).
Bumping the spec re-runs progenitor codegen against the new schema and surfaces
any breaking API changes at compile time. This document covers the happy path
and the things that go wrong.

## Happy path

```sh
./scripts/update-spec               # lists versions available in the submodule
./scripts/update-spec 7.1.60        # bumps + regenerates + runs the full check suite
```

The script:

1. Fetches the latest commit of `third_party/unifi-apis` and checks out
   `origin/HEAD`. (Pin a different commit by `git -C third_party/unifi-apis
   checkout <sha>` before running.)
2. Rewrites the `SPEC_VERSION` line in `build.rs`.
3. Touches `build.rs` so cargo re-runs the build script.
4. Runs `cargo build`, `cargo test --all`, `cargo clippy --all-targets
   --all-features -- -D warnings`, `cargo deny check`.
5. Prints the new submodule SHA and the suggested `git add` + `git commit`.

If every step passes, commit the changes.

## When codegen fails

Failures from `cargo build` look like `ferro-protect codegen failed: ...`. The
build script applies a handful of spec rewrites to make a 3.1 document parse
as 3.0 and to placate progenitor's typify backend:

- top-level `openapi` -> `3.0.3`
- `type: [X, "null"]` shorthand -> `type: X` + `nullable: true`
- `const: X` -> `enum: [X]`
- numeric `exclusiveMinimum`/`exclusiveMaximum` -> 3.0 boolean form
- `oneOf: [<T>, {type: "null"}]` -> `nullable: true` on the parent
- `allOf: [<single $ref>]` -> direct `$ref`
- strip `additionalProperties: false` whenever a combinator is in scope
- synthesize `operationId` for every operation
- rename `default` response to `4XX` (progenitor would otherwise count it as a
  success type)
- drop `multipart/form-data` request bodies and convert `image/jpeg` responses
  to `application/octet-stream` (these endpoints are hand-implemented)

If a new spec version introduces a 3.1 construct we don't handle yet, extend
the `rewrite` function in `build.rs`. The plan's phase 1 fallback options
(more aggressive preprocessing, openapi-generator-cli, or hand-rolled types
for the specific operation) are escape hatches if progenitor itself remains
hostile.

To inspect what the generator actually saw, the generated source lives at
`target/debug/build/ferro-protect-*/out/generated.rs`. Diff against the prior
build by checking out the previous `build.rs`, rebuilding, and comparing.

## When wrappers fail to compile

Look in this order:

1. **`crates/ferro-protect/src/models.rs`** -- this is the seam between
   generated code and the rest of the library. If a type was renamed or moved
   by the new spec, the re-export here breaks first. Fix the re-export.
2. **The specific entity module** (`client.rs`, `cameras.rs`, etc.). Wrappers
   delegate to generated methods named after `operationId` (synthesized as
   `<method>_<path_segments>`). If a path changed shape, the generated method
   name changes too.
3. **`crates/ferro-protect/tests/public_api.rs`** -- the snapshot test that
   touches every public type. If a `models::*` re-export disappears, this
   test fails, pointing you at the right type.

The library never names `crate::generated::...` types in public signatures.
If you find yourself reaching into `generated` from a wrapper, stop -- add
the type to `models.rs` first.

## Reading the generated diff

```sh
# from a clean working tree, after a successful build
cp target/debug/build/ferro-protect-*/out/generated.rs /tmp/generated.before
./scripts/update-spec <new-version>
cp target/debug/build/ferro-protect-*/out/generated.rs /tmp/generated.after
diff -u /tmp/generated.before /tmp/generated.after | less
```

## When the snapshot test fails

If `cargo test` reports `rewrite_output_matches_snapshot` failed, the spec
rewrite pipeline produced different output than last time. This is expected
when you bump the spec submodule or change a rewrite rule. Steps:

1. Run `cargo insta review` and inspect the diff.
2. **Read the diff carefully.** Changes you expected (e.g., new endpoints
   from a spec bump) are fine to accept. Changes you did *not* expect — new
   shapes you've never seen, lost fields, unexpected `oneOf` collapses —
   indicate the spec introduced a construct the rewrite pipeline doesn't
   fully handle. Fix the rewrite first, then re-run.
3. Accept the new snapshot with `cargo insta accept` (or the equivalent in
   `cargo insta review`).
4. Commit the updated `.snap` file alongside the spec bump.

## Agent checklist

A coding agent should be able to upgrade by running, in order:

1. Read this file.
2. Run `./scripts/update-spec` (no args). Note the available versions.
3. Pick the target version (newer minor than current; one step at a time).
4. Run `./scripts/update-spec <version>`. If it exits 0, commit. Done.
5. If `cargo build` fails: classify by the failure stage:
   - Build script failure: a new spec construct needs a `rewrite` rule in
     `build.rs`. Add it, re-run from step 4.
   - typify/progenitor panic: search this file and `build.rs` for prior
     workarounds; if novel, add a rewrite or hand-roll the affected
     operation.
6. If `cargo test --all` or `cargo clippy` fails: read the failing message,
   start at the file mentioned, work outward to `models.rs` and the entity
   wrappers. Do not edit anything under `generated.rs` -- it includes the
   regenerated file from `OUT_DIR`.
7. If `cargo deny check` fails: a new dependency landed (often through
   progenitor itself). Update `deny.toml`'s license allow list with reason.

Do not bypass signing, do not edit `target/`, do not amend prior commits.

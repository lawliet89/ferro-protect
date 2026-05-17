# Upgrading the UniFi Protect spec version

ferro-protect pins one OpenAPI spec version at a time. The version is the
`SPEC_VERSION` constant in [`crates/ferro-protect/build.rs`](crates/ferro-protect/build.rs).
Bumping the spec re-runs typify model codegen against `components.schemas` and
surfaces breaking model or wrapper changes at compile time. This document covers the happy path
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
build script parses the OpenAPI file as JSON, extracts `components.schemas`,
and applies a small schema-only preprocessing pass before handing the schemas
to typify.

Current preprocessing:

- `const: X` -> `enum: [X]`
- `type: [T, "null"]` -> `anyOf: [{ "type": T }, { "type": "null" }]`
- `allOf: [<single $ref>]` -> direct `$ref`
- strip `additionalProperties: false` whenever a combinator is in scope
- relax string enums whose variants collide after Rust identifier sanitizing
- relax the smart-audio detection enum because live 6.2.x devices can return
  values missing from the published 6.2.83 spec

If typify fails on a new schema construct, prefer a narrow preprocessing rule
in `build_support/spec_rewrite.rs`. If only one schema is problematic and the
rewrite would be broad or lossy, hand-write that type in `models.rs` and skip
that schema during codegen with a small allowlist in `build.rs`.

To inspect what the generator actually saw, the generated source lives at
`target/debug/build/ferro-protect-*/out/generated.rs`. Diff against the prior
build by checking out the previous `build.rs`, rebuilding, and comparing.

## When wrappers fail to compile

Look in this order:

1. **`crates/ferro-protect/src/models.rs`** -- this is the seam between
   generated code and the rest of the library. If a type was renamed or moved
   by the new spec, the re-export here breaks first. Fix the re-export.
2. **The specific entity module** (`client.rs`, `cameras.rs`, etc.). Wrappers
   delegate to shared HTTP helpers and return `models::*`. If a path changed
   shape, update the path string in the wrapper.
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

## When the model smoke test fails

If `cargo test` reports `model_codegen` failed, the public model seam no
longer matches what the library expects. This is expected when a spec bump
renames or reshapes a generated type. Steps:

1. Read the compile error or assertion failure and identify the missing or
   changed type.
2. Start in `models.rs`; update re-exports or hand-written inline response
   types there.
3. If typify generated the wrong shape, fix the smallest preprocessing rule in
   `build_support/spec_rewrite.rs` and rebuild.
4. Commit the test update alongside the spec bump.

## Agent checklist

A coding agent should be able to upgrade by running, in order:

1. Read this file.
2. Run `./scripts/update-spec` (no args). Note the available versions.
3. Pick the target version (newer minor than current; one step at a time).
4. Run `./scripts/update-spec <version>`. If it exits 0, commit. Done.
5. If `cargo build` fails: classify by the failure stage:
   - Build script failure: a new spec construct needs a `rewrite` rule in
     `build.rs`. Add it, re-run from step 4.
   - typify failure: search this file and `build_support/spec_rewrite.rs` for
     prior workarounds; if novel, add a rewrite or hand-roll the affected type.
6. If `cargo test --all` or `cargo clippy` fails: read the failing message,
   start at the file mentioned, work outward to `models.rs` and the entity
   wrappers. Do not edit anything under `generated.rs` -- it includes the
   regenerated file from `OUT_DIR`.
7. If `cargo deny check` fails: a new dependency landed. Update `deny.toml`'s
   license allow list with reason.

Do not bypass signing, do not edit `target/`, do not amend prior commits.

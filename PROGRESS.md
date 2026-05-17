# Build progress log

Per PLAN.md, the entry for phase N is committed at the start of phase N+1, so
this file traverses one phase behind the work in each commit it appears in.

## 2026-05-17 10:53 +0800 — Phase 0: workspace skeleton, lints, CI, submodule

**Status**: complete

**Summary**:
Bootstrapped the Cargo workspace with two member crates (`ferro-protect`
library + `ferro-protect-cli` binary, binary name `ferro-protect`), the
stable toolchain pin, rustfmt/clippy (pedantic + nursery)/cargo-deny
configuration, a GitHub Actions CI pipeline, and a local pre-commit hook.
Registered the OpenAPI spec submodule (`beezly/unifi-apis`, pinned at
`b9b8488`). All gates green before commit. Also extended PLAN.md with a
"Forward-compatibility with spec upgrades" section after a user prompt; the
plan now mandates a single `SPEC_VERSION` constant, a `models.rs` re-export
seam, `scripts/update-spec` for one-command bumps, `UPGRADING.md`, and a
public-API snapshot test.

**Files added/changed**:
- `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `rustfmt.toml`,
  `deny.toml`, `.gitignore`, `.gitmodules`
- `crates/ferro-protect/{Cargo.toml,src/lib.rs}`
- `crates/ferro-protect-cli/{Cargo.toml,src/main.rs}`
- `.github/workflows/ci.yml`, `scripts/pre-commit`
- `README.md`, `CHANGELOG.md`, `PLAN.md`
- `third_party/unifi-apis` submodule pointer

**Decisions / deviations**:
- The plan said the spec path was
  `third_party/unifi-apis/ferro-protect/{ver}.json`. The actual upstream
  folder is `unifi-protect/`. Using the real path; logged here for future
  agents.
- gpg-agent's TTY-bound pinentry could not be served through the harness's
  session; the user warmed the cache manually before the commit landed. PLAN
  signing rule was honoured (no `--no-gpg-sign`).

**Next**: Phase 1 — progenitor codegen pipeline.

## 2026-05-17 11:05 +0800 — Phase 1: codegen pipeline

**Status**: complete

**Summary**:
The library now generates ~16,700 lines of Rust from
`third_party/unifi-apis/unifi-protect/6.2.83.json` on every build. `build.rs`
parses the spec as `serde_json::Value`, applies a series of OpenAPI 3.1 ->
3.0 rewrites (so the `openapiv3` crate can parse it) plus a handful of
progenitor-friendly normalizations, then feeds the result to progenitor and
writes `$OUT_DIR/generated.rs`. `src/generated.rs` is a one-line `include!`
with a permissive `#![allow(...)]` so generated code never blocks our
hand-written clippy gate. Added `scripts/update-spec` (one-command spec
bump + check suite) and `UPGRADING.md` (procedure for humans and agents,
plus an agent-only checklist). All four gates pass.

**Files added/changed**:
- `Cargo.toml` (added `openapiv3` to workspace deps)
- `crates/ferro-protect/Cargo.toml` (added build.rs, runtime + build deps)
- `crates/ferro-protect/build.rs` (new)
- `crates/ferro-protect/src/generated.rs` (new)
- `crates/ferro-protect/src/lib.rs` (added `pub(crate) mod generated;`)
- `scripts/update-spec` (new, executable)
- `UPGRADING.md` (new)
- `deny.toml` (added CDLA-Permissive-2.0 for webpki-roots)

**Decisions / deviations**:
- **Why not `oas3` for parsing?** The user asked whether `oas3` (which natively
  supports OpenAPI 3.1) could replace the in-build conversion. It can't, by
  itself, because the bottleneck isn't parsing -- it's progenitor. Progenitor
  0.10's `Generator::generate_tokens` takes `&openapiv3::OpenAPI` (3.0 only),
  and there is no 3.1-aware Rust codegen crate at maturity comparable to
  progenitor. Even if we parsed with `oas3`, we would still have to translate
  to `openapiv3` types to feed progenitor, which means doing the same 3.1 ->
  3.0 conversions one layer deeper. Doing the conversion on the raw
  `serde_json::Value` in `build.rs` is the least magical option: every
  rewrite rule is one short function, the panic site of any new spec feature
  points us straight at the function to extend (see `UPGRADING.md`), and we
  do not take on `oas3` as an additional dependency we would still have to
  bridge. If progenitor later gains 3.1 input or if a 3.1-native generator
  matures, we can revisit; until then the rewrite list lives in
  `build.rs::convert_31_to_30` + `rewrite`.
- The Protect spec ships *every* operation without `operationId`, and
  progenitor refuses to generate without one. Synthesizing IDs of the form
  `{method}_{path_segments_lowercased}` in `build.rs` (e.g. `get_meta_info`,
  `post_cameras_id_ptz_goto_slot`). These are stable across spec versions
  given a stable path/method, which is exactly the property we want.
- The spec uses `default` for every error response, which progenitor
  classifies as a success-shape and trips its
  `response_types.len() <= 1` assertion. Renamed to `4XX` (a valid 3.0
  range), which progenitor treats as error-only.
- typify panics on `allOf: [<single $ref>] + additionalProperties: false`
  ("this is fairly fussy and I don't want to do it"). Flattened those
  schemas to a bare `$ref`. Also stripped `additionalProperties: false` from
  any schema that has a combinator (`allOf`/`oneOf`/`anyOf`); the Rust
  structs typify generates already enforce closed shapes.
- Stripped `multipart/form-data` request bodies (only on `POST /v1/files/{fileType}`,
  phase 8) and rewrote `image/jpeg` responses (only on `GET /v1/cameras/{id}/snapshot`,
  phase 7) to `application/octet-stream`. Both endpoints were already slated
  to bypass progenitor's auto-deserialization per the plan; this just lets
  progenitor still emit a typed method we can build on.
- A `4XX -> 3XX/2XX` collision risk does not arise because the spec only
  defines 2xx and `default` responses for every operation.
- `webpki-roots-1.0.7` brought in CDLA-Permissive-2.0 (a permissive
  data-sharing license, used only for the bundled Mozilla CA list). Added
  to `deny.toml`'s allow list with a justifying comment.

**Next**: Phase 2 — info endpoint end-to-end, plus a live-NVR integration
test scaffold the user asked for (URL/API key supplied via env vars or a
gitignored config file).

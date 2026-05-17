# Build progress log

Entries are listed in chronological order, oldest first. Per PLAN.md, the
entry for phase N is committed at the start of phase N+1, so this file
traverses one phase behind the work in each commit it appears in.

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

## 2026-05-17 11:20 +0800 — Chore: snapshot test for the spec rewrite pipeline

**Status**: complete

**Summary**:
Implemented [docs/TASK_SNAPSHOT.md](docs/TASK_SNAPSHOT.md). Extracted the
rewrite functions out of `build.rs` into
`crates/ferro-protect/build_support/spec_rewrite.rs` behind a single
`pub fn rewrite(serde_json::Value) -> serde_json::Value` entry point;
`build.rs` now does `#[path = "build_support/spec_rewrite.rs"] mod
spec_rewrite;` and calls `spec_rewrite::rewrite(raw)`. A new integration
test at `crates/ferro-protect/tests/spec_rewrite_snapshot.rs` reads the
pinned spec, runs it through `rewrite`, and asserts via
`insta::assert_json_snapshot!`. The accepted snapshot
(`tests/snapshots/spec_rewrite_snapshot__rewrite_output_matches_snapshot.snap`,
5,590 lines) is committed. Added an "When the snapshot test fails" section
to `UPGRADING.md`. All four gates pass.

**Files added/changed**:
- `crates/ferro-protect/build_support/spec_rewrite.rs` (new; moved logic)
- `crates/ferro-protect/build.rs` (now thin, delegates to spec_rewrite)
- `crates/ferro-protect/tests/spec_rewrite_snapshot.rs` (new test)
- `crates/ferro-protect/tests/snapshots/spec_rewrite_snapshot__rewrite_output_matches_snapshot.snap` (new fixture)
- `crates/ferro-protect/Cargo.toml` (insta + serde_json under [dev-dependencies])
- `Cargo.toml` (workspace dep `insta = { version = "1", features = ["json"] }`)
- `UPGRADING.md` (new section)
- `.gitignore` (`**/*.snap.new`)

**Decisions / deviations**:
- Acceptance criterion: `$OUT_DIR/generated.rs` was byte-identical before
  vs after the refactor. Verified via sha256sum (unchanged
  `f6952d2f41b2579076d26eaf618c98a3f30cf57d44c0eb53dcb0f0a52ddd52a8`).
- Acceptance criterion: tripwire works. Locally mutated
  `*v = "3.0.3".to_string()` -> `"3.0.2"` in the rewrite module; the test
  failed with a readable diff at the top-level `openapi` field. Reverted
  and re-ran; back to green.
- The task instructed to "append a PROGRESS.md entry" and commit it
  together with the work, overriding PLAN.md's usual "PROGRESS.md lands in
  the next phase's commit" rule. Following the task instruction.
- Renamed the internal recursive helper (formerly named `rewrite`) to
  `descend` because the public entry point now owns the `rewrite` name.

**Next**: Phase 2 — info endpoint end-to-end + live-NVR integration test
scaffold.

## 2026-05-17 11:55 +0800 — Phase 2: info endpoint end-to-end (library + CLI + live scaffold)

**Status**: complete

**Summary**:
First real client slice. The library now wraps the progenitor-generated
`Client` in a hand-written `ProtectClient` + builder, holds the API key in
`secrecy::SecretString`, sends it via the `X-API-Key` default header, and
exposes one method (`info()`) returning a re-exported `ApplicationInfo`.
Errors from the generated layer pass through a single
`Error::from_progenitor` adaptor that picks `name`/`error` out of any
serialisable error body, so the same adaptor reuses across every endpoint
in later phases. TLS modes: `Native` (default, webpki-roots), `Pinned(Vec<u8>)`
(PEM), and `AcceptInvalid` behind a `dangerous-tls` cargo feature (the CLI
enables this feature so `--insecure` works against self-signed NVRs).

CLI is a clap-derive `Cli` struct with global args (`--host`, `--base-url`,
`--api-key-file`, `--insecure`, `--json`) and an `Info` subcommand. Phase 2
uses a temporary `--api-key <KEY>` flag (hidden in help, marked
`// TODO: remove in phase 3`) so end-to-end tests work before the real
loader lands.

Tests:
- `crates/ferro-protect/tests/info.rs` -- wiremock happy path (200 + JSON
  fixture, asserts the `X-API-Key` header reaches the server) and 401
  error path (asserts the mapped `Error::Api { status, code, message }`).
- `crates/ferro-protect-cli/tests/info.rs` -- `assert_cmd` runs the
  installed binary against a wiremock server in two flavors: default
  human output, `--json` output. Wrapped in `tokio::task::spawn_blocking`
  so the async runtime hosting wiremock doesn't deadlock on the
  synchronous Command call.
- `crates/ferro-protect/tests/live.rs` -- `#[ignore]`d live test against a
  real NVR. Reads `FERRO_PROTECT_LIVE_HOST` + one of
  `FERRO_PROTECT_LIVE_API_KEY_FILE` / `FERRO_PROTECT_LIVE_API_KEY`, plus
  optional `FERRO_PROTECT_LIVE_INSECURE`. Names deliberately distinct
  from the CLI's `UNIFI_PROTECT_*` envs so a developer's normal shell
  cannot accidentally activate it. Runnable via `./scripts/live-test`
  which sources `.env.local` (gitignored) and forwards to `cargo test
  --test live -- --ignored --nocapture`. `.env.example` provided as a
  template; `.env` and `.env.local` added to .gitignore.

All four gates green: 4 ferro-protect tests + 2 ferro-protect-cli tests
+ 1 doc test + the snapshot test from the previous chore.

**Files added/changed**:
- `crates/ferro-protect/src/{error,auth,client,models}.rs` (new)
- `crates/ferro-protect/src/lib.rs` (module declarations + re-exports + quickstart doctest)
- `crates/ferro-protect/Cargo.toml` (secrecy + thiserror + tracing in deps; tokio + wiremock + secrecy in dev-deps; `dangerous-tls` feature)
- `crates/ferro-protect/tests/{info.rs,live.rs}` and `tests/fixtures/info_*.json`
- `crates/ferro-protect-cli/src/main.rs` (clap-derive CLI; temporary `--api-key` scaffold)
- `crates/ferro-protect-cli/Cargo.toml` (forwards `dangerous-tls`; new dev-deps for assert_cmd / wiremock / predicates)
- `crates/ferro-protect-cli/tests/info.rs` (e2e CLI test)
- `scripts/live-test` (new; loads .env.local then runs the ignored live test)
- `.env.example` (template), `.gitignore` (adds `.env` / `.env.local`)

**Decisions / deviations**:
- The integration scaffold the user asked for was promoted into a dedicated
  test file + helper script + env-var contract rather than smuggled into
  the regular wiremock test, so it stays opt-in and discoverable.
- `--api-key <KEY>` flag is intentionally hidden (`hide = true` in clap)
  and documented in code as a phase-3 removal target. The CLI tests use
  it to exercise the e2e path before the smart loader lands.
- `Error::from_progenitor` is generic over `E: Serialize` instead of
  hard-binding to `GenericError`. Future endpoints with different error
  schemas (e.g. `IdRequiredError`) reuse the same adaptor for free.
- Did not promote `auth::ApiKey` to a public type even though clippy was
  fine with it. Holding the raw secret behind the builder's `api_key()`
  method keeps the public surface smaller and lets the secret never
  escape the crate boundary.
- CLI tests use `tokio::task::spawn_blocking` for `assert_cmd::Command`
  because `assert_cmd` is blocking and runs `cargo build` if needed --
  blocking the test's Tokio reactor would have deadlocked the wiremock
  server living on the same runtime.
- `secrecy = "0.10"` is in the workspace `[workspace.dependencies]` from
  phase 0; in 0.10 the idiomatic construction is
  `SecretString::from(String)`, which the code uses throughout.

**Next**: Phase 3 -- smart API key loader (remove `--api-key`, add
file + env-var sources with strict precedence, dedicated tests).

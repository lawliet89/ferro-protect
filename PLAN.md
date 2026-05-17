# UniFi Protect Rust Client — Build Plan

This document is the phased delivery plan. Follow it phase by phase. Do
not skip ahead.

> **Before working on any phase or chore, read [AGENT.md](AGENT.md).**
> It carries the cross-cutting operating rules (commit policy, signing,
> gates, invariants, testing model, logging conventions) every task
> must follow. PLAN.md only carries the *what-to-do-next* — AGENT.md
> carries the *how-to-work*. The two are deliberately separate so an
> agent picking up a single chore does not need to read this whole
> file.

## What we're building

A Rust client library for the UniFi Protect API (version **7.1.60**, local API only, async) plus a CLI tool that exercises the library. Two crates in one Cargo workspace.

- **Library**: `ferro-protect` — async client, typed models, errors, WebSocket subscriptions.
- **CLI**: `ferro-protect-cli` — `clap`-based binary that uses the library and serves as both a real tool and a living integration test.

The OpenAPI 3.1 spec is published at <https://github.com/beezly/unifi-apis>. We consume it as a git submodule, not vendor a copy. The pinned version lives in `crates/ferro-protect/build.rs::SPEC_VERSION` and is currently `7.1.60`.

The seven invariants every phase must preserve (single `SPEC_VERSION`,
`models.rs` seam, mechanical wrappers, `SecretString` everywhere, etc.)
live in [AGENT.md → Invariants you must preserve](AGENT.md#invariants-you-must-preserve).
The forward-compatibility tooling those invariants imply
(`scripts/update-spec`, [UPGRADING.md](UPGRADING.md), and the API
surface snapshot test) is delivered in phase 1 and phase 11
respectively; see those phases for the concrete tasks.

---

## Project layout (target state)

```
.
├── Cargo.toml                          # workspace manifest
├── rust-toolchain.toml                 # pins toolchain channel
├── rustfmt.toml                        # formatting config
├── deny.toml                           # cargo-deny config
├── .gitmodules                         # submodule pointer
├── .github/workflows/ci.yml            # CI pipeline (includes live-env guard)
├── .gitignore                          # includes .env, .env.local
├── .env.example                        # template for UNIFI_PROTECT_* vars
├── scripts/
│   ├── pre-commit                      # optional local hook
│   ├── update-spec                     # one-command spec bump (phase 1)
│   └── live-test                       # source .env.local + run live tests
├── third_party/
│   └── unifi-apis/                     # submodule: github.com/beezly/unifi-apis
├── crates/
│   ├── ferro-protect/                  # library
│   │   ├── Cargo.toml
│   │   ├── build.rs                    # typify model codegen
│   │   ├── build_support/              # shared between build.rs and tests
│   │   │   └── spec_rewrite.rs
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── client.rs
│   │   │   ├── auth.rs
│   │   │   ├── error.rs
│   │   │   ├── models.rs               # public type re-exports from generated
│   │   │   ├── generated.rs            # includes $OUT_DIR/generated.rs
│   │   │   ├── ws/                     # WebSocket layer (phase 7)
│   │   │   └── media.rs                # binary endpoints (phase 5)
│   │   └── tests/
│   │       ├── common/                 # shared helpers (live_client, etc.)
│   │       ├── fixtures/               # canned JSON for wiremock tests
│   │       ├── live.rs                 # always-on, auto-skips when env absent
│   │       └── *.rs                    # per-entity mocked tests
│   └── ferro-protect-cli/              # CLI
│       ├── Cargo.toml
│       ├── src/
│       │   ├── main.rs
│       │   ├── api_key.rs              # the three-source loader
│       │   ├── output.rs               # human vs --json formatting
│       │   └── commands/               # one file per subcommand group
│       └── tests/
│           └── *.rs                    # assert_cmd e2e
├── PLAN.md
├── PROGRESS.md
├── UPGRADING.md                        # spec-bump procedure (phase 1)
├── ARCHITECTURE.md                     # start-here for code readers
├── CHANGELOG.md
└── README.md                           # includes "Running tests" section
```

---

## Phase 0 — Workspace skeleton

**Goal**: green `cargo build`, `cargo fmt --check`, `cargo clippy`, `cargo deny check` on an empty workspace with both crates and full CI wired up.

Tasks:

1. Create `Cargo.toml` workspace manifest with `members = ["crates/*"]`. Define `[workspace.dependencies]` for shared crates (`tokio`, `reqwest`, `serde`, `serde_json`, `thiserror`, `tracing`, `bytes`, `url`, `secrecy`, `futures-util`, `clap`, `anyhow`, `wiremock`, `assert_cmd`, `predicates`, `tokio-tungstenite`). Define `[workspace.lints.clippy]` with `pedantic = "warn"`, `nursery = "warn"`, and a tiny explicit allow list (`module_name_repetitions = "allow"`, `must_use_candidate = "allow"` to start — add more only with a logged reason).
2. Create `rust-toolchain.toml` pinning the stable channel (use the current stable at time of work) with `rustfmt` and `clippy` components.
3. Create `rustfmt.toml` (start minimal: `edition = "2021"`, `max_width = 100`).
4. Create `deny.toml` configured for advisory checks, license allow list (MIT, Apache-2.0, BSD-3-Clause, ISC, Unicode-DFS-2016, and others as they come up — add with a logged reason), and a banned-crates section (empty to start).
5. Add `third_party/unifi-apis` as a submodule: `git submodule add https://github.com/beezly/unifi-apis third_party/unifi-apis`. Pin to a specific commit so future updates are deliberate.
6. Create `crates/ferro-protect/` with a minimal `Cargo.toml` (`lints.workspace = true`) and a `src/lib.rs` containing only `#![forbid(unsafe_code)]` and a doc comment. No build.rs yet.
7. Create `crates/ferro-protect-cli/` with a minimal `Cargo.toml` (depends on `ferro-protect` via `path = "../ferro-protect"`, plus `clap` with `derive` feature, `anyhow`, `tokio` with `rt-multi-thread` + `macros`). `src/main.rs` is a stub `fn main() {}` (still with `#![forbid(unsafe_code)]`).
8. Create `.github/workflows/ci.yml`: matrix on Linux at minimum, steps for `checkout` (with `submodules: recursive`), `rust-toolchain` install, then `cargo fmt --all --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all`, `cargo deny check`. Cache the cargo registry and target dir. Include the `UNIFI_PROTECT_*` env guard from the testing strategy section as the first step after checkout.
9. Create `scripts/pre-commit` (executable bash): runs `cargo fmt --all -- --check` and `cargo clippy --all-targets -- -D warnings`. Document in README how to symlink it into `.git/hooks/pre-commit`.
10. Create `.gitignore` (target/, *.swp, .DS_Store, /PROGRESS.md.bak, `.env`, `.env.local`, etc. — but **do** track `PROGRESS.md` and `.env.example` themselves).
11. Create a stub `README.md` (one paragraph + clone instructions including `--recurse-submodules`). The "Running tests" section is fleshed out in the chore after phase 2, but reserve a heading for it here. Also create an empty `CHANGELOG.md`.
12. Verify everything passes locally. Commit.

**Commit message**: `phase(0): set up workspace skeleton, lints, CI, submodule`

---

## Phase 1 — Codegen pipeline

**Goal**: `cargo build` produces a generated Rust module from the v6.2.83 spec, with clippy clean on hand-written code and silenced on generated code.

Tasks:

1. Add `typify` (latest), `serde_json`, `syn`, `prettyplease`, and whatever JSON Schema parsing support typify requires to `crates/ferro-protect/Cargo.toml` under `[build-dependencies]`. Add `reqwest` (with `json`, `stream`, `rustls-tls`), `bytes`, `chrono` (with `serde` feature), `futures-core`, and `url` to `[dependencies]`.
2. Create `crates/ferro-protect/build.rs`. Hardcode a constant `const SPEC_VERSION: &str = "6.2.83";`. The spec path is derived as `third_party/unifi-apis/unifi-protect/{SPEC_VERSION}.json` (note: the submodule's folder is `unifi-protect/`, not `ferro-protect/`). The build script:
   - Prints `cargo:rerun-if-changed=` for that path.
   - Reads the file. If missing, prints a helpful error telling the user to run `git submodule update --init --recursive`.
   - Parses as `serde_json::Value`. Extracts `components.schemas`, applying only the JSON Schema preprocessing typify needs.
   - Invokes `typify::TypeSpace` for model generation only. No HTTP client, operation, or response enum generation happens here.
   - Writes prettyprinted output to `$OUT_DIR/generated.rs`.
3. Create `crates/ferro-protect/src/generated.rs` containing only:
   ```rust
   #![allow(clippy::all, clippy::pedantic, clippy::nursery, dead_code, unused_imports)]
   include!(concat!(env!("OUT_DIR"), "/generated.rs"));
   ```
4. Add `pub(crate) mod generated;` to `lib.rs`. Do not yet re-export anything from it — that happens in phase 2 when we have a real wrapper.
5. Create `scripts/update-spec` (executable bash). It must:
   - Take an optional positional arg `[VERSION]` (e.g. `7.1.60`); if omitted, prints the list of available versions in `third_party/unifi-apis/unifi-protect/` and exits.
   - `git -C third_party/unifi-apis fetch && git -C third_party/unifi-apis checkout origin/HEAD` (so new versions become visible). Track upstream HEAD commit so the user can pin a different one.
   - Verify the requested spec file exists at the expected path.
   - Rewrite the `SPEC_VERSION` constant in `crates/ferro-protect/build.rs` via `sed` (or a simple Rust helper).
   - `cargo build -p ferro-protect` (forces regeneration), then `cargo test --all`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo deny check`.
   - On success: print the new submodule SHA and the next-step git commands to commit. On failure: leave the state as-is and exit non-zero with a clear message.
   - Be safe to re-run.
6. Create `UPGRADING.md` at the repo root. It must contain, in order: (a) one-paragraph orientation; (b) the happy path (`./scripts/update-spec <new-version>` then commit); (c) what to do when codegen fails (point at phase 1's fallback options + the `book` of OpenAPI spec massagings in `build.rs`); (d) what to do when wrappers fail to compile (the order is `models.rs` first, then the specific entity module — keep generated types out of public signatures); (e) how to read the generated-code diff under `target/debug/build/ferro-protect-*/out/generated.rs`; (f) a short checklist intended for agents (literal numbered steps a coding agent can follow without further context). Keep the file under 120 lines.
7. Verify `cargo build` succeeds, `cargo clippy` is clean on hand-written code, `cargo fmt --check` passes, `./scripts/update-spec` (no args) prints the version list. Commit.

If typify fails on a specific schema, log the failure mode in `PROGRESS.md` and try a small preprocessing rule first. If that would be too broad, hand-write that one type in `models.rs` and skip it during codegen via a small allowlist in `build.rs`. Pick whichever gets you unblocked fastest; document the choice.

**Commit message**: `phase(1): wire up typify model codegen from submoduled spec`

---

## Phase 2 — First end-to-end slice: `info`

**Goal**: `ferro-protect info` against a real NVR returns the application version. Library + CLI + tests, all green.

Tasks:

1. **Library**: `crates/ferro-protect/src/error.rs` — define a `unifi_protect::Error` enum with `thiserror`. Variants: `Http(reqwest::Error)`, `Api { status: u16, code: String, message: String }`, `Json(serde_json::Error)`, `InvalidUrl(String)`, `MissingApiKey`, `Other(String)`. `pub type Result<T> = std::result::Result<T, Error>;`.
2. **Library**: `crates/ferro-protect/src/auth.rs` — thin wrapper holding `SecretString` (from `secrecy` crate). Implements a `reqwest::header::HeaderValue` extractor. Header name: `X-API-Key`.
3. **Library**: `crates/ferro-protect/src/client.rs` — public `ProtectClient` struct with a `builder()` returning `ProtectClientBuilder`. Builder fields: `host`, `api_key` (SecretString), TLS mode (`Native`, `Pinned(Vec<u8>)`, `AcceptInvalid` gated behind `insecure-tls` feature). `.build()` constructs a `reqwest::Client` with the X-API-Key default header, configured TLS, sensible timeouts (connect 10s, total 30s), HTTP/2. Base URL is `https://{host}/proxy/protect/integration`. Builds on shared get/post/patch/bytes helpers.
4. **Library**: implement `impl ProtectClient { pub async fn info(&self) -> Result<ApplicationInfo> }`.
5. **Library**: expose `ApplicationInfo` at `unifi_protect::models::ApplicationInfo`. Create `crates/ferro-protect/src/models.rs` for this purpose -- every public type re-export from generated code, plus any tiny hand-written response type for inline operation schemas, lives here. Consumers must never see `crate::generated::...` types in public signatures. This is the integration seam that absorbs spec changes (see "Forward-compatibility with spec upgrades" above).
6. **Library tests**: `tests/info.rs` — uses `wiremock` to stand up a mock server that responds to `GET /v1/meta/info` with a fixture. Asserts the client parses it correctly. Add a second test for a 401 error response mapping to `Error::Api { status: 401, .. }`.
7. **Live test**: `tests/live.rs` — at this phase, contains one `live_read_info` test. Use the helpers in `tests/common/mod.rs` (see testing strategy section). Test asserts the version string is non-empty and parses as expected. This test is **not** `#[ignore]`d; it auto-skips when env is absent.
8. **CLI**: `crates/ferro-protect-cli/src/main.rs` — sketch out the `Cli` struct with global args (`--host`, `--api-key-file`, `--insecure`, `--json`) and a `Commands` enum with a single `Info` variant for now. Defer api_key resolution to phase 3 — for this phase, accept `--api-key` directly as a temporary scaffold (mark it `// TODO: remove in phase 3` and log in PROGRESS).
9. **CLI**: implement the `info` subcommand. Human output: prints the version. JSON output: prints the full structure.
10. **CLI tests**: `tests/info.rs` using `assert_cmd` — spawn the binary against a `wiremock` server, assert exit code 0 and expected stdout for both human and `--json` flavors.
11. Run fmt, clippy, test, deny. Commit.

**Commit message**: `phase(2): implement info endpoint end-to-end (library + CLI)`

---

## Chore (between phase 2 and phase 3) — testing model + README

The "Testing strategy" section above is the canonical reference. Phase 2's first
implementation may have used `#[ignore]` for live tests (the original plan
specified this); this chore migrates them to the auto-skip model, formalizes the
shared helpers, adds the CI guard, and writes the README testing section. After
this chore lands, all subsequent phases follow the pattern by default.

Tasks:

1. If phase 2 marked live tests with `#[ignore]`, remove the attribute. Live tests
   gate on env vars at the function top, not on test runner flags.
2. Create `crates/ferro-protect/tests/common/mod.rs` with two helpers:
   - `pub fn live_client() -> Option<ProtectClient>` — resolves `UNIFI_PROTECT_HOST`
     and either `UNIFI_PROTECT_API_KEY_FILE` or `UNIFI_PROTECT_API_KEY`,
     plus `UNIFI_PROTECT_INSECURE`. Returns `None` if `HOST` is missing.
     Panics with a clear message if `HOST` is set but no key source is.
   - `pub fn mutations_allowed() -> bool` — `true` only when `UNIFI_PROTECT_ALLOW_MUTATIONS=1`.
3. Rename any existing live tests to `live_read_*` (e.g., the phase-2 info test
   becomes `live_read_info`). Adopt the test prelude pattern from the testing
   strategy section.
4. Update `scripts/live-test` to source `.env.local` and run
   `cargo test --test live -- --nocapture` (no `--ignored` flag any more).
   Document the file's bash dependency in a comment.
5. Update `.github/workflows/ci.yml`: add the `UNIFI_PROTECT_*` guard step
   (see testing strategy section for the bash) early in the job, before any
   `cargo test` invocation.
6. Write a "Running tests" section in `README.md` that covers:
   - **Quick start for contributors**: `cargo test --all` runs everything;
     mocked tests always run, live tests auto-skip without a configured NVR.
   - **Running live tests**: copy `.env.example` to `.env.local`, fill in
     values, then either `source .env.local && cargo test --all` or use
     `./scripts/live-test` for a one-shot.
   - **Running mutating live tests**: additionally set
     `UNIFI_PROTECT_ALLOW_MUTATIONS=1`. State plainly that this can
     change NVR state, trigger physical events (sirens, camera motion,
     doorbell chimes), and should be done deliberately, ideally against
     a non-production NVR.
   - **Env file security note**: `.env.local` is gitignored; keep the API
     key file referenced by `UNIFI_PROTECT_API_KEY_FILE` outside the
     repo or in a gitignored location. Brief mention of `chmod 600` for
     the key file.
   - **CI behavior**: the CI runner blocks any `UNIFI_PROTECT_*` env
     var from being set, so live tests cannot accidentally run from PR
     builds.
7. Run fmt, clippy, test, deny. Confirm `cargo test --all` (with no
   `UNIFI_PROTECT_*` set) passes locally and the live test reports as
   `ok` with skip output to stdout. Then export `UNIFI_PROTECT_HOST=invalid.example`
   without any key, and confirm the test panics with the helper's clear
   "HOST set but no key provided" message (the contract specified in step 2).

**Commit message**: `chore(tests): auto-skip live tests, document test model in README`

---

## Phase 3 — Smart API key loader

**Goal**: CLI accepts the API key via three sources with strict precedence, never via a raw value on the command line.

Source precedence (highest first):
1. `--api-key-file <PATH>` flag (path only, never raw key).
2. `UNIFI_PROTECT_API_KEY_FILE` env var (path).
3. `UNIFI_PROTECT_API_KEY` env var (raw key).

Tasks:

1. **CLI**: create `crates/ferro-protect-cli/src/api_key.rs` implementing the resolver. Returns `SecretString`. Trims trailing whitespace/newlines from file contents. Rejects empty files with a clear error. On Unix, warns (does not error) if the file mode allows group/world read (`mode & 0o077 != 0`).
2. **CLI**: define an `ApiKeyError` enum with variants `NotProvided` (its Display lists all three accepted sources), `ReadFailed { path: PathBuf, source: io::Error }`, `EmptyFile(PathBuf)`.
3. **CLI**: remove the temporary `--api-key` scaffold from phase 2. Update the global args: `--api-key-file <PATH>` is the only key-related flag. Critically, do **not** set `env = "UNIFI_PROTECT_API_KEY"` on this flag in clap — the lookup is manual, in `api_key::resolve()`.
4. **CLI tests**: dedicated `tests/api_key.rs` covering: flag wins over both env vars, `_FILE` env var wins over raw env var, raw env var works alone, missing-all returns `NotProvided` with helpful message, empty file errors, nonexistent file errors, trimmed file contents work.
5. Update `info` command to use the resolver. Run fmt, clippy, test, deny. Commit.

**Commit message**: `phase(3): implement smart API key resolver with three sources`

---

## Phase 4 — Read endpoints across all entities

**Goal**: complete read-only inventory of the NVR via the CLI. Every "list" and "get by id" the spec exposes.

Order (one vertical slice per row — library method + CLI subcommand + wiremock test + assert_cmd test + `live_read_*` test, **commit after each entity pair, not each row**):

1. `cameras list` + `cameras get <id>` → commit.
2. `chimes list` + `chimes get <id>` → commit.
3. `lights list` + `lights get <id>` → commit.
4. `liveviews list` + `liveviews get <id>` → commit.
5. `nvrs list` + `nvrs get <id>` → commit.
6. `sensors list` + `sensors get <id>` → commit.
7. `viewers list` + `viewers get <id>` → commit.

For each entity, add a `live_read_<entity>_list` test (asserts the call
succeeds and returns a `Vec`, no assertions on contents — different NVRs
have different inventories). If at least one device of the type is present
in the list response, also call `get` on the first one in a
`live_read_<entity>_get` test and assert the round trip parses. Skip the
`get` test gracefully if the list is empty.

CLI subcommand pattern (use `clap`'s `Subcommand` derive on a per-entity enum):

```
ferro-protect cameras list [--json]
ferro-protect cameras get <ID> [--json]
```

Human output for lists is a compact table (id, name, type, state). `--json` prints the array unmodified. The `Output` helper module (`src/output.rs`) centralises format selection and shared render helpers; list tables are rendered via a single `output::table()` helper and per-entity `render_one()` blocks stay manual.

Library shape that emerges:

```rust
client.cameras().list().await?
client.cameras().get(id).await?
client.chimes().list().await?
// ...etc
```

Implement these as a small `CamerasApi<'a>`, `ChimesApi<'a>` etc., returned by `client.cameras()` etc. Each holds a `&ProtectClient`.

By the end of phase 4 the library has roughly 14 typed methods and the CLI is a useful read-only NVR inspector. Commit after each entity pair (7 commits in this phase). Phase 4 wraps up with a final log entry in `PROGRESS.md`.

**Commit messages**: `phase(4): add cameras read endpoints`, `phase(4): add chimes read endpoints`, etc.

---

## Phase 5 — Binary endpoints

**Goal**: snapshots, stream URLs, talkback session info. All reads; safe
to run without the mutation gate.

1. `cameras snapshot <id>` — returns `Bytes` in the library. CLI writes to `--out <PATH>`, to stdout if not a TTY, errors with friendly message if stdout is a TTY and no `--out`. Use the `is-terminal` crate. This endpoint uses the shared raw-bytes HTTP helper.
2. `cameras rtsps <id>` — returns the RTSPS URL as a string. Trivial.
3. `cameras talkback <id>` — returns the WebSocket URL and codec metadata. Library exposes the structured info, CLI prints it. Out of scope: actual audio piping.

All three have `live_read_*` tests (calling them does not change NVR state). The snapshot live test asserts the body is non-empty and starts with the JPEG magic bytes (`FF D8 FF`); do not snapshot-test the bytes themselves. One commit per endpoint or one combined — your call. Log the decision.

---

## Phase 6 — Files: list (read)

**Goal**: read half of file management (ringtones etc.). The upload
half is deliberately deferred to phase 10 — see the "all mutations at
the end" reorganisation note in the project history. Splitting Files
across two phases means the CLI `files` namespace exists from phase 6
onward but only gains its `upload` subcommand in phase 10.

1. `GET /v1/files/{fileType}` — list files of a type. `live_read_files_list`.

CLI: `ferro-protect files list <fileType>`. One commit.

---

## Phase 7 — WebSocket subscriptions

**Goal**: streaming read endpoints. Last of the read phases because
they're the highest-risk read surface (long-lived connections, framing
quirks).

1. **First**: `/v1/subscribe/devices`. Implement hand-written using `tokio-tungstenite`. WS URL is `wss://{host}/proxy/protect/integration/v1/subscribe/devices`. Pass `X-API-Key` as a handshake header. Returns `impl Stream<Item = Result<DeviceMessage>>` where `DeviceMessage` is a serde-tagged enum matching the spec's `oneOf { add, update, remove }` discriminator. CLI: `ferro-protect subscribe devices` streams NDJSON to stdout (one JSON object per line). Commit.
2. **Then**: `/v1/subscribe/events` — same pattern, different message type. Commit.
3. **Optional reconnection helper**: behind a `reconnect` cargo feature on the library and a `--reconnect` flag on the CLI. Exponential backoff 8s → 120s, configurable max attempts. Commit.

Live tests: `live_read_subscribe_devices` and `live_read_subscribe_events`. Both
connect, wait up to a short timeout (5s) for either the first message or a
clean idle confirmation, assert the connection handshake succeeded, then
disconnect cleanly. Do **not** assert on message content — different NVRs
produce different activity, and a test waiting for a motion event would flap
forever on a quiet NVR. The handshake itself is the assertion.

If the WebSocket framing turns out to differ from straight JSON-over-WS (it has historically on Protect's private API), log the discovery and document the framing in code comments.

---

## Phase 8 — Mutating CRUD: PATCH and POST

**Goal**: configuration changes via PATCH and creates via POST, in order of increasing impact. First of the mutation phases.

Order (one commit per entity):

1. `viewers patch <id>` (rename, change attached liveview — lowest blast radius).
2. `liveviews patch <id>`.
3. `liveviews create` (POST) and `liveviews delete <id>`.
4. `chimes patch <id>` (volume, ringtone).
5. `lights patch <id>` (mode, brightness).
6. `sensors patch <id>`.
7. `cameras patch <id>` (largest surface — recording settings, smart detect, etc.).

CLI design for each PATCH command:
- Expose the most-useful fields as named flags (e.g. `--name`, `--recording-mode`, `--brightness`).
- Always include `--patch-json '<json>'` escape hatch for fields not covered by flags.
- Include `--dry-run` that prints the constructed JSON body and exits without sending.

Library shape:

```rust
client.cameras().patch(id, CameraPatch::builder().recording_mode(Mode::Always).build()).await?
```

Patch builders should accept `Option<T>` semantics so only set fields are serialized (use `serde(skip_serializing_if = "Option::is_none")`).

Testing per entity:
- Mocked test asserting the PATCH body shape matches expectations.
- `assert_cmd` test confirming CLI flags map to the right JSON fields, plus
  a `--dry-run` test that asserts the printed body and that no HTTP call
  reaches the wiremock server.
- A `live_write_<entity>_patch` test that round-trips: read current value,
  patch it to something else, read back, patch it back to original. Gated
  by `common::mutations_allowed()`. Skip cleanly when off.

Commit after each entity.

---

## Phase 9 — Action endpoints

**Goal**: the "do a thing" POSTs.

Order:

1. `chimes play <id>` — POST `/v1/chimes/{id}/play`.
2. `cameras ptz-goto <id> --slot <n>` — POST `/v1/cameras/{id}/ptz/goto/{slot}`.
3. `cameras ptz-patrol-start <id> --slot <n>`.
4. `cameras ptz-patrol-stop <id>`.
5. `alarm trigger <id>` — POST `/v1/alarm-manager/webhook/{id}`.

These are simple — no body shape complexity. One commit covering all action endpoints is fine, or split if any one of them is unusually complex. Tests: mocked + `assert_cmd` as always. Live tests for these are all `live_write_*` (they cause physical effects). Implement them but expect them to be exercised rarely; the mutation gate is the safety belt.

---

## Phase 10 — Files: upload (mutation)

**Goal**: upload half of file management. The read half landed in
phase 6.

1. `POST /v1/files/{fileType}` — multipart upload. `live_write_files_upload`
   (gated by `common::mutations_allowed()`).

CLI: `ferro-protect files upload <fileType> <PATH>` — adds an `upload`
subcommand to the existing `files` namespace from phase 6. One commit.

---

## Phase 11 — Polish and release prep

1. Audit library docs: top-level rustdoc with a quickstart example. Every public item has a doc comment.
2. Audit CLI `--help`: subcommands grouped sensibly, all flags have help text.
3. Add `insta` snapshot tests for the CLI `--help` output of the root command and each subcommand at `crates/ferro-protect-cli/tests/help_snapshot.rs`. Run `INSTA_UPDATE=auto cargo test` once to generate, then commit the snapshot files. These are tripwires for accidental CLI surface changes during refactors.
4. If the `ApiKeyError` Display formats and a handful of other canonical error messages have stabilized, add an `insta` snapshot test for them too at `crates/ferro-protect-cli/tests/error_messages_snapshot.rs`. Skip if any of these messages are still in flux.
5. Expand `README.md` beyond the testing section added in the post-phase-2 chore: install instructions (`cargo install --path crates/ferro-protect-cli`), CLI quickstart, library quickstart, security notes on API key file handling, troubleshooting (self-signed TLS, finding the host IP, generating an API key in the Protect UI). Cross-link to the testing section.
6. Fill in `CHANGELOG.md` with a `0.1.0` entry summarizing what shipped.
7. Set both crates to version `0.1.0` in their manifests.
8. Final lint sweep: run `cargo clippy --all-targets --all-features -- -D warnings -W clippy::pedantic -W clippy::nursery` and resolve anything that didn't show up before.
9. Add an API-surface snapshot test at `crates/ferro-protect/tests/public_api.rs` that imports every `models::*` type we publicly re-export and constructs a default value (or otherwise touches the type) for each. The goal is purely a compile-time canary: if a future spec rename removes a type, this test fails in one obvious place rather than scattered through wrappers.
10. Sanity-check the upgrade flow: run `./scripts/update-spec` (no args) and confirm it lists versions; then dry-run a bump to the next-newest 6.2.x version on a throwaway branch to verify the script still works end-to-end. Revert. Note the dry-run result in `PROGRESS.md` and `UPGRADING.md`.
11. Sweep `ARCHITECTURE.md`: re-read it as if you'd never seen the code, fix any drift, ensure the file map matches the on-disk layout, and confirm every invariant it claims is still enforced.
12. Tag the commit `v0.1.0`.

**Commit message**: `phase(11): docs, polish, release 0.1.0`

---

## Deferred — revisit before 0.1.0 (or when the trigger fires)

Items surfaced during reviews that are not bugs and not blocking, but
that the next reader should weigh deliberately rather than rediscover.
Move each into a phase when the trigger condition is met, or close it
out in `PROGRESS.md` with a "won't do" rationale.

### Seal the `models.rs` seam against typify newtype tunneling

**Symptom.** Typify renders string-shaped schemas (`ProtectVersion`,
`CameraId`, `ChimeId`, `Name`, `Mac`, etc.) as `pub struct Foo(pub
String)`. `models.rs` re-exports these as-is, so downstream callers can
reach past the seam with `id.0` or pattern-match against the literal
`(pub String)` shape. The "single seam" promise is that `models.rs` is
the one fix-site for a spec bump; `.0` tunnelling breaks that.

**Trigger to act.** First spec bump that renames or restructures one of
these newtypes, OR the first external (out-of-tree) consumer of the
library. Until either fires, the cost (≈4–8 hand-written newtypes with
`Serialize`/`Deserialize`/`Display`/`From`/`AsRef<str>` impls each,
plus serde plumbing) outweighs the benefit.

**Fix shape, when adopted.** Hand-written wrapper newtypes in
`models.rs` that wrap (not re-export) the generated type with a private
inner field, `#[serde(transparent)]` for wire compatibility, and the
minimum trait set wrappers need. Originally raised in the oas3 PR
review; deferred there as architectural rather than a drive-by fix.

### Reshape `drop_drifted_audio_detection_enum` away from value-sniffing

**Symptom.** [build_support/spec_rewrite.rs](crates/ferro-protect/build_support/spec_rewrite.rs)
contains a preprocessing function that matches on the literal string
`"alrmCmonx"` to detect the smart-audio-detection enum and relax it to
a plain `String`. Every other function in the file matches on JSON
Schema structure (`type: ["X", "null"]`, `const`, `allOf: [<$ref>]`),
so this one breaks the file's "pure structural preprocessing" pretense
and bakes a runtime-observed token into a "pure" pipeline.

**Trigger to act.** Next spec bump that adds a second runtime-vs-spec
enum drift case (so the cost of designing the right shape is
amortised), OR a live integration test against a representative NVR
(one whose owner has actually configured smart-audio detection at
some point) passes with the rule disabled.

Confirmed 2026-05 against firmware 7.1.60: rule still required. The
drifted value `smoke_cmonx` is persisted in per-camera
`smartDetectSettings.audioTypes` user config, *not* in the
`cameraFeatureFlags.smartDetectAudioTypes` capability advertisement.
The capability field has normalised to spec values in current
firmware, but the user-config field round-trips whatever was
originally written by older firmware. See PROGRESS.md entry
"Investigated retiring drop_drifted_audio_detection_enum" for the
full retirement experiment and what to look at next time. Do not
trust a quick `curl ... | jq .featureFlags.smartDetectAudioTypes`
check — it inspects the wrong field.

**Two viable fix shapes.**

1. *Named-target preprocessing list.* Add a `RELAX_ENUMS: &[&str] =
   &["smartDetectAudioTypes", ...]` table at the top of
   `spec_rewrite.rs` plus a function that walks
   `components.schemas.<name>` and strips `enum`. Keeps preprocessing
   pure (structural lookup) and extending it for new drift cases is one
   line. Cheapest move.
2. *Hand-written newtype in `models.rs`.* Define
   `pub struct SmartDetectAudioType(String)` and skip the schema during
   codegen via a typify allowlist. Architecturally cleaner — schema
   preprocessing stops carrying runtime knowledge entirely — but
   commits us to allowlist machinery that nothing else needs yet. Pays
   off only if `models.rs` ends up with several other hand-written
   wrappers.

Default recommendation when the trigger fires: option 1, unless other
hand-written wrappers have appeared in the meantime.

---

## Reference: spec source

- Repo: <https://github.com/beezly/unifi-apis>
- Path in submodule: `third_party/unifi-apis/unifi-protect/{SPEC_VERSION}.json` (currently `7.1.60.json`)
- Format: OpenAPI 3.1.0; consumed as JSON Schema by typify with minor preprocessing (see phase 1)
- Base URL pattern: `https://{nvr-host}/proxy/protect/integration` (spec server is `/integration`, paths begin with `/v1/...`)
- Auth: `X-API-Key` request header
- Self-signed TLS is the default on consumer NVRs — handle gracefully

## Reference: progress log template

See [AGENT.md → Progress logging](AGENT.md#progress-logging) for the
canonical entry format and timing rules.

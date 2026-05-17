# UniFi Protect Rust Client — Build Plan

This document is your working spec. Follow it phase by phase. Do not skip ahead.

## What we're building

A Rust client library for the UniFi Protect API (version **6.2.83**, local API only, async) plus a CLI tool that exercises the library. Two crates in one Cargo workspace.

- **Library**: `ferro-protect` — async client, typed models, errors, WebSocket subscriptions.
- **CLI**: `ferro-protect-cli` — `clap`-based binary that uses the library and serves as both a real tool and a living integration test.

The OpenAPI 3.1 spec for v6.2.83 is published at <https://github.com/beezly/unifi-apis>. We will consume it as a git submodule, not vendor a copy.

---

## Operating instructions for you (Claude Code)

Read this section before doing anything else.

### Repository state when you start

The user will have already run `git init` in the working directory. The directory may be otherwise empty, or it may already contain this `PLAN.md`. Treat the working directory as your repo root. Do **not** run `git init` yourself.

### Progress logging

Maintain a `PROGRESS.md` file at the repo root. Create it on first run. Append a new entry every time you finish a phase, or every time you make a decision that deviates from this plan, or every time you hit something that surprises you. Each entry uses this format:

```
## YYYY-MM-DD HH:MM ±HHMM — Phase N: <short title>

**Status**: complete | partial | blocked
**Summary**: one-paragraph description of what was done.
**Files added/changed**: bulleted list.
**Decisions / deviations**: anything you did differently from the plan, with reasoning.
**Next**: what comes next, or what's blocking.
```

Use a real timestamp captured at the moment you write the entry, not the literal placeholder above. On Unix, date +"%Y-%m-%d %H:%M %z" produces the right format (e.g. 2026-05-17 11:05 +0800). The timezone offset is part of the format — drop it and entries written months apart from different machines lose ordering information.

`PROGRESS.md` itself does **not** get committed in the same commit as the work it describes — commit it at the *start* of the next phase, so the log entry for phase N lands in the commit that begins phase N+1. This keeps each phase commit clean and topical. The final phase's log entry goes in its own follow-up commit.

### Commit policy

Commit at the end of every phase. One phase, one commit (squash if you need to). Use Conventional Commits style:

```
phase(N): <short description>

<longer body describing what landed and why, referencing PLAN.md phase N>
```

Examples:

- `phase(0): set up workspace skeleton, lints, CI, submodule`
- `phase(2): implement info endpoint end-to-end (library + CLI)`
- `phase(4): add read endpoints for cameras, chimes, lights`

### PGP signing — important

The user has commit signing configured and may require a passphrase. **Never** use `--no-gpg-sign`, `-S` with a hardcoded key, `--no-verify`, or any flag that bypasses signing or hooks. Attempt the commit normally with `git commit -m "..."`. If it succeeds, great. If it fails because of a passphrase prompt, signing key issue, or anything signing-related:

1. Stop. Do not retry with workarounds.
2. Write the exact commit message and any staged-files context into `PROGRESS.md` under a "Pending commit" section.
3. Print to the user: *"Phase N is ready to commit. Please run the following yourself so your signing key is used:"* followed by the exact `git add` and `git commit` commands.
4. Wait for the user to confirm the commit landed before starting the next phase.

The same applies to `git push` if the user has push signing configured — never bypass.

### Guardrails (non-negotiable, enforced on every commit)

- `#![forbid(unsafe_code)]` at the top of every `lib.rs` and `main.rs`. No `unsafe` blocks anywhere, ever. If you think you need one, stop and write a log entry explaining why before continuing.
- `cargo fmt --all -- --check` must pass before every commit.
- `cargo clippy --all-targets --all-features -- -D warnings` must pass before every commit.
- `cargo test --all` must pass before every commit.
- `cargo deny check` must pass before every commit (once `deny.toml` is in place in phase 0).

If any of these fail, fix them before committing. Do not commit broken state. If you cannot fix a clippy lint without compromising the design, add a `#[allow(...)]` on the smallest possible scope with a comment explaining why, and log the decision in `PROGRESS.md`.

### Working style

- Prefer many small, well-tested changes over big sweeping ones.
- Every phase ends with green CI checks (or their local equivalent) before you commit.
- When a phase says "library + CLI + tests", deliver all three before the phase is done. No half-phases.
- If the plan is unclear or you discover the spec contradicts the plan, log the question and pick the most defensible interpretation. Don't block waiting for clarification on small things.

---

## Forward-compatibility with spec upgrades

We will pin a specific Protect spec version, but new versions ship constantly. The
codebase must absorb a new spec by bumping a single constant and re-running codegen —
not by hand-editing wrapper modules. To make that real:

1. **Single source of truth for the version.** The spec version lives in **one place**:
   the `SPEC_VERSION` constant in `crates/ferro-protect/build.rs`. The spec file path
   is derived from it as `third_party/unifi-apis/unifi-protect/{SPEC_VERSION}.json`.
   Nothing else hardcodes the version. (The plan was originally written referring to
   `ferro-protect/{ver}.json`; the actual folder in the submodule is `unifi-protect/` —
   use that.)
2. **Generated types are accessed only through `models`.** Hand-written code never
   names `crate::generated::types::Foo` directly. Instead `crates/ferro-protect/src/models.rs`
   re-exports the types we expose, optionally with renames. If codegen renames a
   type, the only fix needed is in `models.rs`. Public signatures throughout the
   library refer to `crate::models::Foo`, never `crate::generated::...`.
3. **Wrappers delegate, they do not re-implement.** Every wrapper method
   (`client.cameras().list()`, etc.) is a thin shim that calls the corresponding
   generated client method, translates errors via `From` impls, and returns
   `models::Foo`. No request bodies are constructed by hand when a generated
   builder exists.
4. **PATCH bodies use generated types where the spec defines them.** Where the spec
   only exposes a free-form schema (rare on Protect), define a hand-written builder
   with `#[serde(skip_serializing_if = "Option::is_none")]`, and add a comment
   pointing back to the spec path so a future agent can re-check it.
5. **Update script.** A `scripts/update-spec` shell script (added in phase 1)
   automates the version bump: updates the submodule, lets the user pick a new
   `SPEC_VERSION`, runs `cargo build` (which regenerates), then runs the full check
   suite. It must be safe to re-run.
6. **Upgrade documentation.** An `UPGRADING.md` at the repo root (added in phase 1)
   gives both humans and agents a step-by-step recipe for moving to a newer spec
   version, including how to triage codegen failures, how to interpret the diff of
   generated code, and where to look when wrappers fail to compile (almost always
   `models.rs` first, then the specific entity module).
7. **API surface snapshot test.** Phase 10 adds a tiny test that constructs the
   public types we re-export, so codegen-driven renames break compilation in a
   single, obvious place.

These rules are non-negotiable. If a phase tempts you to name `crate::generated::...`
in a public signature, stop, add the type to `models.rs` first, then continue.

---

## Testing strategy

This section is cross-cutting: phases 2 through 9 all reference it. Read this before
implementing any test code.

### What every endpoint ships

Every endpoint that lands in this codebase has, at minimum:

1. A **mocked integration test** in the library at `crates/ferro-protect/tests/<entity>.rs`,
   using `wiremock`. Exercise at least the happy path (with a committed JSON
   fixture under `crates/ferro-protect/tests/fixtures/`) plus the most relevant
   error path for that endpoint (typically 401 for auth-protected reads, 404
   for `get` endpoints, server-error paths only where the endpoint surfaces
   them specifically).
2. An **end-to-end CLI test** at `crates/ferro-protect-cli/tests/<entity>.rs`
   using `assert_cmd`, spawning the binary against a wiremock server. Assert
   both the human-readable output and `--json` output, plus exit code.
3. A **live integration test** at `crates/ferro-protect/tests/live.rs` that runs
   against a real NVR. Live tests are **not** `#[ignore]`d — they check their
   required env vars at the top of the function and skip cleanly when absent.
   `cargo test --all` runs them automatically when the developer has configured
   a real NVR, and silently no-ops them otherwise.

### Live test env-var contract

All env vars share the `UNIFI_PROTECT_` prefix with the CLI.

**This is a purely ergonomic decision for the human running live tests**,
not an architectural one. A developer who has configured their shell to
talk to their NVR (HOST + API key file) almost always wants both the live
test suite *and* ad-hoc `cargo run -p ferro-protect-cli -- info`
invocations to work from the same sourced `.env.local` -- one file, one
`source`, both work.

Earlier iterations of this plan used a distinct `FERRO_PROTECT_LIVE_*`
prefix as a belt-and-braces safeguard against `cargo test` accidentally
firing requests at a real NVR when the developer just happened to have
the CLI's env vars set. In practice that was good intent at a high
ergonomic cost (two parallel sets of env vars to maintain in every
developer's shell), and the residual risk is small: read-only live tests
calling `info` against a real NVR are harmless, and write-side tests are
gated separately by `UNIFI_PROTECT_ALLOW_MUTATIONS`. The CI guard step
below covers the one accidental-activation case that genuinely matters
(a leaked credential firing during a PR build).

- `UNIFI_PROTECT_HOST` — NVR hostname or `host:port` (no scheme prefix; the
  client always wraps as `https://{host}/proxy/protect/integration`).
  **Required**; absence means all live tests skip.
- `UNIFI_PROTECT_API_KEY_FILE` or `UNIFI_PROTECT_API_KEY` — at least one
  required when `HOST` is set. File path or raw key, respectively.
- `UNIFI_PROTECT_INSECURE` — set to a non-empty value to accept self-signed
  TLS. Honoured by both the CLI's `--insecure` flag and the live-test
  helper. Optional.
- `UNIFI_PROTECT_ALLOW_MUTATIONS` — set to `1` to permit live tests that
  write to the NVR (PATCHes, POSTs to action endpoints, file uploads).
  Optional; defaults to off so a routine `cargo test` cannot accidentally
  ring a siren, reboot a camera, or modify a recording mode. The CLI does
  not read this variable -- it is purely a test gate.

### Test naming convention

- `live_read_*` — non-mutating live tests. Skip when `HOST` is absent. Allowed
  to run freely whenever the NVR is reachable.
- `live_write_*` — mutating live tests. Skip when `HOST` is absent **or** when
  `UNIFI_PROTECT_ALLOW_MUTATIONS=1` is absent.

Implement a shared helper module at `crates/ferro-protect/tests/common/mod.rs`
that exposes:

```rust
pub fn live_client() -> Option<ProtectClient> { /* resolves HOST + key, returns None if absent */ }
pub fn mutations_allowed() -> bool { /* returns true only when ALLOW_MUTATIONS=1 */ }
```

Live tests start with:

```rust
let Some(client) = common::live_client() else { return };
// for live_write_* tests, additionally:
if !common::mutations_allowed() { return; }
```

### Helper script and env file

`scripts/live-test` is a convenience for humans: it sources `.env.local`
(gitignored) into the shell environment, then runs `cargo test --test live --
--nocapture`. `.env.example` (tracked) shows the full env var set with
placeholder values. Agents can simply set env vars directly and skip the script.

### CI safety

The CI workflow (`.github/workflows/ci.yml`) must explicitly assert that no
`UNIFI_PROTECT_*` env vars are present in the runner environment before
running `cargo test --all`. This prevents a leaked credential in repo settings
from accidentally hitting a real NVR during a routine PR build. Add as a bash
step early in the job:

```bash
if env | grep -q '^UNIFI_PROTECT_'; then
  echo "::error::UNIFI_PROTECT_* env vars must not be set in CI" >&2
  exit 1
fi
```

### Insta snapshots: scope is narrow

`insta` is used **only** for outputs of deterministic, pure transformations.
Approved snapshot targets in this codebase:

- The OpenAPI rewrite pipeline output (added between phase 1 and phase 2 as a
  chore — see PROGRESS.md).
- The CLI `--help` text for the root command and each subcommand (added in
  phase 10).
- Error message formatting for a small set of canonical, stable errors
  (e.g., the `ApiKeyError::NotProvided` text). Added in phase 10 if those
  formats have stabilized; deferred otherwise.

`insta` is **not** used for integration test response bodies. Mocked tests
assert specific fields directly (`assert_eq!(result.version, "6.2.83")`,
`assert_eq!(req.headers["X-API-Key"], "test-key")`). Live tests assert
*structural* properties (`assert!(!cameras.is_empty()); assert!(!cam.id.is_empty())`).
Snapshotting deserialized response bodies is one level removed from what we
actually want to verify, and live test outputs aren't deterministic anyway
(they depend on which devices are online, what time it is, what the NVR's
current state is).

---

## Logging

This is cross-cutting: phases 2 onward should add log calls where they add
real value. The wiring landed in a chore between phases 4b and 4c (see
PROGRESS.md). Conventions:

- **Library** (`ferro-protect`) emits through the [`log`](https://docs.rs/log)
  facade. By itself the crate produces no output -- the binary using it is
  responsible for configuring a logger. The library never initialises one.
- **CLI** (`ferro-protect-cli`) wires [`env_logger`](https://docs.rs/env_logger)
  in `crates/ferro-protect-cli/src/logging.rs`. The filter is resolved in
  this priority order: `--log-level <level>` flag > `UNIFI_PROTECT_LOG` env
  > `RUST_LOG` env > the literal default `warn`. Output goes to **stderr**
  (not stdout) so `--json` and the human tables stay parseable through
  `| jq` and friends. Levels emitted in library code today:
  - `info!` -- top-level outcome of each request ("listed N cameras",
    "fetched application info: version X"), `ProtectClient` construction
    with TLS mode label.
  - `debug!` -- breadcrumb at every request entry (`GET /v1/...`),
    timeouts at builder time.
  - `warn!` -- response-mapping fallback paths (unexpected error-body
    shape, unknown error code, etc.).
- New endpoints in phase 4+ follow the same pattern: `debug!` before the
  request, `info!` after a successful response summarising the outcome,
  `warn!` only when something unexpected happens that the user might want
  to know about but we can still proceed.
- Do not log API keys, raw request bodies, or response bodies in full.
  Cardinality (counts, ids, status codes, version strings) is fine.

---

## Architecture documentation

`ARCHITECTURE.md` at the repo root is the "start here" doc for a human (or
agent) who just cloned the repo and wants to understand the shape of the
codebase before reading any source file. It complements, not replaces, the
other docs:

- **README.md** — end-user install / run / security
- **ARCHITECTURE.md** — start-here for code readers (philosophy, invariants, file map, suggested reading order)
- **UPGRADING.md** — spec-bump procedure
- **PLAN.md** — phased build plan (this file)
- **PROGRESS.md** — chronological decision log
- **rustdoc** — API reference

Rules:

1. ARCHITECTURE.md is a **living document**. Update it whenever a phase
   changes a structural decision, adds a new module category, or introduces
   a new invariant. Most phases will not require an update — adding the
   eleventh wrapper method does not change the architecture; adding the
   first WebSocket subscriber does.
2. Keep it tight. Reader's time matters. Target ~300 lines. Push detail to
   the code or to other docs and link out.
3. Phase 10 sweep verifies the document still matches reality; any drift is
   fixed before tagging 0.1.0.
4. When adding a new top-level module or test pattern, update the file map
   in the same commit.

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
│   │   ├── build.rs                    # progenitor codegen
│   │   ├── build_support/              # shared between build.rs and tests
│   │   │   └── spec_rewrite.rs
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── client.rs
│   │   │   ├── auth.rs
│   │   │   ├── error.rs
│   │   │   ├── models.rs               # public type re-exports from generated
│   │   │   ├── generated.rs            # includes $OUT_DIR/generated.rs
│   │   │   ├── ws/                     # WebSocket layer (phase 9)
│   │   │   └── media.rs                # binary endpoints (phase 7)
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

1. Add `progenitor` (latest) and `serde_json`, `syn`, `prettyplease` to `crates/ferro-protect/Cargo.toml` under `[build-dependencies]`. Add `progenitor-client`, `reqwest` (with `json`, `stream`, `rustls-tls`), `bytes`, `chrono` (with `serde` feature), `futures-core` to `[dependencies]`.
2. Create `crates/ferro-protect/build.rs`. Hardcode a constant `const SPEC_VERSION: &str = "6.2.83";`. The spec path is derived as `third_party/unifi-apis/unifi-protect/{SPEC_VERSION}.json` (note: the submodule's folder is `unifi-protect/`, not `ferro-protect/`). The build script:
   - Prints `cargo:rerun-if-changed=` for that path.
   - Reads the file. If missing, prints a helpful error telling the user to run `git submodule update --init --recursive`.
   - Parses as `serde_json::Value`. Walks the tree to convert OpenAPI 3.1 nullable syntax (`"type": ["string", "null"]`, etc.) into 3.0-compatible `nullable: true` shape. Also bump the top-level `openapi` field to `3.0.3` if progenitor refuses 3.1.
   - Invokes `progenitor::Generator` with builder-style interface.
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

If `progenitor` fails on the spec even after the 3.1→3.0 conversion, log the failure mode in `PROGRESS.md` and try one of: (a) more aggressive spec preprocessing, (b) the openapi-generator-cli rust-async template instead, (c) hand-written types for the problematic operations only. Pick whichever gets you unblocked fastest; document the choice.

**Commit message**: `phase(1): wire up progenitor codegen from submoduled spec`

---

## Phase 2 — First end-to-end slice: `info`

**Goal**: `ferro-protect info` against a real NVR returns the application version. Library + CLI + tests, all green.

Tasks:

1. **Library**: `crates/ferro-protect/src/error.rs` — define a `unifi_protect::Error` enum with `thiserror`. Variants: `Http(reqwest::Error)`, `Api { status: u16, code: String, message: String }`, `Json(serde_json::Error)`, `InvalidUrl(String)`, `MissingApiKey`, `Other(String)`. `pub type Result<T> = std::result::Result<T, Error>;`.
2. **Library**: `crates/ferro-protect/src/auth.rs` — thin wrapper holding `SecretString` (from `secrecy` crate). Implements a `reqwest::header::HeaderValue` extractor. Header name: `X-API-Key`.
3. **Library**: `crates/ferro-protect/src/client.rs` — public `ProtectClient` struct with a `builder()` returning `ProtectClientBuilder`. Builder fields: `host`, `api_key` (SecretString), TLS mode (`Native`, `Pinned(Vec<u8>)`, `AcceptInvalid` gated behind `dangerous-tls` feature). `.build()` constructs a `reqwest::Client` with the X-API-Key default header, configured TLS, sensible timeouts (connect 10s, total 30s), HTTP/2. Base URL is `https://{host}/proxy/protect/integration`. Wraps the progenitor-generated client.
4. **Library**: implement `impl ProtectClient { pub async fn info(&self) -> Result<ApplicationInfo> }`. Map any progenitor errors into `unifi_protect::Error`.
5. **Library**: re-export the `ApplicationInfo` type (from `generated`) at `unifi_protect::models::ApplicationInfo`. Create `crates/ferro-protect/src/models.rs` for this purpose — every public type re-export from generated code lives here. Consumers must never see `crate::generated::...` types in public signatures. This is the integration seam that absorbs spec changes (see "Forward-compatibility with spec upgrades" above).
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

Human output for lists is a compact table (id, name, type, state). `--json` prints the array unmodified. The `Output` helper module (`src/output.rs`) should be introduced here so format selection lives in one place.

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

## Phase 5 — Mutating CRUD: PATCH and POST

**Goal**: configuration changes via PATCH and creates via POST, in order of increasing impact.

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

## Phase 6 — Action endpoints

**Goal**: the "do a thing" POSTs.

Order:

1. `chimes play <id>` — POST `/v1/chimes/{id}/play`.
2. `cameras ptz-goto <id> --slot <n>` — POST `/v1/cameras/{id}/ptz/goto/{slot}`.
3. `cameras ptz-patrol-start <id> --slot <n>`.
4. `cameras ptz-patrol-stop <id>`.
5. `alarm trigger <id>` — POST `/v1/alarm-manager/webhook/{id}`.

These are simple — no body shape complexity. One commit covering all action endpoints is fine, or split if any one of them is unusually complex. Tests: mocked + `assert_cmd` as always. Live tests for these are all `live_write_*` (they cause physical effects). Implement them but expect them to be exercised rarely; the mutation gate is the safety belt.

---

## Phase 7 — Binary endpoints

**Goal**: snapshots, stream URLs, talkback session info.

1. `cameras snapshot <id>` — returns `Bytes` in the library. CLI writes to `--out <PATH>`, to stdout if not a TTY, errors with friendly message if stdout is a TTY and no `--out`. Use the `is-terminal` crate. This endpoint will likely need to bypass progenitor's auto-deserialization — call the raw URL via the underlying `reqwest::Client` and return the body bytes.
2. `cameras rtsps <id>` — returns the RTSPS URL as a string. Trivial.
3. `cameras talkback <id>` — returns the WebSocket URL and codec metadata. Library exposes the structured info, CLI prints it. Out of scope: actual audio piping.

All three have `live_read_*` tests (calling them does not change NVR state). The snapshot live test asserts the body is non-empty and starts with the JPEG magic bytes (`FF D8 FF`); do not snapshot-test the bytes themselves. One commit per endpoint or one combined — your call. Log the decision.

---

## Phase 8 — Files endpoint

**Goal**: ringtone and similar file management.

1. `GET /v1/files/{fileType}` — list files of a type. `live_read_files_list`.
2. `POST /v1/files/{fileType}` — multipart upload. `live_write_files_upload`
   (gated by `common::mutations_allowed()`).

CLI: `ferro-protect files list <fileType>`, `ferro-protect files upload <fileType> <PATH>`. One commit.

---

## Phase 9 — WebSocket subscriptions

**Goal**: streaming endpoints. Last because they're the highest-risk.

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

## Phase 10 — Polish and release prep

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

**Commit message**: `phase(10): docs, polish, release 0.1.0`

---

## Reference: spec source

- Repo: <https://github.com/beezly/unifi-apis>
- Path in submodule: `third_party/unifi-apis/unifi-protect/6.2.83.json`
- Format: OpenAPI 3.1.0 (needs 3.0 down-conversion for progenitor; see phase 1)
- Base URL pattern: `https://{nvr-host}/proxy/protect/integration` (spec server is `/integration`, paths begin with `/v1/...`)
- Auth: `X-API-Key` request header
- Self-signed TLS is the default on consumer NVRs — handle gracefully

## Reference: progress log template

Copy this into `PROGRESS.md` for each entry:

```markdown
## YYYY-MM-DD HH:MM ±HHMM — Phase N: <short title>

**Status**: complete

**Summary**:
<one paragraph>

**Files added/changed**:
- path/to/file

**Decisions / deviations**:
<anything off-plan, with reasoning>

**Next**: Phase N+1 — <next thing>
```

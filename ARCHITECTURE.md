# ferro-protect — architecture

A start-here for developers (and agents) new to this codebase. The goal is to
load the shape of the project into your head before you start reading source
files, so the source files feel familiar instead of foreign.

If you want to **use** the library or CLI, read [README.md](README.md) instead.
If you want to **bump the spec version**, read [UPGRADING.md](UPGRADING.md).
If you want the day-by-day **history of decisions**, read
[PROGRESS.md](PROGRESS.md). The phased build plan lives in
[PLAN.md](PLAN.md).

This document is a living one. It is updated whenever a phase changes
structural decisions, adds a new module category, or introduces a new
invariant. See PLAN.md "Architecture documentation" for the rules.

---

## What this is

A Rust client for the UniFi Protect local integration API, plus a CLI that
exercises it. Two crates in one Cargo workspace:

- **`ferro-protect`** — async library. Wraps an OpenAPI-generated HTTP
  client behind a hand-written, ergonomic surface.
- **`ferro-protect-cli`** — `clap`-based binary named `ferro-protect`. Real
  tool; also a living integration test for the library.

The OpenAPI spec is consumed as a git submodule
(`third_party/unifi-apis`) — never vendored. One spec version is pinned at
a time via a single constant in [`crates/ferro-protect/build.rs`](crates/ferro-protect/build.rs).

---

## The shape, in one diagram

```
┌─────────────────────────────────────────────────────────────────────┐
│                     beezly/unifi-apis (submodule)                   │
│              third_party/unifi-apis/unifi-protect/*.json            │
└──────────────────────────────────┬──────────────────────────────────┘
                                   │ {SPEC_VERSION}.json
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│  crates/ferro-protect/build.rs                                      │
│  ─ reads the pinned spec                                            │
│  ─ delegates to build_support/spec_rewrite.rs::rewrite()            │
│  ─ feeds rewritten spec to progenitor                               │
│  ─ writes $OUT_DIR/generated.rs                                     │
└──────────────────────────────────┬──────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│  crates/ferro-protect/src/generated.rs                              │
│  ─ one line:  include!(concat!(env!("OUT_DIR"), "/generated.rs"))   │
│  ─ pub(crate) only — never exposed to library users                 │
│  ─ permissively #[allow(...)]'d so generated code never blocks      │
│    our pedantic+nursery clippy gate                                 │
└──────────────────────────────────┬──────────────────────────────────┘
                                   │ types::*  +  Client
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│  crates/ferro-protect/src/models.rs    ← THE SEAM                   │
│  ─ pub use crate::generated::types::Foo (optionally renamed)        │
│  ─ the ONLY place hand-written code names generated types           │
│  ─ when a spec bump renames a type, this file is the first fix      │
└──────────────────────────────────┬──────────────────────────────────┘
                                   │ models::*
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│  hand-written wrappers (src/client.rs, src/error.rs, src/auth.rs)   │
│  ─ ProtectClient / ProtectClientBuilder                             │
│  ─ Error enum + generic from_progenitor adaptor                     │
│  ─ ApiKey + X-API-Key header plumbing                               │
└──────────────────────────────────┬──────────────────────────────────┘
                                   │ public API
                                   ▼
            ┌──────────────────────┴───────────────────────┐
            ▼                                              ▼
┌─────────────────────────┐                  ┌──────────────────────────┐
│  ferro-protect-cli      │                  │  external consumers      │
│  clap CLI, JSON output  │                  │  (your code)             │
└─────────────────────────┘                  └──────────────────────────┘
```

---

## Why this shape

### Codegen, not hand-rolled types

The Protect spec defines 158 types and 35 operations. Hand-writing them
would be 95% rote and 5% interesting; worse, every spec bump would be a
manual diff session. `progenitor` consumes the OpenAPI document at build
time and emits the typed client.

But progenitor takes OpenAPI 3.0 input (via the `openapiv3` crate), and
the Protect spec is OpenAPI 3.1 with a handful of progenitor-hostile
quirks. `build_support/spec_rewrite.rs` applies a small set of rewrites
(`type: [X,null]` → `nullable: true`, `oneOf: [<T>, null]` collapse,
`const` → single-element `enum`, synthesised `operationId`s for every
operation, etc.) before handing the document to progenitor. See
[UPGRADING.md](UPGRADING.md) for the full list and the panic-site
playbook.

### A snapshot test guards the rewrites

`crates/ferro-protect/tests/spec_rewrite_snapshot.rs` runs the pinned
spec through `spec_rewrite::rewrite()` and asserts the output via `insta`.
Any change to the rewrites or the input spec becomes a reviewable diff.
The accepted `.snap` is committed; see UPGRADING.md "When the snapshot
test fails" for the review flow.

### The `models.rs` seam absorbs spec changes

Hand-written code **never** names `crate::generated::types::Foo` directly.
Every type that crosses a public signature is re-exported (and sometimes
renamed) in `src/models.rs`. When a spec bump renames `LiveviewSettings`
to `LiveviewConfig`, the fix lives in `models.rs` — wrappers downstream
keep using `models::Liveview` and never notice.

### Wrappers delegate, they do not re-implement

Every method on `ProtectClient` is a thin shim that calls the generated
client method, runs the result through `Error::from_progenitor`, and
returns a `models::*` type. Builder bodies are reused from progenitor
when present, hand-written only when the spec leaves them free-form.

### Single source of truth for the spec version

`crates/ferro-protect/build.rs::SPEC_VERSION` is the **only** place the
version is hardcoded. The spec path is derived from it.
`scripts/update-spec` rewrites this constant and reruns the check suite.

---

## Key invariants

These hold across every phase. PLAN.md treats them as non-negotiable.

1. **`#![forbid(unsafe_code)]`** at the top of every `lib.rs` and
   `main.rs`. No `unsafe` blocks anywhere, ever.
2. **No `crate::generated::...` in public signatures.** The `models.rs`
   seam is the only crossing point.
3. **One `SPEC_VERSION` constant.** Nothing else hardcodes the version.
4. **Generated code is permissively `#[allow(...)]`'d.** Hand-written
   code is held to `pedantic + nursery` clippy with `-D warnings`. The
   gate must remain meaningful — silencing it for one piece of generated
   noise must not hide a real lint elsewhere.
5. **Every commit passes all four gates** (`cargo fmt --check`, `cargo
   clippy --all-targets --all-features -- -D warnings`, `cargo test
   --all`, `cargo deny check`). The pre-commit hook at
   `scripts/pre-commit` enforces fmt + clippy locally; CI enforces all
   four.
6. **API keys live in `SecretString`** end to end — flag value, builder
   field, header value (with `set_sensitive(true)`). Never `String`.
7. **`FERRO_PROTECT_LIVE_*` prefix is reserved for live tests only.**
   CI fails fast if any such env var leaks into the runner.

---

## File map

The current state. Updated whenever the structure changes.

### Repo root

| Path | What |
|---|---|
| [Cargo.toml](Cargo.toml) | Workspace manifest. Shared `[workspace.dependencies]`, lints (`pedantic + nursery`, `unsafe_code = "forbid"`), `[profile.dev.package]` overrides for insta+similar. |
| [rust-toolchain.toml](rust-toolchain.toml) | Pins stable channel + components. |
| [rustfmt.toml](rustfmt.toml) | `edition = "2021"`, `max_width = 100`. |
| [deny.toml](deny.toml) | License allow-list, advisory checks, source allow-list (includes the unifi-apis submodule URL). |
| [.github/workflows/ci.yml](.github/workflows/ci.yml) | fmt → clippy → test → deny. Refuses to run if `FERRO_PROTECT_LIVE_*` env vars are present. |
| [scripts/pre-commit](scripts/pre-commit) | Local hook: fmt + clippy. |
| [scripts/update-spec](scripts/update-spec) | One-command spec version bump. |
| [scripts/live-test](scripts/live-test) | Sources `.env.local`, runs the live integration tests with `--features dangerous-tls`. |
| [.env.example](.env.example) | Template for `FERRO_PROTECT_LIVE_*` vars. |

### `crates/ferro-protect/` (library)

| Path | What |
|---|---|
| [Cargo.toml](crates/ferro-protect/Cargo.toml) | Library manifest. `[features] dangerous-tls = []` for opt-in insecure TLS. |
| [build.rs](crates/ferro-protect/build.rs) | Codegen entry point. Holds `SPEC_VERSION`. Delegates rewrite to `build_support/spec_rewrite.rs`. |
| [build_support/spec_rewrite.rs](crates/ferro-protect/build_support/spec_rewrite.rs) | Pure rewrite pipeline. `pub fn rewrite(serde_json::Value) -> serde_json::Value`. Imported by both `build.rs` and the snapshot test via `#[path = "..."]`. |
| [src/lib.rs](crates/ferro-protect/src/lib.rs) | Crate root. Module declarations, public re-exports, quickstart doctest. |
| [src/error.rs](crates/ferro-protect/src/error.rs) | `Error` enum + `Result` alias. Generic `Error::from_progenitor<E: Serialize>` adaptor reads `name`/`error` fields out of any spec error body. |
| [src/auth.rs](crates/ferro-protect/src/auth.rs) | `ApiKey(SecretString)` wrapper. `API_KEY_HEADER` constant. |
| [src/models.rs](crates/ferro-protect/src/models.rs) | **The seam.** Public re-exports from `generated::types::*`. |
| [src/client.rs](crates/ferro-protect/src/client.rs) | `ProtectClient`, `ProtectClientBuilder`, `TlsMode`. The user-facing surface. |
| [src/generated.rs](crates/ferro-protect/src/generated.rs) | Three lines: a permissive `#![allow(...)]` block and `include!(env!("OUT_DIR") + "/generated.rs")`. `pub(crate)` only. |
| [tests/info.rs](crates/ferro-protect/tests/info.rs) | Mocked integration test for `client.info()` (wiremock). |
| [tests/live.rs](crates/ferro-protect/tests/live.rs) | Live tests against a real NVR. Auto-skip when env absent. |
| [tests/common/mod.rs](crates/ferro-protect/tests/common/mod.rs) | `live_client() -> Option<ProtectClient>`, `mutations_allowed() -> bool`. Pulled in by each live test via `mod common;`. |
| [tests/spec_rewrite_snapshot.rs](crates/ferro-protect/tests/spec_rewrite_snapshot.rs) | Insta snapshot test for the rewrite pipeline output. |
| [tests/fixtures/](crates/ferro-protect/tests/fixtures/) | Canned JSON for wiremock tests. |
| [tests/snapshots/](crates/ferro-protect/tests/snapshots/) | Accepted insta snapshots. |

### `crates/ferro-protect-cli/` (CLI)

| Path | What |
|---|---|
| [Cargo.toml](crates/ferro-protect-cli/Cargo.toml) | CLI manifest. Depends on `ferro-protect` with `dangerous-tls` enabled (so `--insecure` works). |
| [src/main.rs](crates/ferro-protect-cli/src/main.rs) | `clap`-derive CLI. Global args + `Info` subcommand. Phase-2 temporary `--api-key` flag, slated for removal in phase 3. |
| [tests/info.rs](crates/ferro-protect-cli/tests/info.rs) | `assert_cmd` end-to-end test against wiremock. |

### `third_party/unifi-apis/` (submodule)

The OpenAPI specs published at <https://github.com/beezly/unifi-apis>. Pinned
at a specific commit. The currently pinned spec is at
`third_party/unifi-apis/unifi-protect/{SPEC_VERSION}.json`.

---

## Error model

A single public `Error` enum lives in [`src/error.rs`](crates/ferro-protect/src/error.rs):

```rust
pub enum Error {
    Http(reqwest::Error),           // transport-level failure
    Api { status, code, message },  // server returned an error response
    Json(serde_json::Error),        // response body didn't match schema
    InvalidUrl(String),
    MissingApiKey,
    Other(String),
}
```

`Error::from_progenitor<E: Serialize>` is the bridge from progenitor's
`Error<E>` to ours. It is generic so every operation in the generated
client (which carries its own error-body schema) maps through one
function. The adaptor serialises the error body to JSON and pulls `name`
and `error` fields — the shape the Protect spec consistently uses. Any
body without those fields falls back to stringified output.

When a new endpoint surfaces a novel error body shape, the right move is
usually to extend the JSON-field probe in `extract_code_and_message`, not
to add a new wrapper. The whole point is one Error type, one adaptor.

---

## Auth and TLS

API keys are held in [`secrecy::SecretString`](https://docs.rs/secrecy)
from the moment they enter the program (CLI flag, env var, file) all the
way to the `HeaderValue` that goes on the wire. The header value is
marked sensitive (`set_sensitive(true)`) so it does not leak into debug
output.

Three TLS modes ([`TlsMode`](crates/ferro-protect/src/client.rs)):

- `Native` (default) — webpki-roots via rustls. Use when the NVR ships a
  certificate signed by a recognised authority.
- `Pinned(Vec<u8>)` — PEM bytes for a specific root cert. The safe option
  for self-signed NVRs.
- `AcceptInvalid` — disables verification entirely. Gated behind the
  `dangerous-tls` cargo feature. The CLI enables this feature so
  `--insecure` works; library consumers must opt in.

---

## Testing model

Three layers, all driven by `cargo test --all`. None are `#[ignore]`d.

1. **Mocked integration** — every endpoint has a `wiremock`-based test in
   `crates/ferro-protect/tests/<entity>.rs` asserting both happy paths
   (with a committed JSON fixture under `tests/fixtures/`) and the most
   relevant error path.
2. **End-to-end CLI** — `assert_cmd` spawns the built binary against a
   `wiremock` server in `crates/ferro-protect-cli/tests/<entity>.rs`,
   asserting exit code, human stdout, and `--json` stdout. CLI tests
   wrap the `Command` invocation in `tokio::task::spawn_blocking` so the
   sync `assert_cmd::Command::assert()` does not block the same Tokio
   reactor hosting the mock server.
3. **Live integration** — `crates/ferro-protect/tests/live.rs` runs
   against a real NVR. Each test calls `common::live_client()` at the
   top; if `FERRO_PROTECT_LIVE_HOST` is unset the helper returns `None`
   and the test early-returns with a printed skip message. When `HOST`
   is set but no key source is, the helper *panics* — a half-configured
   live env is almost always a developer mistake we want surfaced.
   Mutating tests (`live_write_*`, coming in later phases) gate
   additionally on `FERRO_PROTECT_LIVE_ALLOW_MUTATIONS=1` via
   `common::mutations_allowed()`.

`insta` snapshots are used **only** for outputs of deterministic, pure
transformations: the spec rewrite pipeline (now), and the CLI `--help` /
canonical error-message text (planned for phase 10). Snapshots are
deliberately not used for response bodies — those should be asserted on
specific fields so a test's intent stays readable.

CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) explicitly
errors out if any `FERRO_PROTECT_LIVE_*` env var is present in the
runner environment, so a leaked secret cannot accidentally hit a real
NVR from a PR build.

---

## Where to start reading

Suggested order if you want to understand the code end to end:

1. [`crates/ferro-protect/src/lib.rs`](crates/ferro-protect/src/lib.rs) —
   2 minutes. The module shape and what's public.
2. [`crates/ferro-protect/src/models.rs`](crates/ferro-protect/src/models.rs) —
   30 seconds. Now you've seen the seam.
3. [`crates/ferro-protect/src/client.rs`](crates/ferro-protect/src/client.rs) —
   5 minutes. The builder, the TLS modes, the `info()` shim — read this
   to understand the shape every future wrapper method will follow.
4. [`crates/ferro-protect/src/error.rs`](crates/ferro-protect/src/error.rs) —
   3 minutes. Especially `from_progenitor` — that adaptor is the
   linchpin of error mapping across every endpoint in later phases.
5. [`crates/ferro-protect/build.rs`](crates/ferro-protect/build.rs) and
   [`crates/ferro-protect/build_support/spec_rewrite.rs`](crates/ferro-protect/build_support/spec_rewrite.rs) —
   10 minutes. Skim the rewrite cases; do not try to memorise.
   [`UPGRADING.md`](UPGRADING.md) is the reference for when something
   in here breaks.
6. [`crates/ferro-protect-cli/src/main.rs`](crates/ferro-protect-cli/src/main.rs) —
   3 minutes. End-user shape.
7. [`crates/ferro-protect/tests/`](crates/ferro-protect/tests/) — pick
   one file, see the pattern future tests will follow.

After all that, [PLAN.md](PLAN.md) tells you what is intentionally
deferred and where it is going next. [PROGRESS.md](PROGRESS.md) tells
you why each past decision was made — read it when something in the
code surprises you and you suspect there is a non-obvious reason.

---

## Status

Pre-0.1.0. The library currently supports one endpoint (`GET /v1/meta/info`).
Phases 3 through 9 deliver the remaining 34 operations, the WebSocket
subscribers, and the binary-payload endpoints. Phase 10 polishes for
release. See [PLAN.md](PLAN.md) for the full roadmap.

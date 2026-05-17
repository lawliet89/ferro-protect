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

- **`ferro-protect`** — async library. Uses typify-generated models behind a
  hand-written `reqwest` client surface.
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
│  ─ feeds component schemas to typify                                │
│  ─ writes $OUT_DIR/generated.rs                                     │
└──────────────────────────────────┬──────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│  crates/ferro-protect/src/generated.rs                              │
│  ─ one meaningful line:                                             │
│      include!(concat!(env!("OUT_DIR"), "/generated.rs"))            │
│  ─ private module — only models.rs re-exports from it               │
│  ─ permissively #[allow(...)]'d so generated code never blocks      │
│    our pedantic+nursery clippy gate                                 │
└──────────────────────────────────┬──────────────────────────────────┘
                                   │ generated model types
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│  crates/ferro-protect/src/models.rs    ← THE SEAM                   │
│  ─ pub use crate::generated::Foo (optionally renamed)               │
│  ─ the ONLY place hand-written code names generated types           │
│  ─ when a spec bump renames a type, this file is the first fix      │
└──────────────────────────────────┬──────────────────────────────────┘
                                   │ models::*
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│  hand-written wrappers (src/client.rs, src/error.rs, src/auth.rs)   │
│  ─ ProtectClient / ProtectClientBuilder                             │
│  ─ shared reqwest helpers + uniform Error mapping                   │
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

The Protect spec defines 158 types and 35 operations. Hand-writing the models
would be 95% rote and 5% interesting; worse, every spec bump would be a
manual diff session. `typify` consumes the OpenAPI component schemas at build
time and emits the Rust model types.

The Protect spec is OpenAPI 3.1, so `build_support/spec_rewrite.rs` applies a
small schema-only preprocessing pass (`const` → single-element `enum`,
nullable `type` arrays to `anyOf`, singleton `allOf` flattening, etc.) before
handing the schemas to typify. See
[UPGRADING.md](UPGRADING.md) for the full list and the panic-site
playbook.

### A model smoke test guards the seam

`crates/ferro-protect/tests/model_codegen.rs` touches representative
`models::*` re-exports and round-trips the inline `ApplicationInfo` response.
It is intentionally small: build failures and this test catch type rename or
derive drift without snapshotting the full generated source.

### The `models.rs` seam absorbs spec changes

Hand-written code **never** names `crate::generated::Foo` directly.
Every type that crosses a public signature is re-exported (and sometimes
renamed) in `src/models.rs`. When a spec bump renames `LiveviewSettings`
to `LiveviewConfig`, the fix lives in `models.rs` — wrappers downstream
keep using `models::Liveview` and never notice.

### Wrappers stay mechanical

Every wrapper method is a thin shim around shared HTTP helpers on
`ProtectClient` (`get_json`, `post_json`, `patch_json`, `get_bytes`) and
returns a `models::*` type. URL joining, JSON decoding, byte responses, and
non-2xx error mapping stay in those helpers.

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
7. **`UNIFI_PROTECT_*` env vars are forbidden in CI.** Both the CLI and
   the live tests read this prefix; their presence in a CI runner would
   silently hit a real NVR. The CI workflow fails fast if any such var
   is set.

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
| [.github/workflows/ci.yml](.github/workflows/ci.yml) | fmt → clippy → test → deny. Refuses to run if `UNIFI_PROTECT_*` env vars are present. |
| [scripts/pre-commit](scripts/pre-commit) | Local hook: fmt + clippy. |
| [scripts/update-spec](scripts/update-spec) | One-command spec version bump. |
| [scripts/live-test](scripts/live-test) | Sources `.env.local`, runs the live integration tests with `--features dangerous-tls`. |
| [.env.example](.env.example) | Template for `UNIFI_PROTECT_*` vars. |

### `crates/ferro-protect/` (library)

| Path | What |
|---|---|
| [Cargo.toml](crates/ferro-protect/Cargo.toml) | Library manifest. `[features] dangerous-tls = []` for opt-in insecure TLS. |
| [build.rs](crates/ferro-protect/build.rs) | Codegen entry point. Holds `SPEC_VERSION`. Delegates rewrite to `build_support/spec_rewrite.rs`. |
| [build_support/spec_rewrite.rs](crates/ferro-protect/build_support/spec_rewrite.rs) | Pure schema preprocessing pipeline. `pub fn rewrite(serde_json::Value) -> serde_json::Value`. |
| [src/lib.rs](crates/ferro-protect/src/lib.rs) | Crate root. Module declarations, public re-exports, quickstart doctest. |
| [src/error.rs](crates/ferro-protect/src/error.rs) | `Error` enum + `Result` alias. Non-2xx responses are mapped from `{ name, error/message }` bodies with a raw-body fallback. |
| [src/auth.rs](crates/ferro-protect/src/auth.rs) | `ApiKey(SecretString)` wrapper. `API_KEY_HEADER` constant. |
| [src/models.rs](crates/ferro-protect/src/models.rs) | **The seam.** Public re-exports from generated model types plus tiny hand-written inline response models. |
| [src/client.rs](crates/ferro-protect/src/client.rs) | `ProtectClient`, `ProtectClientBuilder`, `TlsMode`. The user-facing surface. |
| [src/generated.rs](crates/ferro-protect/src/generated.rs) | A permissive `#![allow(...)]` block and `include!(concat!(env!("OUT_DIR"), "/generated.rs"))`. Declared as a private `mod generated;` in `lib.rs`; only `models.rs` re-exports from it. |
| [src/cameras.rs](crates/ferro-protect/src/cameras.rs) | `CamerasApi<'a>` (list + get). Sample of the per-entity wrapper pattern phase 4 rolls out. |
| [src/chimes.rs](crates/ferro-protect/src/chimes.rs) | `ChimesApi<'a>` (list + get). Same shape as cameras. |
| [tests/info.rs](crates/ferro-protect/tests/info.rs) | Mocked integration test for `client.info()` (wiremock). |
| [tests/live.rs](crates/ferro-protect/tests/live.rs) | Live tests against a real NVR. Auto-skip when env absent. |
| [tests/common/mod.rs](crates/ferro-protect/tests/common/mod.rs) | `live_client() -> Option<ProtectClient>`, `mutations_allowed() -> bool`. Pulled in by each live test via `mod common;`. |
| [tests/model_codegen.rs](crates/ferro-protect/tests/model_codegen.rs) | Smoke test for generated-model re-exports and inline response models. |
| [tests/fixtures/](crates/ferro-protect/tests/fixtures/) | Canned JSON for wiremock tests. |

### `crates/ferro-protect-cli/` (CLI)

| Path | What |
|---|---|
| [Cargo.toml](crates/ferro-protect-cli/Cargo.toml) | CLI manifest. Depends on `ferro-protect` with `dangerous-tls` enabled (so `--insecure` works). |
| [src/main.rs](crates/ferro-protect-cli/src/main.rs) | `clap`-derive CLI. Global args (`--host`, `--api-key-file`, `--insecure`, `--json`, `--log-level`) + subcommands (`info`, `cameras`, `chimes`, …). |
| [src/lib.rs](crates/ferro-protect-cli/src/lib.rs) | Library half so integration tests can reach internals (`api_key`, `commands`, `output`, `logging`). |
| [src/api_key.rs](crates/ferro-protect-cli/src/api_key.rs) | Resolver with `--api-key-file` > `UNIFI_PROTECT_API_KEY_FILE` > `UNIFI_PROTECT_API_KEY` precedence; injects warnings through an `io::Write` so callers can capture or stream them. |
| [src/logging.rs](crates/ferro-protect-cli/src/logging.rs) | `env_logger` setup: flag > `UNIFI_PROTECT_LOG` > `RUST_LOG` > `warn`. Writes to stderr. |
| [src/output.rs](crates/ferro-protect-cli/src/output.rs) | `emit()` (JSON-or-human dispatch) + `table()` (fixed-column renderer). |
| [src/commands/](crates/ferro-protect-cli/src/commands/) | Per-entity subcommand handlers. One file per entity. |
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

The shared HTTP helpers map non-2xx responses into `Error::Api`. They read the
Protect error shape (`name` plus `error` or `message`) and fall back to a
truncated raw body when the shape is unexpected.

When a new endpoint surfaces a novel error body shape, the right move is
usually to extend that single mapper, not to add endpoint-local error code.

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

## Logging

The library emits structured log records through the
[`log`](https://docs.rs/log) facade and **does not** configure a logger.
The CLI wires `env_logger` in
[`src/logging.rs`](crates/ferro-protect-cli/src/logging.rs); filter
resolution is `--log-level` flag > `UNIFI_PROTECT_LOG` > `RUST_LOG` >
the literal `warn` default. Output goes to stderr so `--json` / table
output on stdout stays parseable.

Levels emitted in library code:

- `info!` -- top-level request outcome ("listed N cameras"), client
  construction with TLS-mode label
- `debug!` -- per-request breadcrumb (`GET /v1/…`), timeout values at
  build time
- `warn!` -- response-mapping fallback paths (unexpected error-body
  shape, missing `name`/`error` field, etc.)

We deliberately do not log API keys, raw request/response bodies, or
anything else high-cardinality enough to leak content. Counts, ids,
status codes, and version strings are fine.

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
   top; if `UNIFI_PROTECT_HOST` is unset the helper returns `None`
   and the test early-returns with a printed skip message. When `HOST`
   is set but no key source is, the helper *panics* — a half-configured
   live env is almost always a developer mistake we want surfaced.
   Mutating tests (`live_write_*`, coming in later phases) gate
   additionally on `UNIFI_PROTECT_ALLOW_MUTATIONS=1` via
   `common::mutations_allowed()`.

`insta` snapshots are reserved for outputs of deterministic, pure
transformations. None are currently active — the spec rewrite snapshot
was retired when typify replaced progenitor (the rewrite layer shrank
so much it no longer warrants a snapshot, and
`tests/model_codegen.rs` covers the seam). Phase 10 will add snapshots
for the CLI `--help` text and canonical error messages; the `insta` +
`similar` `[profile.dev.package]` overrides in the workspace
[Cargo.toml](Cargo.toml) stay in place for that future use. Snapshots
are deliberately not used for response bodies — those should be
asserted on specific fields so a test's intent stays readable.

CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) explicitly
errors out if any `UNIFI_PROTECT_*` env var is present in the
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
   3 minutes. The response mapper is the linchpin of error handling across
   every endpoint in later phases.
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

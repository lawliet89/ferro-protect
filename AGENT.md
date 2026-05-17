# AGENT.md — operating instructions for coding agents in this repo

If you are an agent (Claude Code or otherwise) working on this codebase,
read this file before touching anything else. It is the cross-cutting
contract: commit conventions, signing, gates, invariants, testing rules,
logging conventions, and how to leave a useful trail behind you. It is
deliberately short — the per-task detail lives elsewhere.

For *what* to work on:

- **Phased build plan** — [PLAN.md](PLAN.md). Phases 0-10 plus deferred
  items. Read only the phase or section relevant to your task.
- **One-off chores** — [docs/TASK_*.md](docs/). Self-contained briefs
  (e.g. [docs/TASK_oas3_migration.md](docs/TASK_oas3_migration.md),
  [docs/TASK_cli_output.md](docs/TASK_cli_output.md)). Each task doc
  links its prerequisites; you do not need PLAN.md to do them.

For *how the code is shaped*:

- **Architecture overview** — [ARCHITECTURE.md](ARCHITECTURE.md).
  Diagram, file map, invariants, reading order.
- **Spec upgrade procedure** — [UPGRADING.md](UPGRADING.md).
- **Historical decisions** — [PROGRESS.md](PROGRESS.md). Chronological
  log; read when something in the code surprises you.

---

## Orienting yourself before changing anything

1. Read PROGRESS.md from the bottom up until you understand what was
   most recently in flight. The phase/chore you've been asked to do may
   have prerequisites or recent deviations not yet reflected in PLAN.md.
2. `git status` and `git log -10 --oneline` to confirm the branch state
   matches what the user implied.
3. Read [ARCHITECTURE.md](ARCHITECTURE.md) if you have not already in
   this session — it is the fastest path from zero to "I know where
   things live."
4. Only then start editing.

## Repository state assumptions

The user has already run `git init`. Treat the working directory as
your repo root. Do **not** run `git init`, do **not** rename `main`, do
**not** rewrite remote configuration.

The submodule at `third_party/unifi-apis` may or may not be checked out
when you start. If you need it and it is empty, run:

```sh
git submodule update --init --recursive
```

## Commit policy

- One logical unit of work, one commit. For phases, that is "one phase,
  one commit." For chores, "one chore, one commit." Squash if you need
  to.
- Use Conventional Commits style:
  - `phase(N): <short description>` — phase work
  - `chore(<scope>): <short description>` — anything else
  - `fix(<scope>): <short description>` — bug fix
  - `docs(<scope>): <short description>` — doc-only change
- Include a longer body explaining *why*. Cross-reference the phase or
  task doc that drove the work.
- Commit message body uses HEREDOC to preserve formatting. Co-author
  trailer goes at the end:

  ```sh
  git commit -m "$(cat <<'EOF'
  phase(N): <short description>

  <longer body>

  Co-Authored-By: Claude <noreply@anthropic.com>
  EOF
  )"
  ```

- Only commit when explicitly asked. Do not push unless explicitly
  asked. Never force-push.

## PGP signing — non-negotiable

The user has commit signing configured and may require a passphrase.

- **Never** use `--no-gpg-sign`, `-S` with a hardcoded key,
  `--no-verify`, or any flag that bypasses signing or hooks.
- Attempt the commit normally. If it fails because of a passphrase
  prompt, signing key issue, or anything signing-related:
  1. Stop. Do not retry with workarounds.
  2. Tell the user the GPG cache likely needs warming. The user's
     standard warm-up command is:
     ```sh
     echo unlock | gpg --clearsign --local-user 77820C080DD7DFC5 > /dev/null
     ```
  3. Note the staged files and the exact commit message, so the user
     can either tell you "done, retry" or run it themselves.
  4. Wait. Do not proceed past the commit.
- Same applies to `git push` if push signing is configured.

## Guardrails (enforced on every commit)

All four must be green before you commit. Run them yourself; do not
assume.

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo deny check
```

Additional invariants enforced in code:

- `#![forbid(unsafe_code)]` at the top of every `lib.rs` and `main.rs`.
  No `unsafe` blocks anywhere, ever. If you think you need one, stop
  and log the reason in PROGRESS.md before continuing.
- If a clippy lint cannot be resolved without compromising design, add
  `#[allow(...)]` on the smallest possible scope with a one-line
  comment explaining why, and log the decision in PROGRESS.md.

## Progress logging

Maintain `PROGRESS.md` at the repo root. Create it on first run.
Append a new entry whenever you finish a phase, complete a chore,
deviate from a plan, or hit something that surprises you. One entry,
one decision (do not batch).

### Entry format

```markdown
## YYYY-MM-DD HH:MM ±HHMM — <phase or chore title>

**Status**: complete | partial | blocked

**Summary**:
<one paragraph>

**Files added/changed**:
- path/to/file

**Decisions / deviations**:
<anything off-plan, with reasoning>

**Next**: <what comes next, or what's blocking>
```

Use a real timestamp captured at the moment you write the entry. The
timezone offset is part of the format — drop it and entries written
months apart from different machines lose ordering. On Unix:

```sh
date +"%Y-%m-%d %H:%M %z"
# 2026-05-17 11:05 +0800
```

### Commit timing for PROGRESS.md

The PROGRESS.md entry for phase N is committed at the *start* of the
next phase, not in the same commit as the work it describes. This
keeps each phase commit clean and topical. The final phase's log entry
goes in its own follow-up commit.

For chores (not phases), commit the PROGRESS entry together with the
chore — there is no "next phase" to bundle it with.

## Invariants you must preserve

These are non-negotiable across every phase and chore. Violating them
silently makes future spec bumps painful and breaks the seams the
project relies on.

1. **Single source of truth for the spec version.** The Protect spec
   version lives in `crates/ferro-protect/build.rs::SPEC_VERSION` and
   nowhere else. The spec path is derived from it. To bump, use
   `scripts/update-spec`; never hand-edit the constant in code you
   are also changing for an unrelated reason.

2. **Hand-written code never names `crate::generated::...` types.**
   Every type that crosses a public signature is re-exported (and,
   where it helps, renamed) in
   [crates/ferro-protect/src/models.rs](crates/ferro-protect/src/models.rs).
   When a spec bump renames a type, `models.rs` is the first (often
   only) fix-site.

3. **Wrappers are mechanical.** Each entity wrapper method
   (`CamerasApi::list`, `ChimesApi::get`, etc.) is a one-liner over the
   shared HTTP helpers on `ProtectClient` (`get_json`, `post_json`,
   `patch_json`, `send_no_content`, `get_bytes`) and returns a
   `models::*` type. Never construct request URLs or bodies inline in
   business logic — keep that mechanical layer in the helpers so future
   endpoints stay uniform.

4. **PATCH bodies use generated types where the spec defines them.**
   Where the spec only exposes a free-form schema, define a
   hand-written builder with
   `#[serde(skip_serializing_if = "Option::is_none")]` and add a
   comment pointing back to the spec path.

5. **API keys live in `SecretString` end to end.** From flag value to
   builder field to `HeaderValue` (with `set_sensitive(true)`). Never
   plain `String`.

6. **`UNIFI_PROTECT_*` env vars are forbidden in CI.** The CI workflow
   refuses to run if any are present. Both the CLI and live tests
   read this prefix; their presence in a CI runner would silently hit
   a real NVR.

If a task tempts you to violate one of these, stop. Either reshape the
task or log the reason for the deviation in PROGRESS.md and surface it
to the user before proceeding.

## Testing strategy

Cross-cutting: every endpoint added in any phase follows this.

### What every endpoint ships

1. **Mocked integration test** at
   `crates/ferro-protect/tests/<entity>.rs` using `wiremock`. Happy
   path with a committed JSON fixture under `tests/fixtures/`, plus
   the most relevant error path (401 for auth-protected reads, 404
   for `get` endpoints, etc.).
2. **End-to-end CLI test** at
   `crates/ferro-protect-cli/tests/<entity>.rs` using `assert_cmd`.
   Spawns the binary against a wiremock server. Assert exit code,
   human stdout, and `--json` stdout. Wrap the `Command` invocation in
   `tokio::task::spawn_blocking` so sync `assert_cmd::Command::assert`
   does not block the same Tokio reactor hosting the mock.
3. **Live integration test** at `crates/ferro-protect/tests/live.rs`
   that runs against a real NVR. **Not** `#[ignore]`d — it checks env
   vars at the top of the function and skips cleanly when absent.

### Live test env-var contract

All vars share the `UNIFI_PROTECT_` prefix with the CLI.
Single sourced `.env.local` drives both the CLI and the live tests.

- `UNIFI_PROTECT_HOST` — NVR hostname or `host:port`, no scheme
  prefix. **Required.** Absence means all live tests skip.
- `UNIFI_PROTECT_API_KEY_FILE` or `UNIFI_PROTECT_API_KEY` — at least
  one required when `HOST` is set. File path or raw key.
- `UNIFI_PROTECT_INSECURE` — set to a non-empty/truthy value to
  accept self-signed TLS. Honoured by both the CLI's `--insecure`
  flag and the live-test helper.
- `UNIFI_PROTECT_ALLOW_MUTATIONS=1` — permits `live_write_*` tests
  that change NVR state. Off by default so routine `cargo test`
  cannot accidentally ring a siren, reboot a camera, etc. CLI does
  not read this; purely a test gate.

If `HOST` is set but no key source is, the test helper panics with a
clear message rather than silently skipping. A half-configured live
env is almost always a developer mistake.

### Test naming convention

- `live_read_*` — non-mutating. Skip when `HOST` absent.
- `live_write_*` — mutating. Skip when `HOST` absent **or** when
  `UNIFI_PROTECT_ALLOW_MUTATIONS=1` absent.

Shared helpers in `crates/ferro-protect/tests/common/mod.rs`:

```rust
pub fn live_client() -> Option<ProtectClient>;
pub fn mutations_allowed() -> bool;
```

Live tests start with:

```rust
let Some(client) = common::live_client() else { return };
// for live_write_* additionally:
if !common::mutations_allowed() { return; }
```

### Helper script

`scripts/live-test` sources `.env.local` (gitignored) into the shell
environment and runs the live tests with `--features insecure-tls`
and `--nocapture`. Agents can set env vars directly and skip the
script.

### Insta snapshots — narrow scope

`insta` is used **only** for outputs of deterministic, pure
transformations. Approved targets:

- CLI `--help` text for the root command and each subcommand (phase
  11).
- Canonical, stable error message formatting (phase 11).
- The OpenAPI rewrite pipeline output — *retired* when typify
  replaced progenitor; the smaller `tests/model_codegen.rs` covers
  the seam now.

`insta` is **not** used for integration response bodies. Mocked tests
assert specific fields. Live tests assert structural properties
(`!cameras.is_empty()`). Snapshotting deserialised response bodies is
one level removed from what we actually want to verify.

## Logging conventions

- **Library (`ferro-protect`)** emits through the
  [`log`](https://docs.rs/log) facade. Never initialises a logger.
- **CLI (`ferro-protect-cli`)** wires `env_logger` in
  `crates/ferro-protect-cli/src/logging.rs`. Filter precedence:
  `--log-level` flag > `UNIFI_PROTECT_LOG` env > `RUST_LOG` env >
  literal default `warn`. Output to **stderr** so `--json` and human
  tables on stdout stay parseable.
- Levels emitted in library code:
  - `info!` — top-level request outcome ("listed N cameras"),
    `ProtectClient` construction with TLS mode label.
  - `debug!` — breadcrumb at every request entry (`GET /v1/...`),
    timeouts at builder time.
  - `warn!` — response-mapping fallback paths (unexpected error-body
    shape, unknown error code).
- New endpoints follow the same pattern. `debug!` before, `info!` on
  success, `warn!` only when something unexpected happens.
- **Do not log API keys, raw request bodies, or response bodies in
  full.** Cardinality (counts, ids, status codes, version strings) is
  fine.

## Architecture documentation maintenance

[ARCHITECTURE.md](ARCHITECTURE.md) is a *living document* — the
"start here" for a human or agent who just cloned the repo.

- Update it whenever a phase or chore changes a structural decision,
  adds a new module category, or introduces a new invariant. Most
  changes do not require an update — adding the eleventh wrapper
  method does not change the architecture; adding the first
  WebSocket subscriber does.
- Keep it tight. Target ~350 lines. Push detail to code or to other
  docs and link out.
- When adding a new top-level module or test pattern, update the
  file map in the same commit.
- Phase 11's sweep verifies the document still matches reality
  before tagging 0.1.0.

## Working style

- Prefer many small, well-tested changes over big sweeping ones.
- Every phase or chore ends with the four gates green before commit.
- When a task says "library + CLI + tests", deliver all three before
  it is done. No half-phases.
- If a plan is unclear or the spec contradicts the plan, log the
  question and pick the most defensible interpretation. Do not block
  on small clarifications; surface them in PROGRESS.md.
- When you encounter a deviation worth remembering later, add an
  entry to PLAN.md's "Deferred — revisit before 0.1.0" section
  (with trigger condition) rather than only leaving a TODO comment.

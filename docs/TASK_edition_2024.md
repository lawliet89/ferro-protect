# Chore: migrate workspace to Rust edition 2024

## Why

Edition 2021 is one edition behind. Staying on it leaves the
*edition-gated* changes off the table — chiefly the new prelude
additions (`Future` / `IntoFuture`), the now-`unsafe`
`std::env::set_var` / `remove_var` (which pairs naturally with the
existing `#![forbid(unsafe_code)]` invariant), and the 2024 style
edition for rustfmt. The bump is also a convenient moment to
opt into adjacent *non-edition* changes the project has been
carrying forward by inertia: MSRV-aware Cargo dependency
resolution (`resolver = "3"`, available since Rust 1.84) and the
`#[expect]` attribute (compiler-gated, stable since Rust 1.81).
None of those second-bucket changes *require* the edition flip
— bundling them just means they all land in one reviewable commit
instead of trickling in over follow-up chores.

The cost of doing it now is small. The codebase is pre-0.1.0 with no
external consumers, so a `rust-version` bump is free. A survey of the
hand-written source turned up zero usages of the patterns that 2024
breaks loudest:

| Pattern that 2024 breaks                              | Usages in repo |
|---|---|
| `std::env::set_var` / `remove_var` (now `unsafe`)     | 0 |
| `extern "C" { ... }` blocks (now need `unsafe extern`)| 0 |
| `#[no_mangle]` / `#[export_name]` / `#[link_section]` | 0 (forbidden anyway) |
| `gen` as identifier (reserved keyword)                | 0 |
| `impl Trait` returns in public API (RPIT capture)     | 0 in hand-written code; `typify`-generated needs a build check |
| Lock guards or other `Drop` temporaries in tail-expression position (new scope rules) | 0 obvious; needs `cargo fix --edition` verification |

The most likely real work is in code we don't write: `typify`'s
generated `$OUT_DIR/generated.rs` and any auto-fixes `cargo fix
--edition` proposes for `if let` temporary scopes. Both surface as
build / clippy failures and are bounded.

## Timing

**Standalone chore. Do it between phases, not inside one.** It
touches every crate's `Cargo.toml` and potentially every `*.rs`
file (via `cargo fix --edition`), so bundling it with a feature
phase would smear the diff and make the next spec bump harder to
review. There is no phase in PLAN.md that *requires* 2024 features
to land first — this is debt-paydown, not a blocker.

The chore is also a natural gate-tightener: any code added after
the bump gets the new lints (`if-let-rescope`, `tail-expr-drop-order`,
`unsafe-attr-outside-unsafe`, etc.) for free.

## Branch and commit policy

Work on a branch, not `main`. Suggested name:

```sh
git checkout -b chore/edition-2024
```

Follow the rules in [AGENT.md](../AGENT.md) exactly:

- One chore, one commit. Conventional Commits style:
  `chore(workspace): migrate to Rust edition 2024`.
- Body explains the MSRV bump (1.83 → 1.85), the `resolver` bump
  (2 → 3), and any non-mechanical fix-ups `cargo fix --edition`
  did not handle.
- Co-author trailer at the end, HEREDOC commit body, no
  `--no-verify`, no `--no-gpg-sign`. If GPG passphrase prompts,
  warm the cache per AGENT.md and hand back to the user.
- Update `PROGRESS.md` in the *same* commit as the chore (chores
  don't have a "next phase" to bundle the log entry with — that
  rule is phases-only).
- Do **not** push or open a PR unless the user explicitly asks.

## Action items

### 1. MSRV and toolchain

In `Cargo.toml` (workspace):

- `workspace.package.rust-version`: `"1.83"` → `"1.85"` (the
  minimum that supports edition 2024).
- `workspace.resolver`: `"2"` → `"3"`. Resolver 3 honours
  `rust-version` when picking dependency versions, so a future
  contributor on a fresh `cargo update` won't accidentally pull a
  crate that requires a newer toolchain than `rust-version`
  advertises.

`rust-toolchain.toml` already pins `channel = "stable"` with
`rustfmt` + `clippy`; no change needed — stable is well past 1.85.
Leave the file as-is.

CI (`.github/workflows/ci.yml`) uses
`dtolnay/rust-toolchain@stable`, so it picks up the new toolchain
automatically. No CI workflow edits expected. Confirm by re-running
the workflow on the branch.

### 2. Per-crate edition field

In `Cargo.toml` (workspace):

```toml
[workspace.package]
edition = "2024"   # was "2021"
```

Both member crates (`ferro-protect`, `ferro-protect-cli`) inherit
via `edition.workspace = true`, so no per-crate edits needed.

### 3. Run the automated migration

From the repo root, on the branch:

```sh
cargo fix --edition --all-features --allow-dirty --allow-staged
```

This emits the migration lints introduced by 2024 (notably
`if-let-rescope` and `tail-expr-drop-order`) and applies fixes that
preserve current behaviour. Review the diff carefully — `cargo fix`
sometimes inserts redundant `let _ = ...` bindings to preserve
drop timing. Replace those with the smallest equivalent that reads
naturally (e.g. an explicit `drop(guard)`).

Then bump the edition field (step 2). The order matters: `cargo fix
--edition` migrates *from* the current edition to the next, so it
must run before flipping the field.

Re-run `cargo fmt --all` after the auto-fix; the inserted bindings
are not always idiomatically formatted.

### 4. Re-check `build.rs`-generated code

`crates/ferro-protect/build.rs` feeds the spec through `typify` and
writes `$OUT_DIR/generated.rs`. `typify` emits its output without
an explicit edition pragma, so the generated file inherits the
crate's edition. Two things can bite:

- **RPIT capture rules.** Edition 2024 changes the default lifetime
  capture for `impl Trait` returns. If `typify` emits any
  `fn foo(...) -> impl Iterator<...>` style signatures, they may now
  capture more lifetimes than intended, surfacing as borrow-checker
  errors at call sites. Fix by adding `+ use<>` to the generated
  signature *via a small post-processing pass in `build.rs`* if the
  pattern repeats; one-off, edit by hand in a follow-up.
- **`if let` rescope.** Generated code may have `if let Some(x) =
  long_expr { ... }` where `long_expr` holds a temporary that the
  old edition kept alive past the `if let`. Bumping the edition
  changes that. Surfaces as a runtime test failure or a
  use-after-drop borrow-check error, not silent miscompilation.

Run `cargo build -p ferro-protect --all-features` first after the
edition flip and treat any new error as belonging to this category.
Do not modify `$OUT_DIR/generated.rs` directly — fix the rewrite
layer in `build_support/spec_rewrite.rs` if needed, or post-process
the `TokenStream` in `build.rs` before `prettyplease::unparse`.

### 5. Adopt the 2024 features that pay for themselves *here*

Pick conservatively. The point of this chore is to unblock newer
ergonomics, not to rewrite working code. The shortlist below is
graded by payoff in *this* codebase. Anything not graded "do it":
leave alone.

| Feature | Where it pays here | Do it? |
|---|---|---|
| `if let` chains (require Rust 1.88) | Gated by MSRV — this chore pins `rust-version = "1.85"`, so let-chains are unavailable regardless. Re-evaluate when MSRV reaches 1.88; on closer reading `api_key::resolve` is also sequential early-return precedence rather than the nested `if let A { if let B { ... } }` shape that let-chains flatten, so the payoff may be smaller than first thought. | No (MSRV-gated; see this chore's PROGRESS entry) |
| `#[expect(...)]` attribute (stable 1.81) | Replace `#[allow(dead_code)]` on the unused `post_json`/`patch_json`/`send_no_content`/`get_bytes` helpers in `client.rs` with `#[expect(dead_code, reason = "wired up in phases 5-8")]`. If the allow becomes wrong (because phase 5 actually wires it up and the lint stops firing), `expect` flips to a warning and the stale attribute gets cleaned up automatically. | Yes — same line count, better signal |
| `prelude` adds `Future` / `IntoFuture` | The codebase has no explicit `use std::future::Future` lines (`grep` confirms). Nothing to remove. | No |
| `std::sync::LazyLock` over `once_cell::sync::Lazy` | The codebase uses neither. No payoff today; revisit if a phase introduces a global cache. | No |
| `let-else` (already 2021) | Already used in `api_key.rs:98` and the `live.rs` skip-guards. No edition-bound work. | N/A |
| `#[diagnostic::on_unimplemented]` / `#[diagnostic::do_not_recommend]` (stable 1.78 / 1.85) | Marginal value for a thin async-client crate with few generics. | No |
| `gen` blocks (unstable) | Not stable on the channel `rust-toolchain.toml` pins. | No |

Anything you find during the migration that fits the "do it" rows
above lands in *this* commit. Anything tempting but bigger
(refactoring `client.rs` helpers, restructuring `models.rs`) lands
in a follow-up chore — keep the diff to "edition flip + new lints
satisfied + small ergonomic wins."

### 6. Lints to enable after the flip

Edition 2024 changes the *default level* of a few lints (most
notably `unsafe_op_in_unsafe_fn`, which becomes warn-by-default
under 2024). The chore also adopts a couple of Clippy lints that
aren't edition-gated but pay off once `#[expect]` is in routine
use. Add to `[workspace.lints.rust]` in `Cargo.toml`:

```toml
unsafe_op_in_unsafe_fn = "deny"   # belt-and-braces alongside `unsafe_code = "forbid"`
unused_lifetimes = "warn"
```

And to `[workspace.lints.clippy]`:

```toml
allow_attributes = "warn"         # nudge `#[allow]` → `#[expect]` (Item 5)
allow_attributes_without_reason = "warn"
```

If `cargo clippy --all-targets --all-features -- -D warnings` fires
on any pre-existing code after enabling these, fix the offending
site in the same commit if it is small (one or two lines); defer to
a follow-up if it would balloon the diff and note the deferral in
the PROGRESS entry.

### 7. Verify the four gates

Standard chore close-out (AGENT.md → Guardrails):

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all
cargo deny --locked check
```

The live tests at `crates/ferro-protect/tests/live.rs` skip
cleanly when `UNIFI_PROTECT_HOST` is absent. The user is currently
on a Mac without a live NVR configured; the cleanly-skipping
behaviour is itself part of the gate.

Cross-check that `cargo build` and `cargo test` both still
regenerate `$OUT_DIR/generated.rs` cleanly (delete `target/debug/build/ferro-protect-*` once and rebuild to be sure the
build script reruns under 2024).

### 8. Update docs

- `AGENT.md` → no change. The four-gate list, the unsafe-code
  invariant, and the testing strategy are edition-agnostic.
- `ARCHITECTURE.md` → no change unless step 4 forced a
  `build.rs` post-processing pass; if so, document the pass in the
  file map.
- `README.md` → bump the "Requires Rust 1.83+" line (if present)
  to "Requires Rust 1.85+". Grep for the exact wording first; the
  README may phrase it differently.
- `UPGRADING.md` → no change. The doc is about spec bumps, not
  toolchain bumps.

### 9. PROGRESS.md entry

Append a single entry at the bottom in the standard format. Lead
the **Summary** with the edition flip and the MSRV bump, then list
anything non-mechanical from steps 4 / 5 / 6. Capture the timestamp
at write time per AGENT.md (`date +"%Y-%m-%d %H:%M %z"`).

## What to watch for

- **Generated-code surprises.** If `cargo build` fails inside
  `$OUT_DIR/generated.rs`, the diff is invisible from `git status`.
  Run `cargo expand` or read the file from the target directory to
  see what `typify` emitted under the new edition.
- **Drop-order regressions in tests.** The 2024 tail-expression
  scope change can shift when a `MutexGuard`, `tempfile::TempDir`,
  or wiremock `MockServer` is dropped. If a test starts failing
  *only* under release mode or *only* under parallel `cargo test`,
  treat it as a drop-order issue, not a flake. The `wiremock`-based
  CLI tests in `crates/ferro-protect-cli/tests/` are the most
  likely candidates.
- **`cargo deny`** may complain about a transitive bump if
  `resolver = "3"` pulls a newer minor version of something already
  on the allow-list. Read the failure carefully before relaxing
  `deny.toml`.

## Acceptance

The chore is done when:

- Workspace edition is `2024`, `rust-version` is `1.85`,
  `resolver` is `"3"`.
- `cargo fix --edition` has been run and the diff reviewed.
- All four gates are green on the branch.
- The `#[expect]` adoption from step 5 (parked HTTP helpers in
  `client.rs`) is applied. (Let-chains adoption is MSRV-gated and
  was reconsidered during implementation — see this chore's
  PROGRESS entry.)
- The lint additions in step 6 are present and the workspace
  builds clean under them.
- `PROGRESS.md` has the chore entry.
- One signed commit on a `chore/edition-2024` branch. No push.

If any step needs to be deferred (e.g. lint sweep too large for
one commit), record the deferral in the PROGRESS entry and add a
trigger condition to PLAN.md's "Deferred — revisit before 0.1.0"
section per AGENT.md.

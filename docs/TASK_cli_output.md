# Chore: nicer CLI output (tables and adjacent surface)

## Why

The `oas3` PR review surfaced a comment on
[`crates/ferro-protect-cli/src/commands/cameras.rs:64-76`](../crates/ferro-protect-cli/src/commands/cameras.rs#L64-L76)
asking whether a library would do a better job than the hand-rolled
table renderer in [`output.rs:48-97`](../crates/ferro-protect-cli/src/output.rs#L48-L97).
Copilot's reply listed four table libraries (`comfy-table`, `tabled`,
`prettytable-rs`, `cli-table`).

That suggestion answers the literal question but ducks the broader one:
*if we're going to take a CLI-output dependency, is "table renderer"
the right scope?* This chore exists to answer that deliberately, pick a
direction, and execute it in one bounded sitting rather than
accumulating ad-hoc dependencies endpoint by endpoint as phases 4-8
roll out.

## Timing

**Do this before phase 4c** (lights/liveviews/nvrs/sensors/viewers).
Every new entity adds another `render_table()` and `render_one()`
function shaped exactly like
[`cameras.rs`](../crates/ferro-protect-cli/src/commands/cameras.rs).
Each one is cheap on its own, expensive in aggregate, and rewriting six
of them later under a new convention is bigger churn than picking the
convention now and using it from the next wrapper forward.

If, after the evaluation in step 1, the answer is "stay with what we
have," that's still a useful outcome — document the decision and close
the chore.

## Scope decision (step 1 — research, not code)

Categorize the "nicer output" surface honestly before reaching for a
library. The categories the CLI actually has or will have:

| Category | Status today | Phase that needs it |
|---|---|---|
| List table (multi-record) | hand-rolled `output::table()` | already here; every list endpoint in phase 4+ |
| Single-record detail | hand-rolled `render_one()` per entity | already here; every get endpoint in phase 4+ |
| Semantic colour (state, errors) | none | nice-to-have, no hard requirement |
| Pretty error reporting | `anyhow`'s default `Display` | every error path |
| Progress / spinner | none | phase 7 (talkback), phase 8 (file uploads) |
| Pager for long output | none | probably never |
| Interactive prompts | none | not in scope |

Rust does not have a single equivalent of Python's [`rich`](https://github.com/Textualize/rich)
covering most of those at once. The closest is `ratatui`, which is a
full TUI framework and explicitly the wrong shape for a one-shot CLI.
So the realistic answer is *small composed crates per category*, not
one mega-toolkit.

The actively-felt pain today is **list tables**. Everything else is
either fine, deferred, or has no concrete user-visible problem yet.
Resist scope creep — adopt only what fixes a felt problem.

### Library evaluation

Read each crate's README and pick from this shortlist; do not invent
others.

- **`comfy-table`** — input shape (headers + `Vec<Vec<String>>`) matches
  the current `output::table()` signature exactly. Drop-in replacement
  with one wrapper function. TTY detection, sensible defaults, UTF-8
  and ASCII border presets, column truncation, alignment, optional
  colour. No derive macros — which is *good* for us because typify
  generates `models::Camera` etc. and we cannot annotate generated
  types without a wrapper.

- **`tabled`** — more feature-rich, `#[derive(Tabled)]` on input
  structs. Strictly worse fit here: typify owns `models::Camera`, so
  we'd need to define `CameraView` wrapper structs in the CLI that
  carry the `Tabled` derive and map field-by-field from
  `models::Camera`. That's boilerplate we don't need for a four-column
  list. Reconsider if any entity grows a list view with conditional
  columns, computed cells, or per-column custom formatters.

- **`prettytable-rs`** — macro-heavy, less actively maintained.
  Nothing it does is unique. Skip.

- **`cli-table`** — small, derive macros. Same wrapper-struct issue as
  `tabled`, with less ecosystem traction. Skip.

**Default recommendation: `comfy-table`.** If, while doing step 2, you
discover that the only thing we want is the ASCII output and we never
want borders/colour/TTY-aware presets, abandon the dependency and keep
`output::table()`. Don't take a dep we don't actually use.

### Out-of-scope adjacent items (deliberately deferred)

Each of these came up while scoping; documenting why they're *not*
part of this chore so a future reader does not relitigate them.

- **`tabled` with derive on typify types.** Out — requires CLI-side
  wrapper structs that duplicate every field. Revisit only if we need
  per-column conditional rendering.
- **Colour for `state` / health fields (e.g. `owo-colors` or `nu-ansi-term`).**
  Out for now — no current user complaint, and the JSON path
  (`--json`) must stay byte-identical regardless. Easy to add later as
  a one-line wrapper inside the human render path.
- **`color-eyre` / `miette` for prettier error chains.** Out — anyhow's
  default is adequate at this scale; the spec rewrite layer is the only
  place we'd benefit from source-span hints and that lives in a build
  script that already has its own formatting.
- **`indicatif` for progress bars.** Out until phase 7/8 introduces an
  operation slow enough to need one. Adding it now ships UI nobody
  exercises.
- **`dialoguer` for interactive prompts.** Out — the CLI is strictly
  non-interactive by design (suitable for scripts, cron, CI-style use).

## Action items (assuming the recommendation stands)

### 2. Add `comfy-table` as a workspace dependency

In the root [`Cargo.toml`](../Cargo.toml), add to `[workspace.dependencies]`:

```toml
comfy-table = { version = "<latest>", default-features = false }
```

Default-features off keeps the dependency footprint tight; opt back in
to `tty` / `custom_styling` only if step 3 needs them. Run `cargo deny
check` against the new transitive tree before committing.

Add to [`crates/ferro-protect-cli/Cargo.toml`](../crates/ferro-protect-cli/Cargo.toml)
under `[dependencies]`:

```toml
comfy-table = { workspace = true }
```

The library crate (`ferro-protect`) does **not** depend on
`comfy-table`. CLI rendering belongs in the CLI crate; the library
stays output-agnostic.

### 3. Replace `output::table()` with a comfy-table wrapper

[`crates/ferro-protect-cli/src/output.rs`](../crates/ferro-protect-cli/src/output.rs)
currently exposes `pub fn table(headers: &[&str], rows: &[Vec<String>])
-> String`. Keep the **signature** identical so callers
(`commands/cameras.rs::render_table`, `commands/chimes.rs`, and every
future entity) don't need to change.

Inside, rebuild on comfy-table:

```rust
use comfy_table::presets::UTF8_FULL;
use comfy_table::{ContentArrangement, Table};

pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(headers.iter().copied());
    for row in rows {
        table.add_row(row);
    }
    format!("{table}\n")
}
```

Pick the preset deliberately (`UTF8_FULL` vs `ASCII_MARKDOWN` vs
`NOTHING`) and document the choice in a comment. Default
recommendation: `UTF8_FULL` for human TTY output, `ASCII_MARKDOWN` if
we ever notice copy-paste-to-docs being a real workflow.

Remove the old hand-rolled `push_row` helper. Drop the unused
TTY-detection comment in the docstring if it referenced one.

### 4. Decide on `render_one()`

The single-record renderer in each
[`commands/<entity>.rs`](../crates/ferro-protect-cli/src/commands/)
prints a fixed `Key: value` block. Two reasonable directions:

- **Keep it manual.** Four lines per entity, trivial to read. Pro:
  zero extra dependency surface; con: a sixth entity copying the
  template is a smell.
- **Centralise as a two-column key-value table.** Use the same
  comfy-table machinery with a borderless preset (e.g.
  `ASCII_NO_BORDERS` or `NOTHING`) and right-aligned key column.
  Pro: one place to tune detail formatting; con: borderless tables
  look slightly different from "real" key-value text.

Default recommendation: **keep manual** for now, but extract the
per-field formatting (the `Option<Name>` unwrap with `as_ref().map(...)
.unwrap_or_default()`) into a small `display_optional<T: Display>`
helper in `output.rs` so the six future entities don't each reinvent
it. Don't centralise the whole block; the cost-of-abstraction does not
pay off at this scale.

### 5. Verify nothing else regressed

The existing assert_cmd tests in
[`crates/ferro-protect-cli/tests/`](../crates/ferro-protect-cli/tests/)
assert specific substrings against `--json` and human stdout. The JSON
path must be byte-identical (we did not touch it); the human path will
differ — update the substrings the tests look for, do not loosen them.

Specifically, [`tests/cameras.rs`](../crates/ferro-protect-cli/tests/cameras.rs)
and [`tests/chimes.rs`](../crates/ferro-protect-cli/tests/chimes.rs)
assert the empty-list output. With comfy-table this becomes a table
with a header row but no body rows; pick whether that's the desired
"empty" output or whether `render_table` should special-case `if
cameras.is_empty()` (current code already does — preserve that
behaviour). Same for chimes.

### 6. Update docs

- [`ARCHITECTURE.md`](../ARCHITECTURE.md) — the file map describes
  [`crates/ferro-protect-cli/src/output.rs`](../crates/ferro-protect-cli/src/output.rs)
  as `emit()` + `table()` (fixed-column renderer). Update the "fixed-
  column renderer" phrasing if it changes.
- [`PLAN.md`](../PLAN.md) — Phase 4's wrapper template references the
  per-entity `render_table`/`render_one` shape. Update the shape
  description to match whatever lands here so future-phase agents
  copy the right template.
- No `README.md` change — end-user CLI usage is unchanged.

### 7. Verify, log, commit

Run `cargo fmt --all -- --check`, `cargo clippy --all-targets
--all-features -- -D warnings`, `cargo test --all`, `cargo deny
check`. All four must be green. Append a `PROGRESS.md` entry per the
canonical logging format. Commit with the message:

```
chore(cli): replace hand-rolled table renderer with comfy-table
```

(Or, if step 1's evaluation came back "stay with what we have,"
commit only the PROGRESS entry recording that decision with rationale,
and close the chore.)

## Acceptance criteria

- Either `comfy-table` is a CLI-crate dependency with `output::table()`
  rebuilt on it, **or** `PROGRESS.md` carries a documented "won't do"
  decision with the rationale.
- If adopted: `output::table()` keeps its `(&[&str], &[Vec<String>]) ->
  String` signature so every existing and future entity wrapper stays a
  one-line call.
- If adopted: `render_one()` either stays manual with a shared
  `display_optional` helper, or is centralised — decision documented in
  `PROGRESS.md` either way.
- `--json` output is byte-identical before and after.
- No new dependency in the `ferro-protect` library crate. CLI
  rendering belongs in the CLI crate only.
- All four gates green.

## Out of scope

- **No colour, no error-reporter swap, no progress bars, no pagers.**
  Each is listed in "Out-of-scope adjacent items" above with the
  reason. Adopting more than `comfy-table` in this chore re-opens the
  scope decision and the chore stops being one bounded sitting.
- **No changes to the library crate or to typify-generated code.**
- **No `tabled` with derive macros.** The wrapper-struct boilerplate
  cost is not justified at four columns.
- **No new subcommands or endpoint coverage.** Phase 4c+ adds those.

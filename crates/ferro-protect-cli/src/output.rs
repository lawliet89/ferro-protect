//! Output helpers shared across subcommands.
//!
//! Each subcommand handler builds its result, then calls [`emit`] which
//! prints either pretty-JSON (if `--json`) or the human-formatted string
//! produced by the closure.

use std::fmt::Display;
use std::io::{self, Write};

use comfy_table::presets::UTF8_FULL;
use comfy_table::{ContentArrangement, Table};

/// Write either JSON or human output, depending on `json`.
///
/// JSON path: serde_json pretty print + trailing newline. Human path:
/// writes exactly what `render_human` returns -- closures should include
/// their own trailing newline, matching the convention used by
/// `table()` / per-entity renderers.
///
/// # Errors
/// Underlying I/O failures from `out`, or JSON serialisation failures from
/// `value`.
pub fn emit<T, F, W>(out: &mut W, value: &T, json: bool, render_human: F) -> anyhow::Result<()>
where
    T: serde::Serialize,
    F: FnOnce() -> String,
    W: Write,
{
    if json {
        serde_json::to_writer_pretty(&mut *out, value)?;
        out.write_all(b"\n")?;
    } else {
        out.write_all(render_human().as_bytes())?;
    }
    Ok(())
}

/// Convenience for the common `out = io::stdout().lock()` case.
///
/// # Errors
/// As [`emit`].
pub fn emit_stdout<T, F>(value: &T, json: bool, render_human: F) -> anyhow::Result<()>
where
    T: serde::Serialize,
    F: FnOnce() -> String,
{
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    emit(&mut lock, value, json, render_human)
}

/// Render a list of rows as a human-readable table.
///
/// Callers are expected to short-circuit empty lists with their own
/// "(no foo)" string; this helper assumes there is at least one row
/// worth tabulating.
#[must_use]
pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut table = Table::new();
    // `UTF8_FULL` over the other comfy-table presets: terminal users get
    // visible row/column separation without depending on terminal-specific
    // styling. `ASCII_MARKDOWN` would be nicer for copy-paste into docs but
    // worse at the keyboard; `NOTHING` collapses to spaces which is too
    // close to our old hand-rolled output to be worth the dep. Revisit if
    // a real workflow argues for switching.
    //
    // `ContentArrangement::Dynamic` fits the table to the detected terminal
    // width, wrapping long cells. Behaviour change from the old renderer,
    // which never truncated -- if a user reports surprise wrapping on
    // narrow terminals, swap to `ContentArrangement::Disabled`.
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(headers.iter().copied());

    for row in rows {
        table.add_row(row.iter().map(String::as_str));
    }

    format!("{table}\n")
}

/// Render an optional `Display`-able value as a string, using the empty
/// string when `None`. Used by entity renderers for optional fields like
/// `Camera::name` so each call site stays one expression long.
#[must_use]
pub fn display_optional<T: Display>(value: Option<&T>) -> String {
    value.map(ToString::to_string).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{display_optional, table};

    #[test]
    fn table_renders_headers_and_cells() {
        let out = table(
            &["ID", "NAME"],
            &[
                vec!["cam-1".into(), "Front Door".into()],
                vec!["cam-2".into(), "Backyard".into()],
            ],
        );

        assert!(out.contains("ID"), "header missing: {out}");
        assert!(out.contains("NAME"), "header missing: {out}");
        assert!(out.contains("cam-1"), "cell missing: {out}");
        assert!(out.contains("Front Door"), "cell missing: {out}");
        assert!(out.contains("Backyard"), "cell missing: {out}");
        // UTF8_FULL preset uses box-drawing characters; assert one is
        // present so a future preset swap is a deliberate test failure.
        assert!(
            out.contains('─'),
            "expected UTF8_FULL border char in output: {out}"
        );
        assert!(out.ends_with('\n'), "missing trailing newline: {out:?}");
    }

    #[test]
    fn display_optional_some_renders_value() {
        let name = String::from("Front Door");
        assert_eq!(display_optional(Some(&name)), "Front Door");
    }

    #[test]
    fn display_optional_none_renders_empty() {
        assert_eq!(display_optional::<String>(None), "");
    }
}

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

/// Render a list of rows as a human table.
#[must_use]
pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut table = Table::new();
    // Use UTF-8 borders for human-readable terminal output.
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(headers.iter().copied());

    for row in rows {
        table.add_row(row.iter().map(String::as_str));
    }

    format!("{table}\n")
}

#[must_use]
pub fn display_optional<T: Display>(value: Option<&T>) -> String {
    value.map(ToString::to_string).unwrap_or_default()
}

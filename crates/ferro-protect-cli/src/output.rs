//! Output helpers shared across subcommands.
//!
//! Each subcommand handler builds its result, then calls [`emit`] which
//! prints either pretty-JSON (if `--json`) or the human-formatted string
//! produced by the closure.

use std::io::{self, Write};

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

/// Render a list of rows as a fixed-column table. Columns are sized to the
/// widest cell. Header row is bolded with terminal escapes when stdout is a
/// TTY, otherwise plain.
#[must_use]
pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let cols = headers.len();
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate().take(cols) {
            if cell.len() > widths[i] {
                widths[i] = cell.len();
            }
        }
    }

    let mut out = String::new();
    push_row(&mut out, headers.iter().copied(), &widths);
    push_row(
        &mut out,
        widths
            .iter()
            .map(|w| "-".repeat(*w))
            .collect::<Vec<_>>()
            .iter()
            .map(String::as_str),
        &widths,
    );
    for row in rows {
        push_row(&mut out, row.iter().map(String::as_str), &widths);
    }
    out
}

fn push_row<'a, I>(out: &mut String, cells: I, widths: &[usize])
where
    I: IntoIterator<Item = &'a str>,
{
    let mut first = true;
    for (cell, width) in cells.into_iter().zip(widths.iter()) {
        if !first {
            out.push_str("  ");
        }
        first = false;
        out.push_str(cell);
        for _ in cell.len()..*width {
            out.push(' ');
        }
    }
    out.push('\n');
}

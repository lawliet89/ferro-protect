//! `ferro-protect nvrs …` subcommand. `GET /v1/nvrs` returns a single
//! `Nvr` object; the subcommand exposes `get` (no list) to match.

use anyhow::{Context, Result};
use clap::Subcommand;
use ferro_protect::ProtectClient;
use ferro_protect::models::Nvr;

use crate::output;

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Fetch the NVR for this installation.
    Get,
}

/// Dispatch `nvrs` subcommands.
///
/// # Errors
/// Bubbles up the underlying [`ferro_protect::Error`] (network, API, etc.)
/// and any I/O error from formatting/printing.
pub async fn run(client: &ProtectClient, action: Action, json: bool) -> Result<()> {
    match action {
        Action::Get => {
            let nvr = client.nvrs().get().await.context("fetching nvr")?;
            output::emit_stdout(&nvr, json, || render_one(&nvr))?;
        }
    }
    Ok(())
}

fn render_one(nvr: &Nvr) -> String {
    format!(
        "ID:   {}\nName: {}\n",
        nvr.id,
        output::display_optional(nvr.name.as_ref()),
    )
}

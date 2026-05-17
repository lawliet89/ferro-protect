//! `ferro-protect chimes …` subcommands.

use anyhow::{Context, Result};
use clap::Subcommand;
use ferro_protect::models::{Chime, ChimeId};
use ferro_protect::ProtectClient;

use crate::output;

#[derive(Debug, Subcommand)]
pub enum Action {
    /// List every chime the NVR knows about.
    List,
    /// Look up one chime by ID.
    Get {
        /// Chime ID.
        id: String,
    },
}

/// Dispatch `chimes` subcommands.
///
/// # Errors
/// Bubbles up the underlying [`ferro_protect::Error`] and any I/O
/// failure from formatting/printing.
pub async fn run(client: &ProtectClient, action: Action, json: bool) -> Result<()> {
    match action {
        Action::List => {
            let chimes = client.chimes().list().await.context("listing chimes")?;
            output::emit_stdout(&chimes, json, || render_table(&chimes))?;
        }
        Action::Get { id } => {
            let id = ChimeId::from(id);
            let chime = client
                .chimes()
                .get(&id)
                .await
                .with_context(|| format!("fetching chime {id}"))?;
            output::emit_stdout(&chime, json, || render_one(&chime))?;
        }
    }
    Ok(())
}

fn render_table(chimes: &[Chime]) -> String {
    if chimes.is_empty() {
        return "(no chimes)\n".to_string();
    }
    let headers = &["ID", "NAME", "MAC", "STATE"];
    let rows: Vec<Vec<String>> = chimes
        .iter()
        .map(|c| {
            vec![
                c.id.to_string(),
                c.name.as_ref().map(ToString::to_string).unwrap_or_default(),
                c.mac.to_string(),
                c.state.to_string(),
            ]
        })
        .collect();
    output::table(headers, &rows)
}

fn render_one(chime: &Chime) -> String {
    format!(
        "ID:    {}\nName:  {}\nMAC:   {}\nState: {}\nPaired cameras: {}\n",
        chime.id,
        chime
            .name
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
        chime.mac,
        chime.state,
        chime.camera_ids.len(),
    )
}

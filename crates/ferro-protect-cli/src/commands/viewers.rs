//! `ferro-protect viewers …` subcommands.

use anyhow::{Context, Result};
use clap::Subcommand;
use ferro_protect::ProtectClient;
use ferro_protect::models::{Viewer, ViewerId};

use crate::output;

#[derive(Debug, Subcommand)]
pub enum Action {
    /// List every viewer the NVR knows about.
    List,
    /// Look up one viewer by ID.
    Get {
        /// Viewer ID.
        id: String,
    },
}

/// Dispatch `viewers` subcommands.
///
/// # Errors
/// Bubbles up the underlying [`ferro_protect::Error`] (network, API, etc.)
/// and any I/O error from formatting/printing.
pub async fn run(client: &ProtectClient, action: Action, json: bool) -> Result<()> {
    match action {
        Action::List => {
            let viewers = client.viewers().list().await.context("listing viewers")?;
            output::emit_stdout(&viewers, json, || render_table(&viewers))?;
        }
        Action::Get { id } => {
            let id = ViewerId::from(id);
            let viewer = client
                .viewers()
                .get(&id)
                .await
                .with_context(|| format!("fetching viewer {id}"))?;
            output::emit_stdout(&viewer, json, || render_one(&viewer))?;
        }
    }
    Ok(())
}

fn render_table(viewers: &[Viewer]) -> String {
    if viewers.is_empty() {
        return "(no viewers)\n".to_string();
    }
    let headers = &["ID", "NAME", "MAC", "STATE", "LIVEVIEW"];
    let rows: Vec<Vec<String>> = viewers
        .iter()
        .map(|v| {
            vec![
                v.id.to_string(),
                output::display_optional(v.name.as_ref()),
                v.mac.to_string(),
                v.state.to_string(),
                output::display_optional(v.liveview.as_ref()),
            ]
        })
        .collect();
    output::table(headers, &rows)
}

fn render_one(viewer: &Viewer) -> String {
    format!(
        "ID:       {}\nName:     {}\nMAC:      {}\nState:    {}\nLiveview: {}\n",
        viewer.id,
        output::display_optional(viewer.name.as_ref()),
        viewer.mac,
        viewer.state,
        output::display_optional(viewer.liveview.as_ref()),
    )
}

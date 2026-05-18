//! `ferro-protect liveviews …` subcommands.

use anyhow::{Context, Result};
use clap::Subcommand;
use ferro_protect::ProtectClient;
use ferro_protect::models::{Liveview, LiveviewId};

use crate::output;

#[derive(Debug, Subcommand)]
pub enum Action {
    /// List every liveview the NVR has configured.
    List,
    /// Look up one liveview by ID.
    Get {
        /// Liveview ID.
        id: String,
    },
}

/// Dispatch `liveviews` subcommands.
///
/// # Errors
/// Bubbles up the underlying [`ferro_protect::Error`] (network, API, etc.)
/// and any I/O error from formatting/printing.
pub async fn run(client: &ProtectClient, action: Action, json: bool) -> Result<()> {
    match action {
        Action::List => {
            let liveviews = client
                .liveviews()
                .list()
                .await
                .context("listing liveviews")?;
            output::emit_stdout(&liveviews, json, || render_table(&liveviews))?;
        }
        Action::Get { id } => {
            let id = LiveviewId::from(id);
            let liveview = client
                .liveviews()
                .get(&id)
                .await
                .with_context(|| format!("fetching liveview {id}"))?;
            output::emit_stdout(&liveview, json, || render_one(&liveview))?;
        }
    }
    Ok(())
}

fn render_table(liveviews: &[Liveview]) -> String {
    if liveviews.is_empty() {
        return "(no liveviews)\n".to_string();
    }
    let headers = &["ID", "NAME", "SLOTS", "GLOBAL", "DEFAULT"];
    let rows: Vec<Vec<String>> = liveviews
        .iter()
        .map(|lv| {
            vec![
                lv.id.to_string(),
                lv.name.clone(),
                lv.slots.len().to_string(),
                lv.is_global.to_string(),
                lv.is_default.to_string(),
            ]
        })
        .collect();
    output::table(headers, &rows)
}

fn render_one(liveview: &Liveview) -> String {
    format!(
        "ID:      {}\nName:    {}\nSlots:   {}\nGlobal:  {}\nDefault: {}\n",
        liveview.id,
        liveview.name,
        liveview.slots.len(),
        liveview.is_global,
        liveview.is_default,
    )
}

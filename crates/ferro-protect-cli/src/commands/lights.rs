//! `ferro-protect lights …` subcommands.

use anyhow::{Context, Result};
use clap::Subcommand;
use ferro_protect::models::{Light, LightId};
use ferro_protect::ProtectClient;

use crate::output;

#[derive(Debug, Subcommand)]
pub enum Action {
    /// List every light the NVR knows about.
    List,
    /// Look up one light by ID.
    Get {
        /// Light ID.
        id: String,
    },
}

/// Dispatch `lights` subcommands.
///
/// # Errors
/// Bubbles up the underlying [`ferro_protect::Error`] (network, API, etc.)
/// and any I/O error from formatting/printing.
pub async fn run(client: &ProtectClient, action: Action, json: bool) -> Result<()> {
    match action {
        Action::List => {
            let lights = client.lights().list().await.context("listing lights")?;
            output::emit_stdout(&lights, json, || render_table(&lights))?;
        }
        Action::Get { id } => {
            let id = LightId::from(id);
            let light = client
                .lights()
                .get(&id)
                .await
                .with_context(|| format!("fetching light {id}"))?;
            output::emit_stdout(&light, json, || render_one(&light))?;
        }
    }
    Ok(())
}

fn render_table(lights: &[Light]) -> String {
    if lights.is_empty() {
        return "(no lights)\n".to_string();
    }
    let headers = &["ID", "NAME", "MAC", "STATE", "ON"];
    let rows: Vec<Vec<String>> = lights
        .iter()
        .map(|l| {
            vec![
                l.id.to_string(),
                output::display_optional(l.name.as_ref()),
                l.mac.to_string(),
                l.state.to_string(),
                l.is_light_on.to_string(),
            ]
        })
        .collect();
    output::table(headers, &rows)
}

fn render_one(light: &Light) -> String {
    format!(
        "ID:    {}\nName:  {}\nMAC:   {}\nState: {}\nOn:    {}\n",
        light.id,
        output::display_optional(light.name.as_ref()),
        light.mac,
        light.state,
        light.is_light_on,
    )
}

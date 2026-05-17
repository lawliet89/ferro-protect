//! `ferro-protect sensors …` subcommands.

use anyhow::{Context, Result};
use clap::Subcommand;
use ferro_protect::models::{Sensor, SensorId};
use ferro_protect::ProtectClient;

use crate::output;

#[derive(Debug, Subcommand)]
pub enum Action {
    /// List every sensor the NVR knows about.
    List,
    /// Look up one sensor by ID.
    Get {
        /// Sensor ID.
        id: String,
    },
}

/// Dispatch `sensors` subcommands.
///
/// # Errors
/// Bubbles up the underlying [`ferro_protect::Error`] (network, API, etc.)
/// and any I/O error from formatting/printing.
pub async fn run(client: &ProtectClient, action: Action, json: bool) -> Result<()> {
    match action {
        Action::List => {
            let sensors = client.sensors().list().await.context("listing sensors")?;
            output::emit_stdout(&sensors, json, || render_table(&sensors))?;
        }
        Action::Get { id } => {
            let id = SensorId::from(id);
            let sensor = client
                .sensors()
                .get(&id)
                .await
                .with_context(|| format!("fetching sensor {id}"))?;
            output::emit_stdout(&sensor, json, || render_one(&sensor))?;
        }
    }
    Ok(())
}

fn render_table(sensors: &[Sensor]) -> String {
    if sensors.is_empty() {
        return "(no sensors)\n".to_string();
    }
    let headers = &["ID", "NAME", "MAC", "STATE"];
    let rows: Vec<Vec<String>> = sensors
        .iter()
        .map(|s| {
            vec![
                s.id.to_string(),
                output::display_optional(s.name.as_ref()),
                s.mac.to_string(),
                s.state.to_string(),
            ]
        })
        .collect();
    output::table(headers, &rows)
}

fn render_one(sensor: &Sensor) -> String {
    format!(
        "ID:    {}\nName:  {}\nMAC:   {}\nState: {}\n",
        sensor.id,
        output::display_optional(sensor.name.as_ref()),
        sensor.mac,
        sensor.state,
    )
}

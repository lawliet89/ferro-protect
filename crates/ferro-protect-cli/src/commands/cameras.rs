//! `ferro-protect cameras …` subcommands.

use anyhow::{Context, Result};
use clap::Subcommand;
use ferro_protect::ProtectClient;
use ferro_protect::models::{Camera, CameraId};

use crate::output;

#[derive(Debug, Subcommand)]
pub enum Action {
    /// List every camera the NVR knows about.
    List,
    /// Look up one camera by ID.
    Get {
        /// Camera ID.
        id: String,
    },
}

/// Dispatch `cameras` subcommands.
///
/// # Errors
/// Bubbles up the underlying [`ferro_protect::Error`] (network, API, etc.)
/// and any I/O error from formatting/printing.
pub async fn run(client: &ProtectClient, action: Action, json: bool) -> Result<()> {
    match action {
        Action::List => {
            let cameras = client.cameras().list().await.context("listing cameras")?;
            output::emit_stdout(&cameras, json, || render_table(&cameras))?;
        }
        Action::Get { id } => {
            let id = CameraId::from(id);
            let camera = client
                .cameras()
                .get(&id)
                .await
                .with_context(|| format!("fetching camera {id}"))?;
            output::emit_stdout(&camera, json, || render_one(&camera))?;
        }
    }
    Ok(())
}

fn render_table(cameras: &[Camera]) -> String {
    if cameras.is_empty() {
        return "(no cameras)\n".to_string();
    }
    let headers = &["ID", "NAME", "MAC", "STATE"];
    let rows: Vec<Vec<String>> = cameras
        .iter()
        .map(|c| {
            vec![
                c.id.to_string(),
                output::display_optional(c.name.as_ref()),
                c.mac.to_string(),
                c.state.to_string(),
            ]
        })
        .collect();
    output::table(headers, &rows)
}

fn render_one(camera: &Camera) -> String {
    format!(
        "ID:    {}\nName:  {}\nMAC:   {}\nState: {}\n",
        camera.id,
        output::display_optional(camera.name.as_ref()),
        camera.mac,
        camera.state,
    )
}

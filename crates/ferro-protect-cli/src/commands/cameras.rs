//! `ferro-protect cameras …` subcommands.

use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Subcommand, ValueEnum};
use ferro_protect::ProtectClient;
use ferro_protect::models::{
    Camera, CameraId, ChannelQuality, RtspsStream, SnapshotChannel, SnapshotOptions,
    TalkbackSession,
};
use is_terminal::IsTerminal;

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
    /// Allocate a talkback WebSocket session for a camera. The
    /// server returns the WS URL to push audio into plus the audio
    /// codec/sample-rate config the camera expects.
    Talkback {
        /// Camera ID.
        id: String,
    },
    /// Create RTSPS stream URLs for a camera. Returns one URL per
    /// requested quality level. The server allocates new stream
    /// credentials on each call.
    Rtsps {
        /// Camera ID.
        id: String,
        /// Comma-separated qualities (any of `high`, `medium`, `low`,
        /// `package`). Order is preserved in the output. `package`
        /// only works for cameras with a package camera.
        #[arg(long, value_delimiter = ',', default_value = "high")]
        quality: Vec<QualityArg>,
    },
    /// Fetch a JPEG snapshot from a camera. Writes the bytes to
    /// `--out PATH` if given; otherwise to stdout if stdout is not
    /// a TTY. Refuses to dump binary into an interactive terminal.
    Snapshot {
        /// Camera ID.
        id: String,
        /// Camera channel. Use `package` for cameras with a package
        /// camera (`hasPackageCamera: true`).
        #[arg(long, value_enum)]
        channel: Option<ChannelArg>,
        /// Force 1080P or higher resolution.
        #[arg(long)]
        high_quality: bool,
        /// File to write the JPEG to. If omitted, the bytes are
        /// written to stdout (only when stdout is not a TTY).
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

/// CLI-facing channel enum. Maps 1:1 onto [`SnapshotChannel`] but
/// kept separate so we can keep `clap::ValueEnum` derivation off the
/// library type.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ChannelArg {
    Main,
    Package,
}

impl From<ChannelArg> for SnapshotChannel {
    fn from(value: ChannelArg) -> Self {
        match value {
            ChannelArg::Main => Self::Main,
            ChannelArg::Package => Self::Package,
        }
    }
}

/// CLI-facing quality enum. Maps 1:1 onto [`ChannelQuality`] but
/// kept separate so we can keep `clap::ValueEnum` derivation off the
/// library type.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum QualityArg {
    High,
    Medium,
    Low,
    Package,
}

impl From<QualityArg> for ChannelQuality {
    fn from(value: QualityArg) -> Self {
        match value {
            QualityArg::High => Self::High,
            QualityArg::Medium => Self::Medium,
            QualityArg::Low => Self::Low,
            QualityArg::Package => Self::Package,
        }
    }
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
        Action::Talkback { id } => {
            let id = CameraId::from(id);
            let session = client
                .cameras()
                .talkback_session(&id)
                .await
                .with_context(|| format!("creating talkback session for camera {id}"))?;
            output::emit_stdout(&session, json, || render_talkback_session(&session))?;
        }
        Action::Rtsps { id, quality } => {
            let id = CameraId::from(id);
            let qualities: Vec<ChannelQuality> =
                quality.into_iter().map(ChannelQuality::from).collect();
            let streams = client
                .cameras()
                .rtsps_stream(&id, &qualities)
                .await
                .with_context(|| format!("creating RTSPS streams for camera {id}"))?;
            output::emit_stdout(&streams, json, || render_rtsps_streams(&streams))?;
        }
        Action::Snapshot {
            id,
            channel,
            high_quality,
            out,
        } => {
            // The global `--json` flag is a string-output mode; a JPEG has
            // no meaningful JSON representation (base64-wrapping it would
            // surprise scripted callers more than it'd help). Reject up
            // front so the contract stays clear: `--json` always implies
            // machine-readable text on stdout.
            if json {
                bail!(
                    "`cameras snapshot` produces binary JPEG output, which has no JSON \
                     representation. Drop --json, or pipe the raw bytes (or use --out PATH)."
                );
            }
            let id = CameraId::from(id);
            let opts = SnapshotOptions {
                channel: channel.map(SnapshotChannel::from),
                high_quality,
            };
            let bytes = client
                .cameras()
                .snapshot_with(&id, &opts)
                .await
                .with_context(|| format!("fetching snapshot for camera {id}"))?;
            write_snapshot(&bytes, out.as_deref())?;
        }
    }
    Ok(())
}

/// Write snapshot bytes to `out` (if given) or stdout. Refuses to
/// write to a TTY so a stray `cameras snapshot ID` at the keyboard
/// doesn't spray JPEG bytes into the terminal.
fn write_snapshot(bytes: &[u8], out: Option<&std::path::Path>) -> Result<()> {
    if let Some(path) = out {
        let mut f = File::create(path)
            .with_context(|| format!("creating snapshot output file {}", path.display()))?;
        f.write_all(bytes)
            .with_context(|| format!("writing snapshot to {}", path.display()))?;
        return Ok(());
    }
    let stdout = io::stdout();
    if stdout.is_terminal() {
        bail!(
            "refusing to write JPEG bytes to an interactive terminal. \
             Pass --out PATH, or redirect stdout (e.g. `... > snap.jpg`)."
        );
    }
    let mut lock = stdout.lock();
    lock.write_all(bytes)
        .context("writing snapshot to stdout")?;
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

fn render_talkback_session(s: &TalkbackSession) -> String {
    format!(
        "URL:           {}\nCodec:         {}\nSample rate:   {} Hz\nBits/sample:   {}\n",
        s.url, s.codec, s.sampling_rate, s.bits_per_sample,
    )
}

fn render_rtsps_streams(streams: &[RtspsStream]) -> String {
    if streams.is_empty() {
        return "(no streams returned)\n".to_string();
    }
    let headers = &["QUALITY", "URL"];
    let rows: Vec<Vec<String>> = streams
        .iter()
        .map(|s| vec![s.quality.to_string(), s.url.clone()])
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

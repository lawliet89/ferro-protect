//! `ferro-protect subscribe …` subcommands. Stream WebSocket messages
//! to stdout as NDJSON (one JSON object per line) until interrupted.

use std::io::{self, Write};

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use ferro_protect::{Error, ProtectClient, Subscription};
use futures_util::StreamExt;
use serde::Serialize;
use serde::de::DeserializeOwned;

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Stream device adds, updates, and removes as NDJSON.
    ///
    /// Output is always one JSON object per line, whatever `--json`
    /// says. Runs until Ctrl-C (exit 0) or until the NVR ends the
    /// subscription (non-zero exit).
    Devices,
    /// Stream Protect events (motion, rings, smart detections, ...) as
    /// NDJSON.
    ///
    /// Output and exit behaviour are the same as `subscribe devices`.
    Events,
}

/// Dispatch `subscribe` subcommands.
///
/// # Errors
/// The handshake failing, the NVR rejecting or closing the
/// subscription, or a stdout write failure other than a closed pipe.
pub async fn run(client: &ProtectClient, action: Action) -> Result<()> {
    match action {
        Action::Devices => {
            let sub = client
                .subscribe()
                .devices()
                .await
                .context("subscribing to devices")?;
            stream(sub).await
        }
        Action::Events => {
            let sub = client
                .subscribe()
                .events()
                .await
                .context("subscribing to events")?;
            stream(sub).await
        }
    }
}

async fn stream<T: Serialize + DeserializeOwned>(mut sub: Subscription<T>) -> Result<()> {
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    loop {
        let item = tokio::select! {
            _ = &mut ctrl_c => {
                log::info!("interrupted; closing subscription");
                return sub.close().await.context("closing subscription");
            }
            item = sub.next() => item,
        };
        match item {
            Some(Ok(message)) => {
                if !write_line(&message)? {
                    // Downstream closed the pipe (e.g. `| head -n 1`).
                    return sub.close().await.context("closing subscription");
                }
            }
            // The library has already logged a warning with the
            // decode error; one odd message should not end the stream.
            Some(Err(Error::Json(_))) => {}
            Some(Err(e)) => return Err(e).context("subscription failed"),
            None => bail!("the NVR closed the subscription"),
        }
    }
}

/// Write one NDJSON line. Returns `Ok(false)` when stdout is a closed
/// pipe, so the caller can stop quietly.
fn write_line<T: Serialize>(message: &T) -> Result<bool> {
    let mut line = serde_json::to_string(message)?;
    line.push('\n');
    let mut stdout = io::stdout().lock();
    match stdout
        .write_all(line.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(false),
        Err(e) => Err(e.into()),
    }
}

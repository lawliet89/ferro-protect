//! `ferro-protect subscribe …` subcommands. Stream WebSocket messages
//! to stdout as NDJSON (one JSON object per line) until interrupted.

use std::io::{self, Write};
use std::num::NonZeroU32;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use ferro_protect::{
    Error, ProtectClient, ReconnectConfig, ReconnectingSubscription, Subscription,
};
use futures_util::{Stream, StreamExt};
use serde::Serialize;
use serde::de::DeserializeOwned;

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Stream device adds, updates, and removes as NDJSON.
    ///
    /// Output is always one JSON object per line, whatever `--json`
    /// says. Runs until Ctrl-C (exit 0) or until the subscription ends
    /// for good (non-zero exit): immediately when the NVR drops it, or,
    /// with `--reconnect`, when reconnecting gives up.
    Devices(StreamArgs),
    /// Stream Protect events (motion, rings, smart detections, ...) as
    /// NDJSON.
    ///
    /// Output and exit behaviour are the same as `subscribe devices`.
    Events(StreamArgs),
}

impl Action {
    /// The options shared by every subscription.
    pub const fn args(&self) -> &StreamArgs {
        match self {
            Self::Devices(args) | Self::Events(args) => args,
        }
    }
}

#[derive(Debug, Args)]
pub struct StreamArgs {
    /// Ping the NVR every 30 seconds, and treat 60 seconds without any
    /// reply as a dropped connection. On by default; `--keepalive=false`
    /// disables it. Accepts the same `1/0/true/false/yes/no/on/off`
    /// vocabulary as `--insecure`.
    #[arg(
        long,
        value_parser = clap::builder::BoolishValueParser::new(),
        num_args = 0..=1,
        default_value = "true",
        default_missing_value = "true",
        require_equals = true,
    )]
    pub keepalive: bool,

    /// Reconnect when the subscription drops, waiting 8 seconds before
    /// the first attempt and doubling up to 120 seconds. Each drop is
    /// logged as a warning; messages sent while disconnected are lost.
    /// A 4xx answer such as 401 stops reconnecting.
    #[arg(long)]
    pub reconnect: bool,

    /// With `--reconnect`, give up after this many consecutive failed
    /// attempts. Default: keep trying.
    #[arg(long, requires = "reconnect", value_parser = clap::value_parser!(u32).range(1..))]
    pub max_attempts: Option<u32>,
}

impl StreamArgs {
    fn reconnect_config(&self) -> ReconnectConfig {
        ReconnectConfig {
            max_attempts: self.max_attempts.and_then(NonZeroU32::new),
            ..ReconnectConfig::default()
        }
    }
}

/// Dispatch `subscribe` subcommands. Keepalive is configured on the
/// client by the caller, from [`Action::args`].
///
/// # Errors
/// The handshake failing, the subscription ending for good, or a
/// stdout write failure other than a closed pipe.
pub async fn run(client: &ProtectClient, action: Action) -> Result<()> {
    let subscribe = client.subscribe();
    match action {
        Action::Devices(args) if args.reconnect => {
            let sub = subscribe
                .devices_reconnecting(args.reconnect_config())
                .await
                .context("subscribing to devices")?;
            stream(sub, true).await
        }
        Action::Devices(_) => {
            let sub = subscribe
                .devices()
                .await
                .context("subscribing to devices")?;
            stream(sub, false).await
        }
        Action::Events(args) if args.reconnect => {
            let sub = subscribe
                .events_reconnecting(args.reconnect_config())
                .await
                .context("subscribing to events")?;
            stream(sub, true).await
        }
        Action::Events(_) => {
            let sub = subscribe.events().await.context("subscribing to events")?;
            stream(sub, false).await
        }
    }
}

/// The two subscription types, as far as [`stream`] cares.
trait Closable {
    async fn close(self) -> ferro_protect::Result<()>;
}

impl<T> Closable for Subscription<T> {
    async fn close(self) -> ferro_protect::Result<()> {
        Self::close(self).await
    }
}

impl<T> Closable for ReconnectingSubscription<T> {
    async fn close(self) -> ferro_protect::Result<()> {
        Self::close(self).await
    }
}

/// Print messages until Ctrl-C, a closed stdout, or the end of the
/// stream. When `reconnecting`, error items report drops the stream
/// recovers from, so they are kept only to explain a final end.
async fn stream<S, T>(mut sub: S, reconnecting: bool) -> Result<()>
where
    S: Stream<Item = ferro_protect::Result<T>> + Closable + Unpin,
    T: Serialize + DeserializeOwned,
{
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    let mut last_error = None;
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
            Some(Err(e)) if reconnecting => last_error = Some(e),
            Some(Err(e)) => return Err(e).context("subscription failed"),
            None => match last_error {
                Some(e) => return Err(e).context("gave up reconnecting"),
                None => bail!("the NVR closed the subscription"),
            },
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

#![forbid(unsafe_code)]
#![allow(clippy::doc_markdown)]

//! `ferro-protect` -- command-line tool for the UniFi Protect local
//! integration API. Doubles as a living integration test for the
//! `ferro-protect` library.

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use ferro_protect::{ProtectClient, TlsMode};
use ferro_protect_cli::{api_key, commands};

/// Command-line interface for the UniFi Protect integration API.
#[derive(Debug, Parser)]
#[command(name = "ferro-protect", version, about, long_about = None)]
struct Cli {
    /// NVR hostname (or host:port). Mutually exclusive with --base-url.
    #[arg(long, global = true, env = "UNIFI_PROTECT_HOST")]
    host: Option<String>,

    /// Override the entire base URL (useful for tests).
    #[arg(long, global = true, env = "UNIFI_PROTECT_BASE_URL")]
    base_url: Option<String>,

    /// Path to a file containing the API key.
    ///
    /// The resolver also reads `UNIFI_PROTECT_API_KEY_FILE` and
    /// `UNIFI_PROTECT_API_KEY` env vars (in that priority order). The
    /// `env =` mapping is **deliberately not** declared on this flag so
    /// clap doesn't bypass the manual precedence logic in
    /// `api_key::resolve`.
    #[arg(long, global = true)]
    api_key_file: Option<std::path::PathBuf>,

    /// Skip TLS certificate validation. Use only with NVRs whose cert you
    /// cannot pin. Honours `UNIFI_PROTECT_INSECURE` from the env (accepts
    /// 1/true/yes/on and 0/false/no/off) so a single sourced `.env.local`
    /// drives both the CLI and the live tests.
    #[arg(
        long,
        global = true,
        env = "UNIFI_PROTECT_INSECURE",
        value_parser = clap::builder::BoolishValueParser::new(),
        num_args = 0..=1,
        default_value_t = false,
        default_missing_value = "true",
    )]
    insecure: bool,

    /// Emit JSON instead of human-formatted output.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Show application info (running Protect version, etc.).
    Info,
    /// Camera read endpoints.
    Cameras {
        #[command(subcommand)]
        action: commands::cameras::Action,
    },
    /// Chime read endpoints.
    Chimes {
        #[command(subcommand)]
        action: commands::chimes::Action,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    run(cli).await
}

async fn run(cli: Cli) -> Result<()> {
    // Resolve the key in a sync block so the stderr lock guard (which
    // isn't Send) never lives across an .await point.
    let key = {
        let mut stderr = std::io::stderr().lock();
        api_key::resolve(
            cli.api_key_file.as_deref(),
            &|name| std::env::var(name).ok(),
            &mut stderr,
        )?
    };

    let mut builder = ProtectClient::builder().api_key(key);
    match (&cli.base_url, &cli.host) {
        (Some(url), _) => builder = builder.base_url(url),
        (None, Some(host)) => builder = builder.host(host),
        (None, None) => return Err(anyhow!("one of --host or --base-url is required")),
    }
    if cli.insecure {
        builder = builder.tls(TlsMode::AcceptInvalid);
    }
    let client = builder.build().context("failed to construct client")?;

    match cli.command {
        Command::Info => {
            let info = client.info().await.context("info request failed")?;
            ferro_protect_cli::output::emit_stdout(&info, cli.json, || {
                format!(
                    "Protect application version: {}\n",
                    info.application_version
                )
            })?;
        }
        Command::Cameras { action } => commands::cameras::run(&client, action, cli.json).await?,
        Command::Chimes { action } => commands::chimes::run(&client, action, cli.json).await?,
    }
    Ok(())
}

#![forbid(unsafe_code)]
#![allow(clippy::doc_markdown)]

//! `ferro-protect` -- command-line tool for the UniFi Protect local
//! integration API. Doubles as a living integration test for the
//! `ferro-protect` library.

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use ferro_protect::{ProtectClient, TlsMode};
use secrecy::SecretString;

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

    /// Path to a file containing the API key (phase 3 will broaden this).
    #[arg(long, global = true)]
    api_key_file: Option<std::path::PathBuf>,

    /// **TEMPORARY (phase 2 scaffold).** Pass the API key directly. Will be
    /// removed in phase 3 when the smart loader lands. Use `--api-key-file`
    /// or env vars instead in any real workflow.
    // TODO: remove in phase 3
    #[arg(long, global = true, hide = true)]
    api_key: Option<String>,

    /// Skip TLS certificate validation. Use only with NVRs whose cert you
    /// cannot pin.
    #[arg(long, global = true)]
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
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    run(cli).await
}

async fn run(cli: Cli) -> Result<()> {
    let key = resolve_api_key(&cli)?;

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
            if cli.json {
                let json = serde_json::to_string_pretty(&info)?;
                println!("{json}");
            } else {
                println!("Protect application version: {}", info.application_version);
            }
        }
    }
    Ok(())
}

/// Phase-2 placeholder for the smart API key loader. Replaced wholesale in
/// phase 3 by a `cli::api_key::resolve()` that follows the
/// flag > env-var-pointer > env-var-raw precedence.
fn resolve_api_key(cli: &Cli) -> Result<SecretString> {
    if let Some(path) = &cli.api_key_file {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading API key from {}", path.display()))?;
        return Ok(SecretString::from(raw.trim().to_string()));
    }
    if let Some(raw) = &cli.api_key {
        return Ok(SecretString::from(raw.clone()));
    }
    Err(anyhow!(
        "no API key provided -- pass --api-key-file <PATH> or (temporarily) --api-key <KEY>"
    ))
}

#![forbid(unsafe_code)]

//! `ferro-protect` -- command-line tool for the UniFi Protect local
//! integration API. Doubles as a living integration test for the
//! `ferro-protect` library.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use clap::{Parser, Subcommand};
use ferro_protect::{ProtectClient, TlsMode};
use ferro_protect_cli::api_key::Sources;
use ferro_protect_cli::config::{self, Flags};
use ferro_protect_cli::logging::LogLevel;
use ferro_protect_cli::{api_key, commands, logging};

/// Command-line interface for the UniFi Protect integration API.
///
/// All global flags can also be set via env vars (see `--help` long
/// form on each) or in the TOML config file at
/// `$XDG_CONFIG_HOME/ferro-protect/config.toml`. Precedence is
/// **flag > env > config file > built-in default**. Run
/// `ferro-protect config init` to generate the file interactively, or
/// `ferro-protect config show` to inspect the effective configuration
/// and the source of each value.
#[derive(Debug, Parser)]
#[command(name = "ferro-protect", version, about, long_about = None)]
struct Cli {
    /// NVR hostname (or host:port). Mutually exclusive with --base-url.
    ///
    /// Resolution order: this flag, then `UNIFI_PROTECT_HOST` env var,
    /// then the `host` key in the config file. Hostname only -- no
    /// scheme prefix, no path. The client wraps it as
    /// `https://{host}/proxy/protect/integration`.
    #[arg(long, global = true, conflicts_with = "base_url")]
    host: Option<String>,

    /// Override the entire base URL (useful for tests). Mutually
    /// exclusive with --host.
    ///
    /// Resolution order: this flag, then `UNIFI_PROTECT_BASE_URL` env
    /// var, then the `base_url` key in the config file.
    #[arg(long, global = true)]
    base_url: Option<String>,

    /// Path to a file containing the API key.
    ///
    /// Resolution order for the API key (highest first):
    ///   1. This flag.
    ///   2. `UNIFI_PROTECT_API_KEY_FILE` env (path).
    ///   3. `UNIFI_PROTECT_API_KEY` env (raw key).
    ///   4. `api_key_file` in the config file (path).
    ///   5. `api_key` in the config file (raw, discouraged).
    ///
    /// The `env =` mapping is **deliberately not** declared on this
    /// flag so clap doesn't bypass the manual precedence logic in
    /// `api_key::resolve`.
    #[arg(long, global = true)]
    api_key_file: Option<PathBuf>,

    /// Path to the TOML config file to load.
    ///
    /// File-discovery precedence (highest first): this flag,
    /// `UNIFI_PROTECT_CONFIG_FILE` env var, then the XDG default
    /// (`$XDG_CONFIG_HOME/ferro-protect/config.toml`).
    ///
    /// The first two are *authoritative*: a missing file is a hard
    /// error. The XDG default is *opportunistic*: a missing file at
    /// the XDG path is fine and means "no config".
    ///
    /// The env-var name mirrors `UNIFI_PROTECT_API_KEY_FILE`: `_FILE`
    /// suffix means "path to a file" (vs. a raw value).
    ///
    /// The `env =` mapping is **deliberately not** declared on this
    /// flag so the env lookup happens explicitly in `config::load` and
    /// stays separable from the field-level resolver.
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Skip TLS certificate validation. Use only with NVRs whose cert
    /// you cannot pin. Accepts `1`/`0`, `true`/`false`, `yes`/`no`,
    /// `on`/`off`, case-insensitive (also bare `--insecure` for true).
    ///
    /// Resolution order: this flag, then `UNIFI_PROTECT_INSECURE` env
    /// var, then the `insecure` key in the config file. Default
    /// `false`.
    #[arg(
        long,
        global = true,
        value_parser = clap::builder::BoolishValueParser::new(),
        num_args = 0..=1,
        default_missing_value = "true",
        require_equals = true,
    )]
    insecure: Option<bool>,

    /// Emit JSON instead of human-formatted output. Accepts the same
    /// `1/0/true/false/yes/no/on/off` vocabulary as `--insecure`.
    ///
    /// Resolution order: this flag, then `UNIFI_PROTECT_JSON` env var,
    /// then the `json` key in the config file. Default `false`. To
    /// supply an explicit value use `--json=true` / `--json=false`;
    /// bare `--json` is equivalent to `--json=true`.
    #[arg(
        long,
        global = true,
        value_parser = clap::builder::BoolishValueParser::new(),
        num_args = 0..=1,
        default_missing_value = "true",
        require_equals = true,
    )]
    json: Option<bool>,

    /// Log level for diagnostic output (writes to stderr).
    ///
    /// Resolution order: this flag, then `UNIFI_PROTECT_LOG` env var
    /// (env_logger filter syntax), then `RUST_LOG`, then the
    /// `log_level` key in the config file, then the literal default
    /// `warn`.
    #[arg(long, value_enum, global = true)]
    log_level: Option<LogLevel>,

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
    /// Light read endpoints.
    Lights {
        #[command(subcommand)]
        action: commands::lights::Action,
    },
    /// Liveview read endpoints.
    Liveviews {
        #[command(subcommand)]
        action: commands::liveviews::Action,
    },
    /// NVR read endpoint (get only; one per installation).
    Nvrs {
        #[command(subcommand)]
        action: commands::nvrs::Action,
    },
    /// Sensor read endpoints.
    Sensors {
        #[command(subcommand)]
        action: commands::sensors::Action,
    },
    /// Viewer read endpoints.
    Viewers {
        #[command(subcommand)]
        action: commands::viewers::Action,
    },
    /// Manage the persistent TOML config file. See the subcommand's
    /// own `--help` for the five actions (init/show/edit/path/delete).
    Config {
        #[command(subcommand)]
        action: commands::config::Action,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    run(cli).await
}

async fn run(cli: Cli) -> Result<()> {
    // Config-file commands are special: they should run before we
    // try to build a `ProtectClient`. The `Config` subcommand
    // doesn't need an API key, a host, or anything network-shaped.
    // It also doesn't depend on the file's log_level (the file is
    // what it's *managing*), so flag-only init is correct here.
    if let Command::Config { action } = cli.command {
        logging::init(cli.log_level.map(LogLevel::as_filter), None);
        return commands::config::run(action, cli.config.as_deref(), cli.json.unwrap_or(false))
            .map_err(Into::into);
    }

    let env = |k: &str| std::env::var(k).ok();
    let loaded = config::load(cli.config.as_deref(), &env)?;

    let flags = Flags {
        host: cli.host.clone(),
        base_url: cli.base_url.clone(),
        api_key_file: cli.api_key_file.clone(),
        insecure: cli.insecure,
        json: cli.json,
        log_level: cli.log_level,
    };
    let resolved = config::resolve(&flags, loaded.as_ref(), &env);

    // Init logging *after* config load so the file's `log_level` can
    // act as a fallback when neither --log-level nor UNIFI_PROTECT_LOG
    // / RUST_LOG is set. The flag is still highest-priority via the
    // `cli_level` arg.
    let file_log_fallback = loaded
        .as_ref()
        .and_then(|lc| lc.file.log_level)
        .map(LogLevel::as_filter);
    logging::init(cli.log_level.map(LogLevel::as_filter), file_log_fallback);

    log::debug!(
        "ferro-protect starting: command={:?}, json={:?}, insecure={:?}",
        std::mem::discriminant(&cli.command),
        cli.json,
        cli.insecure,
    );

    // Cross-source mutual exclusion: clap's `conflicts_with` only
    // catches the `--host ... --base-url ...` case on argv. A user can
    // still set `host` in the file and `--base-url` on the flag, or
    // mix env + file. Reject any combination where both end up resolved.
    if let (Some(h), Some(b)) = (resolved.host.as_ref(), resolved.base_url.as_ref()) {
        return Err(anyhow!(
            "`host` and `base_url` cannot both be set. \
             host comes from {:?}, base_url from {:?}. \
             Pick one source per option and clear the other.",
            h.source,
            b.source,
        ));
    }

    // Resolve the key in a sync block so the stderr lock guard (which
    // isn't Send) never lives across an .await point.
    let (key, _key_source) = {
        let mut stderr = std::io::stderr().lock();
        let sources = Sources {
            flag_file: cli.api_key_file.as_deref(),
            config_file: loaded
                .as_ref()
                .and_then(|lc| lc.file.api_key_file.as_deref()),
            config_raw: loaded.as_ref().and_then(|lc| lc.file.api_key.as_ref()),
        };
        api_key::resolve(&sources, &env, &mut stderr)?
    };
    log::debug!("api key resolved (source resolution complete)");

    let mut builder = ProtectClient::builder().api_key(key);
    match (
        resolved.base_url.as_ref().map(|r| r.value.as_str()),
        resolved.host.as_ref().map(|r| r.value.as_str()),
    ) {
        (Some(url), _) => builder = builder.base_url(url),
        (None, Some(host)) => builder = builder.host(host),
        (None, None) => return Err(anyhow!("one of --host or --base-url is required")),
    }
    if resolved.insecure.value {
        builder = builder.tls(TlsMode::AcceptInvalid);
    }
    let client = builder.build().context("failed to construct client")?;

    let json = resolved.json.value;
    match cli.command {
        Command::Info => {
            let info = client.info().await.context("info request failed")?;
            ferro_protect_cli::output::emit_stdout(&info, json, || {
                format!(
                    "Protect application version: {}\n",
                    info.application_version
                )
            })?;
        }
        Command::Cameras { action } => commands::cameras::run(&client, action, json).await?,
        Command::Chimes { action } => commands::chimes::run(&client, action, json).await?,
        Command::Lights { action } => commands::lights::run(&client, action, json).await?,
        Command::Liveviews { action } => commands::liveviews::run(&client, action, json).await?,
        Command::Nvrs { action } => commands::nvrs::run(&client, action, json).await?,
        Command::Sensors { action } => commands::sensors::run(&client, action, json).await?,
        Command::Viewers { action } => commands::viewers::run(&client, action, json).await?,
        Command::Config { .. } => unreachable!("handled above"),
    }
    Ok(())
}

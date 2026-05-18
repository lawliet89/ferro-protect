//! Logger initialization for the `ferro-protect` binary.
//!
//! Filter resolution order, highest priority first:
//!
//! 1. The `--log-level` flag (turned into a [`log::LevelFilter`] by the
//!    caller and passed to [`init`]).
//! 2. `UNIFI_PROTECT_LOG` env var (parsed the same way `env_logger`
//!    parses `RUST_LOG`).
//! 3. `RUST_LOG` env var (env_logger's native default).
//! 4. The literal default `warn`.
//!
//! Logs are written to **stderr** so they don't pollute the `stdout`
//! that `--json` and the human tables produce.

use std::io::Write;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

const APP_ENV: &str = "UNIFI_PROTECT_LOG";
const DEFAULT_FILTER: &str = "warn";

/// Log-level choice. Used by both the `--log-level` clap value-enum
/// and the `log_level` key in the config file.
///
/// Lowercase wire form so both surfaces accept the same set of strings:
/// `error`, `warn`, `info`, `debug`, `trace`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    #[must_use]
    pub const fn as_filter(self) -> log::LevelFilter {
        match self {
            Self::Error => log::LevelFilter::Error,
            Self::Warn => log::LevelFilter::Warn,
            Self::Info => log::LevelFilter::Info,
            Self::Debug => log::LevelFilter::Debug,
            Self::Trace => log::LevelFilter::Trace,
        }
    }

    /// Wire-form string, matching the clap `ValueEnum` derive
    /// (lowercase). Used by `config show` and `config edit` to emit /
    /// accept the same vocabulary as `--log-level`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }
}

/// Initialize the global logger.
///
/// `cli_level` wins over both env vars when present. Re-initialisation
/// is a no-op (env_logger's `try_init` returns an error which we
/// swallow) so test binaries that call this from multiple `#[tokio::test]`
/// async tasks don't trip over each other.
pub fn init(cli_level: Option<log::LevelFilter>) {
    let mut builder = env_logger::Builder::new();

    // Pin format so test assertions can match exact strings. Module
    // path is included at debug+ for grep-ability.
    builder.format(|buf, record| {
        let ts = buf.timestamp();
        if record.level() <= log::Level::Info {
            writeln!(buf, "{ts} {} {}", record.level(), record.args())
        } else {
            writeln!(
                buf,
                "{ts} {} {} {}",
                record.level(),
                record.target(),
                record.args()
            )
        }
    });
    builder.target(env_logger::Target::Stderr);

    // Precedence: explicit flag > UNIFI_PROTECT_LOG > RUST_LOG > "warn".
    if let Some(level) = cli_level {
        builder.filter_level(level);
    } else if let Ok(filter) = std::env::var(APP_ENV) {
        builder.parse_filters(&filter);
    } else if let Ok(filter) = std::env::var("RUST_LOG") {
        builder.parse_filters(&filter);
    } else {
        builder.parse_filters(DEFAULT_FILTER);
    }

    // Ignore "logger already set" -- it just means we ran twice.
    let _ = builder.try_init();
}

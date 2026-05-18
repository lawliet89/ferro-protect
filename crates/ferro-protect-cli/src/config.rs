//! Persistent on-disk configuration for the `ferro-protect` CLI.
//!
//! See `docs/TASK_config_file.md` for the design rationale and
//! `README.md` for user-facing documentation. This module owns:
//!
//! - [`ConfigFile`] — the on-disk schema (TOML, deserialized via
//!   `toml_edit::de`).
//! - [`load`] — file-discovery precedence (`--config` flag >
//!   `UNIFI_PROTECT_CONFIG_FILE` env > XDG default), parses + validates.
//! - [`resolve`] — pure merger that turns ([`Flags`], optional
//!   [`LoadedConfig`], env callback) into a [`ResolvedConfig`] with
//!   per-field [`FieldSource`] attribution.
//!
//! API-key resolution lives in [`crate::api_key`]; this module just
//! surfaces the file-derived sources to it via [`api_key::Sources`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use secrecy::SecretString;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

use crate::logging::LogLevel;

/// Env var that picks the config *file* (not a field within it). Kept
/// separate from [`FIELDS`] because it's about file discovery, not a
/// merged value.
pub const ENV_CONFIG_FILE: &str = "UNIFI_PROTECT_CONFIG_FILE";

/// What kind of TOML value a [`FieldMeta`] expects. Drives `config edit`
/// parsing and the `--template` scaffold's example RHS.
///
/// Marked non-exhaustive so adding a new variant (e.g. `Integer`) is
/// not a breaking change for callers in this crate that match on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    /// Free-form string (TOML `"..."`).
    String,
    /// File-system path (string in TOML, but conventionally tilde-expanded).
    Path,
    /// `true` / `false` (also accepts `1/0/yes/no/on/off` via the boolish parser).
    Bool,
    /// One of [`LogLevel`]'s variants (lowercase wire form).
    LogLevel,
}

/// Per-field metadata. Single source of truth for every recognised
/// config field — drives:
///
/// * the `config show` / `config edit` / `config list` key validators
/// * the `config init --template` scaffold
/// * `config edit`'s per-type value parsing
/// * `config::resolve`'s env-var lookup
/// * the "valid fields: …" help text in error messages
///
/// `api_key` is addressable in `show` (rendered as `<set>`/`<unset>`)
/// but **refused** in `edit` so a raw key never lands on argv.
#[derive(Debug, Clone, Copy)]
pub struct FieldMeta {
    /// Field name as it appears in TOML and on the CLI.
    pub key: &'static str,
    /// Human-readable purpose. One-liner; rendered as `# {description}`
    /// in the `--template` scaffold.
    pub description: &'static str,
    /// Example RHS for the `--template` scaffold. Include quotes if the
    /// value is a string.
    pub example: &'static str,
    /// How `config edit` parses a raw CLI value into a TOML value.
    pub ty: FieldType,
    /// Env var that `config::resolve` consults for this field, or `None`.
    /// `None` for `api_key`/`api_key_file` (env handling lives in
    /// [`crate::api_key`]) and `log_level` (the env_logger filter syntax
    /// of `UNIFI_PROTECT_LOG`/`RUST_LOG` can't reduce to a single
    /// `LogLevel`, so we keep that resolution inside `logging::init`).
    pub env_var: Option<&'static str>,
}

/// The canonical field table. Adding a new field starts here.
pub const FIELDS: &[FieldMeta] = &[
    FieldMeta {
        key: "host",
        description: "NVR hostname or host:port. Mutually exclusive with `base_url`.",
        example: "\"nvr.local\"",
        ty: FieldType::String,
        env_var: Some("UNIFI_PROTECT_HOST"),
    },
    FieldMeta {
        key: "base_url",
        description: "Override the entire base URL. Mutually exclusive with `host`.",
        example: "\"https://nvr.local/proxy/protect/integration\"",
        ty: FieldType::String,
        env_var: Some("UNIFI_PROTECT_BASE_URL"),
    },
    FieldMeta {
        key: "api_key_file",
        description: "Path to a file containing the API key (preferred over inline).",
        example: "\"~/.config/ferro-protect/api_key\"",
        ty: FieldType::Path,
        env_var: None,
    },
    FieldMeta {
        key: "api_key",
        description: "Raw API key inline (discouraged -- prefer `api_key_file`).",
        example: "\"...\"",
        ty: FieldType::String,
        env_var: None,
    },
    FieldMeta {
        key: "insecure",
        description: "Skip TLS certificate validation (typical for self-signed NVRs).",
        example: "false",
        ty: FieldType::Bool,
        env_var: Some("UNIFI_PROTECT_INSECURE"),
    },
    FieldMeta {
        key: "json",
        description: "Default to JSON output instead of human-readable text.",
        example: "false",
        ty: FieldType::Bool,
        env_var: Some("UNIFI_PROTECT_JSON"),
    },
    FieldMeta {
        key: "log_level",
        description: "Log level: error | warn | info | debug | trace.",
        example: "\"warn\"",
        ty: FieldType::LogLevel,
        env_var: None,
    },
];

/// Find a field by key. `None` for unknown keys.
#[must_use]
pub fn field(key: &str) -> Option<&'static FieldMeta> {
    FIELDS.iter().find(|f| f.key == key)
}

/// `true` when `key` matches an entry in [`FIELDS`].
#[must_use]
pub fn is_known_key(key: &str) -> bool {
    field(key).is_some()
}

/// `"host, base_url, ..."` — used in error messages and `config list`.
#[must_use]
pub fn known_keys_joined() -> String {
    FIELDS.iter().map(|f| f.key).collect::<Vec<_>>().join(", ")
}

/// Convenience: env-var name for a known field, panicking on miss. Used
/// by [`resolve`] where the key is hard-coded and known to be valid.
fn env_var_for(key: &str) -> &'static str {
    field(key)
        .and_then(|f| f.env_var)
        .unwrap_or_else(|| panic!("BUG: no env var for field `{key}`"))
}

/// On-disk config schema. Every field is optional; `None` means "not
/// set" (distinct from "set to the type default"), which lets us tell
/// "explicit `false`" apart from "absent" for source attribution.
///
/// `deny_unknown_fields` traps typos like `apikey = ...` at parse time.
/// Mutual-exclusion rules between fields (`host` vs `base_url`,
/// `api_key` vs `api_key_file`) are enforced by [`Self::validate`],
/// which the loader calls after parsing.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key_file: Option<PathBuf>,
    #[serde(default)]
    pub api_key: Option<SecretString>,
    #[serde(default)]
    pub insecure: Option<bool>,
    #[serde(default)]
    pub json: Option<bool>,
    #[serde(default)]
    pub log_level: Option<LogLevel>,
}

impl ConfigFile {
    /// File-level mutual exclusion. Same rules clap enforces at flag
    /// level. Run after deserialization; `toml_edit::de` won't enforce
    /// these for us.
    ///
    /// # Errors
    /// Returns [`ConfigError::HostAndBaseUrl`] or
    /// [`ConfigError::ApiKeyAndFile`] when the relevant pair is both set.
    #[expect(
        clippy::missing_const_for_fn,
        reason = "ConfigError variants with String fields make this not const-fn-able without splitting the error type"
    )]
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.host.is_some() && self.base_url.is_some() {
            return Err(ConfigError::HostAndBaseUrl);
        }
        if self.api_key.is_some() && self.api_key_file.is_some() {
            return Err(ConfigError::ApiKeyAndFile);
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("reading config file {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("parsing config file {}: {source}", path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: toml_edit::de::Error,
    },
    #[error("config file: cannot set both `host` and `base_url`")]
    HostAndBaseUrl,
    #[error("config file: cannot set both `api_key` and `api_key_file`")]
    ApiKeyAndFile,
    #[error(
        "config file not found at {}\n\
         (referenced via {})",
        path.display(),
        via.as_user_hint(),
    )]
    ExplicitMissing {
        path: PathBuf,
        via: FileDiscoverySource,
    },
    #[error("could not determine config directory: {0}")]
    NoConfigDir(String),
}

/// Which input picked the config-file path.
///
/// The XDG default is *opportunistic* (a missing file at the XDG path
/// is fine; we just return `Ok(None)`); the other two are
/// *authoritative* (a missing file is a hard error).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDiscoverySource {
    Flag,
    Env,
    XdgDefault,
}

impl FileDiscoverySource {
    pub(crate) const fn as_user_hint(self) -> &'static str {
        match self {
            Self::Flag => "--config flag",
            Self::Env => "UNIFI_PROTECT_CONFIG_FILE env var",
            Self::XdgDefault => "XDG default path",
        }
    }
}

/// Outcome of a successful [`load`]: the parsed file plus the path it
/// came from (used for source attribution in [`config show`]) and which
/// discovery source picked the path.
#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub file: ConfigFile,
    pub path: PathBuf,
    pub source: FileDiscoverySource,
}

/// Compute the XDG-default config path. Wraps `etcetera` so callers
/// don't have to thread the strategy through.
///
/// # Errors
/// Returns [`ConfigError::NoConfigDir`] if `etcetera` cannot determine a
/// base strategy (typically only on platforms without a home directory
/// concept).
pub fn xdg_default_path() -> Result<PathBuf, ConfigError> {
    use etcetera::{BaseStrategy, choose_base_strategy};
    let strat = choose_base_strategy().map_err(|e| ConfigError::NoConfigDir(e.to_string()))?;
    Ok(strat.config_dir().join("ferro-protect").join("config.toml"))
}

/// Resolve which path to load from, per the file-discovery precedence
/// documented at the module level. Does **not** check that the file
/// exists.
///
/// # Errors
/// [`ConfigError::NoConfigDir`] when falling back to XDG and the base
/// strategy is unavailable.
pub fn resolve_path<E>(
    flag: Option<&Path>,
    env: &E,
) -> Result<(PathBuf, FileDiscoverySource), ConfigError>
where
    E: Fn(&str) -> Option<String> + ?Sized,
{
    if let Some(p) = flag {
        return Ok((p.to_path_buf(), FileDiscoverySource::Flag));
    }
    if let Some(p) = env(ENV_CONFIG_FILE) {
        return Ok((PathBuf::from(p), FileDiscoverySource::Env));
    }
    Ok((xdg_default_path()?, FileDiscoverySource::XdgDefault))
}

/// Load the config file per the file-discovery precedence.
///
/// Returns `Ok(None)` only when the XDG default was selected and the
/// file does not exist. Authoritative sources (flag, env) hard-error
/// on missing file via [`ConfigError::ExplicitMissing`].
///
/// # Errors
/// - [`ConfigError::ExplicitMissing`] — authoritative source pointed
///   at a missing file.
/// - [`ConfigError::Read`] — I/O error other than `NotFound`.
/// - [`ConfigError::Parse`] — TOML deserialization error.
/// - [`ConfigError::HostAndBaseUrl`] / [`ConfigError::ApiKeyAndFile`]
///   — file-level mutual-exclusion violation.
pub fn load<E>(flag: Option<&Path>, env: &E) -> Result<Option<LoadedConfig>, ConfigError>
where
    E: Fn(&str) -> Option<String> + ?Sized,
{
    let (path, source) = resolve_path(flag, env)?;
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return match source {
                FileDiscoverySource::Flag | FileDiscoverySource::Env => {
                    Err(ConfigError::ExplicitMissing { path, via: source })
                }
                FileDiscoverySource::XdgDefault => {
                    log::debug!(
                        "config: no config file found at XDG default {}",
                        path.display()
                    );
                    Ok(None)
                }
            };
        }
        Err(io_err) => {
            return Err(ConfigError::Read {
                path,
                source: io_err,
            });
        }
    };

    let file: ConfigFile = toml_edit::de::from_str(&raw).map_err(|e| ConfigError::Parse {
        path: path.clone(),
        source: e,
    })?;
    file.validate()?;

    log::debug!(
        "config: loaded from {} (via {})",
        path.display(),
        source.as_user_hint(),
    );
    Ok(Some(LoadedConfig { file, path, source }))
}

/// Flag inputs to [`resolve`]. Decoupled from the clap-derived `Cli`
/// struct so unit tests can construct it without bringing in clap.
/// `None` on the `Option<_>` fields means "flag was not passed".
#[derive(Debug, Default, Clone)]
pub struct Flags {
    pub host: Option<String>,
    pub base_url: Option<String>,
    pub api_key_file: Option<PathBuf>,
    pub insecure: Option<bool>,
    pub json: Option<bool>,
    pub log_level: Option<LogLevel>,
}

/// Which source provided the effective value for a given field.
/// `Env(name)` carries the env var name so [`config show`] can attribute
/// per-field (e.g. `from env: UNIFI_PROTECT_HOST`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "name", rename_all = "snake_case")]
pub enum FieldSource {
    Flag,
    Env(&'static str),
    ConfigFile,
    Default,
}

/// A field value paired with its source. Used by [`config show`].
#[derive(Debug, Clone, Serialize)]
pub struct Resolved<T> {
    pub value: T,
    pub source: FieldSource,
}

/// Effective config after merging flags, env, and file. For optional
/// fields, `None` means "no source supplied a value" — printed as
/// `<unset>` by `config show`.
#[derive(Debug)]
pub struct ResolvedConfig {
    pub host: Option<Resolved<String>>,
    pub base_url: Option<Resolved<String>>,
    pub api_key_file: Option<Resolved<PathBuf>>,
    pub insecure: Resolved<bool>,
    pub json: Resolved<bool>,
    /// Always populated -- `FieldSource::Default` carries `LogLevel::Warn`
    /// when no flag / file value was supplied. `UNIFI_PROTECT_LOG` and
    /// `RUST_LOG` are *not* reflected here because their `env_logger`
    /// filter syntax cannot be reduced to a single `LogLevel` variant;
    /// they may still override this value at the live logger.
    pub log_level: Resolved<LogLevel>,
    /// The path of the file the values were merged from, if any. Used
    /// for `ConfigFile` source attribution in `config show`.
    pub config_file_path: Option<PathBuf>,
}

/// Merge flags + env + file into a [`ResolvedConfig`].
///
/// API-key resolution is intentionally **not** done here; the API key
/// has its own multi-source resolver in [`crate::api_key`] that takes a
/// [`crate::api_key::Sources`] built from the same inputs.
pub fn resolve<E>(flags: &Flags, file: Option<&LoadedConfig>, env: &E) -> ResolvedConfig
where
    E: Fn(&str) -> Option<String> + ?Sized,
{
    let cf = file.map(|lc| &lc.file);
    let host = resolve_string(
        env_var_for("host"),
        flags.host.as_deref(),
        env,
        cf.and_then(|c| c.host.as_deref()),
    );
    let base_url = resolve_string(
        env_var_for("base_url"),
        flags.base_url.as_deref(),
        env,
        cf.and_then(|c| c.base_url.as_deref()),
    );
    let api_key_file = resolve_path_field(
        flags.api_key_file.as_deref(),
        cf.and_then(|c| c.api_key_file.as_deref()),
    );
    let insecure = resolve_bool(
        env_var_for("insecure"),
        flags.insecure,
        env,
        cf.and_then(|c| c.insecure),
        false,
    );
    let json = resolve_bool(
        env_var_for("json"),
        flags.json,
        env,
        cf.and_then(|c| c.json),
        false,
    );
    // No env source for log_level on purpose: `--log-level` env handling
    // lives in the logger init flow and uses the same
    // `UNIFI_PROTECT_LOG` / `RUST_LOG` string-filter syntax as
    // `env_logger`, which can't be reduced to a single `LogLevel`
    // variant. `config show` reports the file attribution; whether the
    // live logger is running with that value is a separate question.
    #[expect(
        clippy::option_if_let_else,
        reason = "three-way precedence chain reads more clearly as if/else-if than nested map_or_else"
    )]
    let log_level = if let Some(v) = flags.log_level {
        Resolved {
            value: v,
            source: FieldSource::Flag,
        }
    } else if let Some(v) = cf.and_then(|c| c.log_level) {
        Resolved {
            value: v,
            source: FieldSource::ConfigFile,
        }
    } else {
        Resolved {
            value: LogLevel::Warn,
            source: FieldSource::Default,
        }
    };

    ResolvedConfig {
        host,
        base_url,
        api_key_file,
        insecure,
        json,
        log_level,
        config_file_path: file.map(|lc| lc.path.clone()),
    }
}

fn resolve_string<E>(
    env_name: &'static str,
    flag: Option<&str>,
    env: &E,
    file: Option<&str>,
) -> Option<Resolved<String>>
where
    E: Fn(&str) -> Option<String> + ?Sized,
{
    if let Some(v) = flag {
        return Some(Resolved {
            value: v.to_owned(),
            source: FieldSource::Flag,
        });
    }
    if let Some(v) = env(env_name)
        && !v.is_empty()
    {
        return Some(Resolved {
            value: v,
            source: FieldSource::Env(env_name),
        });
    }
    file.map(|v| Resolved {
        value: v.to_owned(),
        source: FieldSource::ConfigFile,
    })
}

fn resolve_path_field(flag: Option<&Path>, file: Option<&Path>) -> Option<Resolved<PathBuf>> {
    if let Some(p) = flag {
        return Some(Resolved {
            value: p.to_path_buf(),
            source: FieldSource::Flag,
        });
    }
    file.map(|p| Resolved {
        value: p.to_path_buf(),
        source: FieldSource::ConfigFile,
    })
}

fn resolve_bool<E>(
    env_name: &'static str,
    flag: Option<bool>,
    env: &E,
    file: Option<bool>,
    default: bool,
) -> Resolved<bool>
where
    E: Fn(&str) -> Option<String> + ?Sized,
{
    if let Some(v) = flag {
        return Resolved {
            value: v,
            source: FieldSource::Flag,
        };
    }
    if let Some(raw) = env(env_name)
        && let Some(parsed) = parse_boolish(&raw)
    {
        return Resolved {
            value: parsed,
            source: FieldSource::Env(env_name),
        };
    }
    if let Some(v) = file {
        return Resolved {
            value: v,
            source: FieldSource::ConfigFile,
        };
    }
    Resolved {
        value: default,
        source: FieldSource::Default,
    }
}

/// Same vocabulary as `clap::builder::BoolishValueParser`: accepts
/// `1`/`0`, `true`/`false`, `yes`/`no`, `on`/`off`, case-insensitive.
/// Returns `None` on anything else so the caller can fall through.
fn parse_boolish(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_from<I, K, V>(pairs: I) -> impl Fn(&str) -> Option<String>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let map: HashMap<String, String> = pairs
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        move |k| map.get(k).cloned()
    }

    fn empty_env() -> impl Fn(&str) -> Option<String> {
        |_| None
    }

    #[test]
    fn parse_boolish_accepts_clap_vocabulary() {
        for s in ["1", "true", "TRUE", "yes", "on"] {
            assert_eq!(parse_boolish(s), Some(true), "input {s}");
        }
        for s in ["0", "false", "FALSE", "no", "off"] {
            assert_eq!(parse_boolish(s), Some(false), "input {s}");
        }
        for s in ["maybe", "", "  "] {
            assert_eq!(parse_boolish(s), None, "input {s:?}");
        }
    }

    #[test]
    fn validate_rejects_host_plus_base_url() {
        let cf = ConfigFile {
            host: Some("h".into()),
            base_url: Some("https://x".into()),
            ..Default::default()
        };
        assert!(matches!(cf.validate(), Err(ConfigError::HostAndBaseUrl)));
    }

    #[test]
    fn validate_rejects_api_key_plus_file() {
        let cf = ConfigFile {
            api_key: Some(SecretString::from("k")),
            api_key_file: Some(PathBuf::from("/k")),
            ..Default::default()
        };
        assert!(matches!(cf.validate(), Err(ConfigError::ApiKeyAndFile)));
    }

    #[test]
    fn resolve_flag_wins_over_env_and_file() {
        let flags = Flags {
            host: Some("from-flag".into()),
            ..Default::default()
        };
        let lc = LoadedConfig {
            file: ConfigFile {
                host: Some("from-file".into()),
                ..Default::default()
            },
            path: PathBuf::from("/cfg"),
            source: FileDiscoverySource::Flag,
        };
        let env = env_from([("UNIFI_PROTECT_HOST", "from-env")]);
        let r = resolve(&flags, Some(&lc), &env);
        let h = r.host.unwrap();
        assert_eq!(h.value, "from-flag");
        assert_eq!(h.source, FieldSource::Flag);
    }

    #[test]
    fn resolve_env_wins_over_file_when_no_flag() {
        let lc = LoadedConfig {
            file: ConfigFile {
                host: Some("from-file".into()),
                ..Default::default()
            },
            path: PathBuf::from("/cfg"),
            source: FileDiscoverySource::XdgDefault,
        };
        let env = env_from([("UNIFI_PROTECT_HOST", "from-env")]);
        let r = resolve(&Flags::default(), Some(&lc), &env);
        let h = r.host.unwrap();
        assert_eq!(h.value, "from-env");
        assert_eq!(h.source, FieldSource::Env("UNIFI_PROTECT_HOST"));
    }

    #[test]
    fn resolve_file_wins_when_no_flag_or_env() {
        let lc = LoadedConfig {
            file: ConfigFile {
                host: Some("from-file".into()),
                ..Default::default()
            },
            path: PathBuf::from("/cfg"),
            source: FileDiscoverySource::XdgDefault,
        };
        let r = resolve(&Flags::default(), Some(&lc), &empty_env());
        let h = r.host.unwrap();
        assert_eq!(h.value, "from-file");
        assert_eq!(h.source, FieldSource::ConfigFile);
    }

    #[test]
    fn resolve_bool_default_when_unset() {
        let r = resolve(&Flags::default(), None, &empty_env());
        assert!(!r.insecure.value);
        assert_eq!(r.insecure.source, FieldSource::Default);
    }

    #[test]
    fn resolve_empty_env_string_falls_through_to_file() {
        let lc = LoadedConfig {
            file: ConfigFile {
                host: Some("from-file".into()),
                ..Default::default()
            },
            path: PathBuf::from("/cfg"),
            source: FileDiscoverySource::XdgDefault,
        };
        let env = env_from([("UNIFI_PROTECT_HOST", "")]);
        let r = resolve(&Flags::default(), Some(&lc), &env);
        // Empty env string is treated as "not set" so the file value
        // wins. Matches the spirit of clap's behavior, where setting
        // an env var to `""` typically means "don't override".
        assert_eq!(r.host.unwrap().source, FieldSource::ConfigFile);
    }
}

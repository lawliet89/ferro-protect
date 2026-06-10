//! Persistent on-disk configuration for the `ferro-protect` CLI.
//!
//! See `README.md` for user-facing documentation. This module owns:
//!
//! - [`ConfigFile`] — the on-disk schema (TOML, deserialized via
//!   `toml::de`).
//! - [`load`] — file-discovery precedence (`--config` flag >
//!   `UNIFI_PROTECT_CONFIG_FILE` env > XDG default), parses + validates.
//! - [`resolve`] — pure merger that turns ([`Flags`], optional
//!   [`LoadedConfig`], env callback) into an [`EffectiveConfig`] of
//!   plain values. Cross-source `host`/`base_url` mutual exclusion is
//!   enforced here too, so callers don't have to repeat the check.
//!
//! API-key resolution lives in [`crate::api_key`]; this module just
//! surfaces the file-derived sources to it via [`api_key::Sources`].

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

use crate::logging::LogLevel;

/// Env var that picks which config *file* the loader opens (distinct
/// from any field within it).
pub const ENV_CONFIG_FILE: &str = "UNIFI_PROTECT_CONFIG_FILE";

/// On-disk config schema. Every field is `Option<T>` so the resolver
/// can tell "absent" from "explicit `false`" and let env / flag
/// values fill in only the unset slots.
///
/// `deny_unknown_fields` traps typos (`apikey = ...`) and any
/// unsupported key -- including an inline `api_key`, which was never a
/// supported field -- at parse time. Mutual exclusion between `host`
/// and `base_url` is enforced by [`Self::validate`], which the loader
/// calls after parsing.
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
    pub insecure: Option<bool>,
    #[serde(default)]
    pub json: Option<bool>,
    #[serde(default)]
    pub log_level: Option<LogLevel>,
}

impl ConfigFile {
    /// File-level mutual exclusion. Same rules clap enforces at flag
    /// level. Run after deserialization; `toml::de` won't enforce
    /// these for us.
    ///
    /// # Errors
    /// - [`ConfigError::HostAndBaseUrl`] — mutual-exclusion violation.
    /// - [`ConfigError::EmptyValue`] — a string-valued field is set but
    ///   empty or whitespace-only. We reject these here rather than
    ///   pass them through, because they all fail later (empty URL,
    ///   `read_to_string` on an empty path) with less actionable
    ///   messages.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.host.is_some() && self.base_url.is_some() {
            return Err(ConfigError::HostAndBaseUrl);
        }
        if let Some(s) = self.host.as_deref()
            && s.trim().is_empty()
        {
            return Err(ConfigError::EmptyValue { field: "host" });
        }
        if let Some(s) = self.base_url.as_deref()
            && s.trim().is_empty()
        {
            return Err(ConfigError::EmptyValue { field: "base_url" });
        }
        // TOML strings are always UTF-8, so any `PathBuf` deserialized
        // from the config file round-trips through `to_str()`. Treat
        // whitespace-only the same as empty -- it can't be a real path
        // and would otherwise fail later at `read_to_string` with a
        // less actionable message.
        if let Some(p) = self.api_key_file.as_deref()
            && p.to_str().is_some_and(|s| s.trim().is_empty())
        {
            return Err(ConfigError::EmptyValue {
                field: "api_key_file",
            });
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
        source: toml::de::Error,
    },
    #[error("config file: cannot set both `host` and `base_url`")]
    HostAndBaseUrl,
    #[error(
        "config file not found at {}\n\
         (referenced via {})",
        path.display(),
        via,
    )]
    ExplicitMissing {
        path: PathBuf,
        via: FileDiscoverySource,
    },
    #[error("could not determine config directory: {0}")]
    NoConfigDir(String),
    #[error(
        "env `{name}` has invalid value `{value}`: \
         expected one of 1/0, true/false, yes/no, on/off (case-insensitive)"
    )]
    BadEnvBool { name: &'static str, value: String },
    #[error(
        "config file: `{field}` is empty. \
         Comment the line out or remove the entry instead of setting it to an empty value."
    )]
    EmptyValue { field: &'static str },
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

impl fmt::Display for FileDiscoverySource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Flag => "--config flag",
            Self::Env => "UNIFI_PROTECT_CONFIG_FILE env var",
            Self::XdgDefault => "XDG default path",
        };
        f.write_str(label)
    }
}

/// Outcome of a successful [`load`].
///
/// The parsed file plus the path it came from and which discovery
/// source picked the path. `path` is used by the `ExplicitMissing`
/// error to point at the file the flag or env var referenced.
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
    // Empty / whitespace-only env value falls through to the XDG
    // default. Matches the rule we apply for `UNIFI_PROTECT_API_KEY`
    // / `UNIFI_PROTECT_API_KEY_FILE` / `UNIFI_PROTECT_HOST` etc., and
    // keeps `UNIFI_PROTECT_CONFIG_FILE=""` from resolving to a literal
    // empty path that then hard-errors at `read_to_string`.
    if let Some(p) = env(ENV_CONFIG_FILE) {
        let trimmed = p.trim();
        if !trimmed.is_empty() {
            return Ok((PathBuf::from(trimmed), FileDiscoverySource::Env));
        }
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
/// - [`ConfigError::HostAndBaseUrl`] — file-level mutual-exclusion
///   violation.
pub fn load<E>(flag: Option<&Path>, env: &E) -> Result<Option<LoadedConfig>, ConfigError>
where
    E: Fn(&str) -> Option<String> + ?Sized,
{
    let (path, source) = resolve_path(flag, env)?;
    // No `log::debug!` in this function: the only caller (`main::run`)
    // initialises logging *after* `load`, so any messages emitted here
    // would be silently dropped. `main` logs the outcome itself.
    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return match source {
                FileDiscoverySource::Flag | FileDiscoverySource::Env => {
                    Err(ConfigError::ExplicitMissing { path, via: source })
                }
                FileDiscoverySource::XdgDefault => Ok(None),
            };
        }
        Err(io_err) => {
            return Err(ConfigError::Read {
                path,
                source: io_err,
            });
        }
    };

    let mut file: ConfigFile = toml::from_str(&raw).map_err(|e| ConfigError::Parse {
        path: path.clone(),
        source: e,
    })?;
    file.validate()?;
    // Normalise tilde-paths once at load time so every downstream
    // consumer (`api_key::resolve`, `config show`, etc.) sees an
    // absolute path. TOML is not a shell, so `~/...` isn't expanded by
    // the parser. Users hand-editing `api_key_file = "~/..."` is the
    // case to support; without this, `read_to_string` would fail at
    // runtime against a literal `~` directory.
    if let Some(p) = file.api_key_file.take() {
        file.api_key_file = Some(expand_tilde(&p));
    }

    Ok(Some(LoadedConfig { file, path, source }))
}

/// Replace a leading `~/` (or bare `~`) with the value of `$HOME`.
///
/// Returns the path unchanged when:
///
/// - `HOME` is unset (the typical Windows case — `USERPROFILE` is *not*
///   honoured by design; users running on Windows should write absolute
///   paths or set `HOME` explicitly),
/// - the path doesn't start with `~`,
/// - or the path's `~user` form is used (intentionally not supported).
///
/// Public because `config::load` calls it once at parse time to
/// normalise `api_key_file` so every downstream consumer
/// (`api_key::resolve`, `config show`, …) sees an absolute path.
#[must_use]
pub fn expand_tilde(p: &Path) -> PathBuf {
    let Some(s) = p.to_str() else {
        return p.to_path_buf();
    };
    let Some(home) = std::env::var_os("HOME") else {
        return p.to_path_buf();
    };
    if let Some(rest) = s.strip_prefix("~/") {
        return PathBuf::from(home).join(rest);
    }
    if s == "~" {
        return PathBuf::from(home);
    }
    p.to_path_buf()
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

/// Effective config after merging flags, env, and file. Plain values:
/// per-source attribution and the `Resolved<T>` wrapper were removed
/// when `config show` stopped rendering the `SOURCE` column.
#[derive(Debug)]
pub struct EffectiveConfig {
    pub host: Option<String>,
    pub base_url: Option<String>,
    pub api_key_file: Option<PathBuf>,
    pub insecure: bool,
    pub json: bool,
    /// `UNIFI_PROTECT_LOG` and `RUST_LOG` are *not* folded in here —
    /// their `env_logger` filter syntax cannot be reduced to a single
    /// `LogLevel` variant. They may still override this value at the
    /// live logger.
    pub log_level: LogLevel,
}

/// Merge flags + env + file into an [`EffectiveConfig`].
///
/// API-key resolution is intentionally **not** done here; the API key
/// has its own multi-source resolver in [`crate::api_key`] that takes a
/// [`crate::api_key::Sources`] built from the same inputs.
///
/// # Errors
/// - [`ConfigError::BadEnvBool`] — a boolean env var
///   (`UNIFI_PROTECT_INSECURE` / `UNIFI_PROTECT_JSON`) is set to a
///   non-empty value that isn't a recognised boolish token.
/// - [`ConfigError::HostAndBaseUrl`] — after merging across all
///   sources, both `host` and `base_url` ended up set. Caught here
///   (not just in [`ConfigFile::validate`]) because the file might
///   have `host` while the flag carries `base_url`, etc.
pub fn resolve<E>(
    flags: &Flags,
    file: Option<&LoadedConfig>,
    env: &E,
) -> Result<EffectiveConfig, ConfigError>
where
    E: Fn(&str) -> Option<String> + ?Sized,
{
    let cf = file.map(|lc| &lc.file);
    let host = resolve_string(
        "UNIFI_PROTECT_HOST",
        flags.host.as_deref(),
        env,
        cf.and_then(|c| c.host.as_deref()),
    );
    let base_url = resolve_string(
        "UNIFI_PROTECT_BASE_URL",
        flags.base_url.as_deref(),
        env,
        cf.and_then(|c| c.base_url.as_deref()),
    );
    if host.is_some() && base_url.is_some() {
        return Err(ConfigError::HostAndBaseUrl);
    }
    let api_key_file = resolve_path_field(
        crate::api_key::ENV_KEY_FILE,
        flags.api_key_file.as_deref(),
        env,
        cf.and_then(|c| c.api_key_file.as_deref()),
    );
    let insecure = resolve_bool(
        "UNIFI_PROTECT_INSECURE",
        flags.insecure,
        env,
        cf.and_then(|c| c.insecure),
        false,
    )?;
    let json = resolve_bool(
        "UNIFI_PROTECT_JSON",
        flags.json,
        env,
        cf.and_then(|c| c.json),
        false,
    )?;
    let log_level = flags
        .log_level
        .or_else(|| cf.and_then(|c| c.log_level))
        .unwrap_or(LogLevel::Warn);

    Ok(EffectiveConfig {
        host,
        base_url,
        api_key_file,
        insecure,
        json,
        log_level,
    })
}

fn resolve_string<E>(
    env_name: &'static str,
    flag: Option<&str>,
    env: &E,
    file: Option<&str>,
) -> Option<String>
where
    E: Fn(&str) -> Option<String> + ?Sized,
{
    if let Some(v) = flag {
        return Some(v.to_owned());
    }
    // Trim before the emptiness check so `UNIFI_PROTECT_HOST="   "`
    // doesn't slip through as a valid host. Same rule the API-key env
    // path applies. The trimmed value is what we store, so trailing
    // newlines from `set -a; source .env.local` or similar don't make
    // it into URLs.
    if let Some(v) = env(env_name) {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }
    // Trim file values too. `ConfigFile::validate` rejects
    // whitespace-only strings, so anything here is non-empty after
    // trimming.
    file.map(|v| v.trim().to_owned())
}

fn resolve_path_field<E>(
    env_name: &'static str,
    flag: Option<&Path>,
    env: &E,
    file: Option<&Path>,
) -> Option<PathBuf>
where
    E: Fn(&str) -> Option<String> + ?Sized,
{
    if let Some(p) = flag {
        return Some(p.to_path_buf());
    }
    // Empty/whitespace env falls through. Tilde is *not* expanded on
    // env paths; `api_key::resolve` doesn't expand them either, so
    // what we return is what the runtime would open.
    if let Some(raw) = env(env_name) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    file.map(Path::to_path_buf)
}

fn resolve_bool<E>(
    env_name: &'static str,
    flag: Option<bool>,
    env: &E,
    file: Option<bool>,
    default: bool,
) -> Result<bool, ConfigError>
where
    E: Fn(&str) -> Option<String> + ?Sized,
{
    if let Some(v) = flag {
        return Ok(v);
    }
    // Empty/whitespace env falls through. A non-empty but
    // unrecognised value is a hard error -- silently falling through
    // would mask misconfiguration like `UNIFI_PROTECT_JSON=tru`. TOML
    // is similarly strict (a non-bool there fails parsing).
    if let Some(raw) = env(env_name) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return parse_boolish(trimmed).ok_or_else(|| ConfigError::BadEnvBool {
                name: env_name,
                value: raw.clone(),
            });
        }
    }
    Ok(file.unwrap_or(default))
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
    fn validate_rejects_empty_or_whitespace_strings() {
        for (cf, expected) in [
            (
                ConfigFile {
                    host: Some(String::new()),
                    ..Default::default()
                },
                "host",
            ),
            (
                ConfigFile {
                    host: Some("   ".into()),
                    ..Default::default()
                },
                "host",
            ),
            (
                ConfigFile {
                    base_url: Some("  \t".into()),
                    ..Default::default()
                },
                "base_url",
            ),
            (
                ConfigFile {
                    api_key_file: Some(PathBuf::new()),
                    ..Default::default()
                },
                "api_key_file",
            ),
            (
                ConfigFile {
                    api_key_file: Some(PathBuf::from("   ")),
                    ..Default::default()
                },
                "api_key_file",
            ),
        ] {
            match cf.validate() {
                Err(ConfigError::EmptyValue { field }) => assert_eq!(field, expected),
                other => panic!("expected EmptyValue({expected}), got {other:?}"),
            }
        }
    }

    fn loaded(file: ConfigFile) -> LoadedConfig {
        LoadedConfig {
            file,
            path: PathBuf::from("/cfg"),
            source: FileDiscoverySource::XdgDefault,
        }
    }

    #[test]
    fn resolve_flag_wins_over_env_and_file() {
        let flags = Flags {
            host: Some("from-flag".into()),
            ..Default::default()
        };
        let lc = loaded(ConfigFile {
            host: Some("from-file".into()),
            ..Default::default()
        });
        let env = env_from([("UNIFI_PROTECT_HOST", "from-env")]);
        let r = resolve(&flags, Some(&lc), &env).expect("resolve");
        assert_eq!(r.host.as_deref(), Some("from-flag"));
    }

    #[test]
    fn resolve_env_wins_over_file_when_no_flag() {
        let lc = loaded(ConfigFile {
            host: Some("from-file".into()),
            ..Default::default()
        });
        let env = env_from([("UNIFI_PROTECT_HOST", "from-env")]);
        let r = resolve(&Flags::default(), Some(&lc), &env).expect("resolve");
        assert_eq!(r.host.as_deref(), Some("from-env"));
    }

    #[test]
    fn resolve_file_wins_when_no_flag_or_env() {
        let lc = loaded(ConfigFile {
            host: Some("from-file".into()),
            ..Default::default()
        });
        let r = resolve(&Flags::default(), Some(&lc), &empty_env()).expect("resolve");
        assert_eq!(r.host.as_deref(), Some("from-file"));
    }

    #[test]
    fn resolve_bool_default_when_unset() {
        let r = resolve(&Flags::default(), None, &empty_env()).expect("resolve");
        assert!(!r.insecure);
        assert!(!r.json);
    }

    #[test]
    fn resolve_bool_rejects_invalid_env_value() {
        // Regression: a non-empty but unparseable env bool used to fall
        // through silently to file/default, masking a typo like
        // `UNIFI_PROTECT_JSON=tru`. It must error instead.
        let env = env_from([("UNIFI_PROTECT_JSON", "tru")]);
        let err = resolve(&Flags::default(), None, &env).expect_err("should error");
        assert!(
            matches!(err, ConfigError::BadEnvBool { name: "UNIFI_PROTECT_JSON", ref value } if value == "tru"),
            "unexpected error: {err:?}",
        );
    }

    #[test]
    fn resolve_bool_empty_env_falls_through() {
        // `UNIFI_PROTECT_JSON=""` (or whitespace) is still "unset", not
        // an error, matching `resolve_string` / `api_key::resolve`.
        let env = env_from([("UNIFI_PROTECT_JSON", "   ")]);
        let r = resolve(&Flags::default(), None, &env).expect("resolve");
        assert!(!r.json);
    }

    #[test]
    fn resolve_empty_env_string_falls_through_to_file() {
        let lc = loaded(ConfigFile {
            host: Some("from-file".into()),
            ..Default::default()
        });
        let env = env_from([("UNIFI_PROTECT_HOST", "")]);
        let r = resolve(&Flags::default(), Some(&lc), &env).expect("resolve");
        assert_eq!(r.host.as_deref(), Some("from-file"));
    }

    #[test]
    fn resolve_whitespace_only_env_string_falls_through_to_file() {
        let lc = loaded(ConfigFile {
            host: Some("from-file".into()),
            ..Default::default()
        });
        let env = env_from([("UNIFI_PROTECT_HOST", "   \t\n")]);
        let r = resolve(&Flags::default(), Some(&lc), &env).expect("resolve");
        assert_eq!(r.host.as_deref(), Some("from-file"));
    }

    #[test]
    fn resolve_env_string_is_trimmed_before_storing() {
        let env = env_from([("UNIFI_PROTECT_HOST", "  nvr.local  \n")]);
        let r = resolve(&Flags::default(), None, &env).expect("resolve");
        assert_eq!(r.host.as_deref(), Some("nvr.local"));
    }

    #[test]
    fn resolve_rejects_cross_source_host_plus_base_url() {
        // Regression: the file says `host = "..."`, the flag carries
        // `--base-url`. `ConfigFile::validate` doesn't see flags, so
        // the cross-source check has to live in `resolve`.
        let lc = loaded(ConfigFile {
            host: Some("from-file".into()),
            ..Default::default()
        });
        let flags = Flags {
            base_url: Some("https://from-flag/".into()),
            ..Default::default()
        };
        let err = resolve(&flags, Some(&lc), &empty_env()).expect_err("rejects");
        assert!(matches!(err, ConfigError::HostAndBaseUrl), "got {err:?}");
    }

    #[test]
    fn resolve_path_empty_env_config_file_falls_through_to_xdg() {
        // Regression: `UNIFI_PROTECT_CONFIG_FILE=""` (or whitespace)
        // used to resolve to an empty PathBuf and then hard-error
        // inside `load`. It should fall through to the XDG default
        // just like the other empty-env paths.
        let env = env_from([(ENV_CONFIG_FILE, "   ")]);
        let (_path, source) = resolve_path(None, &env).expect("resolves");
        assert!(matches!(source, FileDiscoverySource::XdgDefault));
    }
}

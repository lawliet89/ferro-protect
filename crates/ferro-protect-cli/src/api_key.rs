//! Resolve the Protect API key from one of five sources, in priority order.
//!
//! 1. `--api-key-file <PATH>` flag (path only -- never a raw key on the
//!    command line where it would land in shell history and ps output).
//! 2. `UNIFI_PROTECT_API_KEY_FILE` env var (path).
//! 3. `UNIFI_PROTECT_API_KEY` env var (raw key).
//! 4. Config file `api_key_file` field (path).
//! 5. Config file `api_key` field (raw key).
//!
//! The resolver is pure with respect to its `env` callback so tests can
//! supply their own environment instead of mutating the process's. The
//! config-file sources are passed in via the [`Sources`] struct rather
//! than read directly, so this module stays independent of the config
//! loader.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

pub const ENV_KEY_FILE: &str = "UNIFI_PROTECT_API_KEY_FILE";
pub const ENV_KEY: &str = "UNIFI_PROTECT_API_KEY";

/// Where the resolved key came from. Returned alongside the
/// [`SecretString`] so `config show` can attribute the `api_key` row
/// without re-running the resolver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiKeySource {
    /// `--api-key-file <PATH>` flag.
    Flag,
    /// `UNIFI_PROTECT_API_KEY_FILE` env var (path).
    EnvFile,
    /// `UNIFI_PROTECT_API_KEY` env var (raw key).
    EnvRaw,
    /// Config file `api_key_file` field (path).
    ConfigFile,
    /// Config file `api_key` field (raw key).
    ConfigRaw,
}

impl ApiKeySource {
    #[must_use]
    pub const fn as_user_label(self) -> &'static str {
        match self {
            Self::Flag => "--api-key-file flag",
            Self::EnvFile => "env: UNIFI_PROTECT_API_KEY_FILE",
            Self::EnvRaw => "env: UNIFI_PROTECT_API_KEY",
            Self::ConfigFile => "config file: api_key_file",
            Self::ConfigRaw => "config file: api_key",
        }
    }
}

/// Inputs to [`resolve`], grouped so callers don't have to thread four
/// positional args.
///
/// The two `config_*` fields come from the merged
/// [`crate::config::ConfigFile`] when one is loaded; pass `None` for
/// both when no config file exists.
#[derive(Debug, Default)]
pub struct Sources<'a> {
    pub flag_file: Option<&'a Path>,
    pub config_file: Option<&'a Path>,
    pub config_raw: Option<&'a SecretString>,
}

#[derive(Debug, Error)]
pub enum ApiKeyError {
    #[error(
        "no API key provided. Set one of:\n  \
         * --api-key-file <PATH>\n  \
         * UNIFI_PROTECT_API_KEY_FILE=<PATH> (path to a file containing the key)\n  \
         * UNIFI_PROTECT_API_KEY=<KEY> (raw key in env)\n  \
         * api_key_file = \"<PATH>\" in the config file\n  \
         * api_key = \"<KEY>\" in the config file (discouraged; use api_key_file)"
    )]
    NotProvided,

    #[error("reading API key from {path}: {source}")]
    ReadFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("API key file {} is empty", .0.display())]
    EmptyFile(PathBuf),
}

/// Resolve the API key.
///
/// `env` is a callback for environment lookup; production callers pass
/// `|k| std::env::var(k).ok()`. Tests pass their own closure so the
/// process environment stays untouched.
///
/// `warnings` receives any human-readable warnings the resolver wants to
/// surface (e.g. lax file permissions). Production callers pass
/// `io::stderr().lock()`; tests can pass a `Vec<u8>`.
///
/// Returns the key plus the [`ApiKeySource`] that supplied it, so
/// callers can attribute the key's origin in `config show` without
/// re-running the resolver.
///
/// # Errors
///
/// - [`ApiKeyError::NotProvided`] if no source supplies a key.
/// - [`ApiKeyError::ReadFailed`] if a referenced file cannot be read.
/// - [`ApiKeyError::EmptyFile`] if a referenced file is empty after
///   trimming trailing whitespace.
pub fn resolve<E, W>(
    sources: &Sources<'_>,
    env: &E,
    warnings: &mut W,
) -> Result<(SecretString, ApiKeySource), ApiKeyError>
where
    E: Fn(&str) -> Option<String> + ?Sized,
    W: Write,
{
    if let Some(path) = sources.flag_file {
        return Ok((read_key_file(path, warnings)?, ApiKeySource::Flag));
    }
    if let Some(path) = env(ENV_KEY_FILE) {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return Ok((
                read_key_file(Path::new(trimmed), warnings)?,
                ApiKeySource::EnvFile,
            ));
        }
        // Empty env var falls through (same rule the raw-key branch
        // below applies, and the same rule `config::resolve_string`
        // uses for the host/base_url/etc. env vars). Without this,
        // `UNIFI_PROTECT_API_KEY_FILE=""` would try to read an empty
        // path and error noisily.
    }
    if let Some(raw) = env(ENV_KEY) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Ok((
                SecretString::from(trimmed.to_string()),
                ApiKeySource::EnvRaw,
            ));
        }
        // Empty env falls through to the lower-priority sources.
    }
    if let Some(path) = sources.config_file {
        return Ok((read_key_file(path, warnings)?, ApiKeySource::ConfigFile));
    }
    if let Some(secret) = sources.config_raw {
        let exposed = secret.expose_secret();
        let trimmed = exposed.trim();
        if !trimmed.is_empty() {
            return Ok((
                SecretString::from(trimmed.to_string()),
                ApiKeySource::ConfigRaw,
            ));
        }
    }
    Err(ApiKeyError::NotProvided)
}

fn read_key_file<W: Write>(path: &Path, warnings: &mut W) -> Result<SecretString, ApiKeyError> {
    let raw = std::fs::read_to_string(path).map_err(|source| ApiKeyError::ReadFailed {
        path: path.to_path_buf(),
        source,
    })?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ApiKeyError::EmptyFile(path.to_path_buf()));
    }
    warn_if_world_readable(path, warnings);
    Ok(SecretString::from(trimmed.to_string()))
}

#[cfg(unix)]
fn warn_if_world_readable<W: Write>(path: &Path, warnings: &mut W) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    let mode = meta.permissions().mode();
    if mode & 0o077 != 0 {
        let _ = writeln!(
            warnings,
            "warning: API key file {} has lax permissions ({:o}); recommend `chmod 600 {}`",
            path.display(),
            mode & 0o777,
            path.display(),
        );
    }
}

#[cfg(not(unix))]
fn warn_if_world_readable<W: Write>(_path: &Path, _warnings: &mut W) {
    // No-op outside Unix; mode bits aren't a meaningful concept on Windows.
}

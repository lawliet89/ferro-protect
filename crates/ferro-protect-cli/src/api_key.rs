//! Resolve the Protect API key from one of three sources, in priority order.
//!
//! 1. `--api-key-file <PATH>` flag (path only -- never a raw key on the
//!    command line where it would land in shell history and ps output).
//! 2. `UNIFI_PROTECT_API_KEY_FILE` env var (path).
//! 3. `UNIFI_PROTECT_API_KEY` env var (raw key).
//!
//! The resolver is pure with respect to its `env` callback so tests can
//! supply their own environment instead of mutating the process's.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use secrecy::SecretString;
use thiserror::Error;

pub const ENV_KEY_FILE: &str = "UNIFI_PROTECT_API_KEY_FILE";
pub const ENV_KEY: &str = "UNIFI_PROTECT_API_KEY";

#[derive(Debug, Error)]
pub enum ApiKeyError {
    #[error(
        "no API key provided. Set one of:\n  \
         * --api-key-file <PATH>\n  \
         * UNIFI_PROTECT_API_KEY_FILE=<PATH> (path to a file containing the key)\n  \
         * UNIFI_PROTECT_API_KEY=<KEY> (raw key in env)"
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
/// # Errors
///
/// - [`ApiKeyError::NotProvided`] if no source supplies a key.
/// - [`ApiKeyError::ReadFailed`] if a referenced file cannot be read.
/// - [`ApiKeyError::EmptyFile`] if a referenced file is empty after
///   trimming trailing whitespace.
pub fn resolve<E, W>(
    file_flag: Option<&Path>,
    env: &E,
    warnings: &mut W,
) -> Result<SecretString, ApiKeyError>
where
    E: Fn(&str) -> Option<String> + ?Sized,
    W: Write,
{
    if let Some(path) = file_flag {
        return read_key_file(path, warnings);
    }
    if let Some(path) = env(ENV_KEY_FILE) {
        return read_key_file(Path::new(&path), warnings);
    }
    if let Some(raw) = env(ENV_KEY) {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(ApiKeyError::NotProvided);
        }
        return Ok(SecretString::from(trimmed.to_string()));
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

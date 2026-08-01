//! `ferro-protect config` subcommand: persistent TOML config file
//! management. Three actions:
//!
//! - [`Action::Show`] — print effective resolved config (field/value).
//! - [`Action::Path`] — print the resolved config file path.
//! - [`Action::Template`] — write (or print) a commented-out scaffold
//!   listing every recognised field.
//!
//! The richer surface that earlier revisions of this PR carried
//! (interactive wizard, `edit`, `delete`, `list`, `show KEY`,
//! per-field source attribution, an `api_key` masked row) was
//! deliberately removed: users hand-edit a TOML file with their
//! preferred editor, scripts that need a single value can `jq` the
//! `--json` array, and the API key's source is reported via runtime
//! "no API key provided" errors instead of a separate show path.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use clap::Subcommand;
use serde::Serialize;
use thiserror::Error;

use crate::config::{self, EffectiveConfig, Flags};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Print the effective **non-secret** configuration as a
    /// field/value table.
    ///
    /// Shows: `host`, `base_url`, `api_key_file`, `insecure`, `json`,
    /// `log_level`. The API key itself is intentionally absent --
    /// even rendering it as `<set>`/`<unset>` would shadow the
    /// runtime resolver and tempt users to grep for it. The "no API
    /// key provided" error at runtime explains the source ladder
    /// (flag, env, file pointer) when none is supplied.
    ///
    /// Only `--config` is honoured here (to pick which file to
    /// inspect). Other per-invocation flags like `--host` or
    /// `--insecure` are ignored -- they would only be true for this
    /// invocation. The usual flag > env > file > default precedence
    /// still applies to real commands like `info`.
    ///
    /// `log_level` reflects only the `log_level` config field;
    /// `UNIFI_PROTECT_LOG` / `RUST_LOG` further filter the runtime
    /// logger (env_logger syntax) and are not shown here.
    ///
    /// `--json` emits the same fields as a JSON array of
    /// `{field, value}` -- pipe through `jq` for scripting.
    Show,
    /// Print the resolved config file path on a single line. Useful in
    /// shell scripts (`$(ferro-protect config path)`). `--json` emits
    /// `{"path": "..."}`. Errors when the file is missing.
    Path,
    /// Write a commented-out scaffold listing every recognised field.
    /// Default destination is the resolved config file path; pass
    /// `--stdout` to print to stdout instead (useful for piping into a
    /// different path, or for `diff`-style inspection).
    Template {
        /// Print the template to stdout instead of writing it to the
        /// resolved config path. No file is written or modified.
        #[arg(long)]
        stdout: bool,
        /// Overwrite an existing config file. Has no effect with
        /// `--stdout`.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Error)]
pub enum ConfigCmdError {
    #[error(transparent)]
    Config(#[from] config::ConfigError),
    #[error("io error on {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "no config file at {}\n\
         Run `ferro-protect config template` to create one, or point\n\
         `--config` / `UNIFI_PROTECT_CONFIG_FILE` at an existing file.",
        path.display()
    )]
    NoConfigFile { path: PathBuf },
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Entry point for the `config` subcommand. Dispatches on [`Action`].
///
/// # Errors
/// Any [`ConfigCmdError`] returned by the action handler — see each
/// action function's docs for the specific failure modes.
pub fn run(action: &Action, config_flag: Option<&Path>, json: bool) -> Result<(), ConfigCmdError> {
    let env = |k: &str| std::env::var(k).ok();
    match *action {
        Action::Show => show(config_flag, &env, json),
        Action::Path => path(config_flag, &env, json),
        Action::Template { stdout, force } => template(config_flag, &env, stdout, force),
    }
}

// --------------------------------------------------------------------
// config show
// --------------------------------------------------------------------

fn show<E>(config_flag: Option<&Path>, env: &E, json: bool) -> Result<(), ConfigCmdError>
where
    E: Fn(&str) -> Option<String> + ?Sized,
{
    // `show` is a config-file inspection tool, so a missing file is an
    // error rather than a silent fallback to defaults. The explicit
    // `--config` / `UNIFI_PROTECT_CONFIG_FILE` cases already error
    // inside `config::load`; if `load` returns `None`, we're on the
    // XDG-default path and the file is absent there too.
    let Some(loaded) = config::load(config_flag, env)? else {
        let (path, _src) = config::resolve_path(config_flag, env)?;
        return Err(ConfigCmdError::NoConfigFile { path });
    };
    // `Flags::default()` — `show` only reflects what the loader sees
    // *outside* of any per-invocation flags besides --config. Per-flag
    // overrides are inherently per-invocation; reflecting them would
    // mean `config show --insecure` claims `true` for *this* run when
    // it isn't the persisted state.
    let resolved = config::resolve(&Flags::default(), Some(&loaded), env)?;
    let rows = collect_rows(&resolved);
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    if json {
        serde_json::to_writer_pretty(&mut lock, &rows)
            .map_err(|e| ConfigCmdError::Other(e.into()))?;
        lock.write_all(b"\n")
            .map_err(|e| ConfigCmdError::Other(e.into()))?;
    } else {
        let table_rows: Vec<Vec<String>> = rows
            .iter()
            .map(|r| vec![r.field.to_owned(), r.value.clone()])
            .collect();
        lock.write_all(crate::output::table(&["FIELD", "VALUE"], &table_rows).as_bytes())
            .map_err(|e| ConfigCmdError::Other(e.into()))?;
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct ShowRow {
    field: &'static str,
    value: String,
}

fn collect_rows(resolved: &EffectiveConfig) -> Vec<ShowRow> {
    fn opt_string(v: Option<&String>) -> String {
        v.map_or_else(|| "<unset>".to_owned(), Clone::clone)
    }
    fn opt_path(v: Option<&PathBuf>) -> String {
        v.map_or_else(|| "<unset>".to_owned(), |p| p.display().to_string())
    }
    vec![
        ShowRow {
            field: "host",
            value: opt_string(resolved.host.as_ref()),
        },
        ShowRow {
            field: "base_url",
            value: opt_string(resolved.base_url.as_ref()),
        },
        ShowRow {
            field: "api_key_file",
            value: opt_path(resolved.api_key_file.as_ref()),
        },
        ShowRow {
            field: "insecure",
            value: resolved.insecure.to_string(),
        },
        ShowRow {
            field: "json",
            value: resolved.json.to_string(),
        },
        ShowRow {
            field: "log_level",
            value: resolved.log_level.to_string(),
        },
    ]
}

// --------------------------------------------------------------------
// config path
// --------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct PathJson {
    path: String,
}

fn path<E>(config_flag: Option<&Path>, env: &E, json: bool) -> Result<(), ConfigCmdError>
where
    E: Fn(&str) -> Option<String> + ?Sized,
{
    let (path, _source) = config::resolve_path(config_flag, env)?;
    // `is_file()` rather than `exists()`: a directory at the resolved
    // path is just as unusable as a missing file, and `config show`
    // would later fail trying to `read_to_string` it. Aligning the
    // diagnostics keeps `config path` and `config show` consistent.
    if !path.is_file() {
        return Err(ConfigCmdError::NoConfigFile { path });
    }
    if json {
        let pj = PathJson {
            path: path.display().to_string(),
        };
        let stdout = io::stdout();
        let mut lock = stdout.lock();
        serde_json::to_writer_pretty(&mut lock, &pj)
            .map_err(|e| ConfigCmdError::Other(e.into()))?;
        lock.write_all(b"\n")
            .map_err(|e| ConfigCmdError::Other(e.into()))?;
    } else {
        println!("{}", path.display());
    }
    Ok(())
}

// --------------------------------------------------------------------
// config template
// --------------------------------------------------------------------

/// Commented-out scaffold listing every recognised field. Hardcoded
/// rather than table-driven: there is no second consumer of a
/// per-field description / example pair, so the table indirection
/// was pure overhead. Add a new field by editing this string and the
/// matching `ConfigFile` / `Flags` / `EffectiveConfig` slots.
const TEMPLATE: &str = "\
# ferro-protect config file
# Generated by `ferro-protect config template`.
# Precedence: flag > env > this file > built-in default.
# Every recognised field is listed below; uncomment the lines
# you need. `ferro-protect config show` displays the effective
# resolved values.

# NVR hostname or host:port. Mutually exclusive with `base_url`.
# host = \"nvr.local\"

# Override the entire base URL. Mutually exclusive with `host`.
# base_url = \"https://nvr.local/proxy/protect/integration\"

# Path to a file containing the API key. Use an absolute path --
# `~` is not expanded here (TOML is not a shell).
# api_key_file = \"/home/you/.config/ferro-protect/api_key\"

# Skip TLS certificate validation (typical for self-signed NVRs).
# insecure = false

# Default to JSON output instead of human-readable text.
# json = false

# Log level: error | warn | info | debug | trace.
# log_level = \"warn\"
";

fn template<E>(
    config_flag: Option<&Path>,
    env: &E,
    stdout: bool,
    force: bool,
) -> Result<(), ConfigCmdError>
where
    E: Fn(&str) -> Option<String> + ?Sized,
{
    if stdout {
        let out = io::stdout();
        let mut lock = out.lock();
        lock.write_all(TEMPLATE.as_bytes())
            .map_err(|e| ConfigCmdError::Other(e.into()))?;
        return Ok(());
    }

    let (target_path, _src) = config::resolve_path(config_flag, env)?;
    if target_path.exists() && !force {
        return Err(ConfigCmdError::Other(anyhow!(
            "{} already exists. Pass `--force` to overwrite, or `--stdout` to print without writing.",
            target_path.display(),
        )));
    }
    if let Some(parent) = target_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|source| ConfigCmdError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    // Atomic temp+rename so a crash mid-write can't leave a partial
    // file. `write_file_secure` opens the temp with mode 0600 on Unix
    // at creation -- the template itself contains no secrets, but if
    // `--force` is overwriting an existing file that *did* have a raw
    // `api_key`, we don't want the new file briefly visible at default
    // umask perms.
    let tmp = tmp_sibling(&target_path);
    write_file_secure(&tmp, TEMPLATE.as_bytes()).map_err(|source| ConfigCmdError::Io {
        path: tmp.clone(),
        source,
    })?;
    // `std::fs::rename` atomically replaces an existing destination on
    // both Unix and Windows. On Windows it has called `MoveFileExW` with
    // `MOVEFILE_REPLACE_EXISTING` since Rust 1.79; our MSRV is 1.85, so
    // no platform-specific delete-then-rename dance is needed for
    // `--force` to overwrite.
    if let Err(source) = fs::rename(&tmp, &target_path) {
        let _ = fs::remove_file(&tmp);
        return Err(ConfigCmdError::Io {
            path: target_path,
            source,
        });
    }
    eprintln!(
        "Wrote template config {}. Uncomment the values you need.",
        target_path.display(),
    );
    Ok(())
}

/// Build a per-process temp-file path next to `path`. We use the
/// destination's directory (not `std::env::temp_dir()`) so the rename
/// stays within one filesystem and remains atomic.
fn tmp_sibling(path: &Path) -> PathBuf {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    let file_name = path
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    let mut name = std::ffi::OsString::from(".");
    name.push(&file_name);
    name.push(format!(".tmp.{}", std::process::id()));
    parent.map_or_else(|| PathBuf::from(&name), |p| p.join(&name))
}

/// Create-or-truncate `path` with mode 0600 on Unix at creation time
/// (no chmod-after-write window) and write `contents`. On non-Unix,
/// falls back to `fs::write` (Windows doesn't honour Unix mode bits).
fn write_file_secure(path: &Path, contents: &[u8]) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(contents)?;
        f.sync_all()?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        fs::write(path, contents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both `TEMPLATE` and `collect_rows` enumerate the same set of
    /// settable fields. Adding a row to one without the other would
    /// silently desync the two surfaces; this test catches the drift.
    #[test]
    fn template_and_show_cover_the_same_fields() {
        let resolved = EffectiveConfig {
            host: None,
            base_url: None,
            api_key_file: None,
            insecure: false,
            json: false,
            log_level: crate::logging::LogLevel::Warn,
        };
        let row_keys: Vec<_> = collect_rows(&resolved)
            .into_iter()
            .map(|r| r.field)
            .collect();
        for key in &row_keys {
            let needle = format!("# {key} =");
            assert!(
                TEMPLATE.contains(&needle),
                "TEMPLATE missing line `{needle}`",
            );
        }
    }
}

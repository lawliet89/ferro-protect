//! `ferro-protect config` subcommand: persistent TOML config file
//! management. Three actions:
//!
//! - [`Action::Show`] — print effective config + source attribution.
//! - [`Action::Path`] — print the resolved config file path.
//! - [`Action::Template`] — write (or print) a commented-out scaffold
//!   listing every recognised field.
//!
//! The richer surface that earlier revisions of this PR carried
//! (interactive wizard, `edit`, `delete`, `list`) was deliberately
//! removed: users hand-edit a TOML file with their preferred editor,
//! and `template` gives them the schema to start from. This kept the
//! secret-handling surface (hidden-input pasting, key-file writing,
//! per-field parsing, backup logic) out of the CLI.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use clap::Subcommand;
use serde::Serialize;
use thiserror::Error;

use crate::api_key::{self, ApiKeySource};
use crate::config::{
    self, FIELDS, FieldSource, Flags, LoadedConfig, Resolved, ResolvedConfig, is_known_key,
    known_keys_joined,
};

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Print the effective resolved configuration, with each value
    /// annotated by its source (env / config file / default).
    ///
    /// Per-invocation global flags (`--host`, `--insecure`, …) are
    /// **not** reflected -- they would show `source = flag` for the
    /// current invocation only, which is misleading as "the effective
    /// config". `--config` is the only global flag honoured here (it
    /// picks *which* file to inspect). The full
    /// flag-wins-over-env-wins-over-file precedence is still applied
    /// when the binary runs a real command like `info` or
    /// `cameras list`.
    ///
    /// Note: `log_level` reflects only the `log_level` config field.
    /// Runtime logging is *additionally* filtered by `UNIFI_PROTECT_LOG`
    /// and `RUST_LOG` (env_logger filter syntax, not enum values), which
    /// are independent of this field and not surfaced here.
    ///
    /// Pass a single KEY to print only that field's value (scriptable).
    /// `--json` switches to a structured `{value, source}` form.
    Show {
        /// Print only this single field's value. Without a key, the
        /// full table is printed.
        key: Option<String>,
    },
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
    #[error("unknown config field `{key}`\nvalid fields: {valid}")]
    UnknownKey { key: String, valid: String },
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

fn unknown_key(key: &str) -> ConfigCmdError {
    ConfigCmdError::UnknownKey {
        key: key.to_owned(),
        valid: known_keys_joined(),
    }
}

/// Entry point for the `config` subcommand. Dispatches on [`Action`].
///
/// # Errors
/// Any [`ConfigCmdError`] returned by the action handler — see each
/// action function's docs for the specific failure modes.
pub fn run(action: Action, config_flag: Option<&Path>, json: bool) -> Result<(), ConfigCmdError> {
    let env = |k: &str| std::env::var(k).ok();
    match action {
        Action::Show { key } => show(config_flag, &env, key.as_deref(), json),
        Action::Path => path(config_flag, &env, json),
        Action::Template { stdout, force } => template(config_flag, &env, stdout, force),
    }
}

// --------------------------------------------------------------------
// config show
// --------------------------------------------------------------------

fn show<E>(
    config_flag: Option<&Path>,
    env: &E,
    key: Option<&str>,
    json: bool,
) -> Result<(), ConfigCmdError>
where
    E: Fn(&str) -> Option<String> + ?Sized,
{
    // Validate the user-supplied key (input error) before touching the
    // file (state error) -- input errors shouldn't depend on file state.
    if let Some(k) = key
        && !is_known_key(k)
    {
        return Err(unknown_key(k));
    }
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
    // overrides are an inherently per-invocation thing; reflecting them
    // would mean `config show --insecure` claims insecure=Flag, which
    // is true for that invocation only and misleading as "the
    // effective config".
    let resolved = config::resolve(&Flags::default(), Some(&loaded), env);
    // `flag_file: None` -- `config show` deliberately ignores
    // per-invocation flags besides `--config`. We never want
    // `--config` (a *config* file path) to be mistaken for
    // `--api-key-file`.
    let api_key = resolve_api_key_source_only(None, Some(&loaded), env);

    key.map_or_else(
        || show_all(&resolved, api_key, json),
        |k| show_one(&resolved, api_key, k, json),
    )
}

/// Dry-run of `api_key::resolve`: identifies which source *would*
/// supply the key, without actually reading the file. We deliberately
/// don't read the key file in `config show` — the value is masked
/// anyway, and reading it would leak the warning sidechannel
/// (lax-permissions warning) into normal show output.
fn resolve_api_key_source_only<E>(
    flag_file: Option<&Path>,
    file: Option<&LoadedConfig>,
    env: &E,
) -> Option<ApiKeySource>
where
    E: Fn(&str) -> Option<String> + ?Sized,
{
    if flag_file.is_some() {
        return Some(ApiKeySource::Flag);
    }
    // Mirror the empty-env-falls-through rule from `api_key::resolve`
    // (and `config::resolve_string`): `UNIFI_PROTECT_API_KEY_FILE=""`
    // is treated as "not set" so we don't falsely report `<set>` from
    // an env var that would otherwise blow up at runtime.
    if let Some(path) = env(api_key::ENV_KEY_FILE)
        && !path.trim().is_empty()
    {
        return Some(ApiKeySource::EnvFile);
    }
    if let Some(raw) = env(api_key::ENV_KEY)
        && !raw.trim().is_empty()
    {
        return Some(ApiKeySource::EnvRaw);
    }
    let cf = file.map(|lc| &lc.file)?;
    if cf.api_key_file.is_some() {
        return Some(ApiKeySource::ConfigFile);
    }
    if cf.api_key.is_some() {
        return Some(ApiKeySource::ConfigRaw);
    }
    None
}

#[derive(Debug, Serialize)]
struct ShowRow {
    field: &'static str,
    value: String,
    source: String,
}

#[derive(Debug, Serialize)]
struct ShowSingle {
    value: String,
    source: String,
}

fn show_all(
    resolved: &ResolvedConfig,
    api_key: Option<ApiKeySource>,
    json: bool,
) -> Result<(), ConfigCmdError> {
    let rows = collect_rows(resolved, api_key);

    if json {
        let stdout = io::stdout();
        let mut lock = stdout.lock();
        serde_json::to_writer_pretty(&mut lock, &rows)
            .map_err(|e| ConfigCmdError::Other(e.into()))?;
        lock.write_all(b"\n")
            .map_err(|e| ConfigCmdError::Other(e.into()))?;
        return Ok(());
    }

    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| vec![r.field.to_owned(), r.value.clone(), r.source.clone()])
        .collect();
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    lock.write_all(crate::output::table(&["FIELD", "VALUE", "SOURCE"], &table_rows).as_bytes())
        .map_err(|e| ConfigCmdError::Other(e.into()))?;
    Ok(())
}

fn show_one(
    resolved: &ResolvedConfig,
    api_key: Option<ApiKeySource>,
    key: &str,
    json: bool,
) -> Result<(), ConfigCmdError> {
    if !is_known_key(key) {
        return Err(unknown_key(key));
    }
    // `collect_rows` always emits a row for every known key (with
    // `<unset>` when no source supplied a value), so `find` cannot
    // miss for a key that passed `is_known_key`. The expect message
    // is a guard against future drift between `FIELDS` and
    // `collect_rows`.
    let row = collect_rows(resolved, api_key)
        .into_iter()
        .find(|r| r.field == key)
        .expect("collect_rows emits a row for every FIELDS key");
    if json {
        let single = ShowSingle {
            value: row.value,
            source: row.source,
        };
        let stdout = io::stdout();
        let mut lock = stdout.lock();
        serde_json::to_writer_pretty(&mut lock, &single)
            .map_err(|e| ConfigCmdError::Other(e.into()))?;
        lock.write_all(b"\n")
            .map_err(|e| ConfigCmdError::Other(e.into()))?;
    } else {
        println!("{}", row.value);
    }
    Ok(())
}

fn collect_rows(resolved: &ResolvedConfig, api_key: Option<ApiKeySource>) -> Vec<ShowRow> {
    let cfg_path = resolved.config_file_path.as_deref();
    let mut rows = Vec::with_capacity(FIELDS.len());
    rows.push(ShowRow {
        field: "host",
        value: render_opt(resolved.host.as_ref(), String::clone),
        source: source_label(resolved.host.as_ref().map(|r| &r.source), cfg_path),
    });
    rows.push(ShowRow {
        field: "base_url",
        value: render_opt(resolved.base_url.as_ref(), String::clone),
        source: source_label(resolved.base_url.as_ref().map(|r| &r.source), cfg_path),
    });
    rows.push(ShowRow {
        field: "api_key_file",
        value: render_opt(resolved.api_key_file.as_ref(), |p| p.display().to_string()),
        source: source_label(resolved.api_key_file.as_ref().map(|r| &r.source), cfg_path),
    });
    rows.push(ShowRow {
        field: "api_key",
        value: api_key.map_or_else(|| "<unset>".to_owned(), |_| "<set>".to_owned()),
        source: api_key.map_or_else(|| "default".to_owned(), |s| s.as_user_label().to_owned()),
    });
    rows.push(ShowRow {
        field: "insecure",
        value: resolved.insecure.value.to_string(),
        source: source_label(Some(&resolved.insecure.source), cfg_path),
    });
    rows.push(ShowRow {
        field: "json",
        value: resolved.json.value.to_string(),
        source: source_label(Some(&resolved.json.source), cfg_path),
    });
    rows.push(ShowRow {
        field: "log_level",
        value: resolved.log_level.value.as_str().to_owned(),
        source: source_label(Some(&resolved.log_level.source), cfg_path),
    });
    rows
}

fn render_opt<T, F>(slot: Option<&Resolved<T>>, render: F) -> String
where
    F: FnOnce(&T) -> String,
{
    slot.map_or_else(|| "<unset>".to_owned(), |r| render(&r.value))
}

fn source_label(source: Option<&FieldSource>, file_path: Option<&Path>) -> String {
    match source {
        None | Some(FieldSource::Default) => "default".to_owned(),
        Some(FieldSource::Flag) => "flag".to_owned(),
        Some(FieldSource::Env(name)) => format!("env: {name}"),
        Some(FieldSource::ConfigFile) => file_path.map_or_else(
            || "config file".to_owned(),
            |p| format!("config file: {}", p.display()),
        ),
    }
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
    if !path.exists() {
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

/// `ferro-protect config template` body. Built from the canonical
/// [`FIELDS`] table so adding/renaming a field can't desync the
/// scaffold from the validators.
fn build_template() -> String {
    let mut out = String::from(
        "# ferro-protect config file\n\
         # Generated by `ferro-protect config template`.\n\
         # Precedence: flag > env > this file > built-in default.\n\
         # See `ferro-protect config --help` for the full list of fields.\n",
    );
    for f in FIELDS {
        out.push('\n');
        for line in f.description.lines() {
            out.push_str("# ");
            out.push_str(line);
            out.push('\n');
        }
        out.push_str("# ");
        out.push_str(f.key);
        out.push_str(" = ");
        out.push_str(f.example);
        out.push('\n');
    }
    out
}

fn template<E>(
    config_flag: Option<&Path>,
    env: &E,
    stdout: bool,
    force: bool,
) -> Result<(), ConfigCmdError>
where
    E: Fn(&str) -> Option<String> + ?Sized,
{
    let body = build_template();
    if stdout {
        let out = io::stdout();
        let mut lock = out.lock();
        lock.write_all(body.as_bytes())
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
    write_file_secure(&tmp, body.as_bytes()).map_err(|source| ConfigCmdError::Io {
        path: tmp.clone(),
        source,
    })?;
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

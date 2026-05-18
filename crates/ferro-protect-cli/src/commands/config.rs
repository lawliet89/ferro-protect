//! `ferro-protect config` subcommand: persistent TOML config file
//! management. Six actions:
//!
//! - [`Action::Init`] — interactive wizard, or `--template` for a
//!   commented-out scaffold (non-TTY safe).
//! - [`Action::Show`] — print effective config + source attribution.
//! - [`Action::Edit`] — set or unset a single field in the file.
//! - [`Action::Path`] — print the resolved config file path.
//! - [`Action::Delete`] — remove the file (confirmation or `--yes`).
//! - [`Action::List`] — list recognised field names (no file required).
//!
//! See `docs/TASK_config_file.md` for the design.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use clap::Subcommand;
use is_terminal::IsTerminal;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use thiserror::Error;
use toml_edit::{DocumentMut, value};

use crate::api_key::{self, ApiKeySource};
use crate::config::{
    self, ConfigFile, FIELDS, FieldSource, FieldType, Flags, LoadedConfig, Resolved,
    ResolvedConfig, field, is_known_key, known_keys_joined,
};
use crate::logging::LogLevel;

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Interactive wizard. Refuses to run when stdin is not a TTY,
    /// unless `--template` is passed.
    Init {
        /// Skip the "this will overwrite an existing file" confirmation
        /// (interactive mode) or allow overwrite (`--template` mode).
        #[arg(long)]
        force: bool,
        /// Skip prompts and write a commented-out template that lists
        /// every recognised field. Safe in non-TTY contexts.
        #[arg(long)]
        template: bool,
    },
    /// Print the effective resolved configuration, with each value
    /// annotated by its source (flag / env / config file / default).
    /// Pass a single KEY to print only that field's value (scriptable).
    /// `--json` switches to a structured `{value, source}` form.
    Show {
        /// Print only this single field's value. Without a key, the
        /// full table is printed.
        key: Option<String>,
    },
    /// Print the resolved config file path on a single line. Useful in
    /// shell scripts (`$(ferro-protect config path)`). `--json` emits
    /// `{"path": "..."}`. Errors when the file is missing, so an
    /// `exists` field would always be `true` and was dropped.
    Path,
    /// Set or unset a single field in the config file. Preserves
    /// comments and formatting via `toml_edit`. Refuses to set
    /// `api_key` from argv (would land in shell history / `ps`).
    /// If the file doesn't exist yet, creates it with a wizard-style
    /// header plus the edited field, and emits a stderr `note:`
    /// pointing at `config init` / `config init --template` for
    /// richer scaffolding.
    Edit {
        /// Field name. Use `ferro-protect config show` to see the
        /// recognized fields.
        key: String,
        /// New value. Mutually exclusive with `--unset`.
        value: Option<String>,
        /// Remove the field from the file.
        #[arg(long, conflicts_with = "value")]
        unset: bool,
    },
    /// Delete the resolved config file. Prompts for confirmation unless
    /// `--yes` is passed. Refuses to run in a non-TTY context without
    /// `--yes` (no way to confirm).
    Delete {
        /// Skip the confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Print the names of every recognised config field. Doesn't read
    /// (or require) a config file — answers "what *can* I set?", not
    /// "what *is* set". Useful for shell completion (`complete -W
    /// "$(ferro-protect config list)" ...`) and discovery.
    List {
        /// Two-column human output: `key <TAB> description`. Without
        /// `-v` and `--json`, prints one field per line, nothing else.
        #[arg(long, short = 'v')]
        verbose: bool,
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
    #[error("`{0}` is unset across all sources")]
    NoValue(&'static str),
    #[error(
        "refusing to set `api_key` from the command line.\n\
         The raw key would land in shell history, `ps`, and the parent\n\
         process's argv. Use one of:\n  \
         * `ferro-protect config init` (paste hidden, write to file)\n  \
         * `ferro-protect config edit api_key_file <PATH>` (point at an\n    \
           existing key file)\n  \
         * `UNIFI_PROTECT_API_KEY=<KEY>` env var"
    )]
    RawKeyOnArgv,
    #[error("`config edit` requires either a VALUE or --unset")]
    NeitherValueNorUnset,
    #[error("invalid value for `{field}`: `{value}`.\nExpected: {expected}")]
    InvalidValue {
        field: &'static str,
        value: String,
        expected: &'static str,
    },
    #[error(
        "`config edit {field}` would conflict with existing `{other}` in the file.\nUse `config edit {other} --unset` first if you want to switch."
    )]
    Conflict {
        field: &'static str,
        other: &'static str,
    },
    #[error(
        "interactive `config init` requires a TTY on stdin. Edit the file directly, or use `config edit KEY VALUE` for individual fields."
    )]
    NotATty,
    #[error(
        "no config file at {}\n\
         Run `ferro-protect config init` to create one, point `--config` /\n\
         `UNIFI_PROTECT_CONFIG_FILE` at an existing file, or use\n\
         `ferro-protect config edit KEY VALUE` which will create the file\n\
         on first use.",
        path.display()
    )]
    NoConfigFile { path: PathBuf },
    #[error("wizard cancelled")]
    Cancelled,
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
        Action::Edit { key, value, unset } => {
            edit(config_flag, &env, &key, value.as_deref(), unset)
        }
        Action::Init { force, template } => init(config_flag, &env, force, template),
        Action::Delete { yes } => delete(config_flag, &env, yes),
        Action::List { verbose } => list(verbose, json),
    }
}

// --------------------------------------------------------------------
// config list
// --------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct ListEntry {
    field: &'static str,
    description: &'static str,
    example: &'static str,
}

fn list(verbose: bool, json: bool) -> Result<(), ConfigCmdError> {
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    if json {
        let rows: Vec<ListEntry> = FIELDS
            .iter()
            .map(|f| ListEntry {
                field: f.key,
                description: f.description,
                example: f.example,
            })
            .collect();
        serde_json::to_writer_pretty(&mut lock, &rows)
            .map_err(|e| ConfigCmdError::Other(e.into()))?;
        lock.write_all(b"\n")
            .map_err(|e| ConfigCmdError::Other(e.into()))?;
        return Ok(());
    }
    if verbose {
        let rows: Vec<Vec<String>> = FIELDS
            .iter()
            .map(|f| vec![f.key.to_owned(), f.description.to_owned()])
            .collect();
        lock.write_all(crate::output::table(&["FIELD", "DESCRIPTION"], &rows).as_bytes())
            .map_err(|e| ConfigCmdError::Other(e.into()))?;
        return Ok(());
    }
    // Plain mode: one field per line, nothing else. Optimised for
    // shell-completion / `xargs` consumption -- the simplest possible
    // contract for callers that just want the key set.
    for f in FIELDS {
        writeln!(lock, "{}", f.key).map_err(|e| ConfigCmdError::Other(e.into()))?;
    }
    Ok(())
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
    // per-invocation flags besides `--config` (see comment on `Flags`
    // below). We never want `--config` (a *config* file path) to be
    // mistaken for `--api-key-file`.
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
    let Some(row) = collect_rows(resolved, api_key)
        .into_iter()
        .find(|r| r.field == key)
    else {
        return Err(ConfigCmdError::NoValue(static_field_name(key)));
    };
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

/// `&'static str` lookup for error reporting. `key` is user-supplied so
/// we can't borrow it past this function; instead we map it to a known
/// static. Callers must ensure `key` is in [`FIELDS`].
fn static_field_name(key: &str) -> &'static str {
    field(key).map_or("?", |f| f.key)
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
// config delete
// --------------------------------------------------------------------

fn delete<E>(config_flag: Option<&Path>, env: &E, yes: bool) -> Result<(), ConfigCmdError>
where
    E: Fn(&str) -> Option<String> + ?Sized,
{
    let (target_path, _src) = config::resolve_path(config_flag, env)?;
    if !target_path.exists() {
        return Err(ConfigCmdError::NoConfigFile { path: target_path });
    }
    if !yes {
        if !std::io::stdin().is_terminal() {
            return Err(ConfigCmdError::Other(anyhow!(
                "refusing to delete {} without a TTY for confirmation. Pass `--yes` to skip the prompt.",
                target_path.display(),
            )));
        }
        let confirmed = dialoguer::Confirm::new()
            .with_prompt(format!("Delete {}?", target_path.display()))
            .default(false)
            .interact()
            .map_err(|e| ConfigCmdError::Other(e.into()))?;
        if !confirmed {
            return Err(ConfigCmdError::Cancelled);
        }
    }
    fs::remove_file(&target_path).map_err(|source| ConfigCmdError::Io {
        path: target_path.clone(),
        source,
    })?;
    eprintln!("Deleted {}", target_path.display());
    Ok(())
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
// config edit
// --------------------------------------------------------------------

fn edit<E>(
    config_flag: Option<&Path>,
    env: &E,
    key: &str,
    value: Option<&str>,
    unset: bool,
) -> Result<(), ConfigCmdError>
where
    E: Fn(&str) -> Option<String> + ?Sized,
{
    if !is_known_key(key) {
        return Err(unknown_key(key));
    }
    if !unset && value.is_none() {
        return Err(ConfigCmdError::NeitherValueNorUnset);
    }
    if key == "api_key" && !unset {
        return Err(ConfigCmdError::RawKeyOnArgv);
    }

    let (target_path, _source) = config::resolve_path(config_flag, env)?;
    let creating = !target_path.exists();
    let mut doc = read_or_init_doc(&target_path)?;

    if unset {
        doc.remove(key);
    } else {
        let raw = value.expect("checked above");
        apply_edit(&mut doc, key, raw)?;
    }

    validate_doc(&doc, key)?;
    write_doc(&target_path, &doc)?;
    if creating {
        eprintln!(
            "note: created new config file {} (just `{key}` set so far).\n      \
             Run `ferro-protect config init` for the guided wizard, or\n      \
             `ferro-protect config init --template` for a commented-out scaffold.",
            target_path.display(),
        );
    }
    Ok(())
}

fn read_or_init_doc(target_path: &Path) -> Result<DocumentMut, ConfigCmdError> {
    match fs::read_to_string(target_path) {
        Ok(s) => s
            .parse::<DocumentMut>()
            .map_err(|e| ConfigCmdError::Other(anyhow!("parsing {}: {e}", target_path.display()))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(initial_doc()),
        Err(source) => Err(ConfigCmdError::Io {
            path: target_path.to_path_buf(),
            source,
        }),
    }
}

fn initial_doc() -> DocumentMut {
    let header = "# ferro-protect config file\n\
                  # Generated by `ferro-protect config init` / `config edit`.\n\
                  # Precedence: flag > env > this file > built-in default.\n\
                  # See `ferro-protect config --help` for details.\n\n";
    header
        .parse::<DocumentMut>()
        .expect("static header always parses")
}

fn apply_edit(doc: &mut DocumentMut, key: &str, raw: &str) -> Result<(), ConfigCmdError> {
    // `api_key` is rejected by the caller; every other key in FIELDS
    // has a parser keyed off `FieldType`. Dispatch on the table rather
    // than on the magic-string key so adding a field is one FIELDS
    // entry, not also a new match arm.
    let meta = field(key).expect("filtered by is_known_key earlier in edit()");
    match meta.ty {
        FieldType::String | FieldType::Path => {
            doc[key] = value(raw);
        }
        FieldType::Bool => {
            let b = parse_boolish(raw).ok_or_else(|| ConfigCmdError::InvalidValue {
                field: meta.key,
                value: raw.to_owned(),
                expected: "true | false (or 1/0, yes/no, on/off)",
            })?;
            doc[key] = value(b);
        }
        FieldType::LogLevel => {
            let lv = parse_log_level(raw).ok_or_else(|| ConfigCmdError::InvalidValue {
                field: meta.key,
                value: raw.to_owned(),
                expected: "error | warn | info | debug | trace",
            })?;
            doc[key] = value(lv.as_str());
        }
    }
    Ok(())
}

fn validate_doc(doc: &DocumentMut, just_edited: &str) -> Result<(), ConfigCmdError> {
    // Re-parse via the strict deserializer so unknown keys / wrong
    // types are caught before we write back. Also catches the
    // mutual-exclusion rules between (host, base_url) and (api_key,
    // api_key_file).
    let s = doc.to_string();
    let parsed: ConfigFile = toml_edit::de::from_str(&s).map_err(|source| {
        ConfigCmdError::Config(config::ConfigError::Parse {
            path: PathBuf::from("<edit>"),
            source,
        })
    })?;
    parsed.validate().map_err(|e| match e {
        config::ConfigError::HostAndBaseUrl => ConfigCmdError::Conflict {
            field: if just_edited == "host" {
                "host"
            } else {
                "base_url"
            },
            other: if just_edited == "host" {
                "base_url"
            } else {
                "host"
            },
        },
        config::ConfigError::ApiKeyAndFile => ConfigCmdError::Conflict {
            field: if just_edited == "api_key" {
                "api_key"
            } else {
                "api_key_file"
            },
            other: if just_edited == "api_key" {
                "api_key_file"
            } else {
                "api_key"
            },
        },
        other => ConfigCmdError::Config(other),
    })
}

/// Write `doc` to `path` atomically: serialize to a sibling
/// `.<name>.tmp.<pid>`, fsync, then `rename` over the destination.
/// The temp file is opened with mode `0o600` on Unix at creation, so
/// there is no TOCTOU window where the secret-bearing file exists
/// with default umask perms. The rename is atomic on POSIX filesystems
/// (and across the same-directory case on Windows via `MoveFileEx`'s
/// default semantics, which `fs::rename` exposes).
fn write_doc(path: &Path, doc: &DocumentMut) -> Result<(), ConfigCmdError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|source| ConfigCmdError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let serialized = doc.to_string();
    let tmp = tmp_sibling(path);
    write_file_secure(&tmp, serialized.as_bytes()).map_err(|source| ConfigCmdError::Io {
        path: tmp.clone(),
        source,
    })?;
    if let Err(source) = fs::rename(&tmp, path) {
        // Best-effort cleanup; the rename error is what the caller cares about.
        let _ = fs::remove_file(&tmp);
        return Err(ConfigCmdError::Io {
            path: path.to_path_buf(),
            source,
        });
    }
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

fn parse_boolish(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn parse_log_level(s: &str) -> Option<LogLevel> {
    match s.trim().to_ascii_lowercase().as_str() {
        "error" => Some(LogLevel::Error),
        "warn" => Some(LogLevel::Warn),
        "info" => Some(LogLevel::Info),
        "debug" => Some(LogLevel::Debug),
        "trace" => Some(LogLevel::Trace),
        _ => None,
    }
}

// --------------------------------------------------------------------
// config init
// --------------------------------------------------------------------

/// `ferro-protect config init --template` writes this. Built from the
/// canonical [`FIELDS`] table so adding/renaming a field can't desync
/// the scaffold from the validators.
fn build_template() -> String {
    let mut out = String::from(
        "# ferro-protect config file\n\
         # Generated by `ferro-protect config init --template`.\n\
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

fn init_template<E>(config_flag: Option<&Path>, env: &E, force: bool) -> Result<(), ConfigCmdError>
where
    E: Fn(&str) -> Option<String> + ?Sized,
{
    let (target_path, _src) = config::resolve_path(config_flag, env)?;
    if target_path.exists() && !force {
        return Err(ConfigCmdError::Other(anyhow!(
            "{} already exists. Pass `--force` to overwrite, or `config edit KEY VALUE` / `config delete` to manage it.",
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
    fs::write(&target_path, build_template()).map_err(|source| ConfigCmdError::Io {
        path: target_path.clone(),
        source,
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&target_path, std::fs::Permissions::from_mode(0o600));
    }
    eprintln!(
        "Wrote template config {}. Uncomment the values you need.",
        target_path.display(),
    );
    Ok(())
}

#[expect(
    clippy::too_many_lines,
    reason = "wizard is a linear prompt sequence; decomposing further obscures the read order"
)]
fn init<E>(
    config_flag: Option<&Path>,
    env: &E,
    force: bool,
    template: bool,
) -> Result<(), ConfigCmdError>
where
    E: Fn(&str) -> Option<String> + ?Sized,
{
    if template {
        return init_template(config_flag, env, force);
    }
    if !std::io::stdin().is_terminal() {
        return Err(ConfigCmdError::NotATty);
    }

    let (target_path, _src) = config::resolve_path(config_flag, env)?;
    let existing = read_or_init_doc(&target_path).ok();

    if target_path.exists() && !force {
        let confirmed = dialoguer::Confirm::new()
            .with_prompt(format!(
                "Config file already exists at {}. Overwrite? (a backup at {0}.bak will be written first)",
                target_path.display(),
            ))
            .default(false)
            .interact()
            .map_err(|e| ConfigCmdError::Other(e.into()))?;
        if !confirmed {
            return Err(ConfigCmdError::Cancelled);
        }
        let bak = {
            let mut p = target_path.clone();
            let mut name = p
                .file_name()
                .map(std::ffi::OsStr::to_os_string)
                .unwrap_or_default();
            name.push(".bak");
            p.set_file_name(name);
            p
        };
        // Copy via read + `write_file_secure` rather than `fs::copy`.
        // `fs::copy` preserves the source's mode, which is wrong if the
        // user hand-created the original at 0644 and embedded a raw
        // `api_key` -- the backup would leak the key under default
        // perms. Re-writing through `write_file_secure` always lands
        // at 0o600 on Unix at creation, no chmod-after-write window.
        let contents = fs::read(&target_path).map_err(|source| ConfigCmdError::Io {
            path: target_path.clone(),
            source,
        })?;
        write_file_secure(&bak, &contents).map_err(|source| ConfigCmdError::Io {
            path: bak.clone(),
            source,
        })?;
    }

    // Pull defaults from the existing file so a re-run feels like an
    // "edit my config" flow rather than a wipe.
    let prior: ConfigFile = existing
        .as_ref()
        .and_then(|doc| toml_edit::de::from_str(&doc.to_string()).ok())
        .unwrap_or_default();

    let host = prompt_string(
        "NVR hostname (e.g. nvr.local or 10.0.0.5)",
        prior.host.as_deref(),
        validate_hostname,
    )?;
    let api_key_choice = prompt_key_source()?;
    let api_key_file_str = match api_key_choice {
        KeyChoice::PointFile => Some(prompt_string(
            "Path to API key file",
            prior.api_key_file.as_deref().and_then(Path::to_str),
            |_| Ok(()),
        )?),
        KeyChoice::WriteFile => {
            let path = prompt_string(
                "Where should the key be written?",
                Some("~/.config/ferro-protect/api_key"),
                |_| Ok(()),
            )?;
            let key = prompt_secret("Paste the API key (hidden)")?;
            write_key_file(&config::expand_tilde(Path::new(&path)), key.expose_secret())?;
            Some(path)
        }
        KeyChoice::EmbedRaw | KeyChoice::Skip => None,
    };
    let raw_key_to_embed = matches!(api_key_choice, KeyChoice::EmbedRaw)
        .then(|| {
            prompt_secret(
                "Paste the API key (hidden); will be written into the config with mode 0600",
            )
        })
        .transpose()?;

    let insecure = dialoguer::Confirm::new()
        .with_prompt("Skip TLS certificate validation? (typical for self-signed NVR certs)")
        .default(prior.insecure.unwrap_or(false))
        .interact()
        .map_err(|e| ConfigCmdError::Other(e.into()))?;

    let json = dialoguer::Confirm::new()
        .with_prompt(
            "Default to JSON output? (most users say no; pass --json per invocation when needed)",
        )
        .default(prior.json.unwrap_or(false))
        .interact()
        .map_err(|e| ConfigCmdError::Other(e.into()))?;

    let log_levels = [
        LogLevel::Error,
        LogLevel::Warn,
        LogLevel::Info,
        LogLevel::Debug,
        LogLevel::Trace,
    ];
    let log_idx = dialoguer::Select::new()
        .with_prompt("Log level")
        .items(log_levels.iter().map(|l| l.as_str()))
        .default(
            prior
                .log_level
                .and_then(|p| log_levels.iter().position(|l| *l == p))
                .unwrap_or(1),
        )
        .interact()
        .map_err(|e| ConfigCmdError::Other(e.into()))?;
    let log_level = log_levels[log_idx];

    // Build the document.
    let mut doc = initial_doc();
    doc["host"] = value(host);
    if let Some(p) = api_key_file_str {
        doc["api_key_file"] = value(p);
    }
    if let Some(secret) = raw_key_to_embed {
        doc["api_key"] = value(secret.expose_secret());
    }
    doc["insecure"] = value(insecure);
    doc["json"] = value(json);
    doc["log_level"] = value(log_level.as_str());

    validate_doc(&doc, "host")?; // any key works; just runs the validator
    write_doc(&target_path, &doc)?;

    eprintln!("\nWrote {}", target_path.display());
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum KeyChoice {
    PointFile,
    WriteFile,
    EmbedRaw,
    Skip,
}

fn prompt_key_source() -> Result<KeyChoice, ConfigCmdError> {
    let items = [
        "Point at an existing key file (recommended)",
        "Paste a key and write it to a new key file",
        "Paste a key into the config file itself (less safe)",
        "Skip — I'll set UNIFI_PROTECT_API_KEY/UNIFI_PROTECT_API_KEY_FILE at runtime",
    ];
    let idx = dialoguer::Select::new()
        .with_prompt("API key source")
        .items(items)
        .default(0)
        .interact()
        .map_err(|e| ConfigCmdError::Other(e.into()))?;
    Ok(match idx {
        0 => KeyChoice::PointFile,
        1 => KeyChoice::WriteFile,
        2 => {
            eprintln!(
                "warning: embedding the raw API key in the config file. The file will be \
                 chmod 0600 but anyone who can read the file gets the key."
            );
            KeyChoice::EmbedRaw
        }
        _ => KeyChoice::Skip,
    })
}

fn prompt_string<V>(
    prompt: &str,
    default: Option<&str>,
    validate: V,
) -> Result<String, ConfigCmdError>
where
    V: Fn(&str) -> Result<(), String>,
{
    loop {
        let input = dialoguer::Input::<String>::new().with_prompt(prompt);
        let input = match default {
            Some(d) => input.default(d.to_string()),
            None => input,
        };
        let raw = input
            .interact_text()
            .map_err(|e| ConfigCmdError::Other(e.into()))?;
        match validate(&raw) {
            Ok(()) => return Ok(raw),
            Err(msg) => eprintln!("invalid: {msg}"),
        }
    }
}

fn validate_hostname(raw: &str) -> Result<(), String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("hostname cannot be empty".into());
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Err("hostname must not include a scheme (no http:// or https://)".into());
    }
    if trimmed.contains('/') {
        return Err("hostname must not include a path".into());
    }
    Ok(())
}

fn prompt_secret(prompt: &str) -> Result<SecretString, ConfigCmdError> {
    let raw = rpassword::prompt_password(format!("{prompt}: "))
        .map_err(|e| ConfigCmdError::Other(e.into()))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ConfigCmdError::Other(anyhow!("empty key entered")));
    }
    Ok(SecretString::from(trimmed.to_owned()))
}

fn write_key_file(path: &Path, key: &str) -> Result<(), ConfigCmdError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|source| ConfigCmdError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    // Use the atomic temp-write + rename helper from `write_doc` so the
    // key file is never visible at default umask perms, even briefly.
    let tmp = tmp_sibling(path);
    write_file_secure(&tmp, key.as_bytes()).map_err(|source| ConfigCmdError::Io {
        path: tmp.clone(),
        source,
    })?;
    if let Err(source) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(ConfigCmdError::Io {
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(())
}

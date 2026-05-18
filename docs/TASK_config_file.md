# Chore: TOML config file and interactive `config` subcommand

> Read [AGENT.md](../AGENT.md) before starting. It carries the
> cross-cutting rules (commit policy, signing, gates, logging) every
> chore must follow. This document carries only the *what-to-do*.

## Why

Today `ferro-protect` is configured entirely via CLI flags and env
vars. The `.env.local` workflow works well for tests and CI but is
clunky for ad-hoc interactive use — users must either source the env
file in every shell or pass flags on every invocation. A persistent
on-disk config gives users a set-it-once baseline while preserving the
existing flag/env behavior unchanged: `.env.local` still drives the
live test suite identically.

Two distinct precedence rules govern the feature, and confusing them
is the single biggest review risk. Keep them straight:

- **Field-level precedence** (which value wins for `host`, `api_key`,
  etc.): `flag > env > config file > built-in default`. Standard CLI
  convention.
- **File-discovery precedence** (which file the loader reads):
  `--config <PATH>` flag > `UNIFI_PROTECT_CONFIG_FILE` env > XDG
  default. The first two are authoritative; the XDG default is
  opportunistic.

## Scope and placement

Lands between phase 4 (read endpoints across all entities, just
shipped) and phase 5 (binary endpoints). The chore is self-contained:
no network code, no spec changes, no library changes outside the CLI
crate. Library remains untouched. Single commit.

If review pushes back on size, the natural split point is between
(a) the config file loader + extended resolver and (b) the `config
init` interactive wizard. The wizard alone is roughly half the LOC
and all of the new interactive deps.

## Field-level precedence (document everywhere)

**`flag > env > config file > built-in default`**

Must be documented in:

- `README.md` (new "Configuration" section)
- CLI long help (`long_about` on every global arg that maps to a
  config-file key)
- `.env.example` (one-line note)
- The header comment block of any config file the wizard writes

## Config file

- **Default location**: `$XDG_CONFIG_HOME/ferro-protect/config.toml`,
  falling back to `$HOME/.config/ferro-protect/config.toml` when
  `XDG_CONFIG_HOME` is unset. On Windows:
  `%APPDATA%\ferro-protect\config.toml`. Use the `etcetera` crate
  (smaller transitive footprint than `directories`).
- **Discovery precedence** (which file the loader opens):
  1. `--config <PATH>` flag. Declared **without** `env =` in clap so
     the env lookup happens explicitly in `config::load` and stays
     separable from the field-level resolver.
  2. `UNIFI_PROTECT_CONFIG_FILE` env var. Naming mirrors the existing
     `UNIFI_PROTECT_API_KEY_FILE` convention (raw value vs path-to-file
     distinguished by `_FILE` suffix).
  3. XDG default path.

  Sources 1 and 2 are *authoritative*: pointing either at a missing
  file is a hard error. Source 3 is *opportunistic*: a missing XDG
  file just means "no config", which is fine.
- The discovery decision is logged at `debug` level naming the chosen
  source: `config: loaded from --config flag: ...` / `config: loaded
  from UNIFI_PROTECT_CONFIG_FILE: ...` / `config: loaded from XDG
  default: ...` / `config: no config file found`.
- **Format**: TOML. Add `toml` to workspace deps.
- **Schema** (all fields optional, `#[serde(deny_unknown_fields)]` as
  a typo guard):

  ```toml
  # one of host / base_url
  host = "nvr.local"
  # base_url = "https://nvr.local/proxy/protect/integration"

  # preferred: pointer to a separate key file (tilde-expanded)
  api_key_file = "~/.config/ferro-protect/api_key"
  # discouraged: raw key inline. The wizard chmods the file 0600 and
  # warns loudly. Most users should use api_key_file instead.
  # api_key = "..."

  insecure = false
  json = false
  log_level = "warn"  # error|warn|info|debug|trace
  ```

  The loader must reject: unknown keys (catches typos like `apikey =`);
  both `host` and `base_url` set in the same file (same rule clap
  enforces at flag level); both `api_key` and `api_key_file` set in
  the same file; an unrecognized `log_level` value.

## API key resolver changes

`crates/ferro-protect-cli/src/api_key.rs::resolve` grows two
lower-priority fallbacks behind the existing four sources. Final order:

1. `--api-key-file <PATH>` flag
2. `UNIFI_PROTECT_API_KEY_FILE` env
3. `UNIFI_PROTECT_API_KEY` env
4. Config file `api_key_file` (pointer; tilde-expanded)
5. Config file `api_key` (raw)
6. `ApiKeyError::NotProvided`

The existing `warn_if_world_readable` helper applies to any file
pulled in via (4). When the wizard writes a raw key into the config
file (5), it chmods the config file 0600 and reuses the same
permissions warning helper.

The function signature changes from three positional args to a small
`Sources` struct so the new file-derived fallbacks don't push the
positional list to five arguments. Keep `resolve` pure with respect to
env via the existing callback — production callers pass
`|k| std::env::var(k).ok()`; tests pass their own closure.

## Interactive subcommand: `ferro-protect config`

New top-level subcommand with three actions.

### `ferro-protect config init`

Interactive wizard.

- Refuses to run when stdin is not a TTY (use `is-terminal`, already
  in workspace deps). Print a clear error pointing the user at
  `--config` plus hand-editing as the non-interactive workflow, and
  exit non-zero.
- If the config file already exists, default each prompt to the
  current value (re-edit flow) and offer to back the file up as
  `config.toml.bak` on overwrite. Prompt before clobbering.
- Validate hostnames against the same "no scheme prefix" rule
  `.env.example` documents: reject `https://` / `http://` prefixes,
  reject trailing path segments. Loop until valid input or `Ctrl-C`.
- Key-source prompt order:
  1. Point at an existing file (path prompt; default
     `~/.config/ferro-protect/api_key`).
  2. Paste a key and write it to a new file (path prompt; file chmod
     0600 on creation).
  3. Paste a key and embed it in the config file itself (loud
     warning before accepting; config file chmod 0600).
  4. Skip — leave key resolution to env/flag at runtime.

  Pasted keys read via `rpassword` (hidden input; no echo to terminal).
- Use `dialoguer` for confirm / select / input. Pulls in only
  `console`. If review prefers zero new interactive deps, the fallback
  is a hand-rolled prompt loop using stdin + `is-terminal` + a single
  `termios` toggle for the hidden-input case — more code, less
  surface area, no extra deps.

### `ferro-protect config show`

Print the **effective** resolved config after merging file + env +
flags, with each value annotated by its source:

```
host = "nvr.local"        # from env: UNIFI_PROTECT_HOST
insecure = true           # from --insecure flag
log_level = "warn"        # from config file: /home/u/.config/ferro-protect/config.toml
api_key = <set>           # from env: UNIFI_PROTECT_API_KEY_FILE
```

The raw API key value is **never** printed; show `<set>` / `<unset>`
only. Respects the global `--json` flag (the JSON form drops the
human comment and emits a `{ value, source }` shape per field).

### `ferro-protect config path`

Print the resolved config file path on a single line, whether or not
the file exists. Useful in shell scripts: `$(ferro-protect config
path)`. The path printed reflects file-discovery precedence:
`--config` if set, else `UNIFI_PROTECT_CONFIG_FILE` if set, else the
XDG default.

## Code organization

- New module `crates/ferro-protect-cli/src/config.rs`:
  - `pub struct ConfigFile { ... }` — on-disk schema (serde,
    `deny_unknown_fields`). `Option<T>` everywhere; "absent" and "set
    to the default" must be distinguishable for source attribution.
  - `pub struct ResolvedConfig { ... }` — merged view with per-field
    source attribution. Each field is a small `Resolved<T> { value: T,
    source: Source }` where `Source` enumerates `Flag` / `Env(&'static
    str)` / `ConfigFile(PathBuf)` / `Default`.
  - `pub fn load(explicit: Option<&Path>, env: &E) -> Result<Option<(ConfigFile, PathBuf)>>` —
    handles the three-source file-discovery precedence above. Returns
    `Ok(None)` only when no explicit source was set *and* the XDG
    default does not exist. The returned `PathBuf` is for diagnostics
    (used in `config show` source attribution and the debug log).
  - `pub fn resolve(file: Option<&ConfigFile>, cli: &Cli, env: &E) -> ResolvedConfig` —
    pure merger; table-driven tests cover every precedence path.
- New module `crates/ferro-protect-cli/src/commands/config.rs` for
  the three actions (`init`, `show`, `path`).
- `api_key::resolve` grows the `Sources` struct described above.
- The "warn if lax permissions" helper stays in `api_key.rs` and gets
  reused by `config::save_*` for the config file path.

## Workspace deps to add

Each new dep needs a one-line justification in the chore's commit
body. `cargo deny check` must stay green.

| Crate       | Version | Why                                                   |
|-------------|---------|-------------------------------------------------------|
| `toml`      | `0.8`   | Config file parser; standard Rust ecosystem choice.   |
| `etcetera`  | `0.8`   | XDG/AppData paths, cross-platform, minimal deps.      |
| `rpassword` | `7`     | Hidden-input read for pasted API keys.                |
| `dialoguer` | `0.11`  | Wizard prompts (confirm/select/input); pulls only `console`. |

## Tests

All test files live under `crates/ferro-protect-cli/tests/`.

- **`config_load.rs`** — TOML parsing.
  - Happy path: minimal file, full file.
  - Unknown key rejected (typo guard).
  - `host` + `base_url` both set → rejected.
  - `api_key` + `api_key_file` both set → rejected.
  - Bad `log_level` value → rejected.
  - File-discovery: `--config <tempfile>` honored, `UNIFI_PROTECT_CONFIG_FILE`
    honored, XDG fallback honored, `--config` wins over the env var.
  - `--config <missing>` and `UNIFI_PROTECT_CONFIG_FILE=<missing>` both
    hard-error; missing XDG default does not.
  - Use `tempfile::NamedTempFile` (already in workspace dev-deps).
- **`config_resolve.rs`** — table-driven cases for `resolve()`:
  file-only, env-only, flag-only, and combinations including
  end-to-end precedence (file says host=A, env says host=B, flag says
  host=C → C wins; drop the flag → B wins; drop env → A wins).
- **`cli_config.rs`** (`assert_cmd`):
  - `config show --config <tempfile>` with a valid config asserts the
    source-attribution output for each field.
  - `config show` with `HOME=<tmpdir>` and unset `XDG_CONFIG_HOME`
    finds the XDG-fallback path.
  - `config path` reflects file-discovery precedence across the three
    sources.
  - `config init` is excluded from automated CI (needs a TTY). Mark
    `#[ignore]` with a comment pointing at the manual-test entry in
    the chore's PROGRESS.md write-up.

**Live tests**: none. This chore touches no network code.

## Docs

1. **`README.md`** — new "Configuration" section between "Quick start"
   and "Running tests". Covers:
   - Location (XDG default, `--config` override,
     `UNIFI_PROTECT_CONFIG_FILE` env override).
   - Format (TOML), the schema, and that unknown keys are rejected.
   - Field-level precedence (`flag > env > file > default`) with an
     example walkthrough.
   - Pointer-vs-raw-key tradeoff. Security note: the wizard chmods
     0600; hand-editors should too.
   - Subsection **"Pointing at a config file from a deployment"**:
     explains that `UNIFI_PROTECT_CONFIG_FILE=/etc/ferro-protect/config.toml`
     is the right hook for systemd units, Docker `ENV`, k8s
     ConfigMaps, and CI jobs — anywhere argv is awkward to control.
     Distinguish this from `UNIFI_PROTECT_API_KEY_FILE`, which points
     at a *key* file, not a *config* file.
   - Link to `ferro-protect config init` as the recommended way to
     produce a valid config file.
2. **`.env.example`** — append a short note that env vars override any
   `~/.config/ferro-protect/config.toml`, and that the typical split
   is "config file for interactive use, `.env.local` for tests and
   CI."
3. **CLI long help** — `long_about` on every global flag mentions its
   env var **and** matching config-file key. The `--config` flag's
   `long_about` explicitly names `UNIFI_PROTECT_CONFIG_FILE` and
   notes that both are authoritative (missing file errors out) while
   the XDG default is opportunistic. The new `config` subcommand has
   its own `long_about` walking through `init` / `show` / `path`.
4. **`PROGRESS.md`** — entry per AGENT.md's progress-logging rules.

## Out of scope (deferred)

- **Named profiles** in one file (`[nvr.home]` / `[nvr.work]`). Defer
  until someone actually has two NVRs. Would land later as a
  `--profile <NAME>` flag with per-profile precedence rules.
- **Keyring-backed key storage**. Mention as a future option in the
  README security note; do not build now.
- **Auto-migration from `.env.local` → `config.toml`**. The wizard is
  the migration tool; do not parse env files automatically.

## Acceptance

- `cargo fmt`, `cargo clippy --all-targets --all-features -- -D
  warnings`, `cargo test --all`, `cargo deny check` all green.
- In a fresh `HOME`, `ferro-protect config init` writes a valid config
  file that drives `ferro-protect info` against a real NVR with no
  further env or flag setup.
- `ferro-protect config show` correctly attributes every effective
  value to its source across the four-way matrix
  (file × env × flag × default).
- The end-to-end field-level precedence test (`flag > env > file >
  default`) passes.
- The file-discovery precedence test (`--config > UNIFI_PROTECT_CONFIG_FILE >
  XDG default`) passes, including the "authoritative-source-missing
  is a hard error, opportunistic-source-missing is fine" contract.
- `--help` long form on `--host` mentions both `UNIFI_PROTECT_HOST`
  and the `host` config key. Same pattern for every other global flag
  with a config-file equivalent.

**Commit message**: `chore(cli): add TOML config file and interactive config subcommand`

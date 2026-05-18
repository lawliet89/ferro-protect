# ferro-protect

Async Rust client and CLI for the [UniFi Protect](https://ui.com/) local integration
API. Targets Protect application version **7.1.60**. The workspace publishes two
crates: the `ferro-protect` library and the `ferro-protect-cli` binary
(`ferro-protect`).

## Status

Pre-0.1.0. Built phase by phase against the OpenAPI spec hosted at
<https://github.com/beezly/unifi-apis>.

New to the codebase? Start with [ARCHITECTURE.md](ARCHITECTURE.md) — it
covers the philosophy, the codegen seam, the file map, and a suggested
reading order before you open any source file.

## Device coverage disclaimer

The maintainer's personal UniFi Protect setup does not include every
device type Protect supports. For device categories the maintainer
cannot observe directly, the implementation is built purely from the
OpenAPI spec and exercised only by mocked integration tests; in the
live test suite, the corresponding `*_list` checks still run and
return an empty list, while `*_get` checks skip gracefully when the
NVR has none of that device type.

In practice this means the *shape* of every endpoint is verified
(URL routing, error mapping, JSON deserialisation against the spec),
but the *spec-vs-firmware drift* surface — the kind of mismatch that
[surfaced for smart-audio detection on cameras](crates/ferro-protect/build_support/spec_rewrite.rs)
— has only been validated against the subset of device types in the
maintainer's lab. There may be similar drift on other device types
nobody on this side has live access to.

If you run a Protect installation with devices the test fleet
doesn't cover, **PRs are very welcome.** The most useful
contributions are:

- Running `./scripts/live-test` against your NVR and reporting any
  `unknown variant` / deserialize failures.
- Adding a new `spec_rewrite.rs` preprocessing rule if a device
  exposes a runtime value missing from the spec (see
  `drop_drifted_audio_detection_enum` for the pattern).
- Adding a small fixture-backed mocked test under
  `crates/ferro-protect/tests/fixtures/` if you can share a sanitised
  example of a response your firmware emits.

## Clone

```sh
git clone --recurse-submodules https://github.com/lawliet89/ferro-protect.git
```

If you forgot `--recurse-submodules`:

```sh
git submodule update --init --recursive
```

## Build

```sh
cargo build --workspace
```

## Local checks

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo deny check
```

Install the pre-commit hook to run fmt and clippy automatically:

```sh
ln -s ../../scripts/pre-commit .git/hooks/pre-commit
chmod +x scripts/pre-commit
```

## Configuration

The CLI reads its configuration from up to four sources, in this
priority order (highest first):

1. **Command-line flag** — e.g. `--host nvr.local`.
2. **Environment variable** — e.g. `UNIFI_PROTECT_HOST=nvr.local`.
3. **TOML config file** — see below.
4. **Built-in default**.

### Config file location

By default the CLI looks at
`$XDG_CONFIG_HOME/ferro-protect/config.toml`, falling back to
`$HOME/.config/ferro-protect/config.toml` if `XDG_CONFIG_HOME` is unset.
On Windows the path is `%APPDATA%\ferro-protect\config.toml`.

You can point at a different file two ways, in priority order:

1. `--config <PATH>` flag.
2. `UNIFI_PROTECT_CONFIG_FILE=<PATH>` env var.

The flag and the env var are **authoritative**: pointing either at a
missing file is a hard error. The XDG default is **opportunistic**: a
missing file at that path just means "no config", which is fine.

The env var name mirrors `UNIFI_PROTECT_API_KEY_FILE` — the `_FILE`
suffix consistently means "path to a file" (as opposed to a raw
value).

### Pointing at a config file from a deployment

`UNIFI_PROTECT_CONFIG_FILE=/etc/ferro-protect/config.toml` is the right
hook for systemd units, Docker `ENV` directives, k8s ConfigMaps, and
CI jobs — anywhere argv is awkward to control. Distinguish this from
`UNIFI_PROTECT_API_KEY_FILE`, which points at a *key* file, not a
*config* file.

### Format

```toml
# Either set host or base_url, not both. host is the common case.
host = "nvr.local"
# base_url = "https://nvr.local/proxy/protect/integration"

# Preferred: pointer to a separate key file (tilde-expanded at load time).
api_key_file = "~/.config/ferro-protect/api_key"
# Discouraged alternative: raw key inline. The wizard chmods the file
# 0600 and warns loudly; the loader treats it as a last-resort source.
# api_key = "..."

insecure = false
json = false
log_level = "warn"  # one of: error, warn, info, debug, trace
```

Unknown keys are rejected at load time (typo guard). Setting both
`host` and `base_url`, or both `api_key` and `api_key_file`, is also
rejected.

### Managing the config file

```sh
ferro-protect config init                  # interactive wizard (TTY required)
ferro-protect config init --template       # write a commented-out scaffold; no TTY needed
ferro-protect config show                  # print effective config + source per field
ferro-protect config show host             # bare value, scriptable
ferro-protect config show --json           # JSON form, with per-field {value, source}
ferro-protect config path                  # print the resolved config file path
ferro-protect config edit host nvr.local   # set a single field; preserves comments
ferro-protect config edit host --unset     # remove a field
ferro-protect config delete                # remove the file (prompts for confirmation)
ferro-protect config delete --yes          # skip the prompt
ferro-protect config list                  # list recognised field names (one per line)
ferro-protect config list -v               # field + description table
```

`config list` is useful for shell completion — e.g.

```sh
# bash
complete -W "$(ferro-protect config list)" ferro-protect-config-keys
```

`config show`, `config path`, and `config delete` error when the
resolved config file doesn't exist — they're file-inspection /
file-management commands, so silently rendering defaults or no-opping
would be misleading. Run `config init` (full wizard), `config init
--template` (commented-out scaffold; safe in non-TTY contexts), or
`config edit KEY VALUE` (creates the file on first use, with a stderr
warning) to bootstrap one. Other subcommands (`info`, `cameras list`,
…) still treat a missing XDG default as "no config" and fall back to
env vars + flags as usual.

`config edit` refuses to set `api_key` from the command line — the raw
key would land in shell history, `ps`, and the parent process's argv.
Use `config init` (hidden paste), `api_key_file` (point at a key
file), or the env var instead. `config edit api_key_file <PATH>` is
fine.

### Config files and the test suite

The library's **live tests are env-driven, not config-driven**, on
purpose. Running `cargo test --all` with a populated
`~/.config/ferro-protect/config.toml` but no sourced `.env.local`
results in live tests **skipping** — they will not silently hit your
real NVR just because a config file exists.

CLI integration tests isolate themselves from the developer's config
via `HOME=<tmpdir>` + scrubbed `UNIFI_PROTECT_*` env vars (see
`crates/ferro-protect-cli/tests/common/mod.rs`). To run live tests
with a config-file-only setup, source `.env.local` or set
`UNIFI_PROTECT_HOST` plus an API-key source in the environment first.

### Security notes

When you embed a raw `api_key` in `config.toml`, the wizard chmods the
file `0600` on Unix. If you hand-edit, you should too. The loader
emits a stderr warning if a referenced key file has lax permissions.

A future option would be keyring-backed storage; not built today.

## Running tests

### Quick start

```sh
cargo test --all
```

Runs everything: unit tests, mocked integration tests (against an in-process
`wiremock` server), snapshot tests, doc tests, and live tests. Live tests
**auto-skip** when no NVR is configured -- they check for
`UNIFI_PROTECT_HOST` at the top of the function and early-return as `ok`
when absent. So this command is safe and useful on any machine, NVR or not.

`cargo test --all` against a real NVR works under default parallelism --
the client ships with a proactive rate limiter pinned to Protect's
advertised quota (`10-in-1sec` on 7.1.60, configurable via
`ProtectClientBuilder::rate_limit`) and a retry middleware that honours
`Retry-After` on 429s. You should not need `--test-threads=N`; if you
hit 429s anyway, file an issue.

### How to run live tests

One-time setup:

```sh
cp .env.example .env.local
$EDITOR .env.local          # fill in UNIFI_PROTECT_HOST + key path
chmod 600 <your api key file>
```

Run the live test suite:

```sh
./scripts/live-test
```

The script sources `.env.local`, then runs the live tests with
`--features insecure-tls` (so `--insecure`/self-signed NVRs work) and
`--nocapture` (so test stdout reaches your terminal). If you'd rather
run them through plain `cargo`:

```sh
set -a; source .env.local; set +a
cargo test --all
```

Run an ad hoc command against the real NVR (useful for poking at a single
subcommand without going through the test harness):

```sh
set -a; source .env.local; set +a
cargo run -p ferro-protect-cli -- \
  --host "$UNIFI_PROTECT_HOST" \
  --api-key-file "$UNIFI_PROTECT_API_KEY_FILE" \
  ${UNIFI_PROTECT_INSECURE:+--insecure} \
  info
```

#### Environment variables

All prefixed `UNIFI_PROTECT_` to make accidental activation impossible:

| Var | Purpose |
|---|---|
| `UNIFI_PROTECT_HOST` | NVR hostname or `host:port` -- **no scheme prefix**. The client always wraps this as `https://{host}/proxy/protect/integration`. **Required.** Absence means all live tests skip. |
| `UNIFI_PROTECT_API_KEY_FILE` | Path to a file containing the API key. **Preferred over the raw env var below.** |
| `UNIFI_PROTECT_API_KEY` | Raw API key. Use only if the file form is impractical. |
| `UNIFI_PROTECT_INSECURE` | Set to a non-empty value to accept self-signed TLS (common on home NVRs). |
| `UNIFI_PROTECT_ALLOW_MUTATIONS` | Set to `1` to also run `live_write_*` tests. See below. |
| `UNIFI_PROTECT_LOG` | Log filter for the CLI (env_logger syntax). Overridden by `--log-level`; falls back to `RUST_LOG`, then `warn`. Logs go to stderr. |

If `HOST` is set but no key source is, the test helper panics with a
clear message instead of silently skipping -- a half-configured live env
is almost always a mistake.

#### Good moments to run them

Live tests are safe to run any time — the suite skips cleanly without
an NVR. Specific occasions where running them is *especially* useful:

- After `./scripts/update-spec`: verify the regenerated client still
  matches your NVR's wire protocol.
- After changing the client builder, error mapping, auth, or TLS code:
  the live test confirms the real device accepts the request.
- After touching the API-key resolver or its env-var contract: exercise
  your normal credential flow end to end.
- After adding a new endpoint method: the matching `live_read_*` test
  proves the wrapper round-trips through a real device.
- Before tagging a release.

### Mutating live tests

Tests named `live_write_*` change NVR state: PATCH configuration, trigger
alarms, upload files, etc. They are gated behind a second env var:

```sh
UNIFI_PROTECT_ALLOW_MUTATIONS=1 ./scripts/live-test
```

Run these **deliberately, ideally against a non-production NVR.** They can
trigger physical effects (sirens, chimes, motion notifications), change
recording modes, or modify saved settings.

### Security notes

- `.env`, `.env.local` are gitignored. Do not check them in.
- Prefer `UNIFI_PROTECT_API_KEY_FILE` over the raw env var, and keep the
  key file outside the repo (e.g. `~/.config/ferro-protect/api_key`).
- Restrict the key file's permissions: `chmod 600 <path>`.
- The CI workflow fails fast if any `UNIFI_PROTECT_*` env var is present
  in the runner environment, so a leaked secret cannot accidentally hit a real
  NVR from a PR build.

# ferro-protect

Async Rust client and CLI for the [UniFi Protect](https://ui.com/) local integration
API. Targets Protect application version **6.2.83**. The workspace publishes two
crates: the `ferro-protect` library and the `ferro-protect-cli` binary
(`ferro-protect`).

## Status

Pre-0.1.0. Built phase by phase against the OpenAPI spec hosted at
<https://github.com/beezly/unifi-apis>.

New to the codebase? Start with [ARCHITECTURE.md](ARCHITECTURE.md) — it
covers the philosophy, the codegen seam, the file map, and a suggested
reading order before you open any source file.

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

## Running tests

### Quick start

```sh
cargo test --all
```

Runs everything: unit tests, mocked integration tests (against an in-process
`wiremock` server), snapshot tests, doc tests, and live tests. Live tests
**auto-skip** when no NVR is configured -- they check for
`FERRO_PROTECT_LIVE_HOST` at the top of the function and early-return as `ok`
when absent. So this command is safe and useful on any machine, NVR or not.

### How to run live tests

One-time setup:

```sh
cp .env.example .env.local
$EDITOR .env.local          # fill in FERRO_PROTECT_LIVE_HOST + key path
chmod 600 <your api key file>
```

Run the live test suite:

```sh
./scripts/live-test
```

The script sources `.env.local`, then runs the live tests with
`--features dangerous-tls` (so `--insecure`/self-signed NVRs work) and
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
  --host "$FERRO_PROTECT_LIVE_HOST" \
  --api-key-file "$FERRO_PROTECT_LIVE_API_KEY_FILE" \
  ${FERRO_PROTECT_LIVE_INSECURE:+--insecure} \
  info
```

#### Environment variables

All prefixed `FERRO_PROTECT_LIVE_` to make accidental activation impossible:

| Var | Purpose |
|---|---|
| `FERRO_PROTECT_LIVE_HOST` | NVR hostname or IP. **Required.** Absence means all live tests skip. |
| `FERRO_PROTECT_LIVE_API_KEY_FILE` | Path to a file containing the API key. **Preferred over the raw env var below.** |
| `FERRO_PROTECT_LIVE_API_KEY` | Raw API key. Use only if the file form is impractical. |
| `FERRO_PROTECT_LIVE_INSECURE` | Set to a non-empty value to accept self-signed TLS (common on home NVRs). |
| `FERRO_PROTECT_LIVE_ALLOW_MUTATIONS` | Set to `1` to also run `live_write_*` tests. See below. |

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
FERRO_PROTECT_LIVE_ALLOW_MUTATIONS=1 ./scripts/live-test
```

Run these **deliberately, ideally against a non-production NVR.** They can
trigger physical effects (sirens, chimes, motion notifications), change
recording modes, or modify saved settings.

### Security notes

- `.env`, `.env.local` are gitignored. Do not check them in.
- Prefer `FERRO_PROTECT_LIVE_API_KEY_FILE` over the raw env var, and keep the
  key file outside the repo (e.g. `~/.config/ferro-protect/api_key`).
- Restrict the key file's permissions: `chmod 600 <path>`.
- The CI workflow fails fast if any `FERRO_PROTECT_LIVE_*` env var is present
  in the runner environment, so a leaked secret cannot accidentally hit a real
  NVR from a PR build.

//! Shared helpers for live tests.
//!
//! Cargo compiles every `tests/*.rs` file as a separate integration-test
//! binary, so each test file that wants these helpers brings them in with
//! `mod common;` at the top. The `#[allow(dead_code)]` is intentional --
//! tests that use only one helper would otherwise warn about the others.

#![allow(dead_code)]

use ferro_protect::ProtectClient;
use secrecy::SecretString;

const HOST_ENV: &str = "UNIFI_PROTECT_HOST";
const KEY_FILE_ENV: &str = "UNIFI_PROTECT_API_KEY_FILE";
const KEY_ENV: &str = "UNIFI_PROTECT_API_KEY";
const INSECURE_ENV: &str = "UNIFI_PROTECT_INSECURE";
const ALLOW_MUTATIONS_ENV: &str = "UNIFI_PROTECT_ALLOW_MUTATIONS";

/// Build a [`ProtectClient`] from `UNIFI_PROTECT_*` env vars.
///
/// - Returns `None` when `UNIFI_PROTECT_HOST` is unset -- the caller
///   should early-return so the test counts as `ok` without making any
///   network calls.
/// - **Panics** if `HOST` is set but neither `_API_KEY_FILE` nor `_API_KEY`
///   is. A half-configured live env is almost always a developer mistake
///   we want to surface loudly, not silently skip past.
/// - **Panics** if `_INSECURE` is requested but the library was built
///   without the `dangerous-tls` feature. `scripts/live-test` passes
///   `--features dangerous-tls` automatically; manual `cargo test`
///   invocations targeting a self-signed NVR need to do the same.
pub fn live_client() -> Option<ProtectClient> {
    let host = std::env::var(HOST_ENV).ok()?;

    let key = if let Ok(path) = std::env::var(KEY_FILE_ENV) {
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!("{HOST_ENV}={host} is set but reading {KEY_FILE_ENV}={path} failed: {e}")
        });
        SecretString::from(raw.trim().to_string())
    } else if let Ok(raw) = std::env::var(KEY_ENV) {
        SecretString::from(raw)
    } else {
        panic!("{HOST_ENV} is set but no key source: set {KEY_FILE_ENV} (preferred) or {KEY_ENV}");
    };

    let insecure = parse_boolish_env(INSECURE_ENV);

    let mut builder = ProtectClient::builder().host(host).api_key(key);
    if insecure {
        #[cfg(feature = "dangerous-tls")]
        {
            builder = builder.tls(ferro_protect::TlsMode::AcceptInvalid);
        }
        #[cfg(not(feature = "dangerous-tls"))]
        {
            panic!(
                "{INSECURE_ENV} is set but ferro-protect was built without the `dangerous-tls` feature. \
                 Re-run with `cargo test --features dangerous-tls ...` or use ./scripts/live-test."
            );
        }
    }
    Some(builder.build().expect("live client builds"))
}

/// `true` only when `UNIFI_PROTECT_ALLOW_MUTATIONS` resolves to a
/// truthy value (`1`, `true`, `yes`, `on`). Gates any test that writes
/// to the NVR -- PATCH, action POSTs, file uploads. Kept strict on the
/// truthy side and forgiving on the falsy side so a stray `=0` in
/// `.env.local` is unambiguously off.
pub fn mutations_allowed() -> bool {
    parse_boolish_env(ALLOW_MUTATIONS_ENV)
}

/// Boolish parsing that matches the CLI's `--insecure` flag behaviour
/// (clap's `BoolishValueParser`). Empty / `0` / `false` / `no` / `off`
/// (case-insensitive) -> false; `1` / `true` / `yes` / `on` -> true;
/// missing -> false; anything else -> panic so the misconfiguration is
/// loud.
fn parse_boolish_env(name: &str) -> bool {
    let Ok(raw) = std::env::var(name) else {
        return false;
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "0" | "false" | "no" | "off" => false,
        "1" | "true" | "yes" | "on" => true,
        other => panic!(
            "{name} has unrecognised value {other:?}; use one of 1/true/yes/on or 0/false/no/off"
        ),
    }
}

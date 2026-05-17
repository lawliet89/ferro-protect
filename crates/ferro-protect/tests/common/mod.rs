//! Shared helpers for live tests.
//!
//! Cargo compiles every `tests/*.rs` file as a separate integration-test
//! binary, so each test file that wants these helpers brings them in with
//! `mod common;` at the top. The `#[allow(dead_code)]` is intentional --
//! tests that use only one helper would otherwise warn about the others.

#![allow(dead_code)]

use ferro_protect::ProtectClient;
use secrecy::SecretString;

const HOST_ENV: &str = "FERRO_PROTECT_LIVE_HOST";
const KEY_FILE_ENV: &str = "FERRO_PROTECT_LIVE_API_KEY_FILE";
const KEY_ENV: &str = "FERRO_PROTECT_LIVE_API_KEY";
const INSECURE_ENV: &str = "FERRO_PROTECT_LIVE_INSECURE";
const ALLOW_MUTATIONS_ENV: &str = "FERRO_PROTECT_LIVE_ALLOW_MUTATIONS";

/// Build a [`ProtectClient`] from `FERRO_PROTECT_LIVE_*` env vars.
///
/// - Returns `None` when `FERRO_PROTECT_LIVE_HOST` is unset -- the caller
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

    let insecure = std::env::var(INSECURE_ENV).is_ok_and(|v| !v.is_empty());

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

/// `true` only when `FERRO_PROTECT_LIVE_ALLOW_MUTATIONS=1`. Gates any test
/// that writes to the NVR -- PATCH, action POSTs, file uploads.
pub fn mutations_allowed() -> bool {
    std::env::var(ALLOW_MUTATIONS_ENV).is_ok_and(|v| v == "1")
}

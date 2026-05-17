#![forbid(unsafe_code)]
#![allow(clippy::pedantic, clippy::nursery)]

//! Live integration test. **Not** run by `cargo test` by default.
//!
//! Run with `./scripts/live-test` (which sources `.env.local` if present)
//! or invoke directly:
//!
//! ```sh
//! cargo test -p ferro-protect --test live -- --ignored --nocapture
//! ```
//!
//! Required environment variables:
//!
//! - `FERRO_PROTECT_LIVE_HOST` -- the NVR host (e.g. `nvr.local` or
//!   `10.0.0.5`). May include a port.
//! - One of:
//!     - `FERRO_PROTECT_LIVE_API_KEY_FILE` -- path to a file holding the
//!       API key (trimmed). Preferred.
//!     - `FERRO_PROTECT_LIVE_API_KEY` -- the raw key. Fine for shell env
//!       vars; never commit this anywhere.
//!
//! Optional:
//!
//! - `FERRO_PROTECT_LIVE_INSECURE` -- set to any non-empty value to skip
//!   TLS verification. Use only when your NVR ships a self-signed cert
//!   you cannot pin.
//!
//! These names are deliberately distinct from the CLI's
//! `UNIFI_PROTECT_*` env vars so a developer's normal shell environment
//! does not silently flip these tests from ignored to active.

use ferro_protect::{ProtectClient, TlsMode};
use secrecy::SecretString;

const HOST_ENV: &str = "FERRO_PROTECT_LIVE_HOST";
const KEY_FILE_ENV: &str = "FERRO_PROTECT_LIVE_API_KEY_FILE";
const KEY_ENV: &str = "FERRO_PROTECT_LIVE_API_KEY";
const INSECURE_ENV: &str = "FERRO_PROTECT_LIVE_INSECURE";

fn config() -> (String, SecretString, bool) {
    let host = std::env::var(HOST_ENV)
        .unwrap_or_else(|_| panic!("missing {HOST_ENV} (live test requires real NVR host)"));

    let key = if let Ok(path) = std::env::var(KEY_FILE_ENV) {
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("could not read {KEY_FILE_ENV}={path}: {e}"));
        SecretString::from(raw.trim().to_string())
    } else if let Ok(raw) = std::env::var(KEY_ENV) {
        SecretString::from(raw)
    } else {
        panic!("set {KEY_FILE_ENV} (preferred) or {KEY_ENV}");
    };

    let insecure = std::env::var(INSECURE_ENV).is_ok_and(|v| !v.is_empty());
    (host, key, insecure)
}

fn build_client() -> ProtectClient {
    let (host, key, insecure) = config();
    let mut builder = ProtectClient::builder().host(host).api_key(key);
    if insecure {
        builder = builder.tls(TlsMode::AcceptInvalid);
    }
    builder.build().expect("client builds with live config")
}

#[tokio::test]
#[ignore = "live NVR -- opt in via scripts/live-test or --include-ignored"]
async fn info_returns_real_version() {
    let client = build_client();
    let info = client
        .info()
        .await
        .expect("info call to real NVR succeeded");
    let version = info.application_version.to_string();
    println!("Protect application version: {version}");
    assert!(
        !version.is_empty(),
        "live NVR returned an empty applicationVersion"
    );
}

#![forbid(unsafe_code)]
#![allow(clippy::pedantic, clippy::nursery)]

//! Live integration tests. Run as part of the normal `cargo test --all`
//! suite -- when `FERRO_PROTECT_LIVE_HOST` is unset they early-return and
//! count as `ok`. Configure the live environment via `.env.local` (see
//! `.env.example` for the full var list) and run them with:
//!
//! ```sh
//! ./scripts/live-test                # sources .env.local for you
//! # -- or, manually --
//! source .env.local && cargo test --all
//! ```
//!
//! Mutating tests (`live_write_*`) additionally require
//! `FERRO_PROTECT_LIVE_ALLOW_MUTATIONS=1`. See PLAN.md "Testing strategy"
//! for the contract.

mod common;

#[tokio::test]
async fn live_read_info() {
    let Some(client) = common::live_client() else {
        println!("(skipping live_read_info: FERRO_PROTECT_LIVE_HOST not set)");
        return;
    };
    let info = client
        .info()
        .await
        .expect("info call to real NVR succeeded");
    let version = info.application_version.to_string();
    println!("live_read_info: Protect application version = {version}");
    assert!(
        !version.is_empty(),
        "live NVR returned an empty applicationVersion"
    );
}

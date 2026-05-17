#![forbid(unsafe_code)]
#![allow(clippy::pedantic, clippy::nursery)]

//! Live integration tests. Run as part of the normal `cargo test --all`
//! suite -- when `UNIFI_PROTECT_HOST` is unset they early-return and
//! count as `ok`. Configure the live environment via `.env.local` (see
//! `.env.example` for the full var list) and run them with:
//!
//! ```sh
//! ./scripts/live-test                # sources .env.local for you
//! # -- or, manually --
//! source .env.local && cargo test --all
//! ```
//!
//! The env vars (`UNIFI_PROTECT_HOST`, `UNIFI_PROTECT_API_KEY_FILE` etc.)
//! are shared with the CLI, so a single `.env.local` drives both. Mutating
//! tests (`live_write_*`) additionally require
//! `UNIFI_PROTECT_ALLOW_MUTATIONS=1`. See PLAN.md "Testing strategy" for
//! the contract.

mod common;

#[tokio::test]
async fn live_read_info() {
    let Some(client) = common::live_client() else {
        println!("(skipping live_read_info: UNIFI_PROTECT_HOST not set)");
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

#[tokio::test]
async fn live_read_cameras_list() {
    let Some(client) = common::live_client() else {
        println!("(skipping live_read_cameras_list: UNIFI_PROTECT_HOST not set)");
        return;
    };
    let cameras = client
        .cameras()
        .list()
        .await
        .expect("cameras list call succeeded");
    println!(
        "live_read_cameras_list: {} camera(s) returned",
        cameras.len()
    );
    for c in &cameras {
        println!("  - {} {:?} state={}", c.id, c.name, c.state);
    }
}

#[tokio::test]
async fn live_read_cameras_get() {
    let Some(client) = common::live_client() else {
        println!("(skipping live_read_cameras_get: UNIFI_PROTECT_HOST not set)");
        return;
    };
    let cameras = client
        .cameras()
        .list()
        .await
        .expect("cameras list call succeeded");
    let Some(first) = cameras.first() else {
        println!("(skipping live_read_cameras_get: NVR has no cameras)");
        return;
    };
    let fetched = client
        .cameras()
        .get(&first.id)
        .await
        .expect("cameras get call succeeded");
    println!(
        "live_read_cameras_get: round-tripped {} ({:?})",
        fetched.id, fetched.name
    );
    assert_eq!(fetched.id, first.id, "list+get should agree on id");
}

#[tokio::test]
async fn live_read_lights_list() {
    let Some(client) = common::live_client() else {
        println!("(skipping live_read_lights_list: UNIFI_PROTECT_HOST not set)");
        return;
    };
    let lights = client
        .lights()
        .list()
        .await
        .expect("lights list call succeeded");
    println!("live_read_lights_list: {} light(s) returned", lights.len());
    for l in &lights {
        println!("  - {} {:?} state={}", l.id, l.name, l.state);
    }
}

#[tokio::test]
async fn live_read_lights_get() {
    let Some(client) = common::live_client() else {
        println!("(skipping live_read_lights_get: UNIFI_PROTECT_HOST not set)");
        return;
    };
    let lights = client
        .lights()
        .list()
        .await
        .expect("lights list call succeeded");
    let Some(first) = lights.first() else {
        println!("(skipping live_read_lights_get: NVR has no lights)");
        return;
    };
    let fetched = client
        .lights()
        .get(&first.id)
        .await
        .expect("lights get call succeeded");
    println!(
        "live_read_lights_get: round-tripped {} ({:?})",
        fetched.id, fetched.name
    );
    assert_eq!(fetched.id, first.id, "list+get should agree on id");
}

#[tokio::test]
async fn live_read_chimes_list() {
    let Some(client) = common::live_client() else {
        println!("(skipping live_read_chimes_list: UNIFI_PROTECT_HOST not set)");
        return;
    };
    let chimes = client
        .chimes()
        .list()
        .await
        .expect("chimes list call succeeded");
    println!("live_read_chimes_list: {} chime(s) returned", chimes.len());
    for c in &chimes {
        println!("  - {} {:?} state={}", c.id, c.name, c.state);
    }
}

#[tokio::test]
async fn live_read_chimes_get() {
    let Some(client) = common::live_client() else {
        println!("(skipping live_read_chimes_get: UNIFI_PROTECT_HOST not set)");
        return;
    };
    let chimes = client
        .chimes()
        .list()
        .await
        .expect("chimes list call succeeded");
    let Some(first) = chimes.first() else {
        println!("(skipping live_read_chimes_get: NVR has no chimes)");
        return;
    };
    let fetched = client
        .chimes()
        .get(&first.id)
        .await
        .expect("chimes get call succeeded");
    println!(
        "live_read_chimes_get: round-tripped {} ({:?})",
        fetched.id, fetched.name
    );
    assert_eq!(fetched.id, first.id, "list+get should agree on id");
}

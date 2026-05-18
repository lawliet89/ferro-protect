#![forbid(unsafe_code)]
#![allow(
    clippy::pedantic,
    clippy::nursery,
    reason = "test files prioritise clarity over pedantic style"
)]

//! End-to-end test: spawn the `ferro-protect info` binary against a
//! `wiremock` server and assert the output.
//!
//! The key is passed via `UNIFI_PROTECT_API_KEY` (one of the three
//! resolver sources -- see `crates/ferro-protect-cli/src/api_key.rs`).

mod common;

use predicates::prelude::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FIXTURE: &str = r#"{"applicationVersion":"6.2.83"}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn info_prints_application_version() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/meta/info"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(FIXTURE)
                .insert_header("content-type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let base_url = server.uri();
    let assert = tokio::task::spawn_blocking(move || {
        common::isolated_cmd()
            .env("UNIFI_PROTECT_API_KEY", "test-key")
            .env_remove("UNIFI_PROTECT_API_KEY_FILE")
            .args(["--base-url", &base_url, "info"])
            .assert()
    })
    .await
    .expect("spawn_blocking");

    assert.success().stdout(predicate::str::contains("6.2.83"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn info_json_flag_emits_json() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/meta/info"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(FIXTURE)
                .insert_header("content-type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let base_url = server.uri();
    let assert = tokio::task::spawn_blocking(move || {
        common::isolated_cmd()
            .env("UNIFI_PROTECT_API_KEY", "test-key")
            .env_remove("UNIFI_PROTECT_API_KEY_FILE")
            .args(["--base-url", &base_url, "--json", "info"])
            .assert()
    })
    .await
    .expect("spawn_blocking");

    assert
        .success()
        .stdout(predicate::str::contains("\"applicationVersion\""))
        .stdout(predicate::str::contains("\"6.2.83\""));
}

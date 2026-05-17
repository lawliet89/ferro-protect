#![forbid(unsafe_code)]
#![allow(clippy::pedantic, clippy::nursery)]

//! Smoke tests for the logging wiring: confirm the level filter applies
//! and goes to stderr.

use assert_cmd::Command;
use predicates::prelude::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn run_cmd(base_url: &str, args: &[&str]) -> assert_cmd::assert::Assert {
    Command::cargo_bin("ferro-protect")
        .expect("binary built")
        .env("UNIFI_PROTECT_API_KEY", "test-key")
        .env_remove("UNIFI_PROTECT_API_KEY_FILE")
        .env_remove("UNIFI_PROTECT_HOST")
        .env_remove("UNIFI_PROTECT_LOG")
        .env_remove("RUST_LOG")
        .args(["--base-url", base_url])
        .args(args)
        .assert()
}

async fn start_info_mock() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/meta/info"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"applicationVersion":"6.2.83"}"#)
                .insert_header("content-type", "application/json"),
        )
        .mount(&server)
        .await;
    server
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn default_log_level_is_warn_no_info_emitted() {
    let server = start_info_mock().await;
    let url = server.uri();
    let assert = tokio::task::spawn_blocking(move || run_cmd(&url, &["info"]))
        .await
        .expect("spawn_blocking");

    // No --log-level flag, no envs => default "warn". Library emits an
    // info! when fetching application info; it should NOT appear.
    assert
        .success()
        .stderr(predicate::str::contains("fetched application info").not());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_info_flag_emits_info_to_stderr() {
    let server = start_info_mock().await;
    let url = server.uri();
    let assert =
        tokio::task::spawn_blocking(move || run_cmd(&url, &["--log-level", "info", "info"]))
            .await
            .expect("spawn_blocking");

    assert
        .success()
        .stderr(predicate::str::contains("INFO"))
        .stderr(predicate::str::contains("fetched application info"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn debug_flag_emits_request_breadcrumbs() {
    let server = start_info_mock().await;
    let url = server.uri();
    let assert =
        tokio::task::spawn_blocking(move || run_cmd(&url, &["--log-level", "debug", "info"]))
            .await
            .expect("spawn_blocking");

    assert
        .success()
        .stderr(predicate::str::contains("DEBUG"))
        .stderr(predicate::str::contains("GET /v1/meta/info"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unifi_protect_log_env_var_works() {
    let server = start_info_mock().await;
    let url = server.uri();
    let assert = tokio::task::spawn_blocking(move || {
        Command::cargo_bin("ferro-protect")
            .expect("binary built")
            .env("UNIFI_PROTECT_API_KEY", "test-key")
            .env_remove("UNIFI_PROTECT_API_KEY_FILE")
            .env_remove("UNIFI_PROTECT_HOST")
            .env("UNIFI_PROTECT_LOG", "info")
            .env_remove("RUST_LOG")
            .args(["--base-url", &url, "info"])
            .assert()
    })
    .await
    .expect("spawn_blocking");

    assert
        .success()
        .stderr(predicate::str::contains("fetched application info"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_flag_overrides_env_var() {
    let server = start_info_mock().await;
    let url = server.uri();
    let assert = tokio::task::spawn_blocking(move || {
        Command::cargo_bin("ferro-protect")
            .expect("binary built")
            .env("UNIFI_PROTECT_API_KEY", "test-key")
            .env_remove("UNIFI_PROTECT_API_KEY_FILE")
            .env_remove("UNIFI_PROTECT_HOST")
            // env says debug, flag says error -> flag wins, debug lines absent
            .env("UNIFI_PROTECT_LOG", "debug")
            .env_remove("RUST_LOG")
            .args(["--base-url", &url, "--log-level", "error", "info"])
            .assert()
    })
    .await
    .expect("spawn_blocking");

    assert
        .success()
        .stderr(predicate::str::contains("DEBUG").not())
        .stderr(predicate::str::contains("INFO").not());
}

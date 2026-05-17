#![forbid(unsafe_code)]
#![allow(clippy::pedantic, clippy::nursery)]

//! End-to-end test: spawn the `ferro-protect info` binary against a
//! `wiremock` server and assert the output.

use assert_cmd::Command;
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
        Command::cargo_bin("ferro-protect")
            .expect("binary built")
            .args(["--base-url", &base_url, "--api-key", "test-key", "info"])
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
        Command::cargo_bin("ferro-protect")
            .expect("binary built")
            .args([
                "--base-url",
                &base_url,
                "--api-key",
                "test-key",
                "--json",
                "info",
            ])
            .assert()
    })
    .await
    .expect("spawn_blocking");

    assert
        .success()
        .stdout(predicate::str::contains("\"applicationVersion\""))
        .stdout(predicate::str::contains("\"6.2.83\""));
}

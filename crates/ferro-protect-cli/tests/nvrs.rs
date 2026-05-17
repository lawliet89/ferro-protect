#![forbid(unsafe_code)]
#![allow(clippy::pedantic, clippy::nursery)]

//! End-to-end CLI tests for `ferro-protect nvrs …` against wiremock.

use assert_cmd::Command;
use predicates::prelude::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FIXTURE_NVR: &str = include_str!("../../ferro-protect/tests/fixtures/nvr_ok.json");

fn run_cmd(base_url: &str, args: &[&str]) -> assert_cmd::assert::Assert {
    Command::cargo_bin("ferro-protect")
        .expect("binary built")
        .env("UNIFI_PROTECT_API_KEY", "test-key")
        .env_remove("UNIFI_PROTECT_API_KEY_FILE")
        .env_remove("UNIFI_PROTECT_HOST")
        .args(["--base-url", base_url])
        .args(args)
        .assert()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nvrs_get_human() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/nvrs"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(FIXTURE_NVR)
                .insert_header("content-type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let base_url = server.uri();
    let assert = tokio::task::spawn_blocking(move || run_cmd(&base_url, &["nvrs", "get"]))
        .await
        .expect("spawn_blocking");

    assert
        .success()
        .stdout(predicate::str::contains("test-nvr-1"))
        .stdout(predicate::str::contains("Test NVR"));
}

#![forbid(unsafe_code)]
#![allow(
    clippy::pedantic,
    clippy::nursery,
    reason = "test files prioritise clarity over pedantic style"
)]

//! End-to-end CLI tests for `ferro-protect liveviews …` against wiremock.

mod common;

use predicates::prelude::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FIXTURE_EMPTY_LIST: &str = "[]";
const FIXTURE_NOT_FOUND: &str = r#"{"name":"notFound","error":"Liveview with id 'abc' not found"}"#;

fn run_cmd(base_url: &str, args: &[&str]) -> assert_cmd::assert::Assert {
    common::isolated_cmd()
        .env("UNIFI_PROTECT_API_KEY", "test-key")
        .env_remove("UNIFI_PROTECT_API_KEY_FILE")
        .env_remove("UNIFI_PROTECT_HOST")
        .args(["--base-url", base_url])
        .args(args)
        .assert()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn liveviews_list_empty_human() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/liveviews"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(FIXTURE_EMPTY_LIST)
                .insert_header("content-type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let base_url = server.uri();
    let assert = tokio::task::spawn_blocking(move || run_cmd(&base_url, &["liveviews", "list"]))
        .await
        .expect("spawn_blocking");

    assert
        .success()
        .stdout(predicate::str::contains("(no liveviews)"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn liveviews_get_404_reports_error_and_nonzero_exit() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/liveviews/abc"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_string(FIXTURE_NOT_FOUND)
                .insert_header("content-type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let base_url = server.uri();
    let assert =
        tokio::task::spawn_blocking(move || run_cmd(&base_url, &["liveviews", "get", "abc"]))
            .await
            .expect("spawn_blocking");

    assert
        .failure()
        .stderr(predicate::str::contains("notFound"))
        .stderr(predicate::str::contains("404"));
}

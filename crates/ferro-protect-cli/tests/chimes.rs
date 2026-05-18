#![forbid(unsafe_code)]
#![allow(
    clippy::pedantic,
    clippy::nursery,
    reason = "test files prioritise clarity over pedantic style"
)]

//! End-to-end CLI tests for `ferro-protect chimes …` against wiremock.

use assert_cmd::Command;
use predicates::prelude::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FIXTURE_NOT_FOUND: &str = r#"{"name":"notFound","error":"Chime with id 'abc' not found"}"#;

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
async fn chimes_list_empty_human() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/chimes"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("[]")
                .insert_header("content-type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let base_url = server.uri();
    let assert = tokio::task::spawn_blocking(move || run_cmd(&base_url, &["chimes", "list"]))
        .await
        .expect("spawn_blocking");

    assert
        .success()
        .stdout(predicate::str::contains("(no chimes)"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chimes_get_404_reports_error_and_nonzero_exit() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/chimes/abc"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_string(FIXTURE_NOT_FOUND)
                .insert_header("content-type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let base_url = server.uri();
    let assert = tokio::task::spawn_blocking(move || run_cmd(&base_url, &["chimes", "get", "abc"]))
        .await
        .expect("spawn_blocking");

    assert
        .failure()
        .stderr(predicate::str::contains("notFound"))
        .stderr(predicate::str::contains("404"));
}

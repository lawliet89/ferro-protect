#![forbid(unsafe_code)]
#![allow(
    clippy::pedantic,
    clippy::nursery,
    reason = "test files prioritise clarity over pedantic style"
)]

//! End-to-end CLI tests for `ferro-protect files …` against wiremock.

mod common;

use predicates::prelude::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FIXTURE_LIST_OK: &str = r#"[
  {"type":"animations","name":"a1.png","originalName":"welcome.png","path":"/srv/unifi-protect/storage/animations/a1.png","size":0,"id":"x1"},
  {"type":"animations","name":"a2.gif","path":"/srv/unifi-protect/storage/animations/a2.gif","size":0,"id":"x2"}
]"#;
const FIXTURE_FORBIDDEN: &str = r#"{"name":"forbidden","error":"Insufficient permissions"}"#;

fn run_cmd(base_url: &str, args: &[&str]) -> assert_cmd::assert::Assert {
    common::isolated_cmd()
        .env("UNIFI_PROTECT_API_KEY", "test-key")
        .args(["--base-url", base_url])
        .args(args)
        .assert()
}

async fn mount_list(server: &MockServer, status: u16, body: &'static str) {
    Mock::given(method("GET"))
        .and(path("/v1/files/animations"))
        .respond_with(
            ResponseTemplate::new(status)
                .set_body_string(body)
                .insert_header("content-type", "application/json"),
        )
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn files_list_human_renders_table() {
    let server = MockServer::start().await;
    mount_list(&server, 200, FIXTURE_LIST_OK).await;

    let base_url = server.uri();
    let assert =
        tokio::task::spawn_blocking(move || run_cmd(&base_url, &["files", "list", "animations"]))
            .await
            .expect("spawn_blocking");

    assert
        .success()
        .stdout(predicate::str::contains("ORIGINAL NAME"))
        .stdout(predicate::str::contains("a1.png"))
        .stdout(predicate::str::contains("welcome.png"))
        .stdout(predicate::str::contains("a2.gif"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn files_list_json_emits_spec_fields() {
    let server = MockServer::start().await;
    mount_list(&server, 200, FIXTURE_LIST_OK).await;

    let base_url = server.uri();
    let assert = tokio::task::spawn_blocking(move || {
        run_cmd(&base_url, &["--json", "files", "list", "animations"])
    })
    .await
    .expect("spawn_blocking");

    let out = assert.success().get_output().stdout.clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out).expect("stdout is JSON");
    let arr = parsed.as_array().expect("JSON array");
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["type"], "animations");
    assert_eq!(arr[0]["originalName"], "welcome.png");
    assert!(arr[1].get("originalName").is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn files_list_empty_human() {
    let server = MockServer::start().await;
    mount_list(&server, 200, "[]").await;

    let base_url = server.uri();
    let assert =
        tokio::task::spawn_blocking(move || run_cmd(&base_url, &["files", "list", "animations"]))
            .await
            .expect("spawn_blocking");

    assert
        .success()
        .stdout(predicate::str::contains("(no files)"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn files_list_unknown_type_is_rejected_before_any_request() {
    // The NVR answers an unknown fileType with `200 []`, so the CLI must
    // reject it locally. `expect(0)` fails the test if a request leaks.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .expect(0)
        .mount(&server)
        .await;

    let base_url = server.uri();
    let assert =
        tokio::task::spawn_blocking(move || run_cmd(&base_url, &["files", "list", "ringtones"]))
            .await
            .expect("spawn_blocking");

    assert
        .failure()
        .stderr(predicate::str::contains("invalid value 'ringtones'"))
        .stderr(predicate::str::contains("animations"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn files_list_403_reports_error_and_nonzero_exit() {
    let server = MockServer::start().await;
    mount_list(&server, 403, FIXTURE_FORBIDDEN).await;

    let base_url = server.uri();
    let assert =
        tokio::task::spawn_blocking(move || run_cmd(&base_url, &["files", "list", "animations"]))
            .await
            .expect("spawn_blocking");

    assert
        .failure()
        .stderr(predicate::str::contains("forbidden"))
        .stderr(predicate::str::contains("403"));
}

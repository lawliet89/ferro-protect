#![forbid(unsafe_code)]
#![allow(
    clippy::pedantic,
    clippy::nursery,
    reason = "test files prioritise clarity over pedantic style"
)]

//! End-to-end CLI tests for `ferro-protect cameras …` against wiremock.

use assert_cmd::Command;
use predicates::prelude::*;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FIXTURE_EMPTY_LIST: &str = "[]";
const FIXTURE_NOT_FOUND: &str = r#"{"name":"notFound","error":"Camera with id 'abc' not found"}"#;

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
async fn cameras_list_empty_human() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/cameras"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(FIXTURE_EMPTY_LIST)
                .insert_header("content-type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let base_url = server.uri();
    let assert = tokio::task::spawn_blocking(move || run_cmd(&base_url, &["cameras", "list"]))
        .await
        .expect("spawn_blocking");

    assert
        .success()
        .stdout(predicate::str::contains("(no cameras)"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cameras_list_empty_json() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/cameras"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(FIXTURE_EMPTY_LIST)
                .insert_header("content-type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let base_url = server.uri();
    let assert =
        tokio::task::spawn_blocking(move || run_cmd(&base_url, &["--json", "cameras", "list"]))
            .await
            .expect("spawn_blocking");

    assert.success().stdout(predicate::str::starts_with("[]"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cameras_get_404_reports_error_and_nonzero_exit() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/cameras/abc"))
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
        tokio::task::spawn_blocking(move || run_cmd(&base_url, &["cameras", "get", "abc"]))
            .await
            .expect("spawn_blocking");

    assert
        .failure()
        .stderr(predicate::str::contains("notFound"))
        .stderr(predicate::str::contains("404"));
}

/// Minimal 4-byte JPEG-shaped payload (SOI magic + a tail byte).
/// Real Protect responses are obviously larger; this is enough to
/// exercise the CLI's write path without smuggling a fixture file.
const FIXTURE_JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0];

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cameras_snapshot_writes_to_out_path() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/cameras/abc/snapshot"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(FIXTURE_JPEG)
                .insert_header("content-type", "image/jpeg"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    let out_path = tmp.path().to_path_buf();
    let base_url = server.uri();
    let out_arg = out_path.display().to_string();
    let assert = tokio::task::spawn_blocking(move || {
        run_cmd(
            &base_url,
            &["cameras", "snapshot", "abc", "--out", &out_arg],
        )
    })
    .await
    .expect("spawn_blocking");

    assert.success();
    let written = std::fs::read(&out_path).expect("read snapshot tempfile");
    assert_eq!(written, FIXTURE_JPEG, "snapshot bytes round-trip to --out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cameras_snapshot_forwards_channel_and_high_quality_flags() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/cameras/abc/snapshot"))
        .and(query_param("channel", "package"))
        .and(query_param("highQuality", "true"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(FIXTURE_JPEG)
                .insert_header("content-type", "image/jpeg"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    let out_path = tmp.path().to_path_buf();
    let base_url = server.uri();
    let out_arg = out_path.display().to_string();
    let assert = tokio::task::spawn_blocking(move || {
        run_cmd(
            &base_url,
            &[
                "cameras",
                "snapshot",
                "abc",
                "--channel",
                "package",
                "--high-quality",
                "--out",
                &out_arg,
            ],
        )
    })
    .await
    .expect("spawn_blocking");

    // wiremock's `expect(1)` + query_param matchers will fail the
    // mount-time `.verify()` on drop if the CLI did not send the
    // expected query string. `.success()` here is enough.
    assert.success();
}

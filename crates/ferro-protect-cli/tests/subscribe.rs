#![forbid(unsafe_code)]
#![allow(
    clippy::pedantic,
    clippy::nursery,
    reason = "test files prioritise clarity over pedantic style"
)]
#![allow(
    clippy::result_large_err,
    reason = "tungstenite's handshake callback signature fixes the error type"
)]

//! End-to-end CLI tests for `ferro-protect subscribe …`. wiremock
//! cannot complete a WebSocket upgrade, so the streaming tests run
//! against a one-connection `tokio-tungstenite` server; the
//! refused-upgrade test still uses wiremock.

mod common;

use futures_util::{SinkExt, StreamExt};
use predicates::prelude::*;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const DEVICE_UPDATE: &str = r#"{"item":{"cameraIds":["65b75b9103132403e40004bf"],"id":"65b8a9fd02592403e4001e64","modelKey":"chime"},"type":"update"}"#;
const DEVICE_REMOVE: &str =
    r#"{"item":{"id":"672094f900e26303e800062a","modelKey":"light"},"type":"remove"}"#;
/// From a live 7.3.70 capture (see the library's
/// `subscribe_events.ndjson` fixture).
const EVENT_ADD: &str = r#"{"type":"add","item":{"device":"663e1bc6034d4803e4001e03","id":"50b781f6-5ddf-4d27-a549-351af86a7655","modelKey":"event","smartDetectTypes":["face","person"],"start":1791429589327.0,"type":"smartDetectZone"}}"#;
const RATE_LIMITED: &str =
    r#"{"error":"Too many requests","limit":10,"name":"TOO_MANY_REQUESTS_ERROR","windowMs":1000}"#;

/// Accept one WebSocket connection on `expected_path`, send `frames`,
/// then close. Returns the base URL for `--base-url`.
async fn ws_server(expected_path: &'static str, frames: &[&str]) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let base_url = format!("http://{}", listener.local_addr().expect("addr"));
    let frames: Vec<String> = frames.iter().map(|f| f.to_string()).collect();
    tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.expect("accept");
        let callback = |req: &tokio_tungstenite::tungstenite::handshake::server::Request, resp| {
            assert_eq!(req.uri().path(), expected_path);
            Ok(resp)
        };
        let mut ws = tokio_tungstenite::accept_hdr_async(tcp, callback)
            .await
            .expect("handshake");
        for frame in frames {
            ws.send(Message::text(frame)).await.expect("send");
        }
        ws.close(None).await.expect("close");
        while ws.next().await.is_some() {}
    });
    base_url
}

async fn run_cmd(base_url: String, args: &'static [&'static str]) -> assert_cmd::assert::Assert {
    tokio::task::spawn_blocking(move || {
        common::isolated_cmd()
            .env("UNIFI_PROTECT_API_KEY", "test-key")
            .args(["--base-url", &base_url])
            .args(args)
            .timeout(std::time::Duration::from_secs(30))
            .assert()
    })
    .await
    .expect("spawn_blocking")
}

fn stdout_lines(assert: &assert_cmd::assert::Assert) -> Vec<serde_json::Value> {
    String::from_utf8(assert.get_output().stdout.clone())
        .expect("utf-8 stdout")
        .lines()
        .map(|l| serde_json::from_str(l).expect("each stdout line is JSON"))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscribe_devices_streams_ndjson_until_server_closes() {
    let base_url = ws_server("/v1/subscribe/devices", &[DEVICE_UPDATE, DEVICE_REMOVE]).await;
    let assert = run_cmd(base_url, &["subscribe", "devices"]).await;

    let lines = stdout_lines(&assert);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["type"], "update");
    assert_eq!(lines[0]["item"]["modelKey"], "chime");
    assert_eq!(lines[0]["item"]["cameraIds"][0], "65b75b9103132403e40004bf");
    assert_eq!(lines[1]["type"], "remove");
    assert_eq!(lines[1]["item"]["id"], "672094f900e26303e800062a");
    // A server-side close is abnormal for a long-running stream, so
    // scripts get a non-zero exit to react to.
    assert
        .failure()
        .stderr(predicate::str::contains("closed the subscription"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscribe_events_streams_ndjson_and_ignores_json_flag() {
    let base_url = ws_server("/v1/subscribe/events", &[EVENT_ADD]).await;
    let assert = run_cmd(base_url, &["--json", "subscribe", "events"]).await;

    let lines = stdout_lines(&assert);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["type"], "add");
    assert_eq!(lines[0]["item"]["type"], "smartDetectZone");
    assert_eq!(lines[0]["item"]["device"], "663e1bc6034d4803e4001e03");
    assert_eq!(
        lines[0]["item"]["smartDetectTypes"],
        serde_json::json!(["face", "person"])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscribe_skips_undecodable_messages_with_a_warning() {
    let base_url = ws_server(
        "/v1/subscribe/devices",
        &[r#"{"type":"teleport","item":{}}"#, DEVICE_REMOVE],
    )
    .await;
    let assert = run_cmd(base_url, &["subscribe", "devices"]).await;

    let lines = stdout_lines(&assert);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["type"], "remove");
    assert.stderr(predicate::str::contains(
        "did not match the expected schema",
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscribe_reports_in_band_rejection() {
    let base_url = ws_server("/v1/subscribe/events", &[RATE_LIMITED]).await;
    let assert = run_cmd(base_url, &["subscribe", "events"]).await;

    assert
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("TOO_MANY_REQUESTS_ERROR"))
        .stderr(predicate::str::contains("Too many requests"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subscribe_refused_upgrade_reports_error_and_nonzero_exit() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/subscribe/devices"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_string(r#"{"name":"unauthorized","error":"Unauthorized"}"#)
                .insert_header("content-type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let assert = run_cmd(server.uri(), &["subscribe", "devices"]).await;

    assert
        .failure()
        .stderr(predicate::str::contains("401"))
        .stderr(predicate::str::contains("unauthorized"));
}

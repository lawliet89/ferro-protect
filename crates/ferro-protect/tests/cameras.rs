#![forbid(unsafe_code)]
#![allow(
    clippy::pedantic,
    clippy::nursery,
    reason = "test files prioritise clarity over pedantic style"
)]

//! `client.cameras().list() / .get(id)` against a mock NVR.
//!
//! Happy paths against a real device are covered by the live tests in
//! `tests/live.rs`; here we focus on routing (URL + auth header) and
//! error mapping.

use std::num::NonZeroU64;

use ferro_protect::models::{CameraId, ChannelQuality, SnapshotChannel, SnapshotOptions};
use ferro_protect::{Error, ProtectClient};
use secrecy::SecretString;
use wiremock::matchers::{body_bytes, body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FIXTURE_EMPTY_LIST: &str = include_str!("fixtures/cameras_list_empty.json");
const FIXTURE_NOT_FOUND: &str = include_str!("fixtures/camera_not_found.json");

async fn client_for(server: &MockServer) -> ProtectClient {
    ProtectClient::builder()
        .base_url(server.uri())
        .api_key(SecretString::from("test-key".to_string()))
        .build()
        .expect("client builds")
}

#[tokio::test]
async fn list_returns_empty_vec_when_no_cameras() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/cameras"))
        .and(header("x-api-key", "test-key"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(FIXTURE_EMPTY_LIST)
                .insert_header("content-type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let cameras = client.cameras().list().await.expect("list call succeeds");
    assert!(cameras.is_empty());
}

#[tokio::test]
async fn get_nonexistent_camera_maps_to_404_api_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/cameras/abc123"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_string(FIXTURE_NOT_FOUND)
                .insert_header("content-type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let id = CameraId::from("abc123".to_string());
    let err = client
        .cameras()
        .get(&id)
        .await
        .expect_err("404 should error");
    match err {
        Error::Api {
            status,
            code,
            message,
        } => {
            assert_eq!(status, 404);
            assert_eq!(code, "notFound");
            assert!(
                message.contains("abc123"),
                "expected message to include id, got: {message}"
            );
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}

/// 4-byte JPEG-shaped payload (SOI marker plus a tail byte). Real
/// snapshot bodies are much larger; this is enough to exercise the
/// library's binary read path against the mock without smuggling a
/// fixture file.
const FIXTURE_JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0];

#[tokio::test]
async fn snapshot_with_forwards_channel_and_high_quality_query_params() {
    // Pins the exact query string `snapshot_with` builds. The test
    // would catch a regression to e.g. `?channel=Package` (Display
    // capitalisation drift) or `?highQuality=1` (boolean rendering).
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/cameras/abc/snapshot"))
        .and(query_param("channel", "package"))
        .and(query_param("highQuality", "true"))
        .and(header("x-api-key", "test-key"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(FIXTURE_JPEG)
                .insert_header("content-type", "image/jpeg"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let id = CameraId::from("abc".to_string());
    let bytes = client
        .cameras()
        .snapshot_with(
            &id,
            &SnapshotOptions {
                channel: Some(SnapshotChannel::Package),
                high_quality: true,
            },
        )
        .await
        .expect("snapshot_with succeeds");
    assert_eq!(bytes.as_ref(), FIXTURE_JPEG);
}

#[tokio::test]
async fn snapshot_default_options_sends_bare_path() {
    // Default `SnapshotOptions` must produce a path with no query
    // string at all (not `?channel=Main` or `?highQuality=false`).
    // A regression that always appended `highQuality=false` would
    // still work against most servers, but the live test would no
    // longer be exercising the bare-path case the spec documents as
    // the default.
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

    let client = client_for(&server).await;
    let id = CameraId::from("abc".to_string());
    let bytes = client.cameras().snapshot(&id).await.expect("snapshot ok");
    assert_eq!(bytes.as_ref(), FIXTURE_JPEG);

    // wiremock's `expect(1)` already enforces "exactly one matching
    // request"; received_requests lets us additionally pin that the
    // URL had no query component.
    let received = server.received_requests().await.expect("recorded requests");
    let req = received.first().expect("one request recorded");
    assert!(
        req.url.query().is_none(),
        "expected no query string for default snapshot, got {:?}",
        req.url.query()
    );
}

#[tokio::test]
async fn rtsps_stream_posts_request_body_and_orders_response() {
    // Caller requests `[low, high]`. Server response orders them as
    // `{ high, low }`. The library must return them in request order
    // (low first, then high), and must POST a body of the exact
    // shape `{ "qualities": ["low", "high"] }`.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/cameras/abc/rtsps-stream"))
        .and(header("x-api-key", "test-key"))
        .and(body_json(
            serde_json::json!({ "qualities": ["low", "high"] }),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(
                    r#"{"high":"rtsps://nvr/cam-abc-high","low":"rtsps://nvr/cam-abc-low"}"#,
                )
                .insert_header("content-type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let id = CameraId::from("abc".to_string());
    let streams = client
        .cameras()
        .rtsps_stream(&id, &[ChannelQuality::Low, ChannelQuality::High])
        .await
        .expect("rtsps_stream call succeeds");
    assert_eq!(streams.len(), 2);
    assert_eq!(streams[0].quality, ChannelQuality::Low);
    assert_eq!(streams[0].url, "rtsps://nvr/cam-abc-low");
    assert_eq!(streams[1].quality, ChannelQuality::High);
    assert_eq!(streams[1].url, "rtsps://nvr/cam-abc-high");
}

#[tokio::test]
async fn rtsps_stream_drops_qualities_not_returned_by_server() {
    // Caller requests `[high, low]`; server only returns `high`.
    // The library drops the missing entry rather than surfacing
    // `Option<String>` to callers.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/cameras/abc/rtsps-stream"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"high":"rtsps://nvr/cam-abc-high"}"#)
                .insert_header("content-type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let id = CameraId::from("abc".to_string());
    let streams = client
        .cameras()
        .rtsps_stream(&id, &[ChannelQuality::High, ChannelQuality::Low])
        .await
        .expect("rtsps_stream call succeeds");
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].quality, ChannelQuality::High);
}

#[tokio::test]
async fn talkback_session_posts_empty_body_and_maps_response() {
    // `body_bytes(b"")` is the load-bearing assertion: it pins
    // `post_empty_json`'s no-body contract. A regression to
    // `post_json(&())` would emit a 4-byte `null` body with a JSON
    // content-type and would fail this matcher (mirroring what the
    // real talkback endpoint does — it rejects `null` request
    // bodies).
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/cameras/abc/talkback-session"))
        .and(header("x-api-key", "test-key"))
        .and(body_bytes(b"".as_slice()))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(
                    r#"{"bitsPerSample":16,"codec":"aac","samplingRate":16000,"url":"wss://nvr/talkback/abc"}"#,
                )
                .insert_header("content-type", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let id = CameraId::from("abc".to_string());
    let session = client
        .cameras()
        .talkback_session(&id)
        .await
        .expect("talkback_session call succeeds");
    assert_eq!(session.bits_per_sample, NonZeroU64::new(16).unwrap());
    assert_eq!(session.codec, "aac");
    assert_eq!(session.sampling_rate, NonZeroU64::new(16000).unwrap());
    assert_eq!(session.url, "wss://nvr/talkback/abc");
}

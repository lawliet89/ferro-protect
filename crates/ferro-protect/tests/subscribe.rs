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

//! `client.subscribe().{devices,events}()` against a local WebSocket
//! server. wiremock cannot complete a WebSocket upgrade, so the happy
//! paths run against a one-connection `tokio-tungstenite` server; the
//! refused-upgrade path still uses wiremock.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ferro_protect::models::{
    Device, DeviceAdded, DeviceMessage, DevicePartialWithReference, DeviceRemoved, DeviceState,
    DeviceUpdated, Event, EventMessage,
};
use ferro_protect::{Error, ProtectClient};
use futures_util::{SinkExt, StreamExt};
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Device messages in the live wire shape. The first line is a
/// verbatim capture from a 7.3.70 NVR; the rest follow the spec
/// (single update, single remove, bulk remove with an `id` array).
const FIXTURE_DEVICES: &str = include_str!("fixtures/subscribe_devices.ndjson");
/// Live capture of `ferro-protect subscribe events` on a 7.3.70 NVR:
/// two overlapping person detections, each an `add` followed by
/// `update`s, some repeated verbatim, the later ones carrying `end`.
/// The CLI re-serialised these, which is why timestamps carry `.0`.
const FIXTURE_EVENTS: &str = include_str!("fixtures/subscribe_events.ndjson");
const FIXTURE_NVR: &str = include_str!("fixtures/nvr_ok.json");

/// What the server saw during the handshake and after the frames.
#[derive(Debug, Default)]
struct Seen {
    path: String,
    api_key: Option<String>,
    client_sent_close: bool,
    pings: usize,
}

enum Ending {
    /// Send a close frame, then wait for the client to finish.
    Close,
    /// Wait for the client to close first.
    AwaitClient,
    /// Drop the TCP connection without a close handshake.
    Drop,
    /// Keep reading (so pings get answered) for this long, counting
    /// pings, then close.
    CountPings(Duration),
    /// Hold the connection without reading, so pings go unanswered.
    Stall(Duration),
}

/// Accept one WebSocket connection, send `frames` as text messages,
/// then end the connection as `ending` says. Returns the base URL to
/// point the client at and a handle that resolves to what the server
/// observed.
async fn ws_server(frames: Vec<String>, ending: Ending) -> (String, JoinHandle<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let base_url = format!("http://{}", listener.local_addr().expect("addr"));
    let handle = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.expect("accept");
        let seen = Arc::new(Mutex::new(Seen::default()));
        let seen_cb = Arc::clone(&seen);
        let callback = move |req: &Request, resp: Response| {
            let mut s = seen_cb.lock().unwrap();
            s.path = req.uri().path().to_string();
            s.api_key = req
                .headers()
                .get("x-api-key")
                .map(|v| v.to_str().unwrap().to_string());
            Ok(resp)
        };
        let mut ws = tokio_tungstenite::accept_hdr_async(tcp, callback)
            .await
            .expect("handshake");
        for frame in frames {
            ws.send(Message::text(frame)).await.expect("send");
        }
        match ending {
            Ending::Drop => {
                drop(ws);
            }
            Ending::CountPings(duration) => {
                let count = async {
                    while let Some(msg) = ws.next().await {
                        if matches!(msg, Ok(Message::Ping(_))) {
                            seen.lock().unwrap().pings += 1;
                        }
                    }
                };
                let _ = tokio::time::timeout(duration, count).await;
                ws.close(None).await.expect("close");
                while ws.next().await.is_some() {}
            }
            Ending::Stall(duration) => {
                tokio::time::sleep(duration).await;
                drop(ws);
            }
            Ending::Close | Ending::AwaitClient => {
                if matches!(ending, Ending::Close) {
                    ws.close(None).await.expect("close");
                }
                while let Some(msg) = ws.next().await {
                    if matches!(msg, Ok(Message::Close(_))) {
                        seen.lock().unwrap().client_sent_close = true;
                    }
                }
            }
        }
        Arc::try_unwrap(seen).unwrap().into_inner().unwrap()
    });
    (base_url, handle)
}

fn client_for(base_url: &str) -> ProtectClient {
    client_with_keepalive(base_url, Some(Duration::from_secs(30)))
}

fn client_with_keepalive(base_url: &str, keepalive: Option<Duration>) -> ProtectClient {
    ProtectClient::builder()
        .base_url(base_url)
        .api_key(SecretString::from("test-key".to_string()))
        .subscription_keepalive(keepalive)
        .build()
        .expect("client builds")
}

fn lines(fixture: &str) -> Vec<String> {
    fixture.lines().map(str::to_string).collect()
}

#[tokio::test]
async fn devices_stream_decodes_each_message_then_ends_on_close() {
    let nvr: serde_json::Value = serde_json::from_str(FIXTURE_NVR).unwrap();
    let mut frames = vec![serde_json::json!({ "type": "add", "item": nvr }).to_string()];
    frames.extend(lines(FIXTURE_DEVICES));
    let (base_url, server) = ws_server(frames, Ending::Close).await;

    let client = client_for(&base_url);
    let mut sub = client
        .subscribe()
        .devices()
        .await
        .expect("handshake succeeds");
    let mut messages = Vec::new();
    while let Some(msg) = sub.next().await {
        messages.push(msg.expect("message decodes"));
    }

    assert_eq!(messages.len(), 5);
    match &messages[0] {
        DeviceMessage::Add {
            item: DeviceAdded::One(Device::Nvr(nvr)),
        } => assert_eq!(nvr.id.as_str(), "test-nvr-1"),
        other => panic!("expected nvr add, got {other:?}"),
    }
    match &messages[1] {
        DeviceMessage::Update {
            item: DeviceUpdated::One(DevicePartialWithReference::ChimePartialWithReference(chime)),
        } => {
            assert_eq!(chime.id.as_str(), "65b8a9fd02592403e4001e64");
            assert_eq!(chime.camera_ids.len(), 1);
            assert!(
                chime.name.is_none(),
                "partial update carries only changed fields"
            );
        }
        other => panic!("expected chime update, got {other:?}"),
    }
    match &messages[2] {
        DeviceMessage::Update {
            item: DeviceUpdated::One(DevicePartialWithReference::CameraPartialWithReference(camera)),
        } => assert_eq!(camera.state, Some(DeviceState::Disconnected)),
        other => panic!("expected camera update, got {other:?}"),
    }
    assert!(matches!(
        &messages[3],
        DeviceMessage::Remove {
            item: DeviceRemoved::One(_)
        }
    ));
    assert!(matches!(
        &messages[4],
        DeviceMessage::Remove {
            item: DeviceRemoved::Bulk(_)
        }
    ));

    let seen = server.await.expect("server task");
    assert_eq!(seen.path, "/v1/subscribe/devices");
    assert_eq!(seen.api_key.as_deref(), Some("test-key"));
}

#[tokio::test]
async fn events_stream_decodes_add_and_update() {
    let (base_url, server) = ws_server(lines(FIXTURE_EVENTS), Ending::Close).await;

    let client = client_for(&base_url);
    let sub = client
        .subscribe()
        .events()
        .await
        .expect("handshake succeeds");
    let messages: Vec<EventMessage> = sub.map(|m| m.expect("message decodes")).collect().await;

    assert_eq!(messages.len(), 10);
    let adds = messages
        .iter()
        .filter(|m| matches!(m, EventMessage::Add { .. }))
        .count();
    assert_eq!(adds, 2);
    match &messages[0] {
        EventMessage::Add {
            item: Event::CameraSmartDetectZoneEvent(zone),
        } => {
            assert_eq!(zone.id.as_str(), "dc7b6289-07d3-41ff-b3c7-cd77df45b30f");
            assert_eq!(zone.device.as_str(), "68d21ee002e76203e413b0dc");
            assert_eq!(zone.start, 1_791_429_586_995.0);
            assert!(zone.end.is_none(), "an event starts without an end");
        }
        other => panic!("expected smartDetectZone add, got {other:?}"),
    }
    match &messages[2] {
        EventMessage::Add {
            item: Event::CameraSmartDetectZoneEvent(zone),
        } => {
            let types = serde_json::to_value(&zone.smart_detect_types).unwrap();
            assert_eq!(types, serde_json::json!(["face", "person"]));
        }
        other => panic!("expected smartDetectZone add, got {other:?}"),
    }
    match &messages[4] {
        EventMessage::Update {
            item: Event::CameraSmartDetectZoneEvent(zone),
        } => {
            assert_eq!(zone.id.as_str(), "dc7b6289-07d3-41ff-b3c7-cd77df45b30f");
            assert_eq!(zone.end, Some(1_791_429_599_300.0));
        }
        other => panic!("expected smartDetectZone update, got {other:?}"),
    }

    assert_eq!(server.await.unwrap().path, "/v1/subscribe/events");
}

#[tokio::test]
async fn undecodable_message_yields_error_and_stream_continues() {
    let frames = vec![
        r#"{"type":"teleport","item":{}}"#.to_string(),
        lines(FIXTURE_DEVICES).remove(0),
    ];
    let (base_url, _server) = ws_server(frames, Ending::Close).await;

    let client = client_for(&base_url);
    let mut sub = client.subscribe().devices().await.unwrap();

    let first = sub.next().await.expect("first item");
    assert!(matches!(first, Err(Error::Json(_))), "got {first:?}");
    let second = sub.next().await.expect("second item");
    assert!(matches!(second, Ok(DeviceMessage::Update { .. })));
    assert!(sub.next().await.is_none());
}

#[tokio::test]
async fn dropped_connection_yields_one_websocket_error_then_ends() {
    let (base_url, _server) = ws_server(Vec::new(), Ending::Drop).await;

    let client = client_for(&base_url);
    let mut sub = client.subscribe().devices().await.unwrap();

    let item = sub.next().await.expect("an error item");
    assert!(matches!(item, Err(Error::WebSocket(_))), "got {item:?}");
    assert!(sub.next().await.is_none());
}

#[tokio::test]
async fn close_sends_a_close_frame() {
    let (base_url, server) = ws_server(Vec::new(), Ending::AwaitClient).await;

    let client = client_for(&base_url);
    let sub = client.subscribe().devices().await.unwrap();
    sub.close().await.expect("close succeeds");

    assert!(server.await.unwrap().client_sent_close);
}

#[tokio::test]
async fn refused_upgrade_maps_to_api_error() {
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

    let client = client_for(&server.uri());
    let err = match client.subscribe().devices().await {
        Ok(_) => panic!("401 should refuse the subscription"),
        Err(e) => e,
    };
    match err {
        Error::Api { status, code, .. } => {
            assert_eq!(status, 401);
            assert_eq!(code, "unauthorized");
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}

#[tokio::test]
async fn in_band_error_frame_maps_to_subscription_rejected() {
    // Verbatim from a 7.3.70 NVR whose rate limit was exceeded: the
    // upgrade succeeds, then this frame arrives and the server closes.
    let frame = r#"{"error":"Too many requests","limit":10,"name":"TOO_MANY_REQUESTS_ERROR","windowMs":1000}"#;
    let (base_url, _server) = ws_server(vec![frame.to_string()], Ending::Close).await;

    let client = client_for(&base_url);
    let mut sub = client.subscribe().events().await.expect("upgrade succeeds");

    match sub.next().await.expect("an error item") {
        Err(Error::SubscriptionRejected { code, message }) => {
            assert_eq!(code, "TOO_MANY_REQUESTS_ERROR");
            assert_eq!(message, "Too many requests");
        }
        other => panic!("expected SubscriptionRejected, got {other:?}"),
    }
    assert!(sub.next().await.is_none());
    sub.close()
        .await
        .expect("closing an already-closed subscription is a no-op");
}

#[tokio::test]
async fn keepalive_pings_while_idle() {
    let (base_url, server) =
        ws_server(Vec::new(), Ending::CountPings(Duration::from_millis(400))).await;

    let client = client_with_keepalive(&base_url, Some(Duration::from_millis(50)));
    let mut sub = client.subscribe().devices().await.unwrap();
    // The server answers every ping, so the subscription stays healthy
    // until the server closes after 400ms.
    let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("server closes within 5s");
    assert!(item.is_none(), "expected a clean end, got {item:?}");

    let pings = server.await.unwrap().pings;
    assert!(pings >= 4, "expected a ping every 50ms, saw {pings}");
}

#[tokio::test]
async fn keepalive_ends_a_silent_connection() {
    let (base_url, _server) = ws_server(Vec::new(), Ending::Stall(Duration::from_secs(5))).await;

    let client = client_with_keepalive(&base_url, Some(Duration::from_millis(50)));
    let mut sub = client.subscribe().devices().await.unwrap();
    let item = tokio::time::timeout(Duration::from_secs(2), sub.next())
        .await
        .expect("keepalive gives up well before the server does");
    match item {
        Some(Err(Error::WebSocket(message))) => assert!(message.contains("keepalive")),
        other => panic!("expected a keepalive error, got {other:?}"),
    }
    assert!(sub.next().await.is_none());
}

#[tokio::test]
async fn keepalive_disabled_waits_on_a_silent_connection() {
    let (base_url, _server) = ws_server(Vec::new(), Ending::Stall(Duration::from_secs(5))).await;

    let client = client_with_keepalive(&base_url, None);
    let mut sub = client.subscribe().devices().await.unwrap();
    let waited = tokio::time::timeout(Duration::from_millis(300), sub.next()).await;
    assert!(waited.is_err(), "no keepalive means no error on silence");
}

#[test]
fn zero_keepalive_is_rejected_at_build_time() {
    let result = ProtectClient::builder()
        .base_url("http://127.0.0.1:1")
        .api_key(SecretString::from("test-key".to_string()))
        .subscription_keepalive(Some(Duration::ZERO))
        .build();
    assert!(matches!(result, Err(Error::Other(_))));
}

#![forbid(unsafe_code)]
#![cfg(feature = "reconnect")]
#![allow(
    clippy::pedantic,
    clippy::nursery,
    reason = "test files prioritise clarity over pedantic style"
)]

//! `client.subscribe().{devices,events}_reconnecting()` against a local
//! server that plays one script per connection.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use ferro_protect::models::DeviceMessage;
use ferro_protect::{Error, ProtectClient, ReconnectConfig};
use futures_util::{SinkExt, StreamExt};
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

const REMOVE_A: &str =
    r#"{"item":{"id":"672094f900e26303e800062a","modelKey":"light"},"type":"remove"}"#;
const RATE_LIMITED: &str =
    r#"{"error":"Too many requests","limit":10,"name":"TOO_MANY_REQUESTS_ERROR","windowMs":1000}"#;
const REMOVE_B: &str =
    r#"{"item":{"id":"672094f900e26303e800062b","modelKey":"light"},"type":"remove"}"#;

/// What one accepted connection does after the handshake.
enum Script {
    /// Send the frame, then drop the TCP connection.
    SendThenDrop(&'static str),
    /// Send the frame, then close cleanly.
    SendThenClose(&'static str),
}

/// What happens to connections after the scripts run out.
enum Afterwards {
    /// Stop listening, so connections are refused.
    Refuse,
    /// Answer every request with a plain HTTP 401.
    Unauthorized,
}

/// Serve `scripts` in order, one per connection. Returns the base URL
/// and a counter of connections handled after the scripts ran out.
async fn server(scripts: Vec<Script>, afterwards: Afterwards) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let base_url = format!("http://{}", listener.local_addr().expect("addr"));
    let extra = Arc::new(AtomicUsize::new(0));
    let extra_task = Arc::clone(&extra);
    tokio::spawn(async move {
        for script in scripts {
            let (tcp, _) = listener.accept().await.expect("accept");
            let mut ws = tokio_tungstenite::accept_async(tcp)
                .await
                .expect("handshake");
            match script {
                Script::SendThenDrop(frame) => {
                    ws.send(Message::text(frame)).await.expect("send");
                    drop(ws);
                }
                Script::SendThenClose(frame) => {
                    ws.send(Message::text(frame)).await.expect("send");
                    ws.close(None).await.expect("close");
                    while ws.next().await.is_some() {}
                }
            }
        }
        match afterwards {
            Afterwards::Refuse => drop(listener),
            Afterwards::Unauthorized => loop {
                let (mut tcp, _) = listener.accept().await.expect("accept");
                extra_task.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0_u8; 4096];
                let _ = tcp.read(&mut buf).await;
                let body = r#"{"name":"unauthorized","error":"Unauthorized"}"#;
                let response = format!(
                    "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                tcp.write_all(response.as_bytes()).await.expect("write");
            },
        }
    });
    (base_url, extra)
}

fn client_for(base_url: &str) -> ProtectClient {
    ProtectClient::builder()
        .base_url(base_url)
        .api_key(SecretString::from("test-key".to_string()))
        .build()
        .expect("client builds")
}

fn fast_config(max_attempts: Option<u32>) -> ReconnectConfig {
    ReconnectConfig {
        initial_backoff: Duration::from_millis(10),
        max_backoff: Duration::from_millis(40),
        max_attempts: max_attempts.and_then(NonZeroU32::new),
    }
}

fn removed_id(item: Option<ferro_protect::Result<DeviceMessage>>) -> String {
    match item {
        Some(Ok(DeviceMessage::Remove { item })) => serde_json::to_value(item).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string(),
        other => panic!("expected a remove message, got {other:?}"),
    }
}

async fn next(
    sub: &mut ferro_protect::ReconnectingSubscription<DeviceMessage>,
) -> Option<ferro_protect::Result<DeviceMessage>> {
    tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("item within 5s")
}

#[tokio::test]
async fn reconnects_after_drops_and_reports_each_gap() {
    let (base_url, _) = server(
        vec![
            Script::SendThenDrop(REMOVE_A),
            Script::SendThenClose(REMOVE_B),
        ],
        Afterwards::Refuse,
    )
    .await;

    let client = client_for(&base_url);
    let mut sub = client
        .subscribe()
        .devices_reconnecting(fast_config(Some(2)))
        .await
        .expect("first connection succeeds");

    assert_eq!(removed_id(next(&mut sub).await), "672094f900e26303e800062a");
    // The drop is reported, then the stream reconnects.
    assert!(matches!(
        next(&mut sub).await,
        Some(Err(Error::WebSocket(_)))
    ));
    assert_eq!(removed_id(next(&mut sub).await), "672094f900e26303e800062b");
    // A clean server close is reported as a gap too.
    match next(&mut sub).await {
        Some(Err(Error::WebSocket(message))) => assert!(message.contains("closed")),
        other => panic!("expected a close report, got {other:?}"),
    }
    // Then the listener is gone: two refused attempts, the last error
    // is yielded, and the stream ends.
    assert!(matches!(next(&mut sub).await, Some(Err(Error::Http(_)))));
    assert!(next(&mut sub).await.is_none());
}

#[tokio::test]
async fn permanent_error_stops_without_retrying() {
    let (base_url, unauthorized_hits) = server(
        vec![Script::SendThenClose(REMOVE_A)],
        Afterwards::Unauthorized,
    )
    .await;

    let client = client_for(&base_url);
    let mut sub = client
        .subscribe()
        .devices_reconnecting(fast_config(None))
        .await
        .expect("first connection succeeds");

    assert_eq!(removed_id(sub.next().await), "672094f900e26303e800062a");
    assert!(matches!(sub.next().await, Some(Err(Error::WebSocket(_)))));
    match sub.next().await {
        Some(Err(Error::Api { status, .. })) => assert_eq!(status, 401),
        other => panic!("expected the 401, got {other:?}"),
    }
    assert!(sub.next().await.is_none());
    assert_eq!(unauthorized_hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn first_connection_failure_is_returned_directly() {
    let (base_url, _) = server(Vec::new(), Afterwards::Unauthorized).await;

    let client = client_for(&base_url);
    let result = client
        .subscribe()
        .events_reconnecting(ReconnectConfig::default())
        .await;
    assert!(matches!(result, Err(Error::Api { status: 401, .. })));
}

#[tokio::test]
async fn max_attempts_counts_reconnects_rejected_in_band() {
    // After the first drop, every reconnect upgrades and is then
    // rejected in-band. Those count as failed attempts, so with
    // max_attempts = 2 the stream ends after the second rejection
    // instead of making a third (here: refused) attempt.
    let (base_url, _) = server(
        vec![
            Script::SendThenDrop(REMOVE_A),
            Script::SendThenClose(RATE_LIMITED),
            Script::SendThenClose(RATE_LIMITED),
        ],
        Afterwards::Refuse,
    )
    .await;

    let client = client_for(&base_url);
    let mut sub = client
        .subscribe()
        .devices_reconnecting(fast_config(Some(2)))
        .await
        .expect("first connection succeeds");

    assert_eq!(removed_id(next(&mut sub).await), "672094f900e26303e800062a");
    assert!(matches!(
        next(&mut sub).await,
        Some(Err(Error::WebSocket(_)))
    ));
    for _ in 0..2 {
        match next(&mut sub).await {
            Some(Err(Error::SubscriptionRejected { code, .. })) => {
                assert_eq!(code, "TOO_MANY_REQUESTS_ERROR");
            }
            other => panic!("expected an in-band rejection, got {other:?}"),
        }
    }
    assert!(next(&mut sub).await.is_none());
}

//! WebSocket subscriptions (`/v1/subscribe/*`).
//!
//! # Handshake
//!
//! The upgrade request goes through the same `reqwest` client as every
//! other call, then the upgraded connection is handed to
//! `tokio-tungstenite` with [`WebSocketStream::from_raw_socket`]. That
//! way the WebSocket inherits the client's TLS mode (native, pinned,
//! or accept-invalid), `X-API-Key` default header, and rate limiter
//! without a second TLS stack to configure. The client is built
//! `http1_only` because an HTTP/2 connection cannot be upgraded.
//!
//! # Framing
//!
//! Protect sends one JSON document per text frame. Binary frames are
//! not expected but are decoded the same way rather than dropped.
//! Ping/pong is answered by tungstenite and never surfaces here.

use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll, ready};
use std::time::Duration;

use futures_util::{Stream, StreamExt};
use log::{debug, info, warn};
use reqwest::StatusCode;
use reqwest::header::{
    CONNECTION, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_VERSION, UPGRADE,
};
use serde::de::DeserializeOwned;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

use crate::client::ProtectClient;
use crate::error::{Error, Result};
use crate::models::{DeviceMessage, EventMessage};

/// How long [`Subscription::close`] waits for the server to echo the
/// close frame before dropping the connection anyway.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// A live WebSocket subscription yielding decoded messages.
///
/// Each text frame becomes one `Ok(T)`. A frame that does not decode
/// as `T` yields `Err(Error::Json)` and the stream carries on, so one
/// unexpected message does not end the subscription. A transport
/// failure yields one `Err(Error::WebSocket)` and then the stream
/// ends. A close frame from the server ends the stream without an
/// error.
///
/// Protect reports some failures *after* accepting the upgrade: it
/// sends an error document (`{"name": ..., "error": ...}`) as a text
/// frame and then closes. Rate limiting works this way
/// (`TOO_MANY_REQUESTS_ERROR`), because the handshake counts against
/// the same 10-requests-per-second budget as every other call. Such a
/// frame yields `Err(Error::SubscriptionRejected)`, after which the
/// stream ends.
///
/// Dropping a `Subscription` drops the connection without a close
/// handshake; call [`Self::close`] to disconnect cleanly.
pub struct Subscription<T> {
    socket: WebSocketStream<reqwest::Upgraded>,
    path: &'static str,
    done: bool,
    _message: PhantomData<fn() -> T>,
}

impl<T> Subscription<T> {
    /// Send a close frame and wait (up to 5 seconds) for the server to
    /// acknowledge it. Messages that arrive in the meantime are
    /// discarded.
    ///
    /// Closing a subscription the server has already closed is a no-op.
    ///
    /// # Errors
    /// [`Error::WebSocket`] if the close frame cannot be sent.
    pub async fn close(mut self) -> Result<()> {
        if self.done {
            return Ok(());
        }
        debug!("closing subscription {}", self.path);
        match self.socket.close(None).await {
            Ok(()) => {}
            Err(WsError::ConnectionClosed | WsError::AlreadyClosed) => return Ok(()),
            Err(e) => return Err(e.into()),
        }
        let drain = async { while self.socket.next().await.is_some() {} };
        if tokio::time::timeout(CLOSE_TIMEOUT, drain).await.is_err() {
            warn!(
                "{}: server did not acknowledge close within {CLOSE_TIMEOUT:?}",
                self.path
            );
        }
        info!("closed subscription {}", self.path);
        Ok(())
    }
}

impl<T: DeserializeOwned> Stream for Subscription<T> {
    type Item = Result<T>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if this.done {
                return Poll::Ready(None);
            }
            let decoded = match ready!(this.socket.poll_next_unpin(cx)) {
                Some(Ok(Message::Text(text))) => decode(this.path, text.as_bytes()),
                Some(Ok(Message::Binary(bytes))) => {
                    debug!("{}: binary frame ({} bytes)", this.path, bytes.len());
                    decode(this.path, &bytes)
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => continue,
                Some(Ok(Message::Close(frame))) => {
                    // Keep polling: tungstenite flushes its close reply on
                    // the next read and then reports the end of the stream.
                    info!("{}: server closed the subscription: {frame:?}", this.path);
                    continue;
                }
                Some(Err(e)) => {
                    this.done = true;
                    return Poll::Ready(Some(Err(e.into())));
                }
                None => {
                    this.done = true;
                    continue;
                }
            };
            return Poll::Ready(Some(decoded));
        }
    }
}

/// Decode one frame as `T`, falling back to Protect's error document
/// shape before reporting a schema mismatch.
fn decode<T: DeserializeOwned>(path: &str, frame: &[u8]) -> Result<T> {
    let err = match serde_json::from_slice(frame) {
        Ok(message) => return Ok(message),
        Err(e) => e,
    };
    if let Some(rejection) = Error::from_rejection_frame(frame) {
        warn!("{path}: {rejection}");
        return Err(rejection);
    }
    warn!("{path}: message did not match the expected schema: {err}");
    Err(err.into())
}

/// WebSocket subscription entry point. Cheap to construct; holds a
/// borrow of the [`ProtectClient`] that issued it.
pub struct SubscribeApi<'a> {
    client: &'a ProtectClient,
}

impl<'a> SubscribeApi<'a> {
    pub(crate) const fn new(client: &'a ProtectClient) -> Self {
        Self { client }
    }

    /// `GET /v1/subscribe/devices` (WebSocket). Every add, update, and
    /// remove of Protect-managed hardware, as it happens. Returns once
    /// the handshake has succeeded.
    ///
    /// # Errors
    /// [`Error`] -- typically `Http` (network, TLS) or `Api` (the
    /// server refused the upgrade, e.g. 401).
    pub async fn devices(&self) -> Result<Subscription<DeviceMessage>> {
        self.client.open_subscription("/v1/subscribe/devices").await
    }

    /// `GET /v1/subscribe/events` (WebSocket). Protect events (motion,
    /// rings, smart detections, sensor alarms, ...) as they start and
    /// update. Returns once the handshake has succeeded.
    ///
    /// # Errors
    /// As for [`Self::devices`].
    pub async fn events(&self) -> Result<Subscription<EventMessage>> {
        self.client.open_subscription("/v1/subscribe/events").await
    }
}

impl ProtectClient {
    /// WebSocket subscription endpoints.
    #[must_use]
    pub const fn subscribe(&self) -> SubscribeApi<'_> {
        SubscribeApi::new(self)
    }

    /// Open a WebSocket to `path` and wrap it as a typed [`Subscription`].
    pub(crate) async fn open_subscription<T>(&self, path: &'static str) -> Result<Subscription<T>> {
        debug!("GET {path} (websocket upgrade)");
        let key = generate_key();
        let response = self
            .http_retriable
            .get(self.url(path)?)
            .header(CONNECTION, "Upgrade")
            .header(UPGRADE, "websocket")
            .header(SEC_WEBSOCKET_VERSION, "13")
            .header(SEC_WEBSOCKET_KEY, &key)
            .send()
            .await?;

        let status = response.status();
        if status != StatusCode::SWITCHING_PROTOCOLS {
            if status.is_success() {
                return Err(Error::Other(format!(
                    "{path}: expected 101 Switching Protocols, got {status}"
                )));
            }
            return Err(Error::from_response(response).await);
        }
        let expected_accept = derive_accept_key(key.as_bytes());
        let accept = response.headers().get(SEC_WEBSOCKET_ACCEPT);
        if accept.is_none_or(|v| v.as_bytes() != expected_accept.as_bytes()) {
            return Err(Error::Other(format!(
                "{path}: server sent a missing or wrong Sec-WebSocket-Accept"
            )));
        }

        let upgraded = response.upgrade().await?;
        let socket = WebSocketStream::from_raw_socket(upgraded, Role::Client, None).await;
        info!("subscribed to {path}");
        Ok(Subscription {
            socket,
            path,
            done: false,
            _message: PhantomData,
        })
    }
}

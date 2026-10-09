//! Subscriptions that reconnect after a drop (`reconnect` feature).

use std::future::Future;
use std::num::NonZeroU32;
use std::pin::Pin;
use std::task::{Context, Poll, ready};
use std::time::Duration;

use futures_util::{Stream, StreamExt};
use log::{info, warn};
use serde::de::DeserializeOwned;
use tokio::time::{Instant, Sleep};

use super::{SubscribeApi, Subscription};
use crate::client::ProtectClient;
use crate::error::{Error, Result};
use crate::models::{DeviceMessage, EventMessage};

/// Backoff policy for a [`ReconnectingSubscription`].
///
/// After a drop, the first reconnect waits `initial_backoff`; each
/// further consecutive attempt doubles the wait, up to `max_backoff`.
/// The count resets once a reconnected subscription delivers a message
/// or stays up for `max_backoff`.
#[derive(Debug, Clone)]
pub struct ReconnectConfig {
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    /// Give up after this many consecutive failed connection attempts.
    /// `None` retries forever. Each attempt is one upgrade request, which
    /// the client's HTTP retry policy may itself retry on transient
    /// failures (connection refused, 5xx) before it counts as failed.
    pub max_attempts: Option<NonZeroU32>,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            initial_backoff: Duration::from_secs(8),
            max_backoff: Duration::from_secs(120),
            max_attempts: None,
        }
    }
}

impl ReconnectConfig {
    fn backoff(&self, attempt: u32) -> Duration {
        self.initial_backoff
            .saturating_mul(2_u32.saturating_pow(attempt))
            .min(self.max_backoff)
    }
}

type Connecting<T> = Pin<Box<dyn Future<Output = Result<Subscription<T>>> + Send>>;

enum State<T> {
    Connected {
        subscription: Box<Subscription<T>>,
        since: Instant,
    },
    Waiting(Pin<Box<Sleep>>),
    Connecting(Connecting<T>),
    Done,
}

/// A subscription that reconnects with exponential backoff when the
/// connection drops, the server closes it, or it is rejected in-band
/// (e.g. rate limiting).
///
/// Messages arrive as from [`Subscription`]. In addition, every
/// disconnect is reported as one `Err` item before reconnecting, so a
/// consumer that tracks state can resynchronise: messages sent while
/// disconnected are lost, because Protect does not replay them. Failed
/// reconnection attempts are only logged.
///
/// The stream ends, after yielding the error that ended it, when a
/// reconnection attempt fails permanently (a 4xx other than 408 or
/// 429, e.g. a revoked API key) or `max_attempts` consecutive
/// attempts have failed.
pub struct ReconnectingSubscription<T> {
    client: ProtectClient,
    path: &'static str,
    config: ReconnectConfig,
    state: State<T>,
    /// Connection attempts made since the last healthy connection.
    attempt: u32,
}

impl<T: DeserializeOwned + 'static> ReconnectingSubscription<T> {
    async fn open(
        client: &ProtectClient,
        path: &'static str,
        config: ReconnectConfig,
    ) -> Result<Self> {
        let subscription = client.open_subscription(path).await?;
        Ok(Self {
            client: client.clone(),
            path,
            config,
            state: State::Connected {
                subscription: Box::new(subscription),
                since: Instant::now(),
            },
            attempt: 0,
        })
    }

    /// Whether `max_attempts` consecutive attempts have been made. The
    /// count is reset by a message or a long-lived connection, so the
    /// initial connection (attempt 0) never counts.
    fn attempts_exhausted(&self) -> bool {
        self.config
            .max_attempts
            .is_some_and(|max| self.attempt >= max.get())
    }

    /// Schedule the next attempt and return how long it will wait.
    fn disconnected(&mut self, healthy: bool) -> Duration {
        if healthy {
            self.attempt = 0;
        }
        let delay = self.config.backoff(self.attempt);
        self.state = State::Waiting(Box::pin(tokio::time::sleep(delay)));
        delay
    }

    fn connect(&self) -> Connecting<T> {
        let client = self.client.clone();
        let path = self.path;
        Box::pin(async move { client.open_subscription(path).await })
    }
}

impl<T> ReconnectingSubscription<T> {
    /// Close the current connection cleanly, if there is one, and stop
    /// reconnecting.
    ///
    /// # Errors
    /// As for [`Subscription::close`].
    pub async fn close(self) -> Result<()> {
        match self.state {
            State::Connected { subscription, .. } => (*subscription).close().await,
            _ => Ok(()),
        }
    }
}

impl<T: DeserializeOwned + 'static> Stream for ReconnectingSubscription<T> {
    type Item = Result<T>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            match &mut this.state {
                State::Done => return Poll::Ready(None),
                State::Connected {
                    subscription,
                    since,
                } => {
                    let healthy = since.elapsed() >= this.config.max_backoff;
                    let error = match ready!(subscription.poll_next_unpin(cx)) {
                        Some(Ok(message)) => {
                            this.attempt = 0;
                            return Poll::Ready(Some(Ok(message)));
                        }
                        Some(Err(e @ Error::Json(_))) => return Poll::Ready(Some(Err(e))),
                        Some(Err(e)) => e,
                        None => Error::WebSocket("server closed the subscription".into()),
                    };
                    // A reconnect that upgraded but then ended without
                    // delivering a message (e.g. rejected in-band for rate
                    // limiting) counts as a failed attempt.
                    if !healthy && this.attempts_exhausted() {
                        warn!("{}: giving up reconnecting: {error}", this.path);
                        this.state = State::Done;
                        return Poll::Ready(Some(Err(error)));
                    }
                    let delay = this.disconnected(healthy);
                    warn!(
                        "{}: subscription interrupted: {error}; reconnecting in {delay:?}",
                        this.path
                    );
                    return Poll::Ready(Some(Err(error)));
                }
                State::Waiting(sleep) => {
                    ready!(sleep.as_mut().poll(cx));
                    this.attempt += 1;
                    info!("{}: reconnect attempt {}", this.path, this.attempt);
                    this.state = State::Connecting(this.connect());
                }
                State::Connecting(connecting) => match ready!(connecting.as_mut().poll(cx)) {
                    Ok(subscription) => {
                        info!("{}: reconnected", this.path);
                        this.state = State::Connected {
                            subscription: Box::new(subscription),
                            since: Instant::now(),
                        };
                    }
                    Err(e) => {
                        if is_permanent(&e) || this.attempts_exhausted() {
                            warn!("{}: giving up reconnecting: {e}", this.path);
                            this.state = State::Done;
                            return Poll::Ready(Some(Err(e)));
                        }
                        let attempt = this.attempt;
                        let delay = this.disconnected(false);
                        warn!(
                            "{}: reconnect attempt {attempt} failed: {e}; retrying in {delay:?}",
                            this.path
                        );
                    }
                },
            }
        }
    }
}

/// Errors that another attempt cannot fix: the server understood the
/// request and refused it (bad key, missing permission, wrong path),
/// or the client itself is misconfigured.
const fn is_permanent(error: &Error) -> bool {
    match error {
        Error::Api { status, .. } => {
            *status >= 400 && *status < 500 && *status != 408 && *status != 429
        }
        Error::InvalidUrl(_) | Error::MissingApiKey => true,
        _ => false,
    }
}

impl SubscribeApi<'_> {
    /// [`Self::devices`], reconnecting after drops. The first
    /// connection is made before this returns, so a bad key or
    /// unreachable NVR fails here rather than in the stream.
    ///
    /// # Errors
    /// As for [`Self::devices`].
    pub async fn devices_reconnecting(
        &self,
        config: ReconnectConfig,
    ) -> Result<ReconnectingSubscription<DeviceMessage>> {
        ReconnectingSubscription::open(self.client, "/v1/subscribe/devices", config).await
    }

    /// [`Self::events`], reconnecting after drops. See
    /// [`Self::devices_reconnecting`].
    ///
    /// # Errors
    /// As for [`Self::devices`].
    pub async fn events_reconnecting(
        &self,
        config: ReconnectConfig,
    ) -> Result<ReconnectingSubscription<EventMessage>> {
        ReconnectingSubscription::open(self.client, "/v1/subscribe/events", config).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_from_initial_and_caps_at_max() {
        let config = ReconnectConfig::default();
        let waits: Vec<u64> = (0..7).map(|n| config.backoff(n).as_secs()).collect();
        assert_eq!(waits, [8, 16, 32, 64, 120, 120, 120]);
        assert_eq!(config.backoff(u32::MAX), Duration::from_secs(120));
    }
}

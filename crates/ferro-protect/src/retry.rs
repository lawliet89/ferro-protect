//! Custom retry middleware that honours the `Retry-After` response
//! header.
//!
//! `reqwest-retry`'s built-in [`RetryTransientMiddleware`] knows what
//! *kinds* of responses are retriable but ignores the server's
//! `Retry-After` hint, falling back to its own exponential schedule.
//! UniFi Protect returns `retry-after: 1` on 429, so respecting it
//! matches the contract the server is actually advertising rather
//! than guessing.
//!
//! Behaviour:
//! - **429 Too Many Requests** -> retry with `Retry-After` if present
//!   (capped at `max_backoff`), else exponential backoff.
//! - **5xx Server Error / 408 Request Timeout** -> retry with
//!   exponential backoff and jitter between `initial_backoff` and
//!   `max_backoff`.
//! - **Network / connect / read timeout errors** -> retry the same way.
//! - All other outcomes return without retrying.
//!
//! Only the **delta-seconds** form of `Retry-After` (e.g. `Retry-After:
//! 5`) is honoured. RFC 9110 also permits an HTTP-date form
//! (`Retry-After: Wed, 21 Oct 2026 07:28:00 GMT`); UniFi Protect does
//! not send that form, so parsing it is not worth a dependency on
//! `httpdate`. If a date-form value is encountered, the middleware
//! falls back to its own exponential backoff.
//!
//! Attempts are bounded by `max_retries`. Once exhausted, the last
//! response (or error) is surfaced unchanged so the caller sees the
//! real status code rather than a generic "retries exhausted" wrapper.

use std::time::Duration;

use async_trait::async_trait;
use http::Extensions;
use log::debug;
use rand::Rng;
use reqwest::{Request, Response};
use reqwest_middleware::{Middleware, Next, Result};

#[derive(Debug, Clone)]
#[expect(
    clippy::redundant_pub_crate,
    reason = "pub(crate) needed for cross-module access within the crate"
)]
pub(crate) struct RetryAfterAwareMiddleware {
    pub(crate) max_retries: u32,
    pub(crate) initial_backoff: Duration,
    pub(crate) max_backoff: Duration,
}

#[async_trait]
impl Middleware for RetryAfterAwareMiddleware {
    async fn handle(
        &self,
        req: Request,
        extensions: &mut Extensions,
        next: Next<'_>,
    ) -> Result<Response> {
        let mut attempt: u32 = 0;
        loop {
            let Some(req_clone) = req.try_clone() else {
                // Streaming body etc. — cannot retry safely, pass through.
                return next.clone().run(req, extensions).await;
            };

            let result = next.clone().run(req_clone, extensions).await;

            let Some(delay) = self.decide_retry(&result, attempt) else {
                return result;
            };

            // Drain the response body before sleeping so the underlying
            // connection is returned to reqwest's pool rather than left
            // half-read until drop. Without this, retries cost a fresh
            // TCP+TLS handshake every time under load.
            if let Ok(response) = result {
                let _ = response.bytes().await;
            }

            attempt += 1;
            debug!(
                "retrying request (attempt {attempt}/{max}) after {delay:?}",
                max = self.max_retries
            );
            tokio::time::sleep(delay).await;
        }
    }
}

impl RetryAfterAwareMiddleware {
    /// Decide whether to retry and how long to wait. `None` means stop.
    fn decide_retry(
        &self,
        result: &Result<Response>,
        attempt_just_finished: u32,
    ) -> Option<Duration> {
        if attempt_just_finished >= self.max_retries {
            return None;
        }

        match result {
            Ok(response) => {
                let status = response.status();
                if status.as_u16() == 429 {
                    let from_header = response
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.trim().parse::<u64>().ok())
                        .map(Duration::from_secs);
                    if let Some(d) = from_header {
                        return Some(d.min(self.max_backoff));
                    }
                    return Some(self.backoff_for(attempt_just_finished));
                }
                if status.is_server_error() || status.as_u16() == 408 {
                    return Some(self.backoff_for(attempt_just_finished));
                }
                None
            }
            Err(e) => {
                if is_transient(e) {
                    Some(self.backoff_for(attempt_just_finished))
                } else {
                    None
                }
            }
        }
    }

    fn backoff_for(&self, attempt: u32) -> Duration {
        // Exponential: initial * 2^attempt, capped at max_backoff. Add
        // up to 100ms of jitter so concurrent callers don't synchronise.
        let exp = self
            .initial_backoff
            .saturating_mul(1u32.checked_shl(attempt).unwrap_or(u32::MAX));
        let capped = exp.min(self.max_backoff);
        let jitter_ms = rand::thread_rng().gen_range(0..100);
        capped + Duration::from_millis(jitter_ms)
    }
}

fn is_transient(err: &reqwest_middleware::Error) -> bool {
    let reqwest_middleware::Error::Reqwest(e) = err else {
        return false;
    };
    if e.is_timeout() || e.is_connect() {
        return true;
    }
    // is_request() catches some redirect / decode errors too; only
    // retry true network-failure shapes.
    if let Some(status) = e.status() {
        if status.is_server_error() || status.as_u16() == 429 || status.as_u16() == 408 {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mw(max_retries: u32) -> RetryAfterAwareMiddleware {
        RetryAfterAwareMiddleware {
            max_retries,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(2),
        }
    }

    #[test]
    fn backoff_grows_exponentially_under_cap() {
        let m = mw(10);
        // 100ms initial, 2s cap, plus 0..100ms jitter on every value.
        let d0 = m.backoff_for(0);
        let d1 = m.backoff_for(1);
        let d4 = m.backoff_for(4); // 100*16 = 1600ms (under cap)
        let d6 = m.backoff_for(6); // 100*64 = 6400ms -> capped to 2000ms
        assert!(d0 >= Duration::from_millis(100) && d0 < Duration::from_millis(200));
        assert!(d1 >= Duration::from_millis(200) && d1 < Duration::from_millis(300));
        assert!(d4 >= Duration::from_millis(1600) && d4 < Duration::from_millis(1700));
        assert!(d6 >= Duration::from_secs(2) && d6 < Duration::from_millis(2100));
    }
}

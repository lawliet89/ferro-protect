//! Proactive client-side rate limiter backed by [`governor`].
//!
//! UniFi Protect advertises its quota via RFC 9331
//! `RateLimit-Policy: "10-in-1sec"; q=10; w=1`. We pin the client to a
//! matching fixed quota (10 requests / 1 second by default, with burst=10)
//! so a tight loop of reads cannot overrun the server's leaky-bucket
//! budget and trip a 429. The retry middleware
//! ([`crate::retry::RetryAfterAwareMiddleware`]) still recovers from any
//! 429 that does slip through.
//!
//! [`governor`] implements GCRA (a leaky-bucket variant) over a single
//! atomic state cell -- no spawned timers, no shrink-vs-deadlock corner
//! cases. If Protect ever raises its advertised quota, bump
//! [`RateLimitConfig::rate`] in the builder; we deliberately do not
//! auto-track the `RateLimit-Policy` header because the advertised value
//! has been stable across observed firmware versions and the runtime
//! re-tuning machinery is not worth the surface area.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use governor::clock::DefaultClock;
use governor::middleware::NoOpMiddleware;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};
use http::Extensions;
use reqwest::{Request, Response};
use reqwest_middleware::{Middleware, Next};

type DirectLimiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock, NoOpMiddleware>;

/// Public knobs for the proactive rate limiter.
///
/// Defaults match Protect 7.1.60's advertised policy (`10-in-1sec`).
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Requests allowed in any rolling `per` window. Also acts as the
    /// burst size: the bucket starts full, so up to `rate` requests fire
    /// immediately before refill pacing kicks in.
    pub rate: NonZeroU32,
    /// Window over which `rate` requests are allowed.
    pub per: Duration,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            rate: NonZeroU32::new(10).expect("10 is non-zero"),
            per: Duration::from_secs(1),
        }
    }
}

impl RateLimitConfig {
    /// Convert into a `governor::Quota`. Panics only if `per` is zero,
    /// which is rejected at construction time of a `Duration`-typed value
    /// only when explicitly built as `Duration::ZERO` -- a misuse we don't
    /// guard further here.
    fn quota(&self) -> Quota {
        // `Quota::with_period` takes the period *between* cell refills,
        // i.e. `per / rate`. `allow_burst` then sets the bucket depth.
        let cell_period = self
            .per
            .checked_div(self.rate.get())
            .expect("per must be non-zero");
        Quota::with_period(cell_period)
            .expect("cell period must be non-zero")
            .allow_burst(self.rate)
    }
}

#[expect(
    clippy::redundant_pub_crate,
    reason = "pub(crate) needed for cross-module access within the crate"
)]
#[derive(Clone)]
pub(crate) struct RateLimitMiddleware {
    limiter: Arc<DirectLimiter>,
}

impl RateLimitMiddleware {
    pub(crate) fn new(config: &RateLimitConfig) -> Self {
        Self {
            limiter: Arc::new(RateLimiter::direct(config.quota())),
        }
    }
}

#[async_trait]
impl Middleware for RateLimitMiddleware {
    async fn handle(
        &self,
        req: Request,
        extensions: &mut Extensions,
        next: Next<'_>,
    ) -> reqwest_middleware::Result<Response> {
        self.limiter.until_ready().await;
        next.run(req, extensions).await
    }
}

// Behaviour is exercised end-to-end against a wiremock server in
// `tests/rate_limit.rs::proactive_throttle_caps_burst_to_configured_capacity`.
// Governor uses its own monotonic clock (quanta), which tokio's
// `start_paused = true` cannot stub, so reliable timing assertions live in
// the integration suite.

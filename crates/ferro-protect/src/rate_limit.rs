//! Adaptive client-side rate limiter driven by the NVR's RFC 9331
//! `RateLimit` / `RateLimit-Policy` response headers.
//!
//! The UniFi Protect integration server enforces a per-window quota and
//! advertises it on every response (`RateLimit-Policy: "10-in-1sec";
//! q=10; w=1`). The limiter parses those headers and uses a
//! [`Semaphore`] sized to the advertised quota so the in-process client
//! never exceeds the server's published budget. Each permit is
//! auto-released after `window`, giving a sliding-window approximation
//! that matches the server's leaky-bucket semantics in the common case.
//!
//! On a permit acquisition the helper sends the request; on response,
//! the limiter [`observe`](AdaptiveLimiter::observe)s the headers and
//! grows capacity if the server now advertises a larger quota. Shrinks
//! are logged at `warn!` but not applied -- mutating a live semaphore
//! downward is unsound (we can't reclaim permits already handed out
//! without risking deadlock against in-flight requests). In practice
//! Protect never tightens its policy mid-session.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use log::{debug, info, warn};
use reqwest::header::HeaderMap;
use tokio::sync::Semaphore;

/// Public knobs for the proactive rate limiter.
///
/// The defaults match the policy Protect 7.1.60 currently advertises
/// (`10-in-1sec`); the limiter still self-adjusts upward if a future
/// firmware raises the quota.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Maximum concurrent in-flight requests within `window`.
    pub initial_capacity: u32,
    /// Window over which `initial_capacity` requests are allowed.
    pub window: Duration,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            initial_capacity: 10,
            window: Duration::from_secs(1),
        }
    }
}

#[derive(Clone)]
#[allow(clippy::redundant_pub_crate)]
pub(crate) struct AdaptiveLimiter {
    inner: Arc<Inner>,
}

struct Inner {
    capacity: AtomicU32,
    window_millis: AtomicU64,
    semaphore: Arc<Semaphore>,
}

impl AdaptiveLimiter {
    pub(crate) fn new(config: &RateLimitConfig) -> Self {
        let capacity = config.initial_capacity.max(1);
        let semaphore = Arc::new(Semaphore::new(capacity as usize));
        let window_millis = u64::try_from(config.window.as_millis()).unwrap_or(u64::MAX);
        Self {
            inner: Arc::new(Inner {
                capacity: AtomicU32::new(capacity),
                window_millis: AtomicU64::new(window_millis),
                semaphore,
            }),
        }
    }

    /// Acquire a slot in the current window. Blocks (asynchronously) if
    /// the budget is exhausted. The permit auto-releases after `window`
    /// elapses from this acquisition, regardless of how long the
    /// caller's request takes -- this matches the server's
    /// "N requests per rolling window" semantics rather than
    /// "N concurrent requests."
    pub(crate) async fn acquire(&self) {
        let permit = Arc::clone(&self.inner.semaphore)
            .acquire_owned()
            .await
            .expect("rate-limit semaphore is never closed");
        let window = Duration::from_millis(self.inner.window_millis.load(Ordering::Acquire));
        tokio::spawn(async move {
            tokio::time::sleep(window).await;
            drop(permit);
        });
    }

    /// Parse RFC 9331 `RateLimit-Policy` from a response and grow the
    /// budget if the server now advertises a larger quota. No-op when
    /// headers are absent or unparseable.
    pub(crate) fn observe(&self, headers: &HeaderMap) {
        let Some(policy) = parse_ratelimit_policy(headers) else {
            return;
        };

        let new_window_millis = u64::from(policy.window_seconds).saturating_mul(1000);
        let prev_window_millis = self
            .inner
            .window_millis
            .swap(new_window_millis, Ordering::AcqRel);
        if prev_window_millis != new_window_millis {
            debug!("rate-limit window adjusted: {prev_window_millis}ms -> {new_window_millis}ms");
        }

        let prev_capacity = self.inner.capacity.load(Ordering::Acquire);
        if policy.quota == prev_capacity {
            return;
        }
        if policy.quota > prev_capacity {
            let delta = policy.quota - prev_capacity;
            self.inner.semaphore.add_permits(delta as usize);
            self.inner.capacity.store(policy.quota, Ordering::Release);
            info!(
                "rate-limit capacity increased from server policy: {prev_capacity} -> {}",
                policy.quota
            );
        } else {
            warn!(
                "server now advertises tighter rate limit ({}/{}ms) than client tracks ({prev_capacity}); not shrinking semaphore to avoid deadlock against in-flight requests",
                policy.quota, new_window_millis,
            );
        }
    }
}

#[derive(Debug, PartialEq)]
struct RateLimitPolicy {
    quota: u32,
    window_seconds: u32,
}

fn parse_ratelimit_policy(headers: &HeaderMap) -> Option<RateLimitPolicy> {
    let raw = headers.get("ratelimit-policy")?.to_str().ok()?;
    let mut quota: Option<u32> = None;
    let mut window_seconds: Option<u32> = None;
    for token in raw.split(';') {
        let trimmed = token.trim();
        if let Some(v) = trimmed.strip_prefix("q=") {
            quota = v.trim().parse().ok();
        } else if let Some(v) = trimmed.strip_prefix("w=") {
            window_seconds = v.trim().parse().ok();
        }
    }
    Some(RateLimitPolicy {
        quota: quota?,
        window_seconds: window_seconds?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderValue};

    fn hdrs(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("ratelimit-policy", HeaderValue::from_str(value).unwrap());
        h
    }

    #[test]
    fn parses_observed_protect_policy() {
        let h = hdrs(r#""10-in-1sec"; q=10; w=1; pk=:abc:"#);
        let parsed = parse_ratelimit_policy(&h).unwrap();
        assert_eq!(
            parsed,
            RateLimitPolicy {
                quota: 10,
                window_seconds: 1
            }
        );
    }

    #[test]
    fn parse_missing_header_returns_none() {
        assert!(parse_ratelimit_policy(&HeaderMap::new()).is_none());
    }

    #[test]
    fn parse_garbage_returns_none() {
        let mut h = HeaderMap::new();
        h.insert(
            "ratelimit-policy",
            HeaderValue::from_static("not a policy at all"),
        );
        assert!(parse_ratelimit_policy(&h).is_none());
    }

    #[test]
    fn parse_partial_returns_none() {
        let h = hdrs("q=10");
        assert!(parse_ratelimit_policy(&h).is_none());
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn acquire_blocks_when_budget_exhausted() {
        let limiter = AdaptiveLimiter::new(&RateLimitConfig {
            initial_capacity: 2,
            window: Duration::from_secs(1),
        });
        limiter.acquire().await;
        limiter.acquire().await;

        // Third acquire should block until a permit auto-releases (~1s).
        let start = tokio::time::Instant::now();
        limiter.acquire().await;
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(900),
            "expected ~1s block, got {elapsed:?}"
        );
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn observe_grows_capacity() {
        let limiter = AdaptiveLimiter::new(&RateLimitConfig {
            initial_capacity: 2,
            window: Duration::from_secs(1),
        });
        limiter.acquire().await;
        limiter.acquire().await;

        // Server advertises larger quota -- limiter should add permits.
        limiter.observe(&hdrs(r#""5-in-1sec"; q=5; w=1"#));

        // We can now acquire 3 more immediately without blocking.
        for _ in 0..3 {
            tokio::time::timeout(Duration::from_millis(50), limiter.acquire())
                .await
                .expect("permit should be available after capacity grew");
        }
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn observe_does_not_shrink() {
        let limiter = AdaptiveLimiter::new(&RateLimitConfig {
            initial_capacity: 5,
            window: Duration::from_secs(1),
        });
        // Acquire 3 permits; 2 remain available.
        for _ in 0..3 {
            limiter.acquire().await;
        }

        // Server advertises a tighter quota -- limiter must NOT reclaim
        // permits in flight; the remaining 2 stay available so the
        // observed value of 5 is preserved.
        limiter.observe(&hdrs(r#""1-in-1sec"; q=1; w=1"#));

        for _ in 0..2 {
            tokio::time::timeout(Duration::from_millis(50), limiter.acquire())
                .await
                .expect("pre-shrink permits remain usable");
        }
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn observe_no_op_when_header_absent() {
        let limiter = AdaptiveLimiter::new(&RateLimitConfig::default());
        limiter.observe(&HeaderMap::new());
        // Should not panic; capacity unchanged.
        assert_eq!(limiter.inner.capacity.load(Ordering::Acquire), 10);
    }
}

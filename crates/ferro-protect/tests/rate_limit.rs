#![forbid(unsafe_code)]
#![allow(clippy::pedantic, clippy::nursery)]

//! End-to-end behaviour of the retry middleware + adaptive rate limiter
//! against a `wiremock` server. Exercises three properties of the
//! contract documented in `docs/TASK_rate_limit.md`:
//!
//! 1. 429 with `Retry-After` is transparently retried and succeeds.
//! 2. The retry middleware bounds attempts at `max_retries`.
//! 3. The proactive throttle stops a burst of requests from exceeding
//!    the advertised quota even without server-side 429s.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ferro_protect::{Error, ProtectClient, RateLimitConfig, RetryConfig};
use secrecy::SecretString;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const FIXTURE_OK: &str = r#"{"applicationVersion":"7.1.60"}"#;
const FIXTURE_429: &str = r#"{"name":"tooManyRequests","error":"Too many requests"}"#;

async fn client_for(server: &MockServer) -> ProtectClient {
    ProtectClient::builder()
        .base_url(server.uri())
        .api_key(SecretString::from("test-key".to_string()))
        .build()
        .expect("client builds")
}

#[tokio::test]
async fn retries_429_then_succeeds_honouring_retry_after() {
    let server = MockServer::start().await;

    // Counting responder: first call -> 429 + Retry-After: 1; subsequent -> 200.
    struct FirstThen429 {
        calls: Arc<AtomicUsize>,
    }
    impl Respond for FirstThen429 {
        fn respond(&self, _: &Request) -> ResponseTemplate {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                ResponseTemplate::new(429)
                    .set_body_string(FIXTURE_429)
                    .insert_header("content-type", "application/json")
                    .insert_header("retry-after", "1")
                    .insert_header("ratelimit-policy", r#""10-in-1sec"; q=10; w=1"#)
                    .insert_header("ratelimit", r#""10-in-1sec"; r=0; t=1"#)
            } else {
                ResponseTemplate::new(200)
                    .set_body_string(FIXTURE_OK)
                    .insert_header("content-type", "application/json")
                    .insert_header("ratelimit-policy", r#""10-in-1sec"; q=10; w=1"#)
                    .insert_header("ratelimit", r#""10-in-1sec"; r=9; t=1"#)
            }
        }
    }

    let calls = Arc::new(AtomicUsize::new(0));
    Mock::given(method("GET"))
        .and(path("/v1/meta/info"))
        .respond_with(FirstThen429 {
            calls: Arc::clone(&calls),
        })
        .expect(2)
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let started = Instant::now();
    let info = client.info().await.expect("retry recovers from 429");
    let elapsed = started.elapsed();

    assert_eq!(info.application_version.to_string(), "7.1.60");
    assert_eq!(calls.load(Ordering::SeqCst), 2, "exactly one retry");
    // Retry-After: 1 must have been honoured. The middleware respects
    // the header rather than its own backoff schedule, so elapsed
    // should be at least ~1s.
    assert!(
        elapsed >= Duration::from_millis(900),
        "expected ~1s wait for retry-after, got {elapsed:?}"
    );
}

#[tokio::test]
async fn retry_budget_exhausts_and_surfaces_429() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/meta/info"))
        .respond_with(
            ResponseTemplate::new(429)
                .set_body_string(FIXTURE_429)
                .insert_header("content-type", "application/json"),
        )
        .mount(&server)
        .await;

    // Tight retry: 1 attempt + 1 retry, tiny backoff so the test is fast.
    // The default proactive throttle stays on -- two requests in <1s sit
    // well under the 10/sec default budget, so it does not affect the
    // assertions here. (Per `docs/TASK_rate_limit.md`, tests exercise
    // default-on behaviour.)
    let client = ProtectClient::builder()
        .base_url(server.uri())
        .api_key(SecretString::from("test-key".to_string()))
        .retry(RetryConfig {
            max_retries: 1,
            initial_backoff: Duration::from_millis(10),
            max_backoff: Duration::from_millis(20),
        })
        .build()
        .expect("client builds");

    let err = client.info().await.expect_err("retries exhausted");
    match err {
        Error::Api { status: 429, .. } => {}
        other => panic!("expected Api 429 after retries, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn proactive_throttle_caps_burst_to_configured_capacity() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/meta/info"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(FIXTURE_OK)
                .insert_header("content-type", "application/json")
                // No RateLimit-Policy header here, so the throttle keeps
                // its configured capacity rather than adapting upward.
                .set_delay(Duration::from_millis(10)),
        )
        .mount(&server)
        .await;

    let client = ProtectClient::builder()
        .base_url(server.uri())
        .api_key(SecretString::from("test-key".to_string()))
        .rate_limit(Some(RateLimitConfig {
            initial_capacity: 3,
            window: Duration::from_secs(1),
        }))
        .build()
        .expect("client builds");

    // Fire 6 concurrent requests. With capacity=3 / window=1s, the first
    // 3 go immediately and the next 3 must wait ~1s for permits.
    let started = Instant::now();
    let mut joins = Vec::new();
    for _ in 0..6 {
        let c = client.clone();
        joins.push(tokio::spawn(async move { c.info().await }));
    }
    for j in joins {
        j.await.unwrap().expect("info ok");
    }
    let elapsed = started.elapsed();

    assert!(
        elapsed >= Duration::from_millis(900),
        "throttle should have delayed the second batch by ~1s, got {elapsed:?}"
    );
    // Sanity upper bound -- if this blows out, the limiter is over-blocking.
    assert!(
        elapsed < Duration::from_secs(3),
        "throttle delayed too long ({elapsed:?})"
    );
}

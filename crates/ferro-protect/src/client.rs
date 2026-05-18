//! `ProtectClient` -- the user-facing async client.

use std::time::Duration;

use bytes::Bytes;
use log::{debug, info};
use reqwest::header::HeaderMap;
use reqwest_middleware::{ClientBuilder as MiddlewareClientBuilder, ClientWithMiddleware};
use secrecy::SecretString;
use serde::Serialize;
use serde::de::DeserializeOwned;
use url::Url;

use crate::auth::{API_KEY_HEADER, ApiKey};
use crate::error::{Error, Result};
use crate::models::ApplicationInfo;
use crate::rate_limit::{RateLimitConfig, RateLimitMiddleware};
use crate::retry::RetryAfterAwareMiddleware;

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_TOTAL_TIMEOUT: Duration = Duration::from_secs(30);

const DEFAULT_MAX_RETRIES: u32 = 3;
const DEFAULT_RETRY_INITIAL_BACKOFF: Duration = Duration::from_millis(200);
const DEFAULT_RETRY_MAX_BACKOFF: Duration = Duration::from_secs(5);

/// Controls how the underlying TLS stack validates the NVR's certificate.
///
/// `Native` uses the OS or webpki-bundled trust store and is the safe
/// default. `Pinned` lets you supply a PEM-encoded certificate (typical for
/// a self-signed NVR you've fetched the cert from once). `AcceptInvalid`
/// disables verification entirely and is only available when the library
/// is built with the `insecure-tls` feature.
#[derive(Default, Clone)]
pub enum TlsMode {
    #[default]
    Native,
    Pinned(Vec<u8>),
    #[cfg(feature = "insecure-tls")]
    AcceptInvalid,
}

/// Retry policy for transient HTTP failures.
///
/// Covers 429 Too Many Requests, 5xx, and connect/read timeouts. The
/// retry middleware honours `Retry-After` headers when the server
/// provides one (e.g. UniFi Protect returns `retry-after: 1` on 429),
/// falling back to exponential backoff with jitter between
/// `initial_backoff` and `max_backoff`.
///
/// Mutations (POST/PATCH/DELETE) are **not** retried by default --
/// the same backoff policy is only applied to GETs unless
/// [`ProtectClientBuilder::retry_on_mutations`] is set. Retrying a
/// non-idempotent request after a 5xx can silently double-apply the
/// effect because the server may have processed the original request
/// before the response failed to return.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: DEFAULT_MAX_RETRIES,
            initial_backoff: DEFAULT_RETRY_INITIAL_BACKOFF,
            max_backoff: DEFAULT_RETRY_MAX_BACKOFF,
        }
    }
}

/// Async client for the UniFi Protect local integration API.
///
/// Construct via [`ProtectClient::builder`]. The client is `Clone`-cheap
/// (the underlying `reqwest::Client` is reference-counted) so share one
/// instance across tasks.
///
/// **Defaults you get for free:** a proactive rate limiter pinned to the
/// server's advertised quota (10 requests / 1 second on Protect 7.1.60)
/// and a retry middleware that honours `Retry-After` on 429 / 5xx for
/// idempotent reads. Both are configurable on the builder; see
/// [`RateLimitConfig`] and [`RetryConfig`].
#[derive(Clone)]
pub struct ProtectClient {
    /// Used for idempotent reads (GET). Always wraps the retry middleware.
    http_read: ClientWithMiddleware,
    /// Used for mutations (POST/PATCH/DELETE/...). Bypasses the retry
    /// middleware by default so a transient 5xx after the server already
    /// applied the change is not silently re-fired; opt back in with
    /// `ProtectClientBuilder::retry_on_mutations(true)`. The rate-limit
    /// middleware is still applied so writes count against the same
    /// shared budget as reads.
    http_write: ClientWithMiddleware,
    base_url: Url,
}

impl ProtectClient {
    /// Start a builder for a new client.
    pub fn builder() -> ProtectClientBuilder {
        ProtectClientBuilder::default()
    }

    /// `GET /v1/meta/info`. Returns the running Protect application
    /// version and any other metadata the NVR advertises.
    ///
    /// # Errors
    /// Any [`Error`] variant; in practice [`Error::Http`] for network
    /// failures, [`Error::Api`] for a non-2xx response, [`Error::Json`]
    /// if the body does not match the schema.
    pub async fn info(&self) -> Result<ApplicationInfo> {
        let info: ApplicationInfo = self.get_json("/v1/meta/info").await?;
        info!(
            "fetched application info: version {}",
            info.application_version
        );
        Ok(info)
    }

    pub(crate) async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        debug!("GET {path}");
        let response = self.http_read.get(self.url(path)?).send().await?;
        Self::json_response(response).await
    }

    // POST/PATCH/DELETE/binary helpers. `post_json`, `post_empty_json`,
    // and `get_bytes` are wired up by phase 5's camera endpoints;
    // `patch_json` and `send_no_content` are still inert (carrying
    // `#[expect(dead_code, ...)]`) until phases 6-8 wire up writes,
    // viewer assignment, and DELETE-shaped endpoints. Keeping the
    // shapes here means each future endpoint stays a one-line wrapper.

    pub(crate) async fn post_json<B: Serialize + Sync, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        debug!("POST {path}");
        let response = self
            .http_write
            .post(self.url(path)?)
            .json(body)
            .send()
            .await?;
        Self::json_response(response).await
    }

    /// POST with no request body, parsing the JSON response.
    /// Distinct from [`Self::post_json`] because that helper sets a
    /// `Content-Type: application/json` body even for `()`, which
    /// the talkback-session endpoint (and likely future no-body
    /// POSTs) rejects.
    pub(crate) async fn post_empty_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        debug!("POST {path} (no body)");
        let response = self.http_write.post(self.url(path)?).send().await?;
        Self::json_response(response).await
    }

    #[expect(dead_code, reason = "wired up in phases 5-8")]
    pub(crate) async fn patch_json<B: Serialize + Sync, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        debug!("PATCH {path}");
        let response = self
            .http_write
            .patch(self.url(path)?)
            .json(body)
            .send()
            .await?;
        Self::json_response(response).await
    }

    /// Send a request whose 2xx response carries no body (typically 204).
    /// Phases 5-7 use this for actions, mutations without a return shape,
    /// and DELETE-style endpoints. Defined here so endpoint methods stay
    /// one-liners without each one calling `json_response` and then
    /// discarding `()`-shaped deserialisation errors on empty bodies.
    #[expect(dead_code, reason = "wired up in phases 5-8")]
    pub(crate) async fn send_no_content(&self, method: reqwest::Method, path: &str) -> Result<()> {
        debug!("{method} {path}");
        let response = self
            .http_write
            .request(method, self.url(path)?)
            .send()
            .await?;
        if response.status().is_success() {
            return Ok(());
        }
        Err(Error::from_response(response).await)
    }

    pub(crate) async fn get_bytes(&self, path: &str) -> Result<Bytes> {
        debug!("GET {path}");
        let response = self.http_read.get(self.url(path)?).send().await?;
        if response.status().is_success() {
            return Ok(response.bytes().await?);
        }
        Err(Error::from_response(response).await)
    }

    fn url(&self, path: &str) -> Result<Url> {
        self.base_url
            .join(path.trim_start_matches('/'))
            .map_err(|e| Error::InvalidUrl(format!("{path}: {e}")))
    }

    async fn json_response<T: DeserializeOwned>(response: reqwest::Response) -> Result<T> {
        if response.status().is_success() {
            return Ok(response.json().await?);
        }
        Err(Error::from_response(response).await)
    }
}

/// Builder for [`ProtectClient`].
#[derive(Default)]
pub struct ProtectClientBuilder {
    host: Option<String>,
    base_url: Option<String>,
    api_key: Option<SecretString>,
    tls: TlsMode,
    retry: RetryConfig,
    retry_on_mutations: bool,
    rate_limit: Option<RateLimitConfig>,
    rate_limit_overridden: bool,
}

impl ProtectClientBuilder {
    /// Set the NVR hostname (or `host:port`). Combined with the standard
    /// `/proxy/protect/integration` prefix to form the base URL. Mutually
    /// exclusive with [`Self::base_url`].
    #[must_use]
    pub fn host(mut self, host: impl Into<String>) -> Self {
        self.host = Some(host.into());
        self
    }

    /// Override the entire base URL (e.g. for tests against a mock server).
    /// When set, [`Self::host`] is ignored.
    #[must_use]
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }

    /// Set the API key. The key is held in [`SecretString`] for the
    /// lifetime of the builder and the resulting client.
    #[must_use]
    pub fn api_key(mut self, key: SecretString) -> Self {
        self.api_key = Some(key);
        self
    }

    /// Choose the TLS validation mode. Defaults to [`TlsMode::Native`].
    #[must_use]
    pub fn tls(mut self, mode: TlsMode) -> Self {
        self.tls = mode;
        self
    }

    /// Override the transient-failure retry policy. Defaults give
    /// `max_retries=3`, exponential backoff between 200ms and 5s, with
    /// `Retry-After` honoured when the server returns it.
    #[must_use]
    pub const fn retry(mut self, config: RetryConfig) -> Self {
        self.retry = config;
        self
    }

    /// Apply the retry policy to mutating verbs as well as GETs.
    /// Default is `false` -- mutations bypass the retry middleware so
    /// that a transient 5xx after the server already applied the change
    /// is not silently re-applied.
    #[must_use]
    pub const fn retry_on_mutations(mut self, enabled: bool) -> Self {
        self.retry_on_mutations = enabled;
        self
    }

    /// Override the proactive rate-limit configuration. `Some(config)`
    /// installs a GCRA limiter (via [`governor`]) pinned to the given
    /// rate/window; `None` disables the proactive throttle entirely (the
    /// retry middleware still recovers from any 429s the server returns).
    ///
    /// The default is `Some(RateLimitConfig::default())`, which matches
    /// the policy Protect 7.1.60 advertises (10 requests / 1 second).
    /// Most callers should not change this.
    #[must_use]
    pub const fn rate_limit(mut self, config: Option<RateLimitConfig>) -> Self {
        self.rate_limit = config;
        self.rate_limit_overridden = true;
        self
    }

    /// Finalise the builder.
    ///
    /// # Errors
    /// - [`Error::MissingApiKey`] when no API key was supplied.
    /// - [`Error::InvalidUrl`] when neither `host` nor `base_url` was set,
    ///   or when the constructed URL parses as invalid.
    /// - [`Error::Other`] for invalid header values or TLS configuration
    ///   failures.
    /// - [`Error::Http`] for `reqwest` builder failures.
    pub fn build(self) -> Result<ProtectClient> {
        let api_key = self.api_key.ok_or(Error::MissingApiKey)?;
        let base_url_raw = match (self.base_url, self.host) {
            (Some(url), _) => url,
            (None, Some(host)) => format!("https://{host}/proxy/protect/integration"),
            (None, None) => {
                return Err(Error::InvalidUrl(
                    "either host or base_url must be set".into(),
                ));
            }
        };
        let base_url = parse_base_url(&base_url_raw)?;

        let header_value = ApiKey::new(api_key)
            .header_value()
            .map_err(|e| Error::Other(format!("invalid api key for header: {e}")))?;
        let mut headers = HeaderMap::new();
        headers.insert(API_KEY_HEADER, header_value);

        let tls_label = match &self.tls {
            TlsMode::Native => "native",
            TlsMode::Pinned(_) => "pinned",
            #[cfg(feature = "insecure-tls")]
            TlsMode::AcceptInvalid => "accept-invalid (insecure!)",
        };

        let mut reqwest_builder = reqwest::ClientBuilder::new()
            .default_headers(headers)
            .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
            .timeout(DEFAULT_TOTAL_TIMEOUT);

        reqwest_builder = match self.tls {
            TlsMode::Native => reqwest_builder,
            TlsMode::Pinned(pem) => {
                let cert = reqwest::Certificate::from_pem(&pem)
                    .map_err(|e| Error::Other(format!("invalid pinned certificate: {e}")))?;
                reqwest_builder.add_root_certificate(cert)
            }
            #[cfg(feature = "insecure-tls")]
            TlsMode::AcceptInvalid => reqwest_builder.danger_accept_invalid_certs(true),
        };

        let reqwest_client = reqwest_builder.build()?;

        let retry_middleware = RetryAfterAwareMiddleware {
            max_retries: self.retry.max_retries,
            initial_backoff: self.retry.initial_backoff,
            max_backoff: self.retry.max_backoff,
        };

        // Rate-limit semantics: `Option<RateLimitConfig>` means
        // `Some(custom)` / `None` (explicitly disabled) when the user
        // called `.rate_limit(...)`. If they did not call it at all, we
        // install the default `Some(RateLimitConfig::default())` so the
        // README's `cargo test --all` works against a real NVR without
        // any opt-in.
        let effective_rate_limit = if self.rate_limit_overridden {
            self.rate_limit.clone()
        } else {
            Some(RateLimitConfig::default())
        };
        // One shared GCRA limiter, cloned into both middleware stacks so
        // reads and writes share a single budget.
        let opt_limiter = effective_rate_limit
            .as_ref()
            .map(RateLimitMiddleware::new)
            .transpose()?;

        // `RateLimitMiddleware` is stacked *inside* (after) the retry
        // middleware so every retry attempt acquires a fresh permit.
        // This ensures retries are counted against the server budget
        // rather than bypassing the limiter's per-window quota.
        let http_read = {
            let mut builder =
                MiddlewareClientBuilder::new(reqwest_client.clone()).with(retry_middleware.clone());
            if let Some(ref limiter) = opt_limiter {
                builder = builder.with(limiter.clone());
            }
            builder.build()
        };

        let http_write = {
            let mut builder = MiddlewareClientBuilder::new(reqwest_client);
            if self.retry_on_mutations {
                builder = builder.with(retry_middleware);
            }
            if let Some(limiter) = opt_limiter {
                builder = builder.with(limiter);
            }
            builder.build()
        };

        let rate_limit_label = match (&effective_rate_limit, self.rate_limit_overridden) {
            (Some(cfg), true) => format!("custom ({}/{}ms)", cfg.rate, cfg.per.as_millis()),
            (Some(cfg), false) => format!("default ({}/{}ms)", cfg.rate, cfg.per.as_millis()),
            (None, _) => "disabled".to_string(),
        };

        info!(
            "ProtectClient built: base_url={base_url}, tls={tls_label}, retry={{max={}, initial={:?}, max={:?}, on_mutations={}}}, rate_limit={rate_limit_label}",
            self.retry.max_retries,
            self.retry.initial_backoff,
            self.retry.max_backoff,
            self.retry_on_mutations,
        );
        debug!(
            "client timeouts: connect={DEFAULT_CONNECT_TIMEOUT:?}, total={DEFAULT_TOTAL_TIMEOUT:?}"
        );
        Ok(ProtectClient {
            http_read,
            http_write,
            base_url,
        })
    }
}

fn parse_base_url(raw: &str) -> Result<Url> {
    let mut raw = raw.to_string();
    if !raw.ends_with('/') {
        raw.push('/');
    }
    Url::parse(&raw).map_err(|e| Error::InvalidUrl(format!("{raw}: {e}")))
}

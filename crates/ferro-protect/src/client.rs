//! `ProtectClient` -- the user-facing async client.

use std::time::Duration;

use bytes::Bytes;
use log::{debug, info};
use reqwest::header::HeaderMap;
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use serde::Serialize;
use url::Url;

use crate::auth::{ApiKey, API_KEY_HEADER};
use crate::error::{Error, Result};
use crate::models::ApplicationInfo;

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_TOTAL_TIMEOUT: Duration = Duration::from_secs(30);

/// Controls how the underlying TLS stack validates the NVR's certificate.
///
/// `Native` uses the OS or webpki-bundled trust store and is the safe
/// default. `Pinned` lets you supply a PEM-encoded certificate (typical for
/// a self-signed NVR you've fetched the cert from once). `AcceptInvalid`
/// disables verification entirely and is only available when the library
/// is built with the `dangerous-tls` feature.
#[derive(Default, Clone)]
pub enum TlsMode {
    #[default]
    Native,
    Pinned(Vec<u8>),
    #[cfg(feature = "dangerous-tls")]
    AcceptInvalid,
}

/// Async client for the UniFi Protect local integration API.
///
/// Construct via [`ProtectClient::builder`]. The client is `Clone`-cheap
/// (the underlying `reqwest::Client` is reference-counted) so share one
/// instance across tasks.
#[derive(Clone)]
pub struct ProtectClient {
    http: reqwest::Client,
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
        let response = self.http.get(self.url(path)?).send().await?;
        Self::json_response(response).await
    }

    // The helpers below are unused at phase 4 but cover the shapes
    // phases 5-8 will need (PATCH/POST bodies, 204 No Content, binary
    // payloads). Keeping them here means each future endpoint is a
    // one-line wrapper rather than a one-helper-plus-one-line churn.

    #[allow(dead_code)]
    pub(crate) async fn post_json<B: Serialize + Sync, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        debug!("POST {path}");
        let response = self.http.post(self.url(path)?).json(body).send().await?;
        Self::json_response(response).await
    }

    #[allow(dead_code)]
    pub(crate) async fn patch_json<B: Serialize + Sync, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        debug!("PATCH {path}");
        let response = self.http.patch(self.url(path)?).json(body).send().await?;
        Self::json_response(response).await
    }

    /// Send a request whose 2xx response carries no body (typically 204).
    /// Phases 5-7 use this for actions, mutations without a return shape,
    /// and DELETE-style endpoints. Defined here so endpoint methods stay
    /// one-liners without each one calling `json_response` and then
    /// discarding `()`-shaped deserialisation errors on empty bodies.
    #[allow(dead_code)]
    pub(crate) async fn send_no_content(&self, method: reqwest::Method, path: &str) -> Result<()> {
        debug!("{method} {path}");
        let response = self.http.request(method, self.url(path)?).send().await?;
        if response.status().is_success() {
            return Ok(());
        }
        Err(Error::from_response(response).await)
    }

    #[allow(dead_code)]
    pub(crate) async fn get_bytes(&self, path: &str) -> Result<Bytes> {
        debug!("GET {path}");
        let response = self.http.get(self.url(path)?).send().await?;
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
            #[cfg(feature = "dangerous-tls")]
            TlsMode::AcceptInvalid => "accept-invalid (insecure!)",
        };

        let mut builder = reqwest::ClientBuilder::new()
            .default_headers(headers)
            .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
            .timeout(DEFAULT_TOTAL_TIMEOUT);

        builder = match self.tls {
            TlsMode::Native => builder,
            TlsMode::Pinned(pem) => {
                let cert = reqwest::Certificate::from_pem(&pem)
                    .map_err(|e| Error::Other(format!("invalid pinned certificate: {e}")))?;
                builder.add_root_certificate(cert)
            }
            #[cfg(feature = "dangerous-tls")]
            TlsMode::AcceptInvalid => builder.danger_accept_invalid_certs(true),
        };

        let http = builder.build()?;
        info!("ProtectClient built: base_url={base_url}, tls={tls_label}");
        debug!(
            "client timeouts: connect={DEFAULT_CONNECT_TIMEOUT:?}, total={DEFAULT_TOTAL_TIMEOUT:?}"
        );
        Ok(ProtectClient { http, base_url })
    }
}

fn parse_base_url(raw: &str) -> Result<Url> {
    let mut raw = raw.to_string();
    if !raw.ends_with('/') {
        raw.push('/');
    }
    Url::parse(&raw).map_err(|e| Error::InvalidUrl(format!("{raw}: {e}")))
}

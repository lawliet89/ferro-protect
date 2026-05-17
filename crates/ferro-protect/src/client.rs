//! `ProtectClient` -- the user-facing async client.

use std::time::Duration;

use log::{debug, info};
use reqwest::header::HeaderMap;
use secrecy::SecretString;

use crate::auth::{ApiKey, API_KEY_HEADER};
use crate::error::{Error, Result};
use crate::generated::Client as Inner;
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
    /// Crate-visible so per-entity API modules (`cameras.rs`, etc.) can
    /// call generated methods. Never exposed beyond the crate.
    pub(crate) inner: Inner,
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
        debug!("GET /v1/meta/info");
        let resp = self
            .inner
            .get_meta_info()
            .await
            .map_err(Error::from_progenitor)?;
        let info = resp.into_inner();
        info!(
            "fetched application info: version {}",
            info.application_version
        );
        Ok(info)
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
        let base_url = match (self.base_url, self.host) {
            (Some(url), _) => url,
            (None, Some(host)) => format!("https://{host}/proxy/protect/integration"),
            (None, None) => {
                return Err(Error::InvalidUrl(
                    "either host or base_url must be set".into(),
                ));
            }
        };

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
        let inner = Inner::new_with_client(&base_url, http);
        info!("ProtectClient built: base_url={base_url}, tls={tls_label}");
        debug!(
            "client timeouts: connect={DEFAULT_CONNECT_TIMEOUT:?}, total={DEFAULT_TOTAL_TIMEOUT:?}"
        );
        Ok(ProtectClient { inner })
    }
}

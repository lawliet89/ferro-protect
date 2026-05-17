//! API key handling. Keeps the secret in [`secrecy::SecretString`] so the
//! key never lands in a `Debug` impl by accident, and renders the
//! `X-API-Key` header value on demand.

use reqwest::header::{HeaderName, HeaderValue, InvalidHeaderValue};
use secrecy::{ExposeSecret, SecretString};

/// HTTP header the Protect integration API expects.
pub const API_KEY_HEADER: HeaderName = HeaderName::from_static("x-api-key");

#[derive(Clone)]
pub struct ApiKey(SecretString);

impl ApiKey {
    pub const fn new(key: SecretString) -> Self {
        Self(key)
    }

    pub fn header_value(&self) -> std::result::Result<HeaderValue, InvalidHeaderValue> {
        let mut value = HeaderValue::from_str(self.0.expose_secret())?;
        value.set_sensitive(true);
        Ok(value)
    }
}

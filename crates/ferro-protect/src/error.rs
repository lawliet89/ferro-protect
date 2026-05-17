//! Error type for the ferro-protect client.

use log::warn;
use reqwest::StatusCode;
use thiserror::Error;

/// Convenience alias for `Result<T, ferro_protect::Error>`.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned by [`ProtectClient`](crate::ProtectClient) operations.
#[derive(Debug, Error)]
pub enum Error {
    /// Transport-level HTTP failure: connection refused, TLS handshake error,
    /// timeout, etc. Carries the underlying `reqwest` error.
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    /// The server returned a structured error response. `status` is the HTTP
    /// status code, `code` is the API's symbolic error name (e.g.
    /// `"unauthorized"`), and `message` is its human-readable text.
    #[error("API error {status} ({code}): {message}")]
    Api {
        status: u16,
        code: String,
        message: String,
    },

    /// Response body did not match the expected JSON schema.
    #[error("Failed to deserialize response: {0}")]
    Json(#[from] serde_json::Error),

    /// A URL passed to or constructed by the client was not valid.
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    /// The builder was finalised without an API key.
    #[error("API key not provided")]
    MissingApiKey,

    /// Catch-all for anything that does not fit the variants above.
    /// Prefer adding a more specific variant when patterns emerge.
    #[error("{0}")]
    Other(String),
}

impl Error {
    pub(crate) async fn from_response(response: reqwest::Response) -> Self {
        let status = response.status();
        let raw = match response.text().await {
            Ok(body) => body,
            Err(e) => return Self::Http(e),
        };

        match serde_json::from_str::<ApiErrorBody>(&raw) {
            Ok(body) => {
                let code = body.name.clone();
                Self::api(status, code, body.message())
            }
            Err(e) => {
                warn!("API error body did not match expected shape: {e}");
                Self::api(status, "unknown", truncate_body(&raw))
            }
        }
    }

    fn api(status: StatusCode, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Api {
            status: status.as_u16(),
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(serde::Deserialize)]
struct ApiErrorBody {
    name: String,
    error: Option<String>,
    message: Option<String>,
}

impl ApiErrorBody {
    fn message(self) -> String {
        self.error
            .or(self.message)
            .unwrap_or_else(|| "(no message)".to_string())
    }
}

fn truncate_body(body: &str) -> String {
    const LIMIT: usize = 512;
    let mut truncated = String::new();
    for (idx, ch) in body.char_indices() {
        if idx >= LIMIT {
            truncated.push_str("...");
            return truncated;
        }
        truncated.push(ch);
    }
    truncated
}

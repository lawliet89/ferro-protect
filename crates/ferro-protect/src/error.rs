//! Error type for the ferro-protect client.

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
    /// Translate a progenitor-client error into our public `Error`.
    ///
    /// Generic over the error-body type `E` because each generated operation
    /// can declare its own error schema. We serialize the body to JSON and
    /// look up the `name`/`error` fields the Protect spec consistently uses;
    /// any other shape falls back to a stringified message.
    pub(crate) fn from_progenitor<E>(err: progenitor_client::Error<E>) -> Self
    where
        E: serde::Serialize,
    {
        use progenitor_client::Error as P;
        match err {
            P::CommunicationError(e) | P::ResponseBodyError(e) => Self::Http(e),
            P::InvalidUpgrade(e) => Self::Other(format!("websocket upgrade failed: {e}")),
            P::ErrorResponse(rv) => {
                let status = rv.status().as_u16();
                let body = rv.into_inner();
                let (code, message) = extract_code_and_message(&body);
                Self::Api {
                    status,
                    code,
                    message,
                }
            }
            P::InvalidResponsePayload(_, e) => Self::Json(e),
            P::UnexpectedResponse(resp) => {
                Self::Other(format!("unexpected response status {}", resp.status()))
            }
            P::InvalidRequest(s) | P::PreHookError(s) | P::PostHookError(s) => Self::Other(s),
        }
    }
}

fn extract_code_and_message<E: serde::Serialize>(body: &E) -> (String, String) {
    let value = serde_json::to_value(body).unwrap_or(serde_json::Value::Null);
    let code = value
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let message = value
        .get("error")
        .and_then(serde_json::Value::as_str)
        .or_else(|| value.get("message").and_then(serde_json::Value::as_str))
        .unwrap_or("(no message)")
        .to_string();
    (code, message)
}

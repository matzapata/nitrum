//! Errors returned by [`crate::NitrumClient`].

use thiserror::Error;

/// Client-side failure talking to the crypto API.
#[derive(Debug, Error)]
pub enum SdkError {
    /// Transport / reqwest failure.
    #[error("http client error: {0}")]
    Transport(#[from] reqwest::Error),
    /// Non-success HTTP status without a structured API error.
    #[error("http status {0}")]
    HttpStatus(u16),
    /// API returned `{ "error": "..." }`.
    #[error("{0}")]
    Api(String),
    /// Base64 or payload decode failure.
    #[error("decode error: {0}")]
    Decode(String),
}

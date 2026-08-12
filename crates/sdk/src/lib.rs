//! In-enclave client for the data-plane crypto HTTP API (`:3000`).

mod error;

pub use error::SdkError;

use base64::Engine;
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:3000";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// HTTP client for encrypt / decrypt / random / attestation.
#[derive(Debug, Clone)]
pub struct NitrumClient {
    /// Base URL of the crypto API (no trailing slash).
    base_url: String,
    /// Shared reqwest client.
    http: reqwest::Client,
}

impl Default for NitrumClient {
    fn default() -> Self {
        Self::new(DEFAULT_BASE_URL)
    }
}

impl NitrumClient {
    /// Create a client pointed at `base_url` (e.g. `http://127.0.0.1:3000`).
    ///
    /// # Panics
    ///
    /// Panics if the reqwest client cannot be built (should not happen with these defaults).
    #[must_use]
    pub fn new(base_url: impl Into<String>) -> Self {
        let base_url = base_url.into().trim_end_matches('/').to_owned();
        let http = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
            .build()
            .expect("reqwest client");
        Self { base_url, http }
    }

    /// `GET /health` — returns when the crypto API is up.
    pub async fn health(&self) -> Result<(), SdkError> {
        let res = self
            .http
            .get(format!("{}/health", self.base_url))
            .send()
            .await?;
        if !res.status().is_success() {
            return Err(SdkError::HttpStatus(res.status().as_u16()));
        }
        Ok(())
    }

    /// `POST /encrypt` — returns base64 ciphertext.
    pub async fn encrypt(&self, plaintext: &str) -> Result<String, SdkError> {
        self.post_data("/encrypt", json!({ "plaintext": plaintext }))
            .await
    }

    /// `POST /decrypt` — ciphertext is base64; returns UTF-8 plaintext.
    pub async fn decrypt(&self, ciphertext_b64: &str) -> Result<String, SdkError> {
        self.post_data("/decrypt", json!({ "ciphertext": ciphertext_b64 }))
            .await
    }

    /// `POST /random` — returns raw random bytes (`length` default 32, max 1024).
    pub async fn random(&self, length: usize) -> Result<Vec<u8>, SdkError> {
        let b64: String = self
            .post_data("/random", json!({ "length": length }))
            .await?;
        base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| SdkError::Decode(e.to_string()))
    }

    /// `POST /attestation` — optional base64 fields; returns base64 attestation document.
    pub async fn attestation(
        &self,
        nonce_b64: Option<&str>,
        public_key_b64: Option<&str>,
        user_data_b64: Option<&str>,
    ) -> Result<String, SdkError> {
        self.post_data(
            "/attestation",
            json!({
                "nonce": nonce_b64,
                "publicKey": public_key_b64,
                "userData": user_data_b64,
            }),
        )
        .await
    }

    async fn post_data<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<T, SdkError> {
        let res = self
            .http
            .post(format!("{}{path}", self.base_url))
            .json(&body)
            .send()
            .await?;
        let status = res.status();
        let envelope: ApiEnvelope<T> = res.json().await?;
        if let Some(err) = envelope.error {
            return Err(SdkError::Api(err));
        }
        if !status.is_success() {
            return Err(SdkError::HttpStatus(status.as_u16()));
        }
        envelope
            .data
            .ok_or_else(|| SdkError::Api("missing data in response".into()))
    }
}

#[derive(Debug, Deserialize)]
struct ApiEnvelope<T> {
    /// Success payload.
    data: Option<T>,
    /// Error string when the request failed.
    error: Option<String>,
}

//! Crypto API: DEK-backed encrypt/decrypt and Axum server (attestation, encrypt, decrypt).
//!
//! [`CryptoApi`] holds the DEK-backed [`Crypto`] and exposes [`CryptoApi::setup`] to bootstrap
//! from infra and [`CryptoApi::run`] to serve the HTTP API.

use std::sync::Arc;

use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use anyhow::{Context, Result};
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde::{Deserialize, Serialize};
use tracing::info;

use super::attest::get_attestation_doc;
use crate::storage::{keys, InfraClients};
use crate::state::DataPlaneState;

use super::rand;

// ── Crypto (inner) ───────────────────────────────────────────────────────────

/// Symmetric encryption using a Data Encryption Key (DEK).
/// Output format: `nonce (12 B) || ciphertext+tag`.
pub struct Crypto {
    dek: Vec<u8>,
}

impl Crypto {
    /// Encrypt `data` with AES-256-GCM. Returns `nonce (12 bytes) || ciphertext+tag`.
    pub fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        let key = Key::<Aes256Gcm>::from_slice(&self.dek);
        let cipher = Aes256Gcm::new(key);
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = cipher
            .encrypt(&nonce, data)
            .map_err(|e| anyhow::anyhow!("AES-GCM encrypt error: {e}"))?;
        let mut out = nonce.to_vec();
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    /// Decrypt a blob produced by [`Crypto::encrypt`].
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        const NONCE_LEN: usize = 12;
        anyhow::ensure!(data.len() > NONCE_LEN, "ciphertext too short");
        let (nonce_bytes, ciphertext) = data.split_at(NONCE_LEN);
        let key = Key::<Aes256Gcm>::from_slice(&self.dek);
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(nonce_bytes);
        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow::anyhow!("AES-GCM decrypt error: {e}"))
    }
}

// ── CryptoApi ─────────────────────────────────────────────────────────────────

/// Crypto API: DEK-backed crypto + HTTP server. Use [`CryptoApi::setup`] then [`CryptoApi::run`].
pub struct CryptoApi {
    pub crypto: Arc<Crypto>,
}

impl CryptoApi {
    /// Bootstrap DEK from infra (fetch from storage or create as leader), then return a [`CryptoApi`].
    pub async fn setup(infra: &InfraClients) -> Result<Self> {
        loop {
            if let Some(encrypted_dek) = infra
                .storage
                .get_object(keys::DEK_OBJECT_KEY)
                .await
                .context("failed to retrieve DEK from DynamoDB")?
            {
                tracing::info!("DEK found in DynamoDB, decrypting with KMS");
                let dek = infra
                    .kms
                    .decrypt_with_attestation(&encrypted_dek)
                    .await
                    .context("failed to decrypt DEK with KMS")?;
                return Ok(Self {
                    crypto: Arc::new(Crypto { dek }),
                });
            }

            if let Some(_guard) = infra.leader.try_acquire_leader().await? {
                tracing::info!("no DEK in DynamoDB, generating a new one (leader)");
                let dek = rand::random_key_32().context("failed to generate DEK")?;

                let encrypted_dek = infra
                    .kms
                    .encrypt(&dek)
                    .await
                    .context("failed to encrypt DEK with KMS")?;

                let stored = infra
                    .storage
                    .put_object(keys::DEK_OBJECT_KEY, &encrypted_dek)
                    .await
                    .context("failed to store DEK in DynamoDB")?;

                if stored {
                    tracing::info!("DEK stored successfully");
                    return Ok(Self {
                        crypto: Arc::new(Crypto { dek }),
                    });
                }
            } else {
                tracing::debug!("no DEK yet and not leader, retrying read");
            }

            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }

    /// Run the crypto API server (attestation, encrypt, decrypt) until the process exits.
    pub async fn run(self, state: Arc<DataPlaneState>) {
        let addr = crate::constants::API_LISTEN_ADDR;
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .unwrap_or_else(|e| panic!("failed to bind API server on {addr}: {e}"));

        info!(addr = %addr, "API server listening");

        axum::serve(
            listener,
            router(AppState {
                state,
                crypto: self.crypto,
            }),
        )
        .await
        .expect("API server error");
    }
}

// ── Axum state & handlers ────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub state: Arc<DataPlaneState>,
    pub crypto: Arc<Crypto>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttestationRequest {
    pub nonce: Option<String>,
    pub public_key: Option<String>,
    pub user_data: Option<String>,
}

#[derive(Serialize)]
pub struct AttestationResponse {
    pub document: String,
}

async fn attestation(Json(req): Json<AttestationRequest>) -> Json<AttestationResponse> {
    info!("attestation requested");
    let nonce = req.nonce.as_deref().and_then(|s| B64.decode(s).ok());
    let public_key = req.public_key.as_deref().and_then(|s| B64.decode(s).ok());
    let user_data = req.user_data.as_deref().and_then(|s| B64.decode(s).ok());
    let raw = get_attestation_doc(nonce, public_key, user_data).unwrap_or_else(|e| {
        tracing::error!(error = %e, "attestation failed");
        b"placeholder-attestation-document".to_vec()
    });
    Json(AttestationResponse {
        document: B64.encode(&raw),
    })
}

#[derive(Deserialize)]
pub struct EncryptRequest {
    pub plaintext: String,
}

#[derive(Serialize)]
pub struct EncryptResponse {
    pub ciphertext: String,
}

async fn encrypt(
    State(app): State<AppState>,
    Json(req): Json<EncryptRequest>,
) -> Result<Json<EncryptResponse>, (StatusCode, String)> {
    info!("encrypt requested");
    let plaintext = B64.decode(&req.plaintext).map_err(|e| {
        (StatusCode::BAD_REQUEST, format!("invalid base64 plaintext: {e}"))
    })?;
    let ciphertext = app.crypto.encrypt(&plaintext).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("encrypt error: {e}"))
    })?;
    Ok(Json(EncryptResponse {
        ciphertext: B64.encode(&ciphertext),
    }))
}

#[derive(Deserialize)]
pub struct DecryptRequest {
    pub ciphertext: String,
}

#[derive(Serialize)]
pub struct DecryptResponse {
    pub plaintext: String,
}

async fn decrypt(
    State(app): State<AppState>,
    Json(req): Json<DecryptRequest>,
) -> Result<Json<DecryptResponse>, (StatusCode, String)> {
    info!("decrypt requested");
    let ciphertext = B64.decode(&req.ciphertext).map_err(|e| {
        (StatusCode::BAD_REQUEST, format!("invalid base64 ciphertext: {e}"))
    })?;
    let plaintext = app.crypto.decrypt(&ciphertext).map_err(|e| {
        (StatusCode::UNPROCESSABLE_ENTITY, format!("decrypt error: {e}"))
    })?;
    Ok(Json(DecryptResponse {
        plaintext: B64.encode(&plaintext),
    }))
}

fn router(state: AppState) -> Router {
    Router::new()
        .route("/attestation", post(attestation))
        .route("/encrypt", post(encrypt))
        .route("/decrypt", post(decrypt))
        .with_state(state)
}

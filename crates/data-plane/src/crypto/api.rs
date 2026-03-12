//! Crypto API: DEK-backed encrypt/decrypt and Axum server (attestation, encrypt, decrypt).
//!
//! [`CryptoApi`] holds the DEK-backed [`Crypto`] and exposes [`CryptoApi::setup`] to bootstrap
//! from infra and [`CryptoApi::run`] to serve the HTTP API.

use std::sync::Arc;

use anyhow::Result;
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde::{Deserialize, Serialize};
use tracing::info;

use super::attest::get_attestation_doc;
use crate::state::DataPlaneState;

use super::client::CryptoClient;

// ── CryptoApi ─────────────────────────────────────────────────────────────────

/// Crypto API: DEK-backed crypto + HTTP server. Use [`CryptoApi::setup`] then [`CryptoApi::run`].
pub struct CryptoApi {
    pub crypto: Arc<CryptoClient>,
}

impl CryptoApi {
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
    pub crypto: Arc<CryptoClient>,
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

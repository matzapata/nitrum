//! Provide an HTTP API for the user process to use for encryption and decryption.

use anyhow::Result;
use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::info;

use super::attest::get_attestation_doc;
use crate::state::DataPlaneState;

/// Run the crypto API server (attestation, encrypt, decrypt) until the process exits.
pub async fn run(state: Arc<DataPlaneState>) {
    let addr = state.config.crypto_api_listen_addr.clone();
    let listener = tokio::net::TcpListener::bind(addr.clone())
        .await
        .unwrap_or_else(|e| panic!("failed to bind API server on {addr}: {e}"));

    info!(addr = %addr, "API server listening");

    // TODO: add randomness generator endpoint
    // TODO: add health check endpoint
    let router = Router::new()
        .route("/attestation", post(attestation))
        .route("/encrypt", post(encrypt))
        .route("/decrypt", post(decrypt))
        .with_state(state);

    axum::serve(listener, router)
        .await
        .expect("API server error");
}

// ── Axum state & handlers ────────────────────────────────────────────────────

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

async fn attestation(
    Json(req): Json<AttestationRequest>,
) -> Result<Json<AttestationResponse>, (StatusCode, String)> {
    let nonce = req.nonce.as_deref().and_then(|s| B64.decode(s).ok());
    let public_key = req.public_key.as_deref().and_then(|s| B64.decode(s).ok());
    let user_data = req.user_data.as_deref().and_then(|s| B64.decode(s).ok());

    let raw = get_attestation_doc(nonce, public_key, user_data).map_err(|e| {
        tracing::error!(error = %e, "attestation failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("attestation failed: {e}"),
        )
    })?;

    Ok(Json(AttestationResponse {
        document: B64.encode(&raw),
    }))
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
    State(state): State<Arc<DataPlaneState>>,
    Json(req): Json<EncryptRequest>,
) -> Result<Json<EncryptResponse>, (StatusCode, String)> {
    let plaintext = req.plaintext.as_bytes();
    let ciphertext = state.crypto.encrypt(plaintext).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("encrypt error: {e}"),
        )
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
    State(state): State<Arc<DataPlaneState>>,
    Json(req): Json<DecryptRequest>,
) -> Result<Json<DecryptResponse>, (StatusCode, String)> {
    let ciphertext = B64.decode(&req.ciphertext).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("invalid base64 ciphertext: {e}"),
        )
    })?;
    let plaintext_bytes = state.crypto.decrypt(&ciphertext).map_err(|e| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("decrypt error: {e}"),
        )
    })?;
    let plaintext = String::from_utf8(plaintext_bytes).map_err(|e| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("decrypted bytes not valid UTF-8: {e}"),
        )
    })?;
    Ok(Json(DecryptResponse { plaintext }))
}

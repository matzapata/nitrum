//! Internal API endpoints available to the enclave operator and SDK.

use std::sync::Arc;

use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::attestation::get_attestation_doc;
use crate::crypto::Crypto;

// ── shared state ─────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub crypto: Arc<Crypto>,
}

// ── /attestation ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttestationRequest {
    /// Caller-supplied nonce (base64-encoded bytes).
    pub nonce: Option<String>,
    /// Optional public key to bind into the attestation document (base64).
    pub public_key: Option<String>,
    /// Optional arbitrary data / challenge to embed as user_data (base64).
    pub user_data: Option<String>,
}

#[derive(Serialize)]
pub struct AttestationResponse {
    /// Base64-encoded COSE-signed attestation document from the NSM.
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

// ── /encrypt ─────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct EncryptRequest {
    /// Plaintext bytes, base64-encoded.
    pub plaintext: String,
}

#[derive(Serialize)]
pub struct EncryptResponse {
    /// `nonce || ciphertext+tag`, base64-encoded.
    pub ciphertext: String,
}

async fn encrypt(
    State(state): State<AppState>,
    Json(req): Json<EncryptRequest>,
) -> Result<Json<EncryptResponse>, (StatusCode, String)> {
    info!("encrypt requested");

    let plaintext = B64.decode(&req.plaintext).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("invalid base64 plaintext: {e}"),
        )
    })?;

    let ciphertext = state.crypto.encrypt(&plaintext).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("encrypt error: {e}"),
        )
    })?;

    Ok(Json(EncryptResponse {
        ciphertext: B64.encode(&ciphertext),
    }))
}

// ── /decrypt ─────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct DecryptRequest {
    /// `nonce || ciphertext+tag`, base64-encoded (output of `/encrypt`).
    pub ciphertext: String,
}

#[derive(Serialize)]
pub struct DecryptResponse {
    /// Recovered plaintext, base64-encoded.
    pub plaintext: String,
}

async fn decrypt(
    State(state): State<AppState>,
    Json(req): Json<DecryptRequest>,
) -> Result<Json<DecryptResponse>, (StatusCode, String)> {
    info!("decrypt requested");

    let ciphertext = B64.decode(&req.ciphertext).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("invalid base64 ciphertext: {e}"),
        )
    })?;

    let plaintext = state.crypto.decrypt(&ciphertext).map_err(|e| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("decrypt error: {e}"),
        )
    })?;

    Ok(Json(DecryptResponse {
        plaintext: B64.encode(&plaintext),
    }))
}

// ── router & runner ───────────────────────────────────────────────────────────

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/attestation", post(attestation))
        .route("/encrypt", post(encrypt))
        .route("/decrypt", post(decrypt))
        .with_state(state)
}

pub async fn run(crypto: Arc<Crypto>) {
    let addr = crate::constants::API_LISTEN_ADDR;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind API server on {addr}: {e}"));

    info!(addr = %addr, "API server listening");

    axum::serve(listener, router(AppState { crypto }))
        .await
        .expect("API server error");
}

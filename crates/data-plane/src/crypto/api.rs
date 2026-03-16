//! Provide internal api for crypto operations.

use anyhow::{Context, Result};
use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::{get, post}};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::info;
use super::attest::get_attestation_doc;
use crate::state::DataPlaneState;

/// Run the crypto API server (attestation, encrypt, decrypt) until the process exits.
pub async fn run(state: Arc<DataPlaneState>) -> anyhow::Result<()> {
    let addr = state.config.crypto_api_listen_addr;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind API server on {addr}"))?;

    info!(addr = %addr, "API server listening");

    let router = Router::new()
        .route("/health", get(health))
        .route("/random", post(random))
        .route("/attestation", post(attestation))
        .route("/encrypt", post(encrypt))
        .route("/decrypt", post(decrypt))
        .with_state(state);

    axum::serve(listener, router)
        .await
        .context("API server error")?;

    Ok(())
}

// ── Health ────────────────────────────────────────────────────

async fn health() -> impl IntoResponse {
    (StatusCode::OK, [("content-type", "application/json")], r#"{"status":"ok"}"#)
}

// ── Random ────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RandomRequest {
    /// Number of random bytes to generate (default 32, max 1024).
    pub byte_length: Option<usize>,
}

async fn random(
    req: Option<Json<RandomRequest>>,
) -> impl IntoResponse {
    let len = req
        .and_then(|r| r.byte_length)
        .unwrap_or(32)
        .min(1024);

    let bytes = super::random::rand_bytes(len).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("failed to generate random bytes: {e}"))
    })?;

    (StatusCode::OK, [("content-type", "application/json")], format!(r#"{{"random":"{}"}}"#, B64.encode(&bytes)))
}

// ── Attestation ────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttestationRequest {
    pub nonce: Option<String>,
    pub public_key: Option<String>,
    pub user_data: Option<String>,
}


async fn attestation(
    Json(req): Json<AttestationRequest>,
) -> impl IntoResponse {
    let nonce = req.nonce.as_deref().and_then(|s| B64.decode(s).ok());
    let public_key = req.public_key.as_deref().and_then(|s| B64.decode(s).ok());
    let user_data = req.user_data.as_deref().and_then(|s| B64.decode(s).ok());

    let raw = get_attestation_doc(nonce, public_key, user_data).map_err(|e| {
        tracing::error!(error = %e, "attestation failed");
        (StatusCode::INTERNAL_SERVER_ERROR, format!("attestation failed: {e}"))
    })?;

    (StatusCode::OK, [("content-type", "application/json")], format!(r#"{{"document":"{}"}}"#, B64.encode(&raw)))
}

// ── Encrypt ────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct EncryptRequest {
    pub plaintext: String,
}


async fn encrypt(
    State(state): State<Arc<DataPlaneState>>,
    Json(req): Json<EncryptRequest>,
) -> impl IntoResponse {
    let plaintext = req.plaintext.as_bytes();
    let ciphertext = state.crypto.encrypt(plaintext).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("encrypt error: {e}"))
    })?;
    
    (StatusCode::OK, [("content-type", "application/json")], format!(r#"{{"ciphertext":"{}"}}"#, B64.encode(&ciphertext)))
}

// ── Decrypt ────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct DecryptRequest {
    pub ciphertext: String,
}

async fn decrypt(
    State(state): State<Arc<DataPlaneState>>,
    Json(req): Json<DecryptRequest>,
) -> impl IntoResponse {
    let ciphertext = B64.decode(&req.ciphertext).map_err(|e| {
        (StatusCode::BAD_REQUEST, format!("invalid base64 ciphertext: {e}"))
    })?;
    let plaintext_bytes = state.crypto.decrypt(&ciphertext).map_err(|e| {
        (StatusCode::UNPROCESSABLE_ENTITY, format!("decrypt error: {e}"))
    })?;
    let plaintext = String::from_utf8(plaintext_bytes).map_err(|e| {
        (StatusCode::UNPROCESSABLE_ENTITY, format!("decrypted bytes not valid UTF-8: {e}"))
    })?;
    
    (StatusCode::OK, [("content-type", "application/json")], format!(r#"{{"plaintext":"{}"}}"#, plaintext))
}

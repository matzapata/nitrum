//! Provide internal api for crypto operations.

use super::attest::get_attestation_doc;
use super::kv::{EnclaveKvStore, KvStoreError};
use crate::state::DataPlaneState;
use anyhow::Context;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use tracing::{info, warn};

/// Run the crypto API server (attestation, encrypt, decrypt, KV) until the process exits.
pub async fn run(state: Arc<DataPlaneState>) -> anyhow::Result<()> {
    let addr = state.config.listen_addrs.crypto_api_listen_addr;
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
        .route("/kv/set", post(kv_set))
        .route("/kv/get", post(kv_get))
        .with_state(state);
    let router = telemetry::http::instrument_router(router, "data-plane.crypto-api");

    axum::serve(listener, router)
        .await
        .context("API server error")?;

    Ok(())
}

// ── Health ────────────────────────────────────────────────────

async fn health() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        json!({ "status": "ok" }).to_string(),
    )
}

// ── Random ────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RandomRequest {
    /// Number of random bytes to generate (default 32, max 1024).
    pub length: Option<usize>,
}

async fn random(req: Option<Json<RandomRequest>>) -> impl IntoResponse {
    let len = req.and_then(|r| r.length).unwrap_or(32).min(1024);

    let bytes = match super::random::rand_bytes(len) {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [("content-type", "application/json")],
                json!({ "data": null, "error": format!("failed to generate random bytes: {e}") })
                    .to_string(),
            )
                .into_response();
        }
    };

    (
        StatusCode::OK,
        [("content-type", "application/json")],
        json!({ "data": B64.encode(&bytes), "error": null }).to_string(),
    )
        .into_response()
}

// ── Attestation ────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttestationRequest {
    pub nonce: Option<String>,
    pub public_key: Option<String>,
    pub user_data: Option<String>,
}

async fn attestation(Json(req): Json<AttestationRequest>) -> impl IntoResponse {
    let nonce = req.nonce.as_deref().and_then(|s| B64.decode(s).ok());
    let public_key = req.public_key.as_deref().and_then(|s| B64.decode(s).ok());
    let user_data = req.user_data.as_deref().and_then(|s| B64.decode(s).ok());

    let raw = match get_attestation_doc(nonce, public_key, user_data) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "attestation failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [("content-type", "application/json")],
                json!({ "data": null, "error": format!("attestation failed: {e}") }).to_string(),
            )
                .into_response();
        }
    };

    (
        StatusCode::OK,
        [("content-type", "application/json")],
        json!({ "data": B64.encode(&raw), "error": null }).to_string(),
    )
        .into_response()
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
    let ciphertext = match state.crypto.encrypt(plaintext) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [("content-type", "application/json")],
                json!({ "data": null, "error": format!("encrypt error: {e}") }).to_string(),
            )
                .into_response();
        }
    };

    (
        StatusCode::OK,
        [("content-type", "application/json")],
        json!({ "data": B64.encode(&ciphertext), "error": null }).to_string(),
    )
        .into_response()
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
    let ciphertext = match B64.decode(&req.ciphertext) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [("content-type", "application/json")],
                json!({ "data": null, "error": format!("invalid base64 ciphertext: {e}") })
                    .to_string(),
            )
                .into_response();
        }
    };
    let plaintext_bytes = match state.crypto.decrypt(&ciphertext) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                [("content-type", "application/json")],
                json!({ "data": null, "error": format!("decrypt error: {e}") }).to_string(),
            )
                .into_response();
        }
    };
    let plaintext = match String::from_utf8(plaintext_bytes) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                [("content-type", "application/json")],
                json!({ "data": null, "error": format!("decrypted bytes not valid UTF-8: {e}") })
                    .to_string(),
            )
                .into_response();
        }
    };

    (
        StatusCode::OK,
        [("content-type", "application/json")],
        json!({ "data": plaintext, "error": null }).to_string(),
    )
        .into_response()
}

// ── KV storage (DEK-wrapped values in DynamoDB) ────────────────────────────

#[derive(Deserialize)]
struct KvSetRequest {
    /// Logical key (namespaced to `kv:` in storage; validated).
    key: String,
    /// UTF-8 plaintext stored encrypted at rest.
    value: String,
}

#[derive(Deserialize)]
struct KvGetRequest {
    /// Logical key previously passed to `/kv/set`.
    key: String,
}

async fn kv_set(
    State(state): State<Arc<DataPlaneState>>,
    Json(req): Json<KvSetRequest>,
) -> impl IntoResponse {
    let store = EnclaveKvStore::new(state.storage.clone(), state.crypto.clone());
    if let Err(e) = store.set(&req.key, &req.value).await {
        return kv_error_response(e);
    }

    (
        StatusCode::OK,
        [("content-type", "application/json")],
        json!({ "data": "ok", "error": null }).to_string(),
    )
        .into_response()
}

async fn kv_get(
    State(state): State<Arc<DataPlaneState>>,
    Json(req): Json<KvGetRequest>,
) -> impl IntoResponse {
    let store = EnclaveKvStore::new(state.storage.clone(), state.crypto.clone());
    let value = match store.get(&req.key).await {
        Ok(v) => v,
        Err(e) => return kv_error_response(e),
    };

    (
        StatusCode::OK,
        [("content-type", "application/json")],
        json!({ "data": value, "error": null }).to_string(),
    )
        .into_response()
}

fn kv_error_response(err: KvStoreError) -> axum::response::Response {
    let (status, msg) = match &err {
        KvStoreError::BadRequest(m) => (StatusCode::BAD_REQUEST, m.clone()),
        KvStoreError::NotFound => (StatusCode::NOT_FOUND, err.to_string()),
        KvStoreError::Unprocessable(m) => {
            warn!(error = %m, "kv request failed (unprocessable)");
            (StatusCode::UNPROCESSABLE_ENTITY, m.clone())
        }
        KvStoreError::Internal(e) => {
            warn!(error = %format!("{e:#}"), "kv request failed (internal)");
            (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
        }
    };
    (
        status,
        [("content-type", "application/json")],
        json!({ "data": null, "error": msg }).to_string(),
    )
        .into_response()
}

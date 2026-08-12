//! Internal HTTP server for crypto operations.

use super::attest::get_attestation_doc;
use super::dek::Crypto;
use crate::DataPlaneConfig;
use anyhow::Context;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tracing::{info, warn};

/// Dependencies required by the crypto API server and its handlers.
#[derive(Clone)]
pub(crate) struct CryptoApiState<C: Crypto> {
    /// Crypto client for encrypting/decrypting data.
    crypto: Arc<C>,
}

/// Spawn the crypto API server in the background.
pub fn init<C: Crypto + 'static>(config: &DataPlaneConfig, crypto: &Arc<C>) {
    let state = Arc::new(CryptoApiState {
        crypto: crypto.clone(),
    });
    let listen_addr = config.listen_addrs.crypto_api_listen_addr;
    tokio::spawn(async move {
        info!("crypto API task starting");
        if let Err(e) = run(state, listen_addr).await {
            warn!(error = %e, "crypto API task failed");
        }
        warn!("crypto API task exited");
    });
}

/// Run the crypto API server (attestation, encrypt, decrypt) until the process exits.
async fn run<C: Crypto + 'static>(
    state: Arc<CryptoApiState<C>>,
    addr: std::net::SocketAddr,
) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind API server on {addr}"))?;

    info!(addr = %addr, "API server listening");

    axum::serve(listener, build_router(state))
        .await
        .context("API server error")?;

    Ok(())
}

/// Build the crypto API router (attestation, encrypt, decrypt, random, health).
pub(crate) fn build_router<C: Crypto + 'static>(state: Arc<CryptoApiState<C>>) -> Router {
    let router = Router::new()
        .route("/health", get(health_handler))
        .route("/random", post(random_handler))
        .route("/attestation", post(attestation_handler))
        .route("/encrypt", post(encrypt_handler::<C>))
        .route("/decrypt", post(decrypt_handler::<C>))
        .with_state(state);
    telemetry::http::instrument_router(router, "data-plane.crypto-api")
}

/// API-layer error for crypto handlers (HTTP status + JSON envelope).
enum CryptoApiError {
    BadRequest(String),
    Unprocessable(String),
    Internal { msg: &'static str, detail: String },
}

impl CryptoApiError {
    fn internal(msg: &'static str, err: impl std::fmt::Display) -> Self {
        Self::Internal {
            msg,
            detail: err.to_string(),
        }
    }
}

impl IntoResponse for CryptoApiError {
    fn into_response(self) -> Response {
        let (status, client_msg) = match &self {
            Self::BadRequest(m) => (StatusCode::BAD_REQUEST, m.clone()),
            Self::Unprocessable(m) => {
                warn!(error = %m, "crypto api error");
                (StatusCode::UNPROCESSABLE_ENTITY, m.clone())
            }
            Self::Internal { msg, detail } => {
                warn!(error = %detail, "crypto api error");
                (StatusCode::INTERNAL_SERVER_ERROR, (*msg).to_string())
            }
        };

        (
            status,
            [("content-type", "application/json")],
            json!({ "data": null, "error": client_msg }).to_string(),
        )
            .into_response()
    }
}

fn json_ok(data: impl Serialize) -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        json!({ "data": data, "error": null }).to_string(),
    )
}

// ── Health ────────────────────────────────────────────────────

async fn health_handler() -> impl IntoResponse {
    json_ok("ok")
}

// ── Random ────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RandomRequest {
    /// Number of random bytes to generate (default 32, max 1024).
    pub length: Option<usize>,
}

async fn random_handler(req: Option<Json<RandomRequest>>) -> impl IntoResponse {
    let len = req.and_then(|r| r.length).unwrap_or(32).min(1024);
    match super::random::rand_bytes(len) {
        Ok(bytes) => json_ok(B64.encode(&bytes)).into_response(),
        Err(e) => CryptoApiError::internal("failed to generate random bytes", e).into_response(),
    }
}

// ── Attestation ────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttestationRequest {
    pub nonce: Option<String>,
    pub public_key: Option<String>,
    pub user_data: Option<String>,
}

async fn attestation_handler(Json(req): Json<AttestationRequest>) -> impl IntoResponse {
    let nonce = req.nonce.as_deref().and_then(|s| B64.decode(s).ok());
    let public_key = req.public_key.as_deref().and_then(|s| B64.decode(s).ok());
    let user_data = req.user_data.as_deref().and_then(|s| B64.decode(s).ok());

    match get_attestation_doc(nonce, public_key, user_data) {
        Ok(raw) => json_ok(B64.encode(&raw)).into_response(),
        Err(e) => CryptoApiError::internal("attestation failed", e).into_response(),
    }
}

// ── Encrypt ────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct EncryptRequest {
    pub plaintext: String,
}

async fn encrypt_handler<C: Crypto>(
    State(state): State<Arc<CryptoApiState<C>>>,
    Json(req): Json<EncryptRequest>,
) -> impl IntoResponse {
    match state.crypto.encrypt(req.plaintext.as_bytes()) {
        Ok(ciphertext) => json_ok(B64.encode(&ciphertext)).into_response(),
        Err(e) => CryptoApiError::internal("encrypt error", e).into_response(),
    }
}

// ── Decrypt ────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct DecryptRequest {
    pub ciphertext: String,
}

async fn decrypt_handler<C: Crypto>(
    State(state): State<Arc<CryptoApiState<C>>>,
    Json(req): Json<DecryptRequest>,
) -> impl IntoResponse {
    let ciphertext = match B64.decode(&req.ciphertext) {
        Ok(c) => c,
        Err(e) => {
            return CryptoApiError::BadRequest(format!("invalid base64 ciphertext: {e}"))
                .into_response();
        }
    };
    let plaintext_bytes = match state.crypto.decrypt(&ciphertext) {
        Ok(p) => p,
        Err(e) => {
            return CryptoApiError::Unprocessable(format!("decrypt error: {e}")).into_response();
        }
    };
    match String::from_utf8(plaintext_bytes) {
        Ok(plaintext) => json_ok(plaintext).into_response(),
        Err(e) => CryptoApiError::Unprocessable(format!("decrypted bytes not valid UTF-8: {e}"))
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::AesGcmCrypto;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use serde_json::Value;
    use tower::ServiceExt;

    const DEK: [u8; 32] = [0x42; 32];

    fn router() -> Router {
        build_router(Arc::new(CryptoApiState {
            crypto: Arc::new(AesGcmCrypto::from_dek(&DEK).expect("valid DEK")),
        }))
    }

    async fn json_body(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        serde_json::from_slice(&body).expect("json body")
    }

    #[tokio::test]
    async fn health_ok() {
        let response = router()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = json_body(response).await;
        assert_eq!(body["data"], "ok");
        assert!(body["error"].is_null());
    }

    #[tokio::test]
    #[cfg(not(feature = "enclave"))]
    async fn random_default_length() {
        let response = router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/random")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = json_body(response).await;
        assert!(body["error"].is_null());

        let raw = B64.decode(body["data"].as_str().unwrap()).unwrap();
        assert_eq!(raw.len(), 32);
    }

    #[tokio::test]
    #[cfg(not(feature = "enclave"))]
    async fn random_clamps_to_1024() {
        let response = router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/random")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"length":4096}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = json_body(response).await;
        let raw = B64.decode(body["data"].as_str().unwrap()).unwrap();
        assert_eq!(raw.len(), 1024);
    }

    #[tokio::test]
    async fn encrypt_decrypt_round_trip() {
        let crypto = Arc::new(AesGcmCrypto::from_dek(&DEK).expect("valid DEK"));

        let encrypt_resp = build_router(Arc::new(CryptoApiState {
            crypto: crypto.clone(),
        }))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/encrypt")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"plaintext":"hello-enclave"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(encrypt_resp.status(), StatusCode::OK);
        let enc_body = json_body(encrypt_resp).await;
        let ciphertext = enc_body["data"].as_str().unwrap().to_string();

        let decrypt_resp = build_router(Arc::new(CryptoApiState { crypto }))
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/decrypt")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"ciphertext":"{ciphertext}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(decrypt_resp.status(), StatusCode::OK);

        let dec_body = json_body(decrypt_resp).await;
        assert_eq!(dec_body["data"], "hello-enclave");
        assert!(dec_body["error"].is_null());
    }

    #[tokio::test]
    async fn decrypt_rejects_invalid_base64() {
        let response = router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/decrypt")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"ciphertext":"@@@not-base64@@@"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = json_body(response).await;
        assert!(body["error"].as_str().unwrap().contains("invalid base64"));
    }

    #[tokio::test]
    #[cfg(not(feature = "enclave"))]
    async fn attestation_returns_placeholder_with_nonce() {
        let nonce = B64.encode(b"test-nonce");
        let response = router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/attestation")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"nonce":"{nonce}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = json_body(response).await;
        let doc = B64.decode(body["data"].as_str().unwrap()).unwrap();
        let doc_str = String::from_utf8(doc).unwrap();

        assert!(doc_str.starts_with("placeholder-attestation-document,"));
        assert!(doc_str.contains(&hex::encode(b"test-nonce")));
    }
}

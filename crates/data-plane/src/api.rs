use axum::{Json, Router, routing::post};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::info;
use crate::attestation::get_attestation_doc;

// ── /attestation ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttestationRequest {
    /// Caller-supplied nonce (base64-encoded bytes). When absent the NSM
    /// generates one via GetRandom.
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

    let raw = get_attestation_doc(nonce, public_key, user_data)
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "attestation failed");
            b"placeholder-attestation-document".to_vec()
        });

    Json(AttestationResponse {
        document: B64.encode(&raw),
    })
}


// ── /encrypt ──────────────────────────────────────────────────────────────────

async fn encrypt(Json(body): Json<Value>) -> Json<Value> {
    info!("encrypt requested");
    Json(body)
}

// ── /decrypt ──────────────────────────────────────────────────────────────────

async fn decrypt(Json(body): Json<Value>) -> Json<Value> {
    info!("decrypt requested");
    Json(body)
}

// ── router & runner ───────────────────────────────────────────────────────────

pub fn router() -> Router {
    Router::new()
        .route("/attestation", post(attestation))
        .route("/encrypt", post(encrypt))
        .route("/decrypt", post(decrypt))
}

pub async fn run(addr: String) {
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind API server on {addr}: {e}"));

    info!(addr = %addr, "API server listening");

    axum::serve(listener, router())
        .await
        .expect("API server error");
}

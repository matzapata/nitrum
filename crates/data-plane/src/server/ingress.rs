//! Ingress proxy: Responsible for terminating TLS connections, serving well-known
//! enclave endpoints and ACME challenges, and proxying all other traffic to the
//! user application.
//!
//! Optional well-known paths (enabled via `[well_known]` in `nitrum.toml`) are handled directly
//! (the user app is not invoked):
//!   - GET /.well-known/enclave/status
//!     -> Responds with 200 {"status":"ok"}
//!   - GET /.well-known/enclave/attestation
//!     -> Responds with 200 and base64-encoded attestation document
//!     Optional query: `nonce` — standard base64 of raw nonce bytes (same encoding as the crypto API)
//!   - GET /.well-known/acme-challenge/*
//!     -> Responds with 200 and the ACME HTTP-01 key authorization string
//!
//! The server exposes both a plain HTTP listener (default: port 80, for ACME HTTP-01
//! validation) and a TLS listener (default: port 443) using the same Axum router.
//! TLS config is provided by `tls::build_tls_config`. The certificate is renewed in
//! the background and can be reloaded without restarting the ingress server.

use super::tls::{TlsState, challenge_handler};
use crate::crypto::get_attestation_doc;
use crate::state::DataPlaneState;
use anyhow::Context;
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Query, State},
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use axum_server::bind;
use axum_server::tls_rustls::bind_rustls;
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use serde::Deserialize;
use std::sync::Arc;
use tracing::{Instrument, info, warn};

// ── public entry point ────────────────────────────────────────────────────────

/// Runs the ingress server (HTTP for ACME, HTTPS for app traffic). Returns when
/// either server stops (e.g. bind/serve error) or an error occurs.
pub async fn run(state: Arc<DataPlaneState>) -> anyhow::Result<()> {
    let (tls_config, challenge_server) = if state.config.tls_termination.acme {
        // ACME HTTP-01 challenge handler
        let acme_router = Router::new()
            .route(
                "/.well-known/acme-challenge/{token}",
                get(challenge_handler),
            )
            .fallback(|_: Request<Body>| async { (StatusCode::NOT_FOUND, "Not found") })
            .with_state(state.clone());

        // TLS state machine
        let mut tls_state = TlsState::new(state.clone());
        let tls_config = tls_state.rustls_config();

        // Drive the ACME state machine
        tokio::spawn(async move {
            loop {
                match tls_state.next().await {
                    Ok(event) => tracing::info!(acme_event = ?event, "ACME state transition"),
                    Err(err) => tracing::error!(
                        error.kind = "acme_state",
                        error = %err,
                        "ACME state machine error"
                    ),
                }
            }
        });

        // Bind the ACME HTTP-01 server
        let acme_http_addr = state.config.acme_http01_listen_addr;
        info!(acme_http_addr = %acme_http_addr, "ACME HTTP-01 server listening");
        let challenge_server = bind(acme_http_addr).serve(acme_router.into_make_service());

        (tls_config, Some(challenge_server))
    } else {
        let tls_config = TlsState::new(state.clone()).rustls_config();
        (tls_config, None)
    };

    // HTTPS router
    let mut https_router = Router::new();
    if state.config.well_known.enclave_status {
        https_router = https_router.route("/.well-known/enclave/status", get(ingress_status));
    }
    if state.config.well_known.enclave_attestation {
        https_router =
            https_router.route("/.well-known/enclave/attestation", get(ingress_attestation));
    }
    let https_router = https_router
        .layer(middleware::from_fn(request_id_middleware))
        .fallback(ingress_proxy)
        .with_state(state.clone());

    // Bind the ingress server
    let ingress_addr = state.config.ingress_listen_addr;
    info!(ingress = %ingress_addr, "ingress listening");
    let ingress = bind_rustls(ingress_addr, tls_config).serve(https_router.into_make_service());

    // Wait for the servers to stop
    match challenge_server {
        Some(http) => {
            tokio::select! {
                r = http => r.context("HTTP server stopped")?,
                r = ingress => r.context("Ingress server stopped")?,
            }
        }
        None => ingress.await.context("Ingress server stopped")?,
    }
    Ok(())
}

// ── handlers ─────────────────────────────────────────────────────────────────

async fn request_id_middleware(request: Request<Body>, next: Next) -> Response {
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let span = tracing::info_span!("ingress", request_id = %request_id);
    next.run(request).instrument(span).await
}

#[tracing::instrument(name = "ingress_status", skip_all)]
async fn ingress_status() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        r#"{"status":"ok"}"#,
    )
}

#[derive(Deserialize)]
struct IngressAttestationQuery {
    nonce: Option<String>,
}

#[tracing::instrument(name = "ingress_attestation", skip(state, q))]
async fn ingress_attestation(
    State(state): State<Arc<DataPlaneState>>,
    Query(q): Query<IngressAttestationQuery>,
) -> impl IntoResponse {
    let cert_hash = state.tls_cert_hash.read().unwrap().clone();
    let public_key = match cert_hash {
        Some(h) => Some(h),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [("content-type", "application/json")],
                r#"{"error":"TLS certificate not yet available"}"#,
            )
                .into_response();
        }
    };

    let nonce = q.nonce.as_deref().and_then(|s| B64.decode(s).ok());

    match get_attestation_doc(nonce, public_key, None) {
        Ok(doc) => (
            StatusCode::OK,
            [("content-type", "application/json")],
            format!(r#"{{"data":"{}"}}"#, B64.encode(&doc)),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error.kind = "attestation", error = %e, "attestation failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [("content-type", "application/json")],
                format!(r#"{{"error":"attestation failed: {e}"}}"#),
            )
                .into_response()
        }
    }
}

#[tracing::instrument(name = "ingress_proxy", skip(state, req), fields(url))]
async fn ingress_proxy(
    State(state): State<Arc<DataPlaneState>>,
    req: Request<Body>,
) -> impl IntoResponse {
    let path_and_query = req
        .uri()
        .path_and_query()
        .map_or("/", axum::http::uri::PathAndQuery::as_str);
    let forward_to = format!("127.0.0.1:{}", state.config.nitrum.project.port);
    let url = format!("http://{forward_to}{path_and_query}");
    tracing::Span::current().record("url", url.as_str());
    info!("ingress: proxying to app");

    let client = match reqwest::Client::builder().build() {
        Ok(c) => c,
        Err(e) => {
            warn!(error.kind = "ingress_proxy", error = %e, "ingress: failed to create reqwest client");
            return (StatusCode::BAD_GATEWAY, "proxy client error").into_response();
        }
    };

    let (parts, body) = req.into_parts();
    let body_bytes = match to_bytes(body, 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            warn!(error.kind = "ingress_proxy", error = %e, "ingress: failed to read request body");
            return (StatusCode::BAD_REQUEST, "body read error").into_response();
        }
    };

    let mut backend_req = client
        .request(parts.method.clone(), &url)
        .body(body_bytes.to_vec());
    for (name, value) in &parts.headers {
        let name_str = name.as_str();
        if name_str.eq_ignore_ascii_case("connection")
            || name_str.eq_ignore_ascii_case("keep-alive")
            || name_str.eq_ignore_ascii_case("transfer-encoding")
        {
            continue;
        }
        if let Ok(v) = value.to_str() {
            backend_req = backend_req.header(name_str, v);
        }
    }

    let backend_resp = match backend_req.send().await {
        Ok(r) => r,
        Err(e) => {
            warn!(error.kind = "ingress_proxy", error = %e, "ingress: proxy request failed");
            return (StatusCode::BAD_GATEWAY, "backend unreachable").into_response();
        }
    };

    let status = backend_resp.status();
    let headers = backend_resp.headers().clone();
    let body = match backend_resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            warn!(error.kind = "ingress_proxy", error = %e, "ingress: failed to read backend body");
            return (StatusCode::BAD_GATEWAY, "backend body error").into_response();
        }
    };

    let mut resp = (status, body).into_response();
    for (name, value) in &headers {
        resp.headers_mut().insert(name.clone(), value.clone());
    }

    resp
}

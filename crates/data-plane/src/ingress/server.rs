//! Ingress HTTP/HTTPS server: TLS termination, well-known endpoints, and app proxying.

use super::state::IngressState;
use super::tls::{TlsState, challenge_handler};
use crate::DataPlaneConfig;
use crate::crypto::{CryptoClient, get_attestation_doc};
use crate::storage::StorageClient;
use anyhow::Context;
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Query, State},
    http::{Request, StatusCode},
    response::IntoResponse,
    routing::get,
};
use axum_server::bind;
use axum_server::tls_rustls::bind_rustls;
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use serde::Deserialize;
use std::sync::{Arc, RwLock};
use tracing::{info, warn};

/// Spawn the ingress server in the background.
pub fn init(config: &DataPlaneConfig, storage: &Arc<StorageClient>, crypto: &Arc<CryptoClient>) {
    let state = Arc::new(IngressState {
        config: config.clone(),
        storage: storage.clone(),
        proxy_client: reqwest::Client::new(),
        tls_cert_hash: Arc::new(RwLock::new(None)),
    });
    let crypto = crypto.clone();
    tokio::spawn(async move {
        info!("ingress task starting");
        if let Err(e) = run(state, crypto).await {
            warn!(error = %e, "ingress task failed");
        }
        warn!("ingress task exited");
    });
}

/// Runs the ingress server (HTTP for ACME, HTTPS for app traffic). Returns when
/// either server stops (e.g. bind/serve error) or an error occurs.
async fn run(state: Arc<IngressState>, crypto: Arc<CryptoClient>) -> anyhow::Result<()> {
    let (tls_config, challenge_server) = if state.config.tls_termination.acme {
        // ACME HTTP-01 challenge handler
        let acme_router = Router::new()
            .route(
                "/.well-known/acme-challenge/{token}",
                get(challenge_handler),
            )
            .fallback(|_: Request<Body>| async { (StatusCode::NOT_FOUND, "Not found") })
            .with_state(state.clone());
        let acme_router = telemetry::http::instrument_router(acme_router, "data-plane.ingress");

        // TLS state machine
        let mut tls_state = TlsState::new(state.clone(), crypto.clone());
        let tls_config = tls_state.rustls_config();

        // Drive the ACME state machine
        tokio::spawn(async move {
            loop {
                match tls_state.next().await {
                    Ok(ok) => tracing::info!("event: {ok:?}"),
                    Err(err) => tracing::error!("error: {err:?}"),
                }
            }
        });

        // Bind the ACME HTTP-01 server
        let acme_http_addr = state.config.listen_addrs.acme_http01_listen_addr;
        info!(acme_http_addr = %acme_http_addr, "ACME HTTP-01 server listening");
        let challenge_server = bind(acme_http_addr).serve(acme_router.into_make_service());

        (tls_config, Some(challenge_server))
    } else {
        let tls_config = TlsState::new(state.clone(), crypto).rustls_config();
        (tls_config, None)
    };

    let https_router =
        telemetry::http::instrument_router(build_https_router(state.clone()), "data-plane.ingress");

    // Bind the ingress server
    let ingress_addr = state.config.listen_addrs.ingress_listen_addr;
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

/// Build the HTTPS ingress router (well-known routes + proxy fallback).
#[doc(hidden)]
pub fn build_https_router(state: Arc<IngressState>) -> Router {
    let mut https_router = Router::new();
    if state.config.well_known.enclave_status {
        https_router = https_router.route("/.well-known/enclave/status", get(ingress_status));
    }
    if state.config.well_known.enclave_attestation {
        https_router =
            https_router.route("/.well-known/enclave/attestation", get(ingress_attestation));
    }
    https_router.fallback(ingress_proxy).with_state(state)
}

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

async fn ingress_attestation(
    State(state): State<Arc<IngressState>>,
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
            tracing::error!(error = %e, "attestation failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [("content-type", "application/json")],
                format!(r#"{{"error":"attestation failed: {e}"}}"#),
            )
                .into_response()
        }
    }
}

async fn ingress_proxy(
    State(state): State<Arc<IngressState>>,
    req: Request<Body>,
) -> impl IntoResponse {
    let path_and_query = req
        .uri()
        .path_and_query()
        .map_or("/", axum::http::uri::PathAndQuery::as_str);
    let forward_to = format!("127.0.0.1:{}", state.config.nitrum.project.port.get());
    let url = format!("http://{forward_to}{path_and_query}");
    info!(url = %url, "ingress: proxying to app");

    let (parts, body) = req.into_parts();
    let body_bytes = match to_bytes(body, 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            warn!(error = %e, "ingress: failed to read request body");
            return (StatusCode::BAD_REQUEST, "body read error").into_response();
        }
    };

    let mut backend_req = state
        .proxy_client
        .request(parts.method.clone(), &url)
        .body(body_bytes);
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
            warn!(url = %url, error = %e, "ingress: proxy request failed");
            return (StatusCode::BAD_GATEWAY, "backend unreachable").into_response();
        }
    };

    let status = backend_resp.status();
    let headers = backend_resp.headers().clone();
    let body = match backend_resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            warn!(error = %e, "ingress: failed to read backend body");
            return (StatusCode::BAD_GATEWAY, "backend body error").into_response();
        }
    };

    let mut resp = (status, body).into_response();
    for (name, value) in &headers {
        resp.headers_mut().insert(name.clone(), value.clone());
    }

    resp
}

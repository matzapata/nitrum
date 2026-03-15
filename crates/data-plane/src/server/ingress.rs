//! Ingress proxy: terminates TLS, serves enclave well-known endpoints and ACME
//! challenges directly, and forwards everything else to the user application.
//!
//! Well-known paths served without touching the user app:
//!   GET /.well-known/enclave/status      → 200 {"status":"ok"}
//!   GET /.well-known/enclave/attestation → 200 base64-encoded attestation
//!   GET /.well-known/acme-challenge/*    → 200 key_authorization (for ACME HTTP-01)
//!
//! Both the plain-HTTP listener (port 80, for ACME HTTP-01 validation) and the
//! TLS listener (port 443) serve the same router. The TLS listener uses the
//! `RustlsConfig` built by `tls::build_tls_config`; the cert is hot-reloaded
//! in the background renewal loop without restarting the server.

use crate::state::DataPlaneState;
use crate::storage::keys;
use crate::utils::leader::Leader;
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Path, State},
    http::{Request, StatusCode},
    response::IntoResponse,
    routing::get,
};
use axum_server::bind;
use axum_server::tls_rustls::bind_rustls;
use bytes::Bytes;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{info, warn};

/// Shared app state available to all handlers.
#[derive(Clone)]
struct AppState {
    state: Arc<DataPlaneState>,
    forward_to: String,
}

// ── public entry point ────────────────────────────────────────────────────────

// TODO: return error instead of panicking?
pub async fn run(state: Arc<DataPlaneState>) {
    let http_addr: SocketAddr = state
        .config
        .acme_http01_listen_addr 
        .parse()
        .unwrap_or_else(|e| {
            panic!(
                "ingress: invalid ACME HTTP-01 listen address '{}': {e}",
                state.config.acme_http01_listen_addr
            )
        });
        let https_addr: SocketAddr = state
    .config
    .ingress_listen_addr
    .parse()
    .unwrap_or_else(|e| {
        panic!(
            "ingress: invalid TLS listen address '{}': {e}",
            state.config.ingress_listen_addr
        )
    });
    let forward_to = format!("127.0.0.1:{}", state.config.nitrum.service.port);

    let tls_state = TlsConfig::new(state.clone()).state();
    let acceptor = state.acceptor(state.default_rustls_config());
    let acme_challenge_handler = tls_state.challenge_handler();


    let http_router = Router::new()
        .route("/.well-known/acme-challenge/{token}", get(acme_challenge_handler));

    let https_router = Router::new()
        .route("/.well-known/enclave/status", get(ingress_status))
        .route("/.well-known/enclave/attestation", get(ingress_attestation))
        .fallback(ingress_proxy)
        .with_state(AppState {
            state: state.clone(),
            forward_to: forward_to.clone(),
        });

        // kick off the tls state machine
    tokio::spawn(async move {
        loop {
            match tls_state.next().await.unwrap() {
                Ok(ok) => log::info!("event: {ok:?}"),
                Err(err) => log::error!("error: {err:?}"),
            }
        }
    });

    info!(http = %http_addr, https = %https_addr, app = %forward_to, "ingress listening");
    let http = bind(http_addr).serve(http_router.into_make_service());
    let https = bind_rustls(https_addr, tls_config).serve(https_router.into_make_service());

    tokio::select! {
        r = http => panic!("HTTP server stopped: {:?}", r),
        r = https => panic!("HTTPS server stopped: {:?}", r),
    }
}

// ── handlers ─────────────────────────────────────────────────────────────────

async fn ingress_status() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        r#"{"status":"ok"}"#,
    )
}

async fn ingress_attestation() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "application/json")],
        r#"{"status":"ok"}"#,
    )
}

async fn ingress_proxy(State(app): State<AppState>, req: Request<Body>) -> impl IntoResponse {
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or("/");
    let url = format!("http://{}{}", app.forward_to, path_and_query);
    info!(url = %url, "ingress: proxying to app");

    let client = match reqwest::Client::builder().build() {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "ingress: failed to create reqwest client");
            return (StatusCode::BAD_GATEWAY, "proxy client error").into_response();
        }
    };

    let (parts, body) = req.into_parts();
    let body_bytes = match to_bytes(body, 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            warn!(error = %e, "ingress: failed to read request body");
            return (StatusCode::BAD_REQUEST, "body read error").into_response();
        }
    };

    let mut backend_req = client
        .request(parts.method.clone(), &url)
        .body(body_bytes.to_vec());
    for (name, value) in parts.headers.iter() {
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
    for (name, value) in headers.iter() {
        resp.headers_mut().insert(name.clone(), value.clone());
    }

    resp
}

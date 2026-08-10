//! Ingress HTTP/HTTPS server: TLS termination, well-known endpoints, and app proxying.

use super::health;
use super::state::IngressState;
use super::tls::{TlsState, challenge_handler};
use crate::DataPlaneConfig;
use crate::crypto::{CryptoClient, get_attestation_doc};
use crate::storage::StorageClient;
use anyhow::Context;
use axum::{
    Router,
    body::Body,
    extract::{Query, State},
    http::{HeaderMap, HeaderName, Request, Response, StatusCode, uri::PathAndQuery},
    response::IntoResponse,
    routing::get,
};
use axum_server::bind;
use axum_server::tls_rustls::bind_rustls;
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use serde::Deserialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{info, warn};

/// Spawn the ingress server in the background.
pub fn init(config: &DataPlaneConfig, storage: &Arc<StorageClient>, crypto: &Arc<CryptoClient>) {
    let has_user_process = !config.project.start_command.is_empty();
    let app_ready = Arc::new(AtomicBool::new(!has_user_process));
    health::spawn(config, app_ready.clone());

    let state = Arc::new(IngressState::new(
        config.clone(),
        storage.clone(),
        app_ready,
    ));
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
    Router::new()
        .route("/.well-known/enclave/status", get(ingress_status))
        .route("/.well-known/enclave/attestation", get(ingress_attestation))
        .fallback(ingress_proxy)
        .with_state(state)
}

async fn ingress_status(State(state): State<Arc<IngressState>>) -> impl IntoResponse {
    if state.app_ready.load(Ordering::Relaxed) {
        (
            StatusCode::OK,
            [("content-type", "application/json")],
            r#"{"status":"ok"}"#,
        )
            .into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            [("content-type", "application/json")],
            r#"{"status":"unhealthy"}"#,
        )
            .into_response()
    }
}

#[cfg(test)]
mod status_tests {
    use super::*;
    use crate::{DataPlaneConfig, ListenAddrs};
    use aws_config::{BehaviorVersion, Region};
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use config::{NitrumConfig, PlatformLayout};
    use std::collections::HashMap;
    use tower::ServiceExt;

    fn inert_config() -> DataPlaneConfig {
        let nitrum: NitrumConfig =
            toml::from_str(include_str!("../../../../examples/hello/nitrum.toml"))
                .expect("parse sample nitrum.toml");
        let layout = PlatformLayout::from_project(&nitrum.project);
        let aws = Arc::new(
            aws_config::SdkConfig::builder()
                .behavior_version(BehaviorVersion::latest())
                .region(Region::new("us-east-1"))
                .build(),
        );
        DataPlaneConfig {
            nitrum,
            layout,
            aws,
            imds_base_url: "http://127.0.0.1/latest".to_string(),
            instance_id: "i-test".to_string(),
            kms_key_id: "test-key".to_string(),
            dynamodb_table: "test-table".to_string(),
            otlp_endpoint: None,
            listen_addrs: ListenAddrs {
                ingress_listen_addr: "127.0.0.1:443".parse().unwrap(),
                acme_http01_listen_addr: "127.0.0.1:80".parse().unwrap(),
                crypto_api_listen_addr: "127.0.0.1:3000".parse().unwrap(),
            },
            user_env: HashMap::new(),
        }
    }

    fn router_with_ready(ready: bool) -> Router {
        let config = inert_config();
        let storage = Arc::new(StorageClient::from_config(&config));
        let state = Arc::new(IngressState::new(
            config,
            storage,
            Arc::new(AtomicBool::new(ready)),
        ));
        build_https_router(state)
    }

    #[tokio::test]
    async fn status_ok_when_app_ready() {
        let app = router_with_ready(true);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/.well-known/enclave/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], br#"{"status":"ok"}"#);
    }

    #[tokio::test]
    async fn status_unhealthy_when_app_not_ready() {
        let app = router_with_ready(false);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/.well-known/enclave/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], br#"{"status":"unhealthy"}"#);
    }
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

/// Hop-by-hop headers must not be forwarded by a reverse proxy (RFC 7230 §6.1).
fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailers"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn copy_proxied_headers(src: &HeaderMap, dst: &mut HeaderMap) {
    for (name, value) in src {
        if is_hop_by_hop(name) {
            continue;
        }
        dst.append(name.clone(), value.clone());
    }
}

async fn ingress_proxy(
    State(state): State<Arc<IngressState>>,
    req: Request<Body>,
) -> Response<Body> {
    let (parts, body) = req.into_parts();
    let path_and_query = parts
        .uri
        .path_and_query()
        .cloned()
        .unwrap_or_else(|| PathAndQuery::from_static("/"));
    let uri = state.proxy_uri(path_and_query);

    let mut fwd_headers = HeaderMap::new();
    copy_proxied_headers(&parts.headers, &mut fwd_headers);

    // Forward the client body as-is — `axum::body::Body` is already an
    // `http_body::Body`, so no stream adapter / buffering is needed.
    let mut backend_req = match Request::builder()
        .method(parts.method)
        .uri(uri.clone())
        .body(body)
    {
        Ok(r) => r,
        Err(e) => {
            warn!(uri = %uri, error = %e, "ingress: failed to build backend request");
            return (StatusCode::BAD_REQUEST, "invalid request").into_response();
        }
    };
    *backend_req.headers_mut() = fwd_headers;

    let backend_resp = match state.proxy_client.request(backend_req).await {
        Ok(r) => r,
        Err(e) => {
            warn!(uri = %uri, error = %e, "ingress: proxy request failed");
            return (StatusCode::BAD_GATEWAY, "backend unreachable").into_response();
        }
    };

    let status = backend_resp.status();
    let headers = backend_resp.headers().clone();
    // Stream the backend body to the client without an intermediate buffer.
    let mut resp = Response::new(Body::new(backend_resp.into_body()));
    *resp.status_mut() = status;
    copy_proxied_headers(&headers, resp.headers_mut());
    resp
}

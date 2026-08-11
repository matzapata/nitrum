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
    http::{HeaderMap, Request, Response, StatusCode, uri::PathAndQuery},
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
/// Removes all values for the static hop-by-hop name set in place.
fn strip_hop_by_hop(headers: &mut HeaderMap) {
    for name in [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailers",
        "transfer-encoding",
        "upgrade",
    ] {
        headers.remove(name);
    }
}

/// Reverse-proxy catch-all: forward the client request to the configured
/// backend, streaming both request and response bodies end-to-end.
async fn ingress_proxy(
    State(state): State<Arc<IngressState>>,
    req: Request<Body>,
) -> Response<Body> {
    let (parts, body) = req.into_parts();

    // Preserve path + query; rebuild against the backend base URI.
    let path_and_query = parts
        .uri
        .path_and_query()
        .cloned()
        .unwrap_or_else(|| PathAndQuery::from_static("/"));
    let uri = state.proxy_uri(path_and_query);

    // Drop hop-by-hop headers before forwarding (RFC 7230 §6.1).
    let mut fwd_headers = parts.headers;
    strip_hop_by_hop(&mut fwd_headers);

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

    // Transport / connect failures become 502; the backend's own status is passed through.
    let backend_resp = match state.proxy_client.request(backend_req).await {
        Ok(r) => r,
        Err(e) => {
            warn!(uri = %uri, error = %e, "ingress: proxy request failed");
            return (StatusCode::BAD_GATEWAY, "backend unreachable").into_response();
        }
    };

    let (parts, body) = backend_resp.into_parts();

    // Stream the backend body to the client without an intermediate buffer.
    let mut resp = Response::from_parts(parts, Body::new(body));
    strip_hop_by_hop(resp.headers_mut());

    resp
}

#[cfg(test)]
mod proxy_header_tests {
    use super::*;
    use crate::{DataPlaneConfig, ListenAddrs};
    use aws_config::{BehaviorVersion, Region};
    use axum::body::{Body, to_bytes};
    use axum::http::{HeaderName, HeaderValue, Request};
    use config::{NitrumConfig, PlatformLayout};
    use std::collections::HashMap;
    use std::num::NonZeroU16;
    use std::sync::Mutex;
    use tokio::net::TcpListener;
    use tower::ServiceExt;

    const HOP_BY_HOP: &[&str] = &[
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailers",
        "transfer-encoding",
        "upgrade",
    ];

    fn inert_config(backend_port: u16) -> DataPlaneConfig {
        let nitrum = NitrumConfig {
            project: config::Project {
                name: "nitrum-test".parse().expect("valid test project name"),
                port: NonZeroU16::new(backend_port).expect("non-zero port"),
                start_command: vec![],
            },
            runtime: config::Runtime::default(),
            health_check: config::HealthCheck::default(),
            scaling: config::Scaling::default(),
            tls_termination: config::TlsTermination::default(),
            egress: config::Egress::default(),
            cloud: config::Cloud::default(),
        };
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

    fn ingress_router(backend_port: u16) -> Router {
        let config = inert_config(backend_port);
        let storage = Arc::new(StorageClient::from_config(&config));
        let state = Arc::new(IngressState::new(
            config,
            storage,
            Arc::new(AtomicBool::new(true)),
        ));
        build_https_router(state)
    }

    /// Mock backend that records inbound headers and returns a fixed header set.
    async fn spawn_backend(response_headers: HeaderMap) -> (u16, Arc<Mutex<Option<HeaderMap>>>) {
        let captured = Arc::new(Mutex::new(None));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("local_addr").port();
        let captured_for_task = captured.clone();
        tokio::spawn(async move {
            let app = Router::new().fallback(move |req: Request<Body>| {
                let captured = captured_for_task.clone();
                let response_headers = response_headers.clone();
                async move {
                    *captured.lock().expect("lock") = Some(req.headers().clone());
                    let mut resp = Response::new(Body::from("ok"));
                    *resp.headers_mut() = response_headers;
                    resp
                }
            });
            axum::serve(listener, app).await.expect("serve");
        });
        tokio::task::yield_now().await;
        (port, captured)
    }

    #[test]
    fn strip_hop_by_hop_removes_static_set_and_keeps_end_to_end() {
        let mut headers = HeaderMap::new();
        for name in HOP_BY_HOP {
            headers.insert(
                HeaderName::from_static(name),
                HeaderValue::from_static("drop-me"),
            );
        }
        headers.insert("x-request-id", HeaderValue::from_static("abc"));
        headers.insert("content-type", HeaderValue::from_static("application/json"));
        // Case-insensitive remove (RFC 7230 header names).
        headers.insert("Connection", HeaderValue::from_static("Upgrade"));
        headers.append("connection", HeaderValue::from_static("close"));

        strip_hop_by_hop(&mut headers);

        for name in HOP_BY_HOP {
            assert!(
                !headers.contains_key(*name),
                "expected hop-by-hop `{name}` to be stripped"
            );
        }
        assert_eq!(
            headers.get("x-request-id").map(HeaderValue::as_bytes),
            Some(b"abc".as_slice())
        );
        assert_eq!(
            headers.get("content-type").map(HeaderValue::as_bytes),
            Some(b"application/json".as_slice())
        );
    }

    #[tokio::test]
    async fn proxy_strips_hop_by_hop_request_headers() {
        let mut backend_resp_headers = HeaderMap::new();
        backend_resp_headers.insert("x-backend", HeaderValue::from_static("1"));

        let (port, captured) = spawn_backend(backend_resp_headers).await;
        let app = ingress_router(port);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/echo")
                    .header("connection", "keep-alive, upgrade")
                    .header("keep-alive", "timeout=5")
                    .header("proxy-authorization", "Basic dXNlcjpwYXNz")
                    .header("te", "trailers")
                    .header("trailers", "X-Checksum")
                    .header("transfer-encoding", "chunked")
                    .header("upgrade", "websocket")
                    .header("x-request-id", "req-42")
                    .header("content-type", "text/plain")
                    .body(Body::from("hi"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let seen = captured
            .lock()
            .expect("lock")
            .clone()
            .expect("backend should have received a request");
        for name in HOP_BY_HOP {
            assert!(
                !seen.contains_key(*name),
                "backend must not see hop-by-hop `{name}`"
            );
        }
        assert_eq!(
            seen.get("x-request-id").map(HeaderValue::as_bytes),
            Some(b"req-42".as_slice())
        );
        assert_eq!(
            seen.get("content-type").map(HeaderValue::as_bytes),
            Some(b"text/plain".as_slice())
        );
    }

    #[tokio::test]
    async fn proxy_strips_hop_by_hop_response_headers() {
        let mut backend_resp_headers = HeaderMap::new();
        backend_resp_headers.insert("connection", HeaderValue::from_static("close"));
        backend_resp_headers.insert("keep-alive", HeaderValue::from_static("timeout=5"));
        backend_resp_headers.insert(
            "proxy-authenticate",
            HeaderValue::from_static("Basic realm=\"api\""),
        );
        backend_resp_headers.insert("upgrade", HeaderValue::from_static("websocket"));
        backend_resp_headers.insert("te", HeaderValue::from_static("trailers"));
        backend_resp_headers.insert("trailers", HeaderValue::from_static("X-Checksum"));
        backend_resp_headers.insert("x-backend", HeaderValue::from_static("from-app"));
        backend_resp_headers.insert("content-type", HeaderValue::from_static("text/plain"));

        let (port, _) = spawn_backend(backend_resp_headers).await;
        let app = ingress_router(port);

        let response = app
            .oneshot(Request::builder().uri("/echo").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let headers = response.headers();
        for name in [
            "connection",
            "keep-alive",
            "proxy-authenticate",
            "te",
            "trailers",
            "upgrade",
            "transfer-encoding",
        ] {
            assert!(
                !headers.contains_key(name),
                "client must not see hop-by-hop `{name}`"
            );
        }
        assert_eq!(
            headers.get("x-backend").map(HeaderValue::as_bytes),
            Some(b"from-app".as_slice())
        );
        assert_eq!(
            headers.get("content-type").map(HeaderValue::as_bytes),
            Some(b"text/plain".as_slice())
        );

        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], b"ok");
    }
}

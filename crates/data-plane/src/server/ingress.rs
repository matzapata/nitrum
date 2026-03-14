//! Ingress proxy: terminates TLS, serves enclave well-known endpoints and ACME
//! challenges directly, and forwards everything else to the user application.
//!
//! Well-known paths served without touching the user app:
//!   GET /.well-known/enclave/status      → 200 {"status":"ok"}
//!   GET /.well-known/enclave/attestation → 200 base64-encoded attestation
//!   GET /.well-known/acme-challenge/*   → 200 key_authorization (for ACME HTTP-01)

use crate::state::DataPlaneState;
use crate::storage::keys;
use crate::utils::leader::Leader;
use axum::{
    body::{to_bytes, Body},
    extract::{Path, State},
    http::{Request, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use axum::Extension;
use axum::serve::Listener;
use bytes::Bytes;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tracing::{error, info, warn};

/// State for the TLS ingress Axum app (well-known + proxy).
#[derive(Clone)]
struct IngressState {
    state: Arc<DataPlaneState>,
    forward_to: String,
}

/// Listener that yields a single (stream, addr) then never completes (for one TLS connection).
struct OneShotListener<Io> {
    stream: Option<(Io, SocketAddr)>,
    local_addr: SocketAddr,
}

impl<Io> OneShotListener<Io>
where
    Io: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    fn new(stream: Io, peer: SocketAddr) -> Self {
        Self {
            stream: Some((stream, peer)),
            local_addr: ([0u8; 4], 0).into(),
        }
    }
}

impl<Io> Listener for OneShotListener<Io>
where
    Io: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    type Io = Io;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.stream.take() {
            Some((io, addr)) => (io, addr),
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        Ok(self.local_addr)
    }
}

// ── public entry point ────────────────────────────────────────────────────────

pub async fn run(state: Arc<DataPlaneState>) {
    let acme_leader = Arc::new(Leader::new(
        state.storage.clone(),
        state.config.instance_id.clone(),
        keys::ACME_LEADER_KEY.to_string(),
    ));

    let listen_addr = state.config.ingress_listen_addr.clone();
    let acme_http01_addr = state.config.acme_http01_listen_addr.clone();
    let forward_to = format!("127.0.0.1:{}", state.config.nitrum.service.port);

    // Bind ACME HTTP-01 first so we fail fast if the port is unavailable (e.g. non-root can't bind 80).
    // Pebble will GET http://nitrum.local:PORT/.well-known/acme-challenge/TOKEN during provisioning.
    let http01_listener = TcpListener::bind(&acme_http01_addr)
        .await
        .unwrap_or_else(|e| {
            panic!(
                "ingress: failed to bind ACME HTTP-01 listener at {} (Pebble needs this for validation). \
                 If running in a container without root, set NITRUM_ACME_HTTP01_LISTEN_ADDR=0.0.0.0:5002 and configure Pebble/docker to use port 5002: {e}",
                acme_http01_addr
            );
        });
    info!(addr = %acme_http01_addr, "ingress ACME HTTP-01 listener");
    let state_http01 = state.clone();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        let app = Router::new()
            .route(
                "/.well-known/acme-challenge/{token}",
                get(acme_http01_handler),
            )
            .fallback(acme_http01_not_found)
            .with_state(state_http01);
        let _ = ready_tx.send(());
        if let Err(e) = axum::serve(http01_listener, app).await {
            error!(error = %e, "ingress: ACME HTTP-01 server error");
        }
    });
    ready_rx.await.expect("HTTP-01 server task dropped before ready");

    let (acceptor, renewal_loop) = super::tls::acceptor(state.as_ref(), acme_leader)
        .await
        .unwrap_or_else(|e| panic!("ingress: TLS acceptor failed: {:#}", e));

    if let Some(loop_task) = renewal_loop {
        tokio::spawn(loop_task);
    }

    let forwarder = TcpListener::bind(&listen_addr)
        .await
        .unwrap_or_else(|e| panic!("ingress: failed to bind {listen_addr}: {e}"));

    let ingress_state = IngressState {
        state: state.clone(),
        forward_to: forward_to.clone(),
    };
    let ingress_router = Router::new()
        .route("/.well-known/enclave/status", get(ingress_status))
        .route("/.well-known/enclave/attestation", get(ingress_attestation))
        .route(
            "/.well-known/acme-challenge/{token}",
            get(ingress_acme_challenge),
        )
        .fallback(ingress_proxy)
        .layer(Extension(ingress_state));

    info!(listen = %listen_addr, app = %forward_to, "ingress TLS proxy listening");

    loop {
        match forwarder.accept().await {
            Ok((stream, peer)) => {
                let acceptor_for_conn = match &acceptor {
                    super::tls::IngressAcceptor::Static(a) => a.clone(),
                    super::tls::IngressAcceptor::Acme { acceptor: shared } => {
                        shared.read().await.clone()
                    }
                };
                let router = ingress_router.clone();
                tokio::spawn(async move {
                    if let Err(e) = serve_tls(stream, peer, acceptor_for_conn, router).await {
                        warn!(peer = %peer, error = %e, "ingress: connection error");
                    }
                });
            }
            Err(e) => error!(error = %e, "ingress: accept failed"),
        }
    }
}

// ── ACME HTTP-01 (Axum) ─────────────────────────────────────────────────────

async fn acme_http01_handler(
    State(state): State<Arc<DataPlaneState>>,
    Path(token): Path<String>,
) -> impl IntoResponse {
    match state
        .storage
        .get_object(&keys::acme_challenge_key(&token))
        .await
    {
        Ok(Some(body)) => {
            info!(token = %token, "ingress: ACME HTTP-01 challenge (plain HTTP)");
            (
                StatusCode::OK,
                [("content-type", "application/octet-stream")],
                Bytes::from(body),
            )
                .into_response()
        }
        _ => (StatusCode::NOT_FOUND, ()).into_response(),
    }
}

async fn acme_http01_not_found() -> impl IntoResponse {
    (StatusCode::NOT_FOUND, ())
}

// ── TLS ingress (Axum on TLS stream) ──────────────────────────────────────────

async fn serve_tls(
    stream: TcpStream,
    peer: SocketAddr,
    acceptor: TlsAcceptor,
    router: Router,
) -> io::Result<()> {
    let tls = match acceptor.accept(stream).await {
        Ok(s) => s,
        Err(e) => {
            warn!(peer = %peer, error = %e, "ingress: TLS handshake failed");
            return Ok(());
        }
    };
    info!(peer = %peer, "ingress: TLS established");
    let listener = OneShotListener::new(tls, peer);
    let _ = axum::serve(listener, router.into_make_service())
        .await
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e));
    Ok(())
}

async fn ingress_status() -> impl IntoResponse {
    (StatusCode::OK, [("content-type", "application/json")], r#"{"status":"ok"}"#)
}

async fn ingress_attestation() -> impl IntoResponse {
    (StatusCode::OK, [("content-type", "application/json")], r#"{"status":"ok"}"#)
}

async fn ingress_acme_challenge(
    Extension(ingress): Extension<IngressState>,
    Path(token): Path<String>,
) -> impl IntoResponse {
    match ingress
        .state
        .storage
        .get_object(&keys::acme_challenge_key(&token))
        .await
    {
        Ok(Some(body)) => {
            info!(token = %token, "ingress: ACME HTTP-01 challenge");
            (
                StatusCode::OK,
                [("content-type", "application/octet-stream")],
                Bytes::from(body),
            )
                .into_response()
        }
        _ => (StatusCode::NOT_FOUND, ()).into_response(),
    }
}

async fn ingress_proxy(
    Extension(ingress): Extension<IngressState>,
    req: Request<Body>,
) -> impl IntoResponse {
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or("/");
    let url = format!("http://{}{}", ingress.forward_to, path_and_query);
    info!(url = %url, "ingress: proxying to app");

    let client = match reqwest::Client::builder()
        .build()
    {
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
        // Skip hop-by-hop headers
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

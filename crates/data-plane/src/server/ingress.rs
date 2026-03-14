//! Ingress proxy: terminates TLS, serves enclave well-known endpoints and ACME
//! challenges directly, and forwards everything else to the user application.
//!
//! Well-known paths served without touching the user app:
//!   GET /.well-known/enclave/status      → 200 {"status":"ok"}
//!   GET /.well-known/enclave/attestation → 200 {"status":"ok"}
//!   GET /.well-known/acme-challenge/*   → 200 key_authorization (for ACME HTTP-01)

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tracing::{error, info, warn};

use crate::state::DataPlaneState;
use crate::storage::keys;
use crate::utils::leader::Leader;

const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HEADER_BYTES: usize = 16 * 1024;

const PATH_STATUS: &str = "/.well-known/enclave/status";
const PATH_ATTESTATION: &str = "/.well-known/enclave/attestation";
const PATH_ACME_CHALLENGE_PREFIX: &str = "/.well-known/acme-challenge/";

// TODO: these are hardcoded, they should not.
const RESP_STATUS: &[u8] = b"\
HTTP/1.1 200 OK\r\n\
Content-Type: application/json\r\n\
Content-Length: 15\r\n\
Connection: close\r\n\
\r\n\
{\"status\":\"ok\"}";

const RESP_ATTESTATION: &[u8] = b"\
HTTP/1.1 200 OK\r\n\
Content-Type: application/json\r\n\
Content-Length: 15\r\n\
Connection: close\r\n\
\r\n\
{\"status\":\"ok\"}";

const RESP_NOT_FOUND: &[u8] = b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n";

// ── public entry point ────────────────────────────────────────────────────────

pub async fn run(state: Arc<DataPlaneState>) {
    let acme_leader = Arc::new(Leader::new(
        state.storage.clone(),
        state.config.instance_id.clone(),
        keys::ACME_LEADER_KEY.to_string(),
    ));

    let listen_addr = crate::constants::INGRESS_LISTEN_ADDR;
    let acme_http01_addr = crate::constants::INGRESS_ACME_HTTP01_LISTEN_ADDR;
    let app_port = state.config.nitrum.service.port;
    let app_addr = format!("127.0.0.1:{app_port}");

    // Start ACME HTTP-01 listener first so Pebble can validate during cert provisioning.
    // Pebble hits http://nitrum.local:5002/.well-known/acme-challenge/... ; must be up before acceptor().
    let state_http01 = state.clone();
    tokio::spawn(async move {
        let listener = match TcpListener::bind(acme_http01_addr).await {
            Ok(l) => l,
            Err(e) => {
                error!(addr = %acme_http01_addr, error = %e, "ingress: failed to bind ACME HTTP-01 listener");
                return;
            }
        };
        info!(addr = %acme_http01_addr, "ingress ACME HTTP-01 listener");
        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    let state = state_http01.clone();
                    tokio::spawn(async move {
                        if let Err(e) = serve_acme_http01_only(stream, peer, state).await {
                            warn!(peer = %peer, error = %e, "ingress: ACME HTTP-01 connection error");
                        }
                    });
                }
                Err(e) => error!(error = %e, "ingress: ACME HTTP-01 accept failed"),
            }
        }
    });

    // Give the HTTP-01 listener a moment to bind so Pebble can reach it during provisioning
    tokio::time::sleep(Duration::from_millis(100)).await;

    let (acceptor, renewal_loop) = super::tls::acceptor(state.as_ref(), acme_leader)
        .await
        .unwrap_or_else(|e| panic!("ingress: TLS acceptor failed: {:#}", e));

    if let Some(loop_task) = renewal_loop {
        tokio::spawn(loop_task);
    }

    let listener = TcpListener::bind(listen_addr)
        .await
        .unwrap_or_else(|e| panic!("ingress: failed to bind {listen_addr}: {e}"));

    info!(listen = %listen_addr, app = %app_addr, "ingress TLS proxy listening");

    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                let acceptor_for_conn = match &acceptor {
                    super::tls::IngressAcceptor::Static(a) => a.clone(),
                    super::tls::IngressAcceptor::Acme { acceptor: shared } => shared.read().await.clone(),
                };
                let app = app_addr.clone();
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(e) = serve(stream, peer, &app, acceptor_for_conn, state).await {
                        warn!(peer = %peer, error = %e, "ingress: connection error");
                    }
                });
            }
            Err(e) => error!(error = %e, "ingress: accept failed"),
        }
    }
}

// ── ACME HTTP-01 only (plain HTTP on 5002 for Pebble) ─────────────────────────────

async fn serve_acme_http01_only(
    mut stream: TcpStream,
    peer: SocketAddr,
    state: Arc<DataPlaneState>,
) -> io::Result<()> {
    let mut header_buf = Vec::with_capacity(2048);
    let headers_complete = tokio::time::timeout(
        HEADER_READ_TIMEOUT,
        read_headers(&mut stream, &mut header_buf),
    )
    .await
    .unwrap_or(Ok(false))
    .unwrap_or(false);

    if headers_complete {
        if let Some(path) = request_path(&header_buf) {
            if path.starts_with(PATH_ACME_CHALLENGE_PREFIX) {
                let token = path.trim_start_matches(PATH_ACME_CHALLENGE_PREFIX);
                if let Ok(Some(body)) = state
                    .storage
                    .get_object(&keys::acme_challenge_key(token))
                    .await
                {
                    info!(peer = %peer, token = %token, "ingress: ACME HTTP-01 challenge (plain HTTP)");
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    stream.write_all(resp.as_bytes()).await?;
                    stream.write_all(&body).await?;
                    stream.flush().await?;
                    return Ok(());
                }
            }
        }
    }
    stream.write_all(RESP_NOT_FOUND).await?;
    stream.flush().await?;
    Ok(())
}

// ── per-connection handler ────────────────────────────────────────────────────

async fn serve(
    stream: TcpStream,
    peer: SocketAddr,
    app_addr: &str,
    acceptor: TlsAcceptor,
    state: Arc<DataPlaneState>,
) -> io::Result<()> {
    let tls = match acceptor.accept(stream).await {
        Ok(s) => s,
        Err(e) => {
            warn!(peer = %peer, error = %e, "ingress: TLS handshake failed");
            return Ok(());
        }
    };
    info!(peer = %peer, "ingress: TLS established");
    handle(tls, peer, app_addr, state).await
}

async fn handle<S>(
    mut inbound: S,
    peer: SocketAddr,
    app_addr: &str,
    state: Arc<DataPlaneState>,
) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // ── peek at HTTP request line + headers ───────────────────────────────────
    let mut header_buf = Vec::with_capacity(2048);
    let headers_complete = tokio::time::timeout(
        HEADER_READ_TIMEOUT,
        read_headers(&mut inbound, &mut header_buf),
    )
    .await
    .unwrap_or(Ok(false))
    .unwrap_or(false);

    // ── route well-known enclave paths and ACME challenge ──────────────────────
    if headers_complete {
        if let Some(path) = request_path(&header_buf) {
            match path {
                PATH_STATUS => {
                    info!(peer = %peer, "ingress: GET /.well-known/enclave/status");
                    inbound.write_all(RESP_STATUS).await?;
                    inbound.flush().await?;
                    return Ok(());
                }
                PATH_ATTESTATION => {
                    info!(peer = %peer, "ingress: GET /.well-known/enclave/attestation");
                    inbound.write_all(RESP_ATTESTATION).await?;
                    inbound.flush().await?;
                    return Ok(());
                }
                _ => {
                    if path.starts_with(PATH_ACME_CHALLENGE_PREFIX) {
                        let token = path.trim_start_matches(PATH_ACME_CHALLENGE_PREFIX);
                        if let Ok(Some(body)) = state
                            .storage
                            .get_object(&keys::acme_challenge_key(token))
                            .await
                        {
                            info!(peer = %peer, token = %token, "ingress: ACME HTTP-01 challenge");
                            let resp = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                body.len()
                            );
                            inbound.write_all(resp.as_bytes()).await?;
                            inbound.write_all(&body).await?;
                            inbound.flush().await?;
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    // ── proxy to user app ─────────────────────────────────────────────────────
    info!(peer = %peer, app = %app_addr, "ingress: proxying to app");
    let mut outbound = TcpStream::connect(app_addr).await.map_err(|e| {
        warn!(app = %app_addr, error = %e, "ingress: could not connect to app");
        e
    })?;

    // Replay the already-consumed header bytes before bidirectional copy.
    outbound.write_all(&header_buf).await?;
    let (from_client, from_app) = io::copy_bidirectional(&mut inbound, &mut outbound).await?;

    info!(
        peer = %peer,
        bytes_in = from_client,
        bytes_out = from_app,
        "ingress: connection closed"
    );
    Ok(())
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Read one byte at a time into `buf` until `\r\n\r\n` is seen.
/// Returns `true` if the end-of-headers marker was found, `false` if the size
/// limit was hit first (buf still contains everything consumed so far).
async fn read_headers<S: AsyncRead + Unpin>(stream: &mut S, buf: &mut Vec<u8>) -> io::Result<bool> {
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).await?;
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            return Ok(true);
        }
        if buf.len() >= MAX_HEADER_BYTES {
            return Ok(false);
        }
    }
}

/// Parse the request path from a raw HTTP header buffer.
///
/// The HTTP request line has the form `METHOD SP path SP HTTP/version\r\n`.
/// Returns the path without any query-string component.
fn request_path(buf: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(buf).ok()?;
    let first_line = text.split("\r\n").next()?;
    let path_with_query = first_line.split_ascii_whitespace().nth(1)?;
    Some(path_with_query.split('?').next().unwrap_or(path_with_query))
}

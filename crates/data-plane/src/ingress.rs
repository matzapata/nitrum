//! Ingress proxy: terminates TLS, serves enclave well-known endpoints directly,
//! and forwards everything else to the user application.
//!
//! Well-known paths served without touching the user app:
//!   GET /.well-known/enclave/status      → 200 {"status":"ok"}
//!   GET /.well-known/enclave/attestation → 200 {"status":"ok"}

use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tracing::{error, info, warn};

const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HEADER_BYTES: usize = 16 * 1024;

const PATH_STATUS: &str = "/.well-known/enclave/status";
const PATH_ATTESTATION: &str = "/.well-known/enclave/attestation";

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

// ── public entry point ────────────────────────────────────────────────────────

pub async fn run(app_port: u16, acceptor: TlsAcceptor) {
    let listen_addr = crate::constants::INGRESS_LISTEN_ADDR;
    let app_addr = format!("127.0.0.1:{app_port}");
    let listener = TcpListener::bind(listen_addr)
        .await
        .unwrap_or_else(|e| panic!("ingress: failed to bind {listen_addr}: {e}"));

    info!(listen = %listen_addr, app = %app_addr, "ingress TLS proxy listening");

    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                let acceptor = acceptor.clone();
                let app = app_addr.clone();
                tokio::spawn(async move {
                    if let Err(e) = serve(stream, peer, &app, acceptor).await {
                        warn!(peer = %peer, error = %e, "ingress: connection error");
                    }
                });
            }
            Err(e) => error!(error = %e, "ingress: accept failed"),
        }
    }
}

// ── per-connection handler ────────────────────────────────────────────────────

async fn serve(
    stream: TcpStream,
    peer: SocketAddr,
    app_addr: &str,
    acceptor: TlsAcceptor,
) -> io::Result<()> {
    let tls = match acceptor.accept(stream).await {
        Ok(s) => s,
        Err(e) => {
            warn!(peer = %peer, error = %e, "ingress: TLS handshake failed");
            return Ok(());
        }
    };
    info!(peer = %peer, "ingress: TLS established");
    handle(tls, peer, app_addr).await
}

async fn handle<S>(mut inbound: S, peer: SocketAddr, app_addr: &str) -> io::Result<()>
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

    // ── route well-known enclave paths ────────────────────────────────────────
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
                _ => {}
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

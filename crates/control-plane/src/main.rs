use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tracing::{error, info, warn};

use shared::bridge::{Bridge, BridgeInterface, Direction};
use shared::server::Listener;

/// Port the control-plane's TCP proxy listens on (matches data-plane's CONTROL_PLANE_TCP_PORT).
const TCP_PROXY_PORT: u16 = 8181;
/// Port the control-plane's DNS proxy listens on (matches data-plane's CONTROL_PLANE_DNS_PORT).
const DNS_PROXY_PORT: u16 = 5354;
/// Port the control-plane listens on for external ingress traffic.
const INGRESS_PROXY_PORT: u16 = 3031;
/// Port the data-plane's ingress listener is bound to (matches data-plane's INGRESS_LISTEN_PORT).
const ENCLAVE_INGRESS_PORT: u16 = 7777;

// ── TCP egress proxy ──────────────────────────────────────────────────────────

async fn handle_tcp_connection<S>(mut client: S)
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    // Read 6-byte header: [4 bytes IPv4 big-endian][2 bytes port big-endian]
    let mut header = [0u8; 6];
    if let Err(e) = client.read_exact(&mut header).await {
        warn!(error = %e, "tcp: failed to read destination header");
        return;
    }

    let ip = Ipv4Addr::new(header[0], header[1], header[2], header[3]);
    let port = u16::from_be_bytes([header[4], header[5]]);
    let target = SocketAddrV4::new(ip, port);

    info!(target = %target, "tcp: received connection, connecting to target");

    let mut upstream = match TcpStream::connect(target).await {
        Ok(s) => s,
        Err(e) => {
            error!(target = %target, error = %e, "tcp: failed to connect to target");
            return;
        }
    };

    match io::copy_bidirectional(&mut client, &mut upstream).await {
        Ok((from_client, from_upstream)) => {
            info!(
                target = %target,
                bytes_from_client = from_client,
                bytes_from_upstream = from_upstream,
                "tcp: connection closed"
            );
        }
        Err(e) => {
            warn!(target = %target, error = %e, "tcp: connection error");
        }
    }
}

async fn run_tcp_proxy() {
    let mut listener = Bridge::get_listener(TCP_PROXY_PORT, Direction::EnclaveToHost)
        .await
        .expect("failed to bind TCP proxy");

    info!(port = TCP_PROXY_PORT, "tcp proxy listening");

    loop {
        match listener.accept().await {
            Ok(stream) => {
                tokio::spawn(async move {
                    handle_tcp_connection(stream).await;
                });
            }
            Err(e) => error!(error = %e, "tcp: accept error"),
        }
    }
}

// ── DNS proxy (TCP tunnel → upstream UDP resolver) ───────────────────────────

async fn handle_dns_connection<S>(mut client: S, upstream_dns: String)
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    // The data-plane shuts down its write side after sending the query, so
    // read_to_end captures exactly one DNS request per connection.
    let mut query = Vec::new();
    if let Err(e) = client.read_to_end(&mut query).await {
        warn!(error = %e, "dns: failed to read query");
        return;
    }

    info!(bytes = query.len(), upstream = %upstream_dns, "dns: received query, forwarding upstream");

    let udp = match UdpSocket::bind("0.0.0.0:0").await {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, "dns: failed to bind UDP socket");
            return;
        }
    };

    if let Err(e) = udp.send_to(&query, &upstream_dns).await {
        error!(upstream = %upstream_dns, error = %e, "dns: failed to send query to upstream");
        return;
    }

    let mut resp_buf = [0u8; 512];
    let resp_len = match udp.recv(&mut resp_buf).await {
        Ok(n) => n,
        Err(e) => {
            error!(error = %e, "dns: failed to receive response from upstream");
            return;
        }
    };

    info!(bytes = resp_len, "dns: received response, sending back");

    if let Err(e) = client.write_all(&resp_buf[..resp_len]).await {
        error!(error = %e, "dns: failed to write response");
    }
}

async fn run_dns_proxy(upstream_dns: String) {
    let mut listener = Bridge::get_listener(DNS_PROXY_PORT, Direction::EnclaveToHost)
        .await
        .expect("failed to bind DNS proxy");

    info!(port = DNS_PROXY_PORT, upstream = %upstream_dns, "dns proxy listening");

    loop {
        match listener.accept().await {
            Ok(stream) => {
                let dns_upstream = upstream_dns.clone();
                tokio::spawn(async move {
                    handle_dns_connection(stream, dns_upstream).await;
                });
            }
            Err(e) => error!(error = %e, "dns: accept error"),
        }
    }
}

// ── TCP ingress proxy ─────────────────────────────────────────────────────────

async fn handle_ingress_connection(mut client: TcpStream, client_addr: SocketAddr) {
    info!(client = %client_addr, "ingress: new connection, forwarding to enclave");

    let mut upstream =
        match Bridge::get_client_connection(ENCLAVE_INGRESS_PORT, Direction::HostToEnclave).await {
            Ok(s) => s,
            Err(e) => {
                error!(error = %e, "ingress: failed to connect to enclave");
                return;
            }
        };

    match io::copy_bidirectional(&mut client, &mut upstream).await {
        Ok((from_client, from_upstream)) => {
            info!(
                client = %client_addr,
                bytes_from_client = from_client,
                bytes_from_upstream = from_upstream,
                "ingress: connection closed"
            );
        }
        Err(e) => {
            warn!(client = %client_addr, error = %e, "ingress: connection error");
        }
    }
}

async fn run_ingress_proxy() {
    let listener = TcpListener::bind(format!("0.0.0.0:{INGRESS_PROXY_PORT}"))
        .await
        .expect("failed to bind ingress proxy");

    info!(port = INGRESS_PROXY_PORT, "ingress proxy listening");

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                tokio::spawn(async move {
                    handle_ingress_connection(stream, addr).await;
                });
            }
            Err(e) => error!(error = %e, "ingress: accept error"),
        }
    }
}

// ── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let upstream_dns = std::env::var("DNS_UPSTREAM").unwrap_or_else(|_| "8.8.8.8:53".to_string());

    info!("control-plane starting");

    tokio::join!(
        run_tcp_proxy(),
        run_dns_proxy(upstream_dns),
        run_ingress_proxy(),
    );
}

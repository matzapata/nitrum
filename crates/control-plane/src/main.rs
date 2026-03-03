use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tracing::{error, info, warn};

const TCP_PROXY_PORT: u16 = 8181;
const DNS_PROXY_PORT: u16 = 5354;

// ── TCP egress proxy ──────────────────────────────────────────────────────────

async fn handle_tcp_connection(mut client: TcpStream, client_addr: SocketAddr) {
    // Read 6-byte header: [4 bytes IPv4 big-endian][2 bytes port big-endian]
    let mut header = [0u8; 6];
    if let Err(e) = client.read_exact(&mut header).await {
        warn!(client = %client_addr, error = %e, "tcp: failed to read destination header");
        return;
    }

    let ip = Ipv4Addr::new(header[0], header[1], header[2], header[3]);
    let port = u16::from_be_bytes([header[4], header[5]]);
    let target = SocketAddrV4::new(ip, port);

    info!(client = %client_addr, target = %target, "tcp: received connection, connecting to target");

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
                client = %client_addr,
                target = %target,
                bytes_from_client = from_client,
                bytes_from_upstream = from_upstream,
                "tcp: connection closed"
            );
        }
        Err(e) => {
            warn!(client = %client_addr, target = %target, error = %e, "tcp: connection error");
        }
    }
}

async fn run_tcp_proxy() {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", TCP_PROXY_PORT))
        .await
        .expect("failed to bind TCP proxy");

    info!(port = TCP_PROXY_PORT, "tcp proxy listening");

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                tokio::spawn(async move {
                    handle_tcp_connection(stream, addr).await;
                });
            }
            Err(e) => error!(error = %e, "tcp: accept error"),
        }
    }
}

// ── DNS proxy (TCP tunnel → upstream UDP resolver) ───────────────────────────

async fn handle_dns_connection(mut client: TcpStream, client_addr: SocketAddr, upstream_dns: String) {
    // Read raw DNS query bytes (one per connection — the enclave shuts down its
    // write side when done, so read_to_end captures exactly the query).
    let mut query = Vec::new();
    if let Err(e) = client.read_to_end(&mut query).await {
        warn!(client = %client_addr, error = %e, "dns: failed to read query");
        return;
    }

    info!(client = %client_addr, bytes = query.len(), upstream = %upstream_dns, "dns: received query, forwarding upstream");

    // Forward raw DNS bytes to upstream resolver via UDP
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

    info!(client = %client_addr, bytes = resp_len, "dns: received response, sending back");

    if let Err(e) = client.write_all(&resp_buf[..resp_len]).await {
        error!(error = %e, "dns: failed to write response");
    }
}

async fn run_dns_proxy(upstream_dns: String) {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", DNS_PROXY_PORT))
        .await
        .expect("failed to bind DNS proxy");

    info!(port = DNS_PROXY_PORT, upstream = %upstream_dns, "dns proxy listening");

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                let dns_upstream = upstream_dns.clone();
                tokio::spawn(async move {
                    handle_dns_connection(stream, addr, dns_upstream).await;
                });
            }
            Err(e) => error!(error = %e, "dns: accept error"),
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
    );
}

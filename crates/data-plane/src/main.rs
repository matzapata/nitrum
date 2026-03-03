use std::mem::size_of;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::os::unix::io::AsRawFd;
use std::sync::Arc;

use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tracing::{error, info, warn};

const TCP_PROXY_PORT: u16 = 8080;
const DNS_LISTEN_ADDR: &str = "127.0.0.1:53";
const SOL_IP: libc::c_int = 0;
const SO_ORIGINAL_DST: libc::c_int = 80;

fn get_original_dst(stream: &TcpStream) -> io::Result<SocketAddrV4> {
    let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut len = size_of::<libc::sockaddr_in>() as libc::socklen_t;

    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            SOL_IP,
            SO_ORIGINAL_DST,
            &mut addr as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };

    if rc != 0 {
        return Err(io::Error::last_os_error());
    }

    let ip = Ipv4Addr::from(u32::from_be(addr.sin_addr.s_addr));
    let port = u16::from_be(addr.sin_port);
    Ok(SocketAddrV4::new(ip, port))
}

// ── TCP egress proxy ──────────────────────────────────────────────────────────

async fn handle_tcp_connection(mut client: TcpStream, client_addr: SocketAddr, control_plane_addr: String) {
    let original_dst = match get_original_dst(&client) {
        Ok(dst) => dst,
        Err(e) => {
            warn!(client = %client_addr, error = %e, "failed to get original destination, dropping connection");
            return;
        }
    };

    info!(
        client = %client_addr,
        destination = %original_dst,
        "tcp: accepted connection, forwarding to control plane"
    );

    let mut upstream = match TcpStream::connect(&control_plane_addr).await {
        Ok(s) => s,
        Err(e) => {
            error!(control_plane = %control_plane_addr, error = %e, "tcp: failed to connect to control plane");
            return;
        }
    };

    // Send 6-byte header: [4 bytes IPv4 big-endian][2 bytes port big-endian]
    let ip_bytes = original_dst.ip().octets();
    let port_bytes = original_dst.port().to_be_bytes();
    let header = [
        ip_bytes[0], ip_bytes[1], ip_bytes[2], ip_bytes[3],
        port_bytes[0], port_bytes[1],
    ];

    if let Err(e) = upstream.write_all(&header).await {
        error!(error = %e, "tcp: failed to send destination header to control plane");
        return;
    }

    match io::copy_bidirectional(&mut client, &mut upstream).await {
        Ok((from_client, from_upstream)) => {
            info!(
                client = %client_addr,
                destination = %original_dst,
                bytes_from_client = from_client,
                bytes_from_upstream = from_upstream,
                "tcp: connection closed"
            );
        }
        Err(e) => {
            warn!(client = %client_addr, destination = %original_dst, error = %e, "tcp: connection error");
        }
    }
}

async fn run_tcp_proxy(control_plane_addr: String) {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", TCP_PROXY_PORT))
        .await
        .expect("failed to bind TCP proxy");

    info!(port = TCP_PROXY_PORT, control_plane = %control_plane_addr, "tcp proxy listening");

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                let cp_addr = control_plane_addr.clone();
                tokio::spawn(async move {
                    handle_tcp_connection(stream, addr, cp_addr).await;
                });
            }
            Err(e) => error!(error = %e, "tcp: accept error"),
        }
    }
}

// ── DNS proxy (UDP → TCP tunnel to control plane) ────────────────────────────

async fn run_dns_proxy(control_plane_dns_addr: String) {
    let listen_addr = std::env::var("DNS_LISTEN_ADDR")
        .unwrap_or_else(|_| DNS_LISTEN_ADDR.to_string());

    let socket = Arc::new(
        UdpSocket::bind(&listen_addr)
            .await
            .expect("failed to bind DNS proxy"),
    );

    info!(addr = %listen_addr, control_plane = %control_plane_dns_addr, "dns proxy listening");

    let mut buf = vec![0u8; 4096];
    loop {
        let (len, client_addr) = match socket.recv_from(&mut buf).await {
            Ok(r) => r,
            Err(e) => {
                error!(error = %e, "dns: recv error");
                continue;
            }
        };

        let query = buf[..len].to_vec();
        let cp_addr = control_plane_dns_addr.clone();
        let sock = Arc::clone(&socket);

        tokio::spawn(async move {
            info!(client = %client_addr, bytes = len, "dns: received query, forwarding to control plane");

            // One TCP connection per DNS query (mirrors the VSock bridge approach).
            // The control-plane reads the raw bytes, resolves, and writes the raw response.
            let mut stream = match TcpStream::connect(&cp_addr).await {
                Ok(s) => s,
                Err(e) => {
                    error!(control_plane = %cp_addr, error = %e, "dns: failed to connect to control plane");
                    return;
                }
            };

            if let Err(e) = stream.write_all(&query).await {
                error!(error = %e, "dns: failed to write query");
                return;
            }
            // Signal end of request so the control-plane's read completes
            if let Err(e) = stream.shutdown().await {
                error!(error = %e, "dns: failed to shutdown write side");
                return;
            }

            let mut resp = [0u8; 512];
            let resp_len = match stream.read(&mut resp).await {
                Ok(n) if n > 0 => n,
                Ok(_) => { error!("dns: empty response from control plane"); return; }
                Err(e) => { error!(error = %e, "dns: failed to read response"); return; }
            };

            info!(client = %client_addr, bytes = resp_len, "dns: received response, forwarding to client");

            if let Err(e) = sock.send_to(&resp[..resp_len], client_addr).await {
                error!(client = %client_addr, error = %e, "dns: failed to send response to client");
            }
        });
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

    let control_plane_host = std::env::var("CONTROL_PLANE_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let tcp_proxy_port = std::env::var("CONTROL_PLANE_PORT").unwrap_or_else(|_| "8181".to_string());
    let dns_proxy_port = std::env::var("CONTROL_PLANE_DNS_PORT").unwrap_or_else(|_| "5354".to_string());

    let control_plane_tcp = format!("{}:{}", control_plane_host, tcp_proxy_port);
    let control_plane_dns = format!("{}:{}", control_plane_host, dns_proxy_port);

    info!("data-plane starting");

    tokio::join!(
        run_tcp_proxy(control_plane_tcp),
        run_dns_proxy(control_plane_dns),
    );
}

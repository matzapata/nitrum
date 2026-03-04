use std::mem::size_of;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::os::unix::io::AsRawFd;
use std::sync::Arc;

use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tracing::{error, info, warn};

const TLS_CERT_PATH: &str = "/app/certs/cert.pem";
const TLS_KEY_PATH: &str = "/app/certs/key.pem";

use shared::bridge::{Bridge, BridgeInterface, Direction};
use shared::server::Listener;

/// Port the iptables REDIRECT rule sends egress TCP traffic to.
const TCP_PROXY_PORT: u16 = 8080;
const DNS_LISTEN_ADDR: &str = "127.0.0.1:53";
/// Port the data-plane listens on for ingress from the control-plane.
const INGRESS_LISTEN_PORT: u16 = 7777;
/// Port the control-plane's TCP proxy is listening on.
const CONTROL_PLANE_TCP_PORT: u16 = 8181;
/// Port the control-plane's DNS proxy is listening on.
const CONTROL_PLANE_DNS_PORT: u16 = 5354;

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

async fn handle_tcp_connection(mut client: TcpStream, client_addr: SocketAddr) {
    let original_dst = match get_original_dst(&client) {
        Ok(dst) => dst,
        Err(e) => {
            warn!(client = %client_addr, error = %e, "tcp: failed to get original destination, dropping connection");
            return;
        }
    };

    info!(
        client = %client_addr,
        destination = %original_dst,
        "tcp: accepted connection, forwarding to control plane"
    );

    let mut upstream =
        match Bridge::get_client_connection(CONTROL_PLANE_TCP_PORT, Direction::EnclaveToHost)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                error!(error = %e, "tcp: failed to connect to control plane");
                return;
            }
        };

    // Send 6-byte header: [4 bytes IPv4 big-endian][2 bytes port big-endian]
    let ip_bytes = original_dst.ip().octets();
    let port_bytes = original_dst.port().to_be_bytes();
    let header = [
        ip_bytes[0],
        ip_bytes[1],
        ip_bytes[2],
        ip_bytes[3],
        port_bytes[0],
        port_bytes[1],
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

async fn run_tcp_proxy() {
    let listener = TcpListener::bind(format!("0.0.0.0:{TCP_PROXY_PORT}"))
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

// ── DNS proxy (UDP → TCP tunnel to control plane) ────────────────────────────

async fn run_dns_proxy() {
    let listen_addr = std::env::var("DNS_LISTEN_ADDR")
        .unwrap_or_else(|_| DNS_LISTEN_ADDR.to_string());

    let socket = Arc::new(
        UdpSocket::bind(&listen_addr)
            .await
            .expect("failed to bind DNS proxy"),
    );

    info!(addr = %listen_addr, "dns proxy listening");

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
        let sock = Arc::clone(&socket);

        tokio::spawn(async move {
            info!(client = %client_addr, bytes = len, "dns: received query, forwarding to control plane");

            // One bridge connection per DNS query.
            let mut stream = match Bridge::get_client_connection(
                CONTROL_PLANE_DNS_PORT,
                Direction::EnclaveToHost,
            )
            .await
            {
                Ok(s) => s,
                Err(e) => {
                    error!(error = %e, "dns: failed to connect to control plane");
                    return;
                }
            };

            if let Err(e) = stream.write_all(&query).await {
                error!(error = %e, "dns: failed to write query");
                return;
            }
            // Signal end of request so the control-plane's read_to_end completes.
            if let Err(e) = stream.shutdown().await {
                error!(error = %e, "dns: failed to shutdown write side");
                return;
            }

            let mut resp = [0u8; 512];
            let resp_len = match stream.read(&mut resp).await {
                Ok(n) if n > 0 => n,
                Ok(_) => {
                    error!("dns: empty response from control plane");
                    return;
                }
                Err(e) => {
                    error!(error = %e, "dns: failed to read response");
                    return;
                }
            };

            info!(client = %client_addr, bytes = resp_len, "dns: received response, forwarding to client");

            if let Err(e) = sock.send_to(&resp[..resp_len], client_addr).await {
                error!(client = %client_addr, error = %e, "dns: failed to send response to client");
            }
        });
    }
}

// ── TCP ingress listener ──────────────────────────────────────────────────────

fn make_tls_acceptor() -> tokio_rustls::TlsAcceptor {
    use tokio_rustls::rustls::ServerConfig;
    use rustls_pemfile::{certs, private_key};
    use std::fs::File;
    use std::io::BufReader;

    let cert_path =
        std::env::var("TLS_CERT_PATH").unwrap_or_else(|_| TLS_CERT_PATH.to_string());
    let key_path =
        std::env::var("TLS_KEY_PATH").unwrap_or_else(|_| TLS_KEY_PATH.to_string());

    let cert_chain = certs(&mut BufReader::new(
        File::open(&cert_path).expect("TLS cert file not found"),
    ))
    .collect::<Result<Vec<_>, _>>()
    .expect("failed to parse TLS certificate");

    let key = private_key(&mut BufReader::new(
        File::open(&key_path).expect("TLS key file not found"),
    ))
    .expect("failed to read TLS key file")
    .expect("no private key found in TLS key file");

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)
        .expect("invalid TLS server config");

    tokio_rustls::TlsAcceptor::from(Arc::new(config))
}

async fn handle_ingress_connection<S>(mut client: S, app_addr: String)
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    info!(app = %app_addr, "ingress: new connection, forwarding to app");

    let mut upstream = match TcpStream::connect(&app_addr).await {
        Ok(s) => s,
        Err(e) => {
            error!(app = %app_addr, error = %e, "ingress: failed to connect to app");
            return;
        }
    };

    match io::copy_bidirectional(&mut client, &mut upstream).await {
        Ok((from_client, from_upstream)) => {
            info!(
                bytes_from_client = from_client,
                bytes_from_upstream = from_upstream,
                "ingress: connection closed"
            );
        }
        Err(e) => {
            warn!(error = %e, "ingress: connection error");
        }
    }
}

async fn run_ingress_listener(app_addr: String) {
    let tls_acceptor = make_tls_acceptor();

    let mut listener = Bridge::get_listener(INGRESS_LISTEN_PORT, Direction::HostToEnclave)
        .await
        .expect("failed to bind ingress listener");

    info!(port = INGRESS_LISTEN_PORT, app = %app_addr, "ingress listener");

    loop {
        match listener.accept().await {
            Ok(stream) => {
                let app = app_addr.clone();
                let acceptor = tls_acceptor.clone();

                tokio::spawn(async move {
                    match acceptor.accept(stream).await {
                        Ok(tls_stream) => handle_ingress_connection(tls_stream, app).await,
                        Err(e) => error!(error = %e, "ingress: TLS handshake failed"),
                    }
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

    let app_port = std::env::var("APP_PORT").unwrap_or_else(|_| "8008".to_string());
    let app_addr = format!("127.0.0.1:{app_port}");

    info!("data-plane starting");

    tokio::join!(
        run_tcp_proxy(),
        run_dns_proxy(),
        run_ingress_listener(app_addr),
    );
}

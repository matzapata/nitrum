//! Ingress proxy: accepts TLS connections, terminates TLS, and forwards plain-text to the user app.

use tokio::io;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tracing::{error, info, warn};

pub async fn run(listen_addr: String, app_addr: String, acceptor: TlsAcceptor) {
    let listener = TcpListener::bind(&listen_addr)
        .await
        .unwrap_or_else(|e| panic!("ingress: failed to bind {listen_addr}: {e}"));

    info!(listen = %listen_addr, app = %app_addr, "ingress TLS proxy listening");

    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                let acceptor = acceptor.clone();
                let app = app_addr.clone();
                tokio::spawn(async move {
                    let tls = match acceptor.accept(stream).await {
                        Ok(s) => s,
                        Err(e) => {
                            warn!(peer = %peer, error = %e, "ingress: TLS handshake failed");
                            return;
                        }
                    };
                    if let Err(e) = handle(tls, peer, &app).await {
                        warn!(peer = %peer, error = %e, "ingress: connection error");
                    }
                });
            }
            Err(e) => error!(error = %e, "ingress: accept failed"),
        }
    }
}

async fn handle<S>(mut inbound: S, peer: std::net::SocketAddr, app_addr: &str) -> io::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut outbound = TcpStream::connect(app_addr).await.map_err(|e| {
        warn!(app = %app_addr, error = %e, "ingress: could not connect to app");
        e
    })?;

    info!(peer = %peer, app = %app_addr, "ingress: proxying connection");

    let (from_client, from_app) = io::copy_bidirectional(&mut inbound, &mut outbound).await?;

    info!(
        peer = %peer,
        bytes_in = from_client,
        bytes_out = from_app,
        "ingress: connection closed"
    );
    Ok(())
}

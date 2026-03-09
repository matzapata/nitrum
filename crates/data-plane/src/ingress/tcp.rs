use std::sync::Arc;

use tokio::io::{self, AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tracing::{error, info, warn};

use shared::bridge::{Bridge, BridgeInterface, Direction};
use shared::server::Listener;
use shared::ports;

use crate::config::TlsTermination;

fn make_tls_acceptor(domain: &str) -> tokio_rustls::TlsAcceptor {
    use rcgen::generate_simple_self_signed;
    use tokio_rustls::rustls::ServerConfig;
    use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

    let certified_key = generate_simple_self_signed(vec![domain.to_string()])
        .expect("failed to generate self-signed TLS certificate");

    let cert_der = CertificateDer::from(certified_key.cert.der().to_vec());
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
        certified_key.signing_key.serialize_der(),
    ));

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .expect("invalid TLS server config");

    tokio_rustls::TlsAcceptor::from(Arc::new(config))
}

async fn handle_connection<S>(mut client: S, app_addr: String)
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

pub async fn run(app_addr: String, tls: TlsTermination) {
    let tls_acceptor = if tls.enabled {
        let acceptor = make_tls_acceptor(&tls.domain);
        info!(domain = %tls.domain, "generated self-signed TLS certificate");
        Some(acceptor)
    } else {
        info!("TLS termination disabled, ingress will forward plain TCP");
        None
    };

    let mut listener = Bridge::get_listener(ports::INGRESS, Direction::HostToEnclave)
        .await
        .expect("failed to bind ingress listener");

    info!(port = ports::INGRESS, app = %app_addr, "ingress listener");

    loop {
        match listener.accept().await {
            Ok(stream) => {
                let app = app_addr.clone();
                let acceptor = tls_acceptor.clone();

                tokio::spawn(async move {
                    match acceptor {
                        Some(acceptor) => match acceptor.accept(stream).await {
                            Ok(tls_stream) => handle_connection(tls_stream, app).await,
                            Err(e) => error!(error = %e, "ingress: TLS handshake failed"),
                        },
                        None => handle_connection(stream, app).await,
                    }
                });
            }
            Err(e) => error!(error = %e, "ingress: accept error"),
        }
    }
}

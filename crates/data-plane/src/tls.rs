use rcgen::generate_simple_self_signed;
use std::sync::Arc;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tracing::info;

/// Build a `TlsAcceptor` from a freshly generated self-signed certificate.
pub fn self_signed(domains: Vec<String>) -> TlsAcceptor {
    let cert =
        generate_simple_self_signed(domains).expect("failed to generate self-signed certificate");

    let cert_der = CertificateDer::from(cert.cert.der().to_vec());
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()));

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .expect("invalid TLS config");

    info!("generated self-signed TLS certificate");

    TlsAcceptor::from(Arc::new(config))
}

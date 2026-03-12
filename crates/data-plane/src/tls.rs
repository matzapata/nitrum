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

/// Build a `TlsAcceptor` from a ACME certificate.
pub fn acme(domains: Vec<String>, email: Vec<String>) {
    let mut state = (if !cfg!(feature = "enclave") {
        // TODO: maybe use different flag, like "dev" or alike
        // when using acme outside of aws we default to using pebble

        // get the pebble minica cert and directory from the environment variables
        let pebble_minica_cert = std::env::var("PEBBLE_MINICA_CERT")
            .expect("PEBBLE_MINICA_CERT must be set when using acme outside of aws");
        let pebble_directory = std::env::var("PEBBLE_DIRECTORY")
            .unwrap_or_else(|_| "https://pebble:14000/dir".to_string());

        // read the minica cert and add it to the root store
        let minica_pem = std::fs::read(&pebble_minica_cert)
            .unwrap_or_else(|e| panic!("failed to read {}: {}", pebble_minica_cert, e));
        let pems = pem::parse_many(&minica_pem).expect("failed to parse minica PEM");
        let mut root_store = RootCertStore::empty();
        for p in pems {
            let der = rustls::pki_types::CertificateDer::from(p.into_contents());
            root_store.add(der).expect("failed to add minica cert");
        }
        let client_config = Arc::new(
            ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth(),
        );

        AcmeConfig::new_with_client_tls_config(domains.clone(), client_config)
            .directory(pebble_directory)
    } else {
        AcmeConfig::new(domains.clone()).directory_lets_encrypt(true) // production letsencrypt
    })
    .contact(email.iter().map(|e| format!("mailto:{e}")))
    .cache(DirCache::new("default")) // TODO: use dynamo encrypted cache
    .state();

    let rustls_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_cert_resolver(state.resolver());
    let acceptor = TlsAcceptor::from(Arc::new(rustls_config));

    tokio::spawn(async move {
        loop {
            match state.next().await.unwrap() {
                Ok(ok) => info!("event: {:?}", ok),
                Err(err) => error!("event: {:?}", err),
            }
        }
    });

    acceptor
}

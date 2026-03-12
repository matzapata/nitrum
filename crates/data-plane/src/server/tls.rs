//! TLS acceptor: shared cert from DynamoDB (leader creates/renews) or ephemeral self-signed.
//! TODO: self signed, but also shared, then also add acme

use std::sync::Arc;

use anyhow::{Context, Result};
use rcgen::generate_simple_self_signed;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;
use tracing::info;

use crate::storage::{keys, InfraClients};

/// Build a shared TLS acceptor: leader generates and stores cert in DynamoDB,
/// others load it. Fallback to ephemeral self-signed if none stored yet.
pub async fn acceptor(infra: &InfraClients, domain: &str) -> Result<TlsAcceptor> {
    // If we can become leader, try to create/renew the cert.
    if let Some(_guard) = infra.leader.try_acquire_leader().await? {
        let cert = generate_simple_self_signed(vec![domain.to_string()])
            .context("failed to generate self-signed certificate")?;

        let cert_der = cert.cert.der().to_vec();
        let key_der = cert.signing_key.serialize_der();

        infra
            .storage
            .set_object(keys::CERTIFICATE_OBJECT_KEY, &cert_der)
            .await?;
        infra
            .storage
            .set_object(keys::CERTIFICATE_PRIVATE_KEY_OBJECT_KEY, &key_der)
            .await?;

        info!("stored TLS certificate in DynamoDB (leader)");
        return build_acceptor_from_der(&cert_der, &key_der);
    }

    // Not leader: try to load existing cert from DynamoDB.
    if let (Some(cert_der), Some(key_der)) = (
        infra
            .storage
            .get_object(keys::CERTIFICATE_OBJECT_KEY)
            .await?,
        infra
            .storage
            .get_object(keys::CERTIFICATE_PRIVATE_KEY_OBJECT_KEY)
            .await?,
    ) {
        info!("loaded TLS certificate from DynamoDB");
        return build_acceptor_from_der(&cert_der, &key_der);
    }

    // No cert in DynamoDB: ephemeral self-signed for this instance only.
    info!("no shared cert in DynamoDB, using ephemeral self-signed");
    Ok(self_signed(vec![domain.to_string()]))
}

fn build_acceptor_from_der(cert_der: &[u8], key_der: &[u8]) -> Result<TlsAcceptor> {
    let cert_der = CertificateDer::from(cert_der.to_vec());
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der.to_vec()));

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .context("invalid TLS cert/key from DynamoDB")?;

    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// Build a `TlsAcceptor` from a freshly generated self-signed certificate (ephemeral, not shared).
pub fn self_signed(domains: Vec<String>) -> TlsAcceptor {
    let cert =
        generate_simple_self_signed(domains).expect("failed to generate self-signed certificate");

    let cert_der = CertificateDer::from(cert.cert.der().to_vec());
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()));

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .expect("invalid TLS config");

    info!("generated ephemeral self-signed TLS certificate");
    TlsAcceptor::from(Arc::new(config))
}

//! TLS config: certificate provisioning via self-signed (ephemeral bootstrap) or ACME.
//!
//! When `TlsTermination.acme` is true, [`super::acme`] handles Let's Encrypt (or Pebble).
//! Otherwise the server starts with a generated self-signed cert until shared storage
//! provides a cert (see deployment docs). Cert PEM + key are stored in shared storage for ACME,
//! wrapped with the data-plane DEK (AES-GCM).

use crate::state::DataPlaneState;
use crate::storage::keys;
use crate::utils::leader::Leader;
use anyhow::{Context, Result};
use axum_server::tls_rustls::RustlsConfig;
use sha2::{Digest, Sha256};
use std::io::BufReader;
use std::sync::Arc;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::pki_types::CertificateDer;
use tracing::info;

use super::acme::{AcmeEvent, AcmeState};

#[cfg(feature = "pebble")]
use super::acme::pebble_client_tls_config;

use crate::constants::{ENV_ACME_DIRECTORY_URL, LETS_ENCRYPT_PROD_DIRECTORY};

/// Re-export for ingress (`tls::challenge_handler`).
pub use super::acme::challenge_handler;

// ---------------------------------------------------------------------------
// TlsState
// ---------------------------------------------------------------------------

/// TLS state machine: provides `RustlsConfig` for the server and `.next()` to drive ACME events.
pub struct TlsState {
    state: Arc<DataPlaneState>,
    rustls_config: RustlsConfig,
    acme_state: Option<AcmeState>,
}

impl TlsState {
    /// Build the TLS state. Use `.rustls_config()` for the server and spawn
    /// `.next()` in a loop to drive provisioning/renewal and log events.
    #[must_use]
    pub fn new(state: Arc<DataPlaneState>) -> Self {
        let domain = state.config.tls_termination.domain.clone();
        let acme_enabled = state.config.tls_termination.acme;
        let (server_config, cert_hash) = ephemeral_server_config(std::slice::from_ref(&domain));
        *state.tls_cert_hash.write().unwrap() = Some(cert_hash);
        let rustls_config = RustlsConfig::from_config(server_config);

        if !acme_enabled {
            return Self {
                state,
                rustls_config,
                acme_state: None,
            };
        }

        let acme_leader = Arc::new(Leader::new(
            state.storage.clone(),
            state.config.instance_id.clone(),
            keys::ACME_LEADER_KEY.to_string(),
        ));
        let directory_url = std::env::var(ENV_ACME_DIRECTORY_URL)
            .unwrap_or_else(|_| LETS_ENCRYPT_PROD_DIRECTORY.to_string());

        #[cfg(feature = "pebble")]
        let client_tls_config = Some(pebble_client_tls_config().expect("pebble_client_tls_config"));
        #[cfg(not(feature = "pebble"))]
        let client_tls_config = None;

        let storage_for_acme = state.storage.clone();
        let crypto_for_acme = state.crypto.clone();

        Self {
            state,
            rustls_config,
            acme_state: Some(AcmeState::new(
                domain,
                storage_for_acme,
                crypto_for_acme,
                acme_leader,
                directory_url,
                client_tls_config,
            )),
        }
    }

    /// `RustlsConfig` to pass to `bind_rustls`. Hot-reloaded when ACME renews.
    #[must_use]
    pub fn rustls_config(&self) -> RustlsConfig {
        self.rustls_config.clone()
    }

    /// Drive the ACME state machine. Returns `Ok(event)` on success, `Err` on failure.
    /// When `CertRenewed` is returned the `RustlsConfig` has already been hot-reloaded.
    pub async fn next(&mut self) -> Result<AcmeEvent> {
        let needs_provision = self
            .acme_state
            .as_ref()
            .is_some_and(|a| a.current_chain.is_none());
        if needs_provision {
            let (chain, key) = {
                let acme = self.acme_state.as_mut().unwrap();
                acme.get_or_provision().await?
            };
            {
                let acme = self.acme_state.as_mut().unwrap();
                acme.current_chain = Some(chain.clone());
                *acme.cert_store.write().await = Some((chain.clone(), key.clone()));
            }
            self.apply_cert(&chain, &key);
            return Ok(AcmeEvent::CertIssued);
        }

        if self.acme_state.is_none() {
            return Ok(AcmeEvent::CertIssued);
        }

        let event = {
            let acme = self.acme_state.as_mut().unwrap();
            acme.next().await?
        };

        match event {
            AcmeEvent::CertRenewed => {
                let (cert_opt, domain) = {
                    let acme = self.acme_state.as_ref().unwrap();
                    (acme.cert_store.read().await.clone(), acme.domain.clone())
                };
                if let Some((chain, key)) = cert_opt {
                    self.apply_cert(&chain, &key);
                    info!(domain = %domain, "ACME certificate renewed, TLS config reloaded");
                }
                Ok(AcmeEvent::CertRenewed)
            }
            other @ AcmeEvent::CertIssued => Ok(other),
        }
    }

    fn apply_cert(&self, chain: &str, key: &str) {
        if let Some(hash) = cert_hash_from_chain_pem(chain) {
            *self.state.tls_cert_hash.write().unwrap() = Some(hash);
        }
        if let Ok(cfg) = build_server_config_from_pem(chain, key) {
            self.rustls_config.reload_from_config(cfg);
        }
    }
}

// ---------------------------------------------------------------------------
// ServerConfig helpers + cert hash
// ---------------------------------------------------------------------------

/// Returns SHA-256 hash of the first (leaf) certificate in a PEM chain.
#[must_use]
pub fn cert_hash_from_chain_pem(chain_pem: &str) -> Option<Vec<u8>> {
    let certs: Vec<CertificateDer<'static>> =
        rustls_pemfile::certs(&mut BufReader::new(chain_pem.as_bytes()))
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
    Some(Sha256::digest(certs.first()?.as_ref()).to_vec())
}

fn ephemeral_server_config(domains: &[String]) -> (Arc<ServerConfig>, Vec<u8>) {
    let cert = rcgen::generate_simple_self_signed(domains.to_vec())
        .expect("failed to generate self-signed certificate");
    let chain_pem = cert.cert.pem();
    let key_pem = cert.signing_key.serialize_pem();
    let hash = cert_hash_from_chain_pem(&chain_pem).expect("hash from generated cert");
    let config = build_server_config_from_pem(&chain_pem, &key_pem)
        .expect("ephemeral TLS from generated cert");
    (config, hash)
}

/// Build a rustls [`ServerConfig`] from PEM-encoded certificate chain and private key.
#[doc(hidden)]
pub fn build_server_config_from_pem(chain_pem: &str, key_pem: &str) -> Result<Arc<ServerConfig>> {
    let certs: Vec<CertificateDer<'static>> =
        rustls_pemfile::certs(&mut BufReader::new(chain_pem.as_bytes()))
            .collect::<Result<Vec<_>, _>>()
            .context("parse PEM chain")?;
    let key = rustls_pemfile::private_key(&mut BufReader::new(key_pem.as_bytes()))
        .context("parse PEM key")?
        .context("no private key in PEM")?;
    Ok(Arc::new(
        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .context("build ServerConfig from PEM")?,
    ))
}

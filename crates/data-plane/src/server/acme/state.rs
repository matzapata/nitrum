//! ACME leader election, storage-backed cert, and renewal loop.

use super::client::AcmeClient;
use super::storage::AcmeStorage;
use crate::constants::{ACME_LOCK_RETRY_INTERVAL, CERTIFICATE_RENEWAL_FRACTION};
use crate::crypto::CryptoClient;
use crate::storage::StorageClient;
use crate::utils::leader::Leader;
use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::info;

/// In-memory cert store (chain PEM, key PEM). Used by ACME renewal loop.
pub type CertStore = Arc<RwLock<Option<(String, String)>>>;

#[derive(Debug)]
pub enum AcmeEvent {
    CertIssued,
    CertRenewed,
}

pub struct AcmeState {
    pub(crate) domain: String,
    cert_storage: AcmeStorage,
    leader: Arc<Leader>,
    pub(crate) cert_store: CertStore,
    inner: AcmeClient,
    pub(crate) current_chain: Option<String>,
}

impl AcmeState {
    pub fn new(
        domain: String,
        storage: Arc<StorageClient>,
        crypto: Arc<CryptoClient>,
        leader: Arc<Leader>,
        directory_url: String,
        client_tls_config: Option<Arc<rustls::ClientConfig>>,
    ) -> Self {
        let cert_storage = AcmeStorage::new(storage.clone(), crypto.clone());
        Self {
            domain,
            cert_storage,
            leader,
            cert_store: Arc::new(RwLock::new(None)),
            inner: AcmeClient::new(storage, crypto, directory_url, client_tls_config),
            current_chain: None,
        }
    }

    pub fn cert_store(&self) -> CertStore {
        self.cert_store.clone()
    }

    /// Return an existing cert from storage or provision one (under leader lock only when needed).
    ///
    /// The ACME leader lock is taken only when storage is missing a cert/key pair or the stored
    /// leaf is due for renewal. While waiting for the lock, storage is re-polled so followers pick
    /// up a cert as soon as another instance writes it.
    pub async fn get_or_provision(&self) -> Result<(String, String)> {
        loop {
            if let Some(pair) = self.cert_storage.read_cert_pair().await? {
                if self.current_chain.as_deref() != Some(&pair.0) {
                    return Ok(pair);
                }
                if !Self::renewal_due(&self.inner, &pair.0) {
                    return Ok(pair);
                }
            }

            if let Some(_guard) = self.leader.try_acquire_leader().await? {
                if let Some(pair) = self.cert_storage.read_cert_pair().await? {
                    if self.current_chain.as_deref() != Some(&pair.0) {
                        return Ok(pair);
                    }
                    if !Self::renewal_due(&self.inner, &pair.0) {
                        return Ok(pair);
                    }
                }

                let (account, credentials) = self.inner.load_or_create_account().await?;
                if let Some(creds) = credentials {
                    self.inner.save_account(&creds).await?;
                }
                info!(domain = %self.domain, "provisioning ACME certificate");
                let (chain, key) = self
                    .inner
                    .provision_cert(account, &self.domain, self.cert_storage.as_ref())
                    .await?;
                self.cert_storage.write_cert_pair(&chain, &key).await?;
                return Ok((chain, key));
            }

            tokio::time::sleep(ACME_LOCK_RETRY_INTERVAL).await;
        }
    }

    /// Whether the leaf in `chain_pem` has reached the configured renewal point (or parsing failed).
    fn renewal_due(inner: &AcmeClient, chain_pem: &str) -> bool {
        inner
            .duration_until_renewal(chain_pem)
            .map_or(true, |d| d.as_secs() == 0)
    }

    /// Sleep until renewal time then renew. Call in a loop after `get_or_provision`.
    pub async fn next(&mut self) -> Result<AcmeEvent> {
        let chain = self
            .current_chain
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("no current chain"))?;
        let sleep_dur = self
            .inner
            .duration_until_renewal(chain)
            .unwrap_or_else(|_| {
                tracing::warn!("could not parse cert lifetime, sleeping 1h");
                Duration::from_hours(1)
            });
        tracing::info!(
            "next cert refresh in {} (at {:.0}% of cert life)",
            humantime::format_duration(sleep_dur),
            CERTIFICATE_RENEWAL_FRACTION * 100.0
        );
        tokio::time::sleep(sleep_dur).await;

        let (chain, key) = self.get_or_provision().await?;
        self.current_chain = Some(chain.clone());
        *self.cert_store.write().await = Some((chain, key));
        Ok(AcmeEvent::CertRenewed)
    }
}

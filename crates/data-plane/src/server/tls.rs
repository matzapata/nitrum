//! TLS config: certificate provisioning via self-signed (shared from storage) or ACME.
//!
//! Provider is chosen from `TlsTermination.acme`: when true use ACME (Let's Encrypt; with
//! `pebble` feature, use Pebble CA). Otherwise use self-signed (leader writes to storage, others load).
//! Cert and key are stored in PEM in the shared storage; both providers use the same keys.

use crate::constants::{
    CERTIFICATE_RENEWAL_FRACTION, ENV_ACME_DIRECTORY_URL, LETS_ENCRYPT_PROD_DIRECTORY,
};
use crate::state::DataPlaneState;
use crate::storage::StorageClient;
use crate::storage::keys;
use crate::utils::leader::Leader;
use anyhow::{Context, Result, bail};
use axum::response::IntoResponse;
use axum_server::tls_rustls::RustlsConfig;
use instant_acme::{
    Account, AccountCredentials, AuthorizationStatus, ChallengeType, Identifier, NewOrder,
    OrderStatus, RetryPolicy,
};
use rcgen::{CertificateParams, DistinguishedName, KeyPair};
use sha2::{Digest, Sha256};
use std::io::BufReader;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::pki_types::CertificateDer;
use tracing::{debug, info};
use x509_parser::parse_x509_certificate;

/// In-memory cert store (chain PEM, key PEM). Used by ACME renewal loop.
pub type CertStore = Arc<RwLock<Option<(String, String)>>>;

// ---------------------------------------------------------------------------
// TlsState
// ---------------------------------------------------------------------------

/// TLS state machine: provides RustlsConfig for the server and `.next()` to drive ACME events.
pub struct TlsState {
    state: Arc<DataPlaneState>,
    rustls_config: RustlsConfig,
    acme_state: Option<AcmeState>,
}

impl TlsState {
    /// Build the TLS state. Use `.rustls_config()` for the server and spawn
    /// `.next()` in a loop to drive provisioning/renewal and log events.
    pub fn new(state: Arc<DataPlaneState>) -> Self {
        let domain = state.config.tls_termination.domain.clone();
        let acme_enabled = state.config.tls_termination.acme;
        let (server_config, cert_hash) = ephemeral_server_config(&[domain.clone()]);
        *state.tls_cert_hash.write().unwrap() = Some(cert_hash);
        let rustls_config = RustlsConfig::from_config(server_config);

        if !acme_enabled {
            return TlsState {
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
        TlsState {
            state,
            rustls_config: rustls_config.clone(),
            acme_state: Some(AcmeState::new(
                domain,
                storage_for_acme,
                acme_leader,
                directory_url,
                client_tls_config,
            )),
        }
    }

    /// RustlsConfig to pass to `bind_rustls`. Hot-reloaded when ACME renews.
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
                    (
                        acme.cert_store.read().await.clone(),
                        acme.domain.clone(),
                    )
                };
                if let Some((chain, key)) = cert_opt {
                    self.apply_cert(&chain, &key);
                    info!(domain = %domain, "ACME certificate renewed, TLS config reloaded");
                }
                Ok(AcmeEvent::CertRenewed)
            }
            other => Ok(other),
        }
    }

    /// Apply a new cert/key pair: update the cert hash and reload the rustls config.
    fn apply_cert(&mut self, chain: &str, key: &str) {
        if let Some(hash) = cert_hash_from_chain_pem(chain) {
            *self.state.tls_cert_hash.write().unwrap() = Some(hash);
        }
        if let Ok(cfg) = build_server_config_from_pem(chain, key) {
            self.rustls_config.reload_from_config(cfg);
        }
    }
}

// ---------------------------------------------------------------------------
// ACME HTTP-01 challenge handler
// ---------------------------------------------------------------------------

pub fn challenge_handler(
    axum::extract::State(state): axum::extract::State<Arc<DataPlaneState>>,
    axum::extract::Path(token): axum::extract::Path<String>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = axum::response::Response> + Send>> {
    Box::pin(async move {
        match state.storage.get_object(&keys::acme_challenge_key(&token)).await {
            Ok(Some(body)) => {
                info!(token = %token, "ingress: ACME HTTP-01 challenge");
                (
                    axum::http::StatusCode::OK,
                    [("content-type", "application/octet-stream")],
                    bytes::Bytes::from(body),
                )
                    .into_response()
            }
            _ => (axum::http::StatusCode::NOT_FOUND, ()).into_response(),
        }
    })
}

// ---------------------------------------------------------------------------
// ServerConfig helpers + cert hash
// ---------------------------------------------------------------------------

/// Returns SHA-256 hash of the first (leaf) certificate in a PEM chain.
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

fn build_server_config_from_pem(chain_pem: &str, key_pem: &str) -> Result<Arc<ServerConfig>> {
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

// ---------------------------------------------------------------------------
// ACME certificate storage (shared PEM chain + key)
// ---------------------------------------------------------------------------

/// Reads/writes the ACME-issued certificate chain and private key in shared storage.
struct AcmeStorage {
    client: Arc<StorageClient>,
}

impl AcmeStorage {
    fn new(client: Arc<StorageClient>) -> Self {
        Self { client }
    }

    async fn read_cert_pair(&self) -> Result<Option<(String, String)>> {
        let cert = self
            .client
            .get_object(keys::CERTIFICATE_OBJECT_KEY)
            .await
            .context("read cert")?;
        let key = self
            .client
            .get_object(keys::CERTIFICATE_PRIVATE_KEY_OBJECT_KEY)
            .await
            .context("read key")?;
        match (cert, key) {
            (Some(c), Some(k)) => Ok(Some((
                String::from_utf8(c).context("cert not UTF-8")?,
                String::from_utf8(k).context("key not UTF-8")?,
            ))),
            _ => Ok(None),
        }
    }

    async fn write_cert_pair(&self, chain: &str, key: &str) -> Result<()> {
        self.client
            .set_object(keys::CERTIFICATE_OBJECT_KEY, chain.as_bytes())
            .await
            .context("write cert to storage")?;
        self.client
            .set_object(keys::CERTIFICATE_PRIVATE_KEY_OBJECT_KEY, key.as_bytes())
            .await
            .context("write key to storage")?;
        Ok(())
    }
}

impl AsRef<StorageClient> for AcmeStorage {
    fn as_ref(&self) -> &StorageClient {
        &self.client
    }
}

// ---------------------------------------------------------------------------
// AcmeClient
// ---------------------------------------------------------------------------

struct AcmeClient {
    storage: Arc<StorageClient>,
    directory_url: String,
    client_tls_config: Option<Arc<rustls::ClientConfig>>,
}

impl AcmeClient {
    fn new(
        storage: Arc<StorageClient>,
        directory_url: String,
        client_tls_config: Option<Arc<rustls::ClientConfig>>,
    ) -> Self {
        Self { storage, directory_url, client_tls_config }
    }

    async fn load_or_create_account(&self) -> Result<(Account, Option<AccountCredentials>)> {
        use instant_acme::NewAccount;

        let new_account = NewAccount {
            contact: &[],
            terms_of_service_agreed: true,
            only_return_existing: false,
        };
        let stored = self
            .storage
            .get_object(keys::ACME_ACCOUNT_OBJECT_KEY)
            .await
            .context("read acme account from storage")?;

        // Macro to reduce the four near-identical builder branches.
        // Each branch: (a) restore from stored creds, or (b) create fresh.
        macro_rules! restore_or_create {
            ($builder:expr) => {{
                if let Some(ref data) = stored {
                    debug!(directory_url = %self.directory_url, "ACME: restoring account");
                    let creds: AccountCredentials =
                        serde_json::from_slice(data).context("parse acme account")?;
                    let account = $builder.from_credentials(creds).await.context("restore account")?;
                    return Ok((account, None));
                }
                info!(directory_url = %self.directory_url, "ACME: creating new account");
                let (account, creds) = $builder
                    .create(&new_account, self.directory_url.clone(), None)
                    .await
                    .with_context(|| format!("create acme account ({})", self.directory_url))?;
                Ok((account, Some(creds)))
            }};
        }

        if let Some(tls_config) = &self.client_tls_config {
            restore_or_create!(Account::builder_with_http(https_client::build(tls_config.clone())?))
        } else {
            restore_or_create!(Account::builder().context("create account builder")?)
        }
    }

    async fn save_account(&self, credentials: &AccountCredentials) -> Result<()> {
        let data = serde_json::to_string_pretty(credentials).context("serialize account")?;
        self.storage
            .set_object(keys::ACME_ACCOUNT_OBJECT_KEY, data.as_bytes())
            .await
            .context("write acme account to storage")
    }

    async fn provision_cert(
        &self,
        account: Account,
        domain: &str,
        storage: &StorageClient,
    ) -> Result<(String, String)> {
        let identifiers = [Identifier::Dns(domain.to_string())];
        debug!(domain, "ACME: creating new order");
        let mut order = account
            .new_order(&NewOrder::new(&identifiers))
            .await
            .with_context(|| format!("new order (domain={domain})"))?;

        let mut authorizations = order.authorizations();
        let mut authz = authorizations
            .next()
            .await
            .context("no authorizations")?
            .with_context(|| format!("get authorization (domain={domain})"))?;

        match authz.status {
            AuthorizationStatus::Pending => {
                let mut challenge = authz
                    .challenge(ChallengeType::Http01)
                    .ok_or_else(|| anyhow::anyhow!("no HTTP-01 challenge"))?;
                storage
                    .set_object(
                        &keys::acme_challenge_key(&challenge.token),
                        challenge.key_authorization().as_str().as_bytes(),
                    )
                    .await
                    .context("write challenge token")?;
                challenge.set_ready().await.context("set challenge ready")?;
            }
            AuthorizationStatus::Valid => {
                info!("authorization already valid, skipping challenge");
            }
            other => bail!("unexpected authorization status: {:?}", other),
        }

        let status = order.poll_ready(&RetryPolicy::default()).await.context("poll order ready")?;
        if status != OrderStatus::Ready {
            bail!("unexpected order status: {:?}", status);
        }

        let mut params = CertificateParams::new(vec![domain.to_owned()])?;
        params.distinguished_name = DistinguishedName::new();
        let private_key = KeyPair::generate()?;
        let csr = params.serialize_request(&private_key)?;
        order.finalize_csr(csr.der()).await.context("finalize csr")?;

        let chain = order
            .poll_certificate(&RetryPolicy::default())
            .await
            .context("poll certificate")?;
        Ok((chain, private_key.serialize_pem()))
    }

    fn duration_until_renewal(&self, chain_pem: &str) -> Result<Duration> {
        let certs = rustls_pemfile::certs(&mut BufReader::new(chain_pem.as_bytes()))
            .collect::<Result<Vec<_>, _>>()?;
        let leaf_der = certs.first().context("empty chain")?;
        let (_, cert) = parse_x509_certificate(leaf_der.as_ref()).context("parse leaf cert")?;
        let validity = cert.validity();
        let not_before = validity.not_before.timestamp();
        let not_after = validity.not_after.timestamp();
        let renew_at =
            not_before + ((not_after - not_before) as f64 * CERTIFICATE_RENEWAL_FRACTION) as i64;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;
        Ok(Duration::from_secs((renew_at - now).max(0) as u64))
    }
}

// ---------------------------------------------------------------------------
// AcmeState
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum AcmeEvent {
    CertIssued,
    CertRenewed,
}

pub struct AcmeState {
    domain: String,
    cert_storage: AcmeStorage,
    leader: Arc<Leader>,
    cert_store: CertStore,
    inner: AcmeClient,
    pub(crate) current_chain: Option<String>,
}

impl AcmeState {
    pub fn new(
        domain: String,
        storage: Arc<StorageClient>,
        leader: Arc<Leader>,
        directory_url: String,
        client_tls_config: Option<Arc<rustls::ClientConfig>>,
    ) -> Self {
        Self {
            domain,
            cert_storage: AcmeStorage::new(storage.clone()),
            leader,
            cert_store: Arc::new(RwLock::new(None)),
            inner: AcmeClient::new(storage, directory_url, client_tls_config),
            current_chain: None,
        }
    }

    pub fn cert_store(&self) -> CertStore {
        self.cert_store.clone()
    }

    /// Return an existing cert from storage or provision one (under leader lock).
    pub async fn get_or_provision(&mut self) -> Result<(String, String)> {
        // Fast path: storage already has a (different) cert.
        if let Some(pair) = self.cert_storage.read_cert_pair().await? {
            if self.current_chain.as_deref() != Some(&pair.0) {
                return Ok(pair);
            }
        }

        // Acquire leader lock (retry until obtained).
        let guard = loop {
            if let Some(g) = self.leader.try_acquire_leader().await? {
                break g;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        };

        // Re-check under lock.
        if let Some(pair) = self.cert_storage.read_cert_pair().await? {
            if self.current_chain.as_deref() != Some(&pair.0) {
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
        drop(guard);
        Ok((chain, key))
    }

    /// Sleep until renewal time then renew. Call in a loop after `get_or_provision`.
    pub async fn next(&mut self) -> Result<AcmeEvent> {
        let chain = self
            .current_chain
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("no current chain"))?;
        let sleep_dur = self.inner.duration_until_renewal(chain).unwrap_or_else(|_| {
            tracing::warn!("could not parse cert lifetime, sleeping 1h");
            Duration::from_secs(3600)
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

// ---------------------------------------------------------------------------
// Pebble (feature-gated)
// ---------------------------------------------------------------------------

#[cfg(feature = "pebble")]
fn pebble_client_tls_config() -> Result<Arc<rustls::ClientConfig>> {
    use rustls::RootCertStore;
    use std::io::Cursor;

    let value = std::env::var("PEBBLE_MINICA_CERT")
        .context("PEBBLE_MINICA_CERT not set (required when using pebble feature)")?;
    let certs = rustls_pemfile::certs(&mut BufReader::new(Cursor::new(value.as_bytes())))
        .collect::<Result<Vec<_>, _>>()
        .context("parse pebble CA from PEBBLE_MINICA_CERT")?;
    let mut roots = RootCertStore::empty();
    roots.add_parsable_certificates(certs);
    Ok(Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ))
}

// ---------------------------------------------------------------------------
// HTTPS client (for ACME with custom TLS, e.g. Pebble)
// ---------------------------------------------------------------------------

mod https_client {
    use super::*;
    use bytes::Bytes;
    use hyper_rustls::HttpsConnectorBuilder;
    use hyper_util::client::legacy::{Client, connect::HttpConnector};
    use hyper_util::rt::TokioExecutor;
    use instant_acme::BodyWrapper;

    pub fn build(
        tls_config: Arc<rustls::ClientConfig>,
    ) -> Result<Box<Client<hyper_rustls::HttpsConnector<HttpConnector>, BodyWrapper<Bytes>>>> {
        let https = HttpsConnectorBuilder::new()
            .with_tls_config((*tls_config).clone())
            .https_or_http()
            .enable_http1()
            .build();
        Ok(Box::new(Client::builder(TokioExecutor::new()).build(https)))
    }
}
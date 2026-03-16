//! TLS config: certificate provisioning via self-signed (shared from storage) or ACME.
//!
//! Provider is chosen from `TlsTermination.acme`: when true use ACME (Let's Encrypt; with
//! `pebble` feature, use Pebble CA). Otherwise use self-signed (leader writes to storage, others load).
//! Cert and key are stored in PEM in the shared storage; both providers use the same keys.
//! TODO: cleanups

use crate::constants::{CERTIFICATE_RENEWAL_FRACTION, ENV_ACME_DIRECTORY_URL, LETS_ENCRYPT_PROD_DIRECTORY};
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
// Public: TlsState (rustls_acme-style API)
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
        let tls_config = &state.config.tls_termination;
        let domain = tls_config.domain.clone();
        let (server_config, cert_hash) = ephemeral_server_config(&[domain]);
        *state.tls_cert_hash.write().unwrap() = Some(cert_hash);
        let rustls_config = RustlsConfig::from_config(server_config);

        if !tls_config.acme {
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
            .unwrap_or(LETS_ENCRYPT_PROD_DIRECTORY.to_string());
        #[cfg(feature = "pebble")]
        let client_tls_config = Some(pebble_client_tls_config().expect("pebble_client_tls_config"));
        #[cfg(not(feature = "pebble"))]
        let client_tls_config = None;

        let acme_state = AcmeState::new(
            tls_config.domain.clone(),
            state.storage.clone(),
            acme_leader,
            directory_url,
            client_tls_config,
        );

        TlsState {
            state,
            rustls_config,
            acme_state: Some(acme_state),
        }
    }

    /// RustlsConfig to pass to `bind_rustls`. Hot-reloaded when ACME renews.
    pub fn rustls_config(&self) -> RustlsConfig {
        self.rustls_config.clone()
    }

    /// Drive the ACME state machine. Returns Ok(event) on success, Err on failure. When
    /// CertRenewed is returned, the RustlsConfig has already been hot-reloaded.
    pub async fn next(&mut self) -> Result<AcmeEvent, anyhow::Error> {
        let acme = match &mut self.acme_state {
            None => return Ok(AcmeEvent::CertIssued),
            Some(a) => a,
        };

        if acme.current_chain.is_none() {
            let (chain, key) = acme.get_or_provision().await?;
            acme.current_chain = Some(chain.clone());
            *acme.cert_store.write().await = Some((chain.clone(), key.clone()));
            if let Some(hash) = cert_hash_from_chain_pem(&chain) {
                *self.state.tls_cert_hash.write().unwrap() = Some(hash);
            }
            if let Ok(cfg) = build_server_config_from_pem(&chain, &key) {
                self.rustls_config.reload_from_config(cfg);
            }
            return Ok(AcmeEvent::CertIssued);
        }
        
        match acme.next().await {
            Ok(AcmeEvent::CertRenewed) => {
                let (chain, key) = match acme.cert_store.read().await.as_ref() {
                    Some(pair) => (pair.0.clone(), pair.1.clone()),
                    None => return Ok(AcmeEvent::CertRenewed),
                };
                if let Some(hash) = cert_hash_from_chain_pem(&chain) {
                    *self.state.tls_cert_hash.write().unwrap() = Some(hash);
                }
                if let Ok(new_config) = build_server_config_from_pem(&chain, &key) {
                    self.rustls_config.reload_from_config(new_config);
                    info!(domain = %acme.domain, "ACME certificate renewed, TLS config reloaded");
                }
                Ok(AcmeEvent::CertRenewed)
            }
            other => other.map_err(Into::into),
        }
    }
}

// ---------------------------------------------------------------------------
// ACME HTTP-01 challenge handler (use with .with_state(state) on the router)
// ---------------------------------------------------------------------------

pub fn challenge_handler(
    axum::extract::State(state): axum::extract::State<Arc<DataPlaneState>>,
    axum::extract::Path(token): axum::extract::Path<String>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = axum::response::Response> + Send>> {
    Box::pin(async move {
        match state
            .storage
            .get_object(&keys::acme_challenge_key(&token))
            .await
        {
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
// Shared: build ServerConfig from PEM / ephemeral; cert hash for attestation
// ---------------------------------------------------------------------------

/// Returns SHA-256 hash of the first (leaf) certificate in a PEM chain.
pub fn cert_hash_from_chain_pem(chain_pem: &str) -> Option<Vec<u8>> {
    let certs: Vec<CertificateDer<'static>> =
        rustls_pemfile::certs(&mut BufReader::new(chain_pem.as_bytes()))
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
    let leaf = certs.first()?;
    Some(Sha256::digest(leaf.as_ref()).to_vec())
}

fn ephemeral_server_config(domains: &[String]) -> (Arc<ServerConfig>, Vec<u8>) {
    let cert = rcgen::generate_simple_self_signed(domains.to_vec())
        .expect("failed to generate self-signed certificate");
    let chain_pem = cert.cert.pem();
    let key_pem = cert.signing_key.serialize_pem();
    let hash = cert_hash_from_chain_pem(&chain_pem).expect("hash from generated cert");
    let config =
        build_server_config_from_pem(&chain_pem, &key_pem).expect("ephemeral TLS from generated cert");
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
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("build ServerConfig from PEM")?;
    Ok(Arc::new(config))
}

async fn read_cert_from_storage(storage: &StorageClient) -> Result<Option<(String, String)>> {
    let cert = storage
        .get_object(keys::CERTIFICATE_OBJECT_KEY)
        .await
        .context("read cert from storage")?;
    let key = storage
        .get_object(keys::CERTIFICATE_PRIVATE_KEY_OBJECT_KEY)
        .await
        .context("read key from storage")?;
    match (cert, key) {
        (Some(c), Some(k)) => {
            let chain = String::from_utf8(c).context("cert not UTF-8")?;
            let key_s = String::from_utf8(k).context("key not UTF-8")?;
            Ok(Some((chain, key_s)))
        }
        _ => Ok(None),
    }
}

async fn write_cert_to_storage(storage: &StorageClient, chain: &str, key: &str) -> Result<()> {
    storage
        .set_object(keys::CERTIFICATE_OBJECT_KEY, chain.as_bytes())
        .await
        .context("write cert to storage")?;
    storage
        .set_object(keys::CERTIFICATE_PRIVATE_KEY_OBJECT_KEY, key.as_bytes())
        .await
        .context("write key to storage")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// AcmeClient (internal)
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
        Self {
            storage,
            directory_url,
            client_tls_config,
        }
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

        if let Some(tls_config) = &self.client_tls_config {
            let http = https_client::build(tls_config.clone())?;
            if let Some(ref data) = stored {
                debug!(directory_url = %self.directory_url, "ACME: restoring account from storage");
                let creds: AccountCredentials =
                    serde_json::from_slice(data).context("parse acme account")?;
                let account = Account::builder_with_http(http)
                    .from_credentials(creds)
                    .await
                    .context("restore account")?;
                return Ok((account, None));
            }
            info!(directory_url = %self.directory_url, "ACME: creating new account");
            let (account, creds) =
                Account::builder_with_http(https_client::build(tls_config.clone())?)
                    .create(&new_account, self.directory_url.clone(), None)
                    .await
                    .with_context(|| {
                        format!("create acme account (directory_url={})", self.directory_url)
                    })?;
            return Ok((account, Some(creds)));
        }

        if let Some(ref data) = stored {
            debug!(directory_url = %self.directory_url, "ACME: restoring account from storage");
            let creds: AccountCredentials =
                serde_json::from_slice(data).context("parse acme account")?;
            let account = Account::builder()
                .context("create account builder")?
                .from_credentials(creds)
                .await
                .context("restore account")?;
            return Ok((account, None));
        }
        info!(directory_url = %self.directory_url, "ACME: creating new account");
        let (account, creds) = Account::builder()
            .context("create account builder")?
            .create(&new_account, self.directory_url.clone(), None)
            .await
            .with_context(|| {
                format!("create acme account (directory_url={})", self.directory_url)
            })?;
        Ok((account, Some(creds)))
    }

    async fn save_account(&self, credentials: &AccountCredentials) -> Result<()> {
        let data = serde_json::to_string_pretty(credentials).context("serialize account")?;
        self.storage
            .set_object(keys::ACME_ACCOUNT_OBJECT_KEY, data.as_bytes())
            .await
            .context("write acme account to storage")?;
        Ok(())
    }

    async fn provision_cert(
        &self,
        account: Account,
        domain: &str,
        storage: &StorageClient,
    ) -> Result<(String, String)> {
        let identifiers = [Identifier::Dns(domain.to_string())];
        debug!(domain = %domain, "ACME: creating new order");
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

        let status = order
            .poll_ready(&RetryPolicy::default())
            .await
            .context("poll order ready")?;
        if status != OrderStatus::Ready {
            bail!("unexpected order status: {:?}", status);
        }

        let mut params = CertificateParams::new(vec![domain.to_owned()])?;
        params.distinguished_name = DistinguishedName::new();
        let private_key = KeyPair::generate()?;
        let csr = params.serialize_request(&private_key)?;

        order
            .finalize_csr(csr.der())
            .await
            .context("finalize csr")?;

        let chain = order
            .poll_certificate(&RetryPolicy::default())
            .await
            .context("poll certificate")?;

        Ok((chain, private_key.serialize_pem()))
    }

    fn duration_until_renewal(&self, chain_pem: &str) -> Result<Duration> {
        let mut reader = BufReader::new(chain_pem.as_bytes());
        let certs = rustls_pemfile::certs(&mut reader).collect::<Result<Vec<_>, _>>()?;
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
// AcmeState: drives ACME with Leader for exclusive provisioning
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum AcmeEvent {
    CertIssued,
    CertRenewed,
}

pub struct AcmeState {
    domain: String,
    storage: Arc<StorageClient>,
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
            storage: storage.clone(),
            leader,
            cert_store: Arc::new(RwLock::new(None)),
            inner: AcmeClient::new(storage, directory_url, client_tls_config),
            current_chain: None,
        }
    }

    pub fn cert_store(&self) -> CertStore {
        self.cert_store.clone()
    }

    /// Get existing cert from storage or provision (holding leader lock). Blocks until we have a cert.
    pub async fn get_or_provision(&mut self) -> Result<(String, String)> {
        // Fast path: storage already has a cert
        if let Some((chain, key)) = read_cert_from_storage(&self.storage).await? {
            if self.current_chain.as_deref() != Some(&chain) {
                return Ok((chain, key));
            }
        }

        // Acquire leader lock (retry until we get it)
        let guard = loop {
            if let Some(g) = self.leader.try_acquire_leader().await? {
                break g;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        };

        // Re-check under lock
        if let Some((chain, key)) = read_cert_from_storage(&self.storage).await? {
            if self.current_chain.as_deref() != Some(&chain) {
                return Ok((chain, key));
            }
        }

        // Provision
        let (account, credentials) = self.inner.load_or_create_account().await?;
        if let Some(creds) = credentials {
            self.inner.save_account(&creds).await?;
        }
        info!(domain = %self.domain, "provisioning ACME certificate");
        let (chain, key) = self
            .inner
            .provision_cert(account, &self.domain, self.storage.as_ref())
            .await?;
        write_cert_to_storage(&self.storage, &chain, &key).await?;
        drop(guard);
        Ok((chain, key))
    }

    /// Sleep until 2/3 of cert lifetime then renew. Call after get_or_provision for renewal loop.
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

    let mut reader = BufReader::new(Cursor::new(value.as_bytes()));
    let certs = rustls_pemfile::certs(&mut reader)
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

// HTTPS client (for ACME with custom TLS e.g. Pebble)
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
        let client: Client<_, BodyWrapper<Bytes>> =
            Client::builder(TokioExecutor::new()).build(https);
        Ok(Box::new(client))
    }
}

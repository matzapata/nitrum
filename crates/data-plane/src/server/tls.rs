//! TLS config: certificate provisioning via self-signed (shared from storage) or ACME.
//!
//! Provider is chosen from `TlsTermination.acme`: when true use ACME (Let's Encrypt; with
//! `pebble` feature, use Pebble CA). Otherwise use self-signed (leader writes to storage, others load).
//! Cert and key are stored in PEM in the shared storage; both providers use the same keys.

use anyhow::{Context, Result, bail};
use axum_server::tls_rustls::RustlsConfig;
use instant_acme::{
    Account, AccountCredentials, AuthorizationStatus, ChallengeType, Identifier, NewOrder,
    OrderStatus, RetryPolicy,
};
use rcgen::{CertificateParams, DistinguishedName, KeyPair};
use std::io::BufReader;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::pki_types::CertificateDer;
use tracing::{debug, info, warn};
use x509_parser::parse_x509_certificate;
use crate::state::DataPlaneState;
use crate::storage::StorageClient;
use crate::storage::keys;
use crate::utils::leader::Leader;
use crate::constants::{CERTIFICATE_RENEWAL_FRACTION, LETS_ENCRYPT_STAGING_DIRECTORY};

/// In-memory cert store (chain PEM, key PEM). Used by ACME renewal loop.
pub type CertStore = Arc<RwLock<Option<(String, String)>>>;

/// Future that runs the ACME renewal loop. Spawn this when using ACME.
pub type AcmeRenewalLoop = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

// ---------------------------------------------------------------------------
// Public: build TLS config from state
// ---------------------------------------------------------------------------

pub struct TlsConfig {
    pub acme: bool,
    pub domain: String,
    pub acme_directory: Option<String>,
    acme_leader: Arc<Leader>,
}

impl TlsConfig {
    pub fn new(acme: bool, domain: String, acme_directory: Option<String>) -> Self {
        let acme_leader = Arc::new(Leader::new(
            state.storage.clone(),
            state.config.instance_id.clone(),
            keys::ACME_LEADER_KEY.to_string(),
        ));
        Self { acme, domain, acme_directory, acme_leader }
    }

    pub fn config(&self) -> Result<RustlsConfig> {
    }

    pub fn state(&self) -> TlsState {
    }

    async fn challenge_handler(
        State(app): State<AppState>,
        Path(token): Path<String>,
    ) -> impl IntoResponse {
        match app
            .state
            .storage
            .get_object(&keys::acme_challenge_key(&token))
            .await
        {
            Ok(Some(body)) => {
                info!(token = %token, "ingress: ACME HTTP-01 challenge");
                (
                    StatusCode::OK,
                    [("content-type", "application/octet-stream")],
                    Bytes::from(body),
                )
                    .into_response()
            }
            _ => (StatusCode::NOT_FOUND, ()).into_response(),
        }
    }
}

// ---------------------------------------------------------------------------
// ACME provider: initial config (blocks until first cert)
// ---------------------------------------------------------------------------

async fn provision_acme(
    state: &DataPlaneState,
    acme_leader: Arc<Leader>,
) -> Result<(RustlsConfig, Option<AcmeRenewalLoop>)> {
    let tls = &state.config.tls_termination;
    let domain = tls.domain.clone();

    // TODO: no pebble
    let directory_url = std::env::var("PEBBLE_DIRECTORY")
        .ok()
        .or_else(|| tls.acme_directory.clone())
        .unwrap_or_else(|| LETS_ENCRYPT_STAGING_DIRECTORY.to_string());
    let client_tls_config = {
        #[cfg(feature = "pebble")]
        {
            Some(pebble_client_config()?)
        }
        #[cfg(not(feature = "pebble"))]
        None
    };

    info!(
        directory_url = %directory_url,
        domain = %domain,
        "ACME: provisioning certificate"
    );

    let mut acme_state = AcmeState::new(
        domain.clone(),
        state.storage.clone(),
        acme_leader,
        directory_url.clone(),
        client_tls_config,
    );

    let (chain, key) = acme_state.get_or_provision().await.with_context(|| {
        format!("ACME provision (directory_url={directory_url}, domain={domain})")
    })?;
    acme_state.current_chain = Some(chain.clone());
    *acme_state.cert_store.write().await = Some((chain.clone(), key.clone()));

    let server_config = build_server_config_from_pem(&chain, &key)?;
    let tls_config = RustlsConfig::from_config(server_config);
    let tls_config_for_loop = tls_config.clone();

    let renewal_loop: AcmeRenewalLoop = Box::pin(async move {
        run_acme_renewal_loop(acme_state, tls_config_for_loop).await;
    });
    Ok((tls_config, Some(renewal_loop)))
}

/// Runs in a loop: sleep until 2/3 cert lifetime, then renew and hot-reload the TLS config.
async fn run_acme_renewal_loop(mut acme_state: AcmeState, config: RustlsConfig) {
    loop {
        match acme_state.next().await {
            Ok(AcmeEvent::CertRenewed) => {
                let (chain, key) = match acme_state.cert_store.read().await.as_ref() {
                    Some(pair) => (pair.0.clone(), pair.1.clone()),
                    None => continue,
                };
                match build_server_config_from_pem(&chain, &key) {
                    Ok(new_config) => {
                        config.reload_from_config(new_config);
                        info!(domain = %acme_state.domain, "ACME certificate renewed, TLS config reloaded");
                    }
                    Err(e) => warn!(error = %e, "renewal: failed to build server config"),
                }
            }
            Ok(AcmeEvent::CertIssued) => {}
            Err(e) => {
                warn!(error = %e, "renewal loop error, sleeping 60s");
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Shared: build ServerConfig from PEM
// ---------------------------------------------------------------------------

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
// TODO: cleanup

#[cfg(feature = "pebble")]
pub fn pebble_client_config() -> Result<Arc<rustls::ClientConfig>> {
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

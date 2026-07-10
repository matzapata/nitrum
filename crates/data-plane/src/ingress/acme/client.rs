//! ACME account, orders, and certificate provisioning.

use super::utils::acme_https_client;
use crate::constants::CERTIFICATE_RENEWAL_FRACTION;
use crate::crypto::CryptoClient;
use crate::storage::StorageClient;
use crate::storage::keys;
use anyhow::{Context, Result, bail};
use instant_acme::{
    Account, AccountCredentials, AuthorizationStatus, ChallengeType, Identifier, NewOrder,
    OrderStatus, RetryPolicy,
};
use rcgen::{CertificateParams, DistinguishedName, KeyPair};
use std::io::BufReader;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info};
use x509_parser::parse_x509_certificate;

pub struct AcmeClient {
    /// Persistent storage layer for ACME credentials.
    storage: Arc<StorageClient>,
    /// Encrypts ACME account JSON at rest (same DEK as TLS material).
    crypto: Arc<CryptoClient>,
    /// URL to the ACME directory endpoint.
    directory_url: String,
    /// Optional client TLS configuration for ACME endpoint.
    client_tls_config: Option<Arc<rustls::ClientConfig>>,
}

impl AcmeClient {
    pub(crate) const fn new(
        storage: Arc<StorageClient>,
        crypto: Arc<CryptoClient>,
        directory_url: String,
        client_tls_config: Option<Arc<rustls::ClientConfig>>,
    ) -> Self {
        Self {
            storage,
            crypto,
            directory_url,
            client_tls_config,
        }
    }

    pub(crate) async fn load_or_create_account(
        &self,
    ) -> Result<(Account, Option<AccountCredentials>)> {
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

        macro_rules! restore_or_create {
            ($builder:expr) => {{
                if let Some(ref data) = stored {
                    debug!(directory_url = %self.directory_url, "ACME: restoring account");
                    let plain = self
                        .crypto
                        .decrypt(data)
                        .context("decrypt acme account from storage")?;
                    let creds: AccountCredentials =
                        serde_json::from_slice(&plain).context("parse acme account")?;
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
            restore_or_create!(instant_acme::Account::builder_with_http(acme_https_client(
                tls_config.clone()
            )?))
        } else {
            restore_or_create!(instant_acme::Account::builder().context("create account builder")?)
        }
    }

    pub(crate) async fn save_account(&self, credentials: &AccountCredentials) -> Result<()> {
        let data = serde_json::to_string_pretty(credentials).context("serialize account")?;
        let enc = self
            .crypto
            .encrypt(data.as_bytes())
            .context("encrypt acme account for storage")?;
        self.storage
            .set_object(keys::ACME_ACCOUNT_OBJECT_KEY, &enc)
            .await
            .context("write acme account to storage")
    }

    pub(crate) async fn provision_cert(
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
            other => bail!("unexpected authorization status: {other:?}"),
        }

        let status = order
            .poll_ready(&RetryPolicy::default())
            .await
            .context("poll order ready")?;
        if status != OrderStatus::Ready {
            bail!("unexpected order status: {status:?}");
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

    /// Seconds until the leaf certificate in `chain_pem` expires.
    ///
    /// Saturates at zero when the certificate is already expired.
    pub(crate) fn duration_until_expiry(&self, chain_pem: &str) -> Result<Duration> {
        let certs = rustls_pemfile::certs(&mut BufReader::new(chain_pem.as_bytes()))
            .collect::<Result<Vec<_>, _>>()?;
        let leaf_der = certs.first().context("empty chain")?;
        let (_, cert) = parse_x509_certificate(leaf_der.as_ref()).context("parse leaf cert")?;
        let not_after = cert.validity().not_after.timestamp();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs()
            .cast_signed();
        Ok(Duration::from_secs(
            (not_after - now).max(0).cast_unsigned(),
        ))
    }

    pub(crate) fn duration_until_renewal(&self, chain_pem: &str) -> Result<Duration> {
        let certs = rustls_pemfile::certs(&mut BufReader::new(chain_pem.as_bytes()))
            .collect::<Result<Vec<_>, _>>()?;
        let leaf_der = certs.first().context("empty chain")?;
        let (_, cert) = parse_x509_certificate(leaf_der.as_ref()).context("parse leaf cert")?;
        let validity = cert.validity();
        let not_before = validity.not_before.timestamp();
        let not_after = validity.not_after.timestamp();
        let lifetime = not_after - not_before;
        let renew_at =
            not_before + ((lifetime as f64) * CERTIFICATE_RENEWAL_FRACTION).round() as i64;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs()
            .cast_signed();
        Ok(Duration::from_secs((renew_at - now).max(0).cast_unsigned()))
    }
}

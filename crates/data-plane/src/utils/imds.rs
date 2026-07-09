//! Enclave-side AWS credential source for the shared [`aws_config::SdkConfig`].
//!
//! [`EnclaveProvider`] implements [`ProvideCredentials`] and owns an [`ImdsClient`]
//! that caches short-lived `IMDSv2` role credentials by wall-clock time bucket.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use aws_credential_types::Credentials;
use aws_credential_types::provider::ProvideCredentials;
use aws_credential_types::provider::Result as ProviderResult;
use aws_credential_types::provider::error::CredentialsError;
use aws_credential_types::provider::future;
use serde::Deserialize;
use tokio::sync::Mutex;
use tracing::{debug, warn};

// ── IMDSv2 constants ─────────────────────────────────────────────────────────

/// `IMDSv2` token TTL (seconds). AWS allows up to 21600.
const TOKEN_TTL_SECS: u64 = 21600;

/// How often cached role credentials are refreshed (seconds).
const CREDENTIALS_REFRESH_SECS: u64 = 3600;

/// HTTP timeout for IMDS calls.
const METADATA_HTTP_TIMEOUT: Duration = Duration::from_secs(5);

// ── ImdsClient ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ImdsRoleCredentialsJson {
    access_key_id: String,
    secret_access_key: String,
    token: String,
}

struct CredCache {
    ttl_bucket: u64,
    credentials: Option<Credentials>,
}

/// `IMDSv2` client with built-in credential caching by wall-clock time bucket.
pub struct ImdsClient {
    /// Base URL including the `/latest` segment (no trailing slash).
    /// In a Nitro enclave this is typically `http://169.254.169.254/latest` when gvproxy runs with `-ec2-metadata-access`.
    latest_base: String,
    http: reqwest::Client,
    cache: Mutex<CredCache>,
}

impl std::fmt::Debug for ImdsClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ImdsClient")
    }
}

impl ImdsClient {
    /// Create a new client for the given IMDS base URL (including `/latest`; trailing slashes stripped).
    pub fn new(latest_base: impl AsRef<str>) -> Result<Self> {
        let latest_base = latest_base.as_ref().trim_end_matches('/').to_string();
        let http = reqwest::Client::builder()
            .timeout(METADATA_HTTP_TIMEOUT)
            .build()
            .context("failed to build HTTP client for IMDS")?;
        Ok(Self {
            latest_base,
            http,
            cache: Mutex::new(CredCache {
                ttl_bucket: 0,
                credentials: None,
            }),
        })
    }

    // ── internal helpers ────────────────────────────────────────────────────

    async fn get_token(&self) -> Result<String> {
        const ATTEMPTS: u32 = 4;
        const RETRY_DELAY: Duration = Duration::from_millis(250);

        let mut last_err = None;
        for attempt in 1..=ATTEMPTS {
            match self.fetch_token_once().await {
                Ok(token) => return Ok(token),
                Err(e) => {
                    last_err = Some(e);
                    if attempt < ATTEMPTS {
                        tokio::time::sleep(RETRY_DELAY).await;
                    }
                }
            }
        }
        Err(last_err.unwrap())
    }

    async fn fetch_token_once(&self) -> Result<String> {
        let base = &self.latest_base;
        self.http
            .put(format!("{base}/api/token"))
            .header(
                "X-aws-ec2-metadata-token-ttl-seconds",
                TOKEN_TTL_SECS.to_string(),
            )
            .send()
            .await
            .context("IMDSv2 token request failed")?
            .error_for_status()
            .context("IMDSv2 token non-success response")?
            .text()
            .await
            .context("failed to read IMDSv2 token body")
    }

    async fn get_meta(&self, suffix: &str) -> Result<String> {
        let token = self.get_token().await?;
        let base = &self.latest_base;
        let url = format!("{base}/{suffix}");
        let text = self
            .http
            .get(&url)
            .header("X-aws-ec2-metadata-token", &token)
            .send()
            .await
            .with_context(|| format!("IMDS request failed: {url}"))?
            .error_for_status()
            .with_context(|| format!("IMDS non-success response: {url}"))?
            .text()
            .await
            .with_context(|| format!("failed to read IMDS body: {url}"))?;
        Ok(text.trim().to_string())
    }

    fn ttl_bucket() -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now / CREDENTIALS_REFRESH_SECS
    }

    // ── public API ──────────────────────────────────────────────────────────

    /// Returns the AWS region this instance is running in.
    pub async fn get_region(&self) -> Result<String> {
        self.get_meta("meta-data/placement/region")
            .await
            .context("IMDS region request failed")
    }

    /// Fetch the AWS region, retrying with exponential backoff between attempts.
    ///
    /// `attempts` is the total number of tries; `initial_backoff` is the delay before
    /// the second attempt and doubles (saturating) after each subsequent failure.
    pub async fn get_region_with_retry(
        &self,
        attempts: u32,
        initial_backoff: Duration,
    ) -> Result<String> {
        let mut last_error = None;
        let mut backoff = initial_backoff;

        for attempt in 1..=attempts {
            match self.get_region().await {
                Ok(region) => return Ok(region),
                Err(error) => {
                    warn!(attempt, max_attempts = attempts, %error, "IMDS region fetch failed");
                    last_error = Some(error);
                    if attempt < attempts {
                        tokio::time::sleep(backoff).await;
                        backoff = backoff.saturating_mul(2);
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            anyhow::anyhow!("IMDS region fetch failed without a specific error")
        }))
    }

    /// Returns the EC2 instance ID.
    pub async fn instance_id(&self) -> Result<String> {
        self.get_meta("meta-data/instance-id")
            .await
            .context("IMDS instance-id request failed")
    }

    /// Returns the parent EC2 instance's private IPv4 address.
    pub async fn local_ipv4(&self) -> Result<String> {
        self.get_meta("meta-data/local-ipv4")
            .await
            .context("IMDS local-ipv4 request failed")
    }

    /// Fetch temporary IAM role credentials, refreshing only when the time bucket rolls over.
    pub async fn get_cached_role_credentials(&self) -> Result<Credentials> {
        let bucket = Self::ttl_bucket();
        let mut guard = self.cache.lock().await;

        if guard.ttl_bucket == bucket
            && let Some(ref c) = guard.credentials
        {
            return Ok(c.clone());
        }

        let creds = self.fetch_role_credentials().await?;
        guard.ttl_bucket = bucket;
        guard.credentials = Some(creds.clone());
        drop(guard);
        Ok(creds)
    }

    async fn fetch_role_credentials(&self) -> Result<Credentials> {
        let token = self.get_token().await?;
        let base = &self.latest_base;

        let role_list_url = format!("{base}/meta-data/iam/security-credentials/");
        let role_name = self
            .http
            .get(&role_list_url)
            .header("X-aws-ec2-metadata-token", &token)
            .send()
            .await
            .context("IMDS IAM role list request failed")?
            .error_for_status()
            .context("IMDS IAM role list non-success")?
            .text()
            .await
            .context("failed to read IMDS IAM role name body")?;

        let role_name = role_name
            .lines()
            .find(|l| !l.trim().is_empty())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .context("IMDS returned empty IAM role name")?
            .to_owned();

        let creds_url = format!("{base}/meta-data/iam/security-credentials/{role_name}");
        let creds_json = self
            .http
            .get(&creds_url)
            .header("X-aws-ec2-metadata-token", &token)
            .send()
            .await
            .context("IMDS IAM credentials request failed")?
            .error_for_status()
            .context("IMDS IAM credentials non-success")?
            .text()
            .await
            .context("failed to read IMDS IAM credentials body")?;

        let parsed: ImdsRoleCredentialsJson =
            serde_json::from_str(&creds_json).context("parse IMDS IAM credentials JSON")?;

        let session_token = (!parsed.token.is_empty()).then_some(parsed.token);
        let creds = Credentials::new(
            parsed.access_key_id,
            parsed.secret_access_key,
            session_token,
            None,
            "imds",
        );

        debug!(
            bucket = Self::ttl_bucket(),
            "IMDS role credentials refreshed"
        );
        Ok(creds)
    }
}

// ── EnclaveProvider ──────────────────────────────────────────────────────────

/// Credential provider used for all AWS SDK clients inside the enclave.
///
/// Wraps a shared [`ImdsClient`] (and its cache) in an [`Arc`] so the provider
/// is cheap to clone and hand to multiple SDK service clients.
#[derive(Clone, Debug)]
pub struct EnclaveProvider {
    imds: Arc<ImdsClient>,
}

impl EnclaveProvider {
    /// Create a provider with a new [`ImdsClient`] for `latest_base` (same rules as [`ImdsClient::new`]).
    #[allow(dead_code)]
    pub fn new(latest_base: impl AsRef<str>) -> Result<Self> {
        Ok(Self::with_imds(Arc::new(ImdsClient::new(latest_base)?)))
    }

    /// Share an existing [`ImdsClient`] (e.g. with [`super::ssm::SsmParameters`]).
    pub const fn with_imds(imds: Arc<ImdsClient>) -> Self {
        Self { imds }
    }

    async fn load_credentials(&self) -> ProviderResult {
        self.imds
            .get_cached_role_credentials()
            .await
            .map_err(CredentialsError::provider_error)
    }
}

impl ProvideCredentials for EnclaveProvider {
    fn provide_credentials<'a>(&'a self) -> future::ProvideCredentials<'a>
    where
        Self: 'a,
    {
        future::ProvideCredentials::new(self.load_credentials())
    }
}

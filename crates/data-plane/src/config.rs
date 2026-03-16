//! Data-plane runtime config: nitrum.toml plus env-derived infra settings.

use anyhow::{Context, Result};
use shared::config::Config as NitrumConfig;
use std::net::SocketAddr;
use std::ops::Deref;
use std::path::Path;

/// Data-plane runtime config: nitrum.toml plus env-derived infra settings.
#[derive(Clone)]
pub struct RuntimeConfig {
    /// User provided config.
    pub nitrum: NitrumConfig,
    /// DynamoDB table name (env: NITRUM_DYNAMODB_TABLE).
    pub dynamodb_table: String,
    /// Optional override (env: NITRUM_DYNAMODB_ENDPOINT_URL).
    pub dynamodb_endpoint: Option<String>,
    /// KMS key ID for DEK (env: NITRUM_KMS_KEY_ID).
    pub kms_key_id: String,
    /// Unique ID for this instance (leader lock owner). From IMDS or env fallback.
    pub instance_id: String,
    /// Ingress TLS listen address (env: NITRUM_INGRESS_LISTEN_ADDR).
    pub ingress_listen_addr: SocketAddr,
    /// ACME HTTP-01 challenge listen address (env: NITRUM_ACME_HTTP01_LISTEN_ADDR).
    pub acme_http01_listen_addr: SocketAddr,
    /// Crypto API listen address (env: NITRUM_CRYPTO_API_LISTEN_ADDR).
    pub crypto_api_listen_addr: SocketAddr,
}

impl Deref for RuntimeConfig {
    type Target = NitrumConfig;

    fn deref(&self) -> &Self::Target {
        &self.nitrum
    }
}

impl RuntimeConfig {
    /// Load infra settings from environment and IMDS. Requires NITRUM_DYNAMODB_TABLE and
    /// NITRUM_KMS_KEY_ID to be set (unless using dev fallbacks).
    pub async fn load(config_path: &Path) -> Result<Self> {
        let nitrum = shared::config::load(config_path);

        let dynamodb_endpoint = std::env::var("NITRUM_DYNAMODB_ENDPOINT_URL").ok();
        let dynamodb_table =
            std::env::var("NITRUM_DYNAMODB_TABLE").context("NITRUM_DYNAMODB_TABLE not set")?; // TODO: should come from imds as well
        let kms_key_id = std::env::var("NITRUM_KMS_KEY_ID").context("NITRUM_KMS_KEY_ID not set")?;
        let instance_id = crate::utils::imds::instance_id().await.unwrap_or_else(|_| {
            std::env::var("NITRUM_INSTANCE_ID")
                .unwrap_or_else(|_| format!("local-{}", std::process::id()))
        });

        let ingress_listen_addr: SocketAddr = std::env::var("NITRUM_INGRESS_LISTEN_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:443".to_string())
            .parse()
            .context("invalid NITRUM_INGRESS_LISTEN_ADDR")?;
        let acme_http01_listen_addr: SocketAddr = std::env::var("NITRUM_ACME_HTTP01_LISTEN_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:80".to_string())
            .parse()
            .context("invalid NITRUM_ACME_HTTP01_LISTEN_ADDR")?;
        let crypto_api_listen_addr: SocketAddr = std::env::var("NITRUM_CRYPTO_API_LISTEN_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:3000".to_string())
            .parse()
            .context("invalid NITRUM_CRYPTO_API_LISTEN_ADDR")?;

        Ok(Self {
            nitrum,
            dynamodb_table,
            kms_key_id,
            dynamodb_endpoint,
            instance_id,
            ingress_listen_addr,
            acme_http01_listen_addr,
            crypto_api_listen_addr,
        })
    }
}

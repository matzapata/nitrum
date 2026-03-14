//! Data-plane runtime config: nitrum.toml plus env-derived infra settings.

use anyhow::{Context, Result};
use std::path::Path;
use shared::config::Config as NitrumConfig;

/// Data-plane runtime config: nitrum.toml plus env-derived infra settings.
#[derive(Clone)]
pub struct RuntimeConfig {
    /// From nitrum.toml (service, tls_termination, etc.).
    pub nitrum: NitrumConfig,
    /// DynamoDB table name (env: NITRUM_DYNAMODB_TABLE).
    pub dynamodb_table: String,
    /// Optional override (env: NITRUM_DYNAMODB_ENDPOINT_URL).
    pub dynamodb_endpoint: Option<String>,
    /// KMS key ID for DEK (env: NITRUM_KMS_KEY_ID).
    pub kms_key_id: String,
    /// Unique ID for this instance (leader lock owner). From IMDS or env fallback.
    pub instance_id: String,
}

impl RuntimeConfig {
    /// Load infra settings from environment and IMDS. Requires NITRUM_DYNAMODB_TABLE and
    /// NITRUM_KMS_KEY_ID to be set (unless using dev fallbacks).
    pub async fn load(config_path: &Path) -> Result<Self> {
        let nitrum = shared::config::load(config_path);

        let dynamodb_endpoint = std::env::var("NITRUM_DYNAMODB_ENDPOINT_URL").ok();
        let dynamodb_table = std::env::var("NITRUM_DYNAMODB_TABLE")
            .context("NITRUM_DYNAMODB_TABLE not set")?; // TODO: should come from imds as well
        let kms_key_id = std::env::var("NITRUM_KMS_KEY_ID").context("NITRUM_KMS_KEY_ID not set")?;
        let instance_id = crate::utils::imds::instance_id()
            .await
            .unwrap_or_else(|_| {
                std::env::var("NITRUM_INSTANCE_ID")
                    .unwrap_or_else(|_| format!("local-{}", std::process::id()))
            });

        Ok(Self {
            nitrum,
            dynamodb_table,
            kms_key_id,
            dynamodb_endpoint,
            instance_id,
        })
    }
}

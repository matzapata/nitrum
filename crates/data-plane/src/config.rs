//! Data-plane runtime config: nitrum.toml plus env-derived infra settings.

use anyhow::{Context, Result};

use shared::config::Config as NitrumConfig;

// TODO: rename as data plane config or enclave config
/// Data-plane runtime config: nitrum.toml plus env-derived infra settings.
#[derive(Clone)]
pub struct AppConfig {
    /// From nitrum.toml (service, tls_termination, etc.).
    pub nitrum: NitrumConfig,
    /// DynamoDB table name (env: NITRUM_DYNAMODB_TABLE).
    pub dynamodb_table: String,
    /// KMS key ID for DEK (env: NITRUM_KMS_KEY_ID).
    pub kms_key_id: String,
    /// Optional override (env: NITRUM_DYNAMODB_ENDPOINT_URL).
    pub dynamodb_endpoint: Option<String>,
    /// Unique ID for this instance (leader lock owner). From IMDS or env fallback.
    pub instance_id: String,
}

impl AppConfig {
    /// Load infra settings from environment and IMDS. Requires NITRUM_DYNAMODB_TABLE and
    /// NITRUM_KMS_KEY_ID to be set (unless using dev fallbacks).
    pub async fn load(nitrum: NitrumConfig) -> Result<Self> {
        let dynamodb_table = std::env::var("NITRUM_DYNAMODB_TABLE")
            .context("NITRUM_DYNAMODB_TABLE not set")?;
        let kms_key_id = std::env::var("NITRUM_KMS_KEY_ID").context("NITRUM_KMS_KEY_ID not set")?;
        let dynamodb_endpoint = std::env::var("NITRUM_DYNAMODB_ENDPOINT_URL").ok();
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

    // TODO: load dev with feature flag
    /// Load with dev defaults when env vars are absent (e.g. local testing without AWS).
    pub async fn load_dev(nitrum: NitrumConfig) -> Self {
        let dynamodb_table = std::env::var("NITRUM_DYNAMODB_TABLE")
            .unwrap_or_else(|_| "nitrum-dev".to_string());
        let kms_key_id =
            std::env::var("NITRUM_KMS_KEY_ID").unwrap_or_else(|_| "alias/nitrum-dev".to_string());
        let dynamodb_endpoint = std::env::var("NITRUM_DYNAMODB_ENDPOINT_URL").ok();
        let instance_id = crate::utils::imds::instance_id()
            .await
            .unwrap_or_else(|_| {
                std::env::var("NITRUM_INSTANCE_ID")
                    .unwrap_or_else(|_| format!("dev-{}", std::process::id()))
            });

        Self {
            nitrum,
            dynamodb_table,
            kms_key_id,
            dynamodb_endpoint,
            instance_id,
        }
    }
}

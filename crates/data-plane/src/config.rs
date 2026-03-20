//! Data-plane runtime config: `nitrum.toml` plus IMDS + SSM infra settings.

use anyhow::{Context, Result};
use aws_config::BehaviorVersion;
use aws_config::Region;
use aws_credential_types::provider::SharedCredentialsProvider;
use shared::config::Config as NitrumConfig;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::ops::Deref;
use std::path::Path;
use std::sync::Arc;

use crate::utils::imds::{EnclaveProvider, ImdsClient};
use crate::utils::ssm::SsmParameters;
use crate::constants::DEFAULT_IMDS_LATEST_BASE_URL;

/// Data-plane runtime config: nitrum.toml plus infra settings.
#[derive(Clone)]
pub struct RuntimeConfig {
    /// User provided config.
    pub nitrum: NitrumConfig,
    /// IMDS base URL from `NITRUM_IMDS_BASE_URL` or [`DEFAULT_IMDS_LATEST_BASE_URL`] (no trailing slash).
    pub imds_latest_base_url: String,
    /// AWS region from IMDS.
    pub aws_region: String,
    /// AWS SDK config (credentials via IMDS through shared [`ImdsClient`]).
    pub aws_sdk_config: Arc<aws_config::SdkConfig>,
    /// Instance ID from IMDS.
    pub instance_id: String,
    /// DynamoDB table name from SSM.
    pub dynamodb_table: String,
    /// Optional DynamoDB API endpoint (`NITRUM_DYNAMODB_ENDPOINT_URL`).
    pub dynamodb_endpoint: Option<String>,
    /// KMS key ID from SSM.
    pub kms_key_id: String,
    /// Optional KMS API endpoint (`NITRUM_KMS_ENDPOINT_URL`).
    pub kms_endpoint: Option<String>,
    /// Ingress TLS listen address (`NITRUM_INGRESS_LISTEN_ADDR` or default `0.0.0.0:443`).
    pub ingress_listen_addr: SocketAddr,
    /// ACME HTTP-01 listen address (`NITRUM_ACME_HTTP01_LISTEN_ADDR` or default `0.0.0.0:80`).
    pub acme_http01_listen_addr: SocketAddr,
    /// Crypto API listen address (`NITRUM_CRYPTO_API_LISTEN_ADDR` or default `0.0.0.0:3000`).
    pub crypto_api_listen_addr: SocketAddr,
}

impl Deref for RuntimeConfig {
    type Target = NitrumConfig;

    fn deref(&self) -> &Self::Target {
        &self.nitrum
    }
}

impl RuntimeConfig {
    /// Load infra: [`ImdsClient`] for region, instance id, and SDK credentials; [`SsmParameters`]
    /// for `kms_key_id` and `dynamodb_table`. SSM parameter names are configured in
    /// [`SsmParameters::parameter_names`](crate::utils::ssm::SsmParameters::parameter_names)
    /// (env there only — not in this module).
    pub async fn load(config_path: &Path) -> Result<Self> {
        let nitrum = shared::config::load(config_path);

        let imds_latest_base_url = std::env::var("NITRUM_IMDS_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_IMDS_LATEST_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string();

        let imds = Arc::new(
            ImdsClient::new(&imds_latest_base_url).context("IMDS client init")?,
        );
        let aws_region = imds
            .get_region()
            .await
            .context("IMDS placement region")?;
        let instance_id = imds.instance_id().await.context("IMDS instance-id")?;
        let aws_sdk_config = Arc::new(
            aws_config::defaults(BehaviorVersion::latest())
                .region(Region::new(aws_region.clone()))
                .credentials_provider(SharedCredentialsProvider::new(
                    EnclaveProvider::with_imds(imds.clone()),
                ))
                .load()
                .await,
        );

        let ssm = SsmParameters::new(imds);
        let ssm_map = ssm
            .get_parameters_as_map()
            .await
            .context("SSM GetParameters for nitrum parameters")?;

        let dynamodb_endpoint = std::env::var("NITRUM_DYNAMODB_ENDPOINT_URL").ok();
        let dynamodb_table = resolve_dynamodb_table(&ssm_map)?;
        let kms_key_id = resolve_kms_key_id(&ssm_map)?;
        let kms_endpoint = std::env::var("NITRUM_KMS_ENDPOINT_URL").ok();

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
            imds_latest_base_url,
            aws_sdk_config,
            aws_region,
            dynamodb_table,
            kms_key_id,
            dynamodb_endpoint,
            kms_endpoint,
            instance_id,
            ingress_listen_addr,
            acme_http01_listen_addr,
            crypto_api_listen_addr,
        })
    }
}

fn resolve_dynamodb_table(ssm: &HashMap<String, String>) -> Result<String> {
    let Some(v) = ssm.get("dynamodb_table") else {
        anyhow::bail!(
            "SSM missing dynamodb_table (expected parameter ending in /dynamodb_table, e.g. /nitrum/dynamodb_table)"
        );
    };
    if v.is_empty() {
        anyhow::bail!("SSM dynamodb_table parameter is empty");
    }
    Ok(v.clone())
}

fn resolve_kms_key_id(ssm: &HashMap<String, String>) -> Result<String> {
    let Some(v) = ssm.get("kms_key_id") else {
        anyhow::bail!(
            "SSM missing kms_key_id (expected parameter ending in /kms_key_id, e.g. /nitrum/kms_key_id)"
        );
    };
    if v.is_empty() {
        anyhow::bail!("SSM kms_key_id parameter is empty");
    }
    Ok(v.clone())
}

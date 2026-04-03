//! Data-plane runtime config: `nitrum.toml` plus IMDS + SSM infra settings.

use anyhow::{Context, Result};
use aws_config::BehaviorVersion;
use aws_config::Region;
use aws_credential_types::provider::SharedCredentialsProvider;
use config::NitrumConfig;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::ops::Deref;
use std::path::Path;
use std::sync::Arc;
use tracing::info;

use crate::constants::{
    DEFAULT_IMDS_LATEST_BASE_URL, app_env_parameter_name, data_plane_dynamodb_parameter_name,
    data_plane_kms_parameter_name,
};
use crate::utils::imds::{EnclaveProvider, ImdsClient};
use crate::utils::ssm::SsmParameters;

/// Data-plane runtime config: nitrum.toml plus infra settings.
#[derive(Clone)]
pub struct RuntimeConfig {
    /// User provided config.
    pub nitrum: NitrumConfig,
    /// IMDS base URL from `NITRUM_IMDS_BASE_URL` or [`DEFAULT_IMDS_LATEST_BASE_URL`] (no trailing slash).
    /// Default is standard EC2 IMDS; override for host-side runs or metadata mocks.
    #[allow(dead_code)]
    pub imds_latest_base_url: String,
    /// AWS region from IMDS.
    pub aws_region: String,
    /// AWS SDK config (credentials via IMDS through shared [`ImdsClient`]).
    pub aws_sdk_config: Arc<aws_config::SdkConfig>,
    /// Instance ID from IMDS.
    pub instance_id: String,
    /// `DynamoDB` table name from SSM.
    pub dynamodb_table: String,
    /// Optional `DynamoDB` API endpoint (`NITRUM_DYNAMODB_ENDPOINT_URL`).
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
    /// Application env vars loaded from SSM
    pub user_env: HashMap<String, String>,
}

impl Deref for RuntimeConfig {
    type Target = NitrumConfig;

    fn deref(&self) -> &Self::Target {
        &self.nitrum
    }
}

impl RuntimeConfig {
    /// Load infra: [`ImdsClient`] for region, instance id, and SDK credentials; [`SsmParameters`]
    /// for `kms_key_id` and `dynamodb_table` at fixed paths `/nitrum/{project.name}/data-plane/…`.
    ///
    /// There is no generic egress probe here: on Nitro, API traffic uses the TAP↔gvproxy path;
    /// probing arbitrary hosts such as `httpbin.org` fails in many setups and would block startup for no benefit.
    pub async fn load(config_path: &Path) -> Result<Self> {
        info!("loading runtime config from {}", config_path.display());
        let nitrum = NitrumConfig::try_from(config_path)?;

        info!("loading IMDS config");
        let imds_latest_base_url = std::env::var("NITRUM_IMDS_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_IMDS_LATEST_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string();
        let imds = Arc::new(ImdsClient::new(&imds_latest_base_url).context("IMDS client init")?);

        info!(
            imds_base_url = %imds_latest_base_url,
            "runtime config: fetching AWS region from IMDS (IMDSv2 token + placement/region)"
        );
        let aws_region = imds
            .get_region()
            .await
            .with_context(|| {
                format!(
                    "fetch AWS region from IMDS (GET meta-data/placement/region via {imds_latest_base_url}; set NITRUM_IMDS_BASE_URL if needed)"
                )
            })?;
        let instance_id = imds.instance_id().await.context("IMDS instance-id")?;
        let aws_sdk_config = Arc::new(
            aws_config::defaults(BehaviorVersion::latest())
                .region(Region::new(aws_region.clone()))
                .credentials_provider(SharedCredentialsProvider::new(EnclaveProvider::with_imds(
                    imds.clone(),
                )))
                .load()
                .await,
        );

        info!("loading SSM config");
        let ssm = SsmParameters::new(imds);

        info!("loading DynamoDB config");
        let dynamodb_endpoint = std::env::var("NITRUM_DYNAMODB_ENDPOINT_URL").ok();
        let dynamodb_path = data_plane_dynamodb_parameter_name(&nitrum.project.name);
        let dynamodb_table = ssm
            .get_parameter(&dynamodb_path)
            .await
            .with_context(|| {
                format!(
                    "SSM dynamodb_table (expected {dynamodb_path}, e.g. /nitrum/myapp/data-plane/dynamodb_table)"
                )
            })?;

        info!("loading KMS config");
        let kms_endpoint = std::env::var("NITRUM_KMS_ENDPOINT_URL").ok();
        let kms_path = data_plane_kms_parameter_name(&nitrum.project.name);
        let kms_key_id = ssm.get_parameter(&kms_path).await.with_context(|| {
            format!(
                "SSM kms_key_id (expected {kms_path}, e.g. /nitrum/myapp/data-plane/kms_key_id)"
            )
        })?;

        info!("loading app env from SSM");
        let app_env_path = app_env_parameter_name(&nitrum.project.name);
        let user_env = ssm
            .get_parameters_by_path_recursive(&app_env_path)
            .await
            .with_context(|| format!("SSM GetParametersByPath for app env ({app_env_path})"))?;

        info!("loading listen addresses from env");
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
            aws_region,
            aws_sdk_config,
            instance_id,
            dynamodb_table,
            dynamodb_endpoint,
            kms_key_id,
            kms_endpoint,
            ingress_listen_addr,
            acme_http01_listen_addr,
            crypto_api_listen_addr,
            user_env,
        })
    }
}

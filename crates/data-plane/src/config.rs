//! Data-plane config: `nitrum.toml` plus IMDS + SSM infra settings.

use crate::constants::{
    DEFAULT_ACME_HTTP01_LISTEN_ADDR, DEFAULT_CRYPTO_API_LISTEN_ADDR, DEFAULT_INGRESS_LISTEN_ADDR,
    ENV_ACME_HTTP01_LISTEN_ADDR, ENV_CRYPTO_API_LISTEN_ADDR, ENV_INGRESS_LISTEN_ADDR, OTLP_PORT,
    dynamodb_endpoint_url, dynamodb_ssm_table_name, env_variablesm_ssm_name, kms_endpoint_url,
    kms_ssm_key_name,
};
use crate::utils::env::var_or_nonempty_default;
use crate::utils::imds::{EnclaveProvider, ImdsClient};
use crate::utils::ssm::SsmParameters;
use anyhow::{Context, Result};
use aws_config::BehaviorVersion;
use aws_config::Region;
use aws_credential_types::provider::SharedCredentialsProvider;
use config::NitrumConfig;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::ops::Deref;
use std::sync::Arc;
use tracing::info;

/// Data-plane config: `nitrum.toml` plus infra settings resolved from IMDS and SSM.
#[derive(Clone)]
pub struct DataPlaneConfig {
    /// User-provided config from `nitrum.toml`, with environment overrides already applied.
    pub nitrum: NitrumConfig,
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
    /// Listen addresses for ingress, ACME HTTP-01, and the crypto API.
    pub listen_addrs: ListenAddrs,
    /// Application env vars loaded from SSM.
    pub user_env: HashMap<String, String>,
    /// Effective OTLP/gRPC collector endpoint for telemetry export.
    pub otlp_endpoint: Option<String>,
}
/// Listen addresses for ingress, ACME HTTP-01, and the crypto API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListenAddrs {
    /// Ingress TLS listen address.
    pub ingress_listen_addr: SocketAddr,
    /// ACME HTTP-01 listen address.
    pub acme_http01_listen_addr: SocketAddr,
    /// Crypto API listen address.
    pub crypto_api_listen_addr: SocketAddr,
}

impl Deref for DataPlaneConfig {
    type Target = NitrumConfig;

    fn deref(&self) -> &Self::Target {
        &self.nitrum
    }
}

impl DataPlaneConfig {
    /// Resolve infra for a loaded [`NitrumConfig`]: [`ImdsClient`] for the AWS region, instance id,
    /// SDK credentials, and OTLP endpoint; [`SsmParameters`] for `kms_key_id` and `dynamodb_table`
    /// at fixed paths `/nitrum/{project.name}/data-plane/…`.
    ///
    /// There is no generic egress probe here: on Nitro, API traffic uses the TAP↔gvproxy path;
    /// probing arbitrary hosts such as `httpbin.org` fails in many setups and would block startup for no benefit.
    ///
    /// # Errors
    ///
    /// Returns an error when IMDS, SSM, or listen-address resolution fails.
    pub async fn try_from(nitrum: NitrumConfig) -> Result<Self> {
        info!("Loading IMDS client");
        let imds =
            Arc::new(ImdsClient::new(&nitrum.imds_latest_base_url).context("IMDS client init")?);

        info!("Loading OTLP endpoint from env");
        let otlp_endpoint = match nitrum.otlp_endpoint.as_deref() {
            Some(explicit) => Some(explicit.to_string()),
            None => Some(
                imds.local_ipv4()
                    .await
                    .map(|parent_ipv4| format!("http://{parent_ipv4}:{OTLP_PORT}"))
                    .unwrap_or_else(|_| format!("http://127.0.0.1:{OTLP_PORT}")),
            ),
        };

        info!("Loading AWS region from IMDS");
        let aws_region = imds
            .get_region_with_retry()
            .await
            .with_context(|| {
                format!(
                    "fetch AWS region from IMDS (GET meta-data/placement/region via {}; set `imds_latest_base_url` in nitrum.toml if needed)",
                    nitrum.imds_latest_base_url
                )
            })?;
        let instance_id = imds.instance_id().await.context("IMDS instance-id")?;
        let aws_sdk_config = Arc::new(
            aws_config::defaults(BehaviorVersion::latest())
                .region(Region::new(aws_region.clone()))
                .credentials_provider(SharedCredentialsProvider::new(EnclaveProvider::with_imds(
                    imds,
                )))
                .load()
                .await,
        );

        info!("Loading SSM config");
        let ssm = SsmParameters::new(aws_sdk_config.clone());

        info!("Loading DynamoDB config");
        let dynamodb_endpoint = dynamodb_endpoint_url(); // TODO: make it a param
        let dynamodb_path = dynamodb_ssm_table_name(&nitrum.project.name);
        let dynamodb_table = ssm
            .get_parameter(&dynamodb_path)
            .await
            .with_context(|| {
                format!(
                    "SSM dynamodb_table (expected {dynamodb_path}, e.g. /nitrum/myapp/data-plane/dynamodb_table)"
                )
            })?;

        info!("Loading KMS config");
        let kms_endpoint = kms_endpoint_url();
        let kms_path = kms_ssm_key_name(&nitrum.project.name);
        let kms_key_id = ssm.get_parameter(&kms_path).await.with_context(|| {
            format!(
                "SSM kms_key_id (expected {kms_path}, e.g. /nitrum/myapp/data-plane/kms_key_id)"
            )
        })?;

        info!("Loading app env from SSM");
        let app_env_path = env_variablesm_ssm_name(&nitrum.project.name);
        let user_env = ssm
            .get_parameters_by_path_recursive(&app_env_path)
            .await
            .with_context(|| format!("SSM GetParametersByPath for app env ({app_env_path})"))?;

        info!("Loading listen addresses from env");
        let listen_addrs = ListenAddrs {
            ingress_listen_addr: var_or_nonempty_default(
                ENV_INGRESS_LISTEN_ADDR,
                DEFAULT_INGRESS_LISTEN_ADDR,
            )
            .parse()
            .with_context(|| format!("invalid {ENV_INGRESS_LISTEN_ADDR}"))?,
            acme_http01_listen_addr: var_or_nonempty_default(
                ENV_ACME_HTTP01_LISTEN_ADDR,
                DEFAULT_ACME_HTTP01_LISTEN_ADDR,
            )
            .parse()
            .with_context(|| format!("invalid {ENV_ACME_HTTP01_LISTEN_ADDR}"))?,
            crypto_api_listen_addr: var_or_nonempty_default(
                ENV_CRYPTO_API_LISTEN_ADDR,
                DEFAULT_CRYPTO_API_LISTEN_ADDR,
            )
            .parse()
            .with_context(|| format!("invalid {ENV_CRYPTO_API_LISTEN_ADDR}"))?,
        };

        Ok(Self {
            nitrum,
            aws_region,
            aws_sdk_config,
            instance_id,
            dynamodb_table,
            dynamodb_endpoint,
            kms_key_id,
            kms_endpoint,
            listen_addrs,
            user_env,
            otlp_endpoint,
        })
    }
}

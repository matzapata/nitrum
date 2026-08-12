//! Data-plane config: `nitrum.toml` plus IMDS + SSM infra settings.

use crate::bootstrap::imds::{
    DEFAULT_IMDS_LATEST_BASE_URL, ENV_IMDS_BASE_URL, EnclaveProvider, ImdsClient,
};
use crate::bootstrap::ssm::SsmParameters;
use crate::constants::{
    DEFAULT_ACME_HTTP01_LISTEN_ADDR, DEFAULT_CRYPTO_API_LISTEN_ADDR, DEFAULT_INGRESS_LISTEN_ADDR,
    ENV_ACME_HTTP01_LISTEN_ADDR, ENV_CRYPTO_API_LISTEN_ADDR, ENV_INGRESS_LISTEN_ADDR, OTLP_PORT,
    otlp_endpoint_from_env,
};
use crate::utils::env::var_or_nonempty_default;
use anyhow::{Context, Result};
use aws_config::BehaviorVersion;
use aws_config::Region;
use aws_credential_types::provider::SharedCredentialsProvider;
use config::{NitrumConfig, PlatformLayout};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::ops::Deref;
use std::sync::Arc;
use tracing::info;

/// Data-plane config: `nitrum.toml` plus infra settings resolved from IMDS and SSM.
#[derive(Clone)]
pub struct DataPlaneConfig {
    /// User-provided config from `nitrum.toml`.
    pub nitrum: NitrumConfig,
    /// AWS/SSM naming layout for this deployment.
    pub layout: PlatformLayout,
    /// AWS SDK config (credentials via IMDS through shared [`ImdsClient`]).
    pub aws: Arc<aws_config::SdkConfig>,
    /// IMDS base URL used for region, instance id, credentials, and parent IPv4 discovery.
    pub imds_base_url: String,
    /// Instance ID from IMDS.
    pub instance_id: String,
    /// KMS key ID from SSM.
    pub kms_key_id: String,
    /// `DynamoDB` table name from SSM.
    pub dynamodb_table: String,
    /// Effective OTLP/gRPC collector endpoint for telemetry export.
    pub otlp_endpoint: Option<String>,
    /// Listen addresses for ingress, ACME HTTP-01, and the crypto API.
    pub listen_addrs: ListenAddrs,
    /// Application env vars loaded from SSM.
    pub user_env: HashMap<String, String>,
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
    /// at [`PlatformLayout`] paths under `/nitrum/{project.name}/data-plane/…`.
    ///
    /// There is no generic egress probe here: on Nitro, API traffic uses the TAP↔gvproxy path;
    /// probing arbitrary hosts such as `httpbin.org` fails in many setups and would block startup for no benefit.
    ///
    /// # Errors
    ///
    /// Returns an error when IMDS, SSM, or listen-address resolution fails.
    pub async fn try_from(nitrum: NitrumConfig) -> Result<Self> {
        let layout = PlatformLayout::from_project(&nitrum.project);

        info!("Loading IMDS client");
        let imds = Arc::new(
            ImdsClient::from_env(ENV_IMDS_BASE_URL, DEFAULT_IMDS_LATEST_BASE_URL)
                .context("IMDS client init")?,
        );
        let imds_base_url = imds.latest_base_url().to_string();

        info!("Loading OTLP endpoint from env");
        let otlp_endpoint = match otlp_endpoint_from_env() {
            Some(explicit) => Some(explicit),
            None => Some(imds.local_ipv4().await.map_or_else(
                |_| format!("http://127.0.0.1:{OTLP_PORT}"),
                |parent_ipv4| format!("http://{parent_ipv4}:{OTLP_PORT}"),
            )),
        };

        info!("Loading AWS region from IMDS");
        let aws_region = imds
            .get_region_with_retry()
            .await
            .with_context(|| {
                format!(
                    "fetch AWS region from IMDS (GET meta-data/placement/region via {imds_base_url}; set {ENV_IMDS_BASE_URL} for local dev if needed)"
                )
            })?;
        let instance_id = imds.instance_id().await.context("IMDS instance-id")?;
        let aws = Arc::new(
            aws_config::defaults(BehaviorVersion::latest())
                .region(Region::new(aws_region.clone()))
                .credentials_provider(SharedCredentialsProvider::new(EnclaveProvider::with_imds(
                    imds,
                )))
                .load()
                .await,
        );

        info!("Loading SSM config");
        let ssm = SsmParameters::new(aws.clone());

        info!("Loading DynamoDB config");
        let dynamodb_path = layout.dynamodb_table_param();
        let dynamodb_table = ssm
            .get_parameter(&dynamodb_path)
            .await
            .with_context(|| {
                format!(
                    "SSM dynamodb_table (expected {dynamodb_path}, e.g. /nitrum/myapp/data-plane/dynamodb_table)"
                )
            })?;

        info!("Loading KMS config");
        let kms_path = layout.kms_key_id_param();
        let kms_key_id = ssm.get_parameter(&kms_path).await.with_context(|| {
            format!(
                "SSM kms_key_id (expected {kms_path}, e.g. /nitrum/myapp/data-plane/kms_key_id)"
            )
        })?;

        info!("Loading app env from SSM");
        let app_env_path = layout.app_env_path_prefix();
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
            layout,
            aws,
            imds_base_url,
            instance_id,
            kms_key_id,
            dynamodb_table,
            otlp_endpoint,
            listen_addrs,
            user_env,
        })
    }
}

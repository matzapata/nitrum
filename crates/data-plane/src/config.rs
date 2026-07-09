//! Data-plane config: `nitrum.toml` plus IMDS + SSM infra settings.

use anyhow::{Context, Result};
use aws_config::BehaviorVersion;
use aws_config::Region;
use aws_credential_types::provider::SharedCredentialsProvider;
use config::NitrumConfig;
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

#[cfg(feature = "enclave")] // TODO: remove
use crate::constants::default_otlp_endpoint_for_host;
use crate::constants::{
    AWS_REGION_FETCH_ATTEMPTS, AWS_REGION_FETCH_INITIAL_BACKOFF_MS, ListenAddrs,
    app_env_parameter_name, data_plane_dynamodb_parameter_name, data_plane_kms_parameter_name,
    dynamodb_endpoint_url, kms_endpoint_url,
};
use crate::utils::imds::{EnclaveProvider, ImdsClient};
use crate::utils::ssm::SsmParameters;

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
        info!(
            imds_base_url = %nitrum.imds_latest_base_url,
            "connecting to IMDS"
        );
        let imds =
            Arc::new(ImdsClient::new(&nitrum.imds_latest_base_url).context("IMDS client init")?);

        // TODO: can we move this to the TelemetryConfig?
        info!("resolving OTLP telemetry endpoint");
        let otlp_endpoint = resolve_otlp_endpoint(&nitrum, &imds).await;

        info!("fetching AWS region from IMDS (IMDSv2 token + placement/region)");
        let aws_region = imds
            .get_region_with_retry(
                AWS_REGION_FETCH_ATTEMPTS,
                Duration::from_millis(AWS_REGION_FETCH_INITIAL_BACKOFF_MS),
            )
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

        info!("loading SSM config");
        let ssm = SsmParameters::new(aws_sdk_config.clone());

        info!("loading DynamoDB config");
        let dynamodb_endpoint = dynamodb_endpoint_url(); // TODO: make it a param
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
        let kms_endpoint = kms_endpoint_url();
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
        let listen_addrs = ListenAddrs::from_env()?;

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

/// Resolve the effective OTLP/gRPC collector endpoint for telemetry startup.
///
/// [`NitrumConfig::otlp_endpoint`] always wins when set: an empty value disables OTLP and keeps
/// stdout-only logging. Otherwise, in an enclave the parent host's default collector endpoint is
/// derived from the parent instance's `local-ipv4` via the shared `imds` client; if that lookup
/// fails telemetry degrades to stdout-only logging. Off-enclave there is no platform default, so
/// `None` is returned when the config value is unset.
async fn resolve_otlp_endpoint(nitrum: &NitrumConfig, imds: &ImdsClient) -> Option<String> {
    if let Some(explicit) = nitrum.otlp_endpoint.as_deref() {
        return (!explicit.is_empty()).then(|| explicit.to_string());
    }

    #[cfg(feature = "enclave")]
    {
        match imds.local_ipv4().await {
            Ok(parent_ipv4) => Some(default_otlp_endpoint_for_host(&parent_ipv4)),
            Err(error) => {
                eprintln!(
                    "telemetry: failed to resolve default OTLP endpoint from IMDS ({error:#}); falling back to stdout-only logging"
                );
                None
            }
        }
    }

    #[cfg(not(feature = "enclave"))]
    {
        let _ = imds;
        None
    }
}

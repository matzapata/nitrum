//! Data-plane runtime config: `nitrum.toml` plus IMDS + SSM infra settings.

use anyhow::{Context, Result};
use aws_config::BehaviorVersion;
use aws_config::Region;
use aws_credential_types::provider::SharedCredentialsProvider;
use config::NitrumConfig;
use std::collections::HashMap;
use std::ops::Deref;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

use crate::constants::{
    AWS_REGION_FETCH_ATTEMPTS, AWS_REGION_FETCH_INITIAL_BACKOFF_MS, ListenAddrs,
    app_env_parameter_name, data_plane_dynamodb_parameter_name, data_plane_kms_parameter_name,
    dynamodb_endpoint_url, explicit_otlp_endpoint, imds_latest_base_url, kms_endpoint_url,
    otlp_endpoint_from_override,
};
use crate::utils::imds::{EnclaveProvider, ImdsClient};
use crate::utils::ssm::SsmParameters;

/// Early infra handles created before telemetry init so the rest of config load is observable.
pub struct ConfigBootstrap {
    /// User provided config.
    pub nitrum: NitrumConfig,
    /// IMDS base URL from [`crate::constants::imds_latest_base_url`].
    pub imds_latest_base_url: String,
    imds: Arc<ImdsClient>,
    otlp_endpoint: Option<String>,
}

impl ConfigBootstrap {
    /// Read `nitrum.toml`, connect to IMDS, and resolve the OTLP endpoint.
    ///
    /// # Errors
    ///
    /// Returns an error when the config file cannot be read or the IMDS client cannot be built.
    pub async fn bootstrap(config_path: &Path) -> Result<Self> {
        info!("bootstrapping from {}", config_path.display());
        let nitrum = NitrumConfig::try_from(config_path)?;

        info!("connecting to IMDS");
        let imds_latest_base_url = imds_latest_base_url();
        let imds = Arc::new(ImdsClient::new(&imds_latest_base_url).context("IMDS client init")?);

        info!("resolving OTLP telemetry endpoint");
        let otlp_endpoint = resolve_otlp_endpoint(&imds).await;

        Ok(Self {
            nitrum,
            imds_latest_base_url,
            imds,
            otlp_endpoint,
        })
    }

    /// Resolved OTLP/gRPC collector endpoint for telemetry export.
    #[must_use]
    pub fn otlp_endpoint(&self) -> Option<&str> {
        self.otlp_endpoint.as_deref()
    }

    /// Finish loading infra settings using the shared IMDS session.
    ///
    /// # Errors
    ///
    /// Returns an error when IMDS, SSM, or listen-address resolution fails.
    pub async fn into_runtime_config(self) -> Result<RuntimeConfig> {
        let Self {
            nitrum,
            imds_latest_base_url,
            imds,
            otlp_endpoint,
        } = self;

        info!(
            imds_base_url = %imds_latest_base_url,
            "runtime config: fetching AWS region from IMDS (IMDSv2 token + placement/region)"
        );
        let aws_region = imds
            .get_region_with_retry(
                AWS_REGION_FETCH_ATTEMPTS,
                Duration::from_millis(AWS_REGION_FETCH_INITIAL_BACKOFF_MS),
            )
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
                    imds,
                )))
                .load()
                .await,
        );

        info!("loading SSM config");
        let ssm = SsmParameters::new(aws_sdk_config.clone());

        info!("loading DynamoDB config");
        let dynamodb_endpoint = dynamodb_endpoint_url();
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

        Ok(RuntimeConfig {
            nitrum,
            imds_latest_base_url,
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

/// Data-plane runtime config: nitrum.toml plus infra settings.
#[derive(Clone)]
pub struct RuntimeConfig {
    /// User provided config.
    pub nitrum: NitrumConfig,
    /// IMDS base URL from `NITRUM_IMDS_BASE_URL` or [`crate::constants::DEFAULT_IMDS_LATEST_BASE_URL`] (no trailing slash).
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
    /// Listen addresses for ingress, ACME HTTP-01, and the crypto API.
    pub listen_addrs: ListenAddrs,
    /// Application env vars loaded from SSM
    pub user_env: HashMap<String, String>,
    /// Resolved OTLP/gRPC collector endpoint for telemetry export.
    ///
    /// `None` keeps telemetry in stdout-only mode (see [`resolve_otlp_endpoint`]).
    pub otlp_endpoint: Option<String>,
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
        ConfigBootstrap::bootstrap(config_path)
            .await?
            .into_runtime_config()
            .await
    }
}

/// Resolve the effective OTLP/gRPC collector endpoint for telemetry startup.
///
/// [`crate::constants::ENV_OTLP_ENDPOINT`] always wins when set (an empty value disables OTLP and
/// keeps stdout-only logging). Otherwise, in an enclave the parent host's default collector endpoint
/// is derived from the parent instance's `local-ipv4` via the shared `imds` client; if that lookup
/// fails telemetry degrades to stdout-only logging. Off-enclave there is no platform default, so
/// `None` is returned when the variable is unset.
async fn resolve_otlp_endpoint(imds: &ImdsClient) -> Option<String> {
    let explicit = explicit_otlp_endpoint();
    if explicit.is_some() {
        return otlp_endpoint_from_override(explicit, None);
    }

    #[cfg(feature = "enclave")]
    {
        let platform_host = match imds.local_ipv4().await {
            Ok(parent_ipv4) => Some(parent_ipv4),
            Err(error) => {
                eprintln!(
                    "telemetry: failed to resolve default OTLP endpoint from IMDS ({error:#}); falling back to stdout-only logging"
                );
                None
            }
        };
        otlp_endpoint_from_override(None, platform_host.as_deref())
    }

    #[cfg(not(feature = "enclave"))]
    {
        let _ = imds;
        otlp_endpoint_from_override(None, None)
    }
}

#[cfg(test)]
mod tests {
    use crate::constants::{
        ExplicitOtlpEndpoint, default_otlp_endpoint_for_host, otlp_endpoint_from_override,
    };

    #[test]
    fn otlp_endpoint_from_override_matches_explicit_disabled() {
        assert_eq!(
            otlp_endpoint_from_override(Some(ExplicitOtlpEndpoint::Disabled), Some("10.0.0.2")),
            None
        );
    }

    #[test]
    fn otlp_endpoint_from_override_builds_platform_default() {
        assert_eq!(
            otlp_endpoint_from_override(None, Some("10.0.0.2")),
            Some(default_otlp_endpoint_for_host("10.0.0.2"))
        );
    }
}

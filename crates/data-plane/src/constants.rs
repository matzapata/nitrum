use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::utils::env::{optional_nonempty, var_or_nonempty_default};

/// VSOCK port where gvproxy listens on the host (CID 3).
pub const HOST_PROXY_PORT: u32 = 1024;

/// Delay between polls while waiting for the ACME leader lock (another instance may be provisioning).
pub const ACME_LOCK_RETRY_INTERVAL: Duration = Duration::from_secs(2);

/// Default certificate renewal fraction (2/3 of the certificate lifetime).
pub const CERTIFICATE_RENEWAL_FRACTION: f64 = 2.0 / 3.0;

/// Default Let's Encrypt production directory.
pub const DEFAULT_ACME_DIRECTORY_URL: &str = "https://acme-v02.api.letsencrypt.org/directory";

/// Environment variable for the ACME directory URL.
pub const ENV_ACME_DIRECTORY_URL: &str = "NITRUM_ACME_DIRECTORY_URL";

/// Environment variable holding the OTLP/gRPC telemetry collector endpoint.
///
/// When set (e.g. `http://observability:4317` under local Compose) it overrides
/// the platform default; an empty value disables OTLP and keeps stdout-only
/// logging.
pub const ENV_OTLP_ENDPOINT: &str = "NITRUM_OTLP_ENDPOINT";

/// Default OTLP/gRPC port exposed by the parent host's ADOT collector.
pub const DEFAULT_OTLP_PORT: u16 = 4317;

/// Environment variable overriding the IMDS base URL (includes `/latest`).
pub const ENV_IMDS_BASE_URL: &str = "NITRUM_IMDS_BASE_URL";

/// Default IMDS base URL when [`ENV_IMDS_BASE_URL`] is unset (includes `/latest`, no trailing slash).
///
/// **Nitro enclave:** with gvproxy started using `-ec2-metadata-access`, `IMDSv2` is reached over the
/// TAP path at the standard link-local address (same as on the parent). ACME HTTP-01 still uses
/// `0.0.0.0:80` on the data-plane; IMDS is HTTP to port 80 on `169.254.169.254`, not the ACME listener.
///
/// **Local dev:** set [`ENV_IMDS_BASE_URL`] (e.g. `http://imds:1338/latest` for docker-compose metadata mock).
pub const DEFAULT_IMDS_LATEST_BASE_URL: &str = "http://169.254.169.254/latest";

/// Environment variable overriding the DynamoDB API endpoint.
pub const ENV_DYNAMODB_ENDPOINT_URL: &str = "NITRUM_DYNAMODB_ENDPOINT_URL";

/// Environment variable overriding the KMS API endpoint.
pub const ENV_KMS_ENDPOINT_URL: &str = "NITRUM_KMS_ENDPOINT_URL";

/// Environment variable overriding the SSM API endpoint.
pub const ENV_SSM_ENDPOINT_URL: &str = "NITRUM_SSM_ENDPOINT_URL";

/// Environment variable for the ingress TLS listen address.
pub const ENV_INGRESS_LISTEN_ADDR: &str = "NITRUM_INGRESS_LISTEN_ADDR";

/// Default ingress TLS listen address.
pub const DEFAULT_INGRESS_LISTEN_ADDR: &str = "0.0.0.0:443";

/// Environment variable for the ACME HTTP-01 listen address.
pub const ENV_ACME_HTTP01_LISTEN_ADDR: &str = "NITRUM_ACME_HTTP01_LISTEN_ADDR";

/// Default ACME HTTP-01 listen address.
pub const DEFAULT_ACME_HTTP01_LISTEN_ADDR: &str = "0.0.0.0:80";

/// Environment variable for the crypto API listen address.
pub const ENV_CRYPTO_API_LISTEN_ADDR: &str = "NITRUM_CRYPTO_API_LISTEN_ADDR";

/// Default crypto API listen address.
pub const DEFAULT_CRYPTO_API_LISTEN_ADDR: &str = "0.0.0.0:3000";

/// IMDS region fetch attempts when no region is supplied at startup.
pub const AWS_REGION_FETCH_ATTEMPTS: u32 = 5;

/// Initial backoff between IMDS region fetch retries.
pub const AWS_REGION_FETCH_INITIAL_BACKOFF_MS: u64 = 200;

/// SSM path for KMS key ID under `/nitrum/{project name}/data-plane/kms_key_id`.
#[must_use]
pub fn data_plane_kms_parameter_name(project_name: &str) -> String {
    format!("/nitrum/{project_name}/data-plane/kms_key_id")
}

/// SSM path for `DynamoDB` table name under `/nitrum/{project name}/data-plane/dynamodb_table`.
#[must_use]
pub fn data_plane_dynamodb_parameter_name(project_name: &str) -> String {
    format!("/nitrum/{project_name}/data-plane/dynamodb_table")
}

/// SSM path for app env under `/nitrum/{project name}/env/`.
#[must_use]
pub fn app_env_parameter_name(project_name: &str) -> String {
    format!("/nitrum/{project_name}/env/")
}

/// Build the default OTLP/gRPC collector endpoint for a parent host address.
///
/// In Nitro enclave deployments, `host` is the parent instance private IPv4
/// address from IMDS (`meta-data/local-ipv4`).
#[must_use]
pub fn default_otlp_endpoint_for_host(host: &str) -> String {
    format!("http://{host}:{DEFAULT_OTLP_PORT}")
}

/// Effective IMDS base URL: [`ENV_IMDS_BASE_URL`] when set, otherwise
/// [`DEFAULT_IMDS_LATEST_BASE_URL`] (no trailing slash).
#[must_use]
pub fn imds_latest_base_url() -> String {
    var_or_nonempty_default(ENV_IMDS_BASE_URL, DEFAULT_IMDS_LATEST_BASE_URL)
        .trim_end_matches('/')
        .to_string()
}

/// Effective ACME directory URL: non-empty [`ENV_ACME_DIRECTORY_URL`] when set, otherwise
/// [`DEFAULT_ACME_DIRECTORY_URL`] (Let's Encrypt production).
#[must_use]
pub fn acme_directory_url() -> String {
    var_or_nonempty_default(ENV_ACME_DIRECTORY_URL, DEFAULT_ACME_DIRECTORY_URL)
}

/// Non-empty ACME directory URL override from [`ENV_ACME_DIRECTORY_URL`], if any.
#[must_use]
pub fn acme_directory_url_override() -> Option<String> {
    optional_nonempty(ENV_ACME_DIRECTORY_URL)
}

/// Optional DynamoDB API endpoint override.
#[must_use]
pub fn dynamodb_endpoint_url() -> Option<String> {
    optional_nonempty(ENV_DYNAMODB_ENDPOINT_URL)
}

/// Optional KMS API endpoint override.
#[must_use]
pub fn kms_endpoint_url() -> Option<String> {
    optional_nonempty(ENV_KMS_ENDPOINT_URL)
}

/// Optional SSM API endpoint override.
#[must_use]
pub fn ssm_endpoint_url() -> Option<String> {
    optional_nonempty(ENV_SSM_ENDPOINT_URL)
}

/// Whether regional AWS hostname patterns are required for egress (no custom endpoint override).
#[must_use]
pub fn needs_regional_aws_service_patterns() -> bool {
    dynamodb_endpoint_url().is_none()
        || kms_endpoint_url().is_none()
        || ssm_endpoint_url().is_none()
}

/// Explicit OTLP endpoint override from [`ENV_OTLP_ENDPOINT`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExplicitOtlpEndpoint {
    /// OTLP export disabled (`NITRUM_OTLP_ENDPOINT=""`).
    Disabled,
    /// User-provided collector endpoint.
    Endpoint(String),
}

/// Read [`ENV_OTLP_ENDPOINT`] when set.
///
/// Returns `None` when the variable is unset so callers can apply platform defaults.
#[must_use]
pub fn explicit_otlp_endpoint() -> Option<ExplicitOtlpEndpoint> {
    match std::env::var(ENV_OTLP_ENDPOINT) {
        Err(_) => None,
        Ok(endpoint) if endpoint.is_empty() => Some(ExplicitOtlpEndpoint::Disabled),
        Ok(endpoint) => Some(ExplicitOtlpEndpoint::Endpoint(endpoint)),
    }
}

/// Resolve the OTLP endpoint from an explicit override and an optional platform default host.
#[must_use]
pub fn otlp_endpoint_from_override(
    explicit: Option<ExplicitOtlpEndpoint>,
    platform_host: Option<&str>,
) -> Option<String> {
    match explicit {
        Some(ExplicitOtlpEndpoint::Disabled) => None,
        Some(ExplicitOtlpEndpoint::Endpoint(endpoint)) => Some(endpoint),
        None => platform_host.map(default_otlp_endpoint_for_host),
    }
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

impl ListenAddrs {
    /// Resolve listen addresses from environment variables with crate defaults.
    ///
    /// # Errors
    ///
    /// Returns an error when any override value is not a valid `SocketAddr`.
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            ingress_listen_addr: parse_listen_addr(
                ENV_INGRESS_LISTEN_ADDR,
                DEFAULT_INGRESS_LISTEN_ADDR,
            )?,
            acme_http01_listen_addr: parse_listen_addr(
                ENV_ACME_HTTP01_LISTEN_ADDR,
                DEFAULT_ACME_HTTP01_LISTEN_ADDR,
            )?,
            crypto_api_listen_addr: parse_listen_addr(
                ENV_CRYPTO_API_LISTEN_ADDR,
                DEFAULT_CRYPTO_API_LISTEN_ADDR,
            )?,
        })
    }
}

fn parse_listen_addr(var: &str, default: &str) -> Result<SocketAddr> {
    var_or_nonempty_default(var, default)
        .parse()
        .with_context(|| format!("invalid {var}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_otlp_endpoint_env_semantics() {
        let key = ENV_OTLP_ENDPOINT;
        unsafe { std::env::remove_var(key) };
        assert_eq!(explicit_otlp_endpoint(), None);

        unsafe { std::env::set_var(key, "") };
        assert_eq!(
            explicit_otlp_endpoint(),
            Some(ExplicitOtlpEndpoint::Disabled)
        );

        unsafe { std::env::set_var(key, "http://collector:4317") };
        assert_eq!(
            explicit_otlp_endpoint(),
            Some(ExplicitOtlpEndpoint::Endpoint(
                "http://collector:4317".to_string()
            ))
        );

        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn otlp_endpoint_from_override_prefers_explicit() {
        assert_eq!(
            otlp_endpoint_from_override(
                Some(ExplicitOtlpEndpoint::Endpoint(
                    "http://override:4317".to_string()
                )),
                Some("10.0.0.1")
            ),
            Some("http://override:4317".to_string())
        );
    }

    #[test]
    fn otlp_endpoint_from_override_uses_platform_host() {
        assert_eq!(
            otlp_endpoint_from_override(None, Some("10.0.0.1")),
            Some("http://10.0.0.1:4317".to_string())
        );
    }

    #[test]
    fn otlp_endpoint_from_override_none_without_platform_host() {
        assert_eq!(otlp_endpoint_from_override(None, None), None);
    }

    #[test]
    fn needs_regional_patterns_when_endpoints_unset() {
        let kms = ENV_KMS_ENDPOINT_URL;
        let ssm = ENV_SSM_ENDPOINT_URL;
        let dynamodb = ENV_DYNAMODB_ENDPOINT_URL;
        unsafe {
            std::env::remove_var(kms);
            std::env::remove_var(ssm);
            std::env::remove_var(dynamodb);
        }
        assert!(needs_regional_aws_service_patterns());

        unsafe { std::env::set_var(kms, "") };
        assert!(needs_regional_aws_service_patterns());
        unsafe { std::env::remove_var(kms) };

        unsafe { std::env::set_var(kms, "http://localstack:4566") };
        unsafe { std::env::set_var(ssm, "http://localstack:4566") };
        unsafe { std::env::set_var(dynamodb, "http://localstack:4566") };
        assert!(!needs_regional_aws_service_patterns());
        unsafe {
            std::env::remove_var(kms);
            std::env::remove_var(ssm);
            std::env::remove_var(dynamodb);
        }
    }
}

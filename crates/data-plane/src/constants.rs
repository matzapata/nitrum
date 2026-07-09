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

/// Default OTLP/gRPC port exposed by the parent host's ADOT collector.
pub const DEFAULT_OTLP_PORT: u16 = 4317;

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

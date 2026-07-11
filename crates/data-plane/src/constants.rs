//! Shared runtime configuration: env var names, defaults, and AWS endpoint overrides.

use crate::utils::env::optional_nonempty;

/// OTLP/gRPC port exposed by the parent host's ADOT collector.
pub const OTLP_PORT: u16 = 4317;

/// Environment variable overriding the OTLP collector endpoint.
pub const ENV_OTLP_ENDPOINT: &str = "NITRUM_OTLP_ENDPOINT";

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

/// Returns [`ENV_OTLP_ENDPOINT`] when set (including empty string to disable export).
#[must_use]
pub fn otlp_endpoint_from_env() -> Option<String> {
    std::env::var(ENV_OTLP_ENDPOINT).ok()
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

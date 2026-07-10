use crate::utils::env::{optional_nonempty, var_or_nonempty_default};
use std::time::Duration;

/// VSOCK port where gvproxy listens on the host (CID 3).
pub const HOST_PROXY_PORT: u32 = 1024;

/// Delay between VSOCK connection attempts to gvproxy on the host.
pub const VSOCK_CONNECT_RETRY_INTERVAL: Duration = Duration::from_secs(1);

/// Maximum VSOCK connection attempts before enclave networking init fails.
pub const VSOCK_CONNECT_MAX_ATTEMPTS: u32 = 30;

/// OTLP/gRPC port exposed by the parent host's ADOT collector.
pub const OTLP_PORT: u16 = 4317;

/// Delay between polls while waiting for the ACME leader lock (another instance may be provisioning).
pub const ACME_LOCK_RETRY_INTERVAL: Duration = Duration::from_secs(2);

/// Default certificate renewal fraction (2/3 of the certificate lifetime).
pub const CERTIFICATE_RENEWAL_FRACTION: f64 = 2.0 / 3.0;

/// Default Let's Encrypt production directory.
pub const DEFAULT_ACME_DIRECTORY_URL: &str = "https://acme-v02.api.letsencrypt.org/directory";

/// Environment variable for the ACME directory URL.
pub const ENV_ACME_DIRECTORY_URL: &str = "NITRUM_ACME_DIRECTORY_URL";

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

/// SSM path for KMS key ID under `/nitrum/{project name}/data-plane/kms_key_id`.
#[must_use]
pub fn kms_ssm_key_name(project_name: &str) -> String {
    format!("/nitrum/{project_name}/data-plane/kms_key_id")
}

/// SSM path for `DynamoDB` table name under `/nitrum/{project name}/data-plane/dynamodb_table`.
#[must_use]
pub fn dynamodb_ssm_table_name(project_name: &str) -> String {
    format!("/nitrum/{project_name}/data-plane/dynamodb_table")
}

/// SSM path for app env under `/nitrum/{project name}/env/`.
#[must_use]
pub fn env_variablesm_ssm_name(project_name: &str) -> String {
    format!("/nitrum/{project_name}/env/")
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

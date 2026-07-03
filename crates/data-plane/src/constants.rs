use std::time::Duration;

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

/// Default IMDS base URL when `NITRUM_IMDS_BASE_URL` is unset (includes `/latest`, no trailing slash).
///
/// **Nitro enclave:** with gvproxy started using `-ec2-metadata-access`, `IMDSv2` is reached over the
/// TAP path at the standard link-local address (same as on the parent). ACME HTTP-01 still uses
/// `0.0.0.0:80` on the data-plane; IMDS is HTTP to port 80 on `169.254.169.254`, not the ACME listener.
///
/// **Local dev:** set `NITRUM_IMDS_BASE_URL` (e.g. `http://imds:1338/latest` for docker-compose metadata mock).
pub const DEFAULT_IMDS_LATEST_BASE_URL: &str = "http://169.254.169.254/latest";

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

/// Effective OTLP/gRPC collector endpoint for both telemetry export and the
/// egress allowance that keeps that export from being dropped.
///
/// Returns [`ENV_OTLP_ENDPOINT`] when set, otherwise `default_endpoint`.
/// `None` when the variable is set to an empty value, or when no explicit value
/// and no platform default are available.
#[must_use]
pub fn otlp_endpoint(default_endpoint: Option<&str>) -> Option<String> {
    let endpoint = std::env::var(ENV_OTLP_ENDPOINT)
        .unwrap_or_else(|_| default_endpoint.unwrap_or_default().to_string());
    (!endpoint.is_empty()).then_some(endpoint)
}

/// Effective ACME directory URL: [`ENV_ACME_DIRECTORY_URL`] when set, otherwise
/// [`DEFAULT_ACME_DIRECTORY_URL`] (Let's Encrypt production).
#[must_use]
pub fn acme_directory_url() -> String {
    std::env::var(ENV_ACME_DIRECTORY_URL).unwrap_or_else(|_| DEFAULT_ACME_DIRECTORY_URL.to_string())
}

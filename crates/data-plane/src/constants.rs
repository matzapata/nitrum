/// VSOCK port where gvproxy listens on the host (CID 3).
pub const HOST_PROXY_PORT: u32 = 1024;

/// Default certificate renewal fraction (2/3 of the certificate lifetime).
pub const CERTIFICATE_RENEWAL_FRACTION: f64 = 2.0 / 3.0;

/// Default Let's Encrypt production directory.
pub const LETS_ENCRYPT_PROD_DIRECTORY: &str = "https://acme-v02.api.letsencrypt.org/directory";

/// Environment variable for the ACME directory URL.
pub const ENV_ACME_DIRECTORY_URL: &str = "NITRUM_ACME_DIRECTORY_URL";

/// Default IMDS base URL when `NITRUM_IMDS_BASE_URL` is unset (includes `/latest`, no trailing slash).
///
/// **Nitro enclave:** with gvproxy started using `-ec2-metadata-access`, `IMDSv2` is reached over the
/// TAP path at the standard link-local address (same as on the parent). ACME HTTP-01 still uses
/// `0.0.0.0:80` on the data-plane; IMDS is HTTP to port 80 on `169.254.169.254`, not the ACME listener.
///
/// **Local dev:** set `NITRUM_IMDS_BASE_URL` (e.g. `http://imds:1338/latest` for docker-compose metadata mock).
pub const DEFAULT_IMDS_LATEST_BASE_URL: &str = "http://169.254.169.254/latest";

/// SSM path for KMS key ID under `/nitrum/{project name}/data-plane/kms_key_id`.
pub fn data_plane_kms_parameter_name(project_name: &str) -> String {
    format!("/nitrum/{project_name}/data-plane/kms_key_id")
}

/// SSM path for `DynamoDB` table name under `/nitrum/{project name}/data-plane/dynamodb_table`.
pub fn data_plane_dynamodb_parameter_name(project_name: &str) -> String {
    format!("/nitrum/{project_name}/data-plane/dynamodb_table")
}

/// SSM path for app env under `/nitrum/{project name}/env/`.
pub fn app_env_parameter_name(project_name: &str) -> String {
    format!("/nitrum/{project_name}/env/")
}

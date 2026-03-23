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
/// **Nitro enclave:** the data-plane must not use the link-local address inside the enclave. Traffic
/// goes to the in-enclave viproxy on loopback (started in `networking::init`, default port 8099)
/// that forwards over vsock to the parent (`enclave-imds-proxy`: port 8002 → `169.254.169.254:80`).
/// Port 8099 avoids colliding with ACME HTTP-01 on `0.0.0.0:80`.
///
/// **Host or local dev:** set `NITRUM_IMDS_BASE_URL` explicitly (e.g. `http://169.254.169.254/latest`
/// on the parent EC2, or `http://imds:1338/latest` for docker-compose metadata mock).
pub const DEFAULT_IMDS_LATEST_BASE_URL: &str = "http://127.0.0.1:8099/latest";

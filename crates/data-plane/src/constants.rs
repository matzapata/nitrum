/// VSOCK port where gvproxy listens on the host (CID 3).
pub const HOST_PROXY_PORT: u32 = 1024;

/// Default address for the data-plane API server (attestation, encrypt, decrypt).
pub const API_LISTEN_ADDR: &str = "0.0.0.0:3000";

/// Ingress proxy: listens here and forwards to the user app.
pub const INGRESS_LISTEN_ADDR: &str = "0.0.0.0:443";

// TODO: maybe use a env var for this?
/// Default bind address for ACME HTTP-01 challenge (overridable via `NITRUM_ACME_HTTP01_LISTEN_ADDR`).
pub const INGRESS_ACME_HTTP01_LISTEN_ADDR: &str = "0.0.0.0:5002";

/// Default certificate renewal fraction (2/3 of the certificate lifetime).
pub const CERTIFICATE_RENEWAL_FRACTION: f64 = 2.0 / 3.0;

/// Default Let's Encrypt staging directory.
pub const LETS_ENCRYPT_STAGING_DIRECTORY: &str =
    "https://acme-staging-v02.api.letsencrypt.org/directory";

/// Default Let's Encrypt production directory.
pub const LETS_ENCRYPT_PROD_DIRECTORY: &str = "https://acme-v02.api.letsencrypt.org/directory";

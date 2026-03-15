/// VSOCK port where gvproxy listens on the host (CID 3).
pub const HOST_PROXY_PORT: u32 = 1024;

/// Default certificate renewal fraction (2/3 of the certificate lifetime).
pub const CERTIFICATE_RENEWAL_FRACTION: f64 = 2.0 / 3.0;

/// Default Let's Encrypt staging directory.
pub const LETS_ENCRYPT_STAGING_DIRECTORY: &str =
    "https://acme-staging-v02.api.letsencrypt.org/directory";

/// Default Let's Encrypt production directory.
pub const LETS_ENCRYPT_PROD_DIRECTORY: &str = "https://acme-v02.api.letsencrypt.org/directory";

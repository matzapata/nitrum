/// VSOCK port where gvproxy listens on the host (CID 3).
pub const HOST_PROXY_PORT: u32 = 1024;

/// Default certificate renewal fraction (2/3 of the certificate lifetime).
pub const CERTIFICATE_RENEWAL_FRACTION: f64 = 2.0 / 3.0;

/// Default Let's Encrypt production directory.
pub const LETS_ENCRYPT_PROD_DIRECTORY: &str = "https://acme-v02.api.letsencrypt.org/directory";

/// Environment variable for the ACME directory URL.
pub const ENV_ACME_DIRECTORY_URL: &str = "NITRUM_ACME_DIRECTORY_URL";

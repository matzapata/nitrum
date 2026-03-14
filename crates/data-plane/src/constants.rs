/// VSOCK port where gvproxy listens on the host (CID 3).
pub const HOST_PROXY_PORT: u32 = 1024;

/// Default address for the data-plane API server (attestation, encrypt, decrypt).
pub const API_LISTEN_ADDR: &str = "0.0.0.0:3000";

/// Ingress proxy: listens here and forwards to the user app.
pub const INGRESS_LISTEN_ADDR: &str = "0.0.0.0:443";

// TODO: this is for testing only, check if we can make it to 80 or dynamic at least
/// Plain HTTP listener for ACME HTTP-01 challenge (Pebble validates on this port).
pub const INGRESS_ACME_HTTP01_LISTEN_ADDR: &str = "0.0.0.0:5002";


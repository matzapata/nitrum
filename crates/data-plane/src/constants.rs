/// VSOCK port where gvproxy listens on the host (CID 3).
pub const HOST_PROXY_PORT: u32 = 1024;

/// Default address for the data-plane API server (attestation, encrypt, decrypt).
pub const API_LISTEN_ADDR: &str = "0.0.0.0:3000";

/// Ingress proxy: listens here and forwards to the user app.
pub const INGRESS_LISTEN_ADDR: &str = "0.0.0.0:443";

/// Port the user app listens on inside the enclave.
pub const APP_PORT: u16 = 8080;

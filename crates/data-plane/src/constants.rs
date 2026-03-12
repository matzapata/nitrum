/// VSOCK port where gvproxy listens on the host (CID 3).
pub const HOST_PROXY_PORT: u32 = 1024;

/// Default address for the data-plane API server (attestation, encrypt, decrypt).
pub const API_LISTEN_ADDR: &str = "0.0.0.0:3000";

/// Ingress proxy: listens here and forwards to the user app.
pub const INGRESS_LISTEN_ADDR: &str = "0.0.0.0:443";

/// DynamoDB partition key for the leader lock item.
pub const LOCK_OBJECT_KEY: &str = "lock";

/// Leader lock TTL in seconds (DynamoDB TTL attribute).
pub const LOCK_TTL_SECS: u64 = 60;

//! Shared control-plane configuration (paths, timeouts, networking defaults).

use std::time::Duration;

// ── Enclave / nitro-cli ─────────────────────────────────────────────────────

/// Path to the nitro-cli binary.
pub const NITRO_CLI: &str = "nitro-cli";

/// Path to the enclave image file (control-plane container).
pub const EIF_PATH: &str = "/app/enclave.eif";

/// CID passed to `nitro-cli run-enclave --enclave-cid`.
pub const ENCLAVE_CID: &str = "16";

/// How often we check whether the enclave is still listed by `describe-enclaves`.
pub const ENCLAVE_HEALTH_POLL: Duration = Duration::from_secs(5);

/// Max delay before a restart attempt after the enclave is gone or `run-enclave` fails.
pub const MAX_BACKOFF_SECS: u64 = 300;

// ── Networking / gvproxy ─────────────────────────────────────────────────────

/// Path to the gvproxy API Unix socket.
pub const SOCKET_PATH: &str = "/tmp/network.sock";

/// VSOCK listen address for gvproxy.
pub const VSOCK_LISTEN: &str = ":1024";

/// Enclave IP on the vsock/TAP network (gvproxy forwards to this).
pub const ENCLAVE_IP: &str = "192.168.127.2";

/// Timeout to wait for the gvproxy socket file to appear.
pub const SOCKET_WAIT_TIMEOUT_SECS: u64 = 15;

/// Port forwards: (host_port, enclave_port) — HTTP for ACME HTTP-01, HTTPS for ingress.
pub const FORWARDS: &[(u16, u16)] = &[(80, 80), (443, 443)];

/// Env var for gvproxy binary path; default `gvproxy` (on PATH). Set to `/app/gvproxy` in container.
pub const GVPROXY_BIN_ENV: &str = "GVPROXY_BIN";

/// Default gvproxy binary path.
pub const GVPROXY_BIN_DEFAULT: &str = "gvproxy";

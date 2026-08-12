//! gvproxy networking constants (host side).

/// Path to the gvproxy API Unix socket.
pub const SOCKET_PATH: &str = "/tmp/network.sock";

/// VSOCK listen address for gvproxy.
pub const VSOCK_LISTEN: &str = ":1024";

/// Enclave IP on the vsock/TAP network (gvproxy forwards to this).
pub const ENCLAVE_IP: &str = "192.168.127.2";

/// Timeout to wait for the gvproxy socket file to appear.
pub const SOCKET_WAIT_TIMEOUT_SECS: u64 = 15;

/// Port forwards: (`host_port`, `enclave_port`) — HTTP for ACME HTTP-01, HTTPS for ingress.
pub const FORWARDS: &[(u16, u16)] = &[(80, 80), (443, 443)];

/// Env var for gvproxy binary path; default `gvproxy` (on PATH). Set to `/app/gvproxy` in container.
pub const GVPROXY_BIN_ENV: &str = "GVPROXY_BIN";

/// Default gvproxy binary path.
pub const GVPROXY_BIN_DEFAULT: &str = "gvproxy";

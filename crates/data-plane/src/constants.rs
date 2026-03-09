/// Local TCP proxy listener port used by egress interception.
pub const TCP_PROXY_PORT: u16 = 4444;

/// Local address the DNS proxy binds to.
/// resolv.conf inside the container points here so all DNS is intercepted.
pub const DNS_LISTEN_ADDR: &str = "127.0.0.1:53";

/// Unix uid of the `dataplane` OS user (created in data-plane.dockerfile).
/// Used by network interception rules to avoid redirecting the data-plane's own
/// outbound connections back into the egress proxy.
pub const DATAPLANE_UID: u32 = 1500;

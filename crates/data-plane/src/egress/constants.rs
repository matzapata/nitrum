//! Egress proxy ports and environment overrides.

/// Port the transparent TCP proxy listens on (iptables REDIRECT target).
pub const TCP_PROXY_PORT: u16 = 18080;

/// Port the DNS proxy listens on inside the enclave.
pub const DNS_PROXY_PORT: u16 = 53;

/// Default upstream DNS when running on the gvproxy TAP network.
pub const ENCLAVE_UPSTREAM_DNS: &str = "192.168.127.1:53";

/// Environment variable overriding the upstream DNS resolver (`host:port`).
pub const ENV_UPSTREAM_DNS: &str = "NITRUM_EGRESS_UPSTREAM_DNS";

/// Default TTL for IP→hostname cache entries populated from allowed DNS responses.
pub const IP_CACHE_DEFAULT_TTL_SECS: u64 = 300;

/// iptables chain name for egress NAT rules.
pub const IPTABLES_CHAIN: &str = "NITRUM_EGRESS";

/// `SO_MARK` value on upstream sockets opened by the egress TCP proxy (bypasses re-capture).
pub const EGRESS_SOCKET_MARK: u32 = 0x1;

/// Maximum concurrent upstream DNS exchanges handled by the egress DNS proxy.
pub const DNS_MAX_CONCURRENT_QUERIES: usize = 64;

/// Upstream DNS response timeout for a single query exchange.
pub const DNS_UPSTREAM_TIMEOUT_SECS: u64 = 5;

/// IMDS region fetch attempts when no region is supplied at egress init.
pub const AWS_REGION_FETCH_ATTEMPTS: u32 = 5;

/// Initial backoff between IMDS region fetch retries.
pub const AWS_REGION_FETCH_INITIAL_BACKOFF_MS: u64 = 200;

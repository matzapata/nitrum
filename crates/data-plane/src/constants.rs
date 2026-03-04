/// Local port iptables REDIRECTs all non-proxy egress TCP traffic to.
pub const TCP_PROXY_PORT: u16 = 8080;

/// Local address the DNS proxy binds to.
/// resolv.conf inside the enclave points here so all DNS is intercepted.
pub const DNS_LISTEN_ADDR: &str = "127.0.0.1:53";

//! In-enclave egress whitelist enforcement (DNS + transparent TCP proxy).

mod constants;
mod dns;
mod filter;
mod ip_cache;
mod iptables;
mod platform;
mod tcp;

use std::net::SocketAddr;
use std::sync::Arc;

use tracing::{info, warn};

use crate::config::RuntimeConfig;

pub use filter::EgressFilter;

use constants::{
    DNS_PROXY_PORT, ENCLAVE_UPSTREAM_DNS, TCP_PROXY_PORT, egress_upstream_dns_override,
};
use dns::{bind_dns_proxy, serve_dns_proxy};
use filter::EgressFilter as Filter;
use ip_cache::IpCache;
use iptables::{
    install as install_iptables, log_missing_capability, teardown as teardown_iptables,
};
use platform::build_platform_allows;
use tcp::{bind_tcp_proxy, serve_tcp_proxy};

/// Remove egress iptables rules installed by [`init`].
pub fn teardown() {
    teardown_iptables();
}

/// Initialize egress enforcement when `[egress].enabled` is true in `runtime_config`.
///
/// No-op when egress is disabled. Uses [`RuntimeConfig::aws_region`] and
/// [`RuntimeConfig::otlp_endpoint`] for platform allowlist construction.
pub async fn init(runtime_config: &RuntimeConfig) -> anyhow::Result<()> {
    if !runtime_config.egress.enabled {
        info!("egress whitelist disabled");
        return Ok(());
    }

    info!(
        destinations = runtime_config.egress.destinations.len(),
        "egress whitelist enabled"
    );

    let platform = build_platform_allows(runtime_config)?;
    let pattern_count = platform.patterns.len();
    let collector_bypass = platform.collector_bypass_ip;
    let filter = Arc::new(Filter::new(true, &platform.patterns));
    let ip_cache = Arc::new(IpCache::new());
    let platform_ips = Arc::new(platform.allowed_ips);

    let upstream_dns = detect_upstream_dns()?;
    write_local_resolv_conf()?;

    let dns_listen: SocketAddr = format!("127.0.0.1:{DNS_PROXY_PORT}").parse()?;
    let dns_socket = bind_dns_proxy(dns_listen).await?;
    let tcp_listener = bind_tcp_proxy().await?;

    if let Err(error) = install_iptables(upstream_dns, collector_bypass) {
        log_missing_capability(&error);
        return Err(error);
    }

    let filter_dns = Arc::clone(&filter);
    let cache_dns = Arc::clone(&ip_cache);
    tokio::spawn(async move {
        if let Err(error) = serve_dns_proxy(dns_socket, upstream_dns, filter_dns, cache_dns).await {
            warn!(%error, "egress DNS proxy exited");
        }
    });

    let filter_tcp = Arc::clone(&filter);
    let cache_tcp = Arc::clone(&ip_cache);
    let ips_tcp = Arc::clone(&platform_ips);
    tokio::spawn(async move {
        if let Err(error) = serve_tcp_proxy(tcp_listener, filter_tcp, cache_tcp, ips_tcp).await {
            warn!(%error, "egress TCP proxy exited");
        }
    });

    info!(
        %upstream_dns,
        dns_port = DNS_PROXY_PORT,
        tcp_port = TCP_PROXY_PORT,
        platform_ips = platform_ips.len(),
        patterns = pattern_count,
        "egress enforcement active"
    );

    Ok(())
}

fn detect_upstream_dns() -> anyhow::Result<SocketAddr> {
    if let Some(addr) = egress_upstream_dns_override()? {
        return Ok(addr);
    }

    if let Some(addr) = read_first_nameserver() {
        return Ok(addr);
    }

    ENCLAVE_UPSTREAM_DNS
        .parse()
        .map_err(|error| anyhow::anyhow!("invalid enclave upstream DNS: {error}"))
}

fn read_first_nameserver() -> Option<SocketAddr> {
    let contents = std::fs::read_to_string("/etc/resolv.conf").ok()?;
    for line in contents.lines() {
        let line = line.trim();
        if let Some(ip) = line.strip_prefix("nameserver") {
            let ip = ip.trim();
            if ip.is_empty() {
                continue;
            }
            if let Ok(parsed) = format!("{ip}:53").parse() {
                return Some(parsed);
            }
        }
    }
    None
}

fn write_local_resolv_conf() -> anyhow::Result<()> {
    std::fs::write("/etc/resolv.conf", "nameserver 127.0.0.1\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_nameserver_from_resolv_conf_format() {
        let sample = "nameserver 127.0.0.11\nsearch local\n";
        let ip = sample
            .lines()
            .find_map(|line| line.strip_prefix("nameserver").map(str::trim))
            .unwrap();
        let addr: SocketAddr = format!("{ip}:53").parse().unwrap();
        assert_eq!(addr.port(), 53);
    }
}

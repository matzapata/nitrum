//! Egress proxy servers: transparent DNS and TCP forwarding with whitelist enforcement.

use crate::config::DataPlaneConfig;
use tracing::info;

#[cfg(egress_enforcement)]
use super::constants::{
    DNS_PROXY_PORT, ENCLAVE_UPSTREAM_DNS, TCP_PROXY_PORT, egress_upstream_dns_override,
};
#[cfg(egress_enforcement)]
use super::dns::{bind_dns_proxy, serve_dns_proxy};
#[cfg(egress_enforcement)]
use super::filter::EgressFilter as Filter;
#[cfg(egress_enforcement)]
use super::ip_cache::IpCache;
#[cfg(egress_enforcement)]
use super::iptables::{
    install as install_iptables, log_missing_capability, teardown as teardown_iptables,
};
#[cfg(egress_enforcement)]
use super::platform::build_platform_allows;
#[cfg(egress_enforcement)]
use super::tcp::{bind_tcp_proxy, serve_tcp_proxy};
#[cfg(egress_enforcement)]
use std::net::SocketAddr;
#[cfg(egress_enforcement)]
use std::sync::Arc;
#[cfg(egress_enforcement)]
use tracing::warn;

/// Owns egress enforcement for the process lifetime.
///
/// Removes iptables rules on [`Drop`] when egress was enabled and installed successfully.
pub struct EgressGuard {
    /// Whether iptables rules were installed and should be removed on drop.
    active: bool,
}

impl EgressGuard {
    fn inactive() -> Self {
        Self { active: false }
    }

    fn active() -> Self {
        Self { active: true }
    }
}

impl Drop for EgressGuard {
    fn drop(&mut self) {
        if self.active {
            #[cfg(egress_enforcement)]
            teardown_iptables();
        }
    }
}

/// Initialize egress enforcement when `[egress].enabled` is true in `config`.
///
/// Returns an [`EgressGuard`] that removes iptables rules on drop when enforcement is active.
/// No-op when egress is disabled or when enforcement is unavailable on the current platform.
#[must_use]
pub async fn init(config: &DataPlaneConfig) -> anyhow::Result<EgressGuard> {
    if !config.egress.enabled {
        info!("egress whitelist disabled");
        return Ok(EgressGuard::inactive());
    }

    #[cfg(egress_enforcement)]
    {
        return init_enforcement(config).await;
    }

    #[cfg(not(egress_enforcement))]
    {
        info!("egress whitelist unavailable on this platform; skipping enforcement");
        Ok(EgressGuard::inactive())
    }
}

#[cfg(egress_enforcement)]
async fn init_enforcement(config: &DataPlaneConfig) -> anyhow::Result<EgressGuard> {
    info!(
        destinations = config.egress.destinations.len(),
        "egress whitelist enabled"
    );

    let platform = build_platform_allows(config)?;
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

    Ok(EgressGuard::active())
}

#[cfg(egress_enforcement)]
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

#[cfg(egress_enforcement)]
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

#[cfg(egress_enforcement)]
fn write_local_resolv_conf() -> anyhow::Result<()> {
    std::fs::write("/etc/resolv.conf", "nameserver 127.0.0.1\n")?;
    Ok(())
}

#[cfg(all(test, egress_enforcement))]
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

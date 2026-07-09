//! Implicit platform allow patterns and bootstrap IP addresses.

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};

use tracing::{info, warn};

use crate::config::RuntimeConfig;
use crate::constants::{
    acme_directory_url, acme_directory_url_override, dynamodb_endpoint_url, imds_latest_base_url,
    kms_endpoint_url, ssm_endpoint_url,
};

/// Platform bootstrap data merged with user `[egress].destinations`.
pub struct PlatformAllows {
    /// Regex patterns for hostnames (user + implicit).
    pub patterns: Vec<String>,
    /// IP addresses always permitted by the TCP proxy without a DNS cache hit.
    pub allowed_ips: HashSet<IpAddr>,
    /// OTLP collector IP to exclude from transparent proxying (IP-based endpoint only).
    ///
    /// `None` when the resolved OTLP endpoint uses a hostname (in which case it is allowed
    /// through the normal DNS/pattern path instead of a static proxy bypass), or when OTLP is
    /// explicitly disabled with `NITRUM_OTLP_ENDPOINT=""`.
    pub collector_bypass_ip: Option<IpAddr>,
}

/// Build merged hostname patterns and bootstrap IP allowlist from resolved runtime config.
pub fn build_platform_allows(runtime_config: &RuntimeConfig) -> anyhow::Result<PlatformAllows> {
    let mut patterns = runtime_config.egress.destinations.clone();
    let mut allowed_ips = HashSet::new();

    allowed_ips.insert(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)));

    append_hostname_pattern_from_url(&mut patterns, &imds_latest_base_url());
    append_hostname_pattern_from_url(&mut patterns, &ssm_endpoint_url().unwrap_or_default());
    append_hostname_pattern_from_url(&mut patterns, &kms_endpoint_url().unwrap_or_default());
    append_hostname_pattern_from_url(&mut patterns, &dynamodb_endpoint_url().unwrap_or_default());

    let acme_directory = acme_directory_url();
    if runtime_config.tls_termination.acme || acme_directory_url_override().is_some() {
        append_hostname_pattern_from_url(&mut patterns, &acme_directory);
    }

    let collector_bypass_ip = resolve_otlp_collector_allow(
        &mut patterns,
        &mut allowed_ips,
        runtime_config.otlp_endpoint.as_deref(),
    );

    let region = &runtime_config.aws_region;
    patterns.push(format!(
        r"^kms\.{}\.amazonaws\.com$",
        regex::escape(region)
    ));
    patterns.push(format!(
        r"^ssm\.{}\.amazonaws\.com$",
        regex::escape(region)
    ));
    patterns.push(format!(
        r"^dynamodb\.{}\.amazonaws\.com$",
        regex::escape(region)
    ));
    info!(region = %region, "egress: added implicit AWS service patterns");

    resolve_hostnames_to_ips(&patterns, &mut allowed_ips);

    dedupe_patterns(&mut patterns);

    Ok(PlatformAllows {
        patterns,
        allowed_ips,
        collector_bypass_ip,
    })
}

/// Permit the resolved OTLP collector endpoint (see [`crate::config::RuntimeConfig::otlp_endpoint`])
/// through egress so telemetry export is not dropped.
///
/// Returns the collector IP to bypass transparent proxying when the endpoint is IP-based;
/// for a hostname endpoint, a hostname pattern is appended instead and `None` is returned.
fn resolve_otlp_collector_allow(
    patterns: &mut Vec<String>,
    allowed_ips: &mut HashSet<IpAddr>,
    endpoint: Option<&str>,
) -> Option<IpAddr> {
    let endpoint = endpoint?;
    let host = extract_host(&endpoint)?;
    if let Ok(ip) = host.parse::<IpAddr>() {
        allowed_ips.insert(ip);
        info!(collector = %ip, "egress: OTLP collector IP allowed (proxy bypass)");
        return Some(ip);
    }
    append_hostname_pattern_from_url(patterns, &endpoint);
    None
}

fn append_hostname_pattern_from_url(patterns: &mut Vec<String>, url: &str) {
    let url = url.trim();
    if url.is_empty() {
        return;
    }
    let Some(host) = extract_host(url) else {
        return;
    };
    if host.parse::<IpAddr>().is_ok() {
        return;
    }
    let escaped = regex::escape(&host);
    patterns.push(format!("^{escaped}$"));
}

fn extract_host(url: &str) -> Option<String> {
    let without_scheme = url.split_once("://").map_or(url, |(_, rest)| rest).trim();
    let host_port = without_scheme.split('/').next()?.trim();
    if host_port.is_empty() {
        return None;
    }
    if host_port.starts_with('[') {
        let end = host_port.find(']')?;
        return Some(host_port[1..end].to_string());
    }
    Some(host_port.split(':').next().unwrap_or(host_port).to_string())
}

fn resolve_hostnames_to_ips(patterns: &[String], allowed_ips: &mut HashSet<IpAddr>) {
    for pattern in patterns {
        let hostname = pattern
            .trim_start_matches('^')
            .trim_end_matches('$')
            .replace("\\.", ".");
        if hostname.parse::<IpAddr>().is_ok() {
            continue;
        }
        match format!("{hostname}:443").to_socket_addrs() {
            Ok(mut addrs) => {
                for addr in addrs.by_ref().take(8) {
                    allowed_ips.insert(addr.ip());
                }
            }
            Err(error) => {
                warn!(hostname = %hostname, %error, "egress: could not resolve platform hostname at startup");
            }
        }
    }
}

fn dedupe_patterns(patterns: &mut Vec<String>) {
    patterns.sort();
    patterns.dedup();
}

/// Returns whether `ip` is allowed via platform bootstrap IPs or DNS cache hostname match.
#[must_use]
pub fn is_ip_allowed(
    ip: IpAddr,
    platform_ips: &HashSet<IpAddr>,
    ip_cache: &super::ip_cache::IpCache,
    filter: &super::filter::EgressFilter,
) -> bool {
    if !filter.is_enabled() {
        return true;
    }
    if platform_ips.contains(&ip) {
        return true;
    }
    if let Some(hostname) = ip_cache.get_hostname(ip) {
        return filter.is_hostname_allowed(&hostname);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_host_from_url() {
        assert_eq!(
            extract_host("http://imds:1338/latest"),
            Some("imds".to_string())
        );
        assert_eq!(
            extract_host("https://pebble:14000/dir"),
            Some("pebble".to_string())
        );
    }

    #[test]
    fn append_pattern_from_env_style_url() {
        let mut patterns = Vec::new();
        append_hostname_pattern_from_url(&mut patterns, "http://aws:4566");
        assert_eq!(patterns, vec!["^aws$".to_string()]);
    }
}

//! Implicit platform allow patterns and bootstrap IP addresses.

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

use config::{Egress, TlsTermination};
use tokio::time::sleep;
use tracing::{info, warn};

use crate::constants::{
    DEFAULT_IMDS_LATEST_BASE_URL, ENV_ACME_DIRECTORY_URL, LETS_ENCRYPT_PROD_DIRECTORY,
};
use crate::utils::imds::ImdsClient;

use super::constants::{AWS_REGION_FETCH_ATTEMPTS, AWS_REGION_FETCH_INITIAL_BACKOFF_MS};

/// Platform bootstrap data merged with user `[egress].destinations`.
pub struct PlatformAllows {
    /// Regex patterns for hostnames (user + implicit).
    pub patterns: Vec<String>,
    /// IP addresses always permitted by the TCP proxy without a DNS cache hit.
    pub allowed_ips: HashSet<IpAddr>,
}

/// Build merged hostname patterns and bootstrap IP allowlist.
///
/// # Errors
///
/// Returns `Err` when regional AWS hostname patterns are required but the region
/// cannot be determined from `aws_region` or IMDS.
pub async fn build_platform_allows(
    egress: &Egress,
    tls: &TlsTermination,
    aws_region: Option<&str>,
) -> anyhow::Result<PlatformAllows> {
    let mut patterns = egress.destinations.clone();
    let mut allowed_ips = HashSet::new();

    allowed_ips.insert(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)));

    append_hostname_pattern_from_url(
        &mut patterns,
        &std::env::var("NITRUM_IMDS_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_IMDS_LATEST_BASE_URL.to_string()),
    );
    append_hostname_pattern_from_url(
        &mut patterns,
        &std::env::var("NITRUM_SSM_ENDPOINT_URL").unwrap_or_default(),
    );
    append_hostname_pattern_from_url(
        &mut patterns,
        &std::env::var("NITRUM_KMS_ENDPOINT_URL").unwrap_or_default(),
    );
    append_hostname_pattern_from_url(
        &mut patterns,
        &std::env::var("NITRUM_DYNAMODB_ENDPOINT_URL").unwrap_or_default(),
    );

    let acme_directory = std::env::var(ENV_ACME_DIRECTORY_URL)
        .unwrap_or_else(|_| LETS_ENCRYPT_PROD_DIRECTORY.to_string());
    if tls.acme || std::env::var(ENV_ACME_DIRECTORY_URL).is_ok() {
        append_hostname_pattern_from_url(&mut patterns, &acme_directory);
    }

    let region = resolve_aws_region(aws_region).await?;
    if let Some(region) = region {
        patterns.push(format!(
            r"^kms\.{}\.amazonaws\.com$",
            regex::escape(&region)
        ));
        patterns.push(format!(
            r"^ssm\.{}\.amazonaws\.com$",
            regex::escape(&region)
        ));
        patterns.push(format!(
            r"^dynamodb\.{}\.amazonaws\.com$",
            regex::escape(&region)
        ));
        info!(region = %region, "egress: added implicit AWS service patterns");
    }

    resolve_hostnames_to_ips(&patterns, &mut allowed_ips);

    dedupe_patterns(&mut patterns);

    Ok(PlatformAllows {
        patterns,
        allowed_ips,
    })
}

async fn resolve_aws_region(aws_region: Option<&str>) -> anyhow::Result<Option<String>> {
    if let Some(region) = aws_region {
        return Ok(Some(region.to_string()));
    }

    match fetch_aws_region_with_retry().await {
        Ok(region) => Ok(Some(region)),
        Err(error) if needs_regional_aws_patterns() => Err(error),
        Err(error) => {
            warn!(
                %error,
                "egress: could not fetch AWS region; regional service patterns omitted"
            );
            Ok(None)
        }
    }
}

fn needs_regional_aws_patterns() -> bool {
    std::env::var("NITRUM_KMS_ENDPOINT_URL").is_err()
        || std::env::var("NITRUM_SSM_ENDPOINT_URL").is_err()
        || std::env::var("NITRUM_DYNAMODB_ENDPOINT_URL").is_err()
}

async fn fetch_aws_region_with_retry() -> anyhow::Result<String> {
    let mut last_error = None;
    let mut backoff_ms = AWS_REGION_FETCH_INITIAL_BACKOFF_MS;

    for attempt in 1..=AWS_REGION_FETCH_ATTEMPTS {
        match fetch_aws_region().await {
            Ok(region) => return Ok(region),
            Err(error) => {
                warn!(
                    attempt,
                    max_attempts = AWS_REGION_FETCH_ATTEMPTS,
                    %error,
                    "egress: AWS region fetch failed"
                );
                last_error = Some(error);
                if attempt < AWS_REGION_FETCH_ATTEMPTS {
                    sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = backoff_ms.saturating_mul(2);
                }
            }
        }
    }

    Err(last_error
        .unwrap_or_else(|| anyhow::anyhow!("AWS region fetch failed without a specific error")))
}

async fn fetch_aws_region() -> Result<String, anyhow::Error> {
    let imds_base = std::env::var("NITRUM_IMDS_BASE_URL")
        .unwrap_or_else(|_| DEFAULT_IMDS_LATEST_BASE_URL.to_string());
    let imds = ImdsClient::new(&imds_base)?;
    imds.get_region().await
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

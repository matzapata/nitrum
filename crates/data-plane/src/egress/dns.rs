//! DNS query parsing, NXDOMAIN synthesis, and UDP proxy.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::Semaphore;
use tokio::time::timeout;
use tracing::{info, warn};

use super::constants::{DNS_MAX_CONCURRENT_QUERIES, DNS_UPSTREAM_TIMEOUT_SECS};
use super::filter::EgressFilter;
use super::ip_cache::IpCache;

/// Decision for an incoming DNS query after whitelist evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryDecision {
    /// Forward the query to the upstream resolver.
    Forward,
    /// Respond with NXDOMAIN locally.
    NxDomain,
}

/// Evaluate whether a raw DNS query should be forwarded or blocked.
#[must_use]
pub fn evaluate_query(query: &[u8], filter: &EgressFilter) -> QueryDecision {
    if !filter.is_enabled() {
        return QueryDecision::Forward;
    }
    match parse_query_name(query) {
        Some(hostname) if filter.is_hostname_allowed(&hostname) => QueryDecision::Forward,
        Some(_) => QueryDecision::NxDomain,
        None => QueryDecision::NxDomain,
    }
}

/// Extract the queried hostname from a raw DNS query packet.
#[must_use]
pub fn parse_query_name(query: &[u8]) -> Option<String> {
    if query.len() < 13 {
        return None;
    }
    let mut pos = 12;
    let mut labels: Vec<&str> = Vec::new();
    loop {
        if pos >= query.len() {
            return None;
        }
        let len = query[pos] as usize;
        if len == 0 {
            break;
        }
        if len & 0xC0 == 0xC0 {
            return None;
        }
        pos += 1;
        if pos + len > query.len() {
            return None;
        }
        labels.push(std::str::from_utf8(&query[pos..pos + len]).ok()?);
        pos += len;
    }
    Some(labels.join("."))
}

/// Build a minimal NXDOMAIN response for the given raw DNS query.
#[must_use]
pub fn make_nxdomain(query: &[u8]) -> Vec<u8> {
    let mut resp = Vec::with_capacity(query.len());

    if query.len() >= 2 {
        resp.extend_from_slice(&query[0..2]);
    } else {
        resp.extend_from_slice(&[0, 0]);
    }

    // QR=1, RD=1, RA=1, RCODE=3 (NXDOMAIN)
    resp.push(0x81);
    resp.push(0x83);

    if query.len() >= 6 {
        resp.extend_from_slice(&query[4..6]);
    } else {
        resp.extend_from_slice(&[0, 1]);
    }

    resp.extend_from_slice(&[0, 0, 0, 0, 0, 0]);

    if query.len() > 12 {
        resp.extend_from_slice(&query[12..]);
    }

    resp
}

/// Parse A and AAAA records from a DNS response and return `(ip, ttl)` pairs.
#[must_use]
pub fn extract_a_aaaa_ips(response: &[u8]) -> Vec<(IpAddr, u32)> {
    if response.len() < 12 {
        return Vec::new();
    }

    let question_count = u16::from_be_bytes([response[4], response[5]]) as usize;
    let answer_count = u16::from_be_bytes([response[6], response[7]]) as usize;

    let mut pos = 12;
    for _ in 0..question_count {
        pos = match skip_dns_name(response, pos) {
            Some(next) => next,
            None => return Vec::new(),
        };
        if pos + 4 > response.len() {
            return Vec::new();
        }
        pos += 4;
    }

    let mut ips = Vec::new();
    for _ in 0..answer_count {
        pos = match skip_dns_name(response, pos) {
            Some(next) => next,
            None => break,
        };
        if pos + 10 > response.len() {
            break;
        }
        let record_type = u16::from_be_bytes([response[pos], response[pos + 1]]);
        let _class = u16::from_be_bytes([response[pos + 2], response[pos + 3]]);
        let ttl = u32::from_be_bytes([
            response[pos + 4],
            response[pos + 5],
            response[pos + 6],
            response[pos + 7],
        ]);
        let rdlength = u16::from_be_bytes([response[pos + 8], response[pos + 9]]) as usize;
        pos += 10;
        if pos + rdlength > response.len() {
            break;
        }
        let rdata = &response[pos..pos + rdlength];
        match record_type {
            1 if rdlength == 4 => {
                ips.push((
                    IpAddr::V4(Ipv4Addr::new(rdata[0], rdata[1], rdata[2], rdata[3])),
                    ttl,
                ));
            }
            28 if rdlength == 16 => {
                let mut octets = [0u8; 16];
                octets.copy_from_slice(rdata);
                ips.push((IpAddr::V6(Ipv6Addr::from(octets)), ttl));
            }
            _ => {}
        }
        pos += rdlength;
    }

    ips
}

fn skip_dns_name(packet: &[u8], mut pos: usize) -> Option<usize> {
    loop {
        if pos >= packet.len() {
            return None;
        }
        let len = packet[pos] as usize;
        if len == 0 {
            return Some(pos + 1);
        }
        if len & 0xC0 == 0xC0 {
            return (pos + 2 <= packet.len()).then_some(pos + 2);
        }
        pos += 1;
        if pos + len > packet.len() {
            return None;
        }
        pos += len;
    }
}

/// Start the DNS proxy on `listen_addr`, forwarding allowed queries to `upstream`.
pub async fn bind_dns_proxy(listen_addr: SocketAddr) -> anyhow::Result<Arc<UdpSocket>> {
    let socket = Arc::new(UdpSocket::bind(listen_addr).await?);
    info!(%listen_addr, "egress DNS proxy bound");
    Ok(socket)
}

/// Serve DNS queries on a bound socket until an unrecoverable error occurs.
pub async fn serve_dns_proxy(
    socket: Arc<UdpSocket>,
    upstream: SocketAddr,
    filter: Arc<EgressFilter>,
    ip_cache: Arc<IpCache>,
) -> anyhow::Result<()> {
    info!(%upstream, "egress DNS proxy serving");

    let semaphore = Arc::new(Semaphore::new(DNS_MAX_CONCURRENT_QUERIES));
    let mut buf = vec![0u8; 4096];
    loop {
        let (len, client_addr) = match socket.recv_from(&mut buf).await {
            Ok(result) => result,
            Err(error) => {
                warn!(%error, "egress dns: recv error");
                continue;
            }
        };

        let Ok(permit) = semaphore.clone().acquire_owned().await else {
            break;
        };

        let query = buf[..len].to_vec();
        let sock = Arc::clone(&socket);
        let filter = Arc::clone(&filter);
        let ip_cache = Arc::clone(&ip_cache);

        tokio::spawn(async move {
            let _permit = permit;

            if evaluate_query(&query, &filter) == QueryDecision::NxDomain {
                if let Some(hostname) = parse_query_name(&query) {
                    warn!(hostname = %hostname, "egress dns: blocked by whitelist");
                } else {
                    warn!("egress dns: blocked malformed query");
                }
                let nxdomain = make_nxdomain(&query);
                let _ = sock.send_to(&nxdomain, client_addr).await;
                return;
            }

            let hostname = parse_query_name(&query);
            let upstream_sock = match UdpSocket::bind("0.0.0.0:0").await {
                Ok(s) => s,
                Err(error) => {
                    warn!(%error, "egress dns: failed to bind ephemeral socket");
                    return;
                }
            };

            if upstream_sock.send_to(&query, upstream).await.is_err() {
                warn!(%upstream, "egress dns: upstream send failed");
                return;
            }

            let mut response_buf = vec![0u8; 4096];
            let response_len = match timeout(
                Duration::from_secs(DNS_UPSTREAM_TIMEOUT_SECS),
                upstream_sock.recv_from(&mut response_buf),
            )
            .await
            {
                Ok(Ok((n, _))) => n,
                Ok(Err(error)) => {
                    warn!(%error, "egress dns: upstream recv failed");
                    return;
                }
                Err(_) => {
                    warn!(
                        timeout_secs = DNS_UPSTREAM_TIMEOUT_SECS,
                        %upstream,
                        "egress dns: upstream recv timed out"
                    );
                    return;
                }
            };

            let response = &response_buf[..response_len];
            if let Some(ref name) = hostname {
                for (ip, ttl) in extract_a_aaaa_ips(response) {
                    ip_cache.insert(ip, name.clone(), ttl);
                }
            }

            let _ = sock.send_to(response, client_addr).await;
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_query_name() {
        // Query for example.com (type A)
        let query: Vec<u8> = vec![
            0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, b'e',
            b'x', b'a', b'm', b'p', b'l', b'e', 0x03, b'c', b'o', b'm', 0x00, 0x00, 0x01, 0x00,
            0x01,
        ];
        assert_eq!(parse_query_name(&query), Some("example.com".to_string()));
    }

    #[test]
    fn nxdomain_preserves_query_id() {
        let query = [0xAB, 0xCD, 0x01, 0x00, 0x00, 0x01];
        let resp = make_nxdomain(&query);
        assert_eq!(resp[0], 0xAB);
        assert_eq!(resp[1], 0xCD);
        assert_eq!(resp[3], 0x83);
    }

    #[test]
    fn evaluate_blocks_unknown_host() {
        let filter = EgressFilter::new(true, &["allowed\\.com$".to_string()]);
        let query: Vec<u8> = vec![
            0x00, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, b'e',
            b'x', b'a', b'm', b'p', b'l', b'e', 0x03, b'c', b'o', b'm', 0x00, 0x00, 0x01, 0x00,
            0x01,
        ];
        assert_eq!(evaluate_query(&query, &filter), QueryDecision::NxDomain);
    }

    #[test]
    fn evaluate_blocks_unparseable_query() {
        let filter = EgressFilter::new(true, &["allowed\\.com$".to_string()]);
        assert_eq!(
            evaluate_query(&[0x00, 0x01], &filter),
            QueryDecision::NxDomain
        );
    }
}

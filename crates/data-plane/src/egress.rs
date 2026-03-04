use regex::Regex;

/// Compiled whitelist of hostname patterns applied at the DNS proxy layer.
///
/// When enabled, only DNS queries for hostnames matching at least one pattern are
/// forwarded to the control-plane. Blocked queries receive a NXDOMAIN response
/// immediately, so the app never obtains an IP to connect to.
///
/// Note: connections to hard-coded IPs that bypass DNS are not filtered.
pub struct EgressFilter {
    enabled: bool,
    patterns: Vec<Regex>,
}

impl EgressFilter {
    pub fn new(enabled: bool, whitelist: &[String]) -> Self {
        let patterns = whitelist
            .iter()
            .map(|p| {
                Regex::new(p)
                    .unwrap_or_else(|e| panic!("invalid egress whitelist regex {p:?}: {e}"))
            })
            .collect();
        Self { enabled, patterns }
    }

    /// Returns `true` if the hostname is permitted to make outbound connections.
    pub fn is_allowed(&self, hostname: &str) -> bool {
        if !self.enabled {
            return true;
        }
        self.patterns.iter().any(|re| re.is_match(hostname))
    }
}

/// Extract the queried hostname from a raw DNS query packet.
///
/// Returns `None` if the packet is malformed or too short.
pub fn parse_query_name(query: &[u8]) -> Option<String> {
    if query.len() < 13 {
        return None;
    }
    let mut pos = 12; // skip 12-byte DNS header
    let mut labels: Vec<&str> = Vec::new();
    loop {
        if pos >= query.len() {
            return None;
        }
        let len = query[pos] as usize;
        if len == 0 {
            break;
        }
        // Compression pointers (top 2 bits set) are not expected in queries.
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
pub fn make_nxdomain(query: &[u8]) -> Vec<u8> {
    let mut resp = Vec::with_capacity(query.len());

    // Transaction ID: copy from query bytes 0-1
    if query.len() >= 2 {
        resp.extend_from_slice(&query[0..2]);
    } else {
        resp.extend_from_slice(&[0, 0]);
    }

    // Flags: QR=1 Opcode=0 AA=0 TC=0 RD=1 | RA=1 Z=0 RCODE=3 (NXDOMAIN)
    resp.push(0x81);
    resp.push(0x83);

    // QDCOUNT: copy from query bytes 4-5
    if query.len() >= 6 {
        resp.extend_from_slice(&query[4..6]);
    } else {
        resp.extend_from_slice(&[0, 1]);
    }

    // ANCOUNT=0, NSCOUNT=0, ARCOUNT=0
    resp.extend_from_slice(&[0, 0, 0, 0, 0, 0]);

    // Question section: everything after the 12-byte header
    if query.len() > 12 {
        resp.extend_from_slice(&query[12..]);
    }

    resp
}

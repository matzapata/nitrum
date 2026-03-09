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

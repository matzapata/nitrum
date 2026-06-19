//! Hostname whitelist compiled from user config and platform implicit allows.

use regex::Regex;

/// Compiled whitelist of hostname patterns applied at the DNS proxy layer.
///
/// When enabled, only DNS queries for hostnames matching at least one pattern are
/// forwarded upstream. Blocked queries receive NXDOMAIN immediately.
pub struct EgressFilter {
    enabled: bool,
    patterns: Vec<Regex>,
}

impl EgressFilter {
    /// Build a filter from merged destination regex strings.
    ///
    /// # Panics
    ///
    /// Panics if any pattern is invalid (patterns must be validated at config load time).
    #[must_use]
    pub fn new(enabled: bool, patterns: &[String]) -> Self {
        let compiled = patterns
            .iter()
            .map(|pattern| {
                Regex::new(pattern).unwrap_or_else(|error| {
                    panic!("invalid egress pattern {pattern:?}: {error}");
                })
            })
            .collect();
        Self {
            enabled,
            patterns: compiled,
        }
    }

    /// Returns `true` when egress filtering is disabled or the hostname matches a pattern.
    #[must_use]
    pub fn is_hostname_allowed(&self, hostname: &str) -> bool {
        if !self.enabled {
            return true;
        }
        let hostname = hostname.trim_end_matches('.');
        self.patterns
            .iter()
            .any(|pattern| pattern.is_match(hostname))
    }

    /// Whether filtering is active.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_allows_all() {
        let filter = EgressFilter::new(false, &["blocked\\.com$".to_string()]);
        assert!(filter.is_hostname_allowed("blocked.com"));
    }

    #[test]
    fn enabled_matches_pattern() {
        let filter = EgressFilter::new(true, &["httpbin\\.org$".to_string()]);
        assert!(filter.is_hostname_allowed("httpbin.org"));
        assert!(!filter.is_hostname_allowed("example.com"));
    }

    #[test]
    fn trailing_dot_stripped() {
        let filter = EgressFilter::new(true, &["example\\.com$".to_string()]);
        assert!(filter.is_hostname_allowed("example.com."));
    }
}

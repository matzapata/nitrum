//! TTL cache mapping resolved IPs to hostnames from allowed DNS responses.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::constants::IP_CACHE_DEFAULT_TTL_SECS;

struct CacheEntry {
    /// Hostname that produced this IP via an allowed DNS response.
    hostname: String,
    /// When this entry expires.
    expires: Instant,
}

/// Thread-safe IP→hostname cache used by the TCP transparent proxy.
pub struct IpCache {
    entries: Mutex<HashMap<IpAddr, CacheEntry>>,
    default_ttl: Duration,
}

impl IpCache {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            default_ttl: Duration::from_secs(IP_CACHE_DEFAULT_TTL_SECS),
        }
    }

    /// Record an IP resolved for an allowed hostname.
    pub fn insert(&self, ip: IpAddr, hostname: String, ttl_secs: u32) {
        let ttl = if ttl_secs == 0 {
            self.default_ttl
        } else {
            Duration::from_secs(u64::from(ttl_secs))
        };
        let mut guard = self.entries.lock().expect("ip cache lock");
        guard.insert(
            ip,
            CacheEntry {
                hostname,
                expires: Instant::now() + ttl,
            },
        );
    }

    /// Returns the cached hostname when the entry exists and has not expired.
    #[must_use]
    pub fn get_hostname(&self, ip: IpAddr) -> Option<String> {
        let mut guard = self.entries.lock().expect("ip cache lock");
        let entry = guard.get(&ip)?;
        if entry.expires <= Instant::now() {
            guard.remove(&ip);
            return None;
        }
        Some(entry.hostname.clone())
    }
}

impl Default for IpCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;
    use std::thread;
    use std::time::Duration;

    use super::*;

    #[test]
    fn insert_and_lookup() {
        let cache = IpCache::new();
        let ip = IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4));
        cache.insert(ip, "example.com".to_string(), 60);
        assert_eq!(cache.get_hostname(ip).as_deref(), Some("example.com"));
    }

    #[test]
    fn expired_entry_removed() {
        let cache = IpCache {
            entries: Mutex::new(HashMap::new()),
            default_ttl: Duration::from_millis(1),
        };
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        cache.insert(ip, "short.example".to_string(), 0);
        thread::sleep(Duration::from_millis(5));
        assert!(cache.get_hostname(ip).is_none());
    }
}

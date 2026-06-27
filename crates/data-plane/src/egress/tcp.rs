//! Transparent TCP proxy using `SO_ORIGINAL_DST` and iptables REDIRECT.

use std::collections::HashSet;
use std::mem::size_of;
use std::net::{IpAddr, SocketAddr, SocketAddrV4};
use std::os::unix::io::AsRawFd;
use std::sync::Arc;

use tokio::io;
use tokio::net::{TcpListener, TcpSocket, TcpStream};
use tracing::{debug, info, warn};

use super::constants::{EGRESS_BYPASS_SOURCE_IP, EGRESS_SOCKET_MARK, TCP_PROXY_PORT};
use super::filter::EgressFilter;
use super::ip_cache::IpCache;
use super::platform;

const SOL_IP: libc::c_int = 0;
const SO_ORIGINAL_DST: libc::c_int = 80;
const SO_MARK: libc::c_int = 36;

async fn connect_upstream(dst: SocketAddrV4, mark: u32) -> io::Result<TcpStream> {
    let socket = TcpSocket::new_v4()?;
    // SO_MARK is a core socket option (works even where the `xt_mark` match is absent); it lets
    // the `-m mark` RETURN rule exclude this socket on full kernels (local Compose).
    set_socket_mark(socket.as_raw_fd(), mark)?;
    // Source-IP bypass: bind to the dedicated egress IP so the redirect NAT excludes this upstream
    // via a core `-s` RETURN rule. Best-effort: absent in environments without the bypass address.
    bind_bypass_source(&socket);
    socket.connect(SocketAddr::V4(dst)).await
}

/// Bind `socket` to the egress bypass source IP (port 0) so transparent-redirect NAT skips it.
///
/// Failures are ignored: the address only exists on the enclave TAP fabric, and other environments
/// rely on the `SO_MARK` bypass instead.
fn bind_bypass_source(socket: &TcpSocket) {
    if let Ok(ip) = EGRESS_BYPASS_SOURCE_IP.parse() {
        let _ = socket.bind(SocketAddr::V4(SocketAddrV4::new(ip, 0)));
    }
}

fn set_socket_mark(fd: std::os::unix::io::RawFd, mark: u32) -> io::Result<()> {
    let rc = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            SO_MARK,
            &mark as *const u32 as *const libc::c_void,
            size_of::<u32>() as libc::socklen_t,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn get_original_dst(stream: &TcpStream) -> io::Result<SocketAddrV4> {
    let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut len = size_of::<libc::sockaddr_in>() as libc::socklen_t;

    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            SOL_IP,
            SO_ORIGINAL_DST,
            &raw mut addr as *mut _ as *mut libc::c_void,
            &raw mut len,
        )
    };

    if rc != 0 {
        return Err(io::Error::last_os_error());
    }

    let ip = std::net::Ipv4Addr::from(u32::from_be(addr.sin_addr.s_addr));
    let port = u16::from_be(addr.sin_port);
    Ok(SocketAddrV4::new(ip, port))
}

/// Returns whether a redirected connection to `original_dst` should be relayed.
#[must_use]
pub fn original_dst_allowed(
    ip: IpAddr,
    platform_ips: &HashSet<IpAddr>,
    ip_cache: &IpCache,
    filter: &EgressFilter,
) -> bool {
    platform::is_ip_allowed(ip, platform_ips, ip_cache, filter)
}

async fn handle_tcp_connection(
    mut client: TcpStream,
    client_addr: SocketAddr,
    filter: Arc<EgressFilter>,
    ip_cache: Arc<IpCache>,
    platform_ips: Arc<HashSet<IpAddr>>,
) {
    let original_dst = match get_original_dst(&client) {
        Ok(dst) => dst,
        Err(error) => {
            warn!(%client_addr, %error, "egress tcp: failed to read original destination");
            return;
        }
    };

    let ip = IpAddr::V4(*original_dst.ip());
    if !original_dst_allowed(ip, &platform_ips, &ip_cache, &filter) {
        warn!(
            %client_addr,
            destination = %original_dst,
            "egress tcp: blocked by whitelist"
        );
        return;
    }

    debug!(
        %client_addr,
        destination = %original_dst,
        "egress tcp: forwarding connection"
    );

    let mut upstream = match connect_upstream(original_dst, EGRESS_SOCKET_MARK).await {
        Ok(stream) => stream,
        Err(error) => {
            warn!(destination = %original_dst, %error, "egress tcp: upstream connect failed");
            return;
        }
    };

    match io::copy_bidirectional(&mut client, &mut upstream).await {
        Ok((from_client, from_upstream)) => {
            debug!(
                destination = %original_dst,
                bytes_from_client = from_client,
                bytes_from_upstream = from_upstream,
                "egress tcp: connection closed"
            );
        }
        Err(error) => {
            warn!(destination = %original_dst, %error, "egress tcp: relay error");
        }
    }
}

/// Bind the transparent TCP proxy on `0.0.0.0:{TCP_PROXY_PORT}`.
pub async fn bind_tcp_proxy() -> anyhow::Result<TcpListener> {
    let listen_addr = format!("0.0.0.0:{TCP_PROXY_PORT}");
    let listener = TcpListener::bind(&listen_addr).await?;
    info!(port = TCP_PROXY_PORT, "egress TCP proxy bound");
    Ok(listener)
}

/// Accept redirected connections on `listener` until an unrecoverable error occurs.
pub async fn serve_tcp_proxy(
    listener: TcpListener,
    filter: Arc<EgressFilter>,
    ip_cache: Arc<IpCache>,
    platform_ips: Arc<HashSet<IpAddr>>,
) -> anyhow::Result<()> {
    info!(port = TCP_PROXY_PORT, "egress TCP proxy serving");

    loop {
        let (stream, addr) = listener.accept().await?;
        let filter = Arc::clone(&filter);
        let ip_cache = Arc::clone(&ip_cache);
        let platform_ips = Arc::clone(&platform_ips);
        tokio::spawn(async move {
            handle_tcp_connection(stream, addr, filter, ip_cache, platform_ips).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    #[test]
    fn platform_ip_allowed_without_cache() {
        let filter = EgressFilter::new(true, &[]);
        let mut platform_ips = HashSet::new();
        let ip = IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254));
        platform_ips.insert(ip);
        let cache = IpCache::new();
        assert!(original_dst_allowed(ip, &platform_ips, &cache, &filter));
    }

    #[test]
    fn unknown_ip_denied_when_filter_enabled() {
        let filter = EgressFilter::new(true, &["allowed\\.com$".to_string()]);
        let platform_ips = HashSet::new();
        let cache = IpCache::new();
        let ip = IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34));
        assert!(!original_dst_allowed(ip, &platform_ips, &cache, &filter));
    }

    #[test]
    fn cached_ip_allowed_when_hostname_matches() {
        let filter = EgressFilter::new(true, &["example\\.com$".to_string()]);
        let platform_ips = HashSet::new();
        let cache = IpCache::new();
        let ip = IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34));
        cache.insert(ip, "example.com".to_string(), 60);
        assert!(original_dst_allowed(ip, &platform_ips, &cache, &filter));
    }
}

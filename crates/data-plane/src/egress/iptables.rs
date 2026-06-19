//! iptables NAT rules for DNS and TCP egress redirection.

use std::net::{IpAddr, SocketAddr};
use std::process::Command;

use tracing::{info, warn};

use super::constants::{DNS_PROXY_PORT, EGRESS_SOCKET_MARK, IPTABLES_CHAIN, TCP_PROXY_PORT};

/// Install idempotent OUTPUT NAT rules redirecting DNS and TCP through the egress proxies.
pub fn install(upstream_dns: SocketAddr) -> anyhow::Result<()> {
    if !chain_exists()? {
        run_iptables(&["-t", "nat", "-N", IPTABLES_CHAIN])?;
    } else {
        run_iptables(&["-t", "nat", "-F", IPTABLES_CHAIN])?;
    }

    run_iptables(&[
        "-t",
        "nat",
        "-A",
        IPTABLES_CHAIN,
        "-p",
        "udp",
        "--dport",
        "53",
        "-d",
        "127.0.0.1",
        "-j",
        "RETURN",
    ])?;
    run_iptables(&[
        "-t",
        "nat",
        "-A",
        IPTABLES_CHAIN,
        "-p",
        "udp",
        "--dport",
        "53",
        "-d",
        &format_upstream_ip(upstream_dns.ip()),
        "-j",
        "RETURN",
    ])?;
    run_iptables(&[
        "-t",
        "nat",
        "-A",
        IPTABLES_CHAIN,
        "-p",
        "udp",
        "--dport",
        "53",
        "-j",
        "REDIRECT",
        "--to-ports",
        &DNS_PROXY_PORT.to_string(),
    ])?;

    run_iptables(&[
        "-t",
        "nat",
        "-A",
        IPTABLES_CHAIN,
        "-d",
        "127.0.0.0/8",
        "-j",
        "RETURN",
    ])?;
    run_iptables(&[
        "-t",
        "nat",
        "-A",
        IPTABLES_CHAIN,
        "-d",
        "192.168.127.0/24",
        "-j",
        "RETURN",
    ])?;
    run_iptables(&[
        "-t",
        "nat",
        "-A",
        IPTABLES_CHAIN,
        "-p",
        "tcp",
        "-m",
        "mark",
        "--mark",
        &EGRESS_SOCKET_MARK.to_string(),
        "-j",
        "RETURN",
    ])?;
    run_iptables(&[
        "-t",
        "nat",
        "-A",
        IPTABLES_CHAIN,
        "-p",
        "tcp",
        "-j",
        "REDIRECT",
        "--to-ports",
        &TCP_PROXY_PORT.to_string(),
    ])?;

    if !chain_jump_exists()? {
        run_iptables(&["-t", "nat", "-A", "OUTPUT", "-j", IPTABLES_CHAIN])?;
    }

    info!("egress iptables rules installed");
    Ok(())
}

fn format_upstream_ip(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
    }
}

fn chain_exists() -> anyhow::Result<bool> {
    let status = Command::new("iptables")
        .args(["-t", "nat", "-L", IPTABLES_CHAIN])
        .status()?;
    Ok(status.success())
}

fn chain_jump_exists() -> anyhow::Result<bool> {
    let output = Command::new("iptables")
        .args(["-t", "nat", "-C", "OUTPUT", "-j", IPTABLES_CHAIN])
        .output()?;
    Ok(output.status.success())
}

fn run_iptables(args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new("iptables").args(args).status()?;
    if status.success() {
        return Ok(());
    }
    let code = status.code().unwrap_or(-1);
    anyhow::bail!("iptables {args:?} failed with exit code {code}")
}

/// Remove the egress chain jump and flush custom rules (best-effort on shutdown).
pub fn teardown() {
    if chain_jump_exists().unwrap_or(false) {
        let _ = Command::new("iptables")
            .args(["-t", "nat", "-D", "OUTPUT", "-j", IPTABLES_CHAIN])
            .status();
    }
    if chain_exists().unwrap_or(false) {
        let _ = Command::new("iptables")
            .args(["-t", "nat", "-F", IPTABLES_CHAIN])
            .status();
        let _ = Command::new("iptables")
            .args(["-t", "nat", "-X", IPTABLES_CHAIN])
            .status();
    }
    info!("egress iptables rules removed");
}

/// Log guidance when iptables fails (typically missing `NET_ADMIN`).
pub fn log_missing_capability(error: &anyhow::Error) {
    warn!(
        error = %error,
        "egress iptables setup failed; ensure the container has CAP_NET_ADMIN when `[egress].enabled` is true"
    );
}

//! iptables NAT rules for DNS and TCP egress redirection.
//!
//! Uses the legacy xtables backend (`iptables-legacy` on Alpine). Nitro enclave
//! guests typically lack a working nf_tables path, so the legacy binary is preferred
//! when present (see [`iptables_bin`]).

use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::process::Command;

use tracing::{info, warn};

use super::constants::{
    DNS_PROXY_PORT, EGRESS_BYPASS_SOURCE_IP, EGRESS_BYPASS_TAP_DEVICE, EGRESS_IMDS_BYPASS_IP,
    EGRESS_SOCKET_MARK, IPTABLES_CHAIN, TCP_PROXY_PORT,
};

/// Prefer the legacy xtables binary when present (Nitro EIF runtime image).
///
/// Alpine's `iptables-legacy` package installs `/sbin/iptables-legacy`; some images also
/// expose `/usr/sbin/iptables-legacy`. Fall back to `iptables` on `PATH` otherwise.
fn iptables_bin() -> &'static str {
    const LEGACY_CANDIDATES: [&str; 2] = ["/sbin/iptables-legacy", "/usr/sbin/iptables-legacy"];
    LEGACY_CANDIDATES
        .into_iter()
        .find(|p| Path::new(p).exists())
        .unwrap_or("iptables")
}

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
    // IMDS: do not transparent-proxy (platform allowlist still permits it).
    run_iptables(&[
        "-t",
        "nat",
        "-A",
        IPTABLES_CHAIN,
        "-d",
        EGRESS_IMDS_BYPASS_IP,
        "-j",
        "RETURN",
    ])?;

    install_loop_prevention();

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

/// Install RETURN rules that exclude the egress proxy's own upstream sockets from re-redirection.
///
/// Two complementary, best-effort mechanisms are used so a single missing kernel feature does not
/// disable egress:
/// - Source-IP match (`-s`, always built in): excludes upstream sockets bound to the dedicated
///   bypass address; portable to Nitro enclave kernels (which boot `nomodules`, lacking `xt_mark`).
/// - Socket-mark match (`-m mark`): excludes `SO_MARK`-tagged upstream sockets on kernels that build
///   in `xt_mark` (e.g. local Compose).
///
/// At least one must apply at runtime to prevent a redirect loop; both are attempted.
fn install_loop_prevention() {
    add_bypass_source_ip();

    let source_rule_ok = try_run_iptables(&[
        "-t",
        "nat",
        "-A",
        IPTABLES_CHAIN,
        "-s",
        EGRESS_BYPASS_SOURCE_IP,
        "-j",
        "RETURN",
    ]);

    let mark_rule_ok = try_run_iptables(&[
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
    ]);

    if !source_rule_ok && !mark_rule_ok {
        warn!(
            "egress: no loop-prevention RETURN rule could be installed; \
             upstream connections may be re-redirected"
        );
    }
}

/// Attach the egress bypass source IP to the TAP device (best-effort).
///
/// Only present on the Nitro enclave network fabric; ignored elsewhere.
fn add_bypass_source_ip() {
    let _ = Command::new("ip")
        .args([
            "addr",
            "add",
            &format!("{EGRESS_BYPASS_SOURCE_IP}/32"),
            "dev",
            EGRESS_BYPASS_TAP_DEVICE,
        ])
        .status();
}

/// Detach the egress bypass source IP from the TAP device (best-effort, mirrors install).
fn del_bypass_source_ip() {
    let _ = Command::new("ip")
        .args([
            "addr",
            "del",
            &format!("{EGRESS_BYPASS_SOURCE_IP}/32"),
            "dev",
            EGRESS_BYPASS_TAP_DEVICE,
        ])
        .status();
}

fn format_upstream_ip(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
    }
}

fn chain_exists() -> anyhow::Result<bool> {
    let status = Command::new(iptables_bin())
        .args(["-t", "nat", "-L", IPTABLES_CHAIN])
        .status()?;
    Ok(status.success())
}

fn chain_jump_exists() -> anyhow::Result<bool> {
    let output = Command::new(iptables_bin())
        .args(["-t", "nat", "-C", "OUTPUT", "-j", IPTABLES_CHAIN])
        .output()?;
    Ok(output.status.success())
}

fn run_iptables(args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new(iptables_bin()).args(args).status()?;
    if status.success() {
        return Ok(());
    }
    let code = status.code().unwrap_or(-1);
    anyhow::bail!("iptables {args:?} failed with exit code {code}")
}

/// Run an iptables command, returning whether it succeeded (no error propagation).
///
/// For optional rules that depend on kernel features which may not be built in.
fn try_run_iptables(args: &[&str]) -> bool {
    match Command::new(iptables_bin()).args(args).status() {
        Ok(status) => status.success(),
        Err(error) => {
            warn!(?args, %error, "egress: optional iptables rule could not run");
            false
        }
    }
}

/// Remove the egress chain jump and flush custom rules (best-effort on shutdown).
pub fn teardown() {
    let bin = iptables_bin();
    if chain_jump_exists().unwrap_or(false) {
        let _ = Command::new(bin)
            .args(["-t", "nat", "-D", "OUTPUT", "-j", IPTABLES_CHAIN])
            .status();
    }
    if chain_exists().unwrap_or(false) {
        let _ = Command::new(bin)
            .args(["-t", "nat", "-F", IPTABLES_CHAIN])
            .status();
        let _ = Command::new(bin)
            .args(["-t", "nat", "-X", IPTABLES_CHAIN])
            .status();
    }
    del_bypass_source_ip();
    info!("egress iptables rules removed");
}

/// Log guidance when iptables NAT setup fails.
pub fn log_missing_capability(error: &anyhow::Error) {
    warn!(
        error = %error,
        "egress iptables setup failed; Nitro EIF images need iptables-legacy (not nf_tables). \
         Local Compose needs CAP_NET_ADMIN on the enclave service."
    );
}

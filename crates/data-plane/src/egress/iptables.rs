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

/// IPv6 counterpart of [`iptables_bin`], preferring the legacy xtables backend when present.
fn ip6tables_bin() -> &'static str {
    const LEGACY_CANDIDATES: [&str; 2] = ["/sbin/ip6tables-legacy", "/usr/sbin/ip6tables-legacy"];
    LEGACY_CANDIDATES
        .into_iter()
        .find(|p| Path::new(p).exists())
        .unwrap_or("ip6tables")
}

/// Install idempotent OUTPUT rules enforcing egress through the proxies.
///
/// Three stages, applied in order:
/// - NAT redirects (IPv4): DNS to the DNS proxy, TCP to the TCP proxy. Load-bearing; fatal on
///   failure since egress is non-functional without it.
/// - Filter default-drop (IPv4, best-effort): block non-DNS UDP that the NAT stage does not
///   capture. Defense-in-depth; a missing `filter` table must not brick egress.
/// - Filter default-drop (IPv6, best-effort): block all v6 egress since no v6 proxy exists.
pub fn install(upstream_dns: SocketAddr, collector_bypass: Option<IpAddr>) -> anyhow::Result<()> {
    install_nat_redirects(upstream_dns, collector_bypass)?;
    install_udp_filter();
    install_ipv6_drop();
    info!("egress iptables rules installed");
    Ok(())
}

/// Install the IPv4 NAT chain redirecting DNS and TCP through the egress proxies.
///
/// `collector_bypass`, when an IPv4 address, is excluded from the TCP redirect so OTLP telemetry
/// reaches the host collector directly (mirrors the IMDS bypass).
fn install_nat_redirects(
    upstream_dns: SocketAddr,
    collector_bypass: Option<IpAddr>,
) -> anyhow::Result<()> {
    ensure_chain("nat")?;

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

    // OTLP collector: do not transparent-proxy IP-based telemetry egress (mirrors IMDS).
    if let Some(IpAddr::V4(collector)) = collector_bypass {
        run_iptables(&[
            "-t",
            "nat",
            "-A",
            IPTABLES_CHAIN,
            "-d",
            &format!("{collector}/32"),
            "-j",
            "RETURN",
        ])?;
    }

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

    ensure_chain_jump("nat")?;

    Ok(())
}

/// Default-drop non-DNS UDP egress in the IPv4 filter table (best-effort).
///
/// The NAT stage only redirects UDP/53 to the DNS proxy and TCP to the TCP proxy; any other
/// UDP would otherwise leave the runtime unmediated (raw UDP, QUIC/HTTP3, DNS tunneling over a
/// non-53 port). Dropping it closes that exfiltration channel and forces clients onto TCP/443,
/// which the TCP proxy and allowlist govern. Two flows are exempt:
/// - Loopback (`-o lo`): the proxies' redirected traffic and local DNS proxy delivery.
/// - UDP/53: the DNS proxy's own upstream resolver queries.
///
/// Best-effort: minimal Nitro enclave kernels boot `nomodules` and may not provide the `filter`
/// table. A missing table only logs a warning (the drop is not enforced) rather than failing
/// egress init, which would otherwise crash-loop the enclave.
fn install_udp_filter() {
    if !ensure_filter_chain() {
        warn!("egress: filter table unavailable; non-DNS UDP egress is NOT dropped");
        return;
    }

    try_run_iptables(&[
        "-t",
        "filter",
        "-A",
        IPTABLES_CHAIN,
        "-o",
        "lo",
        "-j",
        "RETURN",
    ]);
    try_run_iptables(&[
        "-t",
        "filter",
        "-A",
        IPTABLES_CHAIN,
        "-p",
        "udp",
        "--dport",
        "53",
        "-j",
        "RETURN",
    ]);
    if !try_run_iptables(&[
        "-t",
        "filter",
        "-A",
        IPTABLES_CHAIN,
        "-p",
        "udp",
        "-j",
        "DROP",
    ]) {
        warn!("egress: could not install non-DNS UDP drop rule; UDP egress is NOT filtered");
    }

    if !chain_jump_exists("filter").unwrap_or(false) {
        try_run_iptables(&["-t", "filter", "-A", "OUTPUT", "-j", IPTABLES_CHAIN]);
    }
}

/// Create or flush the egress chain in the IPv4 filter table (best-effort).
///
/// Returns whether the chain is present afterwards; `false` means the `filter` table itself is
/// unavailable on this kernel.
fn ensure_filter_chain() -> bool {
    if chain_exists("filter").unwrap_or(false) {
        return try_run_iptables(&["-t", "filter", "-F", IPTABLES_CHAIN]);
    }
    try_run_iptables(&["-t", "filter", "-N", IPTABLES_CHAIN])
}

/// Drop all IPv6 egress as defense-in-depth (best-effort).
///
/// Every NAT redirect and filter rule above is IPv4-only. If the runtime ever gains IPv6
/// connectivity, TCP/UDP over v6 would bypass the proxies and allowlist entirely. There is no
/// v6 proxy, so all v6 egress is dropped except loopback. Best-effort: enclave kernels may lack
/// `ip6tables` or IPv6 support, in which case there is no v6 path to bypass and nothing to do.
fn install_ipv6_drop() {
    if ip6_chain_exists() {
        try_run_ip6tables(&["-t", "filter", "-F", IPTABLES_CHAIN]);
    } else if !try_run_ip6tables(&["-t", "filter", "-N", IPTABLES_CHAIN]) {
        warn!("egress: ip6tables unavailable; skipping IPv6 egress drop");
        return;
    }

    try_run_ip6tables(&[
        "-t",
        "filter",
        "-A",
        IPTABLES_CHAIN,
        "-o",
        "lo",
        "-j",
        "RETURN",
    ]);
    try_run_ip6tables(&["-t", "filter", "-A", IPTABLES_CHAIN, "-j", "DROP"]);

    if !ip6_chain_jump_exists() {
        try_run_ip6tables(&["-t", "filter", "-A", "OUTPUT", "-j", IPTABLES_CHAIN]);
    }
}

/// Create the egress chain in `table`, or flush it if it already exists (idempotent install).
fn ensure_chain(table: &str) -> anyhow::Result<()> {
    if chain_exists(table)? {
        run_iptables(&["-t", table, "-F", IPTABLES_CHAIN])?;
    } else {
        run_iptables(&["-t", table, "-N", IPTABLES_CHAIN])?;
    }
    Ok(())
}

/// Jump OUTPUT into the egress chain in `table` unless the jump is already present.
fn ensure_chain_jump(table: &str) -> anyhow::Result<()> {
    if !chain_jump_exists(table)? {
        run_iptables(&["-t", table, "-A", "OUTPUT", "-j", IPTABLES_CHAIN])?;
    }
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

fn chain_exists(table: &str) -> anyhow::Result<bool> {
    let status = Command::new(iptables_bin())
        .args(["-t", table, "-L", IPTABLES_CHAIN])
        .status()?;
    Ok(status.success())
}

fn chain_jump_exists(table: &str) -> anyhow::Result<bool> {
    let output = Command::new(iptables_bin())
        .args(["-t", table, "-C", "OUTPUT", "-j", IPTABLES_CHAIN])
        .output()?;
    Ok(output.status.success())
}

/// Whether the egress chain exists in the IPv6 filter table (best-effort; false on any error).
fn ip6_chain_exists() -> bool {
    Command::new(ip6tables_bin())
        .args(["-t", "filter", "-L", IPTABLES_CHAIN])
        .status()
        .is_ok_and(|s| s.success())
}

/// Whether OUTPUT already jumps into the egress chain in the IPv6 filter table (best-effort).
fn ip6_chain_jump_exists() -> bool {
    Command::new(ip6tables_bin())
        .args(["-t", "filter", "-C", "OUTPUT", "-j", IPTABLES_CHAIN])
        .output()
        .is_ok_and(|o| o.status.success())
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

/// Run an ip6tables command, returning whether it succeeded (no error propagation).
///
/// All IPv6 rules are best-effort: enclave kernels may ship without `ip6tables` or IPv6 support.
fn try_run_ip6tables(args: &[&str]) -> bool {
    match Command::new(ip6tables_bin()).args(args).status() {
        Ok(status) => status.success(),
        Err(error) => {
            warn!(?args, %error, "egress: optional ip6tables rule could not run");
            false
        }
    }
}

/// Remove the egress chain jumps and flush custom rules (best-effort on shutdown).
pub fn teardown() {
    teardown_table("nat");
    teardown_table("filter");
    teardown_ipv6();
    del_bypass_source_ip();
    info!("egress iptables rules removed");
}

/// Remove the OUTPUT jump and delete the egress chain in the given IPv4 `table` (best-effort).
fn teardown_table(table: &str) {
    let bin = iptables_bin();
    if chain_jump_exists(table).unwrap_or(false) {
        let _ = Command::new(bin)
            .args(["-t", table, "-D", "OUTPUT", "-j", IPTABLES_CHAIN])
            .status();
    }
    if chain_exists(table).unwrap_or(false) {
        let _ = Command::new(bin)
            .args(["-t", table, "-F", IPTABLES_CHAIN])
            .status();
        let _ = Command::new(bin)
            .args(["-t", table, "-X", IPTABLES_CHAIN])
            .status();
    }
}

/// Remove the OUTPUT jump and delete the egress chain in the IPv6 filter table (best-effort).
fn teardown_ipv6() {
    let bin = ip6tables_bin();
    if ip6_chain_jump_exists() {
        let _ = Command::new(bin)
            .args(["-t", "filter", "-D", "OUTPUT", "-j", IPTABLES_CHAIN])
            .status();
    }
    if ip6_chain_exists() {
        let _ = Command::new(bin)
            .args(["-t", "filter", "-F", IPTABLES_CHAIN])
            .status();
        let _ = Command::new(bin)
            .args(["-t", "filter", "-X", IPTABLES_CHAIN])
            .status();
    }
}

/// Log guidance when iptables NAT setup fails.
pub fn log_missing_capability(error: &anyhow::Error) {
    warn!(
        error = %error,
        "egress iptables setup failed; Nitro EIF images need iptables-legacy (not nf_tables). \
         Local Compose needs CAP_NET_ADMIN on the enclave service."
    );
}

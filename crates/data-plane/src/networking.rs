use std::process::Command;
use std::sync::OnceLock;

use tracing::info;

use crate::constants::{DATAPLANE_UID, TCP_PROXY_PORT};

const TUN_DEVICE_NAME: &str = "nitrum0";
const TUN_DEVICE_CIDR: &str = "10.0.0.1/24";
const TUN_DEVICE_GATEWAY: &str = "10.0.0.1";
static TUN_FD: OnceLock<libc::c_int> = OnceLock::new();
const IFF_TUN: libc::c_int = 0x0001;
const IFF_NO_PI: libc::c_int = 0x1000;
const TUNSETIFF: libc::c_ulong = 0x4004_54ca;

pub fn setup() {
    cleanup_legacy_nat_rules();
    setup_dns_proxy();
    setup_tcp_redirect_via_tun();
}

fn cleanup_legacy_nat_rules() {
    // Older builds installed a catch-all TCP redirect through this custom chain.
    // Remove it to avoid recursive interception (proxy traffic to 172.20.0.10:8181
    // being captured back into itself).
    let legacy_output_jump = vec![
        "-p".to_string(),
        "tcp".to_string(),
        "-j".to_string(),
        "ENCLAVE_PROXY".to_string(),
    ];
    remove_nat_output_rule(&legacy_output_jump);

    // Remove older direct HTTPS redirect rules now replaced by TUN interception.
    let legacy_tcp_rule = vec![
        "-p".to_string(),
        "tcp".to_string(),
        "--dport".to_string(),
        "443".to_string(),
        "-m".to_string(),
        "owner".to_string(),
        "!".to_string(),
        "--uid-owner".to_string(),
        DATAPLANE_UID.to_string(),
        "!".to_string(),
        "-d".to_string(),
        "127.0.0.1/32".to_string(),
        "-j".to_string(),
        "DNAT".to_string(),
        "--to-destination".to_string(),
        format!("127.0.0.1:{TCP_PROXY_PORT}"),
    ];
    remove_nat_output_rule(&legacy_tcp_rule);

    // Best-effort cleanup of the old chain. Ignore failures if chain does not exist.
    let _ = Command::new("iptables")
        .args(["-t", "nat", "-F", "ENCLAVE_PROXY"])
        .status();
    let _ = Command::new("iptables")
        .args(["-t", "nat", "-X", "ENCLAVE_PROXY"])
        .status();
}

fn setup_dns_proxy() {
    // Redirect outbound DNS from the app into the local DNS proxy.
    let dns_rules = [
        vec![
            "-p".to_string(),
            "udp".to_string(),
            "--dport".to_string(),
            "53".to_string(),
            "-m".to_string(),
            "owner".to_string(),
            "!".to_string(),
            "--uid-owner".to_string(),
            DATAPLANE_UID.to_string(),
            "-j".to_string(),
            "DNAT".to_string(),
            "--to-destination".to_string(),
            "127.0.0.1:53".to_string(),
        ],
        vec![
            "-p".to_string(),
            "tcp".to_string(),
            "--dport".to_string(),
            "53".to_string(),
            "-m".to_string(),
            "owner".to_string(),
            "!".to_string(),
            "--uid-owner".to_string(),
            DATAPLANE_UID.to_string(),
            "-j".to_string(),
            "DNAT".to_string(),
            "--to-destination".to_string(),
            "127.0.0.1:53".to_string(),
        ],
    ];

    for rule in &dns_rules {
        ensure_nat_output_rule(rule);
        info!(rule = ?rule, "installed dns iptables redirect");
    }
}

fn setup_tcp_redirect_via_tun() {
        let _ = TUN_FD.get_or_init(|| create_tun(TUN_DEVICE_NAME));
        run_ip(["addr", "replace", TUN_DEVICE_CIDR, "dev", TUN_DEVICE_NAME]);
        run_ip(["link", "set", "dev", TUN_DEVICE_NAME, "up"]);
        run_ip([
            "route",
            "replace",
            "default",
            "via",
            TUN_DEVICE_GATEWAY,
            "dev",
            TUN_DEVICE_NAME,
        ]);
        info!(
            device = TUN_DEVICE_NAME,
            cidr = TUN_DEVICE_CIDR,
            "installed tcp redirect via TUN"
        );
}

fn run_ip<const N: usize>(args: [&str; N]) {
    let status = Command::new("ip")
        .args(args)
        .status()
        .unwrap_or_else(|e| panic!("failed to run ip {:?}: {e}", args));
    assert!(
        status.success(),
        "ip command failed for args {:?}, exit {:?}",
        args,
        status.code()
    );
}

fn create_tun(name: &str) -> libc::c_int {
    assert!(!name.is_empty(), "TUN device name cannot be empty");
    assert!(
        name.len() < libc::IFNAMSIZ,
        "TUN device name '{}' exceeds IFNAMSIZ ({})",
        name,
        libc::IFNAMSIZ
    );

    let fd = unsafe { libc::open(c"/dev/net/tun".as_ptr(), libc::O_RDWR) };
    assert!(
        fd >= 0,
        "open(/dev/net/tun) failed: {}",
        std::io::Error::last_os_error()
    );

    let mut ifr: libc::ifreq = unsafe { std::mem::zeroed() };
    unsafe {
        std::ptr::copy_nonoverlapping(
            name.as_ptr() as *const libc::c_char,
            ifr.ifr_name.as_mut_ptr() as *mut libc::c_char,
            name.len(),
        );
        ifr.ifr_ifru.ifru_flags = (IFF_TUN | IFF_NO_PI) as i16;
    }

    let rc = unsafe { libc::ioctl(fd, TUNSETIFF as _, &ifr) };
    assert_eq!(
        rc,
        0,
        "ioctl(TUNSETIFF) failed for {}: {}",
        name,
        std::io::Error::last_os_error()
    );

    fd
}

fn ensure_nat_output_rule(rule_spec: &[String]) {
    let mut check_args = vec![
        "-t".to_string(),
        "nat".to_string(),
        "-C".to_string(),
        "OUTPUT".to_string(),
    ];
    check_args.extend(rule_spec.iter().cloned());

    let check_status = Command::new("iptables")
        .args(&check_args)
        .status()
        .unwrap_or_else(|e| panic!("failed to run iptables -C with args {check_args:?}: {e}"));

    if check_status.success() {
        return;
    }
    assert_eq!(
        check_status.code(),
        Some(1),
        "iptables -C failed with unexpected status {:?} for args {:?}",
        check_status.code(),
        check_args
    );

    let mut add_args = vec![
        "-t".to_string(),
        "nat".to_string(),
        "-A".to_string(),
        "OUTPUT".to_string(),
    ];
    add_args.extend(rule_spec.iter().cloned());
    let add_status = Command::new("iptables")
        .args(&add_args)
        .status()
        .unwrap_or_else(|e| panic!("failed to run iptables -A with args {add_args:?}: {e}"));
    assert!(
        add_status.success(),
        "failed to install iptables rule {:?}, status {:?}",
        add_args,
        add_status.code()
    );
}

fn remove_nat_output_rule(rule_spec: &[String]) {
    let mut check_args = vec![
        "-t".to_string(),
        "nat".to_string(),
        "-C".to_string(),
        "OUTPUT".to_string(),
    ];
    check_args.extend(rule_spec.iter().cloned());

    loop {
        let check_status = Command::new("iptables")
            .args(&check_args)
            .status()
            .unwrap_or_else(|e| panic!("failed to run iptables -C with args {check_args:?}: {e}"));

        if !check_status.success() {
            // Rule is absent (status 1) or check failed unexpectedly; stop cleanup.
            break;
        }

        let mut del_args = vec![
            "-t".to_string(),
            "nat".to_string(),
            "-D".to_string(),
            "OUTPUT".to_string(),
        ];
        del_args.extend(rule_spec.iter().cloned());
        let del_status = Command::new("iptables")
            .args(&del_args)
            .status()
            .unwrap_or_else(|e| panic!("failed to run iptables -D with args {del_args:?}: {e}"));
        assert!(
            del_status.success(),
            "failed to delete iptables rule {:?}, status {:?}",
            del_args,
            del_status.code()
        );
    }
}

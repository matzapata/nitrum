//! TAP device creation and Linux network interface configuration inside the enclave.

use super::constants::{
    IFF_NO_PI, IFF_TAP, IMDS_HOST_ROUTE, TAP_DEVICE_NAME, TAP_GATEWAY, TAP_IP_CIDR, TAP_MAC,
    TAP_MTU, TUNSETIFF,
};
use anyhow::{Context, Result};
use std::os::unix::io::{FromRawFd, OwnedFd};
use std::process::Command;
use tracing::info;

/// Brings the loopback interface up so `127.0.0.1` is reachable before TAP setup.
pub(super) fn bring_loopback_up() -> Result<()> {
    info!("bringing loopback interface up");
    run_ip(&["link", "set", "dev", "lo", "up"], &[])
}

/// Opens `/dev/net/tun` and creates a TAP (Ethernet) device with the given name.
pub(super) fn create_tap(name: &str) -> std::io::Result<OwnedFd> {
    assert!(
        !name.is_empty() && name.len() < libc::IFNAMSIZ,
        "TAP device name invalid"
    );

    let fd = unsafe { libc::open(c"/dev/net/tun".as_ptr(), libc::O_RDWR) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }

    let mut ifr: libc::ifreq = unsafe { std::mem::zeroed() };
    unsafe {
        std::ptr::copy_nonoverlapping(
            name.as_ptr(),
            ifr.ifr_name.as_mut_ptr().cast::<u8>(),
            name.len(),
        );
        ifr.ifr_ifru.ifru_flags = IFF_TAP | IFF_NO_PI;
    }

    let rc = unsafe { libc::ioctl(fd, TUNSETIFF as _, &ifr) };
    if rc < 0 {
        let err = std::io::Error::last_os_error();
        unsafe {
            libc::close(fd);
        }
        return Err(err);
    }

    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Assigns MAC, IP, MTU, default route, and an IMDS host route on the TAP interface.
pub(super) fn configure_tap() -> Result<()> {
    run_ip(&["link", "set", "dev", "lo", "up"], &[])?;
    run_ip(
        &["link", "set", "dev", TAP_DEVICE_NAME, "address", TAP_MAC],
        &[],
    )?;
    run_ip(&["addr", "add", TAP_IP_CIDR, "dev", TAP_DEVICE_NAME], &[])?;
    run_ip(
        &["link", "set", "dev", TAP_DEVICE_NAME, "mtu", TAP_MTU],
        &[],
    )?;
    run_ip(&["link", "set", "dev", TAP_DEVICE_NAME, "up"], &[])?;
    run_ip(
        &[
            "route",
            "add",
            "default",
            "via",
            TAP_GATEWAY,
            "dev",
            TAP_DEVICE_NAME,
        ],
        &[],
    )?;
    // Without this, Linux may ARP for 169.254.169.254 on tap0 instead of forwarding to gvproxy.
    run_ip(
        &[
            "route",
            "add",
            IMDS_HOST_ROUTE,
            "via",
            TAP_GATEWAY,
            "dev",
            TAP_DEVICE_NAME,
        ],
        // Exit code 2: route already present (e.g. warm restart).
        &[2],
    )
}

/// Writes a minimal `resolv.conf` pointing DNS queries at the TAP gateway (gvproxy).
pub(super) fn write_resolv_conf() -> Result<()> {
    std::fs::create_dir_all("/run/resolvconf").context("create /run/resolvconf")?;
    std::fs::write("/run/resolvconf/resolv.conf", "nameserver 192.168.127.1\n")
        .context("write /run/resolvconf/resolv.conf")?;
    Ok(())
}

/// Runs `ip` with the given arguments, treating listed exit codes as success.
fn run_ip(args: &[&str], allowed_exit_codes: &[i32]) -> Result<()> {
    let status = Command::new("ip")
        .args(args)
        .status()
        .with_context(|| format!("spawn ip {args:?}"))?;
    if status.success() {
        return Ok(());
    }
    let code = status.code().unwrap_or(-1);
    if allowed_exit_codes.contains(&code) {
        return Ok(());
    }
    anyhow::bail!("ip {args:?} failed with exit code {code}")
}

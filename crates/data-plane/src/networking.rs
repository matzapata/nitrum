//! Enclave network initialization and configuration for the data plane.
//!
//! This module is responsible for establishing the networking stack inside an AWS Nitro enclave.
//! It sets up a virtual TAP (Ethernet) device and manages connectivity to the outside world via
//! VSOCK-based communication with `gvproxy` on the parent EC2 instance. The TAP device is assigned
//! a static IP and MAC address, and built-in routing ensures that traffic such as the Instance
//! Metadata Service (IMDS) is accessible only via secure, isolated host routes.
//!
//! Key responsibilities include:
//! - Creating and configuring a TAP device with static addressing within the enclave.
//! - Establishing host communication using the VSOCK protocol (context ID 3), which connects to
//!   a designated agent (`gvproxy`) that proxies network traffic between the enclave and the host's
//!   network devices.
//! - Enabling communication with external services—including EC2/ECS APIs and telemetry endpoints—over
//!   standard transport protocols, with all egress mediated via the TAP interface and host-side proxy.
//! - Optionally exposing debug or admin features for visibility into network state during development.
//!
//! This setup is crucial for secure, minimal, and auditable networking, enabling data plane components
//! to function in a tightly controlled enclave environment without direct access to host or AWS VPC networks.

use crate::constants::{HOST_PROXY_PORT, VSOCK_CONNECT_MAX_ATTEMPTS, VSOCK_CONNECT_RETRY_INTERVAL};
use anyhow::{Context, Result};
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{debug, error, info, instrument, warn};

/// The name of the TAP network device created inside the enclave.
const TAP_DEVICE_NAME: &str = "tap0";

/// IPv4 address and netmask (CIDR) assigned to the TAP device inside the enclave.
const TAP_IP_CIDR: &str = "192.168.127.2/24";

/// The IPv4 gateway address used for the TAP device's route table.
const TAP_GATEWAY: &str = "192.168.127.1";

/// The IPv4 host route (link-local) for EC2 Instance Metadata Service (IMDS).
/// Traffic to this address is routed via gvproxy.
const IMDS_HOST_ROUTE: &str = "169.254.169.254/32";

/// The MAC address assigned to the TAP device. Used for interface configuration.
const TAP_MAC: &str = "ba:aa:ad:c0:ff:ee";

/// The Maximum Transmission Unit (MTU) for the TAP interface.
const TAP_MTU: &str = "1500";

/// The parent context ID (CID) used for VSOCK communication with the host (always 3 in Nitro enclaves).
const PARENT_CID: u32 = 3;

/// Maximum size (in bytes) for a L2 frame (largest allowed Ethernet frame size).
const MAX_FRAME_SIZE: usize = 65535;

/// Number of bytes used to represent frame length prefix for each transferred frame.
const FRAME_LEN_SIZE: usize = 2;

/// Flag used with TUN/TAP ioctls: designates a TAP (Ethernet) device.
const IFF_TAP: libc::c_short = 0x0002;

/// Flag used with TUN/TAP ioctls: disables packet information prepending (raw Ethernet).
const IFF_NO_PI: libc::c_short = 0x1000;

/// ioctl request code for creating/configuring a TUN/TAP device.
const TUNSETIFF: libc::c_ulong = 0x4004_54ca;

/// Socket domain constant for VSOCK (host/guest communication).
const AF_VSOCK: libc::c_int = 40;

/// Set up enclave networking via TAP device + VSOCK to gvproxy on the host.
///
/// Creates a TAP device, connects to gvproxy over VSOCK, sends the POST /connect
/// handshake, configures the interface (IP, MAC, MTU, gateway, resolv.conf), and
/// spawns two threads that forward L2 frames between the TAP and the VSOCK stream
/// using the 2-byte little-endian length-prefixed protocol.
///
/// Returns once TAP ↔ VSOCK forwarding is running (background threads). Call **before** loading
/// config that needs IMDS/HTTPS egress: the parent must run gvproxy with `-ec2-metadata-access`
/// so `169.254.169.254` (IMDS) is reachable from the guest over TAP.
#[instrument(name = "networking.init", err)]
pub async fn init() -> Result<()> {
    // Enclave/minimal roots often start with `lo` down; without this, `127.0.0.1` returns ENETUNREACH.
    bring_loopback_up().context("bring loopback interface up")?;

    let vsock = connect_vsock_with_retry(HOST_PROXY_PORT)
        .await
        .context("connect to gvproxy via VSOCK")?;

    send_handshake(vsock.as_raw_fd()).context("send POST /connect handshake to gvproxy")?;

    let tap = create_tap(TAP_DEVICE_NAME)
        .with_context(|| format!("create TAP device {TAP_DEVICE_NAME}"))?;

    configure_tap().context("configure TAP interface")?;
    info!(
        device = TAP_DEVICE_NAME,
        ip_cidr = TAP_IP_CIDR,
        gateway = TAP_GATEWAY,
        "TAP interface configured"
    );

    write_resolv_conf().context("write resolv.conf")?;

    start_forwarding(tap.into_raw_fd(), vsock.into_raw_fd());
    Ok(())
}

#[instrument(name = "networking.connect_vsock", fields(port), err)]
async fn connect_vsock_with_retry(port: u32) -> Result<OwnedFd> {
    let mut last_error = None;

    for attempt in 1..=VSOCK_CONNECT_MAX_ATTEMPTS {
        match connect_vsock(port) {
            Ok(fd) => {
                info!(attempt, "connected to gvproxy via VSOCK");
                return Ok(unsafe { OwnedFd::from_raw_fd(fd) });
            }
            Err(error) => {
                if attempt < VSOCK_CONNECT_MAX_ATTEMPTS {
                    debug!(
                        attempt,
                        max_attempts = VSOCK_CONNECT_MAX_ATTEMPTS,
                        error = %error,
                        "VSOCK connect failed, retrying"
                    );
                    tokio::time::sleep(VSOCK_CONNECT_RETRY_INTERVAL).await;
                } else {
                    warn!(
                        attempt,
                        max_attempts = VSOCK_CONNECT_MAX_ATTEMPTS,
                        error = %error,
                        "VSOCK connect failed, giving up"
                    );
                    last_error = Some(error);
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "VSOCK connect failed without a specific error",
        )
    }))
    .with_context(|| {
        format!("gvproxy not reachable on port {port} after {VSOCK_CONNECT_MAX_ATTEMPTS} attempts")
    })
}

#[repr(C)]
struct SockAddrVm {
    family: u16,
    reserved1: u16,
    port: u32,
    cid: u32,
    zero: [u8; 4],
}

fn create_tap(name: &str) -> std::io::Result<OwnedFd> {
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

fn configure_tap() -> Result<()> {
    run_ip(&["link", "set", "dev", "lo", "up"])?;
    run_ip(&["link", "set", "dev", TAP_DEVICE_NAME, "address", TAP_MAC])?;
    run_ip(&["addr", "add", TAP_IP_CIDR, "dev", TAP_DEVICE_NAME])?;
    run_ip(&["link", "set", "dev", TAP_DEVICE_NAME, "mtu", TAP_MTU])?;
    run_ip(&["link", "set", "dev", TAP_DEVICE_NAME, "up"])?;
    run_ip(&[
        "route",
        "add",
        "default",
        "via",
        TAP_GATEWAY,
        "dev",
        TAP_DEVICE_NAME,
    ])?;
    // Without this, Linux may ARP for 169.254.169.254 on tap0 instead of forwarding to gvproxy.
    add_imds_route()
}

fn add_imds_route() -> Result<()> {
    let status = Command::new("ip")
        .args([
            "route",
            "add",
            IMDS_HOST_ROUTE,
            "via",
            TAP_GATEWAY,
            "dev",
            TAP_DEVICE_NAME,
        ])
        .status()
        .context("spawn ip route add for IMDS")?;

    if status.success() || status.code() == Some(2) {
        // Exit code 2: route already present (e.g. warm restart).
        return Ok(());
    }

    let code = status.code().unwrap_or(-1);
    anyhow::bail!("ip route add for IMDS failed with exit code {code}")
}

fn run_ip(args: &[&str]) -> Result<()> {
    let status = Command::new("ip")
        .args(args)
        .status()
        .with_context(|| format!("spawn ip {args:?}"))?;
    if status.success() {
        return Ok(());
    }
    let code = status.code().unwrap_or(-1);
    anyhow::bail!("ip {args:?} failed with exit code {code}")
}

fn bring_loopback_up() -> Result<()> {
    info!("bringing loopback interface up");
    run_ip(&["link", "set", "dev", "lo", "up"])
}

fn write_resolv_conf() -> Result<()> {
    std::fs::create_dir_all("/run/resolvconf").context("create /run/resolvconf")?;
    std::fs::write("/run/resolvconf/resolv.conf", "nameserver 192.168.127.1\n")
        .context("write /run/resolvconf/resolv.conf")?;
    Ok(())
}

fn connect_vsock(port: u32) -> std::io::Result<libc::c_int> {
    let fd = unsafe { libc::socket(AF_VSOCK, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }

    let addr = SockAddrVm {
        family: u16::try_from(AF_VSOCK).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "AF_VSOCK constant does not fit in u16",
            )
        })?,
        reserved1: 0,
        port,
        cid: PARENT_CID,
        zero: [0; 4],
    };

    let rc = unsafe {
        libc::connect(
            fd,
            (&raw const addr).cast::<libc::sockaddr>(),
            libc::socklen_t::try_from(std::mem::size_of::<SockAddrVm>()).map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "SockAddrVm size does not fit in socklen_t",
                )
            })?,
        )
    };

    if rc < 0 {
        let err = std::io::Error::last_os_error();
        unsafe {
            libc::close(fd);
        }
        return Err(err);
    }

    Ok(fd)
}

fn send_handshake(fd: libc::c_int) -> std::io::Result<()> {
    write_all_raw(fd, b"POST /connect HTTP/1.1\r\nHost: \r\n\r\n")
}

fn read_exact_raw(fd: libc::c_int, buf: &mut [u8]) -> std::io::Result<()> {
    let mut pos = 0;
    while pos < buf.len() {
        let n = unsafe {
            libc::read(
                fd,
                buf[pos..].as_mut_ptr().cast::<libc::c_void>(),
                buf.len() - pos,
            )
        };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "connection closed",
            ));
        }
        pos += usize::try_from(n).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "negative read size encountered",
            )
        })?;
    }
    Ok(())
}

fn write_all_raw(fd: libc::c_int, buf: &[u8]) -> std::io::Result<()> {
    let mut pos = 0;
    while pos < buf.len() {
        let n = unsafe {
            libc::write(
                fd,
                buf[pos..].as_ptr().cast::<libc::c_void>(),
                buf.len() - pos,
            )
        };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        pos += usize::try_from(n).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "negative write size encountered",
            )
        })?;
    }
    Ok(())
}

/// Writes all bytes from `bufs` to `fd`, coalescing slices into one `writev` per attempt.
fn write_all_vectored_raw(fd: libc::c_int, bufs: &[&[u8]]) -> std::io::Result<()> {
    let mut head = 0usize;
    let mut offset = 0usize;

    while head < bufs.len() {
        let iovecs: Vec<libc::iovec> = bufs[head..]
            .iter()
            .enumerate()
            .filter_map(|(i, slice)| {
                let slice_offset = if i == 0 { offset } else { 0 };
                if slice_offset >= slice.len() {
                    return None;
                }
                Some(libc::iovec {
                    iov_base: slice[slice_offset..].as_ptr().cast_mut().cast(),
                    iov_len: slice.len() - slice_offset,
                })
            })
            .collect();

        if iovecs.is_empty() {
            break;
        }

        let n = unsafe {
            libc::writev(
                fd,
                iovecs.as_ptr(),
                libc::c_int::try_from(iovecs.len()).map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "too many iovecs for writev",
                    )
                })?,
            )
        };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "writev returned 0",
            ));
        }

        let mut remaining = usize::try_from(n).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "negative write size encountered",
            )
        })?;

        while remaining > 0 {
            let slice_len = bufs[head].len() - offset;
            if remaining < slice_len {
                offset += remaining;
                remaining = 0;
            } else {
                remaining -= slice_len;
                head += 1;
                offset = 0;
            }
        }
    }

    Ok(())
}

/// TAP → VSOCK: read raw Ethernet frames from the TAP device, prepend a
/// 2-byte LE length header, and write to the VSOCK stream.
fn rx_loop(tap_fd: libc::c_int, vsock_fd: libc::c_int, running: &Arc<AtomicBool>) {
    let mut buf = vec![0u8; MAX_FRAME_SIZE];
    while running.load(Ordering::Relaxed) {
        let n = unsafe { libc::read(tap_fd, buf.as_mut_ptr().cast::<libc::c_void>(), buf.len()) };
        if n < 0 {
            error!(error = %std::io::Error::last_os_error(), "TAP read failed");
            break;
        }
        if n == 0 {
            error!("TAP read EOF");
            break;
        }
        let Ok(frame_len) = u16::try_from(n) else {
            error!(bytes_read = n, "TAP frame length exceeds u16 maximum");
            break;
        };
        let len = frame_len.to_le_bytes();
        let frame = &buf[..n.cast_unsigned()];
        if let Err(error) = write_all_vectored_raw(vsock_fd, &[&len, frame]) {
            error!(error = %error, "VSOCK write failed");
            break;
        }
    }
    warn!("TAP → VSOCK forwarding stopped");
    running.store(false, Ordering::Relaxed);
}

/// VSOCK → TAP: read a 2-byte LE length header from the VSOCK stream,
/// then the frame payload, and write the raw frame to the TAP device.
fn tx_loop(vsock_fd: libc::c_int, tap_fd: libc::c_int, running: &Arc<AtomicBool>) {
    let mut len_buf = [0u8; FRAME_LEN_SIZE];
    let mut frame_buf = vec![0u8; MAX_FRAME_SIZE];

    while running.load(Ordering::Relaxed) {
        if let Err(error) = read_exact_raw(vsock_fd, &mut len_buf) {
            error!(error = %error, "VSOCK read (length header) failed");
            break;
        }

        let frame_len = u16::from_le_bytes(len_buf) as usize;
        if frame_len == 0 || frame_len > MAX_FRAME_SIZE {
            error!(frame_len, "invalid frame length");
            break;
        }

        if let Err(error) = read_exact_raw(vsock_fd, &mut frame_buf[..frame_len]) {
            error!(error = %error, "VSOCK read (frame payload) failed");
            break;
        }

        if let Err(error) = write_all_raw(tap_fd, &frame_buf[..frame_len]) {
            error!(error = %error, "TAP write failed");
            break;
        }
    }
    warn!("VSOCK → TAP forwarding stopped");
    running.store(false, Ordering::Relaxed);
}

fn start_forwarding(tap_fd: libc::c_int, vsock_fd: libc::c_int) {
    let running = Arc::new(AtomicBool::new(true));

    info!(
        device = TAP_DEVICE_NAME,
        port = HOST_PROXY_PORT,
        "started TAP ↔ VSOCK frame forwarding"
    );

    let r1 = Arc::clone(&running);
    std::thread::spawn(move || {
        let _span = tracing::info_span!("networking.tap_to_vsock").entered();
        rx_loop(tap_fd, vsock_fd, &r1);
    });

    let r2 = Arc::clone(&running);
    std::thread::spawn(move || {
        let _span = tracing::info_span!("networking.vsock_to_tap").entered();
        tx_loop(vsock_fd, tap_fd, &r2);
    });
}

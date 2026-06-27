use crate::constants::HOST_PROXY_PORT;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{error, info, warn};

const TAP_DEVICE_NAME: &str = "tap0";
const TAP_IP_CIDR: &str = "192.168.127.2/24";
const TAP_GATEWAY: &str = "192.168.127.1";
/// EC2 IMDS link-local address; must be routed via gvproxy (not treated as on-link on `tap0`).
const IMDS_HOST_ROUTE: &str = "169.254.169.254/32";
const TAP_MAC: &str = "ba:aa:ad:c0:ff:ee";
const TAP_MTU: &str = "1500";

const PARENT_CID: u32 = 3;
const MAX_FRAME_SIZE: usize = 65535;
const FRAME_LEN_SIZE: usize = 2;

const IFF_TAP: libc::c_short = 0x0002;
const IFF_NO_PI: libc::c_short = 0x1000;
const TUNSETIFF: libc::c_ulong = 0x4004_54ca;

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
pub async fn init() {
    info!(
        port = HOST_PROXY_PORT,
        "setting up enclave networking (TAP + VSOCK → gvproxy)"
    );

    // Enclave/minimal roots often start with `lo` down; without this, `127.0.0.1` returns ENETUNREACH.
    bring_loopback_up();

    let vsock_fd = loop {
        match connect_vsock(HOST_PROXY_PORT) {
            Ok(fd) => break fd,
            Err(e) => {
                info!(error = %e, "VSOCK connect failed, retrying in 1 s …");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    };
    info!(port = HOST_PROXY_PORT, "connected to gvproxy via VSOCK");

    send_handshake(vsock_fd).expect("failed to send POST /connect handshake");
    info!("handshake sent");

    let tap_fd = create_tap(TAP_DEVICE_NAME).expect("failed to create TAP device");
    info!(device = TAP_DEVICE_NAME, "TAP device created");

    configure_tap();
    info!("TAP interface configured");

    write_resolv_conf();
    info!("resolv.conf written");

    start_forwarding(tap_fd, vsock_fd);
    info!("frame forwarding started (TAP ↔ VSOCK)");
}

#[repr(C)]
struct SockAddrVm {
    family: u16,
    reserved1: u16,
    port: u32,
    cid: u32,
    zero: [u8; 4],
}

fn create_tap(name: &str) -> std::io::Result<libc::c_int> {
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

    Ok(fd)
}

fn configure_tap() {
    run_ip(&["link", "set", "dev", "lo", "up"]);
    run_ip(&["link", "set", "dev", TAP_DEVICE_NAME, "address", TAP_MAC]);
    run_ip(&["addr", "add", TAP_IP_CIDR, "dev", TAP_DEVICE_NAME]);
    run_ip(&["link", "set", "dev", TAP_DEVICE_NAME, "mtu", TAP_MTU]);
    run_ip(&["link", "set", "dev", TAP_DEVICE_NAME, "up"]);
    run_ip(&[
        "route",
        "add",
        "default",
        "via",
        TAP_GATEWAY,
        "dev",
        TAP_DEVICE_NAME,
    ]);
    // Without this, Linux may ARP for 169.254.169.254 on tap0 instead of forwarding to gvproxy.
    add_imds_route();
}

fn add_imds_route() {
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
        .status();
    match status {
        Ok(s) if s.success() => {}
        Ok(s) if s.code() == Some(2) => {
            // EEXIST: route already present (e.g. warm restart).
        }
        Ok(s) => {
            warn!(
                exit_code = ?s.code(),
                "ip route add for IMDS failed; metadata may be unreachable"
            );
        }
        Err(e) => warn!(error = %e, "failed to run ip route add for IMDS"),
    }
}

fn run_ip(args: &[&str]) {
    let status = Command::new("ip")
        .args(args)
        .status()
        .unwrap_or_else(|e| panic!("failed to run ip {args:?}: {e}"));
    assert!(
        status.success(),
        "ip command failed: {args:?}, exit {:?}",
        status.code()
    );
}

fn bring_loopback_up() {
    info!("bringing loopback lo up (127.0.0.1 may otherwise be unreachable)");
    run_ip(&["link", "set", "dev", "lo", "up"]);
}

fn write_resolv_conf() {
    std::fs::create_dir_all("/run/resolvconf").expect("failed to create /run/resolvconf");
    std::fs::write("/run/resolvconf/resolv.conf", "nameserver 192.168.127.1\n")
        .expect("failed to write /run/resolvconf/resolv.conf");
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
        if n <= 0 {
            error!("TAP read error or EOF (n={n})");
            break;
        }
        let len = u16::try_from(n)
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "frame length larger than u16",
                )
            })
            .unwrap()
            .to_le_bytes();
        let frame = &buf[..n.cast_unsigned()];
        if write_all_vectored_raw(vsock_fd, &[&len, frame]).is_err() {
            error!("VSOCK write error");
            break;
        }
    }
    running.store(false, Ordering::Relaxed);
}

/// VSOCK → TAP: read a 2-byte LE length header from the VSOCK stream,
/// then the frame payload, and write the raw frame to the TAP device.
fn tx_loop(vsock_fd: libc::c_int, tap_fd: libc::c_int, running: &Arc<AtomicBool>) {
    let mut len_buf = [0u8; FRAME_LEN_SIZE];
    let mut frame_buf = vec![0u8; MAX_FRAME_SIZE];

    while running.load(Ordering::Relaxed) {
        if read_exact_raw(vsock_fd, &mut len_buf).is_err() {
            error!("VSOCK read (length header) error or EOF");
            break;
        }

        let frame_len = u16::from_le_bytes(len_buf) as usize;
        if frame_len == 0 || frame_len > MAX_FRAME_SIZE {
            error!(frame_len, "invalid frame length");
            break;
        }

        if read_exact_raw(vsock_fd, &mut frame_buf[..frame_len]).is_err() {
            error!("VSOCK read (frame payload) error or EOF");
            break;
        }

        if write_all_raw(tap_fd, &frame_buf[..frame_len]).is_err() {
            error!("TAP write error");
            break;
        }
    }
    running.store(false, Ordering::Relaxed);
}

fn start_forwarding(tap_fd: libc::c_int, vsock_fd: libc::c_int) {
    let running = Arc::new(AtomicBool::new(true));

    let r1 = Arc::clone(&running);
    std::thread::spawn(move || rx_loop(tap_fd, vsock_fd, &r1));

    let r2 = Arc::clone(&running);
    std::thread::spawn(move || tx_loop(vsock_fd, tap_fd, &r2));
}

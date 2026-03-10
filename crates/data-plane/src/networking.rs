// TODO: cleanup this

use tracing::warn;
#[cfg(target_os = "linux")]
use tracing::info;

/// Set up enclave networking via TAP device + VSOCK to gvproxy on the host.
///
/// Creates a TAP device, connects to gvproxy over VSOCK, sends the POST /connect
/// handshake, configures the interface (IP, MAC, MTU, gateway, resolv.conf), and
/// spawns two threads that forward L2 frames between the TAP and the VSOCK stream
/// using the 2-byte little-endian length-prefixed protocol.
#[cfg(target_os = "linux")]
pub fn setup(host_proxy_port: u32) {
    use linux::*;

    info!(
        port = host_proxy_port,
        "setting up enclave networking (TAP + VSOCK → gvproxy)"
    );

    let vsock_fd = connect_vsock_with_retry(host_proxy_port);
    info!(port = host_proxy_port, "connected to gvproxy via VSOCK");

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

#[cfg(not(target_os = "linux"))]
pub fn setup(_host_proxy_port: u32) {
    warn!("enclave networking (TAP + VSOCK) requires Linux; skipping");
}

// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod linux {
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use tracing::{error, info};

    pub const TAP_DEVICE_NAME: &str = "tap0";
    const TAP_IP_CIDR: &str = "192.168.127.2/24";
    const TAP_GATEWAY: &str = "192.168.127.1";
    const TAP_MAC: &str = "ba:aa:ad:c0:ff:ee";
    const TAP_MTU: &str = "1500";

    const PARENT_CID: u32 = 3;
    const MAX_FRAME_SIZE: usize = 65535;
    const FRAME_LEN_SIZE: usize = 2;

    const IFF_TAP: libc::c_short = 0x0002;
    const IFF_NO_PI: libc::c_short = 0x1000;
    const TUNSETIFF: libc::c_ulong = 0x4004_54ca;

    const AF_VSOCK: libc::c_int = 40;

    #[repr(C)]
    struct SockAddrVm {
        svm_family: u16,
        svm_reserved1: u16,
        svm_port: u32,
        svm_cid: u32,
        svm_zero: [u8; 4],
    }

    // ── TAP device ──────────────────────────────────────────────────────────

    pub fn create_tap(name: &str) -> std::io::Result<libc::c_int> {
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
                ifr.ifr_name.as_mut_ptr() as *mut u8,
                name.len(),
            );
            ifr.ifr_ifru.ifru_flags = IFF_TAP | IFF_NO_PI;
        }

        let rc = unsafe { libc::ioctl(fd, TUNSETIFF as _, &ifr) };
        if rc < 0 {
            let err = std::io::Error::last_os_error();
            unsafe { libc::close(fd); }
            return Err(err);
        }

        Ok(fd)
    }

    pub fn configure_tap() {
        run_ip(&["link", "set", "dev", TAP_DEVICE_NAME, "address", TAP_MAC]);
        run_ip(&["addr", "add", TAP_IP_CIDR, "dev", TAP_DEVICE_NAME]);
        run_ip(&["link", "set", "dev", TAP_DEVICE_NAME, "mtu", TAP_MTU]);
        run_ip(&["link", "set", "dev", TAP_DEVICE_NAME, "up"]);
        run_ip(&[
            "route", "add", "default", "via", TAP_GATEWAY, "dev", TAP_DEVICE_NAME,
        ]);
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

    // ── resolv.conf ─────────────────────────────────────────────────────────

    pub fn write_resolv_conf() {
        std::fs::create_dir_all("/run/resolvconf")
        .expect("failed to create /run/resolvconf");
        std::fs::write("/run/resolvconf/resolv.conf", "nameserver 192.168.127.1\n")
            .expect("failed to write /run/resolvconf/resolv.conf");
    }

    // ── VSOCK connection ────────────────────────────────────────────────────

    fn connect_vsock(port: u32) -> std::io::Result<libc::c_int> {
        let fd = unsafe { libc::socket(AF_VSOCK, libc::SOCK_STREAM, 0) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }

        let addr = SockAddrVm {
            svm_family: AF_VSOCK as u16,
            svm_reserved1: 0,
            svm_port: port,
            svm_cid: PARENT_CID,
            svm_zero: [0; 4],
        };

        let rc = unsafe {
            libc::connect(
                fd,
                &addr as *const _ as *const libc::sockaddr,
                std::mem::size_of::<SockAddrVm>() as libc::socklen_t,
            )
        };

        if rc < 0 {
            let err = std::io::Error::last_os_error();
            unsafe { libc::close(fd); }
            return Err(err);
        }

        Ok(fd)
    }

    pub fn connect_vsock_with_retry(port: u32) -> libc::c_int {
        loop {
            match connect_vsock(port) {
                Ok(fd) => return fd,
                Err(e) => {
                    info!(error = %e, "VSOCK connect failed, retrying in 1 s …");
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
        }
    }

    // ── Handshake ───────────────────────────────────────────────────────────

    pub fn send_handshake(fd: libc::c_int) -> std::io::Result<()> {
        write_all_raw(fd, b"POST /connect HTTP/1.1\r\nHost: \r\n\r\n")
    }

    // ── Raw fd I/O helpers ──────────────────────────────────────────────────

    fn read_exact_raw(fd: libc::c_int, buf: &mut [u8]) -> std::io::Result<()> {
        let mut pos = 0;
        while pos < buf.len() {
            let n = unsafe {
                libc::read(
                    fd,
                    buf[pos..].as_mut_ptr() as *mut libc::c_void,
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
            pos += n as usize;
        }
        Ok(())
    }

    fn write_all_raw(fd: libc::c_int, buf: &[u8]) -> std::io::Result<()> {
        let mut pos = 0;
        while pos < buf.len() {
            let n = unsafe {
                libc::write(
                    fd,
                    buf[pos..].as_ptr() as *const libc::c_void,
                    buf.len() - pos,
                )
            };
            if n < 0 {
                return Err(std::io::Error::last_os_error());
            }
            pos += n as usize;
        }
        Ok(())
    }

    // ── Frame forwarding ────────────────────────────────────────────────────

    /// TAP → VSOCK: read raw Ethernet frames from the TAP device, prepend a
    /// 2-byte LE length header, and write to the VSOCK stream.
    fn rx_loop(tap_fd: libc::c_int, vsock_fd: libc::c_int, running: Arc<AtomicBool>) {
        let mut buf = vec![0u8; MAX_FRAME_SIZE];
        while running.load(Ordering::Relaxed) {
            let n = unsafe {
                libc::read(tap_fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len())
            };
            if n <= 0 {
                error!("TAP read error or EOF (n={n})");
                break;
            }
            let len = (n as u16).to_le_bytes();
            if write_all_raw(vsock_fd, &len).is_err()
                || write_all_raw(vsock_fd, &buf[..n as usize]).is_err()
            {
                error!("VSOCK write error");
                break;
            }
        }
        running.store(false, Ordering::Relaxed);
    }

    /// VSOCK → TAP: read a 2-byte LE length header from the VSOCK stream,
    /// then the frame payload, and write the raw frame to the TAP device.
    fn tx_loop(vsock_fd: libc::c_int, tap_fd: libc::c_int, running: Arc<AtomicBool>) {
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

    pub fn start_forwarding(tap_fd: libc::c_int, vsock_fd: libc::c_int) {
        let running = Arc::new(AtomicBool::new(true));

        let r1 = Arc::clone(&running);
        std::thread::spawn(move || rx_loop(tap_fd, vsock_fd, r1));

        let r2 = Arc::clone(&running);
        std::thread::spawn(move || tx_loop(vsock_fd, tap_fd, r2));
    }
}

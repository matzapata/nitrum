use crate::constants::HOST_PROXY_PORT;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{error, info};

const TAP_DEVICE_NAME: &str = "tap0";
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

/// Brave viproxy binary path in the data-plane image (`docker/data-plane.dockerfile`).
const DEFAULT_IMDS_VSOCK_PROXY_PATH: &str = "/app/imds-vsock-proxy";

/// Set up enclave networking via TAP device + VSOCK to gvproxy on the host.
///
/// Creates a TAP device, connects to gvproxy over VSOCK, sends the POST /connect
/// handshake, configures the interface (IP, MAC, MTU, gateway, resolv.conf), and
/// spawns two threads that forward L2 frames between the TAP and the VSOCK stream
/// using the 2-byte little-endian length-prefixed protocol.
///
/// Returns once TAP ↔ VSOCK forwarding is running (background threads). Call **before** loading
/// config that needs HTTPS egress (e.g. SSM), which uses this path to reach the VPC/internet.
///
/// Also starts **IMDS viproxy** on loopback (default `127.0.0.1:8099` → parent vsock `3:8002`) so
/// [`RuntimeConfig::load`](crate::config::RuntimeConfig::load) can reach IMDS without a shell entrypoint.
pub async fn init() {
    // Enclave/minimal roots often start with `lo` down; without this, `127.0.0.1` returns ENETUNREACH.
    bring_loopback_up();
    spawn_imds_vsock_proxy_and_wait().await;

    info!(
        port = HOST_PROXY_PORT,
        "setting up enclave networking (TAP + VSOCK → gvproxy)"
    );

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

// ── IMDS viproxy (loopback TCP → parent VSOCK → IMDS) ───────────────────

fn parse_first_in_addr_host_port(in_addrs: &str) -> (String, u16) {
    let first = in_addrs.split(',').next().unwrap_or(in_addrs).trim();
    let (host, port_str) = first.rsplit_once(':').unwrap_or_else(|| {
        panic!("invalid NITRUM_IMDS_VIPROXY_IN (expected host:port): {first:?}")
    });
    let port: u16 = port_str
        .parse()
        .unwrap_or_else(|_| panic!("invalid TCP port in NITRUM_IMDS_VIPROXY_IN: {port_str:?}"));
    (host.to_string(), port)
}

async fn wait_imds_viproxy_tcp(host: &str, port: u16) {
    let addr = format!("{host}:{port}");
    const ATTEMPTS: u32 = 200;
    const SLEEP_MS: u64 = 50;

    for attempt in 1..=ATTEMPTS {
        match tokio::net::TcpStream::connect(&addr).await {
            Ok(_) => {
                info!(%addr, attempt, "IMDS viproxy is accepting TCP connections");
                return;
            }
            Err(e) if attempt == 1 || attempt % 40 == 0 => {
                info!(%addr, attempt, error = %e, "waiting for IMDS viproxy to listen …");
            }
            Err(_) => {}
        }
        tokio::time::sleep(std::time::Duration::from_millis(SLEEP_MS)).await;
    }

    panic!(
        "timed out waiting for IMDS viproxy on {addr} ({ATTEMPTS} attempts); \
         verify imds-vsock-proxy and parent enclave-imds-proxy / vsock (default OUT 3:8002)"
    );
}

async fn spawn_imds_vsock_proxy_and_wait() {
    let bin = std::env::var("NITRUM_IMDS_VSOCK_PROXY_PATH")
        .unwrap_or_else(|_| DEFAULT_IMDS_VSOCK_PROXY_PATH.to_string());

    if !std::path::Path::new(&bin).exists() {
        panic!(
            "IMDS vsock proxy not found at {bin:?}; install imds-vsock-proxy into the image or set NITRUM_IMDS_VSOCK_PROXY_PATH"
        );
    }

    let in_addrs =
        std::env::var("NITRUM_IMDS_VIPROXY_IN").unwrap_or_else(|_| "127.0.0.1:8099".to_string());
    let out_addrs =
        std::env::var("NITRUM_IMDS_VIPROXY_OUT").unwrap_or_else(|_| "3:8002".to_string());
    let (listen_host, listen_port) = parse_first_in_addr_host_port(&in_addrs);

    info!(
        %bin,
        in_addrs = %in_addrs,
        out_addrs = %out_addrs,
        "spawning IMDS viproxy (loopback TCP → parent VSOCK)"
    );

    let mut child = Command::new(&bin)
        .env("IN_ADDRS", &in_addrs)
        .env("OUT_ADDRS", &out_addrs)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn IMDS viproxy {bin}: {e}"));

    let pid = child.id();
    std::thread::spawn(move || match child.wait() {
        Ok(status) if status.success() => {
            error!(
                pid,
                code = ?status.code(),
                "imds-vsock-proxy exited successfully (server should not exit on its own)"
            );
        }
        Ok(status) => {
            error!(
                pid,
                code = ?status.code(),
                "imds-vsock-proxy exited with failure"
            );
        }
        Err(e) => error!(pid, error = %e, "imds-vsock-proxy wait() failed"),
    });

    wait_imds_viproxy_tcp(&listen_host, listen_port).await;
}

// ── SockAddrVm ──────────────────────────────────────────────────────────

#[repr(C)]
struct SockAddrVm {
    svm_family: u16,
    svm_reserved1: u16,
    svm_port: u32,
    svm_cid: u32,
    svm_zero: [u8; 4],
}

// ── TAP device ──────────────────────────────────────────────────────────

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
            ifr.ifr_name.as_mut_ptr() as *mut u8,
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
    info!("bringing loopback lo up (required for IMDS viproxy on 127.0.0.1)");
    run_ip(&["link", "set", "dev", "lo", "up"]);
}

// ── resolv.conf ─────────────────────────────────────────────────────────

fn write_resolv_conf() {
    std::fs::create_dir_all("/run/resolvconf").expect("failed to create /run/resolvconf");
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
        unsafe {
            libc::close(fd);
        }
        return Err(err);
    }

    Ok(fd)
}

// ── Handshake ───────────────────────────────────────────────────────────

fn send_handshake(fd: libc::c_int) -> std::io::Result<()> {
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
        let n = unsafe { libc::read(tap_fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
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

fn start_forwarding(tap_fd: libc::c_int, vsock_fd: libc::c_int) {
    let running = Arc::new(AtomicBool::new(true));

    let r1 = Arc::clone(&running);
    std::thread::spawn(move || rx_loop(tap_fd, vsock_fd, r1));

    let r2 = Arc::clone(&running);
    std::thread::spawn(move || tx_loop(vsock_fd, tap_fd, r2));
}

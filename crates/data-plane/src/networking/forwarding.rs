//! Bidirectional L2 frame forwarding between the TAP device and the VSOCK stream.

use super::constants::{FRAME_LEN_SIZE, HOST_PROXY_PORT, MAX_FRAME_SIZE, TAP_DEVICE_NAME};
use crate::utils::io::{read_exact_raw, write_all_raw, write_all_vectored_raw};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{error, info, warn};

/// Spawns background threads that forward L2 frames between TAP and VSOCK until one side fails.
pub(super) fn start_forwarding(tap_fd: libc::c_int, vsock_fd: libc::c_int) {
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

/// Reads raw Ethernet frames from TAP, prefixes a 2-byte LE length, and writes to VSOCK.
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

/// Reads length-prefixed frames from VSOCK and writes the raw Ethernet payload to TAP.
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

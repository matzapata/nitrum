//! VSOCK connection and gvproxy handshake with the parent EC2 instance.

use super::constants::{
    AF_VSOCK, PARENT_CID, VSOCK_CONNECT_MAX_ATTEMPTS, VSOCK_CONNECT_RETRY_INTERVAL,
};
use crate::utils::io::write_all_raw;
use anyhow::{Context, Result};
use std::os::unix::io::{FromRawFd, OwnedFd};
use tracing::{debug, info, instrument, warn};

#[repr(C)]
struct SockAddrVm {
    family: u16,
    reserved1: u16,
    port: u32,
    cid: u32,
    zero: [u8; 4],
}

/// Opens a blocking VSOCK stream to the parent CID on the given port.
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

/// Retries VSOCK connect until gvproxy accepts or attempts are exhausted.
#[instrument(name = "networking.connect_vsock", fields(port), err)]
pub(super) async fn connect_vsock_with_retry(port: u32) -> Result<OwnedFd> {
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

/// Sends the HTTP `POST /connect` handshake that activates gvproxy frame forwarding.
pub(super) fn send_handshake(fd: libc::c_int) -> std::io::Result<()> {
    write_all_raw(fd, b"POST /connect HTTP/1.1\r\nHost: \r\n\r\n")
}

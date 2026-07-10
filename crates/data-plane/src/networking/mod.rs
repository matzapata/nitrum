//! Enclave network initialization and configuration for the data plane.
//!
//! This module establishes the networking stack inside an AWS Nitro enclave.
//! It sets up a virtual TAP (Ethernet) device and manages connectivity to the
//! outside world via VSOCK-based communication with `gvproxy` on the parent EC2
//! instance.

mod constants;
mod forwarding;
mod tap;
mod vsock;

use anyhow::{Context, Result};
use constants::HOST_PROXY_PORT;
use std::os::unix::io::{AsRawFd, IntoRawFd};
use tracing::{info, instrument};

use constants::TAP_DEVICE_NAME;
use tap::{bring_loopback_up, configure_tap, create_tap, write_resolv_conf};
use vsock::{connect_vsock_with_retry, send_handshake};

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
        ip_cidr = constants::TAP_IP_CIDR,
        gateway = constants::TAP_GATEWAY,
        "TAP interface configured"
    );

    write_resolv_conf().context("write resolv.conf")?;

    forwarding::start_forwarding(tap.into_raw_fd(), vsock.into_raw_fd());
    Ok(())
}

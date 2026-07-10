//! Network constants for TAP device setup and VSOCK communication with gvproxy.

use std::time::Duration;

/// VSOCK port where gvproxy listens on the host (CID 3).
pub(super) const HOST_PROXY_PORT: u32 = 1024;

/// Delay between VSOCK connection attempts to gvproxy on the host.
pub(super) const VSOCK_CONNECT_RETRY_INTERVAL: Duration = Duration::from_secs(1);

/// Maximum VSOCK connection attempts before enclave networking init fails.
pub(super) const VSOCK_CONNECT_MAX_ATTEMPTS: u32 = 30;

/// The name of the TAP network device created inside the enclave.
pub(super) const TAP_DEVICE_NAME: &str = "tap0";

/// IPv4 address and netmask (CIDR) assigned to the TAP device inside the enclave.
pub(super) const TAP_IP_CIDR: &str = "192.168.127.2/24";

/// The IPv4 gateway address used for the TAP device's route table.
pub(super) const TAP_GATEWAY: &str = "192.168.127.1";

/// The IPv4 host route (link-local) for EC2 Instance Metadata Service (IMDS).
/// Traffic to this address is routed via gvproxy.
pub(super) const IMDS_HOST_ROUTE: &str = "169.254.169.254/32";

/// The MAC address assigned to the TAP device. Used for interface configuration.
pub(super) const TAP_MAC: &str = "ba:aa:ad:c0:ff:ee";

/// The Maximum Transmission Unit (MTU) for the TAP interface.
pub(super) const TAP_MTU: &str = "1500";

/// The parent context ID (CID) used for VSOCK communication with the host (always 3 in Nitro enclaves).
pub(super) const PARENT_CID: u32 = 3;

/// Maximum size (in bytes) for a L2 frame (largest allowed Ethernet frame size).
pub(super) const MAX_FRAME_SIZE: usize = 65535;

/// Number of bytes used to represent frame length prefix for each transferred frame.
pub(super) const FRAME_LEN_SIZE: usize = 2;

/// Flag used with TUN/TAP ioctls: designates a TAP (Ethernet) device.
pub(super) const IFF_TAP: libc::c_short = 0x0002;

/// Flag used with TUN/TAP ioctls: disables packet information prepending (raw Ethernet).
pub(super) const IFF_NO_PI: libc::c_short = 0x1000;

/// ioctl request code for creating/configuring a TUN/TAP device.
pub(super) const TUNSETIFF: libc::c_ulong = 0x4004_54ca;

/// Socket domain constant for VSOCK (host/guest communication).
pub(super) const AF_VSOCK: libc::c_int = 40;

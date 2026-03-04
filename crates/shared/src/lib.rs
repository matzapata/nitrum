pub mod bridge;
pub mod protocol;
pub mod server;

/// Bridge port numbers used by both the data-plane and control-plane to identify
/// each channel across the VSock (production) or TCP (local dev) transport.
///
/// VSock uses these as virtual port numbers; TCP mode uses them as TCP port numbers.
/// Both transports accept a `u16`, so the same constants work for both.
pub mod ports {
    /// TCP egress channel: data-plane connects, control-plane listens.
    pub const TCP_PROXY: u16 = 8181;

    /// DNS channel: data-plane connects, control-plane listens.
    pub const DNS_PROXY: u16 = 5354;

    /// Ingress channel: control-plane connects, data-plane listens.
    pub const INGRESS: u16 = 7777;
}

/// VSock CID of the enclave. In AWS Nitro the enclave's own CID is assigned
/// at launch; adjust this value or read it from an env var as needed.
#[cfg(feature = "enclave")]
pub const ENCLAVE_CID: u32 = 16;

/// VSock CID of the parent/host. In AWS Nitro the parent is always CID 3.
#[cfg(feature = "enclave")]
pub const PARENT_CID: u32 = 3;

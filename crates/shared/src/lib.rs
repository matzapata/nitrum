pub mod bridge;
pub mod server;

/// VSock CID of the enclave. In AWS Nitro the enclave's own CID is assigned
/// at launch; adjust this value or read it from an env var as needed.
#[cfg(feature = "enclave")]
pub const ENCLAVE_CID: u32 = 16;

/// VSock CID of the parent/host. In AWS Nitro the parent is always CID 3.
#[cfg(feature = "enclave")]
pub const PARENT_CID: u32 = 3;

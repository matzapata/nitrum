//! Runtime bootstrap: resolve `DataPlaneConfig` from `nitrum.toml`, IMDS, and SSM.

pub mod config;
pub mod imds;
pub mod ssm;

pub use config::{DataPlaneConfig, ListenAddrs};

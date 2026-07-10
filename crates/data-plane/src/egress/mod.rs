//! Egress whitelist enforcement (DNS + transparent TCP proxy) on supported platforms.
//!
//! Enforcement modules compile only on Linux.

#[cfg(target_os = "linux")]
mod constants;
#[cfg(target_os = "linux")]
mod dns;
#[cfg(target_os = "linux")]
mod filter;
#[cfg(target_os = "linux")]
mod ip_cache;
#[cfg(target_os = "linux")]
mod iptables;
#[cfg(target_os = "linux")]
mod platform;
pub mod server;
#[cfg(target_os = "linux")]
mod tcp;

#[cfg(target_os = "linux")]
pub use filter::EgressFilter;
pub use server::{EgressGuard, init};

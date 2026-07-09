//! Egress whitelist enforcement (DNS + transparent TCP proxy) on supported platforms.

#[cfg(egress_enforcement)]
mod constants;
#[cfg(egress_enforcement)]
mod dns;
#[cfg(egress_enforcement)]
mod filter;
#[cfg(egress_enforcement)]
mod ip_cache;
#[cfg(egress_enforcement)]
mod iptables;
#[cfg(egress_enforcement)]
mod platform;
pub mod server;
#[cfg(egress_enforcement)]
mod tcp;

#[cfg(egress_enforcement)]
pub use filter::EgressFilter;
pub use server::{EgressGuard, init};

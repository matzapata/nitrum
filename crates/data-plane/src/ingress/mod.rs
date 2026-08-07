//! Ingress proxy: Responsible for terminating TLS connections, serving well-known
//! enclave endpoints and ACME challenges, and proxying all other traffic to the
//! user application.
//!
//! Platform well-known paths are always handled directly (the user app is not invoked):
//!   - GET /.well-known/enclave/status
//!     -> 200 {"status":"ok"} when `[health_check]` probes succeed (or no start_command);
//!     503 {"status":"unhealthy"} otherwise. NLB HTTPS health checks use this path.
//!   - GET /.well-known/enclave/attestation
//!     -> Responds with 200 and base64-encoded attestation document
//!     Optional query: `nonce` — standard base64 of raw nonce bytes (same encoding as the crypto API)
//!   - GET /.well-known/acme-challenge/*
//!     -> Responds with 200 and the ACME HTTP-01 key authorization string
//!
//! The server exposes both a plain HTTP listener (default: port 80, for ACME HTTP-01
//! validation) and a TLS listener (default: port 443) using the same Axum router.
//! TLS config is provided by `tls::build_tls_config`. The certificate is renewed in
//! the background and can be reloaded without restarting the ingress server.

pub mod acme;
mod health;
pub mod server;
mod state;
pub mod tls;

pub use server::{build_https_router, init};
pub use state::IngressState;

//! Symmetric encryption using a Data Encryption Key (DEK) and crypto HTTP API.
//!
//! Use [`CryptoApi::setup`] to bootstrap the DEK from infra, then [`CryptoApi::run`] to serve
//! the attestation / encrypt / decrypt endpoints.

pub mod attest;
pub mod api;
pub mod kms;
mod rand;

pub use api::{Crypto, CryptoApi};

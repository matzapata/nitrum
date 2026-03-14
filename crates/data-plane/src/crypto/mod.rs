//! Symmetric encryption using a Data Encryption Key (DEK) and crypto HTTP API.
//!
//! Use [`CryptoClient::new`] to bootstrap the DEK from storage, then [`api::run`] to serve
//! the attestation / encrypt / decrypt endpoints.

pub mod api;
pub mod client;
mod attest;
mod kms;
mod rng;

pub use client::CryptoClient;

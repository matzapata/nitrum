//! Provide a DEK for the user process to use for encryption and decryption.
//!
//! Provide an HTTP API for the user process to use for encryption and decryption.
//! Provide an attestation document for the user process to use for attestation.

pub mod api;
mod attest;
mod client;
mod kms;
mod kv;
mod random;

pub use api::init;
pub use attest::get_attestation_doc;
pub use client::CryptoClient;

//! Provide a DEK for the user process to use for encryption and decryption.
//! Provide an HTTP API for the user process to use for encryption and decryption.
//! Provide an attestation document for the user process to use for attestation.

pub mod api;
mod attest;
mod client;
mod kv;
mod kms;
mod random;

pub use attest::get_attestation_doc;
pub use client::CryptoClient;

//! Provide a DEK for the user process to use for encryption and decryption.
//! Provide an HTTP API for the user process to use for encryption and decryption.
//! Provide an attestation document for the user process to use for attestation.

pub mod api;
mod attest;
pub mod client;
mod kms;
mod rng;

pub use client::CryptoClient;

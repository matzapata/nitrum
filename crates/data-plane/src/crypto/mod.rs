//! Provide a DEK for the user process to use for encryption and decryption.
//!
//! Provide an HTTP API for the user process to use for encryption and decryption.
//! Provide an attestation document for the user process to use for attestation.

mod attest;
mod dek;
mod kms;
mod kv;
#[cfg(feature = "enclave")]
mod nsm;
mod random;
pub mod server;

pub use attest::get_attestation_doc;
pub use dek::{AesGcmCrypto, Crypto};
pub use kms::{AwsKms, Kms};
pub use server::init;

//! Error and result types for attestation verification.

use crate::document::ParsedAttestation;
use thiserror::Error;

/// Successful verification outcome.
#[derive(Debug, Clone)]
pub struct AttestationSuccess {
    /// Fully validated attestation document attributes.
    pub document: ParsedAttestation,
}

/// Failed verification outcome.
#[derive(Debug)]
pub struct AttestationFailure {
    /// Human-readable reason for validation failure.
    pub reason: String,
    /// Underlying error when `VerifyOptions.debug` is enabled.
    pub error: Option<VerifyError>,
}

/// Result of [`crate::verify_attestation`].
#[derive(Debug)]
pub enum AttestationResult {
    /// Document verified successfully.
    Success(AttestationSuccess),
    /// Document failed verification.
    Failure(AttestationFailure),
}

impl AttestationResult {
    /// Returns `true` when verification succeeded.
    #[must_use]
    pub const fn valid(&self) -> bool {
        matches!(self, Self::Success(_))
    }
}

/// Internal verification errors.
#[derive(Debug, Error)]
pub enum VerifyError {
    /// CBOR / COSE structure could not be decoded.
    #[error("decode error: {0}")]
    Decode(String),
    /// Attestation payload missing required fields or malformed.
    #[error("parse error: {0}")]
    Parse(String),
    /// COSE ECDSA signature did not verify.
    #[error("attestation document signature is invalid")]
    InvalidSignature,
    /// Cryptographic operation failed.
    #[error("crypto error: {0}")]
    Crypto(String),
    /// X.509 parse or encoding failure.
    #[error("x509 error: {0}")]
    X509(String),
    /// CA bundle was empty.
    #[error("certificate bundle is empty")]
    EmptyCaBundle,
    /// Leaf was not signed by the top intermediate.
    #[error("signing certificate was not issued by the CA bundle")]
    SigningCertNotIssuedByBundle,
    /// Intermediate chain link failed.
    #[error("certificate chain validation failed")]
    ChainValidationFailed,
    /// Chain does not terminate at the trusted root.
    #[error("certificate chain does not anchor to the trusted root CA")]
    ChainDoesNotAnchorRoot,
    /// Policy required a nonce but the document had none.
    #[error("attestation document has no nonce")]
    MissingNonce,
    /// Document nonce did not match the expected value.
    #[error("attestation nonce does not match expected value")]
    NonceMismatch,
    /// `max_age_ms` was zero or negative.
    #[error("maxAgeMs must be a positive number")]
    InvalidMaxAge,
    /// Document timestamp is older than the allowed window.
    #[error("attestation document is too old")]
    DocumentTooOld,
}

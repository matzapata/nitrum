//! Verify AWS Nitro Enclave attestation documents.
//!
//! Pure library: no HTTP or TLS socket I/O. Callers supply document bytes and
//! optional policy constraints; TLS leaf binding is a separate SHA-256 check.

mod caroot;
mod cose;
mod document;
mod error;
mod policy;
mod x509;

pub use caroot::AWS_NITRO_ROOT_CA_PEM;
pub use document::{ParsedAttestation, VerifyOptions};
pub use error::{AttestationFailure, AttestationResult, AttestationSuccess, VerifyError};

use sha2::{Digest, Sha256};

/// Verify an AWS Nitro Enclave attestation document (COSE_Sign1).
///
/// Performs COSE decode, payload parse, ECDSA-P384 signature check, certificate
/// chain validation to the trusted root, and optional nonce / freshness / PCR policy.
#[must_use]
pub fn verify_attestation(document: &[u8], options: &VerifyOptions) -> AttestationResult {
    let debug = options.debug;

    let sign1 = match cose::decode_cose_sign1(document) {
        Ok(s) => s,
        Err(error) => {
            return fail(
                debug,
                if debug {
                    "Failed to decode COSE_Sign1 structure"
                } else {
                    "Failed to decode attestation document"
                },
                Some(error),
            );
        }
    };

    let attestation = match document::parse_attestation_payload(&sign1.payload) {
        Ok(a) => a,
        Err(error) => {
            return fail(
                debug,
                if debug {
                    "Failed to parse attestation document payload"
                } else {
                    "Failed to parse attestation document"
                },
                Some(error),
            );
        }
    };

    if let Err(error) = cose::verify_cose_signature(&sign1, &attestation) {
        let reason = match &error {
            VerifyError::InvalidSignature => "Attestation document signature is invalid",
            _ => "Failed to verify attestation document signature",
        };
        return fail(debug, reason, Some(error));
    }

    let root = options
        .trusted_root
        .as_deref()
        .unwrap_or(AWS_NITRO_ROOT_CA_PEM.as_bytes());

    if let Err(error) = x509::verify_certificate_chain(&attestation, root) {
        let reason = error.chain_reason();
        return fail(debug, reason, Some(error));
    }

    if let Err(error) = policy::verify_policy_constraints(
        &attestation,
        options.nonce.as_deref(),
        options.max_age_ms,
    ) {
        return fail(debug, error.policy_reason(), Some(error));
    }

    let parsed = document::to_parsed(&attestation);

    if let Some(expected) = &options.expected_pcrs
        && !expected.is_empty()
        && let Err(reason) = policy::check_expected_pcrs(&parsed, expected)
    {
        return AttestationResult::Failure(AttestationFailure {
            reason,
            error: None,
        });
    }

    AttestationResult::Success(AttestationSuccess { document: parsed })
}

/// Return true when `public_key` is the SHA-256 digest of `tls_leaf_certificate_der`.
#[must_use]
pub fn verify_tls_leaf_binds(public_key: Option<&[u8]>, tls_leaf_certificate_der: &[u8]) -> bool {
    let Some(bound) = public_key else {
        return false;
    };
    if bound.len() != 32 {
        return false;
    }
    let digest = Sha256::digest(tls_leaf_certificate_der);
    constant_time_eq(bound, digest.as_slice())
}

fn fail(debug: bool, reason: &str, error: Option<VerifyError>) -> AttestationResult {
    AttestationResult::Failure(AttestationFailure {
        reason: reason.to_owned(),
        error: if debug { error } else { None },
    })
}

/// Constant-time equality for equal-length byte slices.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

impl VerifyError {
    const fn chain_reason(&self) -> &'static str {
        match self {
            Self::EmptyCaBundle => "Certificate bundle is empty",
            Self::SigningCertNotIssuedByBundle => {
                "Signing certificate was not issued by the CA bundle"
            }
            Self::ChainValidationFailed => "Certificate chain validation failed",
            Self::ChainDoesNotAnchorRoot => {
                "Certificate chain does not anchor to the trusted root CA"
            }
            _ => "Failed to validate certificate chain",
        }
    }

    const fn policy_reason(&self) -> &'static str {
        match self {
            Self::MissingNonce => "Attestation document has no nonce",
            Self::NonceMismatch => "Attestation nonce does not match expected value",
            Self::InvalidMaxAge => "maxAgeMs must be a positive number",
            Self::DocumentTooOld => "Attestation document is too old",
            _ => "Policy validation failed",
        }
    }
}

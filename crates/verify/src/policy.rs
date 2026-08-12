//! Nonce, freshness, and expected-PCR policy checks.

use crate::constant_time_eq;
use crate::document::AttestationDocument;
use crate::document::{ParsedAttestation, now_ms};
use crate::error::VerifyError;
use std::collections::BTreeMap;

/// Enforce optional nonce and max-age constraints.
pub fn verify_policy_constraints(
    attestation: &AttestationDocument,
    expected_nonce: Option<&[u8]>,
    max_age_ms: Option<u64>,
) -> Result<(), VerifyError> {
    if let Some(expected) = expected_nonce {
        let Some(actual) = attestation.nonce.as_deref() else {
            return Err(VerifyError::MissingNonce);
        };
        if !constant_time_eq(actual, expected) {
            return Err(VerifyError::NonceMismatch);
        }
    }

    if let Some(max_age) = max_age_ms {
        if max_age == 0 {
            return Err(VerifyError::InvalidMaxAge);
        }
        let now = now_ms();
        let age = now.saturating_sub(attestation.timestamp);
        if age > max_age {
            return Err(VerifyError::DocumentTooOld);
        }
    }

    Ok(())
}

/// When `expected` is non-empty, every listed key must match the document PCR hex.
pub fn check_expected_pcrs(
    document: &ParsedAttestation,
    expected: &BTreeMap<String, String>,
) -> Result<(), String> {
    for (key, expected_hex) in expected {
        let Some(actual_hex) = document.pcrs.get(key) else {
            return Err(format!("Missing PCR {key} in attestation"));
        };
        let actual_bytes =
            hex_to_bytes(actual_hex).ok_or_else(|| format!("Invalid hex for PCR {key}"))?;
        let expected_bytes =
            hex_to_bytes(expected_hex).ok_or_else(|| format!("Invalid hex for PCR {key}"))?;
        if actual_bytes.len() != expected_bytes.len() {
            return Err(format!("PCR {key} length mismatch"));
        }
        if !constant_time_eq(&actual_bytes, &expected_bytes) {
            return Err(format!("PCR {key} does not match expected value"));
        }
    }
    Ok(())
}

fn hex_to_bytes(hex: &str) -> Option<Vec<u8>> {
    let h = hex
        .strip_prefix("0x")
        .or_else(|| hex.strip_prefix("0X"))
        .unwrap_or(hex);
    let h = h.to_ascii_lowercase();
    if !h.len().is_multiple_of(2) {
        return None;
    }
    hex::decode(h).ok()
}

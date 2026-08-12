//! Integration tests for attestation verification.

use base64::Engine;
use std::collections::BTreeMap;
use std::path::PathBuf;
use verify::{AttestationResult, VerifyOptions, verify_attestation, verify_tls_leaf_binds};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Fixture files are base64 text (same shape as API `document` fields).
fn load_attestation(name: &str) -> Vec<u8> {
    let text = std::fs::read_to_string(fixtures_dir().join(name))
        .unwrap_or_else(|e| panic!("read {name}: {e}"));
    base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .unwrap_or_else(|e| panic!("base64 {name}: {e}"))
}

#[test]
fn accepts_valid_cbor() {
    let raw = load_attestation("valid.cbor");
    let result = verify_attestation(&raw, &VerifyOptions::default());
    assert!(result.valid(), "{result:?}");
    let AttestationResult::Success(ok) = result else {
        panic!("expected success");
    };
    assert!(!ok.document.module_id.is_empty());
    assert!(!ok.document.digest.is_empty());
    assert!(!ok.document.pcrs.is_empty());
}

#[test]
fn accepts_debug_mode_cbor() {
    let raw = load_attestation("debug-mode.cbor");
    let result = verify_attestation(&raw, &VerifyOptions::default());
    assert!(result.valid(), "{result:?}");
}

#[test]
fn rejects_invalid_signature() {
    let raw = load_attestation("invalid-signature.cbor");
    let result = verify_attestation(&raw, &VerifyOptions::default());
    let AttestationResult::Failure(fail) = result else {
        panic!("expected failure");
    };
    assert_eq!(fail.reason, "Attestation document signature is invalid");
}

#[test]
fn rejects_mismatched_nonce() {
    let raw = load_attestation("valid.cbor");
    let result = verify_attestation(
        &raw,
        &VerifyOptions {
            nonce: Some(vec![0u8; 32]),
            ..Default::default()
        },
    );
    let AttestationResult::Failure(fail) = result else {
        panic!("expected failure");
    };
    assert!(
        fail.reason == "Attestation document has no nonce"
            || fail.reason == "Attestation nonce does not match expected value",
        "{}",
        fail.reason
    );
}

#[test]
fn accepts_matching_nonce() {
    let raw = load_attestation("valid-nonce.cbor");
    let first = verify_attestation(&raw, &VerifyOptions::default());
    assert!(first.valid(), "{first:?}");
    let AttestationResult::Success(ok) = first else {
        panic!("expected success");
    };
    let nonce = ok.document.nonce.expect("nonce present");
    let second = verify_attestation(
        &raw,
        &VerifyOptions {
            nonce: Some(nonce),
            ..Default::default()
        },
    );
    assert!(second.valid(), "{second:?}");
}

#[test]
fn rejects_pcr_mismatch() {
    let raw = load_attestation("valid.cbor");
    let first = verify_attestation(&raw, &VerifyOptions::default());
    let AttestationResult::Success(ok) = first else {
        panic!("expected success");
    };
    let pcr0 = ok.document.pcrs.get("pcr0").expect("pcr0").clone();
    let mut bad = pcr0.clone();
    bad.replace_range(pcr0.len() - 2.., "00");
    let mut expected = BTreeMap::new();
    expected.insert("pcr0".into(), bad);
    let result = verify_attestation(
        &raw,
        &VerifyOptions {
            expected_pcrs: Some(expected),
            ..Default::default()
        },
    );
    let AttestationResult::Failure(fail) = result else {
        panic!("expected failure");
    };
    assert!(fail.reason.contains("PCR pcr0"), "{}", fail.reason);
}

#[test]
fn accepts_matching_pcrs_case_insensitive() {
    let raw = load_attestation("valid.cbor");
    let first = verify_attestation(&raw, &VerifyOptions::default());
    let AttestationResult::Success(ok) = first else {
        panic!("expected success");
    };
    let pcr0 = ok.document.pcrs.get("pcr0").expect("pcr0").to_uppercase();
    let mut expected = BTreeMap::new();
    expected.insert("pcr0".into(), pcr0);
    let second = verify_attestation(
        &raw,
        &VerifyOptions {
            expected_pcrs: Some(expected),
            ..Default::default()
        },
    );
    assert!(second.valid(), "{second:?}");
}

#[test]
fn rejects_non_positive_max_age() {
    let raw = load_attestation("valid.cbor");
    let result = verify_attestation(
        &raw,
        &VerifyOptions {
            max_age_ms: Some(0),
            ..Default::default()
        },
    );
    let AttestationResult::Failure(fail) = result else {
        panic!("expected failure");
    };
    assert_eq!(fail.reason, "maxAgeMs must be a positive number");
}

#[test]
fn tls_leaf_bind_match_and_mismatch() {
    let der = vec![1u8; 100];
    let digest = {
        use sha2::{Digest, Sha256};
        Sha256::digest(&der).to_vec()
    };
    assert!(verify_tls_leaf_binds(Some(&digest), &der));
    assert!(!verify_tls_leaf_binds(Some(&digest), &[2u8; 100]));
    assert!(!verify_tls_leaf_binds(None, &der));
    assert!(!verify_tls_leaf_binds(Some(&[0u8; 16]), &der));
}

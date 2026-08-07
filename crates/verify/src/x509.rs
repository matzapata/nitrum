//! X.509 certificate chain validation for Nitro attestation documents.

use crate::document::AttestationDocument;
use crate::error::VerifyError;
use ecdsa::signature::Verifier;
use p384::ecdsa::{Signature as P384Signature, VerifyingKey};
use p384::pkcs8::DecodePublicKey;
use x509_parser::prelude::*;
use x509_parser::public_key::PublicKey;

/// Verify the signing certificate chains to `trusted_root` (PEM or DER).
pub fn verify_certificate_chain(
    attestation: &AttestationDocument,
    trusted_root: &[u8],
) -> Result<(), VerifyError> {
    if attestation.cabundle.is_empty() {
        return Err(VerifyError::EmptyCaBundle);
    }

    let signing = parse_cert(&attestation.certificate)?;
    let intermediates: Vec<ParsedCert> = attestation
        .cabundle
        .iter()
        .map(|der| parse_cert(der))
        .collect::<Result<_, _>>()?;
    let root = parse_root(trusted_root)?;

    let top = intermediates.last().ok_or(VerifyError::EmptyCaBundle)?;
    if !verify_cert_signed_by(&signing, top)? {
        return Err(VerifyError::SigningCertNotIssuedByBundle);
    }

    for i in (1..intermediates.len()).rev() {
        let current = &intermediates[i];
        let previous = &intermediates[i - 1];
        if !verify_cert_signed_by(current, previous)? {
            return Err(VerifyError::ChainValidationFailed);
        }
    }

    let first = intermediates
        .first()
        .ok_or(VerifyError::ChainValidationFailed)?;
    if !verify_cert_signed_by(first, &root)? {
        return Err(VerifyError::ChainDoesNotAnchorRoot);
    }

    Ok(())
}

struct ParsedCert {
    /// DER of the full certificate (kept for SPKI extraction).
    der: Vec<u8>,
    /// TBS certificate bytes that were signed.
    tbs: Vec<u8>,
    /// Raw signature value from the certificate.
    signature: Vec<u8>,
}

fn parse_cert(der: &[u8]) -> Result<ParsedCert, VerifyError> {
    let (_, cert) =
        X509Certificate::from_der(der).map_err(|e| VerifyError::X509(format!("parse: {e}")))?;
    Ok(ParsedCert {
        der: der.to_vec(),
        tbs: cert.tbs_certificate.as_ref().to_vec(),
        signature: cert.signature_value.data.to_vec(),
    })
}

fn parse_root(trusted_root: &[u8]) -> Result<ParsedCert, VerifyError> {
    let trimmed = trim_ascii(trusted_root);
    if trimmed.starts_with(b"-----BEGIN") {
        let der = pem_to_der(trimmed)?;
        return parse_cert(&der);
    }
    parse_cert(trusted_root)
}

fn trim_ascii(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(0);
    let end = bytes
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(0, |i| i + 1);
    &bytes[start..end]
}

fn pem_to_der(pem: &[u8]) -> Result<Vec<u8>, VerifyError> {
    let text = std::str::from_utf8(pem).map_err(|e| VerifyError::X509(format!("pem utf8: {e}")))?;
    let b64: String = text.lines().filter(|l| !l.starts_with("-----")).collect();
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64.trim())
        .map_err(|e| VerifyError::X509(format!("pem base64: {e}")))
}

fn verify_cert_signed_by(child: &ParsedCert, issuer: &ParsedCert) -> Result<bool, VerifyError> {
    let (_, issuer_cert) = X509Certificate::from_der(&issuer.der)
        .map_err(|e| VerifyError::X509(format!("issuer: {e}")))?;

    let spki = issuer_cert.public_key();
    match spki.parsed() {
        Ok(PublicKey::EC(_)) => {}
        Ok(_) => return Ok(false),
        Err(e) => return Err(VerifyError::X509(format!("issuer key: {e}"))),
    }

    let verifying_key = VerifyingKey::from_public_key_der(spki.raw)
        .map_err(|e| VerifyError::Crypto(format!("issuer verifying key: {e}")))?;

    // Certificate signatures are DER-encoded ECDSA.
    let signature = match P384Signature::from_der(&child.signature) {
        Ok(s) => s,
        Err(_) => {
            // Some stacks may use raw r||s; try that as fallback.
            match P384Signature::from_slice(&child.signature) {
                Ok(s) => s,
                Err(_) => return Ok(false),
            }
        }
    };

    Ok(verifying_key.verify(&child.tbs, &signature).is_ok())
}

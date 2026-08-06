//! COSE_Sign1 decode and ECDSA-P384 signature verification.

use crate::document::AttestationDocument;
use crate::error::VerifyError;
use ciborium::value::Value;
use ecdsa::signature::Verifier;
use p384::ecdsa::{Signature, VerifyingKey};
use p384::pkcs8::DecodePublicKey;
use x509_parser::prelude::*;

/// Decoded COSE_Sign1 components needed for verification.
pub struct DecodedSign1 {
    /// Protected header bytes (CBOR-encoded map).
    pub protected_header: Vec<u8>,
    /// Attestation payload bytes.
    pub payload: Vec<u8>,
    /// Raw ECDSA signature (r||s for P-384 = 96 bytes).
    pub signature: Vec<u8>,
}

/// Decode a CBOR-encoded COSE_Sign1 structure.
pub fn decode_cose_sign1(raw: &[u8]) -> Result<DecodedSign1, VerifyError> {
    let value: Value =
        ciborium::from_reader(raw).map_err(|e| VerifyError::Decode(format!("cbor: {e}")))?;

    let arr = match value {
        Value::Tag(_, boxed) => match *boxed {
            Value::Array(a) => a,
            _ => {
                return Err(VerifyError::Decode(
                    "Unexpected CBOR structure: expected array or tagged value".into(),
                ));
            }
        },
        Value::Array(a) => a,
        _ => {
            return Err(VerifyError::Decode(
                "Unexpected CBOR structure: expected array or tagged value".into(),
            ));
        }
    };

    if arr.len() != 4 {
        return Err(VerifyError::Decode(format!(
            "COSE_Sign1 must have 4 elements, got {}",
            arr.len()
        )));
    }

    let protected_header = match &arr[0] {
        Value::Bytes(b) => b.clone(),
        _ => return Err(VerifyError::Decode("protectedHeader is not bytes".into())),
    };
    let payload = match &arr[2] {
        Value::Bytes(b) => b.clone(),
        _ => return Err(VerifyError::Decode("payload is not bytes".into())),
    };
    let signature = match &arr[3] {
        Value::Bytes(b) => b.clone(),
        _ => return Err(VerifyError::Decode("signature is not bytes".into())),
    };

    Ok(DecodedSign1 {
        protected_header,
        payload,
        signature,
    })
}

/// Verify the COSE signature using the certificate embedded in the payload.
pub fn verify_cose_signature(
    sign1: &DecodedSign1,
    attestation: &AttestationDocument,
) -> Result<(), VerifyError> {
    let (_, cert) = X509Certificate::from_der(&attestation.certificate)
        .map_err(|e| VerifyError::X509(format!("signing cert: {e}")))?;

    let spki = cert.public_key().raw;
    let verifying_key = VerifyingKey::from_public_key_der(spki)
        .map_err(|e| VerifyError::Crypto(format!("public key: {e}")))?;

    let sig_structure = build_sig_structure(&sign1.protected_header, &sign1.payload)?;
    let signature = Signature::from_slice(&sign1.signature)
        .map_err(|e| VerifyError::Crypto(format!("signature decode: {e}")))?;

    // `Verifier` hashes the Sig_Structure with SHA-384 (P-384 ECDSA).
    verifying_key
        .verify(&sig_structure, &signature)
        .map_err(|_| VerifyError::InvalidSignature)?;

    Ok(())
}

/// Build the COSE Sig_Structure array used for ECDSA signature verification.
fn build_sig_structure(protected_header: &[u8], payload: &[u8]) -> Result<Vec<u8>, VerifyError> {
    let structure = Value::Array(vec![
        Value::Text("Signature1".into()),
        Value::Bytes(protected_header.to_vec()),
        Value::Bytes(Vec::new()),
        Value::Bytes(payload.to_vec()),
    ]);
    let mut buf = Vec::new();
    ciborium::into_writer(&structure, &mut buf)
        .map_err(|e| VerifyError::Decode(format!("sig structure encode: {e}")))?;
    Ok(buf)
}

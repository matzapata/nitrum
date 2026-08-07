//! Attestation document types and payload parsing.

use crate::error::VerifyError;
use ciborium::value::Value;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Options for [`crate::verify_attestation`].
#[derive(Debug, Clone, Default)]
pub struct VerifyOptions {
    /// When true, failures may include the underlying [`crate::VerifyError`].
    pub debug: bool,
    /// Override trusted root (PEM or DER). Defaults to the AWS Nitro root CA.
    pub trusted_root: Option<Vec<u8>>,
    /// When set, document nonce must match these bytes (constant-time).
    pub nonce: Option<Vec<u8>>,
    /// Maximum age of the document timestamp relative to now, in milliseconds.
    pub max_age_ms: Option<u64>,
    /// Expected PCR hex values keyed like `pcr0` (case-insensitive hex compare).
    pub expected_pcrs: Option<BTreeMap<String, String>>,
}

/// Internal decoded attestation payload (raw bytes for crypto fields).
#[derive(Debug, Clone)]
pub struct AttestationDocument {
    /// DER-encoded X.509 signing certificate.
    pub certificate: Vec<u8>,
    /// Intermediate CA certificates (DER), root excluded.
    pub cabundle: Vec<Vec<u8>>,
    /// Nitro module identifier.
    pub module_id: String,
    /// Document creation time in milliseconds since UNIX epoch.
    pub timestamp: u64,
    /// Digest algorithm name (always `SHA384` for Nitro).
    pub digest: String,
    /// PCR index → raw measurement bytes.
    pub pcrs: BTreeMap<u32, Vec<u8>>,
    /// Optional binding bytes (Nitrum: SHA-256 of TLS leaf DER).
    pub public_key: Option<Vec<u8>>,
    /// Optional nonce for replay protection.
    pub nonce: Option<Vec<u8>>,
    /// Optional opaque user data.
    pub user_data: Option<Vec<u8>>,
}

/// Validated attestation document with hex-encoded PCRs.
#[derive(Debug, Clone)]
pub struct ParsedAttestation {
    /// Nitro module identifier.
    pub module_id: String,
    /// Document creation time in milliseconds since UNIX epoch.
    pub timestamp: u64,
    /// Digest algorithm name.
    pub digest: String,
    /// Hex-encoded PCR values keyed by `pcr{n}`.
    pub pcrs: BTreeMap<String, String>,
    /// Optional binding bytes.
    pub public_key: Option<Vec<u8>>,
    /// Optional nonce.
    pub nonce: Option<Vec<u8>>,
    /// Optional user data.
    pub user_data: Option<Vec<u8>>,
}

/// Parse the CBOR attestation payload into an [`AttestationDocument`].
pub fn parse_attestation_payload(payload: &[u8]) -> Result<AttestationDocument, VerifyError> {
    let value: Value = ciborium::from_reader(payload)
        .map_err(|e| VerifyError::Parse(format!("cbor payload: {e}")))?;
    let Value::Map(entries) = value else {
        return Err(VerifyError::Parse("payload is not a CBOR map".into()));
    };

    let get = |key: &str| -> Result<&Value, VerifyError> {
        entries
            .iter()
            .find(|(k, _)| match k {
                Value::Text(t) => t == key,
                _ => false,
            })
            .map(|(_, v)| v)
            .ok_or_else(|| VerifyError::Parse(format!("Missing required field: {key}")))
    };

    let module_id = match get("module_id")? {
        Value::Text(s) => s.clone(),
        _ => return Err(VerifyError::Parse("module_id must be text".into())),
    };

    let timestamp = match get("timestamp")? {
        Value::Integer(i) => i128::from(*i)
            .try_into()
            .map_err(|_| VerifyError::Parse("timestamp out of range".into()))?,
        _ => return Err(VerifyError::Parse("timestamp must be integer".into())),
    };

    let digest = match get("digest")? {
        Value::Text(s) => s.clone(),
        _ => return Err(VerifyError::Parse("digest must be text".into())),
    };

    let pcrs = parse_pcrs(get("pcrs")?)?;
    let certificate = bytes_value(get("certificate")?, "certificate")?;
    let cabundle = parse_cabundle(get("cabundle")?)?;

    let public_key = optional_bytes(&entries, "public_key")?;
    let nonce = optional_bytes(&entries, "nonce")?;
    let user_data = optional_bytes(&entries, "user_data")?;

    Ok(AttestationDocument {
        certificate,
        cabundle,
        module_id,
        timestamp,
        digest,
        pcrs,
        public_key,
        nonce,
        user_data,
    })
}

pub fn to_parsed(attestation: &AttestationDocument) -> ParsedAttestation {
    let mut pcrs = BTreeMap::new();
    for (index, value) in &attestation.pcrs {
        pcrs.insert(format!("pcr{index}"), hex::encode(value));
    }
    ParsedAttestation {
        module_id: attestation.module_id.clone(),
        timestamp: attestation.timestamp,
        digest: attestation.digest.clone(),
        pcrs,
        public_key: attestation.public_key.clone(),
        nonce: attestation.nonce.clone(),
        user_data: attestation.user_data.clone(),
    }
}

/// Current wall-clock time in milliseconds since UNIX epoch.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn parse_pcrs(value: &Value) -> Result<BTreeMap<u32, Vec<u8>>, VerifyError> {
    let mut out = BTreeMap::new();
    match value {
        Value::Map(entries) => {
            for (k, v) in entries {
                let index = match k {
                    Value::Integer(i) => u32::try_from(i128::from(*i))
                        .map_err(|_| VerifyError::Parse("pcr index out of range".into()))?,
                    _ => return Err(VerifyError::Parse("pcr key must be integer".into())),
                };
                let bytes = bytes_value(v, "pcr value")?;
                out.insert(index, bytes);
            }
        }
        _ => return Err(VerifyError::Parse("pcrs must be a map".into())),
    }
    Ok(out)
}

fn parse_cabundle(value: &Value) -> Result<Vec<Vec<u8>>, VerifyError> {
    let Value::Array(items) = value else {
        return Err(VerifyError::Parse("cabundle must be an array".into()));
    };
    items
        .iter()
        .map(|v| bytes_value(v, "cabundle entry"))
        .collect()
}

fn bytes_value(value: &Value, field: &str) -> Result<Vec<u8>, VerifyError> {
    match value {
        Value::Bytes(b) => Ok(b.clone()),
        _ => Err(VerifyError::Parse(format!("{field} must be bytes"))),
    }
}

fn optional_bytes(entries: &[(Value, Value)], key: &str) -> Result<Option<Vec<u8>>, VerifyError> {
    let Some((_, value)) = entries.iter().find(|(k, _)| match k {
        Value::Text(t) => t == key,
        _ => false,
    }) else {
        return Ok(None);
    };
    match value {
        Value::Null => Ok(None),
        Value::Bytes(b) => Ok(Some(b.clone())),
        _ => Err(VerifyError::Parse(format!("{key} must be bytes or null"))),
    }
}

//! NAPI bindings over the `verify` crate for Node.js.

#![deny(clippy::all)]

use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::collections::BTreeMap;
use verify::{
    AttestationResult as CoreAttestationResult, ParsedAttestation as CoreParsedAttestation,
    VerifyOptions as CoreVerifyOptions, verify_attestation as verify_attestation_core,
    verify_tls_leaf_binds,
};

/// Options accepted by [`verify_attestation`].
#[napi(object)]
#[derive(Default)]
pub struct VerifyOptions {
    /// Include underlying error details on failure.
    pub debug: Option<bool>,
    /// Override trusted root (PEM string or DER bytes).
    pub trusted_root: Option<Either<String, Uint8Array>>,
    /// Expected nonce bytes.
    pub nonce: Option<Uint8Array>,
    /// Max document age in milliseconds.
    pub max_age_ms: Option<f64>,
    /// Expected PCR hex map (e.g. `{ pcr0: "..." }`).
    pub expected_pcrs: Option<BTreeMap<String, String>>,
}

/// Public attestation fields returned to JavaScript.
#[napi(object)]
pub struct ParsedAttestation {
    /// Nitro module identifier.
    #[napi(js_name = "module_id")]
    pub module_id: String,
    /// Document timestamp (ms since UNIX epoch).
    pub timestamp: f64,
    /// Digest algorithm name.
    pub digest: String,
    /// Hex-encoded PCR map.
    pub pcrs: BTreeMap<String, String>,
    /// Optional binding bytes.
    #[napi(js_name = "public_key")]
    pub public_key: Option<Uint8Array>,
    /// Optional nonce.
    pub nonce: Option<Uint8Array>,
    /// Optional user data.
    #[napi(js_name = "user_data")]
    pub user_data: Option<Uint8Array>,
}

/// Result object for attestation verification.
#[napi(object)]
pub struct AttestationResult {
    /// Whether verification succeeded.
    pub valid: bool,
    /// Parsed document when `valid` is true.
    pub document: Option<ParsedAttestation>,
    /// Failure reason when `valid` is false.
    pub reason: Option<String>,
    /// Debug error string when requested.
    pub error: Option<String>,
}

/// Verify an AWS Nitro attestation document.
#[napi]
pub fn verify_attestation(
    document: Uint8Array,
    options: Option<VerifyOptions>,
) -> Result<AttestationResult> {
    let opts = options.unwrap_or_default();
    let mut vo = CoreVerifyOptions {
        debug: opts.debug.unwrap_or(false),
        ..Default::default()
    };
    if let Some(root) = opts.trusted_root {
        vo.trusted_root = Some(match root {
            Either::A(pem) => pem.into_bytes(),
            Either::B(bytes) => bytes.to_vec(),
        });
    }
    if let Some(nonce) = opts.nonce {
        vo.nonce = Some(nonce.to_vec());
    }
    if let Some(max_age) = opts.max_age_ms {
        if max_age > 0.0 && max_age.is_finite() {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            {
                vo.max_age_ms = Some(max_age as u64);
            }
        } else {
            // Preserve prior semantics: non-positive maxAgeMs fails during policy checks.
            vo.max_age_ms = Some(0);
        }
    }
    if let Some(pcrs) = opts.expected_pcrs {
        vo.expected_pcrs = Some(pcrs);
    }

    Ok(map_result(verify_attestation_core(document.as_ref(), &vo)))
}

/// Verify that `public_key` equals SHA-256 of the TLS leaf DER.
#[napi]
#[must_use]
pub fn verify_tls_leaf_binds_attestation(
    public_key: Option<Uint8Array>,
    tls_leaf_certificate_der: Uint8Array,
) -> bool {
    let pk = public_key.as_ref().map(AsRef::as_ref);
    verify_tls_leaf_binds(pk, tls_leaf_certificate_der.as_ref())
}

fn map_result(result: CoreAttestationResult) -> AttestationResult {
    match result {
        CoreAttestationResult::Success(ok) => AttestationResult {
            valid: true,
            document: Some(map_document(ok.document)),
            reason: None,
            error: None,
        },
        CoreAttestationResult::Failure(fail) => AttestationResult {
            valid: false,
            document: None,
            reason: Some(fail.reason),
            error: fail.error.map(|e| e.to_string()),
        },
    }
}

fn map_document(doc: CoreParsedAttestation) -> ParsedAttestation {
    ParsedAttestation {
        module_id: doc.module_id,
        #[allow(clippy::cast_precision_loss)]
        timestamp: doc.timestamp as f64,
        digest: doc.digest,
        pcrs: doc.pcrs,
        public_key: doc.public_key.map(Uint8Array::from),
        nonce: doc.nonce.map(Uint8Array::from),
        user_data: doc.user_data.map(Uint8Array::from),
    }
}

//! KMS-backed DEK envelope: `GenerateDataKeyWithoutPlaintext` (AES-256) and `Decrypt`.
//!
//! **Enclave:** `Decrypt` uses a Nitro [`Recipient`](https://docs.aws.amazon.com/kms/latest/developerguide/cryptographic-attestation.html)
//! attestation. KMS returns `CiphertextForRecipient` as **RFC 5652 CMS**; OpenSSL unwraps it. The ephemeral key pair for the
//! recipient is generated with **OpenSSL** (AWS requires RSA-OAEP-SHA256 to the public key embedded in the attestation — not your CMK).
//!
//! **Pebbles / local:** same symmetric envelope; `Decrypt` runs **without** `Recipient` (no NSM attestation).

use crate::DataPlaneConfig;
use crate::constants::ENV_KMS_ENDPOINT_URL;
use crate::utils::env::optional_nonempty;
use anyhow::{Context, Result};
use async_trait::async_trait;
use aws_sdk_kms::primitives::Blob;
use aws_sdk_kms::types::DataKeySpec;
use std::sync::Arc;
use tracing::instrument;

/// Port for DEK envelope generate / unwrap (construction only).
#[async_trait]
pub trait Kms: Send + Sync {
    async fn generate_dek_envelope(&self) -> Result<Vec<u8>>;
    async fn decrypt_with_attestation(&self, ciphertext: &[u8]) -> Result<Vec<u8>>;
}

/// AWS KMS client bound to a specific key ID.
pub struct AwsKms {
    client: aws_sdk_kms::Client,
    key_id: String,
    kms_endpoint: Option<String>,
}

impl AwsKms {
    /// Build a client for `key_id` using `aws`.
    ///
    /// When [`ENV_KMS_ENDPOINT_URL`] is set and non-empty, routes API calls to that endpoint
    /// (local dev / LocalStack); otherwise uses the regional KMS endpoint from the SDK config.
    #[must_use]
    pub fn new(aws: Arc<aws_config::SdkConfig>, key_id: impl Into<String>) -> Self {
        let kms_endpoint = optional_nonempty(ENV_KMS_ENDPOINT_URL);
        let mut builder = aws_sdk_kms::config::Builder::from(aws.as_ref());
        if let Some(ref endpoint) = kms_endpoint {
            builder = builder.endpoint_url(endpoint);
        }
        let client = aws_sdk_kms::Client::from_conf(builder.build());
        Self {
            client,
            key_id: key_id.into(),
            kms_endpoint,
        }
    }

    /// Build from resolved [`DataPlaneConfig`] (`aws` + `kms_key_id`).
    #[must_use]
    pub fn from_config(config: &DataPlaneConfig) -> Self {
        Self::new(config.aws.clone(), config.kms_key_id.clone())
    }

    fn kms_call_context(&self, operation: &str) -> String {
        format!(
            "KMS {operation} (key_id={}, endpoint={})",
            self.key_id,
            self.kms_endpoint
                .as_deref()
                .unwrap_or("(default AWS KMS HTTPS endpoint)")
        )
    }

    #[cfg(feature = "enclave")]
    async fn decrypt_with_attestation_enclave(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        use openssl::pkey::PKey;
        use openssl::rsa::Rsa;

        let rsa = Rsa::generate(2048).context(
            "OpenSSL: generate ephemeral RSA-2048 for KMS Recipient (required by AWS Nitro attestation API)",
        )?;
        let pkey = PKey::from_rsa(rsa).context("OpenSSL: PKey from ephemeral RSA")?;
        let public_der = pkey
            .public_key_to_der()
            .context("OpenSSL: export SPKI DER for attestation public_key field")?;

        let attestation_doc =
            crate::crypto::attest::get_attestation_doc(None, Some(public_der), None)
                .map_err(|e| anyhow::anyhow!("attestation failed: {e}"))?;

        let recipient = build_recipient_info(attestation_doc);

        let ctx = self.kms_call_context("Decrypt (recipient attestation, symmetric data key)");
        let resp = self
            .client
            .decrypt()
            .key_id(&self.key_id)
            .ciphertext_blob(Blob::new(ciphertext))
            .recipient(recipient)
            .send()
            .await
            .with_context(|| access_denied_recipient_context(&ctx))?;

        let ciphertext_for_recipient = resp
            .ciphertext_for_recipient()
            .with_context(|| {
                format!("{ctx}; Decrypt succeeded but ciphertext_for_recipient missing")
            })?
            .as_ref();

        unwrap_ciphertext_for_recipient_cms(ciphertext_for_recipient, &pkey, &ctx)
    }

    #[cfg(not(feature = "enclave"))]
    async fn decrypt_symmetric_envelope_plain(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        let ctx = self.kms_call_context("Decrypt (symmetric envelope, no recipient)");
        let resp = self
            .client
            .decrypt()
            .key_id(&self.key_id)
            .ciphertext_blob(Blob::new(ciphertext))
            .send()
            .await
            .with_context(|| {
                format!("{ctx}. IAM kms:Decrypt. CMK must match the key that wrapped the data key.")
            })?;

        resp.plaintext()
            .map(|b| b.as_ref().to_vec())
            .with_context(|| format!("{ctx}; response had no plaintext"))
    }
}

#[async_trait]
impl Kms for AwsKms {
    /// [`GenerateDataKeyWithoutPlaintext`](https://docs.aws.amazon.com/kms/latest/APIReference/API_GenerateDataKeyWithoutPlaintext.html) (AES-256). Persist the returned blob; recover bytes via [`Kms::decrypt_with_attestation`].
    #[instrument(name = "kms.generate_data_key", skip(self), fields(otel.kind = "client"), err)]
    async fn generate_dek_envelope(&self) -> Result<Vec<u8>> {
        let ctx = self.kms_call_context("GenerateDataKeyWithoutPlaintext");
        let start = std::time::Instant::now();
        let resp = self
            .client
            .generate_data_key_without_plaintext()
            .key_id(&self.key_id)
            .key_spec(DataKeySpec::Aes256)
            .send()
            .await;
        telemetry::metrics::record_kms(
            "generate_data_key",
            start.elapsed().as_secs_f64() * 1000.0,
            resp.is_ok(),
        );
        let resp = resp.with_context(|| {
            format!(
                "{ctx}; CMK must be symmetric ENCRYPT_DECRYPT. IAM: kms:GenerateDataKeyWithoutPlaintext."
            )
        })?;

        resp.ciphertext_blob()
            .map(|b| b.as_ref().to_vec())
            .with_context(|| format!("{ctx}; empty ciphertext_blob"))
    }

    /// Unwrap the stored envelope: **enclave** = attested `Decrypt` + CMS unwrap; **non-enclave** = plain `Decrypt`.
    #[instrument(name = "kms.decrypt", skip(self, ciphertext), fields(otel.kind = "client"), err)]
    async fn decrypt_with_attestation(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        let start = std::time::Instant::now();
        #[cfg(feature = "enclave")]
        let result = self.decrypt_with_attestation_enclave(ciphertext).await;
        #[cfg(not(feature = "enclave"))]
        let result = self.decrypt_symmetric_envelope_plain(ciphertext).await;
        telemetry::metrics::record_kms(
            "decrypt",
            start.elapsed().as_secs_f64() * 1000.0,
            result.is_ok(),
        );
        result
    }
}

#[cfg(feature = "enclave")]
fn access_denied_recipient_context(ctx: &str) -> String {
    format!("{ctx}. If AccessDenied: kms:Decrypt, key policy, or attestation / recipient mismatch.")
}

/// Build the KMS Recipient for attested decrypt (testable without calling KMS).
#[cfg(feature = "enclave")]
fn build_recipient_info(attestation_doc: Vec<u8>) -> aws_sdk_kms::types::RecipientInfo {
    use aws_sdk_kms::types::{KeyEncryptionMechanism, RecipientInfo};
    RecipientInfo::builder()
        .key_encryption_algorithm(KeyEncryptionMechanism::RsaesOaepSha256)
        .attestation_document(Blob::new(attestation_doc))
        .build()
}

/// KMS `CiphertextForRecipient`: **RFC 5652 CMS** (`ContentInfo`), not a raw ciphertext block.
#[cfg(feature = "enclave")]
fn unwrap_ciphertext_for_recipient_cms(
    der: &[u8],
    pkey: &openssl::pkey::PKey<openssl::pkey::Private>,
    kms_ctx: &str,
) -> Result<Vec<u8>> {
    use openssl::cms::CmsContentInfo;

    let cms = CmsContentInfo::from_der(der).with_context(|| {
        format!(
            "{kms_ctx}; OpenSSL could not parse CiphertextForRecipient as CMS ContentInfo (input length {} bytes). \
             AWS returns RFC 5652 PKCS#7 for Nitro Recipient responses.",
            der.len()
        )
    })?;

    cms.decrypt_without_cert_check(pkey.as_ref())
        .with_context(|| {
            format!(
                "{kms_ctx}; OpenSSL CMS_decrypt failed (input length {} bytes). Source error is attached as cause.",
                der.len()
            )
        })
}

#[cfg(all(test, feature = "enclave"))]
mod tests {
    use super::*;
    use openssl::pkey::PKey;
    use openssl::rsa::Rsa;

    #[test]
    fn access_denied_context_mentions_attestation_recipient() {
        let msg = access_denied_recipient_context("KMS Decrypt (key_id=test)");
        assert!(msg.contains("attestation / recipient mismatch"));
        assert!(msg.contains("AccessDenied"));
    }

    #[test]
    fn build_recipient_info_embeds_attestation_bytes() {
        let doc = b"fake-attestation-document".to_vec();
        let recipient = build_recipient_info(doc.clone());
        assert_eq!(
            recipient
                .attestation_document()
                .map(std::convert::AsRef::as_ref),
            Some(doc.as_slice())
        );
    }

    #[test]
    fn unwrap_cms_rejects_garbage() {
        let rsa = Rsa::generate(2048).unwrap();
        let pkey = PKey::from_rsa(rsa).unwrap();
        let err = unwrap_ciphertext_for_recipient_cms(b"not-cms", &pkey, "test-ctx").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("CMS ContentInfo") || msg.contains("parse"));
    }
}

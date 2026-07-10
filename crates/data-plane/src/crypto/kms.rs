//! KMS-backed DEK lifecycle: only **`GenerateDataKeyWithoutPlaintext`** (AES-256) and **Decrypt**.
//!
//! **Enclave:** `Decrypt` uses a Nitro [`Recipient`](https://docs.aws.amazon.com/kms/latest/developerguide/cryptographic-attestation.html)
//! attestation. KMS returns `CiphertextForRecipient` as **RFC 5652 CMS**; OpenSSL unwraps it. The ephemeral key pair for the
//! recipient is generated with **OpenSSL** (AWS requires RSA-OAEP-SHA256 to the public key embedded in the attestation — not your CMK).
//!
//! **Pebbles / local:** same symmetric envelope; `Decrypt` runs **without** `Recipient` (no NSM attestation).

use crate::config::DataPlaneConfig;
use anyhow::{Context, Result};
use aws_sdk_kms::primitives::Blob;
use aws_sdk_kms::types::DataKeySpec;
use tracing::instrument;

/// KMS client bound to a specific key ID.
pub struct Kms {
    client: aws_sdk_kms::Client,
    key_id: String,
    aws_region: String,
    kms_endpoint: Option<String>,
}

impl Kms {
    pub fn new(config: &DataPlaneConfig) -> Self {
        let mut builder = aws_sdk_kms::config::Builder::from(config.aws_sdk_config.as_ref());
        if let Some(ref endpoint) = config.kms_endpoint {
            builder = builder.endpoint_url(endpoint);
        }
        let client = aws_sdk_kms::Client::from_conf(builder.build());
        Self {
            client,
            key_id: config.kms_key_id.clone(),
            aws_region: config.aws_region.clone(),
            kms_endpoint: config.kms_endpoint.clone(),
        }
    }

    fn kms_call_context(&self, operation: &str) -> String {
        format!(
            "KMS {operation} (key_id={}, region={}, endpoint={})",
            self.key_id,
            self.aws_region,
            self.kms_endpoint
                .as_deref()
                .unwrap_or("(default AWS KMS HTTPS endpoint)")
        )
    }

    /// [`GenerateDataKeyWithoutPlaintext`](https://docs.aws.amazon.com/kms/latest/APIReference/API_GenerateDataKeyWithoutPlaintext.html) (AES-256). Persist the returned blob; recover bytes via [`Self::decrypt_with_attestation`].
    #[instrument(name = "kms.generate_data_key", skip(self), fields(otel.kind = "client"), err)]
    pub async fn generate_dek_envelope(&self) -> Result<Vec<u8>> {
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
    pub async fn decrypt_with_attestation(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
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

    #[cfg(feature = "enclave")]
    async fn decrypt_with_attestation_enclave(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        use aws_sdk_kms::types::{KeyEncryptionMechanism, RecipientInfo};
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

        let recipient = RecipientInfo::builder()
            .key_encryption_algorithm(KeyEncryptionMechanism::RsaesOaepSha256)
            .attestation_document(Blob::new(attestation_doc))
            .build();

        let ctx = self.kms_call_context("Decrypt (recipient attestation, symmetric data key)");
        let resp = self
            .client
            .decrypt()
            .key_id(&self.key_id)
            .ciphertext_blob(Blob::new(ciphertext))
            .recipient(recipient)
            .send()
            .await
            .with_context(|| {
                format!(
                    "{ctx}. If AccessDenied: kms:Decrypt, key policy, or attestation / recipient mismatch."
                )
            })?;

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

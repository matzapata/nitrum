//! KMS-backed DEK envelope: `GenerateDataKeyWithoutPlaintext` (AES-256) and `Decrypt`.
//!
//! **Enclave:** `Decrypt` uses a Nitro [`Recipient`](https://docs.aws.amazon.com/kms/latest/developerguide/cryptographic-attestation.html)
//! attestation. KMS returns `CiphertextForRecipient` as **RFC 5652 CMS**; OpenSSL unwraps it. The ephemeral key pair for the
//! recipient is generated with **OpenSSL** (AWS requires RSA-OAEP-SHA256 to the public key embedded in the attestation — not your CMK).

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
}

impl AwsKms {
    /// Build a client for `key_id` using `aws`.
    ///
    /// When [`ENV_KMS_ENDPOINT_URL`] is set and non-empty, routes API calls to that endpoint
    /// (local dev / LocalStack); otherwise uses the regional KMS endpoint from the SDK config.
    #[must_use]
    pub fn new(aws: Arc<aws_config::SdkConfig>, key_id: impl Into<String>) -> Self {
        let mut builder = aws_sdk_kms::config::Builder::from(aws.as_ref());
        if let Some(endpoint) = optional_nonempty(ENV_KMS_ENDPOINT_URL) {
            builder = builder.endpoint_url(endpoint);
        }
        let client = aws_sdk_kms::Client::from_conf(builder.build());
        Self {
            client,
            key_id: key_id.into(),
        }
    }

    /// Build from resolved [`DataPlaneConfig`] (`aws` + `kms_key_id`).
    #[must_use]
    pub fn from_config(config: &DataPlaneConfig) -> Self {
        Self::new(config.aws.clone(), config.kms_key_id.clone())
    }

    /// Build the KMS Recipient for attested decrypt.
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
    ) -> Result<Vec<u8>> {
        use openssl::cms::CmsContentInfo;

        let cms = CmsContentInfo::from_der(der).context("parse CiphertextForRecipient as CMS")?;
        cms.decrypt_without_cert_check(pkey.as_ref())
            .context("CMS decrypt CiphertextForRecipient")
    }
}

#[async_trait]
impl Kms for AwsKms {
    /// [`GenerateDataKeyWithoutPlaintext`](https://docs.aws.amazon.com/kms/latest/APIReference/API_GenerateDataKeyWithoutPlaintext.html) (AES-256). Persist the returned blob; recover bytes via [`Kms::decrypt_with_attestation`].
    #[instrument(name = "kms.generate_data_key", skip(self), fields(otel.kind = "client"), err)]
    async fn generate_dek_envelope(&self) -> Result<Vec<u8>> {
        let resp = self
            .client
            .generate_data_key_without_plaintext()
            .key_id(&self.key_id)
            .key_spec(DataKeySpec::Aes256)
            .send()
            .await
            .context("KMS GenerateDataKeyWithoutPlaintext")?;

        resp.ciphertext_blob()
            .map(|b| b.as_ref().to_vec())
            .context("GenerateDataKeyWithoutPlaintext returned empty ciphertext_blob")
    }

    /// Unwrap the stored envelope: **enclave** = attested `Decrypt` + CMS unwrap; **non-enclave** = plain `Decrypt`.
    #[instrument(name = "kms.decrypt", skip(self, ciphertext), fields(otel.kind = "client"), err)]
    async fn decrypt_with_attestation(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        #[cfg(feature = "enclave")]
        {
            use openssl::pkey::PKey;
            use openssl::rsa::Rsa;

            let rsa =
                Rsa::generate(2048).context("generate ephemeral RSA-2048 for KMS Recipient")?;
            let pkey = PKey::from_rsa(rsa).context("PKey from ephemeral RSA")?;
            let public_der = pkey
                .public_key_to_der()
                .context("export SPKI DER for attestation public_key")?;

            let attestation_doc =
                crate::crypto::attest::get_attestation_doc(None, Some(public_der), None)
                    .map_err(|e| anyhow::anyhow!("attestation failed: {e}"))?;

            let recipient = Self::build_recipient_info(attestation_doc);

            let resp = self
                .client
                .decrypt()
                .key_id(&self.key_id)
                .ciphertext_blob(Blob::new(ciphertext))
                .recipient(recipient)
                .send()
                .await
                .context("KMS Decrypt (recipient)")?;

            let ciphertext_for_recipient = resp
                .ciphertext_for_recipient()
                .context("Decrypt succeeded but ciphertext_for_recipient missing")?
                .as_ref();

            Self::unwrap_ciphertext_for_recipient_cms(ciphertext_for_recipient, &pkey)
        }

        #[cfg(not(feature = "enclave"))]
        {
            let resp = self
                .client
                .decrypt()
                .key_id(&self.key_id)
                .ciphertext_blob(Blob::new(ciphertext))
                .send()
                .await
                .context("KMS Decrypt")?;

            resp.plaintext()
                .map(|b| b.as_ref().to_vec())
                .context("Decrypt response had no plaintext")
        }
    }
}

#[cfg(all(test, feature = "enclave"))]
mod tests {
    use super::*;

    #[test]
    fn build_recipient_info_embeds_attestation_bytes() {
        let doc = b"fake-attestation-document".to_vec();
        let recipient = AwsKms::build_recipient_info(doc.clone());
        assert_eq!(
            recipient
                .attestation_document()
                .map(std::convert::AsRef::as_ref),
            Some(doc.as_slice())
        );
    }
}

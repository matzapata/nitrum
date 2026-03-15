//! KMS helpers for DEK envelope encryption.
//!
//! [`Kms`] wraps the AWS client and key ID
//! [`Kms::decrypt_with_attestation`] to decrypt with attestation.
//!
//! In non-enclave (dev) builds the attestation step is skipped and decryption
//! is performed locally using an RSA private key from `NITRUM_DEV_RSA_PRIVATE_KEY`.

use anyhow::{Context, Result};
use aws_sdk_kms::primitives::Blob;
use aws_sdk_kms::types::EncryptionAlgorithmSpec;

/// KMS client bound to a specific key ID.
pub struct Kms {
    client: aws_sdk_kms::Client,
    key_id: String,
}

impl Kms {
    pub async fn new(key_id: String) -> Self {
        let aws_sdk_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .load()
            .await;
        let client = aws_sdk_kms::Client::new(&aws_sdk_config);

        Self { client, key_id }
    }

    /// Encrypt `plaintext` under the configured RSA-2048 KMS key (RSAES_OAEP_SHA_256).
    #[cfg(feature = "enclave")]
    pub async fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let resp = self
            .client
            .encrypt()
            .key_id(&self.key_id)
            .plaintext(Blob::new(plaintext))
            .encryption_algorithm(EncryptionAlgorithmSpec::RsaesOaepSha256)
            .send()
            .await
            .context("KMS Encrypt failed")?;

        resp.ciphertext_blob()
            .map(|b| b.as_ref().to_vec())
            .context("KMS Encrypt returned no ciphertext")
    }

    /// Dev / non-enclave: encrypt locally with public key from NITRUM_DEV_RSA_PRIVATE_KEY, or KMS if unset.
    #[cfg(not(feature = "enclave"))]
    pub async fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        use rsa::{
            Oaep, RsaPublicKey, pkcs1::DecodeRsaPrivateKey as _, pkcs8::DecodePrivateKey as _,
        };
        use sha2::Sha256;

        if let Ok(pem) = std::env::var("NITRUM_DEV_RSA_PRIVATE_KEY") {
            let private_key = rsa::RsaPrivateKey::from_pkcs8_pem(&pem)
                .or_else(|_| rsa::RsaPrivateKey::from_pkcs1_pem(&pem))
                .context("failed to parse RSA private key from NITRUM_DEV_RSA_PRIVATE_KEY")?;

            let public_key = RsaPublicKey::from(&private_key);
            let mut rng = rand::thread_rng();
            return public_key
                .encrypt(&mut rng, Oaep::new::<Sha256>(), plaintext)
                .context("local RSA-OAEP-SHA256 encrypt failed");
        }

        let resp = self
            .client
            .encrypt()
            .key_id(&self.key_id)
            .plaintext(Blob::new(plaintext))
            .encryption_algorithm(EncryptionAlgorithmSpec::RsaesOaepSha256)
            .send()
            .await
            .context("KMS Encrypt failed")?;

        resp.ciphertext_blob()
            .map(|b| b.as_ref().to_vec())
            .context("KMS Encrypt returned no ciphertext")
    }

    /// Decrypt using Nitro attestation-based Recipient (enclave only).
    #[cfg(feature = "enclave")]
    pub async fn decrypt_with_attestation(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        use aws_sdk_kms::types::{KeyEncryptionMechanism, RecipientInfo};
        use rsa::{Oaep, RsaPrivateKey, pkcs8::EncodePublicKey as _};
        use sha2::Sha256;

        let mut rng = rand::thread_rng();
        let private_key =
            RsaPrivateKey::new(&mut rng, 2048).context("failed to generate ephemeral RSA key")?;
        let public_key_der = rsa::RsaPublicKey::from(&private_key)
            .to_public_key_der()
            .context("failed to DER-encode ephemeral public key")?
            .to_vec();

        let attestation_doc =
            crate::crypto::attest::get_attestation_doc(None, Some(public_key_der), None)
                .map_err(|e| anyhow::anyhow!("attestation failed: {e}"))?;

        let recipient = RecipientInfo::builder()
            .key_encryption_algorithm(KeyEncryptionMechanism::RsaesOaepSha256)
            .attestation_document(Blob::new(attestation_doc))
            .build();

        let resp = self
            .client
            .decrypt()
            .key_id(&self.key_id)
            .ciphertext_blob(Blob::new(ciphertext))
            .encryption_algorithm(EncryptionAlgorithmSpec::RsaesOaepSha256)
            .recipient(recipient)
            .send()
            .await
            .context("KMS Decrypt (with attestation) failed")?;

        let ciphertext_for_recipient = resp
            .ciphertext_for_recipient()
            .context("KMS did not return ciphertext_for_recipient")?
            .as_ref();

        let plaintext = private_key
            .decrypt(Oaep::new::<Sha256>(), ciphertext_for_recipient)
            .context("failed to decrypt ciphertext_for_recipient with ephemeral key")?;

        Ok(plaintext)
    }

    /// Dev / non-enclave: decrypt locally with NITRUM_DEV_RSA_PRIVATE_KEY.
    #[cfg(not(feature = "enclave"))]
    pub async fn decrypt_with_attestation(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        use rsa::{Oaep, pkcs1::DecodeRsaPrivateKey as _, pkcs8::DecodePrivateKey as _};
        use sha2::Sha256;

        let pem = std::env::var("NITRUM_DEV_RSA_PRIVATE_KEY")
            .context("NITRUM_DEV_RSA_PRIVATE_KEY not set (required for dev-mode decrypt)")?;

        let private_key = rsa::RsaPrivateKey::from_pkcs8_pem(&pem)
            .or_else(|_| rsa::RsaPrivateKey::from_pkcs1_pem(&pem))
            .context("failed to parse RSA private key from NITRUM_DEV_RSA_PRIVATE_KEY")?;

        private_key
            .decrypt(Oaep::new::<Sha256>(), ciphertext)
            .context("RSA-OAEP-SHA256 decrypt failed")
    }
}

use super::attest::get_attestation_doc;
use super::kms::Kms;
use super::rng::rand_bytes;
use crate::config::RuntimeConfig;
use crate::storage::{StorageClient, keys};
use crate::utils::leader::Leader;
use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};
use anyhow::{Context, Result};
use std::sync::Arc;

const NONCE_LEN: usize = 12;

// ── CryptoClient ────────────────────────────────────────────────────────────────────

/// Client that holds the DEK-backed crypto. Bootstrap with [`CryptoClient::new`], then use encrypt/decrypt.
pub struct CryptoClient {
    cipher: Aes256Gcm,
}

impl CryptoClient {
    /// Bootstrap DEK from storage (fetch and decrypt with KMS, or create as leader and store).
    pub async fn new(config: RuntimeConfig, storage: Arc<StorageClient>) -> Result<Self> {
        let kms = Kms::new(config.kms_key_id.clone()).await;

        let leader = Leader::new(
            storage.clone(),
            config.instance_id.clone(),
            keys::CRYPTO_LEADER_KEY.to_string(),
        );

        loop {
            if let Some(encrypted_dek) = storage
                .get_object(keys::DEK_OBJECT_KEY)
                .await
                .context("failed to retrieve DEK from storage")?
            {
                tracing::info!("DEK found in storage, decrypting with KMS");
                let dek = kms
                    .decrypt_with_attestation(&encrypted_dek)
                    .await
                    .context("failed to decrypt DEK with KMS")?;
                let cipher =
                Aes256Gcm::new_from_slice(&dek).map_err(|e| anyhow::anyhow!("invalid key: {e}"))?;
                return Ok(Self { cipher });
            }

            if let Some(_guard) = leader.try_acquire_leader().await? {
                tracing::info!("no DEK in storage, generating a new one (leader)");
                let dek = rand_bytes(32).context("failed to generate DEK")?;

                let encrypted_dek = kms
                    .encrypt(&dek)
                    .await
                    .context("failed to encrypt DEK with KMS")?;

                let stored = storage
                    .put_object(keys::DEK_OBJECT_KEY, &encrypted_dek)
                    .await
                    .context("failed to store DEK in storage")?;

                if stored {
                    tracing::info!("DEK stored successfully");
                    let cipher =
                    Aes256Gcm::new_from_slice(&dek).map_err(|e| anyhow::anyhow!("invalid key: {e}"))?;
                    return Ok(Self { cipher });
                }
            } else {
                tracing::debug!("no DEK yet and not leader, retrying read");
            }

            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }

    /// Encrypt `data` with the DEK. Returns `nonce (12 bytes) || ciphertext+tag`.
    pub fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = self
            .cipher
            .encrypt(&nonce, data)
            .map_err(|e| anyhow::anyhow!("AES-GCM encrypt error: {e}"))?;

        let mut out = nonce.to_vec();
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    /// Decrypt a blob produced by [`CryptoClient::encrypt`].
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        anyhow::ensure!(data.len() > NONCE_LEN, "ciphertext too short");
        let (nonce_bytes, ciphertext) = data.split_at(NONCE_LEN);
        let nonce = Nonce::from_slice(nonce_bytes);

        self.cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow::anyhow!("AES-GCM decrypt error: {e}"))
    }

    /// Get an attestation document for the current enclave.
    pub fn get_attestation_doc(
        &self,
        nonce: Option<Vec<u8>>,
        public_key: Option<Vec<u8>>,
        user_data: Option<Vec<u8>>,
    ) -> Result<Vec<u8>> {
        get_attestation_doc(nonce, public_key, user_data).map_err(|e| anyhow::anyhow!("{}", e))
    }
}


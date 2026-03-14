use std::sync::Arc;

use anyhow::{Context, Result};
use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};

use crate::config::RuntimeConfig;
use crate::storage::{keys, StorageClient};
use crate::utils::leader::Leader;
use super::attest::get_attestation_doc;
use super::kms::Kms;
use super::rng::rand_bytes;

/// Symmetric encryption using a Data Encryption Key (DEK).
/// Output format: `nonce (12 B) || ciphertext+tag`.
pub struct Crypto {
    dek: Vec<u8>,
}

impl Crypto {
    /// Encrypt `data` with AES-256-GCM. Returns `nonce (12 bytes) || ciphertext+tag`.
    pub fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        let key = Key::<Aes256Gcm>::from_slice(&self.dek);
        let cipher = Aes256Gcm::new(key);
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = cipher
            .encrypt(&nonce, data)
            .map_err(|e| anyhow::anyhow!("AES-GCM encrypt error: {e}"))?;
        let mut out = nonce.to_vec();
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    /// Decrypt a blob produced by [`Crypto::encrypt`].
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        const NONCE_LEN: usize = 12;
        anyhow::ensure!(data.len() > NONCE_LEN, "ciphertext too short");
        let (nonce_bytes, ciphertext) = data.split_at(NONCE_LEN);
        let key = Key::<Aes256Gcm>::from_slice(&self.dek);
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(nonce_bytes);
        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow::anyhow!("AES-GCM decrypt error: {e}"))
    }
}

/// Client that holds the DEK-backed crypto. Bootstrap with [`CryptoClient::new`], then use encrypt/decrypt.
pub struct CryptoClient {
    crypto: Arc<Crypto>,
}

impl CryptoClient {
    /// Bootstrap DEK from storage (fetch and decrypt with KMS, or create as leader and store).
    pub async fn new(config: RuntimeConfig, storage: Arc<StorageClient>) -> Result<Self> {
        let sdk_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .load()
            .await;
        let kms_client = aws_sdk_kms::Client::new(&sdk_config);
        let kms = Kms::new(kms_client, config.kms_key_id.clone());

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
                return Ok(Self {
                    crypto: Arc::new(Crypto { dek }),
                });
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
                    return Ok(Self {
                        crypto: Arc::new(Crypto { dek }),
                    });
                }
            } else {
                tracing::debug!("no DEK yet and not leader, retrying read");
            }

            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }

    /// Encrypt `data` with the DEK. Returns `nonce (12 bytes) || ciphertext+tag`.
    pub fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        self.crypto.encrypt(data)
    }

    /// Decrypt a blob produced by [`CryptoClient::encrypt`].
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        self.crypto.decrypt(data)
    }

    pub fn get_attestation_doc(
        &self,
        nonce: Option<Vec<u8>>,
        public_key: Option<Vec<u8>>,
        user_data: Option<Vec<u8>>,
    ) -> Result<Vec<u8>> {
        get_attestation_doc(nonce, public_key, user_data).map_err(|e| anyhow::anyhow!("{}", e))
    }
}

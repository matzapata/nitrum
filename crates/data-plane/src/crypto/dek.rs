//! AES-GCM crypto and DEK bootstrap via [`Kms`].

use super::kms::Kms;
use crate::storage::Leader;
use crate::storage::ObjectStore;
use crate::storage::keys;
use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};
use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::Duration;

/// AES-GCM nonce length in bytes.
const NONCE_LEN: usize = 12;
/// AES-GCM authentication tag length in bytes.
const GCM_TAG_LEN: usize = 16;

/// DEK bootstrap retry duration.
const DEK_BOOTSTRAP_RETRY: Duration = if cfg!(test) {
    Duration::from_millis(10)
} else {
    Duration::from_secs(2)
};

/// Symmetric encrypt/decrypt backed by a DEK (pure crypto — no I/O).
pub trait Crypto: Send + Sync {
    /// Encrypt `data`. Returns `nonce (12 bytes) || ciphertext+tag`.
    fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>>;

    /// Decrypt a blob produced by [`Crypto::encrypt`].
    fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>>;
}

/// AES-256-GCM crypto once a DEK is loaded. Construct via [`AesGcmCrypto::from_kms`] or
/// [`AesGcmCrypto::from_dek`].
pub struct AesGcmCrypto {
    cipher: Aes256Gcm,
}

impl AesGcmCrypto {
    /// Build from a raw 32-byte DEK (tests and benches).
    pub fn from_dek(dek: &[u8]) -> Result<Self> {
        anyhow::ensure!(
            dek.len() == 32,
            "DEK must be 32 bytes (AES-256), got {} bytes",
            dek.len()
        );
        let cipher =
            Aes256Gcm::new_from_slice(dek).map_err(|e| anyhow::anyhow!("invalid key: {e}"))?;
        Ok(Self { cipher })
    }

    /// Load or create the DEK from storage (leader creates; others wait and decrypt with KMS).
    pub async fn from_kms<S, K>(storage: Arc<S>, kms: &K, instance_id: &str) -> Result<Self>
    where
        S: ObjectStore + 'static,
        K: Kms,
    {
        let leader = Leader::new(
            storage.clone(),
            instance_id.to_string(),
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
                    .context("load DEK: decrypt from storage")?;
                return Self::from_dek(&dek);
            }

            if let Some(_guard) = leader.try_acquire_leader().await? {
                tracing::info!("no DEK in storage, generating a new one (leader)");

                let encrypted_dek = kms
                    .generate_dek_envelope()
                    .await
                    .context("leader bootstrap: generate DEK envelope")?;
                let dek = kms
                    .decrypt_with_attestation(&encrypted_dek)
                    .await
                    .context("leader bootstrap: unwrap DEK envelope")?;

                let crypto = Self::from_dek(&dek)?;

                let stored = storage
                    .put_object(keys::DEK_OBJECT_KEY, &encrypted_dek)
                    .await
                    .context("failed to store DEK in storage")?;

                if stored {
                    tracing::info!("DEK stored successfully");
                    return Ok(crypto);
                }
            } else {
                tracing::debug!("no DEK yet and not leader, retrying read");
            }

            tokio::time::sleep(DEK_BOOTSTRAP_RETRY).await;
        }
    }
}

impl Crypto for AesGcmCrypto {
    fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = self
            .cipher
            .encrypt(&nonce, data)
            .map_err(|e| anyhow::anyhow!("AES-GCM encrypt error: {e}"))?;

        let mut out = Vec::with_capacity(NONCE_LEN + data.len() + GCM_TAG_LEN);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        anyhow::ensure!(data.len() > NONCE_LEN, "ciphertext too short");
        let (nonce_bytes, ciphertext) = data.split_at(NONCE_LEN);
        let nonce = Nonce::from_slice(nonce_bytes);

        self.cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow::anyhow!("AES-GCM decrypt error: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::memory::InMemoryObjectStore;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const DEK: [u8; 32] = [0x11; 32];
    const OTHER_DEK: [u8; 32] = [0x22; 32];

    /// Always fails encrypt/decrypt.
    struct FailingCrypto;

    impl Crypto for FailingCrypto {
        fn encrypt(&self, _data: &[u8]) -> Result<Vec<u8>> {
            anyhow::bail!("encrypt failed")
        }

        fn decrypt(&self, _data: &[u8]) -> Result<Vec<u8>> {
            anyhow::bail!("decrypt failed")
        }
    }

    /// Envelope bytes are the DEK plaintext (identity "KMS" for unit tests).
    struct IdentityKms {
        dek: [u8; 32],
        generates: AtomicUsize,
    }

    #[async_trait]
    impl Kms for IdentityKms {
        async fn generate_dek_envelope(&self) -> Result<Vec<u8>> {
            self.generates.fetch_add(1, Ordering::SeqCst);
            Ok(self.dek.to_vec())
        }

        async fn decrypt_with_attestation(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
            Ok(ciphertext.to_vec())
        }
    }

    #[test]
    fn encrypt_decrypt_round_trips_empty_small_and_binary() {
        let client = AesGcmCrypto::from_dek(&DEK).unwrap();
        for plaintext in [
            Vec::<u8>::new(),
            b"a".to_vec(),
            b"hello".to_vec(),
            (0u8..=255).collect::<Vec<_>>(),
            vec![0xABu8; 1024],
        ] {
            let ciphertext = client.encrypt(&plaintext).unwrap();
            assert!(ciphertext.len() > NONCE_LEN);

            let recovered = client.decrypt(&ciphertext).unwrap();
            assert_eq!(recovered, plaintext);
        }
    }

    #[test]
    fn decrypt_rejects_truncated_ciphertext() {
        let client = AesGcmCrypto::from_dek(&DEK).unwrap();
        let ciphertext = client.encrypt(b"payload").unwrap();
        let truncated = &ciphertext[..ciphertext.len().saturating_sub(1)];
        assert!(client.decrypt(truncated).is_err());
    }

    #[test]
    fn decrypt_rejects_bit_flip() {
        let client = AesGcmCrypto::from_dek(&DEK).unwrap();
        let mut ciphertext = client.encrypt(b"payload").unwrap();
        let last = ciphertext.len() - 1;
        ciphertext[last] ^= 0x01;
        assert!(client.decrypt(&ciphertext).is_err());
    }

    #[test]
    fn decrypt_rejects_wrong_dek() {
        let a = AesGcmCrypto::from_dek(&DEK).unwrap();
        let b = AesGcmCrypto::from_dek(&OTHER_DEK).unwrap();

        let ciphertext = a.encrypt(b"secret").unwrap();
        assert!(b.decrypt(&ciphertext).is_err());
    }

    #[test]
    fn decrypt_rejects_ciphertext_shorter_than_nonce() {
        let client = AesGcmCrypto::from_dek(&DEK).unwrap();

        let err = client.decrypt(&[0u8; NONCE_LEN]).unwrap_err();
        assert!(err.to_string().contains("ciphertext too short"));

        let err = client.decrypt(&[0u8; 4]).unwrap_err();
        assert!(err.to_string().contains("ciphertext too short"));
    }

    #[tokio::test]
    async fn bootstrap_contention_single_dek() {
        let store = Arc::new(InMemoryObjectStore::new());
        let kms_a = IdentityKms {
            dek: DEK,
            generates: AtomicUsize::new(0),
        };
        let kms_b = IdentityKms {
            dek: DEK,
            generates: AtomicUsize::new(0),
        };

        let (ca, cb) = tokio::join!(
            AesGcmCrypto::from_kms(store.clone(), &kms_a, "i-a"),
            AesGcmCrypto::from_kms(store.clone(), &kms_b, "i-b"),
        );
        let ca = ca.expect("crypto a");
        let cb = cb.expect("crypto b");

        let ct = ca.encrypt(b"shared").unwrap();
        assert_eq!(cb.decrypt(&ct).unwrap(), b"shared");

        let stored = store
            .get_object(keys::DEK_OBJECT_KEY)
            .await
            .unwrap()
            .expect("dek stored once");
        assert_eq!(stored, DEK);
    }

    #[test]
    fn failing_crypto_rejects_encrypt_and_decrypt() {
        let crypto = FailingCrypto;
        assert!(crypto.encrypt(b"x").is_err());
        assert!(crypto.decrypt(b"x").is_err());
    }
}

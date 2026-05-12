//! DEK-wrapped key-value persistence in shared storage.

use crate::crypto::CryptoClient;
use crate::storage::{StorageClient, keys};
use anyhow::Context;
use std::sync::Arc;
use thiserror::Error;

/// Maximum UTF-8 length of a logical KV key (excluding the `kv:` prefix).
const MAX_LOGICAL_KEY_LEN: usize = 512;

/// Maximum UTF-8 byte length of a stored value.
const MAX_VALUE_UTF8_LEN: usize = 384 * 1024;

/// Errors from enclave KV operations (mapped to HTTP status in the API layer).
#[derive(Debug, Error)]
pub enum KvStoreError {
    /// Invalid key or value (HTTP 400).
    #[error("{0}")]
    BadRequest(String),
    /// No object at `kv:<key>` (HTTP 404).
    #[error("key not found")]
    NotFound,
    /// Decrypt failed or plaintext is not UTF-8 on read (HTTP 422).
    #[error("{0}")]
    Unprocessable(String),
    /// Storage failure, encrypt failure on write, or unexpected error (HTTP 500).
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

/// Persists string values under logical keys; values are encrypted with the data-plane DEK before DynamoDB.
pub struct EnclaveKvStore {
    /// Shared object storage (DynamoDB).
    pub storage: Arc<StorageClient>,
    /// DEK-backed AES-GCM encrypt/decrypt.
    pub crypto: Arc<CryptoClient>,
}

impl EnclaveKvStore {
    /// Builds a store from shared clients.
    pub fn new(storage: Arc<StorageClient>, crypto: Arc<CryptoClient>) -> Self {
        Self { storage, crypto }
    }

    fn validate_logical_key(logical_key: &str) -> Result<(), KvStoreError> {
        if logical_key.is_empty() {
            return Err(KvStoreError::BadRequest("key must not be empty".into()));
        }
        if logical_key.len() > MAX_LOGICAL_KEY_LEN {
            return Err(KvStoreError::BadRequest(format!(
                "key exceeds maximum length of {MAX_LOGICAL_KEY_LEN} bytes"
            )));
        }
        if !logical_key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '@' | '.' | '/' | '-'))
        {
            return Err(KvStoreError::BadRequest(
                "key may only contain ASCII letters, digits, and _ : @ . / -".into(),
            ));
        }
        Ok(())
    }

    fn validate_value(value: &str) -> Result<(), KvStoreError> {
        if value.len() > MAX_VALUE_UTF8_LEN {
            return Err(KvStoreError::BadRequest(format!(
                "value exceeds maximum length of {MAX_VALUE_UTF8_LEN} bytes"
            )));
        }
        Ok(())
    }

    /// Encrypts `plaintext` and overwrites the object at `kv:<logical_key>`.
    pub async fn set(&self, logical_key: &str, plaintext: &str) -> Result<(), KvStoreError> {
        Self::validate_logical_key(logical_key)?;
        Self::validate_value(plaintext)?;

        let ciphertext = self
            .crypto
            .encrypt(plaintext.as_bytes())
            .map_err(KvStoreError::Internal)?;
        let pk = keys::kv_object_key(logical_key);
        self.storage
            .set_object(&pk, &ciphertext)
            .await
            .with_context(|| format!("kv set_object({pk})"))
            .map_err(KvStoreError::Internal)?;
        Ok(())
    }

    /// Reads `kv:<logical_key>`, decrypts, and returns UTF-8 plaintext.
    pub async fn get(&self, logical_key: &str) -> Result<String, KvStoreError> {
        Self::validate_logical_key(logical_key)?;
        let pk = keys::kv_object_key(logical_key);
        let blob = self
            .storage
            .get_object(&pk)
            .await
            .with_context(|| format!("kv get_object({pk})"))
            .map_err(KvStoreError::Internal)?;
        let Some(bytes) = blob else {
            return Err(KvStoreError::NotFound);
        };
        let plain = self
            .crypto
            .decrypt(&bytes)
            .map_err(|e| KvStoreError::Unprocessable(format!("decrypt error: {e:#}")))?;
        String::from_utf8(plain).map_err(|e| {
            KvStoreError::Unprocessable(format!("decrypted value is not valid UTF-8: {e}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_logical_key_accepts_allowed() {
        EnclaveKvStore::validate_logical_key("app_state_v1").unwrap();
        EnclaveKvStore::validate_logical_key("ns/wallet:1").unwrap();
    }

    #[test]
    fn validate_logical_key_rejects_empty() {
        assert!(matches!(
            EnclaveKvStore::validate_logical_key(""),
            Err(KvStoreError::BadRequest(_))
        ));
    }

    #[test]
    fn validate_logical_key_rejects_bad_chars() {
        assert!(EnclaveKvStore::validate_logical_key("a b").is_err());
        assert!(EnclaveKvStore::validate_logical_key("a\n").is_err());
    }

    #[test]
    fn kv_object_key_format() {
        assert_eq!(
            crate::storage::keys::kv_object_key("mykey"),
            "kv:mykey"
        );
    }
}

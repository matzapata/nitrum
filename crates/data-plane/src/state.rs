//! Shared state for the data-plane: config and clients.

use crate::config::RuntimeConfig;
use crate::crypto::CryptoClient;
use crate::storage::StorageClient;
use std::sync::{Arc, RwLock};

/// Shared state for the data-plane: config and clients.
#[derive(Clone)]
pub struct DataPlaneState {
    /// Runtime configuration.
    pub config: RuntimeConfig,
    /// Storage client for storing data.
    pub storage: Arc<StorageClient>,
    /// Crypto client for encrypting/decrypting data.
    pub crypto: Arc<CryptoClient>,
    /// SHA-256 hash of the current TLS leaf certificate (DER). Set by the TLS layer on load/reload.
    pub tls_cert_hash: Arc<RwLock<Option<Vec<u8>>>>,
}

impl DataPlaneState {
    pub fn new(
        config: RuntimeConfig,
        storage: Arc<StorageClient>,
        crypto: Arc<CryptoClient>,
    ) -> Self {
        Self {
            config,
            storage,
            crypto,
            tls_cert_hash: Arc::new(RwLock::new(None)),
        }
    }
}

//! Shared state for the data-plane: config and clients.

use crate::config::DataPlaneConfig;
use crate::crypto::CryptoClient;
use crate::storage::StorageClient;
use std::sync::{Arc, RwLock};

/// Shared state for the data-plane: config and clients.
#[derive(Clone)]
pub struct DataPlaneState {
    /// Resolved data-plane configuration.
    pub config: DataPlaneConfig,
    /// Storage client for storing data.
    pub storage: Arc<StorageClient>,
    /// Crypto client for encrypting/decrypting data.
    pub crypto: Arc<CryptoClient>,
    /// Reused HTTP client for ingress reverse-proxy requests to the user application.
    pub proxy_client: reqwest::Client,
    /// SHA-256 hash of the current TLS leaf certificate (DER). Set by the TLS layer on load/reload.
    pub tls_cert_hash: Arc<RwLock<Option<Vec<u8>>>>,
}

impl DataPlaneState {
    #[must_use]
    pub fn new(
        config: DataPlaneConfig,
        storage: Arc<StorageClient>,
        crypto: Arc<CryptoClient>,
    ) -> Self {
        Self {
            config,
            storage,
            crypto,
            proxy_client: reqwest::Client::new(),
            tls_cert_hash: Arc::new(RwLock::new(None)),
        }
    }
}

//! Shared state for the data-plane: config and clients.

use std::sync::Arc;
use crate::config::RuntimeConfig;
use crate::crypto::CryptoClient;
use crate::storage::StorageClient;

/// Shared state for the data-plane: config and clients.
#[derive(Clone)]
pub struct DataPlaneState {
    pub config: RuntimeConfig,
    pub storage: Arc<StorageClient>,
    pub crypto: Arc<CryptoClient>,
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
        }
    }
}

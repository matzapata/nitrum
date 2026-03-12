//! Shared state for the data-plane: config and infra clients.

use std::sync::Arc;

use crate::config::AppConfig;
use crate::storage::InfraClients;

/// Shared state for the data-plane: config and infra clients.
/// Passed to API, ingress, crypto, tls, etc. (e.g. `Arc<DataPlaneState>` or `State<DataPlaneState>` in Axum).
#[derive(Clone)]
pub struct DataPlaneState {
    pub config: AppConfig,
    pub infra: Arc<InfraClients>,
}

impl DataPlaneState {
    pub fn new(config: AppConfig, infra: Arc<InfraClients>) -> Self {
        Self { config, infra }
    }
}

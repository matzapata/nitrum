//! Shared dependencies for the ingress HTTP/HTTPS server.

use crate::DataPlaneConfig;
use crate::storage::StorageClient;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

/// Dependencies required by the ingress server and its handlers.
#[derive(Clone)]
pub struct IngressState {
    /// Resolved data-plane configuration.
    pub config: DataPlaneConfig,
    /// Storage client for ACME HTTP-01 challenge payloads.
    pub storage: Arc<StorageClient>,
    /// Reused HTTP client for reverse-proxy requests to the user application.
    pub proxy_client: reqwest::Client,
    /// SHA-256 hash of the current TLS leaf certificate (DER). Set by the TLS layer on load/reload.
    pub tls_cert_hash: Arc<RwLock<Option<Vec<u8>>>>,
    /// Whether the user application last passed `[health_check]` probes.
    ///
    /// Starts `false` when a `start_command` is configured (probes must succeed first).
    /// Stays `true` when there is no user process (platform-only mode).
    pub app_ready: Arc<AtomicBool>,
}

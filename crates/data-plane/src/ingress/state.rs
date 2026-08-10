//! Shared dependencies for the ingress HTTP/HTTPS server.

use crate::DataPlaneConfig;
use crate::storage::StorageClient;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

/// Idle keep-alive sockets retained per backend host for the reverse proxy.
///
/// Must stay well above typical concurrent proxied load; capping too low
/// (e.g. 64) forces connection churn under burst and tanks throughput.
const PROXY_POOL_MAX_IDLE_PER_HOST: usize = 256;

/// Dependencies required by the ingress server and its handlers.
#[derive(Clone)]
pub struct IngressState {
    /// Resolved data-plane configuration.
    pub config: DataPlaneConfig,
    /// Storage client for ACME HTTP-01 challenge payloads.
    pub storage: Arc<StorageClient>,
    /// Reused HTTP client for reverse-proxy requests to the user application.
    pub proxy_client: reqwest::Client,
    /// Precomputed `http://127.0.0.1:{port}` for reverse-proxy URL assembly.
    pub proxy_base_url: String,
    /// SHA-256 hash of the current TLS leaf certificate (DER). Set by the TLS layer on load/reload.
    pub tls_cert_hash: Arc<RwLock<Option<Vec<u8>>>>,
    /// Whether the user application last passed `[health_check]` probes.
    ///
    /// Starts `false` when a `start_command` is configured (probes must succeed first).
    /// Stays `true` when there is no user process (platform-only mode).
    pub app_ready: Arc<AtomicBool>,
}

impl IngressState {
    /// HTTP client for loopback reverse-proxy to the user app.
    ///
    /// Enables `TCP_NODELAY` and an explicit per-host idle pool so concurrent
    /// proxied requests reuse keep-alive connections.
    #[must_use]
    pub fn build_proxy_client() -> reqwest::Client {
        reqwest::Client::builder()
            .tcp_nodelay(true)
            .pool_max_idle_per_host(PROXY_POOL_MAX_IDLE_PER_HOST)
            .build()
            .expect("ingress proxy reqwest client")
    }

    /// Build ingress state with a tuned proxy client and empty TLS cert hash.
    ///
    /// Precomputes the loopback proxy base URL from `[project].port`.
    #[must_use]
    pub fn new(
        config: DataPlaneConfig,
        storage: Arc<StorageClient>,
        app_ready: Arc<AtomicBool>,
    ) -> Self {
        let port = config.nitrum.project.port.get();
        let proxy_base_url = format!("http://127.0.0.1:{port}");
        Self {
            config,
            storage,
            proxy_client: Self::build_proxy_client(),
            proxy_base_url,
            tls_cert_hash: Arc::new(RwLock::new(None)),
            app_ready,
        }
    }

    /// Assemble a full backend URL from the precomputed base and a path (and optional query).
    #[must_use]
    pub fn proxy_url(&self, path_and_query: &str) -> String {
        let mut url = String::with_capacity(self.proxy_base_url.len() + path_and_query.len());
        url.push_str(&self.proxy_base_url);
        url.push_str(path_and_query);
        url
    }
}

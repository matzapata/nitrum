//! Shared dependencies for the ingress HTTP/HTTPS server.

use crate::DataPlaneConfig;
use crate::storage::StorageClient;
use axum::body::Body;
use axum::http::uri::{Authority, PathAndQuery, Scheme, Uri};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

/// Idle keep-alive sockets retained per backend host for the reverse proxy.
///
/// Must stay well above typical concurrent proxied load; capping too low
/// (e.g. 64) forces connection churn under burst and tanks throughput.
const PROXY_POOL_MAX_IDLE_PER_HOST: usize = 256;

/// HTTP client used for loopback reverse-proxy to the user application.
pub type ProxyClient = Client<HttpConnector, Body>;

/// Dependencies required by the ingress server and its handlers.
#[derive(Clone)]
pub struct IngressState {
    /// Resolved data-plane configuration.
    pub config: DataPlaneConfig,
    /// Storage client for ACME HTTP-01 challenge payloads.
    pub storage: Arc<StorageClient>,
    /// Reused HTTP client for reverse-proxy requests to the user application.
    pub proxy_client: ProxyClient,
    /// Precomputed `127.0.0.1:{port}` authority for reverse-proxy URI assembly.
    pub proxy_authority: Authority,
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
    /// proxied requests reuse keep-alive connections. Uses raw `hyper_util`
    /// (no reqwest redirect/proxy/URL-parse layers) for the fixed loopback hop.
    #[must_use]
    pub fn build_proxy_client() -> ProxyClient {
        let mut connector = HttpConnector::new();
        connector.set_nodelay(true);
        Client::builder(TokioExecutor::new())
            .pool_max_idle_per_host(PROXY_POOL_MAX_IDLE_PER_HOST)
            .build(connector)
    }

    /// Build ingress state with a tuned proxy client and empty TLS cert hash.
    ///
    /// Precomputes the loopback proxy authority from `[project].port`.
    #[must_use]
    pub fn new(
        config: DataPlaneConfig,
        storage: Arc<StorageClient>,
        app_ready: Arc<AtomicBool>,
    ) -> Self {
        let port = config.nitrum.project.port.get();
        let proxy_authority: Authority = format!("127.0.0.1:{port}")
            .parse()
            .expect("loopback proxy authority");
        Self {
            config,
            storage,
            proxy_client: Self::build_proxy_client(),
            proxy_authority,
            tls_cert_hash: Arc::new(RwLock::new(None)),
            app_ready,
        }
    }

    /// Assemble a backend URI from the precomputed authority and path (and optional query).
    ///
    /// Clones `Authority` / `PathAndQuery` (cheap `Bytes` refcount bumps) — no string
    /// formatting or URL re-parsing per request.
    #[must_use]
    pub fn proxy_uri(&self, path_and_query: PathAndQuery) -> Uri {
        Uri::builder()
            .scheme(Scheme::HTTP)
            .authority(self.proxy_authority.clone())
            .path_and_query(path_and_query)
            .build()
            .expect("loopback proxy URI")
    }
}

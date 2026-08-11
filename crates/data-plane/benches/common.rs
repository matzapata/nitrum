//! Shared helpers for data-plane benchmarks.
//!
//! Builds inert [`DataPlaneConfig`] values without live AWS or IMDS, plus a
//! loopback mock backend and ingress router factory used by the latency benches.

use aws_config::BehaviorVersion;
use aws_config::Region;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use config::{NitrumConfig, PlatformLayout, Project};
use data_plane::StorageClient;
use data_plane::ingress::{IngressState, build_https_router};
use data_plane::{DataPlaneConfig, ListenAddrs};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::net::TcpListener;

/// Build a [`DataPlaneConfig`] with inert infra placeholders for offline benchmarks.
#[must_use]
pub fn data_plane_config(nitrum: NitrumConfig) -> DataPlaneConfig {
    DataPlaneConfig {
        aws: Arc::new(
            aws_config::SdkConfig::builder()
                .behavior_version(BehaviorVersion::latest())
                .region(Region::new("us-east-1"))
                .build(),
        ),
        nitrum: nitrum.clone(),
        layout: PlatformLayout::from_project(&nitrum.project),
        imds_base_url: "http://127.0.0.1/latest".to_string(),
        instance_id: "i-bench".to_string(),
        dynamodb_table: "bench-table".to_string(),
        kms_key_id: "bench-kms-key".to_string(),
        listen_addrs: ListenAddrs {
            ingress_listen_addr: "127.0.0.1:443".parse().expect("valid ingress listen addr"),
            acme_http01_listen_addr: "127.0.0.1:80".parse().expect("valid acme listen addr"),
            crypto_api_listen_addr: "127.0.0.1:3000"
                .parse()
                .expect("valid crypto api listen addr"),
        },
        user_env: HashMap::new(),
        otlp_endpoint: None,
    }
}

/// Minimal [`NitrumConfig`] for offline benches (defaults elsewhere; port overridden per run).
#[must_use]
pub fn sample_nitrum_config() -> NitrumConfig {
    NitrumConfig {
        project: Project {
            name: "nitrum-bench".parse().expect("valid bench project name"),
            port: std::num::NonZeroU16::new(8080).expect("8080 is non-zero"),
            start_command: vec![],
        },
        runtime: config::Runtime::default(),
        health_check: config::HealthCheck::default(),
        scaling: config::Scaling::default(),
        tls_termination: config::TlsTermination::default(),
        egress: config::Egress::default(),
        cloud: config::Cloud::default(),
    }
}

/// Set the backend port on an existing [`DataPlaneConfig`].
#[must_use]
pub const fn with_backend_port(mut config: DataPlaneConfig, port: u16) -> DataPlaneConfig {
    config.nitrum.project.port =
        std::num::NonZeroU16::new(port).expect("benchmark backend port must be non-zero");
    config
}

/// Bind a loopback Axum app that answers every path with `200 ok`.
///
/// Drains the request body so keep-alive reuse stays valid for large POSTs.
/// Returns the ephemeral port; the server task lives as long as the current runtime.
pub async fn spawn_mock_backend() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock backend");
    let addr = listener.local_addr().expect("local addr");

    let app = Router::new().fallback(|req: Request<Body>| async move {
        // Discard without failing keep-alive when the proxy forwards large bodies.
        let _ = axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024).await;
        (StatusCode::OK, "ok")
    });

    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("mock backend serve");
    });

    addr.port()
}

/// Build the HTTPS ingress router pointed at `backend_port`.
pub fn ingress_router(backend_port: u16) -> Router {
    let data_plane_cfg = with_backend_port(data_plane_config(sample_nitrum_config()), backend_port);
    let storage = Arc::new(StorageClient::from_config(&data_plane_cfg));
    let state = Arc::new(IngressState::new(
        data_plane_cfg,
        storage,
        Arc::new(AtomicBool::new(true)),
    ));
    build_https_router(state)
}

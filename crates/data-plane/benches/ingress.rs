//! Ingress proxy routing via `tower::ServiceExt::oneshot` against a mock loopback backend.
//!
//! Measures the full proxy path, including request body buffering, backend
//! round-trip, and response body drain — not router dispatch alone.

use aws_config::BehaviorVersion;
use aws_config::Region;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use config::{NitrumConfig, PlatformLayout, Project};
use criterion::{
    BatchSize, BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main,
};
use data_plane::InMemoryObjectStore;
use data_plane::ingress::{IngressState, build_https_router};
use data_plane::{DataPlaneConfig, ListenAddrs};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::net::TcpListener;
use tower::ServiceExt;

const BODY_LIMIT: usize = 1024 * 1024;

struct ProxyCase {
    name: &'static str,
    method: Method,
    path: &'static str,
    body: Vec<u8>,
}

/// Build a [`DataPlaneConfig`] with inert infra placeholders for offline benchmarks.
fn data_plane_config(nitrum: NitrumConfig) -> DataPlaneConfig {
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
fn sample_nitrum_config() -> NitrumConfig {
    NitrumConfig {
        project: Project {
            name: "nitrum-bench".parse().expect("valid bench project name"),
            port: std::num::NonZeroU16::new(8080).expect("8080 is non-zero"),
            start_command: vec![],
            dockerfile: None,
        },
        runtime: config::Runtime::default(),
        health_check: config::HealthCheck::default(),
        scaling: config::Scaling::default(),
        tls_termination: config::TlsTermination::default(),
        egress: config::Egress::default(),
        cloud: config::Cloud::default(),
        local: config::Local::default(),
    }
}

/// Set the backend port on an existing [`DataPlaneConfig`].
const fn with_backend_port(mut config: DataPlaneConfig, port: u16) -> DataPlaneConfig {
    config.nitrum.project.port =
        std::num::NonZeroU16::new(port).expect("benchmark backend port must be non-zero");
    config
}

/// Bind a loopback Axum app that answers every path with `200 ok`.
///
/// Drains the request body so keep-alive reuse stays valid for large POSTs.
/// Returns the ephemeral port; the server task lives as long as the current runtime.
async fn spawn_mock_backend() -> u16 {
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
fn ingress_router(backend_port: u16) -> Router {
    let data_plane_cfg = with_backend_port(data_plane_config(sample_nitrum_config()), backend_port);
    let storage = Arc::new(InMemoryObjectStore::new());
    let state = Arc::new(IngressState::new(
        data_plane_cfg,
        storage,
        Arc::new(AtomicBool::new(true)),
    ));
    build_https_router(state)
}

fn ingress_benches(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let backend_port = rt.block_on(spawn_mock_backend());
    let app = ingress_router(backend_port);

    let cases = [
        ProxyCase {
            name: "get_empty",
            method: Method::GET,
            path: "/health",
            body: vec![],
        },
        ProxyCase {
            name: "post_1kiB",
            method: Method::POST,
            path: "/",
            body: vec![0xCDu8; 1024],
        },
        ProxyCase {
            name: "post_64kiB",
            method: Method::POST,
            path: "/",
            body: vec![0xCDu8; 64 * 1024],
        },
    ];

    {
        let mut group = c.benchmark_group("ingress");
        for case in &cases {
            if case.body.is_empty() {
                group.throughput(Throughput::Elements(1));
            } else {
                group.throughput(Throughput::Bytes(case.body.len() as u64));
            }

            let app = app.clone();
            group.bench_with_input(BenchmarkId::new("proxy", case.name), case, |b, case| {
                b.to_async(&rt).iter_batched(
                    || {
                        let mut builder = Request::builder().method(&case.method).uri(case.path);
                        if !case.body.is_empty() {
                            builder = builder.header("content-type", "application/octet-stream");
                        }
                        let req = builder
                            .body(Body::from(case.body.clone()))
                            .expect("build request");
                        (app.clone(), req)
                    },
                    |(app, req)| async move {
                        let resp = app.oneshot(req).await.unwrap();
                        assert_eq!(resp.status(), StatusCode::OK);

                        let body = to_bytes(resp.into_body(), BODY_LIMIT)
                            .await
                            .expect("response body");
                        black_box(body);
                    },
                    BatchSize::SmallInput,
                );
            });
        }

        group.finish();
    }
}

criterion_group!(benches, ingress_benches);
criterion_main!(benches);

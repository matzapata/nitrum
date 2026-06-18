//! Ingress proxy routing via `tower::ServiceExt::oneshot` against a mock loopback backend.
//!
//! Measures the full proxy path, including per-request `reqwest::Client` construction,
//! request body buffering, and backend round-trip — not router dispatch alone.

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use config::{NitrumConfig, WellKnown};
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, black_box};
use data_plane::server::ingress::build_https_router;
use data_plane::{CryptoClient, DataPlaneState, StorageClient, runtime_config, with_backend_port};
use std::sync::Arc;
use tokio::net::TcpListener;
use tower::ServiceExt;

const DEK: [u8; 32] = [0x42; 32];
const NITRUM_TOML: &str = include_str!("../../../samples/hello/nitrum.toml");

struct BenchEnv {
    app: Router,
}

impl BenchEnv {
    fn new(backend_port: u16) -> Self {
        let mut nitrum: NitrumConfig =
            toml::from_str(NITRUM_TOML).expect("parse sample nitrum.toml");
        nitrum.well_known = WellKnown {
            enclave_status: false,
            enclave_attestation: false,
        };

        let runtime_cfg = with_backend_port(runtime_config(nitrum), backend_port);
        let storage = Arc::new(StorageClient::new(&runtime_cfg));
        let crypto = Arc::new(CryptoClient::from_dek(&DEK).expect("valid DEK"));
        let state = Arc::new(DataPlaneState::new(runtime_cfg, storage, crypto));
        let app = build_https_router(state);

        Self { app }
    }
}

struct ProxyCase {
    name: &'static str,
    method: Method,
    path: &'static str,
    body: Vec<u8>,
}

fn build_request(case: &ProxyCase) -> Request<Body> {
    let mut builder = Request::builder().method(&case.method).uri(case.path);
    if !case.body.is_empty() {
        builder = builder.header("content-type", "application/octet-stream");
    }
    builder
        .body(Body::from(case.body.clone()))
        .expect("build request")
}

fn spawn_mock_backend(rt: &tokio::runtime::Runtime) -> u16 {
    rt.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock backend");
        let addr = listener.local_addr().expect("local addr");

        let app = Router::new().fallback(|_: Request<Body>| async { (StatusCode::OK, "ok") });

        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("mock backend serve");
        });

        addr.port()
    })
}

fn ingress_benches(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let backend_port = spawn_mock_backend(&rt);
    let env = BenchEnv::new(backend_port);

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

    let mut group = c.benchmark_group("ingress");
    for case in &cases {
        if !case.body.is_empty() {
            group.throughput(Throughput::Bytes(case.body.len() as u64));
        }
        let app = env.app.clone();
        group.bench_with_input(BenchmarkId::new("proxy", case.name), case, |b, case| {
            b.to_async(&rt).iter_batched(
                || build_request(case),
                |req| {
                    let app = app.clone();
                    async move {
                        let resp = app.oneshot(req).await.expect("oneshot");
                        black_box(resp.status())
                    }
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn main() {
    let mut c = criterion::Criterion::default().configure_from_args();
    ingress_benches(&mut c);
    c.final_summary();
}

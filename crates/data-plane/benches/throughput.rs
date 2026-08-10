//! Concurrent throughput sweeps for ingress (status vs proxy) and crypto API.
//!
//! Unlike the single-request latency benches, each iteration fans out N concurrent
//! `oneshot` calls so Criterion throughput (elems/sec) tracks ops/sec under load —
//! the local analog of the k6 VU sweep.
//!
//! Groups:
//! - `throughput_ingress_status` / `throughput_ingress_proxy` — hop-tax comparison
//!   (in-process status handler vs reverse-proxy to a mock loopback backend)
//! - `*_otel` — same paths with `telemetry::http::instrument_router` applied
//!   (production wraps ingress this way)
//! - `throughput_ingress_proxy_tuned` — proxy with `tcp_nodelay` + larger idle pool
//! - `throughput_crypto_roundtrip` — encrypt+decrypt via the crypto API router

mod common;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use common::{data_plane_config, with_backend_port};
use config::NitrumConfig;
use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, black_box};
use data_plane::StorageClient;
use data_plane::crypto::{CryptoClient, api_state, build_router as build_crypto_router};
use data_plane::ingress::{IngressState, build_https_router};
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::net::TcpListener;
use tower::ServiceExt;

const NITRUM_TOML: &str = include_str!("../../../examples/hello/nitrum.toml");
const DEK: [u8; 32] = [0x42; 32];
const CONCURRENCY: &[usize] = &[1, 10, 50, 100, 200];
const CRYPTO_PLAINTEXT: &str = "nitrum-macro-load-test";

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

fn tuned_proxy_client() -> reqwest::Client {
    reqwest::Client::builder()
        .tcp_nodelay(true)
        .pool_max_idle_per_host(64)
        .build()
        .expect("build tuned proxy client")
}

fn ingress_router(
    backend_port: u16,
    with_otel: bool,
    proxy_client: Option<reqwest::Client>,
) -> Router {
    let nitrum: NitrumConfig = toml::from_str(NITRUM_TOML).expect("parse sample nitrum.toml");
    let data_plane_cfg = with_backend_port(data_plane_config(nitrum), backend_port);
    let storage = Arc::new(StorageClient::from_config(&data_plane_cfg));
    let mut state = IngressState::new(data_plane_cfg, storage, Arc::new(AtomicBool::new(true)));
    if let Some(proxy_client) = proxy_client {
        state.proxy_client = proxy_client;
    }
    let router = build_https_router(Arc::new(state));
    if with_otel {
        telemetry::http::instrument_router(router, "data-plane.ingress")
    } else {
        router
    }
}

fn crypto_api_router() -> Router {
    let nitrum: NitrumConfig = toml::from_str(NITRUM_TOML).expect("parse sample nitrum.toml");
    let data_plane_cfg = data_plane_config(nitrum);
    let storage = Arc::new(StorageClient::from_config(&data_plane_cfg));
    let crypto = Arc::new(CryptoClient::from_dek(&DEK).expect("valid DEK"));
    build_crypto_router(api_state(crypto, storage))
}

fn health_proxy_request() -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri("/health")
        .body(Body::empty())
        .expect("build health request")
}

fn status_request() -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri("/.well-known/enclave/status")
        .body(Body::empty())
        .expect("build status request")
}

fn encrypt_request() -> Request<Body> {
    let body = serde_json::json!({ "plaintext": CRYPTO_PLAINTEXT }).to_string();
    Request::builder()
        .method(Method::POST)
        .uri("/encrypt")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .expect("build encrypt request")
}

fn decrypt_request(ciphertext_b64: &str) -> Request<Body> {
    let body = serde_json::json!({ "ciphertext": ciphertext_b64 }).to_string();
    Request::builder()
        .method(Method::POST)
        .uri("/decrypt")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .expect("build decrypt request")
}

async fn crypto_encrypt_decrypt_once(app: Router) -> StatusCode {
    let encrypt_resp = app
        .clone()
        .oneshot(encrypt_request())
        .await
        .expect("encrypt oneshot");
    assert_eq!(encrypt_resp.status(), StatusCode::OK);
    let encrypt_body = to_bytes(encrypt_resp.into_body(), 1024 * 1024)
        .await
        .expect("encrypt body");
    let encrypt_json: Value = serde_json::from_slice(&encrypt_body).expect("encrypt json");
    let ciphertext = encrypt_json["data"].as_str().expect("encrypt data field");
    let _ = B64.decode(ciphertext).expect("encrypt data is base64");

    let decrypt_resp = app
        .oneshot(decrypt_request(ciphertext))
        .await
        .expect("decrypt oneshot");
    black_box(decrypt_resp.status())
}

fn bench_concurrent_oneshot(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    rt: &tokio::runtime::Runtime,
    app: &Router,
    build_req: fn() -> Request<Body>,
) {
    for &concurrency in CONCURRENCY {
        group.throughput(Throughput::Elements(concurrency as u64));
        let app = app.clone();
        group.bench_with_input(
            BenchmarkId::from_parameter(concurrency),
            &concurrency,
            |b, &n| {
                b.to_async(rt).iter_batched(
                    || {
                        (0..n)
                            .map(|_| (app.clone(), build_req()))
                            .collect::<Vec<_>>()
                    },
                    |batch| async move {
                        let futs = batch.into_iter().map(|(app, req)| async move {
                            let resp = app.oneshot(req).await.expect("oneshot");
                            black_box(resp.status())
                        });
                        futures::future::join_all(futs).await
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }
}

fn throughput_benches(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let backend_port = spawn_mock_backend(&rt);

    let ingress_plain = ingress_router(backend_port, false, None);
    let ingress_otel = ingress_router(backend_port, true, None);
    let ingress_tuned = ingress_router(backend_port, false, Some(tuned_proxy_client()));
    let crypto_app = crypto_api_router();

    {
        let mut group = c.benchmark_group("throughput_ingress_status");
        bench_concurrent_oneshot(&mut group, &rt, &ingress_plain, status_request);
        group.finish();
    }

    {
        let mut group = c.benchmark_group("throughput_ingress_proxy");
        bench_concurrent_oneshot(&mut group, &rt, &ingress_plain, health_proxy_request);
        group.finish();
    }

    {
        let mut group = c.benchmark_group("throughput_ingress_status_otel");
        bench_concurrent_oneshot(&mut group, &rt, &ingress_otel, status_request);
        group.finish();
    }

    {
        let mut group = c.benchmark_group("throughput_ingress_proxy_otel");
        bench_concurrent_oneshot(&mut group, &rt, &ingress_otel, health_proxy_request);
        group.finish();
    }

    {
        let mut group = c.benchmark_group("throughput_ingress_proxy_tuned");
        bench_concurrent_oneshot(&mut group, &rt, &ingress_tuned, health_proxy_request);
        group.finish();
    }

    {
        let mut group = c.benchmark_group("throughput_crypto_roundtrip");
        for &concurrency in CONCURRENCY {
            group.throughput(Throughput::Elements(concurrency as u64));
            let app = crypto_app.clone();
            group.bench_with_input(
                BenchmarkId::from_parameter(concurrency),
                &concurrency,
                |b, &n| {
                    b.to_async(&rt).iter_batched(
                        || (0..n).map(|_| app.clone()).collect::<Vec<_>>(),
                        |batch| async move {
                            let futs = batch
                                .into_iter()
                                .map(|app| async move { crypto_encrypt_decrypt_once(app).await });
                            futures::future::join_all(futs).await
                        },
                        BatchSize::SmallInput,
                    );
                },
            );
        }
        group.finish();
    }
}

fn main() {
    let mut c = criterion::Criterion::default().configure_from_args();
    throughput_benches(&mut c);
    c.final_summary();
}

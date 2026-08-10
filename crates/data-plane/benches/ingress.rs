//! Ingress proxy routing via `tower::ServiceExt::oneshot` against a mock loopback backend.
//!
//! Measures the full proxy path, including request body buffering, backend
//! round-trip, and response body drain — not router dispatch alone.

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use common::{ingress_router, spawn_mock_backend};
use criterion::{
    BatchSize, BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main,
};
use tower::ServiceExt;

const BODY_LIMIT: usize = 1024 * 1024;

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

fn ingress_benches(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let backend_port = spawn_mock_backend(&rt);
    let app = ingress_router(backend_port, false);

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
                    || (app.clone(), build_request(case)),
                    |(app, req)| async move {
                        let resp = app.oneshot(req).await.expect("oneshot");
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

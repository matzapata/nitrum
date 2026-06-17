//! Phase 2 will benchmark ingress routing via `tower::ServiceExt::oneshot` against an extracted
//! router and a mock loopback backend (requires `build_ingress_router` refactor + `tower` dev-dep).

use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn ingress_placeholder(c: &mut Criterion) {
    c.bench_function("ingress_placeholder", |b| {
        b.iter(|| black_box(0u8));
    });
}

criterion_group!(benches, ingress_placeholder);
criterion_main!(benches);

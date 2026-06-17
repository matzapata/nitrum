//! Phase 2 will benchmark `build_server_config_from_pem` and full TLS handshakes via loopback.
//! The rustls crypto provider must be installed before any rustls API use.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::sync::Once;

static INIT_RUSTLS: Once = Once::new();

fn ensure_rustls_provider() {
    INIT_RUSTLS.call_once(|| {
        rustls::crypto::ring::default_provider()
            .install_default()
            .expect("failed to install default rustls crypto provider");
    });
}

fn tls_handshake_placeholder(c: &mut Criterion) {
    ensure_rustls_provider();

    c.bench_function("tls_handshake_placeholder", |b| {
        b.iter(|| black_box(0u8));
    });
}

criterion_group!(benches, tls_handshake_placeholder);
criterion_main!(benches);

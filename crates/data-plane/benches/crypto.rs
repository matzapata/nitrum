//! Phase 2 will benchmark `CryptoClient::encrypt` / `decrypt` across payload sizes
//! (64B, 1KiB, 64KiB, 384KiB) using a `from_dek` test constructor to avoid KMS/storage bootstrap.

use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn crypto_placeholder(c: &mut Criterion) {
    c.bench_function("crypto_placeholder", |b| {
        b.iter(|| black_box(0u8));
    });
}

criterion_group!(benches, crypto_placeholder);
criterion_main!(benches);

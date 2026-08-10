//! AES-GCM encrypt/decrypt throughput across payload sizes.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use data_plane::CryptoClient;

const DEK: [u8; 32] = [0x42; 32];

const SIZES: &[(u64, &str)] = &[
    (64, "64B"),
    (1024, "1KiB"),
    (64 * 1024, "64KiB"),
    (384 * 1024, "384KiB"),
];

fn encrypt_benches(c: &mut Criterion) {
    let client = CryptoClient::from_dek(&DEK).expect("valid DEK");
    let mut group = c.benchmark_group("encrypt");
    for &(size, label) in SIZES {
        let payload = vec![0xABu8; size as usize];
        group.throughput(Throughput::Bytes(size));
        group.bench_with_input(BenchmarkId::new("encrypt", label), &payload, |b, data| {
            b.iter(|| client.encrypt(black_box(data)).expect("encrypt"));
        });
    }
    group.finish();
}

fn decrypt_benches(c: &mut Criterion) {
    let client = CryptoClient::from_dek(&DEK).expect("valid DEK");
    let mut group = c.benchmark_group("decrypt");
    for &(size, label) in SIZES {
        let payload = vec![0xABu8; size as usize];
        let ciphertext = client.encrypt(&payload).expect("encrypt for setup");
        group.throughput(Throughput::Bytes(size));
        group.bench_with_input(
            BenchmarkId::new("decrypt", label),
            &ciphertext,
            |b, data| b.iter(|| client.decrypt(black_box(data)).expect("decrypt")),
        );
    }
    group.finish();
}

criterion_group!(benches, encrypt_benches, decrypt_benches);
criterion_main!(benches);

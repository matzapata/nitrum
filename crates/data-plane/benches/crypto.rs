//! AES-GCM encrypt/decrypt throughput across payload sizes.

use criterion::{BenchmarkId, Criterion, Throughput, black_box};
use data_plane::CryptoClient;

const DEK: [u8; 32] = [0x42; 32];

const SIZES: &[(u64, &str)] = &[
    (64, "64B"),
    (1024, "1KiB"),
    (64 * 1024, "64KiB"),
    (384 * 1024, "384KiB"),
];

fn crypto_benches(c: &mut Criterion) {
    let client = CryptoClient::from_dek(&DEK).expect("valid DEK");

    let mut encrypt_group = c.benchmark_group("encrypt");
    for &(size, label) in SIZES {
        let payload = vec![0xABu8; size as usize];
        encrypt_group.throughput(Throughput::Bytes(size));
        encrypt_group.bench_with_input(BenchmarkId::new("encrypt", label), &payload, |b, data| {
            b.iter(|| client.encrypt(black_box(data)).expect("encrypt"))
        });
    }
    encrypt_group.finish();

    let mut decrypt_group = c.benchmark_group("decrypt");
    for &(size, label) in SIZES {
        let payload = vec![0xABu8; size as usize];
        let ciphertext = client.encrypt(&payload).expect("encrypt for setup");
        decrypt_group.throughput(Throughput::Bytes(size));
        decrypt_group.bench_with_input(
            BenchmarkId::new("decrypt", label),
            &ciphertext,
            |b, data| b.iter(|| client.decrypt(black_box(data)).expect("decrypt")),
        );
    }
    decrypt_group.finish();
}

fn main() {
    let mut c = criterion::Criterion::default().configure_from_args();
    crypto_benches(&mut c);
    c.final_summary();
}

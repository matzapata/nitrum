//! TLS `ServerConfig` build from PEM and full loopback handshakes.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use data_plane::ingress::tls::build_server_config_from_pem;
use std::sync::Arc;
use std::sync::Once;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::{TlsAcceptor, TlsConnector};

static INIT_RUSTLS: Once = Once::new();

fn ensure_rustls_provider() {
    INIT_RUSTLS.call_once(|| {
        rustls::crypto::ring::default_provider()
            .install_default()
            .expect("failed to install default rustls crypto provider");
    });
}

struct PemFixture {
    chain_pem: String,
    key_pem: String,
}

fn generate_pem_fixture() -> PemFixture {
    let cert = rcgen::generate_simple_self_signed(vec!["nitrum.local".to_string()])
        .expect("generate self-signed cert");
    PemFixture {
        chain_pem: cert.cert.pem(),
        key_pem: cert.signing_key.serialize_pem(),
    }
}

fn pem_config_build(c: &mut Criterion, fixture: &PemFixture) {
    c.bench_function("build_server_config_from_pem", |b| {
        b.iter(|| {
            black_box(
                build_server_config_from_pem(&fixture.chain_pem, &fixture.key_pem)
                    .expect("build server config"),
            )
        });
    });
}

/// Binds a fresh listener each iteration: cold TCP connect + TLS handshake + one-byte read.
fn tls_handshake(c: &mut Criterion, fixture: &PemFixture) {
    let server_config =
        build_server_config_from_pem(&fixture.chain_pem, &fixture.key_pem).expect("server config");

    let mut roots = RootCertStore::empty();
    let certs: Vec<_> =
        rustls_pemfile::certs(&mut std::io::BufReader::new(fixture.chain_pem.as_bytes()))
            .collect::<Result<Vec<_>, _>>()
            .expect("parse certs");
    for cert in certs {
        roots.add(cert).expect("add cert to roots");
    }

    let client_config = Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    );

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    c.bench_function("tls_handshake_loopback", |b| {
        b.to_async(&rt).iter(|| {
            let server_config = server_config.clone();
            let client_config = client_config.clone();

            async move {
                let listener = TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("bind listener");
                let addr = listener.local_addr().expect("local addr");

                let server = tokio::spawn(async move {
                    let (tcp, _) = listener.accept().await.expect("accept");
                    let acceptor = TlsAcceptor::from(server_config);
                    let mut tls = acceptor.accept(tcp).await.expect("server tls accept");
                    let _ = tls.write_all(b"ok").await;
                });

                let tcp = tokio::net::TcpStream::connect(addr)
                    .await
                    .expect("client connect");
                let connector = TlsConnector::from(client_config);
                let server_name = ServerName::try_from("nitrum.local").expect("server name");
                let mut tls = connector
                    .connect(server_name, tcp)
                    .await
                    .expect("client handshake");

                let mut buf = [0u8; 2];
                let n = tls.read(&mut buf).await.expect("read response");
                black_box(n);

                server.await.expect("server task");
            }
        });
    });
}

fn build_server_config_from_pem_benches(c: &mut Criterion) {
    ensure_rustls_provider();
    let fixture = generate_pem_fixture();
    pem_config_build(c, &fixture);
}

fn tls_handshake_loopback_benches(c: &mut Criterion) {
    ensure_rustls_provider();
    let fixture = generate_pem_fixture();
    tls_handshake(c, &fixture);
}

criterion_group!(
    benches,
    build_server_config_from_pem_benches,
    tls_handshake_loopback_benches,
);
criterion_main!(benches);

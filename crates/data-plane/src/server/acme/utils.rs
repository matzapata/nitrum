//! Utils for ACME (Let's Encrypt / Pebble).

use anyhow::{Context, Result};
use bytes::Bytes;
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::client::legacy::{Client, connect::HttpConnector};
use hyper_util::rt::TokioExecutor;
use instant_acme::BodyWrapper;
use rustls::RootCertStore;
use std::io::{BufReader, Cursor};
use std::sync::Arc;

#[cfg(feature = "pebble")]
pub(crate) fn pebble_client_tls_config() -> Result<Arc<rustls::ClientConfig>> {
    let value = std::env::var("PEBBLE_MINICA_CERT")
        .context("PEBBLE_MINICA_CERT not set (required when using pebble feature)")?;
    let certs = rustls_pemfile::certs(&mut BufReader::new(Cursor::new(value.as_bytes())))
        .collect::<Result<Vec<_>, _>>()
        .context("parse pebble CA from PEBBLE_MINICA_CERT")?;
    let mut roots = RootCertStore::empty();
    roots.add_parsable_certificates(certs);
    Ok(Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ))
}

/// Hyper HTTPS client for ACME directory (e.g. Pebble with custom CA).
pub(crate) fn acme_https_client(
    tls_config: Arc<rustls::ClientConfig>,
) -> Result<Box<Client<hyper_rustls::HttpsConnector<HttpConnector>, BodyWrapper<Bytes>>>> {
    let https = HttpsConnectorBuilder::new()
        .with_tls_config((*tls_config).clone())
        .https_or_http()
        .enable_http1()
        .build();
    Ok(Box::new(Client::builder(TokioExecutor::new()).build(https)))
}

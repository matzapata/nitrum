//! PEM certificate chain + private key in shared storage (DEK-wrapped ciphertext only).

use crate::crypto::Crypto;
use crate::storage::ObjectStore;
use crate::storage::keys;
use anyhow::{Context, Result};
use std::sync::Arc;

pub struct AcmeStorage<S: ObjectStore, C: Crypto> {
    /// Backing object store.
    client: Arc<S>,
    /// Data-plane DEK; used to encrypt PEM before `set_object` and decrypt after `get_object`.
    crypto: Arc<C>,
}

impl<S: ObjectStore, C: Crypto> AcmeStorage<S, C> {
    pub(crate) const fn new(client: Arc<S>, crypto: Arc<C>) -> Self {
        Self { client, crypto }
    }

    pub(crate) async fn read_cert_pair(&self) -> Result<Option<(String, String)>> {
        let cert = self
            .client
            .get_object(keys::CERTIFICATE_OBJECT_KEY)
            .await
            .context("read cert")?;
        let key = self
            .client
            .get_object(keys::CERTIFICATE_PRIVATE_KEY_OBJECT_KEY)
            .await
            .context("read key")?;
        match (cert, key) {
            (Some(c), Some(k)) => {
                let cert_plain = self
                    .crypto
                    .decrypt(&c)
                    .context("decrypt cert from storage")?;
                let key_plain = self
                    .crypto
                    .decrypt(&k)
                    .context("decrypt key from storage")?;
                Ok(Some((
                    String::from_utf8(cert_plain).context("cert not UTF-8")?,
                    String::from_utf8(key_plain).context("key not UTF-8")?,
                )))
            }
            _ => Ok(None),
        }
    }

    pub(crate) async fn write_cert_pair(&self, chain: &str, key: &str) -> Result<()> {
        let cert_enc = self
            .crypto
            .encrypt(chain.as_bytes())
            .context("encrypt cert for storage")?;
        let key_enc = self
            .crypto
            .encrypt(key.as_bytes())
            .context("encrypt key for storage")?;
        self.client
            .set_object(keys::CERTIFICATE_OBJECT_KEY, &cert_enc)
            .await
            .context("write cert to storage")?;
        self.client
            .set_object(keys::CERTIFICATE_PRIVATE_KEY_OBJECT_KEY, &key_enc)
            .await
            .context("write key to storage")?;
        Ok(())
    }
}

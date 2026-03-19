//! PEM certificate chain + private key in shared storage.

use crate::storage::StorageClient;
use crate::storage::keys;
use anyhow::{Context, Result};
use std::sync::Arc;

pub(crate) struct AcmeStorage {
    client: Arc<StorageClient>,
}

impl AcmeStorage {
    pub(crate) fn new(client: Arc<StorageClient>) -> Self {
        Self { client }
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
            (Some(c), Some(k)) => Ok(Some((
                String::from_utf8(c).context("cert not UTF-8")?,
                String::from_utf8(k).context("key not UTF-8")?,
            ))),
            _ => Ok(None),
        }
    }

    pub(crate) async fn write_cert_pair(&self, chain: &str, key: &str) -> Result<()> {
        self.client
            .set_object(keys::CERTIFICATE_OBJECT_KEY, chain.as_bytes())
            .await
            .context("write cert to storage")?;
        self.client
            .set_object(keys::CERTIFICATE_PRIVATE_KEY_OBJECT_KEY, key.as_bytes())
            .await
            .context("write key to storage")?;
        Ok(())
    }
}

impl AsRef<StorageClient> for AcmeStorage {
    fn as_ref(&self) -> &StorageClient {
        &self.client
    }
}

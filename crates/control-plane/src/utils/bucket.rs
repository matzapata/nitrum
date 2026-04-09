//! Generic S3 object helpers (download).

use anyhow::{Context, Result};
use std::path::Path;
use tracing::info;

/// S3 bucket scoped to an S3 client and bucket name (same idea as the CLI `utils::bucket::Bucket`).
pub struct Bucket {
    client: aws_sdk_s3::Client,
    name: String,
}

impl Bucket {
    #[must_use]
    pub fn new(aws_sdk_config: &aws_config::SdkConfig, name: impl Into<String>) -> Self {
        Self {
            client: aws_sdk_s3::Client::new(aws_sdk_config),
            name: name.into(),
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Downloads an object to `dest` (overwrites if present).
    ///
    /// # Errors
    ///
    /// Returns an error when `GetObject`, reading the body, or writing `dest` fails.
    pub async fn download(&self, key: &str, dest: impl AsRef<Path>) -> Result<()> {
        let bucket = self.name.as_str();
        let dest = dest.as_ref();
        info!(%bucket, %key, path = %dest.display(), "S3 GetObject");
        let resp = self
            .client
            .get_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
            .with_context(|| format!("S3 GetObject s3://{bucket}/{key}"))?;
        let body = resp.body.collect().await.context("read S3 object body")?;
        let bytes = body.into_bytes();
        tokio::fs::write(dest, &bytes)
            .await
            .with_context(|| format!("write {}", dest.display()))?;
        info!(len = bytes.len(), "S3 download finished");
        Ok(())
    }
}

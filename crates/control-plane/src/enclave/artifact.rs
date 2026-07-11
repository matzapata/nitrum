//! Resolved path to the enclave EIF on disk (downloaded from S3 or bind-mounted).

use crate::constants::ARTIFACTS_DIR;
use crate::storage::Bucket;
use anyhow::{Context, Result, bail};
use config::artifact::{eif_s3_key, validate_eif_version_label};
use std::path::{Path, PathBuf};

/// Runtime EIF location used by the enclave supervisor.
#[derive(Clone)]
pub struct RuntimeEif {
    /// Absolute or cwd-relative path to the EIF file on the host.
    path: PathBuf,
}

impl RuntimeEif {
    /// Local EIF path (for example `--eif` with a bind-mounted file). Fails if the path is missing or not a regular file.
    pub fn try_from_local(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if !path.is_file() {
            bail!(
                "EIF path does not exist or is not a file: {}",
                path.display()
            );
        }
        Ok(Self { path })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Downloads `s3://{bucket}/{version_label}.eif` into `{ARTIFACTS_DIR}/{version_label}.eif`.
    pub async fn try_from_bucket(bucket: Bucket, version_label: &str) -> Result<Self> {
        let version_label =
            validate_eif_version_label(version_label).map_err(|e| anyhow::anyhow!("{e}"))?;
        let s3_key = eif_s3_key(version_label);
        let dir = PathBuf::from(ARTIFACTS_DIR);
        tokio::fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("create {}", dir.display()))?;
        let path = dir.join(&s3_key);
        bucket.download(&s3_key, &path).await?;
        Ok(Self { path })
    }
}

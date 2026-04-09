use crate::constants::ARTIFACTS_DIR;
use crate::utils::bucket::Bucket;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

/// Resolved path to the enclave EIF on disk (downloaded from S3 or bind-mounted).
#[derive(Clone)]
pub struct EnclaveArtifact {
    path: PathBuf,
}

impl EnclaveArtifact {
    /// Local EIF path (for example `--eif` with a bind-mounted file). Fails if the path is missing or not a regular file.
    pub fn try_from_local(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if !path.is_file() {
            bail!("EIF path does not exist or is not a file: {}", path.display());
        }
        Ok(Self { path })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Downloads `s3://{bucket}/{hash}.eif` into `{ARTIFACTS_DIR}/{hash}.eif` (same key convention as `nitrum cloud deploy`).
    pub async fn try_from_bucket(bucket: Bucket, artifact_hash: &str) -> Result<Self> {
        let artifact_hash = validate_artifact_hash(artifact_hash)?;
        let s3_key = format!("{artifact_hash}.eif");
        let dir = PathBuf::from(ARTIFACTS_DIR);
        tokio::fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("create {}", dir.display()))?;
        let path = dir.join(&s3_key);
        bucket.download(&s3_key, &path).await?;
        Ok(Self { path })
    }
}

fn validate_artifact_hash(hash: &str) -> Result<&str> {
    let hash = hash.trim();
    if hash.is_empty() {
        bail!("artifact hash must not be empty");
    }
    if hash.contains('/') || hash.contains('\\') || hash.contains("..") {
        bail!("artifact hash must not contain path separators or '..'");
    }
    Ok(hash)
}

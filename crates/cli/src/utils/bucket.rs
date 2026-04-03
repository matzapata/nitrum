//! S3 bucket helpers (create, upload, delete).

use anyhow::{Context, Result, bail};
use aws_sdk_s3::error::SdkError as S3SdkError;
use aws_sdk_s3::operation::head_bucket::HeadBucketError;
use aws_sdk_s3::operation::head_object::HeadObjectError;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{
    BucketLocationConstraint, CreateBucketConfiguration, Delete, ObjectIdentifier,
};
use std::path::Path;
use tracing::{info, warn};

/// S3 bucket scoped to an S3 client and bucket name.
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

    /// Ensures the bucket exists in this account (head, then create if needed).
    ///
    /// # Errors
    ///
    /// Returns an error when `HeadBucket`/`CreateBucket` fail with an
    /// unrecoverable error or when the region cannot be derived.
    pub async fn create_if_non_existent(&self) -> Result<()> {
        let region = self.client.config().region().map_or_else(
            || {
                std::env::var("AWS_REGION")
                    .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
                    .unwrap_or_else(|_| "us-east-1".to_string())
            },
            |r| r.as_ref().to_string(),
        );

        let bucket = self.name.as_str();
        info!(%bucket, %region, "S3 HeadBucket (check if bucket exists)");
        if self
            .client
            .head_bucket()
            .bucket(bucket)
            .send()
            .await
            .is_ok()
        {
            info!(%bucket, "S3 bucket already exists for this account");
            return Ok(());
        }

        info!(%bucket, %region, "S3 CreateBucket");
        let mut req = self.client.create_bucket().bucket(bucket);
        if let Some(lc) = bucket_location_constraint(&region)? {
            let cbc = CreateBucketConfiguration::builder()
                .location_constraint(lc)
                .build();
            req = req.create_bucket_configuration(cbc);
        }

        match req.send().await {
            Ok(_) => {
                info!(%bucket, "S3 bucket created");
                Ok(())
            }
            Err(e) => {
                let msg = format!("{e:?}");
                warn!(%bucket, err = %msg, "S3 CreateBucket error");
                if msg.contains("BucketAlreadyOwnedByYou") {
                    Ok(())
                } else if msg.contains("BucketAlreadyExists") {
                    bail!(
                        "S3 bucket `{bucket}` already exists in another account; use a different `project.name` in nitrum.toml"
                    );
                } else {
                    Err(e.into())
                }
            }
        }
    }

    /// Uploads a file unless an object with the same key already exists.
    ///
    /// # Errors
    ///
    /// Returns an error when `HeadObject`/`PutObject` or local file reads fail.
    pub async fn upload(&self, key: &str, path: &Path) -> Result<()> {
        let bucket = self.name.as_str();
        match self
            .client
            .head_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
        {
            Ok(_) => {
                info!(%bucket, %key, "S3 object already present; skipping upload");
            }
            Err(e) => {
                if !head_object_is_not_found(&e) {
                    return Err(e.into());
                }
                let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                info!(
                    %bucket,
                    %key,
                    path = %path.display(),
                    bytes = len,
                    "S3 PutObject (upload EIF)"
                );
                let body = ByteStream::read_from()
                    .path(path)
                    .build()
                    .await
                    .with_context(|| format!("read file {}", path.display()))?;
                self.client
                    .put_object()
                    .bucket(bucket)
                    .key(key)
                    .body(body)
                    .send()
                    .await
                    .context("S3 PutObject failed")?;
                info!(%bucket, %key, "S3 upload finished");
            }
        }
        Ok(())
    }

    /// Deletes all objects and then the bucket. No-op if the bucket does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error when listing, deleting objects, or deleting the bucket
    /// fails with an unrecoverable error.
    pub async fn destroy(&self) -> Result<()> {
        let bucket = self.name.as_str();

        match self.client.head_bucket().bucket(bucket).send().await {
            Ok(_) => {}
            Err(e) => {
                if head_bucket_is_not_found(&e) {
                    info!(%bucket, "S3 bucket does not exist — skip delete");
                    return Ok(());
                }
                return Err(e.into());
            }
        }

        info!(%bucket, "S3 emptying bucket before DeleteBucket");
        loop {
            let resp = self
                .client
                .list_objects_v2()
                .bucket(bucket)
                .max_keys(1000)
                .send()
                .await
                .with_context(|| format!("ListObjectsV2 `{bucket}`"))?;

            let contents = resp.contents();
            if contents.is_empty() {
                break;
            }

            let mut objects = Vec::with_capacity(contents.len());
            for o in contents {
                let Some(key) = o.key() else { continue };
                objects.push(
                    ObjectIdentifier::builder()
                        .key(key)
                        .build()
                        .map_err(|e| anyhow::anyhow!("ObjectIdentifier: {e}"))?,
                );
            }

            if objects.is_empty() {
                break;
            }

            let delete = Delete::builder()
                .set_objects(Some(objects))
                .build()
                .context("build Delete for DeleteObjects")?;

            let out = self
                .client
                .delete_objects()
                .bucket(bucket)
                .delete(delete)
                .send()
                .await
                .context("DeleteObjects")?;

            let errors = out.errors();
            if !errors.is_empty() {
                let detail: Vec<String> = errors
                    .iter()
                    .map(|e| format!("{}: {}", e.key().unwrap_or("?"), e.message().unwrap_or("?")))
                    .collect();
                bail!("DeleteObjects failures: {}", detail.join("; "));
            }
        }

        self.client
            .delete_bucket()
            .bucket(bucket)
            .send()
            .await
            .with_context(|| format!("DeleteBucket `{bucket}`"))?;
        info!(%bucket, "S3 bucket deleted");
        Ok(())
    }
}

/// S3 [`CreateBucket`](aws_sdk_s3::Client::create_bucket) requires a [`CreateBucketConfiguration`]
/// with `location_constraint` for every region **except** `us-east-1`, where AWS expects the field
/// to be omitted (legacy default region).
fn bucket_location_constraint(region: &str) -> Result<Option<BucketLocationConstraint>> {
    if region == "us-east-1" {
        return Ok(None);
    }
    let lc: BucketLocationConstraint = region.parse().with_context(|| {
        format!("unknown S3 location region `{region}` (cannot derive CreateBucketConfiguration)")
    })?;
    Ok(Some(lc))
}

fn head_object_is_not_found<R>(err: &S3SdkError<HeadObjectError, R>) -> bool {
    matches!(
        err,
        S3SdkError::ServiceError(ctx) if ctx.err().is_not_found()
    )
}

fn head_bucket_is_not_found<R>(err: &S3SdkError<HeadBucketError, R>) -> bool {
    matches!(
        err,
        S3SdkError::ServiceError(ctx) if ctx.err().is_not_found()
    )
}

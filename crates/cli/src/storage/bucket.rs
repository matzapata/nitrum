//! S3 bucket helpers (create, upload, delete).

use anyhow::{Context, Result, bail};
use aws_sdk_s3::error::SdkError as S3SdkError;
use aws_sdk_s3::operation::get_bucket_policy::GetBucketPolicyError;
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
                let len = std::fs::metadata(path).map_or(0, |m| m.len());
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

    /// Uploads `bytes` to `key`, overwriting any existing object.
    ///
    /// # Errors
    ///
    /// Returns an error when `PutObject` fails.
    pub async fn put_bytes(&self, key: &str, bytes: Vec<u8>) -> Result<()> {
        let bucket = self.name.as_str();
        let len = bytes.len();
        info!(%bucket, %key, bytes = len, "S3 PutObject (overwrite)");
        self.client
            .put_object()
            .bucket(bucket)
            .key(key)
            .body(ByteStream::from(bytes))
            .send()
            .await
            .context("S3 PutObject failed")?;
        info!(%bucket, %key, "S3 upload finished");
        Ok(())
    }

    /// Ensures CloudFormation can `s3:GetObject` on `cloudformation/*` in this bucket.
    ///
    /// Merges a statement (`AllowCloudFormationGetTemplate`) into the existing bucket
    /// policy when one is present; otherwise puts a policy with only that statement.
    /// Skips `PutBucketPolicy` when that Sid already exists with matching Resource
    /// and `aws:SourceAccount` / `aws:SourceArn` conditions.
    ///
    /// # Errors
    ///
    /// Returns an error when Get/PutBucketPolicy fail for reasons other than a
    /// missing policy, or when an existing policy cannot be parsed as JSON.
    pub async fn ensure_cloudformation_template_read_policy(
        &self,
        account_id: &str,
        stack_name: &str,
    ) -> Result<()> {
        let bucket = self.name.as_str();
        let region = self.region();
        let partition = s3_partition(&region);
        let resource = format!("arn:{partition}:s3:::{bucket}/cloudformation/*");
        let source_arn = cfn_stack_source_arn(partition, &region, account_id, stack_name);
        let statement = cfn_get_object_statement(&resource, account_id, &source_arn);

        let existing = match self.client.get_bucket_policy().bucket(bucket).send().await {
            Ok(resp) => resp.policy().map(ToOwned::to_owned),
            Err(e) => {
                if get_bucket_policy_is_missing(&e) {
                    None
                } else {
                    return Err(e.into());
                }
            }
        };

        let policy = match existing {
            None => Some(serde_json::json!({
                "Version": "2012-10-17",
                "Statement": [statement],
            })),
            Some(raw) => merge_cfn_template_statement(&raw, statement)?,
        };

        let Some(policy) = policy else {
            info!(
                %bucket,
                "S3 bucket policy already allows CloudFormation template GetObject; skipping PutBucketPolicy"
            );
            return Ok(());
        };

        let policy_str =
            serde_json::to_string(&policy).context("serialize S3 bucket policy JSON")?;
        info!(%bucket, "S3 PutBucketPolicy (CloudFormation template GetObject)");
        self.client
            .put_bucket_policy()
            .bucket(bucket)
            .policy(policy_str)
            .send()
            .await
            .context("PutBucketPolicy failed")?;
        Ok(())
    }

    fn region(&self) -> String {
        self.client.config().region().map_or_else(
            || {
                std::env::var("AWS_REGION")
                    .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
                    .unwrap_or_else(|_| "us-east-1".to_string())
            },
            |r| r.as_ref().to_string(),
        )
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

fn get_bucket_policy_is_missing<R>(err: &S3SdkError<GetBucketPolicyError, R>) -> bool {
    if let S3SdkError::ServiceError(ctx) = err {
        let code = ctx.err().meta().code();
        if code == Some("NoSuchBucketPolicy") {
            return true;
        }
        ctx.err()
            .meta()
            .message()
            .is_some_and(|m| m.contains("does not exist") || m.contains("NoSuchBucketPolicy"))
    } else {
        false
    }
}

fn s3_partition(region: &str) -> &'static str {
    if region.starts_with("us-gov-") {
        "aws-us-gov"
    } else if region.starts_with("cn-") {
        "aws-cn"
    } else {
        "aws"
    }
}

const CFN_TEMPLATE_POLICY_SID: &str = "AllowCloudFormationGetTemplate";

fn cfn_stack_source_arn(
    partition: &str,
    region: &str,
    account_id: &str,
    stack_name: &str,
) -> String {
    format!("arn:{partition}:cloudformation:{region}:{account_id}:stack/{stack_name}/*")
}

fn cfn_get_object_statement(
    resource: &str,
    account_id: &str,
    source_arn: &str,
) -> serde_json::Value {
    serde_json::json!({
        "Sid": CFN_TEMPLATE_POLICY_SID,
        "Effect": "Allow",
        "Principal": {"Service": "cloudformation.amazonaws.com"},
        "Action": "s3:GetObject",
        "Resource": resource,
        "Condition": {
            "StringEquals": {
                "aws:SourceAccount": account_id,
            },
            "ArnLike": {
                "aws:SourceArn": source_arn,
            },
        },
    })
}

/// `None` when the Sid already exists with matching Resource and SourceAccount/SourceArn.
fn merge_cfn_template_statement(
    raw_policy: &str,
    statement: serde_json::Value,
) -> Result<Option<serde_json::Value>> {
    let mut policy: serde_json::Value =
        serde_json::from_str(raw_policy).context("parse existing S3 bucket policy JSON")?;
    let obj = policy
        .as_object_mut()
        .context("S3 bucket policy JSON must be an object")?;
    let statements = obj
        .entry("Statement")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    if statements.is_object() {
        let existing = statements.take();
        *statements = serde_json::Value::Array(vec![existing]);
    }
    let list = statements
        .as_array_mut()
        .context("S3 bucket policy Statement must be an object or array")?;
    if list.iter().any(|s| {
        policy_statement_has_cfn_sid(s) && cfn_statement_resource_and_condition_match(s, &statement)
    }) {
        return Ok(None);
    }
    if let Some(slot) = list.iter_mut().find(|s| policy_statement_has_cfn_sid(s)) {
        *slot = statement;
    } else {
        list.push(statement);
    }
    Ok(Some(policy))
}

fn policy_statement_has_cfn_sid(stmt: &serde_json::Value) -> bool {
    stmt.get("Sid").and_then(serde_json::Value::as_str) == Some(CFN_TEMPLATE_POLICY_SID)
}

fn cfn_statement_resource_and_condition_match(
    existing: &serde_json::Value,
    desired: &serde_json::Value,
) -> bool {
    json_same_resource(existing.get("Resource"), desired.get("Resource"))
        && condition_key(existing, "StringEquals", "aws:SourceAccount")
            == condition_key(desired, "StringEquals", "aws:SourceAccount")
        && condition_key(existing, "ArnLike", "aws:SourceArn")
            == condition_key(desired, "ArnLike", "aws:SourceArn")
        && condition_key(existing, "StringEquals", "aws:SourceAccount").is_some()
        && condition_key(existing, "ArnLike", "aws:SourceArn").is_some()
}

fn json_same_resource(a: Option<&serde_json::Value>, b: Option<&serde_json::Value>) -> bool {
    match (normalize_resource_list(a), normalize_resource_list(b)) {
        (Some(a), Some(b)) => a == b,
        _ => a == b,
    }
}

fn normalize_resource_list(v: Option<&serde_json::Value>) -> Option<Vec<&str>> {
    match v {
        Some(serde_json::Value::String(s)) => Some(vec![s.as_str()]),
        Some(serde_json::Value::Array(arr)) => arr.iter().map(serde_json::Value::as_str).collect(),
        _ => None,
    }
}

fn condition_key(stmt: &serde_json::Value, operator: &str, key: &str) -> Option<String> {
    let v = stmt.get("Condition")?.get(operator)?.get(key)?;
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(a) if a.len() == 1 => a[0].as_str().map(ToOwned::to_owned),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_statement() -> serde_json::Value {
        cfn_get_object_statement(
            "arn:aws:s3:::b/cloudformation/*",
            "123456789012",
            "arn:aws:cloudformation:us-east-1:123456789012:stack/nitrum-app/*",
        )
    }

    #[test]
    fn statement_includes_source_account_and_arn() {
        let stmt = sample_statement();
        assert_eq!(
            stmt["Condition"]["StringEquals"]["aws:SourceAccount"],
            "123456789012"
        );
        assert_eq!(
            stmt["Condition"]["ArnLike"]["aws:SourceArn"],
            "arn:aws:cloudformation:us-east-1:123456789012:stack/nitrum-app/*"
        );
    }

    #[test]
    fn merge_appends_when_sid_missing() {
        let existing = r#"{"Version":"2012-10-17","Statement":[{"Sid":"Other","Effect":"Allow"}]}"#;
        let merged = merge_cfn_template_statement(existing, sample_statement())
            .expect("merge")
            .expect("needs put");
        let stmts = merged["Statement"].as_array().expect("array");
        assert_eq!(stmts.len(), 2);
        assert!(stmts.iter().any(policy_statement_has_cfn_sid));
    }

    #[test]
    fn merge_skips_put_when_resource_and_condition_match() {
        let existing = serde_json::json!({
            "Version": "2012-10-17",
            "Statement": [sample_statement()],
        })
        .to_string();
        assert!(
            merge_cfn_template_statement(&existing, sample_statement())
                .expect("merge")
                .is_none()
        );
    }

    #[test]
    fn merge_skips_put_when_aws_returns_array_condition_values() {
        let existing = r#"{
            "Version":"2012-10-17",
            "Statement":[{
                "Sid":"AllowCloudFormationGetTemplate",
                "Resource":["arn:aws:s3:::b/cloudformation/*"],
                "Condition":{
                    "StringEquals":{"aws:SourceAccount":["123456789012"]},
                    "ArnLike":{"aws:SourceArn":["arn:aws:cloudformation:us-east-1:123456789012:stack/nitrum-app/*"]}
                }
            }]
        }"#;
        assert!(
            merge_cfn_template_statement(existing, sample_statement())
                .expect("merge")
                .is_none()
        );
    }

    #[test]
    fn merge_replaces_when_source_account_missing() {
        let existing = r#"{"Version":"2012-10-17","Statement":[{"Sid":"AllowCloudFormationGetTemplate","Resource":"arn:aws:s3:::b/cloudformation/*"}]}"#;
        let merged = merge_cfn_template_statement(existing, sample_statement())
            .expect("merge")
            .expect("needs put");
        let stmts = merged["Statement"].as_array().expect("array");
        assert_eq!(stmts.len(), 1);
        assert_eq!(
            stmts[0]["Condition"]["StringEquals"]["aws:SourceAccount"],
            "123456789012"
        );
    }

    #[test]
    fn source_arn_uses_stack_wildcard() {
        assert_eq!(
            cfn_stack_source_arn("aws", "eu-west-1", "123456789012", "nitrum-app"),
            "arn:aws:cloudformation:eu-west-1:123456789012:stack/nitrum-app/*"
        );
    }

    #[test]
    fn partitions() {
        assert_eq!(s3_partition("us-east-1"), "aws");
        assert_eq!(s3_partition("us-gov-west-1"), "aws-us-gov");
        assert_eq!(s3_partition("cn-north-1"), "aws-cn");
    }
}

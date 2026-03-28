//! AWS SDK helpers (S3 + CloudFormation) — replaces shelling out to the AWS CLI.
// TODO: cleanup

use anyhow::{Context, Result, bail};
use aws_config::Region;
use aws_sdk_cloudformation::error::SdkError as CfSdkError;
use aws_sdk_cloudformation::operation::RequestId;
use aws_sdk_cloudformation::operation::describe_stacks::DescribeStacksError;
use aws_sdk_cloudformation::types::{Capability, Parameter, StackStatus};
use aws_sdk_s3::error::SdkError as S3SdkError;
use aws_sdk_s3::operation::head_bucket::HeadBucketError;
use aws_sdk_s3::operation::head_object::HeadObjectError;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{
    BucketLocationConstraint, CreateBucketConfiguration, Delete, ObjectIdentifier,
};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::artifact::EnclaveArtifact;

pub struct EnclaveCloudStack {
    config: aws_config::SdkConfig,
    project_root: PathBuf,
    stack_name: String,
    bucket: String,
}

impl EnclaveCloudStack {
    /// Loads AWS configuration, writes the bundled CloudFormation template under `.nitrum/` for inspection,
    /// and returns the stack handle.
    pub async fn new(
        project_root: PathBuf,
        stack_name: String,
        region_override: Option<String>,
    ) -> Result<Self> {
        Self::write_bundled_template(&project_root)?;
        let bucket = derived_eif_bucket_name(&stack_name)?;
        let config = sdk_config(region_override).await;
        Ok(Self {
            config,
            project_root,
            stack_name,
            bucket,
        })
    }

    /// Resolved region for user-facing messages (`AWS_REGION` / profile when no explicit override).
    #[must_use]
    pub fn region_display(&self) -> String {
        self.config
            .region()
            .map(|r| r.as_ref().to_string())
            .unwrap_or_else(resolve_aws_region)
    }

    #[must_use]
    pub fn bucket_name(&self) -> &str {
        &self.bucket
    }

    fn template_path(&self) -> PathBuf {
        self.project_root
            .join(crate::constants::ENCLAVE_CLOUD_STACK_TEMPLATE_FILE)
    }

    fn write_bundled_template(project_root: &Path) -> Result<()> {
        let path = project_root.join(crate::constants::ENCLAVE_CLOUD_STACK_TEMPLATE_FILE);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        std::fs::write(&path, crate::constants::cloud_stack_template())
            .with_context(|| format!("write {}", path.display()))?;
        Ok(())
    }

    /// Upload EIF artifact and create/update the stack, then return stack outputs.
    pub async fn deploy(
        &self,
        artifact: &EnclaveArtifact,
        retain: bool,
        control_plane_image_tag: &str,
    ) -> Result<std::collections::BTreeMap<String, String>> {
        let eif_label = eif_version_label_from_hash(&artifact.hash);
        ensure_bucket_exists(&self.config, &self.bucket).await?;
        s3_put_file_if_needed(&self.config, &self.bucket, &eif_label, &artifact.eif_path).await?;

        let retain_str = if retain { "true" } else { "false" };
        let params = vec![
            ("EnvironmentName".to_string(), self.stack_name.clone()),
            ("Retain".to_string(), retain_str.to_string()),
            ("EifS3Bucket".to_string(), self.bucket.clone()),
            ("EifS3Key".to_string(), eif_label.clone()),
            ("EifVersionLabel".to_string(), eif_label),
            ("AsgMinSize".to_string(), "1".to_string()),
            ("AsgMaxSize".to_string(), "1".to_string()),
            ("AsgDesiredCapacity".to_string(), "1".to_string()),
            (
                "ControlPlaneImageTag".to_string(),
                control_plane_image_tag.to_string(),
            ),
        ];

        let template_path = self.template_path();
        let template_body = std::fs::read_to_string(&template_path)
            .with_context(|| format!("read CloudFormation template {}", template_path.display()))?;
        cloudformation_deploy(&self.config, &self.stack_name, &template_body, &params).await?;
        cloudformation_stack_outputs(&self.config, &self.stack_name).await
    }

    /// Delete this stack and wait until deletion finishes.
    pub async fn destroy(&self) -> Result<()> {
        cloudformation_delete_stack_wait(&self.config, &self.stack_name).await?;
        s3_empty_and_delete_bucket(&self.config, &self.bucket).await
    }
}

fn sanitize_bucket_label(s: &str) -> String {
    s.trim()
        .trim_matches(|c| c == '.' || c == '-')
        .chars()
        .map(|c| match c {
            'A'..='Z' => c.to_ascii_lowercase(),
            'a'..='z' | '0'..='9' | '-' | '.' => c,
            _ => '-',
        })
        .collect()
}

/// Default EIF bucket: `nitrum-{project-name}` (S3 label rules, ≤63 chars).
fn derived_eif_bucket_name(project_name: &str) -> Result<String> {
    let slug = sanitize_bucket_label(project_name);
    if slug.is_empty() {
        bail!("`name` in nitrum.toml is empty after sanitization; set a valid project slug");
    }
    let s = format!("nitrum-{slug}");
    if !(3..=63).contains(&s.len()) {
        bail!("derived S3 bucket name `{s}` is not 3-63 characters; shorten `name` in nitrum.toml");
    }
    Ok(s)
}

/// `AWS_REGION`, then `AWS_DEFAULT_REGION`, then `us-east-1`.
#[must_use]
pub fn resolve_aws_region() -> String {
    std::env::var("AWS_REGION")
        .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
        .unwrap_or_else(|_| "us-east-1".to_string())
}

/// First 12 lowercase hex chars of a SHA-256 hex string.
#[must_use]
pub fn eif_version_label_from_hash(hash: &str) -> String {
    hash.chars().take(12).collect()
}

/// Shared SDK config (credentials + region from the default chain; see [`aws_config::load_from_env`]).
///
/// When `region_override` is set, it wins over `AWS_REGION` / profile / IMDS for the resolved config.
pub async fn sdk_config(region_override: Option<String>) -> aws_config::SdkConfig {
    match &region_override {
        Some(r) => {
            info!(region = %r, "loading AWS config (region from --region)");
            aws_config::from_env()
                .region(Region::new(r.clone()))
                .load()
                .await
        }
        None => {
            info!("loading AWS config (default chain: env, profile, IMDS, …)");
            aws_config::load_from_env().await
        }
    }
}

/// CloudFormation returns `ValidationError` with message `Stack with id … does not exist`, but
/// [`DescribeStacksError`]'s [`std::fmt::Display`] is only `unhandled error (ValidationError)` — the
/// detail is in metadata. String-matching `to_string()` misses it and surfaces a false failure.
fn describe_stacks_reports_missing_stack<R>(err: &CfSdkError<DescribeStacksError, R>) -> bool {
    match err {
        CfSdkError::ServiceError(ctx) => {
            let e = ctx.err();
            let code = e.meta().code();
            let msg = e.meta().message();
            let missing = code == Some("ValidationError")
                && msg.is_some_and(|m| m.contains("does not exist"));
            debug!(
                code,
                message = msg,
                request_id = e.request_id(),
                missing_stack = missing,
                "DescribeStacks service error"
            );
            missing
        }
        _ => {
            debug!(error = %err, "DescribeStacks non-service error");
            false
        }
    }
}

fn bucket_location_constraint(region: &str) -> Result<Option<BucketLocationConstraint>> {
    if region == "us-east-1" {
        return Ok(None);
    }
    let lc: BucketLocationConstraint = region.parse().with_context(|| {
        format!("unknown S3 location region `{region}` (cannot derive CreateBucketConfiguration)")
    })?;
    Ok(Some(lc))
}

/// Create the bucket if it is missing; succeed if it already exists for this account.
pub async fn ensure_bucket_exists(config: &aws_config::SdkConfig, bucket: &str) -> Result<()> {
    let client = aws_sdk_s3::Client::new(config);
    let region = config
        .region()
        .map(|r| r.as_ref().to_string())
        .unwrap_or_else(resolve_aws_region);

    info!(%bucket, %region, "S3 HeadBucket (check if bucket exists)");
    if client.head_bucket().bucket(bucket).send().await.is_ok() {
        info!(%bucket, "S3 bucket already exists for this account");
        return Ok(());
    }

    info!(%bucket, %region, "S3 CreateBucket");
    let mut req = client.create_bucket().bucket(bucket);
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
                    "S3 bucket `{bucket}` already exists in another account; use a different `name` in nitrum.toml"
                );
            } else {
                Err(e.into())
            }
        }
    }
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

/// List and delete all objects, then delete the bucket. No-op if the bucket does not exist.
pub async fn s3_empty_and_delete_bucket(
    config: &aws_config::SdkConfig,
    bucket: &str,
) -> Result<()> {
    let client = aws_sdk_s3::Client::new(config);

    match client.head_bucket().bucket(bucket).send().await {
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
        let resp = client
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

        let out = client
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

    client
        .delete_bucket()
        .bucket(bucket)
        .send()
        .await
        .with_context(|| format!("DeleteBucket `{bucket}`"))?;
    info!(%bucket, "S3 bucket deleted");
    Ok(())
}

/// Upload `path` to `s3://bucket/key` unless the object already exists with the **same size** as the local file (cheap `HeadObject` check).
pub async fn s3_put_file_if_needed(
    config: &aws_config::SdkConfig,
    bucket: &str,
    key: &str,
    path: &Path,
) -> Result<()> {
    let meta = std::fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    let local_len: i64 = meta.len().try_into().unwrap_or(-1);

    let client = aws_sdk_s3::Client::new(config);
    let head = client.head_object().bucket(bucket).key(key).send().await;

    let upload = match head {
        Ok(resp) => {
            let remote = resp.content_length().unwrap_or(-1);
            if remote == local_len && local_len >= 0 {
                info!(
                    %bucket,
                    %key,
                    bytes = local_len,
                    "S3 object already present with same size; skipping upload"
                );
                false
            } else {
                info!(
                    %bucket,
                    %key,
                    remote_len = remote,
                    local_len,
                    "S3 object missing or size differs; uploading"
                );
                true
            }
        }
        Err(e) => {
            if head_object_is_not_found(&e) {
                info!(%bucket, %key, "S3 object not found; uploading");
                true
            } else {
                return Err(e.into());
            }
        }
    };

    if upload {
        s3_put_file(config, bucket, key, path).await?;
    }
    Ok(())
}

/// Upload a local file to S3 (replaces `aws s3 cp`).
pub async fn s3_put_file(
    config: &aws_config::SdkConfig,
    bucket: &str,
    key: &str,
    path: &Path,
) -> Result<()> {
    let client = aws_sdk_s3::Client::new(config);
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
    client
        .put_object()
        .bucket(bucket)
        .key(key)
        .body(body)
        .send()
        .await
        .context("S3 PutObject failed")?;
    info!(%bucket, %key, "S3 upload finished");
    Ok(())
}

fn cf_parameters(pairs: &[(String, String)]) -> Vec<Parameter> {
    pairs
        .iter()
        .map(|(k, v)| {
            Parameter::builder()
                .parameter_key(k)
                .parameter_value(v)
                .build()
        })
        .collect()
}

fn stack_in_progress(status: Option<&StackStatus>) -> bool {
    matches!(
        status,
        Some(
            StackStatus::CreateInProgress
                | StackStatus::RollbackInProgress
                | StackStatus::DeleteInProgress
                | StackStatus::UpdateInProgress
                | StackStatus::UpdateCompleteCleanupInProgress
                | StackStatus::UpdateRollbackInProgress
                | StackStatus::ReviewInProgress
                | StackStatus::ImportInProgress
                | StackStatus::ImportRollbackInProgress
        )
    )
}

fn stack_failed(status: Option<&StackStatus>) -> bool {
    matches!(
        status,
        Some(
            StackStatus::CreateFailed
                | StackStatus::RollbackFailed
                | StackStatus::RollbackComplete
                | StackStatus::DeleteFailed
                | StackStatus::UpdateFailed
                | StackStatus::UpdateRollbackFailed
                | StackStatus::UpdateRollbackComplete
                | StackStatus::ImportRollbackFailed
        )
    )
}

async fn wait_stack_stable(
    client: &aws_sdk_cloudformation::Client,
    stack_name: &str,
) -> Result<()> {
    let mut tick: u32 = 0;
    loop {
        let resp = client
            .describe_stacks()
            .stack_name(stack_name)
            .send()
            .await
            .context("DescribeStacks while waiting")?;
        let stack = resp.stacks().first().context("stack missing during wait")?;
        let status = stack.stack_status();
        tick = tick.saturating_add(1);
        if tick == 1 || tick % 6 == 0 {
            info!(
                %stack_name,
                ?status,
                reason = stack.stack_status_reason(),
                "CloudFormation stack wait poll"
            );
        } else {
            debug!(%stack_name, ?status, "CloudFormation stack wait poll");
        }
        if matches!(
            status,
            Some(StackStatus::CreateComplete | StackStatus::UpdateComplete)
        ) {
            info!(%stack_name, ?status, "CloudFormation stack reached stable success");
            return Ok(());
        }
        if stack_failed(status) {
            let reason = stack.stack_status_reason().unwrap_or("no reason returned");
            bail!("stack `{stack_name}` failed: {:?} — {reason}", status);
        }
        if stack_in_progress(status) || status.is_none() {
            sleep(Duration::from_secs(5)).await;
            continue;
        }
        bail!("stack `{stack_name}` unexpected status: {:?}", status);
    }
}

/// Create or update a stack from an in-memory template body (`CAPABILITY_IAM`).
pub async fn cloudformation_deploy(
    config: &aws_config::SdkConfig,
    stack_name: &str,
    template_body: &str,
    parameter_pairs: &[(String, String)],
) -> Result<()> {
    let client = aws_sdk_cloudformation::Client::new(config);
    let parameters = cf_parameters(parameter_pairs);
    let cap = Capability::CapabilityIam;

    info!(%stack_name, param_count = parameter_pairs.len(), "DescribeStacks (create vs update)");
    let describe = client.describe_stacks().stack_name(stack_name).send().await;
    let exists = match describe {
        Ok(ref resp) => {
            let st = resp.stacks().first();
            let status = st.and_then(|s| s.stack_status());
            let exists = status.is_some_and(|s| *s != StackStatus::DeleteComplete);
            info!(
                %stack_name,
                ?status,
                stacks_returned = resp.stacks().len(),
                exists,
                "DescribeStacks OK"
            );
            exists
        }
        Err(e) => {
            if describe_stacks_reports_missing_stack(&e) {
                info!(%stack_name, "stack not found — will CreateStack");
                false
            } else {
                warn!(%stack_name, error = ?e, "DescribeStacks failed");
                return Err(e.into());
            }
        }
    };

    if !exists {
        info!(%stack_name, template_chars = template_body.len(), "CreateStack");
        client
            .create_stack()
            .stack_name(stack_name)
            .template_body(template_body)
            .set_parameters(Some(parameters))
            .capabilities(cap)
            .send()
            .await
            .context("CreateStack failed")?;
        info!(%stack_name, "CreateStack accepted; waiting for completion");
    } else {
        info!(%stack_name, "UpdateStack");
        let upd = client
            .update_stack()
            .stack_name(stack_name)
            .template_body(template_body)
            .set_parameters(Some(parameters))
            .capabilities(cap)
            .send()
            .await;

        if let Err(e) = upd {
            let s = e.to_string();
            let dbg = format!("{e:?}");
            if s.contains("No updates are to be performed")
                || dbg.contains("No updates are to be performed")
            {
                info!(%stack_name, "no template/parameter changes — skipping wait");
                return Ok(());
            }
            warn!(%stack_name, display = %s, debug = %dbg, "UpdateStack failed");
            return Err(e.into());
        }
        info!(%stack_name, "UpdateStack accepted; waiting for completion");
    }

    wait_stack_stable(&client, stack_name).await
}

/// Delete stack and block until `DELETE_COMPLETE` (or missing).
pub async fn cloudformation_delete_stack_wait(
    config: &aws_config::SdkConfig,
    stack_name: &str,
) -> Result<()> {
    let client = aws_sdk_cloudformation::Client::new(config);

    info!(%stack_name, "destroy: DescribeStacks");
    let describe = client.describe_stacks().stack_name(stack_name).send().await;
    match describe {
        Ok(resp) => {
            if let Some(st) = resp.stacks().first() {
                let status = st.stack_status();
                if matches!(status, Some(StackStatus::DeleteComplete)) {
                    info!(%stack_name, "stack already DELETE_COMPLETE — nothing to do");
                    return Ok(());
                }
                info!(%stack_name, ?status, "stack exists — will DeleteStack");
            } else {
                info!(%stack_name, "DescribeStacks returned no stacks — treating as gone");
                return Ok(());
            }
        }
        Err(e) => {
            if describe_stacks_reports_missing_stack(&e) {
                info!(%stack_name, "stack does not exist — nothing to delete");
                return Ok(());
            }
            warn!(%stack_name, error = ?e, "DescribeStacks failed");
            return Err(e.into());
        }
    }

    info!(%stack_name, "DeleteStack");
    client
        .delete_stack()
        .stack_name(stack_name)
        .send()
        .await
        .context("DeleteStack failed")?;
    info!(%stack_name, "DeleteStack accepted; polling until gone");

    let mut n = 0u32;
    loop {
        let desc = client.describe_stacks().stack_name(stack_name).send().await;
        match desc {
            Err(e) => {
                if describe_stacks_reports_missing_stack(&e) {
                    info!(%stack_name, "DescribeStacks: stack gone after delete");
                    return Ok(());
                }
                warn!(%stack_name, error = ?e, "DescribeStacks during delete wait");
                return Err(e.into());
            }
            Ok(resp) => {
                let st = resp.stacks().first();
                let status = st.and_then(|x| x.stack_status());
                n = n.saturating_add(1);
                if n == 1 || n % 6 == 0 {
                    info!(%stack_name, ?status, "delete wait poll");
                } else {
                    debug!(%stack_name, ?status, "delete wait poll");
                }
                match status {
                    Some(StackStatus::DeleteComplete) | None => {
                        info!(%stack_name, "stack delete finished");
                        return Ok(());
                    }
                    Some(StackStatus::DeleteFailed) => {
                        bail!("stack `{stack_name}` delete failed");
                    }
                    Some(StackStatus::DeleteInProgress) | _ => {
                        sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        }
    }
}

/// Describe stack outputs as stable-sorted key → value (skips outputs with no value).
pub async fn cloudformation_stack_outputs(
    config: &aws_config::SdkConfig,
    stack_name: &str,
) -> Result<std::collections::BTreeMap<String, String>> {
    let client = aws_sdk_cloudformation::Client::new(config);
    let resp = client
        .describe_stacks()
        .stack_name(stack_name)
        .send()
        .await
        .with_context(|| format!("DescribeStacks for stack `{stack_name}` outputs"))?;
    let stack = resp
        .stacks()
        .first()
        .with_context(|| format!("DescribeStacks returned no stacks for `{stack_name}`"))?;
    let mut map = std::collections::BTreeMap::new();
    for o in stack.outputs() {
        let key = o.output_key().unwrap_or("").trim();
        if key.is_empty() {
            continue;
        }
        if let Some(v) = o.output_value() {
            map.insert(key.to_string(), v.to_string());
        }
    }
    Ok(map)
}

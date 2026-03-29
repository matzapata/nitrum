//! CloudFormation stack and EIF S3 bucket helpers.

use anyhow::{Result, bail};
use std::collections::BTreeMap;

use crate::artifact::EnclaveArtifact;
use crate::utils::bucket::Bucket;
use crate::utils::cloudformation::CloudFormation;

pub struct EnclaveCloudStack {
    bucket: Bucket,
    cloudformation: CloudFormation,
}

impl EnclaveCloudStack {
    /// Loads AWS configuration and returns the stack handle.
    pub async fn new(stack_name: String) -> Result<Self> {
        let bucket_name = Self::derived_eif_bucket_name(&stack_name)?;
        let aws_sdk_config = aws_config::load_from_env().await;
        Ok(Self {
            bucket: Bucket::new(&aws_sdk_config, bucket_name),
            cloudformation: CloudFormation::new(
                &aws_sdk_config,
                stack_name,
                Self::cloud_stack_template(),
            ),
        })
    }

    /// Region string for prompts (`AWS_REGION` / `AWS_DEFAULT_REGION` / `us-east-1`).
    #[must_use]
    pub fn region_display(&self) -> String {
        std::env::var("AWS_REGION")
            .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
            .unwrap_or_else(|_| "us-east-1".to_string())
    }

    #[must_use]
    pub fn bucket_name(&self) -> &str {
        self.bucket.name()
    }

    #[must_use]
    pub fn stack_name(&self) -> &str {
        self.cloudformation.stack_name()
    }

    /// Upload EIF artifact and create/update the stack, then return stack outputs.
    pub async fn deploy(
        &self,
        artifact: &EnclaveArtifact,
        retain: bool,
        control_plane_image_tag: &str,
    ) -> Result<BTreeMap<String, String>> {
        let eif_label = artifact.hash.chars().take(12).collect();
        let retain_str = if retain { "true" } else { "false" };
        let params = vec![
            (
                "EnvironmentName".to_string(),
                self.cloudformation.stack_name().to_string(),
            ),
            ("Retain".to_string(), retain_str.to_string()),
            ("EifS3Bucket".to_string(), self.bucket.name().to_string()),
            ("EifS3Key".to_string(), eif_label.clone()),
            ("EifVersionLabel".to_string(), eif_label.clone()),
            ("AsgMinSize".to_string(), "1".to_string()),
            ("AsgMaxSize".to_string(), "1".to_string()),
            ("AsgDesiredCapacity".to_string(), "1".to_string()),
            (
                "ControlPlaneImageTag".to_string(),
                control_plane_image_tag.to_string(),
            ),
        ];

        self.bucket.create_if_non_existent().await?;
        self.bucket.upload(&eif_label, &artifact.eif_path).await?;

        let need_wait = self.cloudformation.update_if_needed(&params).await?;
        if need_wait {
            self.cloudformation.wait_until_stable().await?;
        }

        self.cloudformation.outputs().await
    }

    /// Delete this stack and wait until deletion finishes, then empty and delete the EIF bucket.
    pub async fn destroy(&self) -> Result<()> {
        self.cloudformation.destroy().await?;
        self.bucket.destroy().await
    }

    fn cloud_stack_template() -> &'static str {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/stack.yml"))
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
        let slug = Self::sanitize_bucket_label(project_name);
        if slug.is_empty() {
            bail!("`name` in nitrum.toml is empty after sanitization; set a valid project slug");
        }
        let s = format!("nitrum-{slug}");
        if !(3..=63).contains(&s.len()) {
            bail!(
                "derived S3 bucket name `{s}` is not 3-63 characters; shorten `name` in nitrum.toml"
            );
        }
        Ok(s)
    }
}

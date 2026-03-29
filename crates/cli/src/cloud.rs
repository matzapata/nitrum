//! CloudFormation stack and EIF S3 bucket helpers.

use anyhow::Result;
use std::collections::BTreeMap;

use crate::artifact::EnclaveArtifact;
use crate::utils::bucket::Bucket;
use crate::utils::cloudformation::CloudFormation;
use shared::config::{NitrumConfig, Scaling};

pub struct EnclaveCloudStack {
    bucket: Bucket,
    cloudformation: CloudFormation,
}

impl EnclaveCloudStack {
    /// Loads AWS configuration and returns the stack handle.
    pub async fn new(config: &NitrumConfig) -> Result<Self> {
        let bucket_name = format!("nitrum-{}", config.name);
        let stack_name = format!("nitrum-{}", config.name);
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
        config: &NitrumConfig,
    ) -> Result<BTreeMap<String, String>> {
        let scaling: &Scaling = &config.scaling;
        let eif_label: String = artifact.hash.chars().take(12).collect();
        let retain_str = if retain { "true" } else { "false" };
        let params = vec![
            ("ProjectName".to_string(), config.name.clone()),
            ("Retain".to_string(), retain_str.to_string()),
            ("EifS3Bucket".to_string(), self.bucket.name().to_string()),
            ("EifS3Key".to_string(), eif_label.clone()),
            ("EifVersionLabel".to_string(), eif_label.clone()),
            ("AsgMinSize".to_string(), scaling.min_replicas.to_string()),
            ("AsgMaxSize".to_string(), scaling.max_replicas.to_string()),
            (
                "AsgDesiredCapacity".to_string(),
                scaling.desired_replicas.to_string(),
            ),
            ("EnclaveCpuCount".to_string(), scaling.num_cpus.to_string()),
            (
                "EnclaveMemoryMib".to_string(),
                scaling.ram_size_mib.to_string(),
            ),
            (
                "ControlPlaneImage".to_string(),
                config.control_plane.clone(),
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
}

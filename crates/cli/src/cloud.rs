//! `CloudFormation` stack and EIF S3 bucket helpers.

use anyhow::Result;
use std::collections::BTreeMap;

use crate::artifact::EnclaveArtifact;
use crate::utils::bucket::Bucket;
use crate::utils::cloudformation::CloudFormation;
use config::{NitrumConfig, Scaling};

pub struct EnclaveCloudStack {
    bucket: Bucket,
    cloudformation: CloudFormation,
}

impl EnclaveCloudStack {
    /// Loads AWS configuration and returns the stack handle.
    ///
    /// # Errors
    ///
    /// Returns an error when the AWS configuration cannot be loaded.
    pub async fn new(config: &NitrumConfig) -> Result<Self> {
        let bucket_name = format!("nitrum-{}", config.project.name);
        let stack_name = format!("nitrum-{}", config.project.name);
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
    ///
    /// # Errors
    ///
    /// Returns an error when S3 bucket creation/upload or CloudFormation
    /// operations fail.
    pub async fn deploy(
        &self,
        artifact: &EnclaveArtifact,
        retain: bool,
        debug_mode: bool,
        config: &NitrumConfig,
        kms_administrator_role_arn: Option<&str>,
    ) -> Result<BTreeMap<String, String>> {
        let scaling: &Scaling = &config.scaling;
        let eif_label: String = artifact.hash.chars().take(12).collect();
        let eif_s3_key = format!("{eif_label}.eif");
        let retain_str = if retain { "true" } else { "false" };
        let control_plane_debug_arg = if debug_mode { "--debug-mode" } else { "" };

        let pcr0 = if debug_mode {
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
        } else {
            artifact.pcr0.trim()
        };
        if pcr0.is_empty() {
            anyhow::bail!(
                "EnclaveArtifact.pcr0 is empty; rebuild the EIF or run `nitrum describe` on it"
            );
        }

        let mut params = vec![
            ("ProjectName".to_string(), config.project.name.clone()),
            ("Retain".to_string(), retain_str.to_string()),
            ("EifS3Bucket".to_string(), self.bucket.name().to_string()),
            ("EifS3Key".to_string(), eif_s3_key.clone()),
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
                config.runtime.control_plane.clone(),
            ),
            (
                "ControlPlaneDebugArg".to_string(),
                control_plane_debug_arg.to_string(),
            ),
            ("EifImageSha384".to_string(), pcr0.to_string()),
        ];
        if let Some(arn) = kms_administrator_role_arn {
            params.push(("KmsAdministratorRoleArn".to_string(), arn.to_string()));
        }

        self.bucket.create_if_non_existent().await?;
        self.bucket.upload(&eif_s3_key, &artifact.eif_path).await?;

        let need_wait = self.cloudformation.update_if_needed(&params).await?;
        if need_wait {
            self.cloudformation.wait_until_stable().await?;
        }

        self.cloudformation.outputs().await
    }

    /// Delete this stack and wait until deletion finishes, then empty and delete the EIF bucket.
    ///
    /// # Errors
    ///
    /// Returns an error when CloudFormation stack deletion or bucket teardown
    /// fail.
    pub async fn destroy(&self) -> Result<()> {
        self.cloudformation.destroy().await?;
        self.bucket.destroy().await
    }

    const fn cloud_stack_template() -> &'static str {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/stack.yml"))
    }
}

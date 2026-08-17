//! `CloudFormation` stack and EIF S3 bucket helpers.

mod cloudformation;
mod ssm;
pub mod template;

use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::artifact::EnclaveArtifact;
use crate::storage::Bucket;
use crate::utils::aws;
use config::artifact::{eif_s3_key, eif_version_label_from_hash};
use config::{NitrumConfig, PlatformLayout, Scaling};

pub use cloudformation::CloudFormation;
pub use ssm::Ssm;
pub use template::{
    BUNDLED_CLOUD_TEMPLATE_VERSION, CFN_TEMPLATE_BODY_MAX_BYTES, CFN_TEMPLATE_S3_KEY,
    StackTemplate, StackTemplateKind, bundled_cloud_stack_template, eject_cloud_template,
    load_cloud_template, parse_template_version, s3_template_url, stack_template_kind,
    warn_template_skew,
};

pub struct EnclaveCloudStack {
    project_root: PathBuf,
    bucket: Bucket,
    cloudformation: CloudFormation,
    region: String,
    sts: aws_sdk_sts::Client,
}

impl EnclaveCloudStack {
    /// Loads AWS configuration and returns the stack handle.
    ///
    /// # Errors
    ///
    /// Returns an error when the AWS configuration cannot be loaded.
    pub async fn new(project_root: &Path, config: &NitrumConfig) -> Result<Self> {
        let layout = PlatformLayout::from_project(&config.project);
        let aws_sdk_config = aws_config::load_from_env().await;
        let region = aws_sdk_config
            .region()
            .map_or_else(aws::region_display, |r| r.as_ref().to_string());
        Ok(Self {
            project_root: project_root.to_path_buf(),
            bucket: Bucket::new(&aws_sdk_config, layout.s3_bucket()),
            cloudformation: CloudFormation::new(&aws_sdk_config, layout.stack_name()),
            region,
            sts: aws_sdk_sts::Client::new(&aws_sdk_config),
        })
    }

    /// Region string for prompts (`AWS_REGION` / `AWS_DEFAULT_REGION` / `us-east-1`).
    #[must_use]
    pub fn region_display(&self) -> String {
        self.region.clone()
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
    /// All CloudFormation parameters are passed explicitly on every deploy so
    /// `UpdateStack` never resets omitted params with template defaults (e.g. KMS admin).
    ///
    /// # Errors
    ///
    /// Returns an error when S3 bucket creation/upload or CloudFormation
    /// operations fail, or when a configured custom template cannot be read.
    pub async fn deploy(
        &self,
        artifact: &EnclaveArtifact,
        retain: bool,
        debug_mode: bool,
        config: &NitrumConfig,
    ) -> Result<BTreeMap<String, String>> {
        let scaling: &Scaling = &config.scaling;
        let cloud = &config.cloud;
        let eif_label = eif_version_label_from_hash(&artifact.hash);
        let eif_s3_key = eif_s3_key(&eif_label);
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

        let (rolling_min_in_service, rolling_pause) = if cloud.safe_rolling {
            ("1", "PT5M")
        } else {
            ("0", "PT0S")
        };

        let (yaml, is_custom) = load_cloud_template(&self.project_root, cloud.template.as_deref())?;
        warn_template_skew(&yaml, is_custom, "CloudFormation", "nitrum cloud eject");

        let params = vec![
            ("ProjectName".to_string(), config.project.name.to_string()),
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
            ("InstanceType".to_string(), scaling.instance_type.clone()),
            (
                "RollingMinInstancesInService".to_string(),
                rolling_min_in_service.to_string(),
            ),
            ("RollingPauseTime".to_string(), rolling_pause.to_string()),
            (
                "EnableXRayTracing".to_string(),
                if cloud.xray_tracing {
                    "true".to_string()
                } else {
                    "false".to_string()
                },
            ),
            (
                "LogRetentionInDays".to_string(),
                cloud.log_retention_days.to_string(),
            ),
            (
                "SnsAlarmTopicArn".to_string(),
                cloud.sns_alarm_topic_arn.clone().unwrap_or_default(),
            ),
            (
                "ControlPlaneImage".to_string(),
                config.runtime.control_plane.to_string(),
            ),
            (
                "ControlPlaneDebugArg".to_string(),
                control_plane_debug_arg.to_string(),
            ),
            ("EifImageSha384".to_string(), pcr0.to_string()),
            (
                "KmsAdministratorRoleArn".to_string(),
                cloud.kms_administrator_cfn_value().to_string(),
            ),
            (
                "InstanceManagedPolicyArns".to_string(),
                cloud.instance_managed_policy_arns_cfn_value(),
            ),
        ];

        self.bucket.create_if_non_existent().await?;
        self.bucket.upload(&eif_s3_key, &artifact.eif_path).await?;

        let template = self.materialize_template(yaml).await?;
        let need_wait = self
            .cloudformation
            .update_if_needed(&params, &template)
            .await?;
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

    async fn materialize_template(&self, yaml: String) -> Result<StackTemplate> {
        match stack_template_kind(yaml.len()) {
            StackTemplateKind::Body => Ok(StackTemplate::Body(yaml)),
            StackTemplateKind::Url => {
                let account_id = self.caller_account_id().await?;
                self.bucket
                    .ensure_cloudformation_template_read_policy(
                        &account_id,
                        self.cloudformation.stack_name(),
                    )
                    .await?;
                self.bucket
                    .put_bytes(CFN_TEMPLATE_S3_KEY, yaml.into_bytes())
                    .await?;
                Ok(StackTemplate::Url(s3_template_url(
                    &self.region,
                    self.bucket.name(),
                    CFN_TEMPLATE_S3_KEY,
                )))
            }
        }
    }

    async fn caller_account_id(&self) -> Result<String> {
        let identity =
            self.sts.get_caller_identity().send().await.context(
                "sts:GetCallerIdentity (needed for CloudFormation template bucket policy)",
            )?;
        identity
            .account()
            .map(ToOwned::to_owned)
            .context("sts:GetCallerIdentity returned no account id")
    }
}

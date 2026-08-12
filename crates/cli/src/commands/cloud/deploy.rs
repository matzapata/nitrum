//! Deploy with AWS `CloudFormation` (bundled template) and S3 EIF upload.

use anyhow::{Context, Result};
use clap::Args;
use std::path::PathBuf;

use crate::{artifact::EnclaveArtifact, cloud::EnclaveCloudStack, project::CliProject, utils};

#[derive(Args)]
pub struct DeployArgs {
    /// Use this project name instead of `project.name` in nitrum.toml (CloudFormation/S3/SSM/Docker tag)
    #[arg(long = "as", value_name = "NAME")]
    pub as_name: Option<String>,
    /// Project directory (default: current directory)
    #[arg(short, long)]
    pub path: Option<PathBuf>,
    /// Retain KMS, `DynamoDB`, logs, and SSM on stack delete (`Retain=true` in `CloudFormation`)
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub retain: bool,
    /// Path to EIF file to deploy (default: `.nitrum/artifacts/{name}.eif` in project directory)
    #[arg(long, value_name = "PATH")]
    pub eif: Option<PathBuf>,
    /// Pass `--debug-mode` to control-plane runtime
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub debug_mode: bool,
    /// Skip the deployment confirmation prompt
    #[arg(long)]
    pub force: bool,
}

pub async fn run(args: DeployArgs) -> Result<()> {
    let project = CliProject::load(args.path, args.as_name)?;
    project
        .config
        .validate_cloud()
        .context("invalid [cloud] / [scaling] combination for deploy")?;

    let artifact = if let Some(p) = &args.eif {
        let eif_path = if p.is_absolute() {
            p.clone()
        } else {
            project.root.join(p)
        };

        utils::with_spinner(
            "Loading existing enclave artifact…",
            "Loaded existing enclave artifact.",
            EnclaveArtifact::try_from(&eif_path, &project.config),
        )
        .await?
    } else {
        utils::with_spinner(
            "Building enclave artifact from source…",
            "Built enclave artifact from source.",
            EnclaveArtifact::try_from(&project.root, &project.config),
        )
        .await?
    };

    let stack_name = project.config.project.name.clone();
    let cloud_stack = EnclaveCloudStack::new(&project.config).await?;

    warn_if_kms_admin_mismatch(&project.config.cloud).await?;

    let region_display = cloud_stack.region_display();
    let bucket = cloud_stack.bucket_name();
    let retain_str = if args.retain { "true" } else { "false" };
    let cloud = &project.config.cloud;
    let scaling = &project.config.scaling;
    let kms_admin = cloud.kms_administrator_cfn_value();
    let sns = cloud.sns_alarm_topic_arn.as_deref().unwrap_or("(none)");

    if !args.force
        && !utils::confirm(&format!(
            "Deploy CloudFormation stack (ProjectName={stack_name}, Retain={retain_str}, DebugMode={}, \
             region {region_display}, S3 `s3://{bucket}`, instance_type={}, ASG {}-{} (desired {}), \
             enclave {} vCPU / {} MiB, xray={}, safe_rolling={}, log_retention_days={}, \
             kms_admin={kms_admin}, sns_alarms={sns})?",
            args.debug_mode,
            scaling.instance_type,
            scaling.min_replicas,
            scaling.max_replicas,
            scaling.desired_replicas,
            scaling.num_cpus,
            scaling.ram_size_mib,
            cloud.xray_tracing,
            cloud.safe_rolling,
            cloud.log_retention_days,
        ))
    {
        return Ok(());
    }

    let deploy_success = format!("Stack `{stack_name}` deployed.");
    let outputs = utils::with_spinner(
        "Deploying artifact and CloudFormation stack…",
        &deploy_success,
        cloud_stack.deploy(&artifact, args.retain, args.debug_mode, &project.config),
    )
    .await?;

    let out_path = project.root.join("out.json");
    let mut envelope = serde_json::Map::new();
    envelope.insert(
        stack_name.to_string(),
        serde_json::to_value(&outputs).expect("output map serializes to JSON"),
    );
    utils::write_json_value_pretty(&out_path, &serde_json::Value::Object(envelope))?;
    println!("Wrote {}.", out_path.display());

    Ok(())
}

/// Warn when `[cloud].kms_administrator_role_arn` is set and the caller is not that principal.
///
/// Every EIF update rewrites the KMS key policy (`EifImageSha384`), which requires
/// `kms:PutKeyPolicy` as that administrator. Mismatched identity → opaque CFN AccessDenied.
async fn warn_if_kms_admin_mismatch(cloud: &config::Cloud) -> Result<()> {
    let Some(admin_arn) = cloud.kms_administrator_role_arn.as_deref() else {
        return Ok(());
    };
    if admin_arn == "AWS_ACCOUNT_ROOT" {
        return Ok(());
    }

    let aws_cfg = aws_config::load_from_env().await;
    let sts = aws_sdk_sts::Client::new(&aws_cfg);
    let identity = sts
        .get_caller_identity()
        .send()
        .await
        .context("sts:GetCallerIdentity (needed to check KMS administrator match)")?;
    let caller = identity.arn().unwrap_or("");
    if caller_matches_kms_admin(caller, admin_arn) {
        return Ok(());
    }

    eprintln!(
        "warning: current AWS identity `{caller}` does not match \
         `cloud.kms_administrator_role_arn` ({admin_arn}). \
         This deploy may fail with AccessDenied on kms:PutKeyPolicy when updating the PCR0 \
         condition in the KMS key policy. Assume that role (or set the ARN to your deployer) \
         before deploying."
    );
    Ok(())
}

/// True when `caller_arn` is the admin principal or an STS assumed-role session for that IAM role.
fn caller_matches_kms_admin(caller_arn: &str, admin_arn: &str) -> bool {
    if caller_arn == admin_arn {
        return true;
    }
    // arn:aws:sts::ACCOUNT:assumed-role/ROLE_NAME/SESSION → compare to iam role ARN
    let Some(rest) = caller_arn
        .strip_prefix("arn:aws:sts::")
        .or_else(|| caller_arn.strip_prefix("arn:aws-us-gov:sts::"))
        .or_else(|| caller_arn.strip_prefix("arn:aws-cn:sts::"))
    else {
        return false;
    };
    let Some((account, after_account)) = rest.split_once(':') else {
        return false;
    };
    let Some(role_and_session) = after_account.strip_prefix("assumed-role/") else {
        return false;
    };
    let Some((role_name, _)) = role_and_session.split_once('/') else {
        return false;
    };
    let expected = format!("arn:aws:iam::{account}:role/{role_name}");
    let expected_gov = format!("arn:aws-us-gov:iam::{account}:role/{role_name}");
    let expected_cn = format!("arn:aws-cn:iam::{account}:role/{role_name}");
    admin_arn == expected || admin_arn == expected_gov || admin_arn == expected_cn
}

#[cfg(test)]
mod tests {
    use super::caller_matches_kms_admin;

    #[test]
    fn matches_exact_arn() {
        let arn = "arn:aws:iam::123456789012:role/nitrum-kms-admin";
        assert!(caller_matches_kms_admin(arn, arn));
    }

    #[test]
    fn matches_assumed_role_session() {
        assert!(caller_matches_kms_admin(
            "arn:aws:sts::123456789012:assumed-role/nitrum-kms-admin/session",
            "arn:aws:iam::123456789012:role/nitrum-kms-admin",
        ));
    }

    #[test]
    fn rejects_other_role() {
        assert!(!caller_matches_kms_admin(
            "arn:aws:sts::123456789012:assumed-role/ci/session",
            "arn:aws:iam::123456789012:role/nitrum-kms-admin",
        ));
    }
}

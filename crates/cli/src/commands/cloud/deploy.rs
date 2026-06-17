//! Deploy with AWS `CloudFormation` (bundled `samples/hello` template) and S3 EIF upload.

use anyhow::Result;
use clap::Args;
use config::NitrumConfig;
use std::env;
use std::path::PathBuf;

use crate::{artifact::EnclaveArtifact, cloud::EnclaveCloudStack, utils};

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
    /// Optional full IAM principal ARN (role or user) for KMS key administration in the stack key policy.
    /// When omitted, the parameter is not sent and `CloudFormation` uses the template default.
    #[arg(long = "kms-administrator-role-arn", value_name = "ARN")]
    pub kms_administrator_role_arn: Option<String>,
}

pub async fn run(args: DeployArgs) -> Result<()> {
    let root = args
        .path
        .unwrap_or_else(|| env::current_dir().expect("current directory"));
    let config = NitrumConfig::try_from(root.join("nitrum.toml").as_path())?
        .with_name(args.as_name.clone())?;

    let artifact = if let Some(p) = &args.eif {
        let eif_path = if p.is_absolute() {
            p.clone()
        } else {
            root.join(p)
        };

        utils::with_spinner(
            "Loading existing enclave artifact…",
            "Loaded existing enclave artifact.",
            EnclaveArtifact::try_from(&eif_path, &config),
        )
        .await?
    } else {
        utils::with_spinner(
            "Building enclave artifact from source…",
            "Built enclave artifact from source.",
            EnclaveArtifact::try_from(&root, &config),
        )
        .await?
    };

    // Create CloudFormation stack
    let stack_name = config.project.name.clone();
    let cloud_stack = EnclaveCloudStack::new(&config).await?;

    // Confirm deployment
    let region_display = cloud_stack.region_display();
    let bucket = cloud_stack.bucket_name();
    let retain_str = if args.retain { "true" } else { "false" };
    if !args.force
        && !utils::confirm(&format!(
            "Deploy CloudFormation stack (ProjectName={stack_name}, Retain={retain_str}, DebugMode={}, region {region_display}, S3 `s3://{bucket}`, ASG {}-{} (desired {}), scale_policy {}, enclave {} vCPU / {} MiB)?",
            args.debug_mode,
            config.scaling.min_replicas,
            config.scaling.max_replicas,
            config.scaling.desired_replicas,
            config.scaling.scale_policy.as_deploy_param(),
            config.scaling.num_cpus,
            config.scaling.ram_size_mib,
        ))
    {
        return Ok(());
    }

    let kms_administrator_role_arn = args
        .kms_administrator_role_arn
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    // Deploy artifact and CloudFormation stack
    let deploy_success = format!("Stack `{stack_name}` deployed.");
    let outputs = utils::with_spinner(
        "Deploying artifact and CloudFormation stack…",
        &deploy_success,
        cloud_stack.deploy(
            &artifact,
            args.retain,
            args.debug_mode,
            &config,
            kms_administrator_role_arn,
        ),
    )
    .await?;

    // Write outputs to out.json
    let out_path = root.join("out.json");
    let mut envelope = serde_json::Map::new();
    envelope.insert(
        stack_name.clone(),
        serde_json::to_value(&outputs).expect("output map serializes to JSON"),
    );
    utils::write_json_value_pretty(&out_path, &serde_json::Value::Object(envelope))?;
    println!("Wrote {}.", out_path.display());

    Ok(())
}

//! Deploy with AWS `CloudFormation` (bundled `samples/hello` template) and S3 EIF upload.

use anyhow::Result;
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
    /// Optional full IAM principal ARN (role or user) for KMS key administration in the stack key policy.
    /// When omitted, the parameter is not sent and `CloudFormation` uses the template default.
    #[arg(long = "kms-administrator-role-arn", value_name = "ARN")]
    pub kms_administrator_role_arn: Option<String>,
}

pub async fn run(args: DeployArgs) -> Result<()> {
    let project = CliProject::load(args.path, args.as_name)?;

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

    let region_display = cloud_stack.region_display();
    let bucket = cloud_stack.bucket_name();
    let retain_str = if args.retain { "true" } else { "false" };
    if !args.force
        && !utils::confirm(&format!(
            "Deploy CloudFormation stack (ProjectName={stack_name}, Retain={retain_str}, DebugMode={}, region {region_display}, S3 `s3://{bucket}`, ASG {}-{} (desired {}), enclave {} vCPU / {} MiB)?",
            args.debug_mode,
            project.config.scaling.min_replicas,
            project.config.scaling.max_replicas,
            project.config.scaling.desired_replicas,
            project.config.scaling.num_cpus,
            project.config.scaling.ram_size_mib,
        ))
    {
        return Ok(());
    }

    let kms_administrator_role_arn = args
        .kms_administrator_role_arn
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let deploy_success = format!("Stack `{stack_name}` deployed.");
    let outputs = utils::with_spinner(
        "Deploying artifact and CloudFormation stack…",
        &deploy_success,
        cloud_stack.deploy(
            &artifact,
            args.retain,
            args.debug_mode,
            &project.config,
            kms_administrator_role_arn,
        ),
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

//! Deploy with AWS CloudFormation (bundled `samples/hello` template) and S3 EIF upload.

use anyhow::Result;
use clap::Args;
use std::env;
use std::path::PathBuf;
use shared::config::NitrumConfig;

use crate::{artifact::EnclaveArtifact, cloud::EnclaveCloudStack, utils};

#[derive(Args)]
pub struct DeployArgs {
    /// Project directory (default: current directory)
    #[arg(short, long)]
    pub path: Option<PathBuf>,
    /// Retain KMS, DynamoDB, logs, and SSM on stack delete (`Retain=true` in CloudFormation)
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub retain: bool,
    /// Path to EIF file to deploy (default: `enclave.eif` in project directory)
    #[arg(long, value_name = "PATH")]
    pub eif: Option<PathBuf>,
    /// Docker tag for `matzapata/nitrum-control-plane` on instances
    #[arg(long, default_value = "latest")]
    pub control_plane_image_tag: String,
}

pub async fn run(args: DeployArgs) -> Result<()> {
    let root = args
        .path
        .unwrap_or_else(|| env::current_dir().expect("current directory"));
    let config = NitrumConfig::try_from(root.join("nitrum.toml").as_path())?;

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
    let stack_name = config.name.clone();
    let cloud_stack = EnclaveCloudStack::new(stack_name.clone()).await?;

    // Confirm deployment
    let region_display = cloud_stack.region_display();
    let bucket = cloud_stack.bucket_name();
    let retain_str = if args.retain { "true" } else { "false" };
    if !utils::confirm(&format!(
        "Deploy CloudFormation stack `{stack_name}` (EnvironmentName={stack_name}, Retain={retain_str}, region {region_display}, S3 `s3://{bucket}`)?"
    )) {
        return Ok(());
    }

    // Deploy artifact and CloudFormation stack
    let deploy_success = format!("Stack `{stack_name}` deployed.");
    let outputs = utils::with_spinner(
        "Deploying artifact and CloudFormation stack…",
        &deploy_success,
        cloud_stack.deploy(&artifact, args.retain, &args.control_plane_image_tag),
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

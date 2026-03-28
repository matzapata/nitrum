//! Delete the CloudFormation stack and EIF S3 bucket created by `nitrum deploy`.

use anyhow::Result;
use clap::Args;
use std::env;
use std::path::PathBuf;
use shared::config::NitrumConfig;
use crate::{cloud::EnclaveCloudStack, utils};

#[derive(Args)]
pub struct DestroyArgs {
    /// Project directory (default: current directory); used to read `nitrum.toml` for stack name (`name`)
    #[arg(short, long)]
    pub path: Option<PathBuf>,
    /// Skip the confirmation prompt (for scripts)
    #[arg(long)]
    pub force: bool,
    /// AWS region (overrides `AWS_REGION` / `AWS_DEFAULT_REGION`)
    #[arg(long, value_name = "REGION")]
    pub region: Option<String>,
}

pub async fn run(args: DestroyArgs) -> Result<()> {
    // Load config
    let root = args
        .path
        .unwrap_or_else(|| env::current_dir().expect("current directory"));
    let config = NitrumConfig::try_from(root.join("nitrum.toml").as_path())?;
    
    // Create CloudFormation stack
    let stack_name = config.name;
    let cloud_stack = EnclaveCloudStack::new(
        root.clone(),
        stack_name.clone(),
        args.region.clone(),
    )
    .await?;

    // Confirm deletion
    let bucket = cloud_stack.bucket_name().to_string();
    let region_display = cloud_stack.region_display();
    if !args.force
        && !utils::confirm(&format!(
            "Delete CloudFormation stack `{stack_name}` and empty + delete S3 bucket `{bucket}` (region {region_display})?"
        ))
    {
        return Ok(());
    }

    // Delete CloudFormation stack
    let destroy_success = format!("Stack `{stack_name}` deleted.");
    utils::with_spinner(
        "Deleting stack and S3 bucket (this may take a while)…",
        &destroy_success,
        cloud_stack.destroy(),
    )
    .await?;

    Ok(())
}

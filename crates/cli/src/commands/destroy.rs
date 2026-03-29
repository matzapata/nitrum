//! Delete the CloudFormation stack and EIF S3 bucket created by `nitrum deploy`.

use crate::{cloud::EnclaveCloudStack, utils};
use anyhow::Result;
use clap::Args;
use shared::config::NitrumConfig;
use std::env;
use std::path::PathBuf;

#[derive(Args)]
pub struct DestroyArgs {
    /// Use this project `name` instead of `name` in nitrum.toml (CloudFormation/S3/SSM/Docker tag)
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,
    /// Project directory (default: current directory); used to read `nitrum.toml` for stack name (`name`)
    #[arg(short, long)]
    pub path: Option<PathBuf>,
    /// Skip the confirmation prompt (for scripts)
    #[arg(long)]
    pub force: bool,
}

pub async fn run(args: DestroyArgs) -> Result<()> {
    // Load config
    let root = args
        .path
        .unwrap_or_else(|| env::current_dir().expect("current directory"));
    let config = NitrumConfig::try_from(root.join("nitrum.toml").as_path())?
        .with_name(args.name.clone())?;

    // Create CloudFormation stack
    let cloud_stack = EnclaveCloudStack::new(&config).await?;

    // Confirm deletion
    let stack_name = cloud_stack.stack_name();
    let bucket = cloud_stack.bucket_name();
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

//! Delete the `CloudFormation` stack and EIF S3 bucket created by `nitrum cloud deploy`.

use crate::{cloud::EnclaveCloudStack, project::CliProject, utils};
use anyhow::Result;
use clap::Args;
use std::path::PathBuf;

#[derive(Args)]
pub struct DestroyArgs {
    /// Use this project name instead of `project.name` in nitrum.toml (CloudFormation/S3/SSM/Docker tag)
    #[arg(long = "as", value_name = "NAME")]
    pub as_name: Option<String>,
    /// Project directory (default: current directory); used to read `nitrum.toml` for stack name (`project.name`)
    #[arg(short, long)]
    pub path: Option<PathBuf>,
    /// Skip the confirmation prompt (for scripts)
    #[arg(long)]
    pub force: bool,
}

pub async fn run(args: DestroyArgs) -> Result<()> {
    let project = CliProject::load(args.path, args.as_name)?;
    let cloud_stack = EnclaveCloudStack::new(&project.config).await?;

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

    let destroy_success = format!("Stack `{stack_name}` deleted.");
    utils::with_spinner(
        "Deleting stack and S3 bucket (this may take a while)…",
        &destroy_success,
        cloud_stack.destroy(),
    )
    .await?;

    Ok(())
}

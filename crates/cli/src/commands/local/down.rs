use anyhow::Result;
use clap::Args;
use std::path::PathBuf;

use crate::{local::EnclaveLocalStack, project::CliProject, utils};

#[derive(Args)]
pub struct DownArgs {
    /// Use this project name instead of `project.name` in nitrum.toml (CloudFormation/S3/SSM/Docker tag)
    #[arg(long = "as", value_name = "NAME")]
    pub as_name: Option<String>,
    /// Project directory (default: current directory)
    #[arg(short, long)]
    pub path: Option<PathBuf>,
}

pub async fn run(args: DownArgs) -> Result<()> {
    let project = CliProject::load(args.path, args.as_name)?;
    let local_stack = EnclaveLocalStack::new(&project.root, &project.config);
    utils::with_spinner(
        "Stopping local stack…",
        "Local stack stopped.",
        local_stack.down(),
    )
    .await?;

    Ok(())
}

use clap::Args;
use std::path::PathBuf;

use anyhow::Result;

use crate::{local::EnclaveLocalStack, project::CliProject};

#[derive(Args)]
pub struct LogsArgs {
    /// Use this project name instead of `project.name` in nitrum.toml (CloudFormation/S3/SSM/Docker tag)
    #[arg(long = "as", value_name = "NAME")]
    pub as_name: Option<String>,
    /// Project directory (default: current directory)
    #[arg(short, long)]
    pub path: Option<PathBuf>,
    /// Number of log lines to show from the end of each service (docker compose `--tail`)
    #[arg(long)]
    pub tail: Option<u32>,
    /// Follow log output
    #[arg(short, long)]
    pub follow: bool,
}

pub async fn run(args: LogsArgs) -> Result<()> {
    let project = CliProject::load(args.path, args.as_name)?;
    let local_stack = EnclaveLocalStack::new(&project.root, &project.config);
    local_stack.logs(args.tail, args.follow).await
}

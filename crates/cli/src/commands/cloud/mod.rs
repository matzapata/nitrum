mod deploy;
mod destroy;
mod env;
mod logs;

use anyhow::Result;
use clap::{Args, Subcommand};

#[derive(Args)]
pub struct CloudArgs {
    #[command(subcommand)]
    pub command: CloudCommand,
}

#[derive(Subcommand)]
pub enum CloudCommand {
    /// Deploy to AWS (`CloudFormation` + S3 EIF upload)
    Deploy(deploy::DeployArgs),
    /// Delete the `CloudFormation` stack and EIF S3 bucket
    Destroy(destroy::DestroyArgs),
    /// Application environment variables in SSM Parameter Store
    Env(env::EnvArgs),
    /// Read deployed control-plane `CloudWatch` logs
    Logs(logs::LogsArgs),
}

pub async fn run(args: CloudArgs) -> Result<()> {
    match args.command {
        CloudCommand::Deploy(a) => deploy::run(a).await,
        CloudCommand::Destroy(a) => destroy::run(a).await,
        CloudCommand::Env(a) => env::run(a).await,
        CloudCommand::Logs(a) => logs::run(a).await,
    }
}

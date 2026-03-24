//! Delete the CloudFormation stack created by `nitrum deploy`.

use clap::Args;
use std::env;
use std::path::PathBuf;
use tracing::info;

use crate::utils::{aws, console};

#[derive(Args)]
pub struct DestroyArgs {
    /// Project directory (default: current directory); used to read `nitrum.toml` for stack name (`name`)
    #[arg(short, long)]
    pub path: Option<PathBuf>,
    /// Skip the confirmation prompt (for scripts)
    #[arg(long)]
    pub force: bool,
}

pub async fn run(args: DestroyArgs) {
    info!("nitrum destroy: starting");
    let root = args
        .path
        .unwrap_or_else(|| env::current_dir().expect("current directory"));
    info!(project_root = %root.display(), "project directory");
    let cfg_path = root.join("nitrum.toml");
    let config = match shared::config::try_load(&cfg_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let stack_name = config.name;
    info!(%stack_name, "stack name from nitrum.toml");

    if !args.force && !console::confirm(&format!("Delete CloudFormation stack `{stack_name}`?")) {
        return;
    }

    let sdk = aws::sdk_config(None).await;
    if let Err(e) = aws::cloudformation_delete_stack_wait(&sdk, &stack_name).await {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
    info!(%stack_name, "nitrum destroy: finished");

    // TODO: delete s3 bucket
}

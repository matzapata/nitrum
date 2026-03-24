//! Delete the CloudFormation stack and EIF S3 bucket created by `nitrum deploy`.

use clap::Args;
use indicatif::ProgressBar;
use std::env;
use std::path::PathBuf;
use tracing::info;

use crate::utils::{aws, console, project};

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
    let bucket = match project::derived_eif_bucket_name(&config.name) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("{e:#}");
            std::process::exit(1);
        }
    };
    let stack_name = config.name;
    info!(%stack_name, %bucket, "stack and bucket from nitrum.toml");

    if !args.force
        && !console::confirm(&format!(
            "Delete CloudFormation stack `{stack_name}` and empty + delete S3 bucket `{bucket}`?"
        ))
    {
        return;
    }

    let sdk = aws::sdk_config(None).await;

    let spinner = console::style_spinner(
        ProgressBar::new_spinner(),
        "Deleting CloudFormation stack (this may take several minutes)…",
    );
    match aws::cloudformation_delete_stack_wait(&sdk, &stack_name).await {
        Ok(()) => spinner.finish_with_message(format!("Stack `{stack_name}` deleted.")),
        Err(e) => {
            spinner.finish_and_clear();
            eprintln!("{e:#}");
            std::process::exit(1);
        }
    }

    let spinner = console::style_spinner(ProgressBar::new_spinner(), "Deleting S3 EIF bucket…");
    match aws::s3_empty_and_delete_bucket(&sdk, &bucket).await {
        Ok(()) => spinner.finish_with_message(format!("Bucket `{bucket}` removed.")),
        Err(e) => {
            spinner.finish_and_clear();
            eprintln!("{e:#}");
            std::process::exit(1);
        }
    }

    info!(%stack_name, %bucket, "nitrum destroy: finished");
}

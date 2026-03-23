//! Run `cdk destroy` in `.nitrum/infra`.

use clap::Args;
use std::env;
use std::path::PathBuf;

use crate::constants;
use crate::utils::{cdk, console};

#[derive(Args)]
pub struct DestroyArgs {
    /// Project directory (default: current directory)
    #[arg(short, long)]
    pub path: Option<PathBuf>,
    /// Deployment environment for infra (`dev` or `prod`)
    #[arg(long, value_name = "ENV", default_value = "dev", value_parser = ["dev", "prod"])]
    pub env: String,
    /// Pass `--force` to `cdk destroy` (skip CDK confirmation)
    #[arg(long)]
    pub force: bool,
}

pub async fn run(args: DestroyArgs) {
    let root = args
        .path
        .unwrap_or_else(|| env::current_dir().expect("current directory"));
    let infra_dir = root.join(constants::NITRUM_INFRA_DIR);

    if !infra_dir.is_dir() {
        eprintln!(
            "{} does not exist. Run `nitrum deploy` first.",
            infra_dir.display()
        );
        std::process::exit(1);
    }

    if !console::confirm(&format!("Destroy AWS resources ({} deployment)?", args.env,)) {
        return;
    }

    // `infra.ts` requires EIF_PATH for stack synthesis; destroy does not use the file,
    // so pass a stable file that always exists in the infra directory.
    let env_vars = [
        ("DEPLOYMENT", args.env.clone()),
        (
            "EIF_PATH",
            infra_dir.join("README.md").display().to_string(),
        ),
    ];
    let mut destroy_args = vec!["NitrumStack".to_string(), "--force".to_string()];
    if let Err(e) = cdk::run_cdk(&infra_dir, "destroy", &destroy_args, &env_vars).await {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}

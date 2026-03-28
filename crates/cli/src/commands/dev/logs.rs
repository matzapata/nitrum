use clap::Args;
use std::env;

use anyhow::Result;
use shared::config::NitrumConfig;

use crate::local::EnclaveLocalStack;

#[derive(Args)]
pub struct LogsArgs {
    /// Project directory (default: current directory)
    #[arg(short, long)]
    pub root: Option<std::path::PathBuf>,
    /// Number of log lines to show from the end of each service (docker compose `--tail`)
    #[arg(long)]
    pub tail: Option<u32>,
    /// Follow log output
    #[arg(short, long)]
    pub follow: bool,
}

pub async fn run(args: LogsArgs) -> Result<()> {
    // Load config
    let root = args
        .root
        .clone()
        .unwrap_or_else(|| env::current_dir().expect("current directory"));
    let cfg = NitrumConfig::try_from(root.join("nitrum.toml").as_path())?;

    // Tail logs
    let local_stack = EnclaveLocalStack::new(&root, &cfg.name);
    local_stack.logs(args.tail, args.follow).await
}

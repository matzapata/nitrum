use anyhow::Result;
use clap::Args;
use shared::config::NitrumConfig;
use std::env;

use crate::{local::EnclaveLocalStack, utils};

#[derive(Args)]
pub struct DownArgs {
    /// Project directory (default: current directory)
    #[arg(short, long)]
    pub root: Option<std::path::PathBuf>,
}

pub async fn run(args: DownArgs) -> Result<()> {
    // Load config
    let root = args
        .root
        .unwrap_or_else(|| env::current_dir().expect("current directory"));
    let cfg = NitrumConfig::try_from(root.join("nitrum.toml").as_path())?;

    // Stop local stack
    let local_stack = EnclaveLocalStack::new(&root, &cfg);
    utils::with_spinner(
        "Stopping local stack…",
        "Local stack stopped.",
        local_stack.down(),
    )
    .await?;

    Ok(())
}

use anyhow::Result;
use clap::Args;
use shared::config::NitrumConfig;
use std::env;

use crate::{local::EnclaveLocalStack, utils};

#[derive(Args)]
pub struct UpArgs {
    /// Project directory (default: current directory)
    #[arg(short, long)]
    pub root: Option<std::path::PathBuf>,
}

pub async fn run(args: UpArgs) -> Result<()> {
    // Load config
    let root = args
        .root
        .unwrap_or_else(|| env::current_dir().expect("current directory"));
    let cfg = NitrumConfig::try_from(root.join("nitrum.toml").as_path())?;

    // Start local stack
    let local_stack = EnclaveLocalStack::new(&root, &cfg.name);
    utils::with_spinner(
        "Starting local stack…",
        "Local stack started.",
        local_stack.up(),
    )
    .await?;

    println!();
    println!("Stack is running.");
    println!();
    println!("  https://nitrum.local/     — main ingress (HTTPS on host port 443)");
    println!(
        "  https://127.0.0.1:443/    — same endpoint; use curl -k if the cert name mismatches"
    );
    println!();
    println!("  If nitrum.local does not resolve, add to /etc/hosts: 127.0.0.1 nitrum.local");
    println!();

    Ok(())
}

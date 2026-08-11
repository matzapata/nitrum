use anyhow::Result;
use clap::Args;
use std::path::PathBuf;

use crate::{local::EnclaveLocalStack, project::CliProject, utils};

#[derive(Args)]
pub struct UpArgs {
    /// Use this project name instead of `project.name` in nitrum.toml (CloudFormation/S3/SSM/Docker tag)
    #[arg(long = "as", value_name = "NAME")]
    pub as_name: Option<String>,
    /// Project directory (default: current directory)
    #[arg(short, long)]
    pub path: Option<PathBuf>,
}

pub async fn run(args: UpArgs) -> Result<()> {
    let project = CliProject::load(args.path, args.as_name)?;
    let local_stack = EnclaveLocalStack::new(&project.root, &project.config)?;
    let (cpus, memory) = local_stack.resource_limits();
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
    println!("  Enclave limits: {cpus} CPUs, {memory} (from nitrum.toml [scaling])");
    println!();
    println!("  If nitrum.local does not resolve, add to /etc/hosts: 127.0.0.1 nitrum.local");
    println!();

    Ok(())
}

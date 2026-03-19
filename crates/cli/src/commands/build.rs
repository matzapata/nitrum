//! Build enclave eif file

use clap::Args;

#[derive(Args)]
pub struct BuildArgs {
    /// Path to project directory (default: current directory)
    #[arg(short, long)]
    pub path: Option<std::path::PathBuf>,
}

pub async fn run(_args: BuildArgs) {
    // TODO: build enclave using dev as base image (version for dev)
}

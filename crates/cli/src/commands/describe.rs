//! Describe an EIF via `nitro-cli describe-eif` inside the Nitro CLI Docker image.

use clap::Args;
use std::env;
use std::path::PathBuf;

use crate::utils::docker;

#[derive(Args)]
pub struct DescribeArgs {
    /// Path to the enclave image file (default: enclave.eif in the current directory)
    #[arg(value_name = "EIF")]
    pub eif: Option<PathBuf>,
}

pub async fn run(args: DescribeArgs) {
    let cwd = env::current_dir().expect("current directory");

    let eif_path = match &args.eif {
        Some(p) if p.is_absolute() => p.clone(),
        Some(p) => cwd.join(p),
        None => cwd.join("enclave.eif"),
    };

    if let Err(e) = docker::describe_eif(&eif_path).await {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}

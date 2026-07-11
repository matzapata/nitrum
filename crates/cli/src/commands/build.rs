//! Build enclave Docker image (prod data-plane base) and produce `.nitrum/artifacts/{name}.eif` via nitro-cli in Docker.

use anyhow::Result;
use clap::Args;
use std::path::PathBuf;

use crate::{artifact::EnclaveArtifact, project::CliProject, utils};

#[derive(Args)]
pub struct BuildArgs {
    /// Use this project name instead of `project.name` in nitrum.toml (CloudFormation/S3/SSM/Docker tag)
    #[arg(long = "as", value_name = "NAME")]
    pub as_name: Option<String>,
    /// Path to project directory (default: current directory)
    #[arg(short, long)]
    pub path: Option<PathBuf>,
}

pub async fn run(args: BuildArgs) -> Result<()> {
    let project = CliProject::load(args.path, args.as_name)?;

    let artifact = utils::with_spinner(
        "Building enclave artifact from source…",
        "EIF built and measured.",
        EnclaveArtifact::try_from(&project.root, &project.config),
    )
    .await?;

    println!();
    println!("EIF written to {}", artifact.eif_path.display());
    println!("EIF hash (sha256): {}", artifact.hash);
    println!("PCR0: {}", artifact.pcr0);
    println!("PCR1: {}", artifact.pcr1);
    println!("PCR2: {}", artifact.pcr2);

    Ok(())
}

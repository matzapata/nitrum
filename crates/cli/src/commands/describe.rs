//! Describe an EIF via `nitro-cli describe-eif` inside the Nitro CLI Docker image.

use anyhow::Result;
use clap::Args;
use std::path::PathBuf;

use crate::artifact::{EnclaveArtifact, project_eif_path};
use crate::project::CliProject;

#[derive(Args)]
pub struct DescribeArgs {
    /// Use this project name instead of `project.name` in nitrum.toml (CloudFormation/S3/SSM/Docker tag)
    #[arg(long = "as", value_name = "NAME")]
    pub as_name: Option<String>,
    /// Path to the enclave image file (default: `.nitrum/artifacts/{name}.eif` in the current directory)
    #[arg(value_name = "EIF")]
    pub eif: Option<PathBuf>,
}

pub async fn run(args: DescribeArgs) -> Result<()> {
    let project = CliProject::load(None, args.as_name)?;

    let eif_path = match &args.eif {
        Some(p) if p.is_absolute() => p.clone(),
        Some(p) => project.root.join(p),
        None => project_eif_path(&project.root, &project.config.project.name),
    };

    let artifact = EnclaveArtifact::try_from(&eif_path, &project.config).await?;

    println!("EIF: {}", artifact.eif_path.display());
    println!("Hash (sha256): {}", artifact.hash);
    println!("PCR0: {}", artifact.pcr0);
    println!("PCR1: {}", artifact.pcr1);
    println!("PCR2: {}", artifact.pcr2);

    Ok(())
}

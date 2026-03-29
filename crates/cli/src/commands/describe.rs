//! Describe an EIF via `nitro-cli describe-eif` inside the Nitro CLI Docker image.

use anyhow::Result;
use clap::Args;
use shared::config::NitrumConfig;
use std::env;
use std::path::PathBuf;

use crate::artifact::EnclaveArtifact;

#[derive(Args)]
pub struct DescribeArgs {
    /// Path to the enclave image file (default: enclave.eif in the current directory)
    #[arg(value_name = "EIF")]
    pub eif: Option<PathBuf>,
}

pub async fn run(args: DescribeArgs) -> Result<()> {
    let cwd = env::current_dir().expect("current directory");
    let cfg = NitrumConfig::try_from(cwd.join("nitrum.toml").as_path())?;

    let eif_path = match &args.eif {
        Some(p) if p.is_absolute() => p.clone(),
        Some(p) => cwd.join(p),
        None => cwd.join("enclave.eif"),
    };

    // Load up artifact
    let artifact = EnclaveArtifact::try_from(&eif_path, &cfg).await?;

    println!("EIF: {}", artifact.eif_path.display());
    println!("Hash (sha256): {}", artifact.hash);
    println!("PCR0: {}", artifact.pcr0);
    println!("PCR1: {}", artifact.pcr1);
    println!("PCR2: {}", artifact.pcr2);

    Ok(())
}

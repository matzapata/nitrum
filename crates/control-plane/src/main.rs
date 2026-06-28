mod artifact;
mod constants;
mod enclave;
mod monitoring;
mod networking;
mod utils;

use artifact::EnclaveArtifact;
use clap::Parser;
use enclave::Enclave;
use monitoring::Monitoring;
use networking::Networking;
use std::path::PathBuf;
use tracing::info;
use utils::bucket::Bucket;

#[derive(clap::Parser)]
#[command(name = "control-plane")]
struct Args {
    /// Path to the EIF file (`nitro-cli --eif-path`). Conflicts with `--eif-bucket` / `--eif-hash`.
    #[arg(long, value_name = "PATH", conflicts_with_all = ["eif_bucket", "eif_hash"])]
    eif: Option<PathBuf>,
    /// S3 bucket containing the EIF (object key is `{eif-hash}.eif`, same as `nitrum cloud deploy`).
    #[arg(
        long,
        value_name = "NAME",
        requires = "eif_hash",
        conflicts_with = "eif"
    )]
    eif_bucket: Option<String>,
    /// Version label / hash prefix for the EIF (first 12 hex chars of EIF sha256 from deploy; S3 key `{hash}.eif`).
    #[arg(
        long,
        value_name = "HASH",
        requires = "eif_bucket",
        conflicts_with = "eif"
    )]
    eif_hash: Option<String>,
    /// Enable debug mode.
    #[arg(long, default_value_t = false)]
    debug_mode: bool,
    /// vCPUs passed to `nitro-cli run-enclave --cpu-count` (must match nitro allocator).
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u32).range(1..))]
    cpu_count: u32,
    /// Memory (MiB) passed to `nitro-cli run-enclave --memory` (must match nitro allocator).
    #[arg(long, default_value_t = 4320, value_parser = clap::value_parser!(u32).range(1..))]
    memory_mib: u32,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let monitoring = Monitoring::init();

    let sdk = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let artifact = if let Some(path) = args.eif {
        match EnclaveArtifact::try_from_local(path) {
            Ok(a) => a,
            Err(e) => {
                tracing::error!(error = %e, "invalid --eif path");
                std::process::exit(1);
            }
        }
    } else if let (Some(bucket_name), Some(hash)) =
        (args.eif_bucket.as_ref(), args.eif_hash.as_ref())
    {
        let bucket = Bucket::new(&sdk, bucket_name.as_str());
        match EnclaveArtifact::try_from_bucket(bucket, hash.as_str()).await {
            Ok(a) => a,
            Err(e) => {
                tracing::error!(error = %e, "failed to download EIF from S3");
                std::process::exit(1);
            }
        }
    } else {
        tracing::error!("provide --eif PATH or both --eif-bucket and --eif-hash");
        std::process::exit(1);
    };

    info!("starting networking");
    let mut networking = Networking::new();
    if let Err(e) = networking.run().await {
        tracing::error!(error = %e, "failed to start networking");
        std::process::exit(1);
    }

    info!("starting enclave");
    let mut enclave = Enclave::new(artifact, args.debug_mode, args.cpu_count, args.memory_mib);
    enclave.run();
    info!("enclave started");

    info!("waiting for shutdown signal");
    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for ctrl_c");
    info!("received SIGINT, shutting down");

    monitoring.shutdown().await;
}

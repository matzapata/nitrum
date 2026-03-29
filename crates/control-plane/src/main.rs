mod enclave;
mod networking;

use clap::Parser;
use enclave::Enclave;
use networking::Networking;
use tracing::info;

#[derive(clap::Parser)]
#[command(name = "control-plane")]
struct Args {
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

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let _networking = Networking::run().await.unwrap_or_else(|e| {
        tracing::error!(error = %e, "failed to start networking");
        std::process::exit(1);
    });
    info!("networking up, waiting for shutdown signal");

    // TODO: tokio spawn, also restart on crash
    let _enclave = Enclave::run(args.debug_mode, args.cpu_count, args.memory_mib)
        .await
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "failed to start enclave");
        });

    info!("enclave started, waiting for shutdown signal");
    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for ctrl_c");
    info!("received SIGINT, shutting down");
}

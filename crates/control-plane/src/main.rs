mod constants;
mod enclave;
mod monitoring;
mod networking;

use clap::Parser;
use enclave::Enclave;
use monitoring::Monitoring;
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

    let monitoring = Monitoring::init().await;

    info!("starting networking");
    let mut networking = Networking::new();
    if let Err(e) = networking.run().await {
        tracing::error!(error = %e, "failed to start networking");
        std::process::exit(1);
    }

    info!("starting enclave");
    let mut enclave = Enclave::new(args.debug_mode, args.cpu_count, args.memory_mib);
    enclave.run();
    info!("enclave started");

    info!("waiting for shutdown signal");
    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for ctrl_c");
    info!("received SIGINT, shutting down");

    monitoring.shutdown().await;
}

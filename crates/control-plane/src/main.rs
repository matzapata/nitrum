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
    #[arg(long, default_value = "false")]
    debug_mode: bool,
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
    });
    info!("networking up, waiting for shutdown signal");

    let _enclave = Enclave::run(args.debug_mode).await.unwrap_or_else(|e| {
        tracing::error!(error = %e, "failed to start enclave");
    });
    info!("enclave started, waiting for shutdown signal");

    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for ctrl_c");
    info!("received SIGINT, shutting down");
}

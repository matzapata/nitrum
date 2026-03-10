mod enclave;
mod networking;

use std::path::PathBuf;
use clap::Parser;
use tracing::info;
use shared::config;
use networking::Networking;
use enclave::Enclave;

#[derive(clap::Parser)]
#[command(name = "control-plane")]
struct Args {
    /// Path to nitrum.toml.
    #[arg(long, default_value = "./nitrum.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let cfg = config::load(&args.config);

    Networking::start();
    Enclave::start();
}

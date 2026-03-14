use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tracing::{error, info};

mod config;
mod constants;
mod crypto;
mod networking;
mod server;
mod state;
mod storage;
mod utils;

#[derive(Parser)]
#[command(name = "data-plane")]
struct Args {
    /// Path to nitrum.toml.
    #[arg(long, default_value = "nitrum.toml")]
    config: PathBuf,

    /// Optional command to run after networking and API are up (e.g. `node /app/src/main.js`). If omitted, the process runs until SIGINT.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    command: Vec<String>,
}

#[tokio::main]
async fn main() {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install default rustls crypto provider");

    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Parse args and load runtime config
    let args = Args::parse();
    let runtime_config = config::RuntimeConfig::load(&args.config).await.unwrap_or_else(|e| {
        error!(error = %e, "failed to load runtime config (set NITRUM_DYNAMODB_TABLE and NITRUM_KMS_KEY_ID, or use load_dev for local)");
        std::process::exit(1);
    });

    info!("runtime config loaded");

    // Kick off networking first, most services depend on it
    networking::run().await;

    // Create storage client
    let storage = Arc::new(storage::StorageClient::new(&runtime_config).await);

    // Create crypto client (instantiates its own crypto leader lock internally)
    let crypto = Arc::new(
        crypto::CryptoClient::new(runtime_config.clone(), storage.clone())
            .await
            .unwrap_or_else(|e| {
                error!(error = %e, "crypto setup failed");
                std::process::exit(1);
            }),
    );

    // Create state
    let state = Arc::new(state::DataPlaneState::new(
        runtime_config.clone(),
        storage,
        crypto.clone(),
    ));

    // Create crypto API
    let crypto_state = state.clone();
    tokio::spawn(async move {
        info!("API task starting");
        crypto::api::run(crypto_state).await;
        tracing::warn!("API task exited");
    });

    let ingress_state = state.clone();
    tokio::spawn(async move {
        info!("ingress task starting");
        server::ingress::run(ingress_state).await;
        tracing::warn!("ingress task exited");
    });

    // Run user process if provided, otherwise run until SIGINT
    let exit_code = if args.command.is_empty() {
        info!("no command provided, running until SIGINT");
        let _ = tokio::signal::ctrl_c().await;
        info!("received SIGINT, shutting down");
        0
    } else {
        tokio::select! {
            result = server::runner::run(&args.command) => {
                result.unwrap_or_else(|e| {
                    tracing::error!(error = %e, "failed to run user process");
                    1
                })
            }
            _ = tokio::signal::ctrl_c() => {
                info!("received SIGINT, shutting down");
                0
            }
        }
    };

    std::process::exit(exit_code);
}

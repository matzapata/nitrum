mod config;
mod constants;
mod crypto;
mod server;
mod state;
mod storage;
mod utils;

#[cfg(feature = "enclave")]
mod networking;

use crate::config::RuntimeConfig;
use crate::crypto::CryptoClient;
use crate::state::DataPlaneState;
use crate::storage::StorageClient;
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info};

#[derive(Parser)]
#[command(name = "data-plane")]
struct Args {
    /// Path to the config file. Default: nitrum.toml.
    #[arg(long, default_value = "nitrum.toml")]
    config: PathBuf,

    /// Optional command to run the user process (e.g. `node /app/src/main.js`). If omitted, the process runs until SIGINT.
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

    let args = Args::parse();

    // gvproxy TAP path must be up before IMDS / SSM / HTTPS egress; IMDS uses `169.254.169.254`
    // when the parent runs gvproxy with `-ec2-metadata-access`.
    #[cfg(feature = "enclave")]
    networking::init().await;
    #[cfg(not(feature = "enclave"))]
    info!("enclave networking (TAP + VSOCK) requires feature `enclave`; skipping");

    let runtime_config = RuntimeConfig::load(&args.config).await.unwrap_or_else(|e| {
        error!(
            error = %format!("{:#}", e),
            config_path = %args.config.display(),
            "failed to load runtime config"
        );
        std::process::exit(1);
    });

    // Create storage client
    let storage = Arc::new(StorageClient::new(&runtime_config).await);

    // Create crypto client
    let crypto = Arc::new(
        CryptoClient::new(runtime_config.clone(), storage.clone())
            .await
            .unwrap_or_else(|e| {
                error!(
                    error = %format!("{:#}", e),
                    "crypto setup failed"
                );
                std::process::exit(1);
            }),
    );

    // Create shared data plane state
    let state = Arc::new(DataPlaneState::new(
        runtime_config.clone(),
        storage,
        crypto.clone(),
    ));

    // Kick off crypto api for internal usage
    let crypto_state = state.clone();
    tokio::spawn(async move {
        info!("API task starting");
        if let Err(e) = crypto::api::run(crypto_state).await {
            error!(error = %e, "API task failed");
        }
        tracing::warn!("API task exited");
    });

    // Kick off ingress for external usage
    let ingress_state = state.clone();
    tokio::spawn(async move {
        info!("ingress task starting");
        if let Err(e) = server::ingress::run(ingress_state).await {
            error!(error = %e, "ingress task failed");
        }
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

use std::path::Path;
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
    #[arg(long)]
    config: Option<std::path::PathBuf>,

    /// Command to run after networking and API are up (e.g. `node /app/src/main.js`).
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

    if args.command.is_empty() {
        error!("no user command provided");
        std::process::exit(1);
    }

    let config_path = args
        .config
        .as_deref()
        .unwrap_or_else(|| Path::new("nitrum.toml"));
    let nitrum = shared::config::load(config_path);

    let app_config = config::AppConfig::load(nitrum).await.unwrap_or_else(|e| {
        error!(error = %e, "failed to load app config (set NITRUM_DYNAMODB_TABLE and NITRUM_KMS_KEY_ID, or use load_dev for local)");
        std::process::exit(1);
    });

    let infra = Arc::new(storage::InfraClients::from_config(app_config.clone()).await);
    let state = Arc::new(state::DataPlaneState::new(app_config, infra.clone()));

    networking::run().await;

    let crypto_api = crypto::CryptoApi::setup(infra.as_ref()).await.unwrap_or_else(|e| {
        error!(error = %e, "crypto setup failed");
        std::process::exit(1);
    });

    let acceptor = server::tls::acceptor(infra.as_ref(), &state.config.nitrum.tls_termination.domain)
        .await
        .unwrap_or_else(|e| {
            error!(error = %e, "TLS acceptor failed");
            std::process::exit(1);
        });

    let api_state = state.clone();
    tokio::spawn(async move {
        info!("API task starting");
        crypto_api.run(api_state).await;
        tracing::warn!("API task exited");
    });

    let service_port = state.config.nitrum.service.port;
    tokio::spawn(async move {
        info!(app_port = service_port, "ingress task starting");
        server::ingress::run(service_port, acceptor).await;
        tracing::warn!("ingress task exited");
    });

    let exit_code = tokio::select! {
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
    };

    std::process::exit(exit_code);
}

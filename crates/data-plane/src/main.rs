use clap::Parser;
use tracing::info;

mod api;
mod app;
mod attestation;
mod constants;
mod crypto;
mod infra;
mod ingress;
mod networking;
mod tls;

use shared::config;

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
        tracing::error!("no user command provided");
        std::process::exit(1);
    }

    let path = args
        .config
        .as_deref()
        .unwrap_or(std::path::Path::new("nitrum.toml"));
    let cfg = config::load(path);

    let crypto = std::sync::Arc::new(crypto::setup().await.unwrap_or_else(|e| {
        tracing::error!(error = %e, "crypto setup failed");
        std::process::exit(1);
    }));

    tokio::spawn(async {
        info!("networking task starting");
        networking::run().await;
        tracing::warn!("networking task exited");
    });

    let api_crypto = crypto.clone();
    tokio::spawn(async move {
        info!("API task starting");
        api::run(api_crypto).await;
        tracing::warn!("API task exited");
    });

    let acceptor = tls::self_signed(vec![cfg.tls_termination.domain]);
    tokio::spawn(async move {
        info!(app_port = cfg.service.port, "ingress task starting");
        ingress::run(cfg.service.port, acceptor).await;
        tracing::warn!("ingress task exited");
    });

    let exit_code = tokio::select! {
        result = app::run(&args.command) => {
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

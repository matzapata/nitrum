use clap::Parser;
use tracing::info;

mod api;
mod app;
mod attestation;
mod constants;
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
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    if let Some(ref path) = args.config {
        let _cfg = config::load(path);
        info!(path = %path.display(), "nitrum config loaded");
    }

    networking::setup();

    let api_addr = std::env::var("NITRUM_API_ADDR")
        .unwrap_or_else(|_| constants::API_LISTEN_ADDR.to_string());
    tokio::spawn(api::run(api_addr));

    let app_port = std::env::var("APP_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(constants::APP_PORT);
    let ingress_listen = std::env::var("INGRESS_LISTEN_ADDR")
        .unwrap_or_else(|_| constants::INGRESS_LISTEN_ADDR.to_string());
    let app_addr = format!("127.0.0.1:{app_port}");
    let acceptor = tls::self_signed(vec!["localhost".to_string()]);
    info!("generated self-signed TLS certificate");
    tokio::spawn(ingress::run(ingress_listen, app_addr, acceptor));

    if args.command.is_empty() {
        info!("no user command; data-plane running (API only). Press Ctrl+C to exit.");
        tokio::signal::ctrl_c().await.expect("failed to listen for ctrl_c");
    } else {
        let code = app::run(&args.command)
            .await
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "failed to run user process");
                1
            });
        std::process::exit(code);
    }
}

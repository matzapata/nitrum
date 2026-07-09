use anyhow::Context;
use clap::Parser;
use config::NitrumConfig;
use data_plane::{CryptoClient, DataPlaneConfig, DataPlaneState, StorageClient, crypto, server};
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

// TODO: remove this
fn resolve_start_command(cli_start_command: Vec<String>, config: &DataPlaneConfig) -> Vec<String> {
    if cli_start_command.is_empty() {
        config.project.start_command.clone()
    } else {
        cli_start_command
    }
}

#[tokio::main]
async fn main() {
    std::process::exit(match run().await {
        Ok(code) => code,
        Err(error) => {
            error!(error = %format!("{:#}", error));
            1
        }
    });
}

async fn run() -> anyhow::Result<i32> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install default rustls crypto provider");

    let Args {
        config,
        command: cli_start_command,
    } = Args::parse();

    // Initialize networking (TAP + VSOCK) TODO: improve comment and error handling
    #[cfg(feature = "enclave")]
    data_plane::networking::init().await;

    // Load config
    let nitrum = NitrumConfig::try_from(config.as_path())
        .with_context(|| format!("load config from {}", config.display()))?;
    let data_plane_config = DataPlaneConfig::try_from(nitrum)
        .await
        .context("resolve data-plane infra config")?;

    // Initialize telemetry
    let _telemetry_guard = telemetry::init(
        telemetry::TelemetryConfig::new("data-plane")
            .with_otlp_endpoint(data_plane_config.otlp_endpoint.as_deref()),
    );

    // Initialize egress
    let _egress_guard = data_plane::egress::init(&data_plane_config)
        .await
        .context("egress init failed")?;

    // Initialize storage client
    let storage_client = Arc::new(StorageClient::new(&data_plane_config));

    // Initialize crypto client
    let crypto_client = Arc::new(
        CryptoClient::new(data_plane_config.clone(), storage_client.clone())
            .await
            .context("crypto setup failed")?,
    );

    // Initialize state
    let state = Arc::new(DataPlaneState::new(
        data_plane_config.clone(),
        storage_client,
        crypto_client,
    ));

    crypto::init(state.clone());
    server::ingress::init(state);

    let effective_start_command = resolve_start_command(cli_start_command, &data_plane_config);
    let user_env = &data_plane_config.user_env;
    let exit_code = tokio::select! {
        code = async {
            if effective_start_command.is_empty() {
                info!("no command provided, running until SIGINT");
                std::future::pending::<i32>().await
            } else {
                server::runner::run(&effective_start_command, user_env)
                    .await
                    .unwrap_or_else(|e| {
                        error!(error = %e, "failed to run user process");
                        1
                    })
            }
        } => code,
        _ = tokio::signal::ctrl_c() => {
            info!("received SIGINT, shutting down");
            0
        }
    };

    Ok(exit_code)
}

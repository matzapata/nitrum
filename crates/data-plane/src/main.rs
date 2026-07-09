use anyhow::Context;
use clap::Parser;
use config::NitrumConfig;
use data_plane::{CryptoClient, DataPlaneConfig, StorageClient};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::error;

#[derive(Parser)]
#[command(name = "data-plane")]
struct Args {
    /// Path to the config file. Default: nitrum.toml.
    #[arg(long, default_value = "nitrum.toml")]
    config: PathBuf,
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

    let Args { config } = Args::parse();

    // Initialize networking
    #[cfg(feature = "enclave")]
    data_plane::networking::init()
        .await
        .context("enclave networking init failed")?;

    // Load config
    let nitrum = NitrumConfig::try_from(config.as_path())
        .with_context(|| format!("load config from {}", config.display()))?;
    let data_plane_config = DataPlaneConfig::try_from(nitrum)
        .await
        .context("resolve data-plane infra config")?;

    // Initialize egress
    let _egress_guard = data_plane::egress::init(&data_plane_config)
        .await
        .context("egress init failed")?;

    // Initialize telemetry
    let _telemetry_guard = telemetry::init(
        telemetry::TelemetryConfig::new("data-plane")
            .with_otlp_endpoint(data_plane_config.otlp_endpoint.as_deref()),
    );

    // Initialize storage and crypto clients
    let storage_client = Arc::new(StorageClient::new(&data_plane_config));
    let crypto_client = Arc::new(
        CryptoClient::new(&data_plane_config, &storage_client)
            .await
            .context("crypto setup failed")?,
    );

    // Initialize crypto and ingress servers
    data_plane::crypto::init(&data_plane_config, &crypto_client, &storage_client);
    data_plane::ingress::init(&data_plane_config, &storage_client, &crypto_client);

    // Run user process
    let exit_code = data_plane::runner::run_until_shutdown(&data_plane_config).await;

    Ok(exit_code)
}

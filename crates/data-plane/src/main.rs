use clap::Parser;
use data_plane::{CryptoClient, DataPlaneState, RuntimeConfig, StorageClient};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info};

#[cfg(all(target_os = "linux", any(feature = "enclave", feature = "pebble")))]
async fn setup_egress_if_enabled(runtime_config: &RuntimeConfig) {
    if let Err(error) = data_plane::egress::init(
        &runtime_config.egress,
        &runtime_config.tls_termination,
        Some(&runtime_config.aws_region),
    )
    .await
    {
        error!(
            error = %format!("{error:#}"),
            "egress whitelist setup failed"
        );
        std::process::exit(1);
    }
}

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

    // OTLP export to the host collector when `NITRUM_OTLP_ENDPOINT` is set
    // (e.g. `http://192.168.127.1:4317`, the gvproxy gateway to the host);
    // stdout only otherwise so `nitrum local` works without a collector.
    let telemetry_guard = telemetry::init(telemetry::TelemetryConfig {
        service_name: "data-plane".to_string(),
        resource_attributes: Vec::new(),
        otlp_endpoint: std::env::var("NITRUM_OTLP_ENDPOINT").ok(),
    });
    telemetry::metrics::init_instruments();

    let Args {
        config,
        command: cli_command,
    } = Args::parse();

    // gvproxy TAP path must be up before IMDS / SSM / HTTPS egress; IMDS uses `169.254.169.254`
    // when the parent runs gvproxy with `-ec2-metadata-access`.
    #[cfg(feature = "enclave")]
    data_plane::networking::init().await;
    #[cfg(not(feature = "enclave"))]
    info!("enclave networking (TAP + VSOCK) requires feature `enclave`; skipping");

    let runtime_config = RuntimeConfig::load(&config).await.unwrap_or_else(|e| {
        error!(
            error = %format!("{:#}", e),
            config_path = %config.display(),
            "failed to load runtime config"
        );
        std::process::exit(1);
    });

    #[cfg(all(target_os = "linux", any(feature = "enclave", feature = "pebble")))]
    setup_egress_if_enabled(&runtime_config).await;

    let user_command: Vec<String> = if cli_command.is_empty() {
        runtime_config.nitrum.project.start_command.clone()
    } else {
        cli_command
    };

    // Create storage client
    let storage: Arc<StorageClient> = Arc::new(StorageClient::new(&runtime_config));

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
        if let Err(e) = data_plane::crypto::api::run(crypto_state).await {
            error!(error = %e, "API task failed");
        }
        tracing::warn!("API task exited");
    });

    // Kick off ingress for external usage
    let ingress_state = state.clone();
    tokio::spawn(async move {
        info!("ingress task starting");
        if let Err(e) = data_plane::server::ingress::run(ingress_state).await {
            error!(error = %e, "ingress task failed");
        }
        tracing::warn!("ingress task exited");
    });

    // Run user process if provided, otherwise run until SIGINT
    let exit_code = if user_command.is_empty() {
        info!("no command provided, running until SIGINT");
        let _ = tokio::signal::ctrl_c().await;
        info!("received SIGINT, shutting down");
        0
    } else {
        tokio::select! {
            result = data_plane::server::runner::run(&user_command, &runtime_config.user_env) => {
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

    #[cfg(all(target_os = "linux", any(feature = "enclave", feature = "pebble")))]
    if runtime_config.egress.enabled {
        data_plane::egress::teardown();
    }

    // Flush traces/metrics/logs to the collector before exiting (normal path).
    telemetry_guard.shutdown().await;

    std::process::exit(exit_code);
}

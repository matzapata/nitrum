use clap::Parser;
use data_plane::{CryptoClient, DataPlaneState, RuntimeConfig, StorageClient};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info};

#[cfg(all(target_os = "linux", any(feature = "enclave", feature = "pebble")))]
async fn setup_egress_if_enabled(runtime_config: &RuntimeConfig, otlp_endpoint: Option<&str>) {
    if let Err(error) = data_plane::egress::init(
        &runtime_config.egress,
        &runtime_config.tls_termination,
        Some(&runtime_config.aws_region),
        otlp_endpoint,
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

#[cfg(feature = "enclave")]
async fn resolve_otlp_endpoint() -> Option<String> {
    if std::env::var(data_plane::constants::ENV_OTLP_ENDPOINT).is_ok() {
        return data_plane::constants::otlp_endpoint(None);
    }

    match data_plane::default_otlp_endpoint_from_imds().await {
        Ok(endpoint) => data_plane::constants::otlp_endpoint(Some(&endpoint)),
        Err(error) => {
            eprintln!(
                "telemetry: failed to resolve default OTLP endpoint from IMDS ({error:#}); falling back to stdout-only logging"
            );
            None
        }
    }
}

#[cfg(not(feature = "enclave"))]
fn resolve_otlp_endpoint() -> Option<String> {
    data_plane::constants::otlp_endpoint(None)
}

#[tokio::main]
async fn main() {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install default rustls crypto provider");

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

    #[cfg(feature = "enclave")]
    let otlp_endpoint = resolve_otlp_endpoint().await;
    #[cfg(not(feature = "enclave"))]
    let otlp_endpoint = resolve_otlp_endpoint();
    let telemetry_guard = telemetry::init(telemetry::TelemetryConfig {
        service_name: "data-plane".to_string(),
        resource_attributes: Vec::new(),
        otlp_endpoint: otlp_endpoint.clone(),
    });
    telemetry::metrics::init_instruments();

    let runtime_config = RuntimeConfig::load(&config).await.unwrap_or_else(|e| {
        eprintln!("failed to load runtime config: {e:#}");
        error!(
            error = %format!("{:#}", e),
            config_path = %config.display(),
            "failed to load runtime config"
        );
        std::process::exit(1);
    });

    #[cfg(all(target_os = "linux", any(feature = "enclave", feature = "pebble")))]
    setup_egress_if_enabled(&runtime_config, otlp_endpoint.as_deref()).await;

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

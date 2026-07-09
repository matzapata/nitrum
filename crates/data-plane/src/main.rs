use clap::Parser;
use data_plane::{ConfigBootstrap, CryptoClient, DataPlaneState, RuntimeConfig, StorageClient};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use telemetry::TelemetryGuard;
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

async fn load_runtime(
    config_path: &Path,
) -> Result<(RuntimeConfig, TelemetryGuard), anyhow::Error> {
    #[cfg(feature = "enclave")]
    data_plane::networking::init().await;

    let bootstrap = ConfigBootstrap::bootstrap(config_path).await?;
    let telemetry_guard = telemetry::init(telemetry::TelemetryConfig {
        service_name: "data-plane".to_string(),
        resource_attributes: Vec::new(),
        otlp_endpoint: bootstrap.otlp_endpoint().map(str::to_string),
    });
    telemetry::metrics::init_instruments();

    #[cfg(not(feature = "enclave"))]
    info!("enclave networking (TAP + VSOCK) requires feature `enclave`; skipping");

    let runtime_config = bootstrap.into_runtime_config().await?;

    #[cfg(all(target_os = "linux", any(feature = "enclave", feature = "pebble")))]
    data_plane::egress::init(&runtime_config).await?;

    Ok((runtime_config, telemetry_guard))
}

fn spawn_servers(state: Arc<DataPlaneState>) {
    let crypto_state = state.clone();
    tokio::spawn(async move {
        info!("API task starting");
        if let Err(e) = data_plane::crypto::api::run(crypto_state).await {
            error!(error = %e, "API task failed");
        }
        tracing::warn!("API task exited");
    });

    tokio::spawn(async move {
        info!("ingress task starting");
        if let Err(e) = data_plane::server::ingress::run(state).await {
            error!(error = %e, "ingress task failed");
        }
        tracing::warn!("ingress task exited");
    });
}

fn resolve_start_command(cli_start_command: Vec<String>, config: &RuntimeConfig) -> Vec<String> {
    if cli_start_command.is_empty() {
        config.nitrum.project.start_command.clone()
    } else {
        cli_start_command
    }
}

async fn run_until_exit(
    effective_start_command: &[String],
    user_env: &HashMap<String, String>,
) -> i32 {
    if effective_start_command.is_empty() {
        info!("no command provided, running until SIGINT");
        let _ = tokio::signal::ctrl_c().await;
        info!("received SIGINT, shutting down");
        return 0;
    }

    tokio::select! {
        result = data_plane::server::runner::run(effective_start_command, user_env) => {
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
}

#[tokio::main]
async fn main() {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install default rustls crypto provider");

    let Args {
        config,
        command: cli_start_command,
    } = Args::parse();

    let (runtime_config, telemetry_guard) = load_runtime(&config).await.unwrap_or_else(|e| {
        eprintln!("failed to start data-plane: {e:#}");
        error!(
            error = %format!("{:#}", e),
            config_path = %config.display(),
            "failed to start data-plane"
        );
        std::process::exit(1);
    });

    let effective_start_command = resolve_start_command(cli_start_command, &runtime_config);

    let storage: Arc<StorageClient> = Arc::new(StorageClient::new(&runtime_config));
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
    let state = Arc::new(DataPlaneState::new(
        runtime_config.clone(),
        storage,
        crypto,
    ));

    spawn_servers(state);

    let exit_code = run_until_exit(&effective_start_command, &runtime_config.user_env).await;

    #[cfg(all(target_os = "linux", any(feature = "enclave", feature = "pebble")))]
    if runtime_config.egress.enabled {
        data_plane::egress::teardown();
    }

    telemetry_guard.shutdown().await;

    std::process::exit(exit_code);
}

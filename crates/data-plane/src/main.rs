use anyhow::Context;
use clap::Parser;
use config::NitrumConfig;
use data_plane::{AesGcmCrypto, AwsKms, DataPlaneConfig, DynamoObjectStore};
use std::path::PathBuf;
use std::process::ExitCode;
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
async fn main() -> ExitCode {
    // Stdout subscriber first so networking / IMDS / SSM failures are visible
    // (production has no nitro debug console — it changes PCR0).
    // Guard drops on return and flushes OTLP exporters (avoid `process::exit`).
    let mut telemetry = telemetry::init(telemetry::TelemetryConfig::platform("data-plane"));

    let outcome = async {
        rustls::crypto::ring::default_provider()
            .install_default()
            .expect("failed to install default rustls crypto provider");

        let Args { config } = Args::parse();

        // Nitro enclave TAP/VSOCK setup; skipped for local Compose (`pebble` feature only).
        #[cfg(all(target_os = "linux", feature = "enclave"))]
        data_plane::networking::init()
            .await
            .context("enclave networking init failed")?;

        let nitrum = NitrumConfig::try_from(config.as_path())
            .with_context(|| format!("load config from {}", config.display()))?;
        let data_plane_config = DataPlaneConfig::try_from(nitrum)
            .await
            .context("resolve data-plane infra config")?;

        // OTLP to the parent collector uses the same TAP path as other egress.
        let _egress_guard = data_plane::egress::init(&data_plane_config)
            .await
            .context("egress init failed")?;

        // Attach OTLP once the collector endpoint is known and egress is up.
        telemetry.enable_otlp(data_plane_config.otlp_endpoint.as_deref());

        let storage_client = Arc::new(DynamoObjectStore::from_config(&data_plane_config));
        let kms = AwsKms::from_config(&data_plane_config);
        let crypto_client = Arc::new(
            AesGcmCrypto::from_kms(storage_client.clone(), &kms, &data_plane_config.instance_id)
                .await
                .context("crypto setup failed")?,
        );

        data_plane::crypto::init(&data_plane_config, &crypto_client);
        data_plane::ingress::init(&data_plane_config, &storage_client, &crypto_client);

        Ok::<_, anyhow::Error>(data_plane::runner::run_until_shutdown(&data_plane_config).await)
    }
    .await;

    match outcome {
        Ok(code) => code,
        Err(error) => {
            // Log while the guard is still alive so OTLP exporters can receive it.
            error!(error = %format!("{:#}", error));
            ExitCode::FAILURE
        }
    }
}

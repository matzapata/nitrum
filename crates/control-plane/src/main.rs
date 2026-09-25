use clap::Parser;
use control_plane::ControlPlaneConfig;
use std::path::PathBuf;
use std::process::ExitCode;
use tracing::error;

#[derive(clap::Parser)]
#[command(name = "control-plane")]
struct Args {
    /// Path to the EIF file (`nitro-cli --eif-path`). Conflicts with `--eif-bucket` / `--eif-hash`.
    #[arg(long, value_name = "PATH", conflicts_with_all = ["eif_bucket", "eif_hash"])]
    eif: Option<PathBuf>,
    /// S3 bucket containing the EIF (object key is `{eif-hash}.eif`, same as `nitrum cloud deploy`).
    #[arg(
        long,
        value_name = "NAME",
        requires = "eif_hash",
        conflicts_with = "eif"
    )]
    eif_bucket: Option<String>,
    /// Version label / hash prefix for the EIF (first 12 hex chars of EIF sha256 from deploy; S3 key `{hash}.eif`).
    #[arg(
        long,
        value_name = "HASH",
        requires = "eif_bucket",
        conflicts_with = "eif"
    )]
    eif_hash: Option<String>,
    /// Enable debug mode.
    #[arg(long, default_value_t = false)]
    debug_mode: bool,
    /// vCPUs passed to `nitro-cli run-enclave --cpu-count` (must match nitro allocator).
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u32).range(1..))]
    cpu_count: u32,
    /// Memory (MiB) passed to `nitro-cli run-enclave --memory` (must match nitro allocator).
    #[arg(long, default_value_t = 4320, value_parser = clap::value_parser!(u32).range(1..))]
    memory_mib: u32,
}

#[tokio::main]
async fn main() -> ExitCode {
    // Host has `NITRUM_OTLP_ENDPOINT` at process start — one-shot init is fine.
    // Guard drops on return and flushes OTLP exporters (avoid `process::exit`).
    let _telemetry = telemetry::init(
        telemetry::TelemetryConfig::platform("control-plane")
            .with_otlp_endpoint(std::env::var("NITRUM_OTLP_ENDPOINT").ok()),
    );

    let args = Args::parse();

    let config = match ControlPlaneConfig::from_cli(
        args.eif,
        args.eif_bucket,
        args.eif_hash,
        args.debug_mode,
        args.cpu_count,
        args.memory_mib,
    ) {
        Ok(config) => config,
        Err(e) => {
            error!(error = %format!("{:#}", e));
            return ExitCode::FAILURE;
        }
    };

    match control_plane::run(config).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!(error = %format!("{:#}", e));
            ExitCode::FAILURE
        }
    }
}

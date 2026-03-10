use clap::Parser;
use std::process::Command;
use tracing::info;

mod networking;

use shared::config;

#[derive(Parser)]
#[command(name = "data-plane")]
struct Args {
    /// VSOCK port where gvproxy listens on the host (CID 3).
    #[arg(long, default_value_t = 1024)]
    host_proxy_port: u32,

    /// Path to nitrum.toml. When present with a command after `--`, the app is run with networking up (ingress + egress).
    #[arg(long)]
    config: Option<std::path::PathBuf>,

    /// Command to run after networking is up (e.g. `node /app/src/main.js`). Enables testing ingress and egress.
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

    networking::setup(args.host_proxy_port);

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    if !args.command.is_empty() {
        info!(command = ?args.command, "running app for ingress/egress");
        run_app(&args.command);
        return;
    }

    info!("testing connectivity via gvproxy …");
    match test_connectivity().await {
        Ok(body) => info!("httpbin.org/ip response:\n{body}"),
        Err(e) => tracing::error!("connectivity test failed: {e}"),
    }

    info!("networking is up — keeping process alive");
    std::future::pending::<()>().await;
}

fn run_app(argv: &[String]) {
    let (program, rest) = argv
        .split_first()
        .expect("command non-empty");
    let status = Command::new(program)
        .args(rest)
        .status()
        .unwrap_or_else(|e| panic!("failed to run {program:?}: {e}"));
    std::process::exit(
        status.code().unwrap_or_else(|| libc::EXIT_FAILURE as i32),
    );
}

async fn test_connectivity() -> Result<String, Box<dyn std::error::Error>> {
    let client = reqwest::Client::builder()
        .user_agent("nitrum-data-plane/0.1")
        .build()?;

    let response = client
        .get("http://httpbin.org/ip")
        .send()
        .await?
        .error_for_status()?;

    let body = response.text().await?;
    Ok(body)
}

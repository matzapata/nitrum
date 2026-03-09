use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use clap::Parser;
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::info;

mod api;
mod attestation;
mod config;
mod constants;
mod egress;
mod ingress;
mod networking;

#[derive(Parser)]
#[command(name = "data-plane")]
struct Args {
    /// Path to the nitrum.toml configuration file.
    #[arg(long, default_value = "/app/nitrum.toml")]
    config: PathBuf,

    /// Customer application command and arguments (everything after `--`).
    /// Example: data-plane --config /app/nitrum.toml -- node /app/src/main.js
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    app_cmd: Vec<String>,
}

/// Drop the process to `DATAPLANE_UID` so the data-plane's own egress
/// connections are exempted by the iptables `--uid-owner` rule set up in
/// `networking::setup()`. Must be called AFTER spawning the customer app so
/// the app process inherits root uid and its traffic IS subject to the redirect.
fn drop_privileges() {
    use crate::constants::DATAPLANE_UID;
    let ret = unsafe { libc::setuid(DATAPLANE_UID) };
    assert_eq!(
        ret,
        0,
        "setuid({DATAPLANE_UID}) failed: {}",
        std::io::Error::last_os_error()
    );
    info!(uid = DATAPLANE_UID, "privileges dropped to data-plane user");
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = config::load(&args.config);
    let egress_filter = Arc::new(egress::filter::EgressFilter::new(
        config.egress.enabled,
        &config.egress.whitelist,
    ));

    // ── Network setup (runs as root, transport-independent) ───────────────────
    networking::setup();

    // ── Spawn the customer app BEFORE dropping privileges ─────────────────────
    // Child inherits root uid so its egress traffic IS subject to the iptables
    // redirect; the data-plane (uid 1500 after drop_privileges) is exempted.
    let child_handle = if args.app_cmd.is_empty() {
        info!("no customer app specified, running data-plane only");
        None
    } else {
        info!(cmd = ?args.app_cmd, "spawning customer app");
        let mut child = tokio::process::Command::new(&args.app_cmd[0])
            .args(&args.app_cmd[1..])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn customer app");

        let stdout = child.stdout.take().expect("stdout not captured");
        let stderr = child.stderr.take().expect("stderr not captured");
        info!(pid = child.id(), "customer app started");
        Some((child, stdout, stderr))
    };

    // ── Drop to DATAPLANE_UID so the data-plane's own egress isn't redirected ──
    drop_privileges();

    // ── Start the async proxy tasks ───────────────────────────────────────────

    let app_port = std::env::var("APP_PORT").unwrap_or_else(|_| "8008".to_string());
    let app_addr = format!("127.0.0.1:{app_port}");

    let api_port = std::env::var("NITRUM_API_PORT").unwrap_or_else(|_| "3000".to_string());
    let api_addr = format!("127.0.0.1:{api_port}");

    #[cfg(feature = "enclave")]
    info!("data-plane starting — transport: vsock (enclave mode)");
    #[cfg(not(feature = "enclave"))]
    info!("data-plane starting — transport: tcp (local dev mode)");

    // Stream child stdout / stderr through tracing and exit when the app exits.
    let child_watcher = async move {
        let Some((mut child, stdout, stderr)) = child_handle else {
            std::future::pending::<()>().await;
            return;
        };

        // Spawn log-forwarder tasks so app output is interleaved with data-plane logs.
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                info!(target: "app", "{line}");
            }
        });
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                info!(target: "app", "{line}");
            }
        });

        let status = child.wait().await.expect("error waiting for customer app");
        let code = status.code().unwrap_or(1);
        info!(exit_code = code, "customer app exited, shutting down data-plane");
        std::process::exit(code);
    };

    tokio::join!(
        egress::tcp::run(Arc::clone(&egress_filter)),
        egress::dns::run(Arc::clone(&egress_filter)),
        ingress::tcp::run(app_addr, config.tls_termination),
        api::run(api_addr),
        child_watcher,
    );
}

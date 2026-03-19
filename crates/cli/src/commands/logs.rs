use clap::Args;
use std::env;
use std::process::Stdio;

use anyhow::{Context, Result, bail};
use tokio::process::Command;

use crate::constants;
use crate::utils::compose;

#[derive(Args)]
pub struct LogsArgs {
    /// Project directory (default: current directory)
    #[arg(short, long)]
    pub root: Option<std::path::PathBuf>,
    /// Number of log lines to show from the end of each service (docker compose `--tail`)
    #[arg(long)]
    pub tail: Option<u32>,
    /// Follow log output
    #[arg(short, long)]
    pub follow: bool,
}

pub async fn run(args: LogsArgs) {
    let root = args
        .root
        .clone()
        .unwrap_or_else(|| env::current_dir().expect("current directory"));

    if let Err(e) = run_logs(&root, &args).await {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}

async fn run_logs(root: &std::path::Path, args: &LogsArgs) -> Result<()> {
    compose::require_compose_file(root)?;

    let mut cmd = Command::new("docker");
    cmd.current_dir(root)
        .arg("compose")
        .arg("-f")
        .arg(constants::ENCLAVE_DEV_COMPOSE_FILE)
        .arg("logs")
        .arg("enclave")
        .arg("--no-log-prefix");

    if let Some(n) = args.tail {
        cmd.arg("--tail").arg(n.to_string());
    }
    if args.follow {
        cmd.arg("--follow");
    }

    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let status = cmd
        .status()
        .await
        .context("failed to spawn `docker compose logs`")?;

    if !status.success() {
        bail!("docker compose logs exited with status {status}");
    }
    Ok(())
}

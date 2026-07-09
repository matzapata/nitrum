//! Run and supervise the user process.
//!
//! This module is responsible for managing the lifecycle of a user-supplied application or service
//! process within the data plane. It launches the process as configured, monitors its status,
//! and ensures orderly shutdown and cleanup under normal operations or in the event of system signals.
//!
//! Key responsibilities:
//! - Spawning the user process as specified in the configuration file (`[project].start_command`).
//! - Forwarding environment variables and using appropriate standard IO options.
//! - Supervising the running process: monitoring exit codes, handling process termination signals,
//!   and reaping zombies if necessary.
//! - Handling shutdown: on receipt of a termination request (e.g., SIGINT), gracefully shutting down
//!   the running process or issuing a KILL signal as a fallback.
//!
//! The main entrypoint is [`run_until_shutdown`] which runs the user's command or blocks until shutdown
//! if no command is provided. Proper guarantees are made that no orphaned processes remain after shutdown.
//!
//! This module is intended as internal infrastructure, not as a generic supervisor — it's specialized for
//! launching a single user-controlled application per data-plane run.

use crate::DataPlaneConfig;
use anyhow::Result;
use std::collections::HashMap;
use std::process::Stdio;
use tokio::process::Child;
use tracing::{error, info};

/// Owns the user child process and sends SIGKILL on drop when still running.
struct RunnerGuard {
    /// Child process handle; cleared after a successful wait.
    child: Option<Child>,
}

impl RunnerGuard {
    async fn wait(&mut self) -> Result<i32> {
        let child = self
            .child
            .as_mut()
            .expect("runner guard wait called without child");
        let status = child.wait().await?;
        self.child = None;
        Ok(status.code().unwrap_or(-1))
    }
}

impl Drop for RunnerGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
        }
    }
}

/// Supervise the user process until it exits or a shutdown signal is received.
///
/// Uses `[project].start_command` from config. When no command is configured, the
/// data-plane runs until SIGINT.
pub async fn run_until_shutdown(config: &DataPlaneConfig) -> i32 {
    tokio::select! {
        code = supervise(&config.project.start_command, &config.user_env) => code,
        _ = shutdown_signal() => {
            info!("received SIGINT, shutting down");
            0
        }
    }
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for ctrl_c");
}

async fn supervise<S: std::hash::BuildHasher + Sync>(
    command: &[String],
    child_env: &HashMap<String, String, S>,
) -> i32 {
    if command.is_empty() {
        info!("no command provided, running until SIGINT");
        std::future::pending::<i32>().await
    } else {
        match run(command, child_env).await {
            Ok(code) => code,
            Err(error) => {
                error!(error = %error, "failed to run user process");
                1
            }
        }
    }
}

/// Spawn the user command and stream its stdout/stderr to the tracing log (target "app").
///
/// `child_env` is the child's full environment (e.g. [`DataPlaneConfig::user_env`] from SSM only).
/// Returns the process exit code when the child exits.
async fn run<S: std::hash::BuildHasher + Sync>(
    command: &[String],
    child_env: &HashMap<String, String, S>,
) -> Result<i32> {
    let (program, args) = command
        .split_first()
        .expect("run() called with non-empty command");

    info!(target: "app", program = %program, "spawning user process");

    let mut child = tokio::process::Command::new(program)
        .args(args)
        .env_clear()
        .envs(child_env)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let stdout_handle = tokio::spawn(forward_lines(stdout, "stdout"));
    let stderr_handle = tokio::spawn(forward_lines(stderr, "stderr"));

    let mut guard = RunnerGuard { child: Some(child) };
    let code = guard.wait().await?;

    let _ = tokio::time::timeout(std::time::Duration::from_millis(100), async move {
        tokio::join!(stdout_handle, stderr_handle)
    })
    .await;

    info!(target: "app", exit_code = code, "user process exited");
    Ok(code)
}

async fn forward_lines<R>(reader: R, label: &'static str)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        info!(target: "app", %label, "{}", line);
    }
}

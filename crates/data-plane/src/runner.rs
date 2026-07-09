//! Run and supervise the user process.

use crate::DataPlaneConfig;
use anyhow::Result;
use std::collections::HashMap;
use std::process::Stdio;
use tokio::process::Child;
use tracing::{error, info};

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

//! Run the user process and forward its stdout/stderr to the data-plane logs.

use anyhow::Result;
use std::collections::HashMap;
use std::process::Stdio;
use tracing::info;

/// Spawn the user command and stream its stdout/stderr to the tracing log (target "app").
///
/// `child_env` is the child's full environment (e.g. [`DataPlaneConfig::user_env`] from SSM only).
/// Returns the process exit code when the child exits.
pub async fn run<S: std::hash::BuildHasher + Sync>(
    command: &[String],
    child_env: &HashMap<String, String, S>,
) -> Result<i32> {
    if command.is_empty() {
        return Ok(0);
    }

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

    let status = child.wait().await?;
    let code = status.code().unwrap_or(-1);

    // Allow a moment for final lines to be read
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

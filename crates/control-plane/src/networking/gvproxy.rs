//! gvproxy process lifecycle on the EC2 host.

use super::constants::{
    GVPROXY_BIN_DEFAULT, GVPROXY_BIN_ENV, SOCKET_PATH, SOCKET_WAIT_TIMEOUT_SECS, VSOCK_LISTEN,
};
use anyhow::Context;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tracing::info;

/// Holds the gvproxy child process.
pub struct Gvproxy {
    /// Running gvproxy process, if startup succeeded.
    child: Option<Child>,
}

impl Gvproxy {
    #[must_use]
    pub const fn new() -> Self {
        Self { child: None }
    }

    /// Kill existing gvproxy, start a new one, and wait for the API socket.
    pub async fn start(&mut self) -> anyhow::Result<()> {
        Self::terminate_existing().await;

        info!(
            vsock = VSOCK_LISTEN,
            socket = SOCKET_PATH,
            "starting gvproxy"
        );

        let bin = std::env::var(GVPROXY_BIN_ENV).unwrap_or_else(|_| GVPROXY_BIN_DEFAULT.into());
        let child = Command::new(&bin)
            .arg("-listen")
            .arg(format!("vsock://{VSOCK_LISTEN}"))
            .arg("-listen")
            .arg(format!("unix://{SOCKET_PATH}"))
            .arg("-ec2-metadata-access=true")
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("spawn gvproxy `{bin}`"))?;

        tokio::time::sleep(Duration::from_secs(SOCKET_WAIT_TIMEOUT_SECS)).await;
        if !Path::new(SOCKET_PATH).exists() {
            anyhow::bail!("gvproxy did not create socket in time");
        }

        self.child = Some(child);
        Ok(())
    }

    /// Stops gvproxy and removes the API socket.
    pub async fn shutdown(&mut self) -> anyhow::Result<()> {
        if let Some(mut child) = self.child.take() {
            info!("stopping gvproxy (PID {})", child.id());
            child.kill().context("kill gvproxy")?;
            child.wait().context("wait for gvproxy")?;
        }
        if Path::new(SOCKET_PATH).exists() {
            tokio::fs::remove_file(SOCKET_PATH)
                .await
                .with_context(|| format!("remove stale socket {SOCKET_PATH}"))?;
        }
        Ok(())
    }

    /// Kill any running gvproxy, then remove stale socket.
    async fn terminate_existing() {
        info!("terminating any existing gvproxy");
        let _ = Command::new("pkill").arg("gvproxy").status();
        tokio::time::sleep(Duration::from_millis(500)).await;

        if Path::new(SOCKET_PATH).exists() {
            info!(path = SOCKET_PATH, "removing stale socket");
            let _ = tokio::fs::remove_file(SOCKET_PATH).await;
        }
    }
}

impl Drop for Gvproxy {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            info!("stopping gvproxy (PID {})", child.id());
            let _ = child.kill();
            let _ = child.wait();
        }
        if Path::new(SOCKET_PATH).exists() {
            let _ = std::fs::remove_file(SOCKET_PATH);
        }
    }
}

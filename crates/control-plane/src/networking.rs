//! Start gvproxy (terminate any existing one first), wait for API socket, set up port forwards, kill on drop.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use tracing::{error, info, warn};

/// TODO: make this configurable
const SOCKET_PATH: &str = "/tmp/network.sock";
const VSOCK_LISTEN: &str = ":1024";
const ENCLAVE_IP: &str = "192.168.127.2";
const SOCKET_WAIT_TIMEOUT_SECS: u64 = 15;
const SOCKET_POLL_INTERVAL_MS: u64 = 200;
/// Port forwards: (host_port, enclave_port).
const FORWARDS: &[(u16, u16)] = &[(443, 443), (9090, 9090)];

/// Holds the gvproxy child process. Kills it on drop.
pub struct Networking {
    child: Option<Child>,
}

impl Networking {
    /// Terminate existing gvproxy, start a new one, wait for socket, set up forwards.
    pub async fn start() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Self::terminate_existing();

        info!(vsock = VSOCK_LISTEN, socket = SOCKET_PATH, "starting gvproxy");

        let child = Command::new("gvproxy")
            .arg("-listen")
            .arg(format!("vsock://{}", VSOCK_LISTEN))
            .arg("-listen")
            .arg(format!("unix://{}", SOCKET_PATH))
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()?;

        tokio::time::sleep(Duration::from_secs(SOCKET_WAIT_TIMEOUT_SECS)).await;
        if !Path::new(SOCKET_PATH).exists() {
            return Err("gvproxy did not create socket in time".into());
        }

        for (local, remote) in FORWARDS {
            setup_forward(*local, *remote).await?;
        }

        Ok(Self {
            child: Some(child),
        })
    }

    /// Terminate any running gvproxy, then remove stale socket.
    async fn terminate_existing() {
        info!("terminating any existing gvproxy");
        let _ = Command::new("pkill").arg("gvproxy").status().await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        if Path::new(SOCKET_PATH).exists() {
            info!(path = SOCKET_PATH, "removing stale socket");
            let _ = tokio::fs::remove_file(SOCKET_PATH).await;
        }
    }
}

impl Drop for Networking {
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

fn setup_forward(
    local_port: u16,
    remote_port: u16,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let body = format!(
        r#"{{"local":":{}","remote":"{}:{}"}}"#,
        local_port, ENCLAVE_IP, remote_port
    );

    let status = Command::new("curl")
        .args([
            "-sS",
            "--unix-socket",
            SOCKET_PATH,
            "http://localhost/services/forwarder/expose",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "-d",
            &body,
        ])
        .status();

    match status {
        Ok(s) if s.success() => {
            info!(local = local_port, remote = remote_port, "port forward set");
        }
        Ok(s) => {
            warn!(local = local_port, remote = remote_port, code = ?s.code(), "port forward failed");
        }
        Err(e) => {
            error!(error = %e, "curl for port forward failed");
            return Err(e.into());
        }
    }
    Ok(())
}

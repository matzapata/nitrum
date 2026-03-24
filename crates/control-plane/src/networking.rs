//! Start gvproxy (kill any existing one first), wait for API socket, set up port forwards, kill on drop.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{info, warn};

/// Path to the socket file.
const SOCKET_PATH: &str = "/tmp/network.sock";

/// VSOCK listen address.
const VSOCK_LISTEN: &str = ":1024";

/// Enclave IP address.
const ENCLAVE_IP: &str = "192.168.127.2";

/// Timeout to wait for the socket to be created.
const SOCKET_WAIT_TIMEOUT_SECS: u64 = 15;

/// Port forwards: (host_port, enclave_port).
const FORWARDS: &[(u16, u16)] = &[(443, 443), (9090, 9090)];

/// Env var for gvproxy binary path; default "gvproxy" (on PATH). Set to "/app/gvproxy" in container.
const GVPROXY_BIN_ENV: &str = "GVPROXY_BIN";
const GVPROXY_BIN_DEFAULT: &str = "gvproxy";

/// Holds the gvproxy child process. Kills it on drop.
pub struct Networking {
    child: Option<Child>,
}

impl Networking {
    /// Kill existing gvproxy, start a new one, wait for socket, set up forwards.
    pub async fn run() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Self::terminate_existing().await;

        info!(
            vsock = VSOCK_LISTEN,
            socket = SOCKET_PATH,
            "starting gvproxy"
        );

        let bin = std::env::var(GVPROXY_BIN_ENV).unwrap_or_else(|_| GVPROXY_BIN_DEFAULT.into());
        let child = Command::new(&bin)
            .arg("-listen")
            .arg(format!("vsock://{}", VSOCK_LISTEN))
            .arg("-listen")
            .arg(format!("unix://{}", SOCKET_PATH))
            .arg("-ec2-metadata-access=true")
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

        Ok(Self { child: Some(child) })
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

async fn setup_forward(
    local_port: u16,
    remote_port: u16,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let body = format!(
        r#"{{"local":":{}","remote":"{}:{}"}}"#,
        local_port, ENCLAVE_IP, remote_port
    );

    let status_code = post_forwarder_expose(SOCKET_PATH, &body).await?;

    match status_code {
        Some(c) if (200..300).contains(&c) => {
            info!(local = local_port, remote = remote_port, "port forward set");
        }
        Some(c) => {
            warn!(
                local = local_port,
                remote = remote_port,
                status = c,
                "port forward failed"
            );
        }
        None => {
            warn!(
                local = local_port,
                remote = remote_port,
                "port forward failed: could not parse HTTP status"
            );
        }
    }
    Ok(())
}

/// POST `/services/forwarder/expose` on gvproxy's Unix socket. Returns response status, or I/O error.
#[cfg(unix)]
async fn post_forwarder_expose(socket_path: &str, body: &str) -> std::io::Result<Option<u16>> {
    const EXPOSE_PATH: &str = "/services/forwarder/expose";
    const IO_TIMEOUT: Duration = Duration::from_secs(15);

    let connect = tokio::net::UnixStream::connect(socket_path);
    let mut stream = tokio::time::timeout(IO_TIMEOUT, connect)
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "unix socket connect"))??;

    let request = format!(
        "POST {EXPOSE_PATH} HTTP/1.1\r\n\
         Host: localhost\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len(),
    );

    let write_read = async {
        stream.write_all(request.as_bytes()).await?;
        stream.flush().await?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        Ok::<_, std::io::Error>(response)
    };

    let response = tokio::time::timeout(IO_TIMEOUT, write_read)
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "forwarder expose I/O"))??;

    Ok(parse_http_status_line(&response))
}

#[cfg(unix)]
fn parse_http_status_line(raw: &[u8]) -> Option<u16> {
    let line_end = raw.windows(2).position(|w| w == b"\r\n")?;
    let line = std::str::from_utf8(&raw[..line_end]).ok()?;
    let mut parts = line.split_whitespace();
    parts.next()?; // HTTP/x.y
    parts.next()?.parse().ok()
}

#[cfg(not(unix))]
async fn post_forwarder_expose(
    _socket_path: &str,
    _body: &str,
) -> std::io::Result<Option<u16>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "gvproxy Unix socket API requires a Unix host",
    ))
}

//! Start gvproxy and configure port forwards into the enclave.

mod constants;
mod forwarder;
mod gvproxy;

use constants::{ENCLAVE_IP, FORWARDS, SOCKET_PATH};
use gvproxy::Gvproxy;
use tracing::{info, warn};

/// Host-side networking handle (gvproxy + configured forwards).
pub struct Networking {
    gvproxy: Gvproxy,
}

/// Kill existing gvproxy, start a new one, wait for socket, set up forwards.
pub async fn init() -> anyhow::Result<Networking> {
    let mut gvproxy = Gvproxy::new();
    gvproxy.start().await?;

    for (local, remote) in FORWARDS {
        setup_forward(*local, *remote).await?;
    }

    Ok(Networking { gvproxy })
}

impl Networking {
    /// Stops gvproxy and removes the API socket.
    pub async fn shutdown(mut self) -> anyhow::Result<()> {
        self.gvproxy.shutdown().await
    }
}

async fn setup_forward(local_port: u16, remote_port: u16) -> anyhow::Result<()> {
    let body = format!(r#"{{"local":":{local_port}","remote":"{ENCLAVE_IP}:{remote_port}"}}"#);

    let status_code = forwarder::post_forwarder_expose(SOCKET_PATH, &body).await?;

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

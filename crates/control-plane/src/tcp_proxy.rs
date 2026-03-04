use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::net::TcpStream;
use tracing::{error, info, warn};

use shared::bridge::{Bridge, BridgeInterface, Direction};
use shared::server::Listener;
use shared::{ports, protocol};

async fn handle_connection<S>(mut client: S)
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let mut header = [0u8; 6];
    if let Err(e) = client.read_exact(&mut header).await {
        warn!(error = %e, "tcp: failed to read destination header");
        return;
    }

    let target = protocol::decode_destination(header);

    info!(target = %target, "tcp: received connection, connecting to target");

    let mut upstream = match TcpStream::connect(target).await {
        Ok(s) => s,
        Err(e) => {
            error!(target = %target, error = %e, "tcp: failed to connect to target");
            return;
        }
    };

    match io::copy_bidirectional(&mut client, &mut upstream).await {
        Ok((from_client, from_upstream)) => {
            info!(
                target = %target,
                bytes_from_client = from_client,
                bytes_from_upstream = from_upstream,
                "tcp: connection closed"
            );
        }
        Err(e) => {
            warn!(target = %target, error = %e, "tcp: connection error");
        }
    }
}

pub async fn run() {
    let mut listener = Bridge::get_listener(ports::TCP_PROXY, Direction::EnclaveToHost)
        .await
        .expect("failed to bind TCP proxy");

    info!(port = ports::TCP_PROXY, "tcp proxy listening");

    loop {
        match listener.accept().await {
            Ok(stream) => {
                tokio::spawn(async move {
                    handle_connection(stream).await;
                });
            }
            Err(e) => error!(error = %e, "tcp: accept error"),
        }
    }
}

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::UdpSocket;
use tracing::{error, info, warn};

use shared::bridge::{Bridge, BridgeInterface, Direction};
use shared::server::Listener;
use shared::ports;

async fn handle_connection<S>(mut client: S, upstream_dns: String)
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    // The data-plane shuts down its write side after sending the query, so
    // read_to_end captures exactly one DNS request per connection.
    let mut query = Vec::new();
    if let Err(e) = client.read_to_end(&mut query).await {
        warn!(error = %e, "dns: failed to read query");
        return;
    }

    info!(bytes = query.len(), upstream = %upstream_dns, "dns: received query, forwarding upstream");

    let udp = match UdpSocket::bind("0.0.0.0:0").await {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, "dns: failed to bind UDP socket");
            return;
        }
    };

    if let Err(e) = udp.send_to(&query, &upstream_dns).await {
        error!(upstream = %upstream_dns, error = %e, "dns: failed to send query to upstream");
        return;
    }

    let mut resp_buf = [0u8; 512];
    let resp_len = match udp.recv(&mut resp_buf).await {
        Ok(n) => n,
        Err(e) => {
            error!(error = %e, "dns: failed to receive response from upstream");
            return;
        }
    };

    info!(bytes = resp_len, "dns: received response, sending back");

    if let Err(e) = client.write_all(&resp_buf[..resp_len]).await {
        error!(error = %e, "dns: failed to write response");
    }
}

pub async fn run(upstream_dns: String) {
    let mut listener = Bridge::get_listener(ports::DNS_PROXY, Direction::EnclaveToHost)
        .await
        .expect("failed to bind DNS proxy");

    info!(port = ports::DNS_PROXY, upstream = %upstream_dns, "dns proxy listening");

    loop {
        match listener.accept().await {
            Ok(stream) => {
                let dns_upstream = upstream_dns.clone();
                tokio::spawn(async move {
                    handle_connection(stream, dns_upstream).await;
                });
            }
            Err(e) => error!(error = %e, "dns: accept error"),
        }
    }
}

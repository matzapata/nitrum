use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UdpSocket;
use tracing::{error, info, warn};

use shared::bridge::{Bridge, BridgeInterface, Direction};
use shared::ports;

use crate::constants::DNS_LISTEN_ADDR;
use crate::egress::{self, EgressFilter};

pub async fn run(egress: Arc<EgressFilter>) {
    let listen_addr = std::env::var("DNS_LISTEN_ADDR")
        .unwrap_or_else(|_| DNS_LISTEN_ADDR.to_string());

    let socket = Arc::new(
        UdpSocket::bind(&listen_addr)
            .await
            .expect("failed to bind DNS proxy"),
    );

    info!(addr = %listen_addr, "dns proxy listening");

    let mut buf = vec![0u8; 4096];
    loop {
        let (len, client_addr) = match socket.recv_from(&mut buf).await {
            Ok(r) => r,
            Err(e) => {
                error!(error = %e, "dns: recv error");
                continue;
            }
        };

        let query = buf[..len].to_vec();
        let sock = Arc::clone(&socket);
        let egress = Arc::clone(&egress);

        tokio::spawn(async move {
            let hostname = egress::parse_query_name(&query);

            // Egress whitelist check — block before forwarding to control-plane.
            if let Some(ref h) = hostname {
                if !egress.is_allowed(h) {
                    warn!(hostname = %h, "dns: egress blocked by whitelist");
                    let nxdomain = egress::make_nxdomain(&query);
                    let _ = sock.send_to(&nxdomain, client_addr).await;
                    return;
                }
            }

            info!(client = %client_addr, bytes = len, hostname = ?hostname, "dns: received query, forwarding to control plane");

            let mut stream = match Bridge::get_client_connection(
                ports::DNS_PROXY,
                Direction::EnclaveToHost,
            )
            .await
            {
                Ok(s) => s,
                Err(e) => {
                    error!(error = %e, "dns: failed to connect to control plane");
                    return;
                }
            };

            if let Err(e) = stream.write_all(&query).await {
                error!(error = %e, "dns: failed to write query");
                return;
            }
            // Signal end of request so the control-plane's read_to_end completes.
            if let Err(e) = tokio::io::AsyncWriteExt::shutdown(&mut stream).await {
                error!(error = %e, "dns: failed to shutdown write side");
                return;
            }

            let mut resp = [0u8; 512];
            let resp_len = match stream.read(&mut resp).await {
                Ok(n) if n > 0 => n,
                Ok(_) => {
                    error!("dns: empty response from control plane");
                    return;
                }
                Err(e) => {
                    error!(error = %e, "dns: failed to read response");
                    return;
                }
            };

            info!(client = %client_addr, bytes = resp_len, "dns: received response, forwarding to client");

            if let Err(e) = sock.send_to(&resp[..resp_len], client_addr).await {
                error!(client = %client_addr, error = %e, "dns: failed to send response to client");
            }
        });
    }
}

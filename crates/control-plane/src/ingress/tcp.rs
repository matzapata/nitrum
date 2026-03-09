use std::net::SocketAddr;

use tokio::io;
use tokio::net::{TcpListener, TcpStream};
use tracing::{error, info, warn};

use shared::bridge::{Bridge, BridgeInterface, Direction};
use shared::ports;

async fn handle_connection(mut client: TcpStream, client_addr: SocketAddr) {
    info!(client = %client_addr, "ingress: new connection, forwarding to enclave");

    let mut upstream =
        match Bridge::get_client_connection(ports::INGRESS, Direction::HostToEnclave).await {
            Ok(s) => s,
            Err(e) => {
                error!(error = %e, "ingress: failed to connect to enclave");
                return;
            }
        };

    match io::copy_bidirectional(&mut client, &mut upstream).await {
        Ok((from_client, from_upstream)) => {
            info!(
                client = %client_addr,
                bytes_from_client = from_client,
                bytes_from_upstream = from_upstream,
                "ingress: connection closed"
            );
        }
        Err(e) => {
            warn!(client = %client_addr, error = %e, "ingress: connection error");
        }
    }
}

pub async fn run(port: u16) {
    let listener = TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .expect("failed to bind ingress proxy");

    info!(port, "ingress proxy listening");

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                tokio::spawn(async move {
                    handle_connection(stream, addr).await;
                });
            }
            Err(e) => error!(error = %e, "ingress: accept error"),
        }
    }
}

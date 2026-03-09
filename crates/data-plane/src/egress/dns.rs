use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UdpSocket;
use tracing::{error, info, warn};

use shared::bridge::{Bridge, BridgeInterface, Direction};
use shared::ports;

use crate::constants::DNS_LISTEN_ADDR;
use crate::egress::filter::EgressFilter;

/// Build a minimal NXDOMAIN response for the given raw DNS query.
fn make_nxdomain(query: &[u8]) -> Vec<u8> {
    let mut resp = Vec::with_capacity(query.len());

    // Transaction ID: copy from query bytes 0-1
    if query.len() >= 2 {
        resp.extend_from_slice(&query[0..2]);
    } else {
        resp.extend_from_slice(&[0, 0]);
    }

    // Flags: QR=1 Opcode=0 AA=0 TC=0 RD=1 | RA=1 Z=0 RCODE=3 (NXDOMAIN)
    resp.push(0x81);
    resp.push(0x83);

    // QDCOUNT: copy from query bytes 4-5
    if query.len() >= 6 {
        resp.extend_from_slice(&query[4..6]);
    } else {
        resp.extend_from_slice(&[0, 1]);
    }

    // ANCOUNT=0, NSCOUNT=0, ARCOUNT=0
    resp.extend_from_slice(&[0, 0, 0, 0, 0, 0]);

    // Question section: everything after the 12-byte header
    if query.len() > 12 {
        resp.extend_from_slice(&query[12..]);
    }

    resp
}


/// Extract the queried hostname from a raw DNS query packet.
///
/// Returns `None` if the packet is malformed or too short.
fn parse_query_name(query: &[u8]) -> Option<String> {
    if query.len() < 13 {
        return None;
    }
    let mut pos = 12; // skip 12-byte DNS header
    let mut labels: Vec<&str> = Vec::new();
    loop {
        if pos >= query.len() {
            return None;
        }
        let len = query[pos] as usize;
        if len == 0 {
            break;
        }
        // Compression pointers (top 2 bits set) are not expected in queries.
        if len & 0xC0 == 0xC0 {
            return None;
        }
        pos += 1;
        if pos + len > query.len() {
            return None;
        }
        labels.push(std::str::from_utf8(&query[pos..pos + len]).ok()?);
        pos += len;
    }
    Some(labels.join("."))
}



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
            let hostname = parse_query_name(&query);

            // Egress whitelist check — block before forwarding to control-plane.
            if let Some(ref h) = hostname {
                if !egress.is_allowed(h) {
                    warn!(hostname = %h, "dns: egress blocked by whitelist");
                    let nxdomain = make_nxdomain(&query);
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


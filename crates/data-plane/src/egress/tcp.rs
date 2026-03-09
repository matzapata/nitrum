use std::mem::size_of;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::os::unix::io::AsRawFd;
use std::sync::Arc;

use tokio::io::{self, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{error, info, warn};

use shared::bridge::{Bridge, BridgeInterface, Direction};
use shared::{ports, protocol};

use crate::constants::TCP_PROXY_PORT;
use crate::egress::filter::EgressFilter;

const SOL_IP: libc::c_int = 0;
const SO_ORIGINAL_DST: libc::c_int = 80;

fn get_original_dst(stream: &TcpStream) -> io::Result<SocketAddrV4> {
    let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut len = size_of::<libc::sockaddr_in>() as libc::socklen_t;

    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            SOL_IP,
            SO_ORIGINAL_DST,
            &mut addr as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };

    if rc != 0 {
        return Err(io::Error::last_os_error());
    }

    let ip = Ipv4Addr::from(u32::from_be(addr.sin_addr.s_addr));
    let port = u16::from_be(addr.sin_port);
    Ok(SocketAddrV4::new(ip, port))
}

async fn handle_connection(mut client: TcpStream, client_addr: SocketAddr, _egress: Arc<EgressFilter>) {
    let original_dst = match get_original_dst(&client) {
        Ok(dst) => dst,
        Err(e) => {
            warn!(client = %client_addr, error = %e, "tcp: failed to get original destination, dropping connection");
            return;
        }
    };

    info!(
        client = %client_addr,
        destination = %original_dst,
        "tcp: accepted connection, forwarding to control plane"
    );

    let mut upstream =
        match Bridge::get_client_connection(ports::TCP_PROXY, Direction::EnclaveToHost)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                error!(error = %e, "tcp: failed to connect to control plane");
                return;
            }
        };

    if let Err(e) = upstream.write_all(&protocol::encode_destination(original_dst)).await {
        error!(error = %e, "tcp: failed to send destination header to control plane");
        return;
    }

    match io::copy_bidirectional(&mut client, &mut upstream).await {
        Ok((from_client, from_upstream)) => {
            info!(
                client = %client_addr,
                destination = %original_dst,
                bytes_from_client = from_client,
                bytes_from_upstream = from_upstream,
                "tcp: connection closed"
            );
        }
        Err(e) => {
            warn!(client = %client_addr, destination = %original_dst, error = %e, "tcp: connection error");
        }
    }
}

pub async fn run(egress: Arc<EgressFilter>) {
    let listener = TcpListener::bind(format!("0.0.0.0:{TCP_PROXY_PORT}"))
        .await
        .expect("failed to bind TCP proxy");

    info!(port = TCP_PROXY_PORT, "tcp proxy listening");

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                let egress = Arc::clone(&egress);
                tokio::spawn(async move {
                    handle_connection(stream, addr, egress).await;
                });
            }
            Err(e) => error!(error = %e, "tcp: accept error"),
        }
    }
}

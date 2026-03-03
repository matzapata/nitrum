use crate::server::{error::ServerError, Listener};
use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};

#[derive(Debug, Clone, Copy)]
pub enum ContextID {
    Parent,
    Enclave,
}

/// Represents the direction of traffic across the enclave/host boundary.
///
/// A `Client` with `EnclaveToHost` opens a connection from the enclave to the
/// host. A `Listener` with `EnclaveToHost` listens inside the enclave for
/// connections that originate from the host.
#[derive(Debug, Clone, Copy)]
pub enum Direction {
    EnclaveToHost,
    HostToEnclave,
}

impl Direction {
    /// CID (or IP in local mode) that a client should connect to for this direction.
    pub fn get_client_cid(&self) -> ContextID {
        match self {
            Self::EnclaveToHost => ContextID::Parent,
            Self::HostToEnclave => ContextID::Enclave,
        }
    }
}

/// Unified interface for opening connections across the enclave/host boundary.
/// Backed by VSock when the `enclave` feature is enabled, TCP otherwise.
#[async_trait]
pub trait BridgeInterface {
    type Listener: Listener;
    type ClientConnection: AsyncRead + AsyncWrite + Unpin + Send + 'static;

    async fn get_client_connection(
        port: u16,
        direction: Direction,
    ) -> Result<Self::ClientConnection, ServerError>;

    async fn get_listener(port: u16, direction: Direction) -> Result<Self::Listener, ServerError>;
}

// ── VSock bridge (feature = "enclave") ───────────────────────────────────────

#[cfg(feature = "enclave")]
mod enclave_bridge {
    use super::*;
    use crate::server::VsockServer;
    use tokio_vsock::VsockStream;

    pub struct VSockBridge;

    #[async_trait]
    impl BridgeInterface for VSockBridge {
        type Listener = VsockServer;
        type ClientConnection = VsockStream;

        async fn get_client_connection(
            port: u16,
            direction: Direction,
        ) -> Result<Self::ClientConnection, ServerError> {
            let cid: u32 = match direction.get_client_cid() {
                ContextID::Enclave => crate::ENCLAVE_CID,
                ContextID::Parent => crate::PARENT_CID,
            };
            Ok(VsockStream::connect(tokio_vsock::VsockAddr::new(cid, port.into())).await?)
        }

        async fn get_listener(
            port: u16,
            _direction: Direction,
        ) -> Result<Self::Listener, ServerError> {
            // VMADDR_CID_ANY (u32::MAX) — accept connections on any local CID.
            VsockServer::bind(u32::MAX, port.into()).await
        }
    }

    pub type BridgeServer = VsockServer;
    pub type BridgeClient = VsockStream;
}

#[cfg(feature = "enclave")]
pub use enclave_bridge::{BridgeClient, BridgeServer, VSockBridge as Bridge};

// ── TCP bridge (default, no "enclave" feature) ────────────────────────────────

#[cfg(not(feature = "enclave"))]
mod local_bridge {
    use super::*;
    use crate::server::TcpServer;
    use std::net::IpAddr;
    use tokio::net::TcpStream;

    /// Static IP assigned to the enclave container in docker-compose.
    pub const ENCLAVE_IP: &str = "172.20.0.11";
    /// Static IP assigned to the control-plane container in docker-compose.
    pub const PARENT_IP: &str = "172.20.0.10";

    impl std::convert::From<ContextID> for IpAddr {
        fn from(value: ContextID) -> Self {
            match value {
                ContextID::Enclave => ENCLAVE_IP.parse().expect("hardcoded value"),
                ContextID::Parent => PARENT_IP.parse().expect("hardcoded value"),
            }
        }
    }

    pub struct TcpBridge;

    #[async_trait]
    impl BridgeInterface for TcpBridge {
        type Listener = TcpServer;
        type ClientConnection = TcpStream;

        async fn get_client_connection(
            port: u16,
            direction: Direction,
        ) -> Result<Self::ClientConnection, ServerError> {
            let ip: IpAddr = direction.get_client_cid().into();
            Ok(TcpStream::connect((ip, port)).await?)
        }

        async fn get_listener(
            port: u16,
            _direction: Direction,
        ) -> Result<Self::Listener, ServerError> {
            // Bind to all interfaces — the client uses the specific container IP to reach us.
            TcpServer::bind(("0.0.0.0", port)).await
        }
    }

    pub type BridgeServer = TcpServer;
    pub type BridgeClient = TcpStream;
}

#[cfg(not(feature = "enclave"))]
pub use local_bridge::{BridgeClient, BridgeServer, TcpBridge as Bridge};

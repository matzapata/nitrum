pub mod error;

use async_trait::async_trait;
use error::ServerError;
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::info;

/// Abstracts over TCP and VSock listeners so callers can accept connections
/// without knowing the underlying transport.
#[async_trait]
pub trait Listener: Send {
    type Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static;

    async fn accept(&mut self) -> Result<Self::Stream, ServerError>;
}

pub struct TcpServer(tokio::net::TcpListener);

impl TcpServer {
    pub async fn bind(addr: impl tokio::net::ToSocketAddrs) -> Result<Self, ServerError> {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        info!(local_addr = ?listener.local_addr(), "tcp listener bound");
        Ok(Self(listener))
    }
}

#[async_trait]
impl Listener for TcpServer {
    type Stream = tokio::net::TcpStream;

    async fn accept(&mut self) -> Result<Self::Stream, ServerError> {
        let (stream, _) = self.0.accept().await?;
        Ok(stream)
    }
}

#[cfg(feature = "enclave")]
pub struct VsockServer(tokio_vsock::VsockListener);

#[cfg(feature = "enclave")]
impl VsockServer {
    pub async fn bind(cid: u32, port: u32) -> Result<Self, ServerError> {
        let listener = tokio_vsock::VsockListener::bind(tokio_vsock::VsockAddr::new(cid, port))?;
        info!(cid, port, "vsock listener bound");
        Ok(Self(listener))
    }
}

#[cfg(feature = "enclave")]
#[async_trait]
impl Listener for VsockServer {
    type Stream = tokio_vsock::VsockStream;

    async fn accept(&mut self) -> Result<Self::Stream, ServerError> {
        let (stream, _) = self.0.accept().await?;
        Ok(stream)
    }
}

//! HTTP client for gvproxy's forwarder API over a Unix socket.

use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// POST `/services/forwarder/expose` on gvproxy's Unix socket. Returns response status, or I/O error.
pub async fn post_forwarder_expose(socket_path: &str, body: &str) -> std::io::Result<Option<u16>> {
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

pub(super) fn parse_http_status_line(raw: &[u8]) -> Option<u16> {
    let line_end = raw.windows(2).position(|w| w == b"\r\n")?;
    let line = std::str::from_utf8(&raw[..line_end]).ok()?;
    let mut parts = line.split_whitespace();
    parts.next()?; // HTTP/x.y
    parts.next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::parse_http_status_line;

    #[test]
    fn parses_status_line() {
        let raw = b"HTTP/1.1 204 No Content\r\n\r\n";
        assert_eq!(parse_http_status_line(raw), Some(204));
    }
}

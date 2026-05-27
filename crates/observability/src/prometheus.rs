//! Prometheus text exposition for local control-plane debugging (`NITRUM_PROMETHEUS=1`).
//!
//! This server is intentionally lightweight and intended for development use only.

use crate::emf::{DIM_COMPONENT, DIM_INSTANCE, DIM_PROJECT};
use crate::registry::{MetricKind, MetricsRegistry};
use std::fmt::Write as _;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Render registry contents as Prometheus text format 0.0.4.
#[must_use]
pub fn render(registry: &MetricsRegistry, project: &str, component: &str, instance_id: &str) -> String {
    let samples = registry.snapshot();
    let mut out = String::new();
    for sample in samples {
        let kind = match sample.kind {
            MetricKind::Counter => "counter",
            MetricKind::Gauge => "gauge",
        };
        let _ = writeln!(out, "# TYPE {} {}", sample.name, kind);
        let mut labels = format!(
            "{}=\"{}\",{}=\"{}\",{}=\"{}\"",
            DIM_PROJECT,
            escape_label(project),
            DIM_COMPONENT,
            escape_label(component),
            DIM_INSTANCE,
            escape_label(instance_id),
        );
        for (k, v) in &sample.dims {
            let _ = write!(labels, ",{}=\"{}\"", escape_label(k), escape_label(v));
        }
        let _ = writeln!(out, "{}{{{}}} {}", sample.name, labels, sample.value);
    }
    out
}

fn escape_label(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Serve `GET /metrics` on `listen_addr` until the process exits.
pub async fn serve(
    listen_addr: &str,
    registry: Arc<MetricsRegistry>,
    project: String,
    component: String,
    instance_id: String,
) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(listen_addr).await?;
    loop {
        let (mut socket, _) = listener.accept().await?;
        let registry = Arc::clone(&registry);
        let project = project.clone();
        let component = component.clone();
        let instance_id = instance_id.clone();
        tokio::spawn(async move {
            let mut buf = vec![0_u8; 4096];
            let Ok(n) = socket.read(&mut buf).await else {
                return;
            };
            if n == 0 {
                return;
            }
            let req = String::from_utf8_lossy(&buf[..n]);
            let (status, body) = if req.starts_with("GET /metrics") {
                (
                    "200 OK",
                    render(&registry, &project, &component, &instance_id),
                )
            } else {
                ("404 Not Found", String::from("Not Found\n"))
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/plain; version=0.0.4; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        });
    }
}

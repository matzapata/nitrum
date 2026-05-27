//! Prometheus text exposition for local control-plane debugging (`NITRUM_PROMETHEUS=1`).

use crate::emf::{DIM_COMPONENT, DIM_INSTANCE, DIM_PROJECT};
use crate::registry::{MetricKind, MetricsRegistry};
use std::fmt::Write as _;
use std::sync::Arc;

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
        let (socket, _) = listener.accept().await?;
        let registry = Arc::clone(&registry);
        let project = project.clone();
        let component = component.clone();
        let instance_id = instance_id.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            let Ok(n) = socket.peek(&mut buf).await else {
                return;
            };
            let req = String::from_utf8_lossy(&buf[..n]);
            let body = if req.starts_with("GET /metrics") {
                render(&registry, &project, &component, &instance_id)
            } else {
                String::from("Not Found\n")
            };
            let status = if req.starts_with("GET /metrics") {
                "200 OK"
            } else {
                "404 Not Found"
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/plain; version=0.0.4; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.writable().await;
            let _ = socket.try_write(response.as_bytes());
        });
    }
}

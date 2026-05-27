use observability::{Component, MetricsHandle, Telemetry, TelemetryConfig};
use std::sync::Arc;

/// CloudWatch tracing export and EMF metrics.
pub struct Monitoring {
    telemetry: Telemetry,
    metrics: Arc<MetricsHandle>,
}

impl Monitoring {
    pub async fn init() -> Self {
        let instance_id = std::env::var("NITRUM_INSTANCE_ID").ok();
        let project_name = std::env::var("NITRUM_PROJECT_NAME").ok();
        let telemetry = match project_name.clone() {
            Some(project) => {
                Telemetry::init(TelemetryConfig {
                    project,
                    component: Component::ControlPlane,
                    log_stream_suffix: std::process::id().to_string(),
                    aws_config: None,
                    cloudwatch: true,
                    instance_id: instance_id.clone(),
                })
                .await
            }
            None => Telemetry::init_fmt_only(Component::ControlPlane, None),
        };
        let metrics = Arc::new(telemetry.metrics().clone());

        if std::env::var("NITRUM_PROMETHEUS")
            .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        {
            let registry = metrics.registry();
            let project = project_name.unwrap_or_else(|| "local".to_string());
            let instance = instance_id.unwrap_or_else(|| "local".to_string());
            let listen = std::env::var("NITRUM_PROMETHEUS_LISTEN")
                .unwrap_or_else(|_| "127.0.0.1:9090".to_string());
            tokio::spawn(async move {
                if let Err(e) = observability::prometheus::serve(
                    &listen,
                    registry,
                    project,
                    "control-plane".to_string(),
                    instance,
                )
                .await
                {
                    eprintln!("prometheus /metrics server failed: {e:#}");
                }
            });
        }

        Self { telemetry, metrics }
    }

    /// Metrics handle for enclave supervisor gauges and counters.
    #[must_use]
    pub fn metrics(&self) -> Arc<MetricsHandle> {
        Arc::clone(&self.metrics)
    }

    /// Graceful shutdown: wait for the CloudWatch worker and metrics flusher to drain.
    pub async fn shutdown(self) {
        self.telemetry.shutdown().await;
    }
}

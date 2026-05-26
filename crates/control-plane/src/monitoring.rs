use observability::{Component, Telemetry, TelemetryConfig};

/// CloudWatch tracing export; owns the worker guard so drop triggers flush.
pub struct Monitoring(Telemetry);

impl Monitoring {
    pub async fn init() -> Self {
        let telemetry = match std::env::var("NITRUM_PROJECT_NAME") {
            Ok(project) => {
                Telemetry::init(TelemetryConfig {
                    project,
                    component: Component::ControlPlane,
                    log_stream_suffix: std::process::id().to_string(),
                    aws_config: None,
                    cloudwatch: true,
                })
                .await
            }
            Err(_) => Telemetry::init_fmt_only(Component::ControlPlane, None),
        };
        Self(telemetry)
    }

    /// Graceful shutdown: wait for the CloudWatch worker to drain.
    pub async fn shutdown(self) {
        self.0.shutdown().await;
    }
}

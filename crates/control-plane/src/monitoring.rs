//! Control-plane telemetry bootstrap: structured stdout logs plus OpenTelemetry
//! (traces + metrics + logs over OTLP) when `NITRUM_OTLP_ENDPOINT` is set.
//! Backed by the `telemetry` crate; no AWS-specific telemetry lives here.

use telemetry::{TelemetryConfig, TelemetryGuard};

/// Owns telemetry background workers; [`Monitoring::shutdown`] flushes them.
pub struct Monitoring {
    /// Guard owning the OpenTelemetry providers for the process lifetime.
    guard: TelemetryGuard,
}

impl Monitoring {
    /// Initialize logging/metrics/tracing for the control-plane.
    ///
    /// Exports over OTLP when `NITRUM_OTLP_ENDPOINT` is set (e.g.
    /// `http://127.0.0.1:4317` for a collector on the host); otherwise logs to
    /// stdout only. Must be called from within the Tokio runtime.
    pub fn init() -> Self {
        let guard = telemetry::init(TelemetryConfig {
            service_name: "control-plane".to_string(),
            resource_attributes: Vec::new(),
            otlp_endpoint: std::env::var("NITRUM_OTLP_ENDPOINT").ok(),
        });
        telemetry::metrics::init_instruments();
        Self { guard }
    }

    /// Graceful shutdown: flush the OpenTelemetry exporters.
    pub async fn shutdown(self) {
        self.guard.shutdown().await;
    }
}

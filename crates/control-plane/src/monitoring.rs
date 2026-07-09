//! Control-plane telemetry bootstrap: structured stdout logs plus OpenTelemetry
//! (traces + metrics + logs over OTLP) when `NITRUM_OTLP_ENDPOINT` is set.
//! Backed by the `telemetry` crate; no AWS-specific telemetry lives here.

use telemetry::{TelemetryConfig, TelemetryGuard};

/// Owns telemetry background workers; flushes OTLP exporters on [`Drop`].
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
        let guard = telemetry::init(
            TelemetryConfig::new("control-plane")
                .with_otlp_endpoint(std::env::var("NITRUM_OTLP_ENDPOINT").ok()),
        );
        Self { guard }
    }
}

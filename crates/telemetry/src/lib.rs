//! Shared observability for the Nitrum workspace (OpenTelemetry-first).
//!
//! Every binary initializes telemetry once via [`init`]. Two things are always
//! wired:
//! - structured logs to stdout (non-ANSI, container-friendly), and
//! - the application metric instruments (see [`metrics`]).
//!
//! When an OTLP/gRPC endpoint is configured, traces, metrics, and logs are also
//! exported over OTLP to a local OpenTelemetry Collector, which is responsible
//! for translating them into a backend (CloudWatch/X-Ray on AWS, something else
//! on other platforms). No backend-specific code lives here: swapping platforms
//! is a Collector configuration change, not a code change.
//!
//! Redaction: only low-cardinality, non-sensitive attributes are ever attached
//! to telemetry (service, route template, method, status class, operation
//! names). Request/response headers, bodies, and secrets are never recorded.

pub mod metrics;

#[cfg(feature = "http")]
pub mod http;

mod otel;

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// Telemetry configuration supplied by each binary at startup.
pub struct TelemetryConfig {
    /// `service.name` reported on all exported telemetry (e.g. `data-plane`).
    pub service_name: String,
    /// Extra OpenTelemetry resource attributes (e.g. `service.instance.id`).
    pub resource_attributes: Vec<(String, String)>,
    /// OTLP/gRPC collector endpoint (e.g. `http://127.0.0.1:4317`).
    /// When `None`, only stdout logging is configured (local dev / tests).
    pub otlp_endpoint: Option<String>,
}

/// Owns the OpenTelemetry providers so they stay alive for the process lifetime.
///
/// [`TelemetryGuard::shutdown`] flushes and stops all exporters; dropping the
/// guard without calling it leaves flushing to provider `Drop` (best-effort).
#[derive(Default)]
pub struct TelemetryGuard {
    /// Tracer provider exporting spans over OTLP; `None` in stdout-only mode.
    tracer_provider: Option<SdkTracerProvider>,
    /// Meter provider exporting metrics over OTLP; `None` in stdout-only mode.
    meter_provider: Option<SdkMeterProvider>,
    /// Logger provider exporting log records over OTLP; `None` in stdout-only mode.
    logger_provider: Option<SdkLoggerProvider>,
}

impl TelemetryGuard {
    /// Flush and stop all OTLP exporters. Best-effort; errors are ignored so a
    /// shutdown flush never blocks process exit.
    ///
    /// Async to keep a stable shutdown contract for callers and to allow future
    /// async flush paths, even though provider shutdown is currently synchronous.
    #[allow(clippy::unused_async)]
    pub async fn shutdown(self) {
        if let Some(provider) = self.tracer_provider {
            let _ = provider.shutdown();
        }
        if let Some(provider) = self.meter_provider {
            let _ = provider.shutdown();
        }
        if let Some(provider) = self.logger_provider {
            let _ = provider.shutdown();
        }
    }
}

/// Read the global log filter from `RUST_LOG`, defaulting to `info`.
fn env_filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
}

/// Initialize process-wide telemetry and return a guard that flushes on shutdown.
///
/// Always installs a stdout logging layer. When `cfg.otlp_endpoint` is set, also
/// installs OTLP trace and log layers, builds and registers the global OTLP meter
/// provider, and returns the providers in the guard. If the OTLP exporters cannot
/// be built, telemetry degrades to stdout-only logging rather than failing.
///
/// Must be called from within a Tokio runtime: the OTLP gRPC exporters require
/// one. Call [`metrics::init_instruments`] afterwards to create the application
/// metric instruments against the (now global) meter provider.
#[must_use]
pub fn init(cfg: TelemetryConfig) -> TelemetryGuard {
    let Some(endpoint) = cfg.otlp_endpoint.as_deref().filter(|e| !e.is_empty()) else {
        init_stdout_only();
        return TelemetryGuard::default();
    };

    let resource = otel::build_resource(&cfg.service_name, &cfg.resource_attributes);
    match otel::build_providers(endpoint, resource) {
        Ok(providers) => install_with_otlp(providers),
        Err(error) => {
            eprintln!(
                "telemetry: OTLP exporters disabled ({error:#}); falling back to stdout-only logging"
            );
            init_stdout_only();
            TelemetryGuard::default()
        }
    }
}

/// Install a stdout-only subscriber (structured, non-ANSI logs).
fn init_stdout_only() {
    tracing_subscriber::registry()
        .with(env_filter())
        .with(fmt::layer().with_ansi(false))
        .init();
}

/// Install the full stdout + OTLP (traces, metrics, logs) pipeline.
fn install_with_otlp(providers: otel::Providers) -> TelemetryGuard {
    // `tracing-opentelemetry` exports spans; the appender bridge ships `tracing`
    // events as OTLP log records. Both are bound to the registry below.
    let trace_layer =
        tracing_opentelemetry::layer().with_tracer(providers.tracer_provider.tracer("nitrum"));
    let logs_layer = OpenTelemetryTracingBridge::new(&providers.logger_provider);

    // Register globals so `opentelemetry::global::{tracer,meter}` resolve to the
    // OTLP providers (used by `metrics` and any library emitting OTel directly).
    opentelemetry::global::set_tracer_provider(providers.tracer_provider.clone());
    opentelemetry::global::set_meter_provider(providers.meter_provider.clone());

    tracing_subscriber::registry()
        .with(env_filter())
        .with(fmt::layer().with_ansi(false))
        .with(trace_layer)
        .with(logs_layer)
        .init();

    TelemetryGuard {
        tracer_provider: Some(providers.tracer_provider),
        meter_provider: Some(providers.meter_provider),
        logger_provider: Some(providers.logger_provider),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::Key;

    #[test]
    fn build_resource_includes_service_name_and_attributes() {
        let resource = otel::build_resource(
            "data-plane",
            &[("service.instance.id".to_string(), "i-123".to_string())],
        );

        assert_eq!(
            resource
                .get(&Key::from_static_str("service.name"))
                .map(|v| v.to_string()),
            Some("data-plane".to_string())
        );
        assert_eq!(
            resource
                .get(&Key::from_static_str("service.instance.id"))
                .map(|v| v.to_string()),
            Some("i-123".to_string())
        );
    }

    #[test]
    fn init_without_otlp_endpoint_is_stdout_only_and_does_not_panic() {
        let guard = init(TelemetryConfig {
            service_name: "test".to_string(),
            resource_attributes: Vec::new(),
            otlp_endpoint: None,
        });
        // No OTLP providers are created in stdout-only mode.
        assert!(guard.tracer_provider.is_none());
        assert!(guard.meter_provider.is_none());
        assert!(guard.logger_provider.is_none());
    }
}

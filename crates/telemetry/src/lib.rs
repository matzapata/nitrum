//! Shared observability for the Nitrum workspace (OpenTelemetry-first).
//!
//! Every binary initializes telemetry once via [`init`]. Two things are always
//! wired:
//! - structured logs to stdout (non-ANSI, container-friendly), and
//! - the application metric instruments (see [`metrics`]).
//!
//! When an OTLP/gRPC endpoint is configured (at [`init`] or later via
//! [`TelemetryGuard::enable_otlp`]), traces, metrics, and logs are also
//! exported over OTLP to a local OpenTelemetry Collector, which is responsible
//! for translating them into a backend (CloudWatch/X-Ray on AWS, something else
//! on other platforms). No backend-specific code lives here: swapping platforms
//! is a Collector configuration change, not a code change.
//!
//! Redaction: only low-cardinality, non-sensitive attributes are ever attached
//! to telemetry (service, route template, method, status class, operation
//! names). Request/response headers, bodies, and secrets are never recorded.

pub mod env;
pub mod metrics;

#[cfg(feature = "http")]
pub mod http;

mod otel;

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::filter::FilterFn;
use tracing_subscriber::reload;
use tracing_subscriber::{
    EnvFilter, Layer, Registry, fmt, layer::SubscriberExt, util::SubscriberInitExt,
};

/// Boxed OTLP layers installed behind a [`reload`] handle (starts as `None`).
type DynLayer = Box<dyn Layer<Registry> + Send + Sync>;
type OtlpReload = reload::Handle<Option<DynLayer>, Registry>;

/// Telemetry configuration supplied by each binary at startup.
pub struct TelemetryConfig {
    /// `service.name` reported on all exported telemetry (e.g. `data-plane`).
    pub service_name: String,
    /// Extra OpenTelemetry resource attributes (e.g. `service.instance.id`).
    pub resource_attributes: Vec<(String, String)>,
    /// OTLP/gRPC collector endpoint (e.g. `http://127.0.0.1:4317`).
    /// When `None`, only stdout logging is configured until [`TelemetryGuard::enable_otlp`].
    pub otlp_endpoint: Option<String>,
}

impl TelemetryConfig {
    /// Create telemetry config for `service_name` with stdout logging only.
    pub fn new(service_name: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
            resource_attributes: Vec::new(),
            otlp_endpoint: None,
        }
    }

    /// Create telemetry config for a Nitrum platform binary (`data-plane`,
    /// `control-plane`): tags telemetry with [`env::CORE_COMPONENT`] and
    /// [`env::NAMESPACE`] in addition to `service_name`.
    #[must_use]
    pub fn platform(service_name: impl Into<String>) -> Self {
        Self::new(service_name).with_resource_attributes([
            ("nitrum.component", env::CORE_COMPONENT),
            ("service.namespace", env::NAMESPACE),
        ])
    }

    /// Attach an OTLP/gRPC collector endpoint when export is enabled.
    #[must_use]
    pub fn with_otlp_endpoint(mut self, endpoint: Option<impl AsRef<str>>) -> Self {
        self.otlp_endpoint = endpoint.map(|e| e.as_ref().to_string());
        self
    }

    /// Attach OpenTelemetry resource attributes (e.g. `nitrum.component=core`).
    #[must_use]
    pub fn with_resource_attributes(
        mut self,
        attributes: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        self.resource_attributes.extend(
            attributes
                .into_iter()
                .map(|(key, value)| (key.into(), value.into())),
        );
        self
    }
}

/// Owns the OpenTelemetry providers so they stay alive for the process lifetime.
///
/// Flushes and stops all OTLP exporters on [`Drop`]. Prefer returning from
/// `main` (e.g. [`std::process::ExitCode`]) so Drop runs. Call [`Self::shutdown`]
/// only when you must flush before [`std::process::exit`], which skips destructors.
pub struct TelemetryGuard {
    /// Tracer provider exporting spans over OTLP; `None` in stdout-only mode.
    tracer_provider: Option<SdkTracerProvider>,
    /// Meter provider exporting metrics over OTLP; `None` in stdout-only mode.
    meter_provider: Option<SdkMeterProvider>,
    /// Logger provider exporting log records over OTLP; `None` in stdout-only mode.
    logger_provider: Option<SdkLoggerProvider>,
    /// Service name for late [`Self::enable_otlp`] resource construction.
    service_name: String,
    /// Resource attributes for late [`Self::enable_otlp`].
    resource_attributes: Vec<(String, String)>,
    /// Reload handle for optional OTLP layers (always present after [`init`]).
    otlp_reload: Option<OtlpReload>,
}

impl TelemetryGuard {
    /// Flush and stop all OTLP exporters. Best-effort; errors are ignored so a
    /// shutdown flush never blocks process exit.
    fn shutdown_providers(&mut self) {
        if let Some(provider) = self.tracer_provider.take() {
            let _ = provider.shutdown();
        }
        if let Some(provider) = self.meter_provider.take() {
            let _ = provider.shutdown();
        }
        if let Some(provider) = self.logger_provider.take() {
            let _ = provider.shutdown();
        }
    }

    /// Attach OTLP export after stdout logging is already running.
    ///
    /// Used by the data-plane after config/IMDS resolve the collector endpoint.
    /// No-op when `endpoint` is empty/`None`, when OTLP is already enabled, or
    /// when exporters cannot be built (falls back to stdout-only with `eprintln!`).
    pub fn enable_otlp(&mut self, endpoint: Option<&str>) {
        let Some(endpoint) = endpoint.filter(|e| !e.is_empty()) else {
            return;
        };
        if self.tracer_provider.is_some() {
            return;
        }
        let Some(handle) = self.otlp_reload.as_ref() else {
            return;
        };

        let resource = otel::build_resource(&self.service_name, &self.resource_attributes);
        let providers = match otel::build_providers(endpoint, resource) {
            Ok(providers) => providers,
            Err(error) => {
                eprintln!(
                    "telemetry: OTLP exporters disabled ({error:#}); falling back to stdout-only logging"
                );
                return;
            }
        };

        let trace_layer =
            tracing_opentelemetry::layer().with_tracer(providers.tracer_provider.tracer("nitrum"));
        // Filter SDK/internal targets out of the bridge. Their own AfterShutdown
        // warnings are emitted via `tracing`; feeding them back into the logger
        // provider recurses until the tokio worker stack overflows.
        let logs_layer = OpenTelemetryTracingBridge::new(&providers.logger_provider).with_filter(
            FilterFn::new(|metadata| {
                let target = metadata.target();
                !(target.starts_with("opentelemetry") || target.starts_with("tonic"))
            }),
        );

        let boxed: DynLayer = Box::new(trace_layer.and_then(logs_layer));
        if let Err(error) = handle.reload(Some(boxed)) {
            eprintln!(
                "telemetry: OTLP layer reload failed ({error}); falling back to stdout-only logging"
            );
            let _ = providers.tracer_provider.shutdown();
            let _ = providers.meter_provider.shutdown();
            let _ = providers.logger_provider.shutdown();
            return;
        }

        opentelemetry::global::set_tracer_provider(providers.tracer_provider.clone());
        opentelemetry::global::set_meter_provider(providers.meter_provider.clone());

        self.tracer_provider = Some(providers.tracer_provider);
        self.meter_provider = Some(providers.meter_provider);
        self.logger_provider = Some(providers.logger_provider);
    }

    /// Flush and stop all OTLP exporters before the guard is dropped.
    ///
    /// Prefer letting the guard drop on normal `main` return. Use this only when
    /// you must flush before [`std::process::exit`].
    #[allow(clippy::unused_async)]
    pub async fn shutdown(mut self) {
        self.shutdown_providers();
    }
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        self.shutdown_providers();
    }
}

/// Read the global log filter from `RUST_LOG`, defaulting to `info`.
fn env_filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
}

/// Initialize process-wide telemetry and return a guard that flushes on shutdown.
///
/// Always installs a stdout logging layer (so early bootstrap failures are
/// visible) and application metric instruments. When `cfg.otlp_endpoint` is set,
/// also attaches OTLP immediately. Otherwise call [`TelemetryGuard::enable_otlp`]
/// later once the endpoint is known. If OTLP exporters cannot be built,
/// telemetry degrades to stdout-only logging rather than failing.
///
/// Must be called from within a Tokio runtime when enabling OTLP: the OTLP gRPC
/// exporters require one.
#[must_use]
pub fn init(cfg: TelemetryConfig) -> TelemetryGuard {
    let (otlp_layer, otlp_reload) = reload::Layer::new(None::<DynLayer>);

    // OTLP reload slot is innermost (`S = Registry`) so [`enable_otlp`] can
    // install boxed layers typed against `Registry`.
    tracing_subscriber::registry()
        .with(otlp_layer)
        .with(env_filter())
        .with(fmt::layer().with_ansi(false))
        .init();

    let mut guard = TelemetryGuard {
        tracer_provider: None,
        meter_provider: None,
        logger_provider: None,
        service_name: cfg.service_name,
        resource_attributes: cfg.resource_attributes,
        otlp_reload: Some(otlp_reload),
    };

    if let Some(endpoint) = cfg.otlp_endpoint.as_deref() {
        guard.enable_otlp(Some(endpoint));
    }

    metrics::init_instruments();
    guard
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
        let guard = init(TelemetryConfig::new("test"));
        // No OTLP providers are created in stdout-only mode.
        assert!(guard.tracer_provider.is_none());
        assert!(guard.meter_provider.is_none());
        assert!(guard.logger_provider.is_none());
    }
}

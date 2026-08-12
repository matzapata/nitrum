//! OpenTelemetry OTLP exporter and provider wiring.
//!
//! Builds the three OTLP/gRPC exporters (traces, metrics, logs) and their SDK
//! providers from a single collector endpoint. The providers are owned by the
//! caller (stored in the telemetry guard) so they stay alive for export and can
//! be flushed on shutdown.

use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;

/// The set of OTLP providers backing traces, metrics, and logs export.
pub struct Providers {
    /// Exports spans over OTLP/gRPC.
    pub tracer_provider: SdkTracerProvider,
    /// Exports metrics over OTLP/gRPC (also installed as the global meter provider).
    pub meter_provider: SdkMeterProvider,
    /// Exports log records over OTLP/gRPC.
    pub logger_provider: SdkLoggerProvider,
}

/// Build the OpenTelemetry resource describing this service instance.
///
/// `service_name` becomes `service.name`; `attributes` are added verbatim as
/// additional resource attributes (e.g. `service.instance.id`).
pub fn build_resource(service_name: &str, attributes: &[(String, String)]) -> Resource {
    Resource::builder()
        .with_service_name(service_name.to_string())
        .with_attributes(
            attributes
                .iter()
                .map(|(k, v)| KeyValue::new(k.clone(), v.clone())),
        )
        .build()
}

/// Build the trace, metric, and log providers exporting to `endpoint` over gRPC.
///
/// # Errors
///
/// Returns `Err` if any of the OTLP exporters cannot be constructed (e.g. an
/// invalid endpoint or transport setup failure).
pub fn build_providers(endpoint: &str, resource: Resource) -> anyhow::Result<Providers> {
    let tracer_provider = SdkTracerProvider::builder()
        .with_batch_exporter(
            opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .build()?,
        )
        .with_resource(resource.clone())
        .build();

    let meter_provider = SdkMeterProvider::builder()
        .with_periodic_exporter(
            opentelemetry_otlp::MetricExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .build()?,
        )
        .with_resource(resource.clone())
        .build();

    let logger_provider = SdkLoggerProvider::builder()
        .with_batch_exporter(
            opentelemetry_otlp::LogExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .build()?,
        )
        .with_resource(resource)
        .build();

    Ok(Providers {
        tracer_provider,
        meter_provider,
        logger_provider,
    })
}

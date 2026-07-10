//! Well-known OpenTelemetry environment variable names.
//!
//! Also defines Nitrum's component/namespace tagging conventions shared by
//! platform binaries (`data-plane`, `control-plane`) and user application
//! processes.

/// Standard OpenTelemetry env var for the OTLP/gRPC collector endpoint.
pub const OTEL_EXPORTER_OTLP_ENDPOINT: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";

/// Standard OpenTelemetry env var for the OTLP transport protocol.
pub const OTEL_EXPORTER_OTLP_PROTOCOL: &str = "OTEL_EXPORTER_OTLP_PROTOCOL";

/// Standard OpenTelemetry env var for `service.name`.
pub const OTEL_SERVICE_NAME: &str = "OTEL_SERVICE_NAME";

/// Standard OpenTelemetry env var for extra resource attributes.
pub const OTEL_RESOURCE_ATTRIBUTES: &str = "OTEL_RESOURCE_ATTRIBUTES";

/// OTLP/gRPC protocol value for [`OTEL_EXPORTER_OTLP_PROTOCOL`].
pub const OTLP_PROTOCOL_GRPC: &str = "grpc";

/// `service.namespace` shared by all Nitrum platform and application telemetry.
pub const NAMESPACE: &str = "nitrum";

/// `nitrum.component` value for Nitrum platform binaries (`data-plane`, `control-plane`).
pub const CORE_COMPONENT: &str = "core";

/// `nitrum.component` value for user application processes instrumented via
/// Nitrum-injected `OTEL_*` env vars.
pub const USER_APP_COMPONENT: &str = "user-app";

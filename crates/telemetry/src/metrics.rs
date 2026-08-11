//! Application metrics built on OpenTelemetry instruments.
//!
//! Instruments are created once via [`init_instruments`] against the global
//! meter provider installed by [`crate::init`]. Every recording helper is a
//! no-op until then (and is harmless in stdout-only mode, where the global
//! provider is the OTel no-op meter). Only low-cardinality attributes are used
//! so the metric backend does not suffer a cardinality blow-up.

use std::sync::OnceLock;

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Gauge, Histogram};

/// Process-wide instruments, populated once by [`init_instruments`].
static INSTRUMENTS: OnceLock<Instruments> = OnceLock::new();

/// The set of OpenTelemetry instruments emitted across the workspace.
struct Instruments {
    /// Count of HTTP requests served, by service/route/method/status class.
    requests: Counter<u64>,
    /// HTTP request handling duration in milliseconds.
    request_duration_ms: Histogram<f64>,
    /// Count of ACME certificate lifecycle events, by event type.
    acme_events: Counter<u64>,
    /// Seconds until the active TLS certificate expires, by domain.
    cert_expiry_seconds: Gauge<f64>,
    /// Count of enclave (re)start attempts, by result.
    enclave_restarts: Counter<u64>,
}

/// Create the metric instruments against the global meter provider.
///
/// Called automatically by [`crate::init`]. Safe to call when no OTLP endpoint is
/// configured: the instruments bind to the no-op meter and recording is free.
pub fn init_instruments() {
    let meter = opentelemetry::global::meter("nitrum");
    let instruments = Instruments {
        requests: meter.u64_counter("nitrum.requests").build(),
        request_duration_ms: meter.f64_histogram("nitrum.request.duration.ms").build(),
        acme_events: meter.u64_counter("nitrum.acme.events").build(),
        cert_expiry_seconds: meter.f64_gauge("nitrum.acme.cert.expiry.seconds").build(),
        enclave_restarts: meter.u64_counter("nitrum.enclave.restarts").build(),
    };
    let _ = INSTRUMENTS.set(instruments);
}

/// Record one served HTTP request (count + duration). No-op if uninitialized.
///
/// `route` is the matched route template (never a raw URL) and `status_class` is
/// a coarse class such as `2xx`.
pub fn record_request(
    service: &'static str,
    route: &str,
    method: &str,
    status_class: &'static str,
    duration_ms: f64,
) {
    let Some(instruments) = INSTRUMENTS.get() else {
        return;
    };
    let attributes = [
        KeyValue::new("service", service),
        KeyValue::new("route", route.to_string()),
        KeyValue::new("method", method.to_string()),
        KeyValue::new("status_class", status_class),
    ];
    instruments.requests.add(1, &attributes);
    instruments
        .request_duration_ms
        .record(duration_ms, &attributes);
}

/// Record an ACME certificate lifecycle event (e.g. `issued`, `renewed`).
/// No-op if uninitialized.
pub fn record_acme_event(event: &'static str) {
    let Some(instruments) = INSTRUMENTS.get() else {
        return;
    };
    instruments
        .acme_events
        .add(1, &[KeyValue::new("event", event)]);
}

/// Record seconds remaining until the active TLS certificate for `domain`
/// expires. No-op if uninitialized.
pub fn set_cert_expiry_seconds(domain: &str, seconds: f64) {
    let Some(instruments) = INSTRUMENTS.get() else {
        return;
    };
    instruments
        .cert_expiry_seconds
        .record(seconds, &[KeyValue::new("domain", domain.to_string())]);
}

/// Record an enclave (re)start attempt, tagged by `result` (e.g. `started`,
/// `failed`). No-op if uninitialized.
pub fn record_enclave_restart(result: &'static str) {
    let Some(instruments) = INSTRUMENTS.get() else {
        return;
    };
    instruments
        .enclave_restarts
        .add(1, &[KeyValue::new("result", result)]);
}

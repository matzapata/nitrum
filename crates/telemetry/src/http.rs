//! Generic axum request-metrics middleware.
//!
//! Records request count and latency, dimensioned by the matched route template
//! (never the raw URL, to avoid metric cardinality blow-ups), method, and status
//! class. Only route/method/status are captured; request and response headers
//! and bodies are never inspected or logged.

use std::time::Instant;

use axum::{
    Router,
    extract::{MatchedPath, Request},
    middleware::{Next, from_fn},
    response::Response,
};

use crate::metrics;

/// Attach request metrics to all routes of `router`, tagged with `service`.
///
/// `service` is a low-cardinality label such as `data-plane.ingress`. Apply after
/// the routes are registered so the matched-path extension is available.
pub fn instrument_router<S>(router: Router<S>, service: &'static str) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router.layer(from_fn(move |req: Request, next: Next| {
        track(service, req, next)
    }))
}

/// Measure one request and record it via [`metrics::record_request`].
async fn track(service: &'static str, req: Request, next: Next) -> Response {
    let start = Instant::now();
    let method = req.method().as_str().to_owned();
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| "other".to_string(), |m| m.as_str().to_owned());

    let response = next.run(req).await;

    let status_class = status_class(response.status().as_u16());
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    metrics::record_request(service, &route, &method, status_class, elapsed_ms);

    response
}

/// Map an HTTP status code to its class label (`2xx`, `4xx`, ...).
const fn status_class(status: u16) -> &'static str {
    match status {
        100..=199 => "1xx",
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        _ => "5xx",
    }
}

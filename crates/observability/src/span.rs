//! Shared structured field names for Nitrum logging conventions.

/// Ingress correlation id (HTTP header `x-request-id` or generated UUID).
pub const REQUEST_ID: &str = "request_id";

/// Stable error category for operators and CloudWatch Insights (`error.kind` in JSON).
pub const ERROR_KIND: &str = "error.kind";

/// Project identifier (`nitrum.toml` `project.name` or `NITRUM_PROJECT_NAME`).
pub const PROJECT: &str = "project";

/// Log plane: `control-plane`, `data-plane`, or `app`.
pub const COMPONENT: &str = "component";

//! Production-grade logging for Nitrum control-plane and data-plane.
//!
//! # Structured fields
//!
//! Every log event includes:
//! - [`span::PROJECT`] — `nitrum.toml` `project.name` or `NITRUM_PROJECT_NAME`
//! - [`span::COMPONENT`] — `control-plane`, `data-plane`, or `app` (when `tracing` target is `app`)
//!
//! Ingress handlers should set [`span::REQUEST_ID`]. Failures should set [`span::ERROR_KIND`].
//!
//! # Environment
//!
//! - `RUST_LOG` — `tracing_subscriber::EnvFilter` (default `info`)
//! - `NITRUM_LOG_FORMAT` — `json` or `human` (default `human`)
//! - `NITRUM_PROJECT_NAME` — when set on control-plane host, enables CloudWatch export without explicit config

mod cloudwatch;
mod fields;
mod format;
mod redact;
pub mod span;

use aws_sdk_cloudwatchlogs::Client;
use fields::{NitrumEventFormat, RedactingFieldFormatter};
use format::LogFormat;
use std::sync::Arc;
use std::time::Duration;
use tracing_cloudwatch::CloudWatchWorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};

pub use format::LogFormat as LogOutputFormat;
pub use redact::{redact_field, redact_str};

/// Nitrum runtime component for the `component` log field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Component {
    /// Host-side control-plane process.
    ControlPlane,
    /// In-enclave data-plane process.
    DataPlane,
    /// User application stdout/stderr (`tracing` target `app`).
    App,
}

impl Component {
    /// CloudWatch log group suffix and JSON `component` value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ControlPlane => "control-plane",
            Self::DataPlane => "data-plane",
            Self::App => "app",
        }
    }

    /// CloudWatch log group path segment under `/nitrum/{project}/`.
    #[must_use]
    pub const fn log_group_suffix(self) -> &'static str {
        match self {
            Self::ControlPlane => "control-plane",
            Self::DataPlane => "data-plane",
            Self::App => "data-plane",
        }
    }
}

/// Configuration for [`Telemetry::init`].
pub struct TelemetryConfig {
    /// Project name (`nitrum.toml` or `NITRUM_PROJECT_NAME`).
    pub project: String,
    /// Default `component` field (overridden to `app` when event target is `app`).
    pub component: Component,
    /// Suffix appended after `{component}-` for the CloudWatch log stream name.
    pub log_stream_suffix: String,
    /// Optional pre-loaded AWS SDK config (data-plane enclave). When `None`, loads defaults.
    pub aws_config: Option<Arc<aws_config::SdkConfig>>,
    /// When `false`, skip CloudWatch even if project is set (local dev).
    pub cloudwatch: bool,
}

/// Initialized tracing subscriber; call [`Self::shutdown`] on graceful exit to drain CloudWatch.
pub struct Telemetry {
    guard: Option<CloudWatchWorkerGuard>,
}

impl Telemetry {
    /// Initialize tracing: `EnvFilter`, redacting fmt, optional CloudWatch.
    pub async fn init(config: TelemetryConfig) -> Self {
        let log_format = LogFormat::from_env();
        let env_filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

        let project = Arc::<str>::from(config.project.as_str());
        let event_format = NitrumEventFormat::new(project.clone(), config.component, log_format);

        let fmt_layer = tracing_subscriber::fmt::layer()
            .with_ansi(matches!(log_format, LogFormat::Human))
            .event_format(event_format)
            .fmt_fields(RedactingFieldFormatter);

        let cloudwatch_enabled = config.cloudwatch
            && (std::env::var("NITRUM_PROJECT_NAME").is_ok() || !config.project.is_empty());

        if !cloudwatch_enabled {
            Registry::default().with(env_filter).with(fmt_layer).init();
            return Self { guard: None };
        }

        let log_group = format!(
            "/nitrum/{}/{}",
            config.project,
            config.component.log_group_suffix()
        );
        let stream_prefix = config.component.as_str();
        let log_stream = format!("{stream_prefix}-{}", config.log_stream_suffix);

        let sdk_config = match config.aws_config {
            Some(c) => c,
            None => {
                Arc::new(aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await)
            }
        };
        let client = Client::new(sdk_config.as_ref());

        if let Err(e) = cloudwatch::ensure_log_stream(&client, &log_group, &log_stream).await {
            eprintln!(
                "observability (cloudwatch) setup failed ({log_group}/{log_stream}): {e:#}; using fmt logs only"
            );
            Registry::default().with(env_filter).with(fmt_layer).init();
            return Self { guard: None };
        }

        let (cw_layer, guard) = tracing_cloudwatch::layer().with_client(
            client,
            tracing_cloudwatch::ExportConfig::default()
                .with_log_group_name(&log_group)
                .with_log_stream_name(&log_stream)
                .with_batch_size(50)
                .with_interval(Duration::from_secs(1)),
        );

        Registry::default()
            .with(env_filter)
            .with(fmt_layer)
            .with(cw_layer)
            .init();

        Self { guard: Some(guard) }
    }

    /// Fmt-only subscriber for early bootstrap or hosts without `NITRUM_PROJECT_NAME`.
    #[must_use]
    pub fn init_fmt_only(default_component: Component, project: Option<&str>) -> Self {
        let log_format = LogFormat::from_env();
        let env_filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
        let project = Arc::<str>::from(project.unwrap_or("local"));
        let event_format = NitrumEventFormat::new(project, default_component, log_format);

        let fmt_layer = tracing_subscriber::fmt::layer()
            .with_ansi(matches!(log_format, LogFormat::Human))
            .event_format(event_format)
            .fmt_fields(RedactingFieldFormatter);

        Registry::default().with(env_filter).with(fmt_layer).init();

        Self { guard: None }
    }

    /// Graceful shutdown: wait for the CloudWatch worker to drain.
    pub async fn shutdown(mut self) {
        if let Some(g) = self.guard.take() {
            g.shutdown().await;
        }
    }
}

use std::time::Duration;

use aws_sdk_cloudwatchlogs::Client;
use tracing_cloudwatch::CloudWatchWorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// `CloudWatch` tracing export; owns the worker guard so drop triggers flush (see `CloudWatchWorkerGuard`).
pub struct Monitoring {
    guard: Option<CloudWatchWorkerGuard>,
}

impl Monitoring {
    pub async fn init() -> Self {
        let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
        let fmt_layer = tracing_subscriber::fmt::layer();

        let Some(project_name) = std::env::var("NITRUM_PROJECT_NAME").ok() else {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt_layer)
                .init();
            return Self { guard: None };
        };

        let log_group = format!("/nitrum/{project_name}/control-plane");
        let log_stream = format!("control-plane-{}", std::process::id());
        let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let client = Client::new(&config);
        if let Err(e) = ensure_log_stream(&client, &log_group, &log_stream).await {
            eprintln!(
                "monitoring (cloudwatch) setup failed ({log_group}/{log_stream}): {e:#}; using fmt logs only"
            );
            tracing_subscriber::registry()
                .with(env_filter)
                .with(fmt_layer)
                .init();
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

        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .with(cw_layer)
            .init();

        Self { guard: Some(guard) }
    }

    /// Graceful shutdown: wait for the `CloudWatch` worker to drain.
    /// If this is never called, dropping `Monitoring` still runs `CloudWatchWorkerGuard`'s destructor (best-effort flush signal).
    pub async fn shutdown(mut self) {
        if let Some(g) = self.guard.take() {
            g.shutdown().await;
        }
    }
}

async fn ensure_log_stream(client: &Client, group: &str, stream: &str) -> anyhow::Result<()> {
    let out = client
        .create_log_stream()
        .log_group_name(group)
        .log_stream_name(stream)
        .send()
        .await;
    match out {
        Ok(_) => Ok(()),
        Err(e) => {
            if format!("{e:?}").contains("ResourceAlreadyExistsException") {
                Ok(())
            } else {
                Err(anyhow::anyhow!("{e}"))
            }
        }
    }
}

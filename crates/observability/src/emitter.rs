//! Flush registry snapshots to CloudWatch Logs as EMF lines.

use crate::registry::MetricsRegistry;
use anyhow::{Context, Result};
use aws_sdk_cloudwatchlogs::Client;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

struct FlushContext {
    client: Client,
    log_group: String,
    log_stream: String,
    namespace: String,
    project: String,
    component: String,
    instance_id: String,
    sequence: Arc<Mutex<Option<String>>>,
}

/// Publishes metrics to a dedicated CloudWatch Logs stream using EMF.
pub struct MetricsEmitter {
    client: Client,
    log_group: String,
    log_stream: String,
    namespace: String,
    project: String,
    component: String,
    instance_id: String,
    registry: Arc<MetricsRegistry>,
    sequence: Arc<Mutex<Option<String>>>,
    shutdown_tx: watch::Sender<bool>,
    flush_handle: Option<JoinHandle<()>>,
}

impl MetricsEmitter {
    /// Create emitter and start a periodic flush task.
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        client: Client,
        log_group: String,
        metrics_stream_suffix: String,
        project: String,
        component: &str,
        instance_id: String,
        registry: Arc<MetricsRegistry>,
        sequence: Arc<Mutex<Option<String>>>,
        flush_interval: Duration,
    ) -> Result<Self> {
        let log_stream = format!("{metrics_stream_suffix}-metrics");
        crate::cloudwatch::ensure_log_stream(&client, &log_group, &log_stream).await?;

        let namespace = format!("Nitrum/{project}");
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let ctx = FlushContext {
            client: client.clone(),
            log_group: log_group.clone(),
            log_stream: log_stream.clone(),
            namespace: namespace.clone(),
            project: project.clone(),
            component: component.to_string(),
            instance_id: instance_id.clone(),
            sequence: Arc::clone(&sequence),
        };
        let flush_handle =
            spawn_flush_task(ctx, Arc::clone(&registry), flush_interval, shutdown_rx);

        Ok(Self {
            client,
            log_group,
            log_stream,
            namespace,
            project,
            component: component.to_string(),
            instance_id,
            registry,
            sequence,
            shutdown_tx,
            flush_handle: Some(flush_handle),
        })
    }

    /// Flush metrics immediately.
    pub async fn flush(&self) -> Result<()> {
        flush_once(
            &FlushContext {
                client: self.client.clone(),
                log_group: self.log_group.clone(),
                log_stream: self.log_stream.clone(),
                namespace: self.namespace.clone(),
                project: self.project.clone(),
                component: self.component.clone(),
                instance_id: self.instance_id.clone(),
                sequence: Arc::clone(&self.sequence),
            },
            &self.registry,
        )
        .await
    }

    /// Stop periodic flush and drain once.
    pub async fn shutdown(mut self) {
        let _ = self.shutdown_tx.send(true);
        if let Some(h) = self.flush_handle.take() {
            let _ = h.await;
        }
    }
}

fn spawn_flush_task(
    ctx: FlushContext,
    registry: Arc<MetricsRegistry>,
    interval: Duration,
    mut shutdown_rx: watch::Receiver<bool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let _ = flush_once(&ctx, &registry).await;
                }
                changed = shutdown_rx.changed() => {
                    if changed.is_ok() && *shutdown_rx.borrow() {
                        break;
                    }
                }
            }
        }
        let _ = flush_once(&ctx, &registry).await;
    })
}

async fn flush_once(ctx: &FlushContext, registry: &MetricsRegistry) -> Result<()> {
    let samples = registry.take_samples();
    let Some(body) = crate::emf::build_emf_document(
        &ctx.namespace,
        &ctx.project,
        &ctx.component,
        &ctx.instance_id,
        &samples,
    ) else {
        return Ok(());
    };

    put_emf_log_event(ctx, &body).await
}

async fn put_emf_log_event(ctx: &FlushContext, message: &str) -> Result<()> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);

    let max_attempts = 3_u8;
    for attempt in 0..max_attempts {
        let token = ctx.sequence.lock().await.clone();

        let event = aws_sdk_cloudwatchlogs::types::InputLogEvent::builder()
            .message(message)
            .timestamp(ts)
            .build()
            .context("build InputLogEvent")?;

        let mut req = ctx
            .client
            .put_log_events()
            .log_group_name(&ctx.log_group)
            .log_stream_name(&ctx.log_stream)
            .log_events(event);

        if let Some(ref t) = token {
            req = req.sequence_token(t);
        }

        match req.send().await {
            Ok(out) => {
                if let Some(next) = out.next_sequence_token() {
                    *ctx.sequence.lock().await = Some(next.to_string());
                }
                return Ok(());
            }
            Err(e) => {
                let refresh = put_needs_sequence_refresh(&e);
                if refresh && attempt + 1 < max_attempts {
                    let fresh = fetch_upload_sequence_token(
                        &ctx.client,
                        &ctx.log_group,
                        &ctx.log_stream,
                    )
                    .await
                    .unwrap_or(None);
                    *ctx.sequence.lock().await = fresh;
                    continue;
                }
                return Err(anyhow::Error::from(e)).context("PutLogEvents EMF");
            }
        }
    }

    Ok(())
}

fn put_needs_sequence_refresh(
    e: &aws_sdk_cloudwatchlogs::error::SdkError<
        aws_sdk_cloudwatchlogs::operation::put_log_events::PutLogEventsError,
    >,
) -> bool {
    use aws_sdk_cloudwatchlogs::error::ProvideErrorMetadata as _;
    if e.code() == Some("InvalidSequenceTokenException") {
        return true;
    }
    let s = format!("{e:?}");
    s.contains("InvalidSequenceTokenException") || s.contains("invalid sequence token")
}

async fn fetch_upload_sequence_token(
    client: &Client,
    log_group: &str,
    stream: &str,
) -> Result<Option<String>> {
    let out = client
        .describe_log_streams()
        .log_group_name(log_group)
        .log_stream_name_prefix(stream)
        .send()
        .await
        .context("DescribeLogStreams (EMF sequence)")?;

    for ls in out.log_streams() {
        if ls.log_stream_name.as_deref() == Some(stream) {
            return Ok(ls.upload_sequence_token().map(ToString::to_string));
        }
    }
    Ok(None)
}

/// Handle passed to application code for recording metrics.
#[derive(Clone)]
pub struct MetricsHandle {
    registry: Arc<MetricsRegistry>,
    flush: Option<MetricsFlushTarget>,
}

#[derive(Clone)]
struct MetricsFlushTarget {
    client: Client,
    log_group: String,
    log_stream: String,
    namespace: String,
    project: String,
    component: String,
    instance_id: String,
    sequence: Arc<Mutex<Option<String>>>,
}

impl MetricsHandle {
    /// Registry-only handle (no CloudWatch flush).
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            registry: Arc::new(MetricsRegistry::new()),
            flush: None,
        }
    }

    #[allow(clippy::missing_const_for_fn, clippy::too_many_arguments)]
    pub(crate) fn new(
        registry: Arc<MetricsRegistry>,
        client: Client,
        log_group: String,
        log_stream: String,
        namespace: String,
        project: String,
        component: String,
        instance_id: String,
        sequence: Arc<Mutex<Option<String>>>,
    ) -> Self {
        Self {
            registry,
            flush: Some(MetricsFlushTarget {
                client,
                log_group,
                log_stream,
                namespace,
                project,
                component,
                instance_id,
                sequence,
            }),
        }
    }

    /// Shared registry (e.g. for Prometheus scrape).
    #[must_use]
    pub fn registry(&self) -> Arc<MetricsRegistry> {
        Arc::clone(&self.registry)
    }

    /// Increment a counter.
    pub fn counter(&self, name: &str, delta: u64) {
        self.registry.counter(name, delta);
    }

    /// Increment a counter with dimensions.
    pub fn counter_dims(&self, name: &str, delta: u64, dims: &[(&str, &str)]) {
        self.registry.counter_dims(name, delta, dims);
    }

    /// Set a gauge.
    pub fn gauge(&self, name: &str, value: f64) {
        self.registry.gauge(name, value);
    }

    /// Set a gauge with dimensions.
    pub fn gauge_dims(&self, name: &str, value: f64, dims: &[(&str, &str)]) {
        self.registry.gauge_dims(name, value, dims);
    }

    /// Flush to CloudWatch Logs (EMF) when export is enabled.
    pub async fn flush(&self) -> Result<()> {
        if let Some(t) = &self.flush {
            flush_once(
                &FlushContext {
                    client: t.client.clone(),
                    log_group: t.log_group.clone(),
                    log_stream: t.log_stream.clone(),
                    namespace: t.namespace.clone(),
                    project: t.project.clone(),
                    component: t.component.clone(),
                    instance_id: t.instance_id.clone(),
                    sequence: Arc::clone(&t.sequence),
                },
                &self.registry,
            )
            .await
        } else {
            Ok(())
        }
    }
}

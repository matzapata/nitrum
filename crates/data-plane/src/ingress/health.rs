//! Background HTTP probes of the user application via `[health_check]`.

use crate::DataPlaneConfig;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tracing::{debug, info, warn};

/// Connect + request timeout for each probe attempt.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Consecutive probe failures required to clear [`IngressState::app_ready`].
const FAILURE_THRESHOLD: u32 = 3;

/// Retry delay while the app is not yet ready (cold start / recovering).
///
/// Using the full `[health_check].interval` here would leave status at 503 for too long
/// after the process starts listening (e.g. first probe races bind).
const STARTUP_RETRY: Duration = Duration::from_millis(500);

/// Spawn a background task that probes the user app and updates `app_ready`.
///
/// No-op when `[project].start_command` is empty (platform-only mode).
pub fn spawn(config: &DataPlaneConfig, app_ready: Arc<AtomicBool>) {
    if config.project.start_command.is_empty() {
        return;
    }

    let path = config.health_check.path.as_str().to_string();
    let port = config.health_check.port.get();
    let interval = Duration::from_secs(u64::from(config.health_check.interval.get()));
    let url = format!("http://127.0.0.1:{port}{path}");

    tokio::spawn(async move {
        info!(%url, interval_secs = interval.as_secs(), "app health probe starting");
        run_probe_loop(&url, interval, &app_ready).await;
    });
}

async fn run_probe_loop(url: &str, interval: Duration, app_ready: &AtomicBool) {
    let client = match reqwest::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .connect_timeout(PROBE_TIMEOUT)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "app health probe: failed to build HTTP client");
            return;
        }
    };

    let mut consecutive_failures: u32 = 0;

    loop {
        match probe_once(&client, url).await {
            Ok(()) => {
                consecutive_failures = 0;
                if !app_ready.swap(true, Ordering::Relaxed) {
                    info!(%url, "app health probe: ready");
                }
            }
            Err(reason) => {
                consecutive_failures = consecutive_failures.saturating_add(1);
                debug!(
                    %url,
                    consecutive_failures,
                    %reason,
                    "app health probe: failed"
                );
                if consecutive_failures >= FAILURE_THRESHOLD
                    && app_ready.swap(false, Ordering::Relaxed)
                {
                    warn!(
                        %url,
                        consecutive_failures,
                        %reason,
                        "app health probe: not ready"
                    );
                }
            }
        }

        // Poll quickly until ready; use the configured interval only while healthy.
        let delay = if app_ready.load(Ordering::Relaxed) {
            interval
        } else {
            STARTUP_RETRY.min(interval)
        };
        tokio::time::sleep(delay).await;
    }
}

async fn probe_once(client: &reqwest::Client, url: &str) -> Result<(), String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("request error: {e}"))?;
    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("HTTP {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::routing::get;
    use std::net::SocketAddr;
    use std::sync::atomic::AtomicU32;
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;

    async fn serve_health(
        status_code: axum::http::StatusCode,
        shutdown: oneshot::Receiver<()>,
    ) -> SocketAddr {
        let app = Router::new().route("/health", get(move || async move { (status_code, "ok") }));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown.await;
                })
                .await
                .expect("serve");
        });
        // Give the server a moment to accept connections.
        tokio::task::yield_now().await;
        addr
    }

    #[tokio::test]
    async fn probe_succeeds_on_2xx() {
        let (tx, rx) = oneshot::channel();
        let addr = serve_health(axum::http::StatusCode::OK, rx).await;
        let url = format!("http://{addr}/health");
        let client = reqwest::Client::builder()
            .timeout(PROBE_TIMEOUT)
            .connect_timeout(PROBE_TIMEOUT)
            .build()
            .unwrap();
        probe_once(&client, &url)
            .await
            .expect("probe should succeed");
        let _ = tx.send(());
    }

    #[tokio::test]
    async fn probe_fails_on_5xx() {
        let (tx, rx) = oneshot::channel();
        let addr = serve_health(axum::http::StatusCode::INTERNAL_SERVER_ERROR, rx).await;
        let url = format!("http://{addr}/health");
        let client = reqwest::Client::builder()
            .timeout(PROBE_TIMEOUT)
            .connect_timeout(PROBE_TIMEOUT)
            .build()
            .unwrap();
        assert!(probe_once(&client, &url).await.is_err());
        let _ = tx.send(());
    }

    #[tokio::test]
    async fn probe_loop_marks_ready_then_not_ready() {
        let hit_count = Arc::new(AtomicU32::new(0));
        let hits = hit_count.clone();
        let app = Router::new().route(
            "/health",
            get(move || {
                let hits = hits.clone();
                async move {
                    let n = hits.fetch_add(1, Ordering::Relaxed);
                    if n < 2 {
                        (axum::http::StatusCode::OK, "ok")
                    } else {
                        (axum::http::StatusCode::SERVICE_UNAVAILABLE, "down")
                    }
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("serve");
        });
        tokio::task::yield_now().await;

        let app_ready = Arc::new(AtomicBool::new(false));
        let url = format!("http://{addr}/health");
        let flag = app_ready.clone();
        let probe = tokio::spawn(async move {
            run_probe_loop(&url, Duration::from_millis(50), &flag).await;
        });

        // Wait until ready after successes.
        tokio::time::timeout(Duration::from_secs(2), async {
            while !app_ready.load(Ordering::Relaxed) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("should become ready");

        // Wait until not ready after FAILURE_THRESHOLD consecutive failures.
        tokio::time::timeout(Duration::from_secs(2), async {
            while app_ready.load(Ordering::Relaxed) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("should become not ready");

        probe.abort();
        let _ = shutdown_tx.send(());
    }
}

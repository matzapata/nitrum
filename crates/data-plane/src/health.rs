//! Application health probe (`[health_check]` in `nitrum.toml`).

use crate::state::DataPlaneState;
use observability::MetricsHandle;
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

/// Polls the user application health endpoint and publishes `AppHealthCheckPass` (0 or 1).
pub async fn run_poller(state: Arc<DataPlaneState>, metrics: MetricsHandle) {
    let hc = &state.config.nitrum.health_check;
    let interval = Duration::from_secs(u64::from(hc.interval));
    let url = format!("http://127.0.0.1:{}{}", hc.port, hc.path);

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "health check: failed to build HTTP client");
            metrics.gauge("AppHealthCheckPass", 0.0);
            return;
        }
    };

    loop {
        let pass = client
            .get(&url)
            .send()
            .await
            .is_ok_and(|r| r.status().is_success());
        metrics.gauge("AppHealthCheckPass", if pass { 1.0 } else { 0.0 });
        tokio::time::sleep(interval).await;
    }
}

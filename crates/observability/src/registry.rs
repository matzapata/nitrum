//! In-process metric registry (counters and gauges) flushed periodically as EMF.

use std::collections::HashMap;
use std::sync::Mutex;

/// Kind of metric stored in the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricKind {
    /// Monotonically increasing value since process start; reset on flush.
    Counter,
    /// Last-written value; retained across flushes until overwritten.
    Gauge,
}

/// Unique key for a metric name plus optional dimensions.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MetricKey {
    /// CloudWatch metric name.
    pub name: String,
    /// Extra dimensions beyond the standard ProjectName / Component / InstanceId set.
    pub dims: Vec<(String, String)>,
    pub kind: MetricKind,
}

/// Snapshot of one metric at flush time.
pub struct MetricSample {
    /// Metric name.
    pub name: String,
    /// Extra dimensions.
    pub dims: Vec<(String, String)>,
    /// Observed numeric value.
    pub value: f64,
    pub kind: MetricKind,
}

/// Thread-safe registry of counters and gauges.
pub struct MetricsRegistry {
    inner: Mutex<HashMap<MetricKey, f64>>,
}

impl MetricsRegistry {
    /// Empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Increment a counter by `delta` (default dimensions only).
    pub fn counter(&self, name: &str, delta: u64) {
        self.counter_dims(name, delta, &[]);
    }

    /// Increment a counter with extra dimensions.
    pub fn counter_dims(&self, name: &str, delta: u64, dims: &[(&str, &str)]) {
        let key = MetricKey {
            name: name.to_string(),
            dims: dims
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            kind: MetricKind::Counter,
        };
        let mut guard = self.inner.lock().expect("metrics registry lock");
        *guard.entry(key).or_insert(0.0) += delta as f64;
    }

    /// Set a gauge (default dimensions only).
    pub fn gauge(&self, name: &str, value: f64) {
        self.gauge_dims(name, value, &[]);
    }

    /// Set a gauge with extra dimensions.
    pub fn gauge_dims(&self, name: &str, value: f64, dims: &[(&str, &str)]) {
        let key = MetricKey {
            name: name.to_string(),
            dims: dims
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            kind: MetricKind::Gauge,
        };
        let mut guard = self.inner.lock().expect("metrics registry lock");
        guard.insert(key, value);
    }

    /// Copy current values without resetting counters (for Prometheus scrape).
    #[must_use]
    pub fn snapshot(&self) -> Vec<MetricSample> {
        let guard = self.inner.lock().expect("metrics registry lock");
        guard
            .iter()
            .filter(|(_, v)| **v != 0.0)
            .map(|(key, value)| MetricSample {
                name: key.name.clone(),
                dims: key.dims.clone(),
                value: *value,
                kind: key.kind,
            })
            .collect()
    }

    /// Drain counters (reset to zero) and copy gauges for EMF emission.
    pub fn take_samples(&self) -> Vec<MetricSample> {
        let mut guard = self.inner.lock().expect("metrics registry lock");
        let mut out = Vec::with_capacity(guard.len());
        guard.retain(|key, value| {
            if *value == 0.0 && key.kind == MetricKind::Counter {
                return false;
            }
            out.push(MetricSample {
                name: key.name.clone(),
                dims: key.dims.clone(),
                value: *value,
                kind: key.kind,
            });
            key.kind == MetricKind::Gauge
        });
        out
    }
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

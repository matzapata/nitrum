//! CloudWatch Embedded Metric Format (EMF) JSON builder.

use crate::registry::{MetricKind, MetricSample};
use serde_json::{Map, Value};

/// Standard dimension names included on every Nitrum metric series.
pub const DIM_PROJECT: &str = "ProjectName";
pub const DIM_COMPONENT: &str = "Component";
pub const DIM_INSTANCE: &str = "InstanceId";

/// EMF unit strings for CloudWatch.
const fn unit_for_kind(kind: MetricKind) -> &'static str {
    match kind {
        MetricKind::Counter => "Count",
        MetricKind::Gauge => "None",
    }
}

/// Build one EMF log line (JSON object) for the given samples.
///
/// Groups metrics that share the same extra-dimension set into one `CloudWatchMetrics` entry.
pub fn build_emf_document(
    namespace: &str,
    project: &str,
    component: &str,
    instance_id: &str,
    samples: &[MetricSample],
) -> Option<String> {
    if samples.is_empty() {
        return None;
    }

    let timestamp_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());

    let mut root = Map::new();
    root.insert(
        DIM_PROJECT.to_string(),
        Value::String(project.to_string()),
    );
    root.insert(
        DIM_COMPONENT.to_string(),
        Value::String(component.to_string()),
    );
    root.insert(
        DIM_INSTANCE.to_string(),
        Value::String(instance_id.to_string()),
    );

    for sample in samples {
        for (k, v) in &sample.dims {
            root.insert(k.clone(), Value::String(v.clone()));
        }
        root.insert(
            sample.name.clone(),
            serde_json::Number::from_f64(sample.value)
                .map_or(Value::Null, Value::Number),
        );
    }

    let mut groups: Vec<(Vec<String>, Vec<&MetricSample>)> = Vec::new();
    for sample in samples {
        let mut dim_names = vec![
            DIM_PROJECT.to_string(),
            DIM_COMPONENT.to_string(),
            DIM_INSTANCE.to_string(),
        ];
        for (k, _) in &sample.dims {
            if !dim_names.contains(k) {
                dim_names.push(k.clone());
            }
        }
        if let Some(g) = groups.iter_mut().find(|(names, _)| *names == dim_names) {
            g.1.push(sample);
        } else {
            groups.push((dim_names, vec![sample]));
        }
    }

    let mut cw_metrics = Vec::new();
    for (dim_names, group_samples) in groups {
        let dim_name_refs: Vec<&str> = dim_names.iter().map(String::as_str).collect();
        let metrics: Vec<Value> = group_samples
            .iter()
            .map(|s| {
                serde_json::json!({
                    "Name": s.name,
                    "Unit": unit_for_kind(s.kind),
                })
            })
            .collect();
        cw_metrics.push(serde_json::json!({
            "Namespace": namespace,
            "Dimensions": [dim_name_refs],
            "Metrics": metrics,
        }));
    }

    root.insert(
        "_aws".to_string(),
        serde_json::json!({
            "Timestamp": timestamp_ms,
            "CloudWatchMetrics": cw_metrics,
        }),
    );

    serde_json::to_string(&Value::Object(root)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{MetricKind, MetricSample};

    #[test]
    fn emf_includes_namespace_and_metric_values() {
        let samples = vec![MetricSample {
            name: "EnclaveRunning".to_string(),
            dims: vec![],
            value: 1.0,
            kind: MetricKind::Gauge,
        }];
        let json = build_emf_document("Nitrum/demo", "demo", "control-plane", "i-abc", &samples)
            .expect("emf json");
        assert!(json.contains("Nitrum/demo"));
        assert!(json.contains("\"EnclaveRunning\":1"));
        assert!(json.contains("_aws"));
    }
}

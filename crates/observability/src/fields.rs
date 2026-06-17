//! Custom `fmt` formatting that injects `project` / `component` and redacts sensitive values.

use crate::Component;
use crate::format::LogFormat;
use crate::redact::redact_field;
use crate::span::{COMPONENT, PROJECT};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing_subscriber::fmt::format::{FormatFields, Writer};
use tracing_subscriber::fmt::{FmtContext, FormatEvent};
use tracing_subscriber::registry::LookupSpan;

/// Event formatter that adds Nitrum default fields and applies redaction.
pub struct NitrumEventFormat {
    /// Project name on every event.
    project: Arc<str>,
    /// Default component when the event target is not `app`.
    default_component: Component,
    /// EC2 instance id (or `local`) for multi-replica log correlation.
    instance_id: Arc<str>,
    /// Underlying JSON or human formatter.
    log_format: LogFormat,
}

impl NitrumEventFormat {
    pub fn new(
        project: impl Into<Arc<str>>,
        default_component: Component,
        instance_id: impl Into<Arc<str>>,
        log_format: LogFormat,
    ) -> Self {
        Self {
            project: project.into(),
            default_component,
            instance_id: instance_id.into(),
            log_format,
        }
    }

    fn component_for_event(&self, target: &str) -> &'static str {
        if target == "app" {
            Component::App.as_str()
        } else {
            self.default_component.as_str()
        }
    }
}

struct FieldCollector {
    fields: BTreeMap<String, String>,
}

impl Visit for FieldCollector {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let name = field.name().to_string();
        let rendered = format!("{value:?}");
        let out = redact_field(&name, &rendered);
        self.fields.insert(name, out);
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        let name = field.name().to_string();
        let out = redact_field(&name, value);
        self.fields.insert(name, out);
    }
}

impl<S, N> FormatEvent<S, N> for NitrumEventFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> fmt::Result {
        let target = event.metadata().target();
        let component = self.component_for_event(target);
        let mut collector = FieldCollector {
            fields: BTreeMap::new(),
        };
        event.record(&mut collector);

        match self.log_format {
            LogFormat::Json => {
                let mut map = serde_json::Map::new();
                map.insert(
                    "timestamp".to_string(),
                    serde_json::Value::String(timestamp_rfc3339()),
                );
                map.insert(
                    "level".to_string(),
                    serde_json::Value::String(event.metadata().level().to_string()),
                );
                map.insert(
                    "target".to_string(),
                    serde_json::Value::String(target.to_string()),
                );
                map.insert(
                    PROJECT.to_string(),
                    serde_json::Value::String(self.project.to_string()),
                );
                map.insert(
                    COMPONENT.to_string(),
                    serde_json::Value::String(component.to_string()),
                );
                map.insert(
                    "instance_id".to_string(),
                    serde_json::Value::String(self.instance_id.to_string()),
                );
                for (k, v) in collector.fields {
                    map.insert(k, serde_json::Value::String(v));
                }
                if let Ok(line) = serde_json::to_string(&serde_json::Value::Object(map)) {
                    writeln!(writer, "{line}")?;
                }
            }
            LogFormat::Human => {
                write!(
                    writer,
                    "{} {PROJECT}={} {COMPONENT}={component} instance_id={} target={target} ",
                    event.metadata().level(),
                    self.project,
                    self.instance_id,
                )?;
                ctx.field_format().format_fields(writer.by_ref(), event)?;
                writeln!(writer)?;
            }
        }
        Ok(())
    }
}

fn timestamp_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}Z", dur.as_secs(), dur.subsec_millis())
}

/// Field formatter that redacts values for human mode span/event fields.
pub struct RedactingFieldFormatter;

impl<'writer> FormatFields<'writer> for RedactingFieldFormatter {
    fn format_fields<R: tracing_subscriber::field::RecordFields>(
        &self,
        writer: Writer<'writer>,
        fields: R,
    ) -> fmt::Result {
        struct Visitor<'writer> {
            writer: Writer<'writer>,
            first: bool,
        }

        impl Visit for Visitor<'_> {
            fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
                if !self.first {
                    let _ = write!(self.writer, " ");
                }
                self.first = false;
                let name = field.name();
                let rendered = format!("{value:?}");
                let out = redact_field(name, &rendered);
                let _ = write!(self.writer, "{name}={out}");
            }

            fn record_str(&mut self, field: &Field, value: &str) {
                if !self.first {
                    let _ = write!(self.writer, " ");
                }
                self.first = false;
                let name = field.name();
                let out = redact_field(name, value);
                let _ = write!(self.writer, "{name}={out}");
            }
        }

        let mut visitor = Visitor {
            writer,
            first: true,
        };
        fields.record(&mut visitor);
        Ok(())
    }
}

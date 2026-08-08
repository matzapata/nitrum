//! OpenTelemetry environment variables injected into the user application process.

use crate::DataPlaneConfig;
use std::collections::HashMap;
use telemetry::env::{
    NAMESPACE, OTEL_EXPORTER_OTLP_ENDPOINT, OTEL_EXPORTER_OTLP_PROTOCOL, OTEL_RESOURCE_ATTRIBUTES,
    OTEL_SERVICE_NAME, OTLP_PROTOCOL_GRPC, USER_APP_COMPONENT,
};

/// Whether OTLP export is enabled for the given collector endpoint.
#[must_use]
pub fn otlp_export_enabled(endpoint: Option<&str>) -> bool {
    endpoint.is_some_and(|value| !value.is_empty())
}

/// Build the merged environment for the user process: SSM [`DataPlaneConfig::user_env`]
/// plus Nitrum-injected OpenTelemetry variables when export is enabled.
///
/// Injected OTel values override same-named keys from SSM so the platform collector
/// endpoint cannot be redirected accidentally. Returns the merged environment plus
/// whether OpenTelemetry variables were injected.
#[must_use]
pub fn build_user_process_env(config: &DataPlaneConfig) -> (HashMap<String, String>, bool) {
    let mut env = config.user_env.clone();
    let injected = user_otel_env(config).is_some_and(|otel| {
        env.extend(otel);
        true
    });
    (env, injected)
}

/// OpenTelemetry variables for the user app, or `None` when OTLP export is disabled.
#[must_use]
pub fn user_otel_env(config: &DataPlaneConfig) -> Option<HashMap<String, String>> {
    if !otlp_export_enabled(config.otlp_endpoint.as_deref()) {
        return None;
    }
    let endpoint = config.otlp_endpoint.as_deref()?;

    let mut env = HashMap::new();
    env.insert(
        OTEL_EXPORTER_OTLP_ENDPOINT.to_string(),
        endpoint.to_string(),
    );
    env.insert(
        OTEL_EXPORTER_OTLP_PROTOCOL.to_string(),
        OTLP_PROTOCOL_GRPC.to_string(),
    );
    env.insert(
        OTEL_SERVICE_NAME.to_string(),
        config.project.name.to_string(),
    );
    env.insert(
        OTEL_RESOURCE_ATTRIBUTES.to_string(),
        format!("nitrum.component={USER_APP_COMPONENT},service.namespace={NAMESPACE}"),
    );
    Some(env)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ListenAddrs;
    use aws_config::BehaviorVersion;
    use aws_config::Region;
    use config::NitrumConfig;
    use config::PlatformLayout;
    use std::net::SocketAddr;
    use std::sync::Arc;

    fn test_config(otlp_endpoint: Option<&str>, project_name: &str) -> DataPlaneConfig {
        let aws = Arc::new(
            aws_config::SdkConfig::builder()
                .behavior_version(BehaviorVersion::latest())
                .region(Region::new("us-east-1"))
                .build(),
        );

        let layout = PlatformLayout::new(project_name.parse().expect("valid test project name"));

        DataPlaneConfig {
            nitrum: NitrumConfig {
                project: config::Project {
                    name: project_name.parse().expect("valid test project name"),
                    port: std::num::NonZeroU16::new(8080).expect("8080 is non-zero"),
                    start_command: vec![],
                },
                runtime: config::Runtime::default(),
                health_check: config::HealthCheck::default(),
                scaling: config::Scaling::default(),
                tls_termination: config::TlsTermination::default(),
                egress: config::Egress::default(),
                cloud: config::Cloud::default(),
            },
            layout,
            aws,
            imds_base_url: "http://127.0.0.1/latest".to_string(),
            instance_id: "i-test".to_string(),
            dynamodb_table: "table".to_string(),
            kms_key_id: "key".to_string(),
            listen_addrs: ListenAddrs {
                ingress_listen_addr: "0.0.0.0:443".parse::<SocketAddr>().unwrap(),
                acme_http01_listen_addr: "0.0.0.0:80".parse::<SocketAddr>().unwrap(),
                crypto_api_listen_addr: "0.0.0.0:3000".parse::<SocketAddr>().unwrap(),
            },
            user_env: HashMap::from([("DEMO".to_string(), "hello".to_string())]),
            otlp_endpoint: otlp_endpoint.map(str::to_string),
        }
    }

    #[test]
    fn otlp_export_enabled_respects_empty_endpoint() {
        assert!(!otlp_export_enabled(Some("")));
        assert!(otlp_export_enabled(Some("http://127.0.0.1:4317")));
        assert!(!otlp_export_enabled(None));
    }

    #[test]
    fn user_otel_env_uses_project_name_and_core_namespace() {
        let env = user_otel_env(&test_config(Some("http://10.0.0.1:4317"), "nitrum-hello"))
            .expect("otel env");

        assert_eq!(
            env.get(OTEL_EXPORTER_OTLP_ENDPOINT),
            Some(&"http://10.0.0.1:4317".to_string())
        );
        assert_eq!(
            env.get(OTEL_EXPORTER_OTLP_PROTOCOL),
            Some(&"grpc".to_string())
        );
        assert_eq!(
            env.get(OTEL_SERVICE_NAME),
            Some(&"nitrum-hello".to_string())
        );
        assert_eq!(
            env.get(OTEL_RESOURCE_ATTRIBUTES),
            Some(&"nitrum.component=user-app,service.namespace=nitrum".to_string())
        );
    }

    #[test]
    fn build_user_process_env_overrides_conflicting_ssm_keys() {
        let mut config = test_config(Some("http://10.0.0.1:4317"), "nitrum-hello");
        config
            .user_env
            .insert(OTEL_SERVICE_NAME.to_string(), "override-me".to_string());

        let (env, injected) = build_user_process_env(&config);
        assert!(injected);
        assert_eq!(env.get("DEMO"), Some(&"hello".to_string()));
        assert_eq!(
            env.get(OTEL_SERVICE_NAME),
            Some(&"nitrum-hello".to_string())
        );
    }

    #[test]
    fn build_user_process_env_reports_no_injection_when_export_disabled() {
        let config = test_config(Some(""), "nitrum-hello");
        let (env, injected) = build_user_process_env(&config);
        assert!(!injected);
        assert_eq!(env.get("DEMO"), Some(&"hello".to_string()));
    }

    #[test]
    fn user_otel_env_none_when_export_disabled() {
        assert!(user_otel_env(&test_config(Some(""), "nitrum-hello")).is_none());
    }
}

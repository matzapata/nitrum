//! Offline helpers for benchmarks and integration tests.
//!
//! Builds inert [`RuntimeConfig`] values without live AWS or IMDS. Not for production startup.

use crate::config::RuntimeConfig;
use aws_config::BehaviorVersion;
use aws_config::Region;
use config::NitrumConfig;
use std::collections::HashMap;
use std::sync::Arc;

/// Build a [`RuntimeConfig`] with inert infra placeholders for offline benchmarks.
#[doc(hidden)]
pub fn runtime_config(nitrum: NitrumConfig) -> RuntimeConfig {
    let aws_sdk_config = Arc::new(
        aws_config::SdkConfig::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .build(),
    );

    RuntimeConfig {
        nitrum,
        imds_latest_base_url: "http://127.0.0.1".to_string(),
        aws_region: "us-east-1".to_string(),
        aws_sdk_config,
        instance_id: "i-bench".to_string(),
        dynamodb_table: "bench-table".to_string(),
        dynamodb_endpoint: None,
        kms_key_id: "bench-kms-key".to_string(),
        kms_endpoint: None,
        ingress_listen_addr: "127.0.0.1:443".parse().expect("valid ingress listen addr"),
        acme_http01_listen_addr: "127.0.0.1:80".parse().expect("valid acme listen addr"),
        crypto_api_listen_addr: "127.0.0.1:3000"
            .parse()
            .expect("valid crypto api listen addr"),
        user_env: HashMap::new(),
    }
}

/// Set the backend port on an existing [`RuntimeConfig`].
#[doc(hidden)]
pub fn with_backend_port(mut config: RuntimeConfig, port: u16) -> RuntimeConfig {
    config.nitrum.project.port = port;
    config
}

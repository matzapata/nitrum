//! Shared helpers for data-plane benchmarks.
//!
//! Builds inert [`DataPlaneConfig`] values without live AWS or IMDS.

use aws_config::BehaviorVersion;
use aws_config::Region;
use config::{NitrumConfig, PlatformLayout};
use data_plane::{DataPlaneConfig, ListenAddrs};
use std::collections::HashMap;
use std::sync::Arc;

/// Build a [`DataPlaneConfig`] with inert infra placeholders for offline benchmarks.
#[must_use]
pub fn data_plane_config(nitrum: NitrumConfig) -> DataPlaneConfig {
    let aws = Arc::new(
        aws_config::SdkConfig::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .build(),
    );

    let layout = PlatformLayout::from_project(&nitrum.project);

    DataPlaneConfig {
        nitrum,
        layout,
        aws,
        imds_base_url: "http://127.0.0.1/latest".to_string(),
        instance_id: "i-bench".to_string(),
        dynamodb_table: "bench-table".to_string(),
        kms_key_id: "bench-kms-key".to_string(),
        listen_addrs: ListenAddrs {
            ingress_listen_addr: "127.0.0.1:443".parse().expect("valid ingress listen addr"),
            acme_http01_listen_addr: "127.0.0.1:80".parse().expect("valid acme listen addr"),
            crypto_api_listen_addr: "127.0.0.1:3000"
                .parse()
                .expect("valid crypto api listen addr"),
        },
        user_env: HashMap::new(),
        otlp_endpoint: None,
    }
}

/// Set the backend port on an existing [`DataPlaneConfig`].
#[must_use]
pub const fn with_backend_port(mut config: DataPlaneConfig, port: u16) -> DataPlaneConfig {
    config.nitrum.project.port = port;
    config
}

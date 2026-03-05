use std::sync::Arc;

use tracing::info;

mod api;
mod attestation;
mod config;
mod constants;
mod dns_proxy;
mod egress;
mod ingress;
mod tcp_proxy;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = config::load();

    let egress_filter = Arc::new(egress::EgressFilter::new(
        config.egress.enabled,
        &config.egress.whitelist,
    ));

    let app_port = std::env::var("APP_PORT").unwrap_or_else(|_| "8008".to_string());
    let app_addr = format!("127.0.0.1:{app_port}");

    let api_port = std::env::var("NITRUM_API_PORT").unwrap_or_else(|_| "3000".to_string());
    let api_addr = format!("127.0.0.1:{api_port}");

    info!("data-plane starting");

    tokio::join!(
        tcp_proxy::run(Arc::clone(&egress_filter)),
        dns_proxy::run(Arc::clone(&egress_filter)),
        ingress::run(app_addr, config.tls_termination),
        api::run(api_addr),
    );
}

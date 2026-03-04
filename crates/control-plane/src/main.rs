use tracing::info;

mod constants;
mod dns_proxy;
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

    let upstream_dns = std::env::var("DNS_UPSTREAM").unwrap_or_else(|_| "8.8.8.8:53".to_string());

    let ingress_port = std::env::var("INGRESS_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(constants::INGRESS_PORT);

    info!("control-plane starting");

    tokio::join!(
        tcp_proxy::run(),
        dns_proxy::run(upstream_dns),
        ingress::run(ingress_port),
    );
}

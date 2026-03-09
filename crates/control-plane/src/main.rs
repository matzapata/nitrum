use tracing::info;

mod constants;
mod egress;
mod ingress;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let upstream_dns = std::env::var("DNS_UPSTREAM").unwrap_or(constants::DNS_UPSTREAM.to_string());

    let ingress_port = std::env::var("INGRESS_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(constants::INGRESS_PORT);

    tokio::join!(
        egress::tcp::run(),
        egress::dns::run(upstream_dns),
        ingress::tcp::run(ingress_port),
    );
}

use clap::Parser;
use tracing::info;

mod networking;

#[derive(Parser)]
#[command(name = "data-plane")]
struct Args {
    /// VSOCK port where gvproxy listens on the host (CID 3).
    #[arg(long, default_value_t = 1024)]
    host_proxy_port: u32,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    networking::setup(args.host_proxy_port);

    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    info!("testing connectivity via gvproxy …");
    match test_connectivity().await {
        Ok(body) => info!("httpbin.org/ip response:\n{body}"),
        Err(e) => tracing::error!("connectivity test failed: {e}"),
    }

    info!("networking is up — keeping process alive");
    std::future::pending::<()>().await;
}

async fn test_connectivity() -> Result<String, Box<dyn std::error::Error>> {
    let client = reqwest::Client::builder()
        .user_agent("nitrum-data-plane/0.1")
        .build()?;

    let response = client
        .get("http://httpbin.org/ip")
        .send()
        .await?
        .error_for_status()?;

    let body = response.text().await?;
    Ok(body)
}

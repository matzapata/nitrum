//! EC2 Instance Metadata Service (IMDSv2) helpers.
//!
//! Inside a Nitro Enclave, IMDS is proxied through vsock-proxy using the
//! allowlist entry for 169.254.169.254.
//! TODO: test this all, also get session for dynamodb table, etc

use anyhow::{Context, Result};

const IMDS_BASE: &str = "http://169.254.169.254/latest";
const TOKEN_TTL_SECONDS: &str = "21600";

/// Fetch a short-lived IMDSv2 session token.
async fn get_token(client: &reqwest::Client) -> Result<String> {
    client
        .put(format!("{IMDS_BASE}/api/token"))
        .header("X-aws-ec2-metadata-token-ttl-seconds", TOKEN_TTL_SECONDS)
        .send()
        .await
        .context("IMDSv2 token request failed")?
        .text()
        .await
        .context("failed to read IMDSv2 token body")
}

// TODO: get dynamo table and etc, etc
/// Returns the AWS region this instance is running in.
pub async fn get_region() -> Result<String> {
    // Prefer the environment variable so local dev works without a real IMDS.
    if let Ok(r) = std::env::var("AWS_REGION").or_else(|_| std::env::var("AWS_DEFAULT_REGION")) {
        return Ok(r);
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .context("failed to build HTTP client")?;

    let token = get_token(&client).await?;

    let region = client
        .get(format!("{IMDS_BASE}/meta-data/placement/region"))
        .header("X-aws-ec2-metadata-token", &token)
        .send()
        .await
        .context("IMDS region request failed")?
        .text()
        .await
        .context("failed to read IMDS region body")?;

    Ok(region)
}

/// Returns a unique instance ID for this process (leader lock owner).
/// Tries IMDS instance-id first, then NITRUM_INSTANCE_ID env, then a local fallback.
pub async fn instance_id() -> Result<String> {
    if let Ok(id) = std::env::var("NITRUM_INSTANCE_ID") {
        return Ok(id);
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .context("failed to build HTTP client for IMDS")?;

    let token = get_token(&client).await?;

    let id = client
        .get(format!("{IMDS_BASE}/meta-data/instance-id"))
        .header("X-aws-ec2-metadata-token", &token)
        .send()
        .await
        .context("IMDS instance-id request failed")?
        .text()
        .await
        .context("failed to read IMDS instance-id body")?;

    Ok(id)
}

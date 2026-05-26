//! CloudWatch Logs export via `tracing-cloudwatch`.

use anyhow::Result;
use aws_sdk_cloudwatchlogs::Client;

/// Creates the log stream if it does not already exist.
pub async fn ensure_log_stream(client: &Client, group: &str, stream: &str) -> Result<()> {
    let out = client
        .create_log_stream()
        .log_group_name(group)
        .log_stream_name(stream)
        .send()
        .await;
    match out {
        Ok(_) => Ok(()),
        Err(e) => {
            if format!("{e:?}").contains("ResourceAlreadyExistsException") {
                Ok(())
            } else {
                Err(anyhow::anyhow!("{e}"))
            }
        }
    }
}

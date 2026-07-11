//! SSM access via [`GetParameter`](https://docs.aws.amazon.com/systems-manager/latest/APIReference/API_GetParameter.html)
//! and [`GetParametersByPath`](https://docs.aws.amazon.com/systems-manager/latest/APIReference/API_GetParametersByPath.html).
//!
//! [`SsmParameters`] reuses the shared [`aws_config::SdkConfig`] built during runtime bootstrap.

use crate::constants::ssm_endpoint_url;
use anyhow::{Context, Result};
use aws_sdk_ssm::Client;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::debug;
/// Batch SSM loader tied to the shared runtime [`aws_config::SdkConfig`].
#[derive(Clone, Debug)]
pub struct SsmParameters {
    sdk_config: Arc<aws_config::SdkConfig>,
}

impl SsmParameters {
    pub const fn new(sdk_config: Arc<aws_config::SdkConfig>) -> Self {
        Self { sdk_config }
    }

    fn ssm_client(&self) -> Client {
        let mut builder = aws_sdk_ssm::config::Builder::from(self.sdk_config.as_ref());
        if let Some(url) = ssm_endpoint_url() {
            builder = builder.endpoint_url(url);
        }
        Client::from_conf(builder.build())
    }

    /// Single-parameter fetch (infra keys are plain `String`, not `SecureString`).
    pub async fn get_parameter(&self, name: &str) -> Result<String> {
        let client = self.ssm_client();
        let out = client
            .get_parameter()
            .name(name)
            .with_decryption(false)
            .send()
            .await
            .with_context(|| format!("SSM GetParameter {name}"))?;
        let Some(p) = out.parameter() else {
            anyhow::bail!("SSM parameter not found: {name}");
        };
        let value = p.value().unwrap_or("");
        if value.is_empty() {
            anyhow::bail!("SSM parameter empty: {name}");
        }
        Ok(value.to_string())
    }

    /// Load all parameters under `path` (recursive) with decryption. Maps **last path segment** → value.
    pub async fn get_parameters_by_path_recursive(
        &self,
        path: &str,
    ) -> Result<HashMap<String, String>> {
        let path = path.trim_end_matches('/');
        if path.is_empty() {
            return Ok(HashMap::new());
        }

        let client = self.ssm_client();
        let mut map = HashMap::new();
        let mut next_token = None::<String>;

        loop {
            let mut req = client
                .get_parameters_by_path()
                .path(path)
                .recursive(true)
                .with_decryption(true);
            if let Some(ref t) = next_token {
                req = req.next_token(t);
            }
            let out = req.send().await.context("SSM GetParametersByPath failed")?;

            for p in out.parameters() {
                let name = p.name().unwrap_or("");
                let value = p.value().unwrap_or("");
                if name.is_empty() || value.is_empty() {
                    continue;
                }
                let key = parameter_name_to_env_key(name);
                map.insert(key, value.to_string());
            }

            next_token = out.next_token().map(str::to_string);
            if next_token.is_none() {
                break;
            }
        }

        debug!(keys = ?map.keys().collect::<Vec<_>>(), "SSM app env by path");
        Ok(map)
    }
}

/// Last path segment of an SSM parameter name, used as the child process env var name.
#[must_use]
pub fn parameter_name_to_env_key(name: &str) -> String {
    name.split('/').next_back().unwrap_or(name).to_string()
}

#[cfg(test)]
mod tests {
    use super::parameter_name_to_env_key;

    #[test]
    fn last_segment_env_key() {
        assert_eq!(
            parameter_name_to_env_key("/nitrum/nitrum-app/env/DATABASE_URL"),
            "DATABASE_URL"
        );
        assert_eq!(parameter_name_to_env_key("SIMPLE"), "SIMPLE");
    }
}

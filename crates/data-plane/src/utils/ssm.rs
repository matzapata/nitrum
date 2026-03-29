//! SSM access via [`GetParameter`](https://docs.aws.amazon.com/systems-manager/latest/APIReference/API_GetParameter.html)
//! and [`GetParametersByPath`](https://docs.aws.amazon.com/systems-manager/latest/APIReference/API_GetParametersByPath.html).
//!
//! [`SsmParameters`] holds an [`Arc<ImdsClient>`](crate::utils::imds::ImdsClient): region and
//! SigV4 credentials for SSM come from the same IMDS-backed [`EnclaveProvider`] as the rest of the app.

use std::sync::Arc;

use anyhow::{Context, Result};
use aws_config::BehaviorVersion;
use aws_config::Region;
use aws_credential_types::provider::SharedCredentialsProvider;
use aws_sdk_ssm::Client;
use std::collections::HashMap;
use tracing::debug;

use super::imds::{EnclaveProvider, ImdsClient};

/// Last path segment of an SSM parameter name, used as the child process env var name.
#[must_use]
pub fn parameter_name_to_env_key(name: &str) -> String {
    name.split('/').next_back().unwrap_or(name).to_string()
}

/// Batch SSM loader tied to a shared [`ImdsClient`] (same IMDS session / credential cache as the rest of the data-plane).
#[derive(Clone, Debug)]
pub struct SsmParameters {
    imds: Arc<ImdsClient>,
}

impl SsmParameters {
    pub fn new(imds: Arc<ImdsClient>) -> Self {
        Self { imds }
    }

    async fn ssm_client(&self) -> Result<Client> {
        // TODO: sdk_config should be built in different module and shared
        let region = self
            .imds
            .get_region()
            .await
            .context("IMDS placement region for SSM")?;
        let sdk_config = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(region))
            .credentials_provider(SharedCredentialsProvider::new(EnclaveProvider::with_imds(
                self.imds.clone(),
            )))
            .load()
            .await;

        let mut builder = aws_sdk_ssm::config::Builder::from(&sdk_config);
        if let Some(url) = std::env::var("NITRUM_SSM_ENDPOINT_URL")
            .ok()
            .filter(|s| !s.is_empty())
        {
            builder = builder.endpoint_url(url);
        }
        Ok(Client::from_conf(builder.build()))
    }

    /// Single-parameter fetch (infra keys are plain `String`, not `SecureString`).
    pub async fn get_parameter(&self, name: &str) -> Result<String> {
        let client = self.ssm_client().await?;
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

        let client = self.ssm_client().await?;
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

//! SSM parameter batch load via [`GetParameters`](https://docs.aws.amazon.com/systems-manager/latest/APIReference/API_GetParameters.html).
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

/// Batch SSM loader tied to a shared [`ImdsClient`] (same IMDS session / credential cache as the rest of the data-plane).
#[derive(Clone, Debug)]
pub struct SsmParameters {
    imds: Arc<ImdsClient>,
}

impl SsmParameters {
    pub fn new(imds: Arc<ImdsClient>) -> Self {
        Self { imds }
    }

    /// Comma-separated override (`NITRUM_SSM_PARAMETER_NAMES`) or default Nitrum paths.
    pub fn parameter_names() -> Vec<String> {
        if let Ok(names) = std::env::var("NITRUM_SSM_PARAMETER_NAMES") {
            let v: Vec<String> = names
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !v.is_empty() {
                return v;
            }
        }

        let kms = std::env::var("NITRUM_SSM_PARAM_KMS_KEY_ID")
            .ok()
            .filter(|s| !s.is_empty());
        let ddb = std::env::var("NITRUM_SSM_PARAM_DYNAMODB_TABLE")
            .ok()
            .filter(|s| !s.is_empty());
        match (kms, ddb) {
            (Some(k), Some(d)) => vec![k, d],
            _ => vec![
                    "/nitrum/kms_key_id".to_string(),
                    "/nitrum/dynamodb_table".to_string(),
            ],
        }
    }

    /// Load parameters and map **last path segment** → value (e.g. `kms_key_id`, `dynamodb_table`).
    pub async fn get_parameters_as_map(&self) -> Result<HashMap<String, String>> {
        let region = self
            .imds
            .get_region()
            .await
            .context("IMDS placement region for SSM")?;
        // TODO: check this, we can pass sdk_condf as param instead
        let sdk_config = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(region))
            .credentials_provider(SharedCredentialsProvider::new(EnclaveProvider::with_imds(
                self.imds.clone(),
            )))
            .load()
            .await;

        let names = Self::parameter_names();
        let mut builder = aws_sdk_ssm::config::Builder::from(&sdk_config);
        if let Some(url) = std::env::var("NITRUM_SSM_ENDPOINT_URL")
            .ok()
            .filter(|s| !s.is_empty())
        {
            builder = builder.endpoint_url(url);
        }
        let client = Client::from_conf(builder.build());

        let out = client
            .get_parameters()
            .set_names(Some(names.clone()))
            .with_decryption(false)
            .send()
            .await
            .context("SSM GetParameters failed")?;

        let invalid = out.invalid_parameters();
        if !invalid.is_empty() {
            debug!(?invalid, "SSM invalid or missing parameter names");
        }

        let mut map = HashMap::new();
        for p in out.parameters() {
            let name = p.name().unwrap_or("");
            let value = p.value().unwrap_or("");
            if name.is_empty() || value.is_empty() {
                continue;
            }
            let key = name.split('/').next_back().unwrap_or(name).to_string();
            map.insert(key, value.to_string());
        }

        debug!(keys = ?map.keys().collect::<Vec<_>>(), "SSM parameter refresh");
        Ok(map)
    }
}

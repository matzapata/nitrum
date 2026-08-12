//! Thin AWS SSM Parameter Store client wrapper.

use anyhow::{Context, Result};
use aws_sdk_ssm::types::ParameterType;

/// SSM client built from [`aws_config::load_from_env`].
pub struct Ssm {
    client: aws_sdk_ssm::Client,
}

impl Ssm {
    /// Build an SSM client from the default AWS configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when `aws_config::load_from_env` fails.
    pub async fn new() -> Result<Self> {
        let aws_sdk_config = aws_config::load_from_env().await;
        Ok(Self {
            client: aws_sdk_ssm::Client::new(&aws_sdk_config),
        })
    }

    /// `PutParameter` as `SecureString` with overwrite.
    ///
    /// # Errors
    ///
    /// Returns an error when the SSM `PutParameter` call fails.
    pub async fn set(&self, name: &str, value: impl Into<String>) -> Result<()> {
        self.client
            .put_parameter()
            .name(name)
            .value(value.into())
            .r#type(ParameterType::SecureString)
            .overwrite(true)
            .send()
            .await
            .with_context(|| format!("ssm put-parameter {name}"))?;
        Ok(())
    }

    /// `GetParameter` with decryption. Returns `None` if the parameter does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error when the SSM `GetParameter` call fails.
    pub async fn get(&self, name: &str) -> Result<Option<String>> {
        let out = self
            .client
            .get_parameter()
            .name(name)
            .with_decryption(true)
            .send()
            .await
            .with_context(|| format!("ssm get-parameter {name}"))?;
        Ok(out
            .parameter()
            .and_then(|p| p.value().map(std::string::ToString::to_string)))
    }

    /// `GetParametersByPath` with recursion and decryption. Each item is `(last_path_segment, value)`.
    ///
    /// # Errors
    ///
    /// Returns an error when the SSM `GetParametersByPath` call fails.
    pub async fn list(&self, path: &str) -> Result<Vec<(String, String)>> {
        let path = path.trim_end_matches('/');
        let mut rows = Vec::new();
        let mut next_token = None::<String>;

        loop {
            let mut req = self
                .client
                .get_parameters_by_path()
                .path(path)
                .recursive(true)
                .with_decryption(true);
            if let Some(ref t) = next_token {
                req = req.next_token(t);
            }
            let out = req
                .send()
                .await
                .with_context(|| format!("ssm get-parameters-by-path {path}"))?;

            for p in out.parameters() {
                let name = p.name().unwrap_or("");
                let value = p.value().unwrap_or("");
                let key = name.split('/').next_back().unwrap_or(name).to_string();
                rows.push((key, value.to_string()));
            }

            next_token = out.next_token().map(str::to_string);
            if next_token.is_none() {
                break;
            }
        }

        Ok(rows)
    }

    /// `DeleteParameter`.
    ///
    /// # Errors
    ///
    /// Returns an error when the SSM `DeleteParameter` call fails.
    pub async fn delete(&self, name: &str) -> Result<()> {
        self.client
            .delete_parameter()
            .name(name)
            .send()
            .await
            .with_context(|| format!("ssm delete-parameter {name}"))?;
        Ok(())
    }
}

//! Generic DynamoDB-backed object storage.
//!
//! Table schema (single-table design, partition key = `pk: String`):
//!
//! | pk (S)   | value (B)     |
//! |----------|----------------|
//! | key      | opaque bytes   |
//!
//! Use [`keys`] for well-known object keys (DEK, cert, etc.).

use anyhow::{Context, Result};
use aws_sdk_dynamodb::{
    Client, error::SdkError, operation::put_item::PutItemError, primitives::Blob,
    types::AttributeValue,
};

use crate::config::RuntimeConfig;
use crate::utils::time;

const LOCK_TTL_SECS: u64 = 60;

// ── StorageClient ────────────────────────────────────────────────────────────

/// Client for the shared object storage table. Holds table name and DynamoDB client.
pub struct StorageClient {
    client: Client,
    table: String,
}

impl StorageClient {
    /// Build from runtime config and shared SDK config (DynamoDB encapsulated in storage).
    pub async fn new(config: &RuntimeConfig) -> Self {
        let sdk_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .load()
            .await;
        let mut builder = aws_sdk_dynamodb::config::Builder::from(&sdk_config);
        if let Some(ref endpoint) = config.dynamodb_endpoint {
            builder = builder.endpoint_url(endpoint);
        }
        let client = Client::from_conf(builder.build());
        Self {
            client,
            table: config.dynamodb_table.clone(),
        }
    }

    /// Try to acquire a distributed lock identified by `key`.
    ///
    /// Returns `true` if the lock was obtained, `false` if another instance holds it.
    /// The lock auto-expires after `LOCK_TTL_SECS` seconds via DynamoDB TTL.
    pub async fn try_acquire_lock(&self, key: &str, owner: &str) -> Result<bool> {
        let now = time::unix_now();
        let expiry = (now + LOCK_TTL_SECS).to_string();
        let now_str = now.to_string();

        let result = self
            .client
            .put_item()
            .table_name(&self.table)
            .item("pk", AttributeValue::S(key.to_string()))
            .item("owner", AttributeValue::S(owner.to_string()))
            .item("ttl", AttributeValue::N(expiry))
            .condition_expression("attribute_not_exists(pk) OR #t < :now")
            .expression_attribute_names("#t", "ttl")
            .expression_attribute_values(":now", AttributeValue::N(now_str))
            .send()
            .await;

        match result {
            Ok(_) => Ok(true),
            Err(e) if is_condition_failed(&e) => Ok(false),
            Err(e) => Err(e).context("DynamoDB try_acquire_lock failed"),
        }
    }

    /// Release a lock identified by `key` (owner-checked delete).
    pub async fn release_lock(&self, key: &str, owner: &str) -> Result<()> {
        self.client
            .delete_item()
            .table_name(&self.table)
            .key("pk", AttributeValue::S(key.to_string()))
            .condition_expression("#o = :owner")
            .expression_attribute_names("#o", "owner")
            .expression_attribute_values(":owner", AttributeValue::S(owner.to_string()))
            .send()
            .await
            .context("DynamoDB release_lock failed")?;
        Ok(())
    }

    /// Fetch an object by key. Returns `None` if the key does not exist.
    pub async fn get_object(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let resp = self
            .client
            .get_item()
            .table_name(&self.table)
            .key("pk", AttributeValue::S(key.to_string()))
            .send()
            .await
            .with_context(|| format!("DynamoDB get_item({key}) failed"))?;

        match resp.item {
            None => Ok(None),
            Some(mut item) => {
                let blob = item
                    .remove("value")
                    .with_context(|| format!("DynamoDB item({key}) missing 'value' attribute"))?;
                match blob {
                    AttributeValue::B(b) => Ok(Some(b.into_inner())),
                    other => anyhow::bail!("unexpected attribute type for '{key}': {other:?}"),
                }
            }
        }
    }

    /// Overwrite an object (unconditional put).
    pub async fn set_object(&self, key: &str, value: &[u8]) -> Result<()> {
        self.client
            .put_item()
            .table_name(&self.table)
            .item("pk", AttributeValue::S(key.to_string()))
            .item("value", AttributeValue::B(Blob::new(value)))
            .send()
            .await
            .with_context(|| format!("DynamoDB set_object({key}) failed"))?;
        Ok(())
    }

    /// Put an object only if it does not exist (conditional put).
    ///
    /// Returns `true` if this call wrote the value, `false` if the key already existed.
    pub async fn put_object(&self, key: &str, value: &[u8]) -> Result<bool> {
        let result = self
            .client
            .put_item()
            .table_name(&self.table)
            .item("pk", AttributeValue::S(key.to_string()))
            .item("value", AttributeValue::B(Blob::new(value)))
            .condition_expression("attribute_not_exists(pk)")
            .send()
            .await;

        match result {
            Ok(_) => Ok(true),
            Err(e) if is_condition_failed(&e) => Ok(false),
            Err(e) => Err(e).with_context(|| format!("DynamoDB put_object({key}) failed")),
        }
    }

    /// Delete an object by key.
    pub async fn delete_object(&self, key: &str) -> Result<()> {
        self.client
            .delete_item()
            .table_name(&self.table)
            .key("pk", AttributeValue::S(key.to_string()))
            .send()
            .await
            .with_context(|| format!("DynamoDB delete_object({key}) failed"))?;
        Ok(())
    }
}

fn is_condition_failed(err: &SdkError<PutItemError>) -> bool {
    matches!(
        err,
        SdkError::ServiceError(e) if e.err().is_conditional_check_failed_exception()
    )
}

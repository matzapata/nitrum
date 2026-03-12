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

// ── StorageClient ────────────────────────────────────────────────────────────

/// Client for the shared object storage table. Holds table name and DynamoDB client.
pub struct DynamoDBClient {
    client: Client,
    table: String,
}

impl DynamoDBClient {
    pub fn new(client: Client, table: String) -> Self {
        Self { client, table }
    }

    /// Fetch an object by key. Returns `None` if the key does not exist.
    pub async fn get_object(&self, key: &str) -> Result<Option<Vec<u8>>> {
        get_object_impl(&self.client, &self.table, key).await
    }

    /// Overwrite an object (unconditional put).
    pub async fn set_object(&self, key: &str, value: &[u8]) -> Result<()> {
        set_object_impl(&self.client, &self.table, key, value).await
    }

    /// Put an object only if it does not exist (conditional put).
    ///
    /// Returns `true` if this call wrote the value, `false` if the key already existed.
    pub async fn put_object(&self, key: &str, value: &[u8]) -> Result<bool> {
        put_object_impl(&self.client, &self.table, key, value).await
    }

    /// Delete an object by key.
    pub async fn delete_object(&self, key: &str) -> Result<()> {
        delete_object_impl(&self.client, &self.table, key).await
    }
}

// ── internal implementations (used by StorageClient) ───────────────────────────

async fn get_object_impl(
    client: &Client,
    table: &str,
    key: &str,
) -> Result<Option<Vec<u8>>> {
    let resp = client
        .get_item()
        .table_name(table)
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

async fn set_object_impl(
    client: &Client,
    table: &str,
    key: &str,
    value: &[u8],
) -> Result<()> {
    client
        .put_item()
        .table_name(table)
        .item("pk", AttributeValue::S(key.to_string()))
        .item("value", AttributeValue::B(Blob::new(value)))
        .send()
        .await
        .with_context(|| format!("DynamoDB set_object({key}) failed"))?;
    Ok(())
}

async fn put_object_impl(
    client: &Client,
    table: &str,
    key: &str,
    value: &[u8],
) -> Result<bool> {
    let result = client
        .put_item()
        .table_name(table)
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

async fn delete_object_impl(client: &Client, table: &str, key: &str) -> Result<()> {
    client
        .delete_item()
        .table_name(table)
        .key("pk", AttributeValue::S(key.to_string()))
        .send()
        .await
        .with_context(|| format!("DynamoDB delete_object({key}) failed"))?;
    Ok(())
}

fn is_condition_failed(err: &SdkError<PutItemError>) -> bool {
    matches!(
        err,
        SdkError::ServiceError(e) if e.err().is_conditional_check_failed_exception()
    )
}

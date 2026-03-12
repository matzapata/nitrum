//! DynamoDB helpers for distributed state shared across enclave instances.
//!
//! Table schema (single-table design, partition key = `pk: String`):
//!
//! | pk       | value (B)       | owner (S) | ttl (N)       |
//! |----------|-----------------|-----------|---------------|
//! | "dek"    | encrypted DEK   | –         | –             |
//! | "cert"   | PEM bytes       | –         | –             |
//! | "lock"   | –               | owner-id  | epoch seconds |
//!
//! TTL is enabled on the `ttl` attribute so stale locks are auto-expired.

use anyhow::{Context, Result};
use aws_sdk_dynamodb::{
    Client, error::SdkError, operation::put_item::PutItemError, primitives::Blob,
    types::AttributeValue,
};

const PK_DEK: &str = "dek";
const PK_CERT: &str = "cert";
const PK_LOCK: &str = "lock";
const LOCK_TTL_SECS: u64 = 60;

// ── lock ─────────────────────────────────────────────────────────────────────

/// Try to acquire the distributed setup lock.
///
/// Returns `true` if the lock was obtained, `false` if another instance holds
/// it.  The lock auto-expires after [`LOCK_TTL_SECS`] seconds via DynamoDB TTL
/// so a crashed holder will eventually release it.
pub async fn try_acquire_lock(client: &Client, table: &str, owner: &str) -> Result<bool> {
    let now = unix_now();
    let expiry = (now + LOCK_TTL_SECS).to_string();
    let now_str = now.to_string();

    let result = client
        .put_item()
        .table_name(table)
        .item("pk", AttributeValue::S(PK_LOCK.to_string()))
        .item("owner", AttributeValue::S(owner.to_string()))
        .item("ttl", AttributeValue::N(expiry))
        // Only place the lock if it doesn't exist OR the existing one has expired.
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

/// Release a previously acquired lock (owner-checked delete).
pub async fn release_lock(client: &Client, table: &str, owner: &str) -> Result<()> {
    client
        .delete_item()
        .table_name(table)
        .key("pk", AttributeValue::S(PK_LOCK.to_string()))
        .condition_expression("#o = :owner")
        .expression_attribute_names("#o", "owner")
        .expression_attribute_values(":owner", AttributeValue::S(owner.to_string()))
        .send()
        .await
        .context("DynamoDB release_lock failed")?;
    Ok(())
}

// ── DEK ──────────────────────────────────────────────────────────────────────

/// Persist the KMS-encrypted DEK.  Uses a conditional put so only the first
/// writer wins; concurrent losers should retry via [`retrieve_dek`].
///
/// Returns `true` if this call stored the value, `false` if one already existed.
pub async fn store_dek(client: &Client, table: &str, encrypted_dek: &[u8]) -> Result<bool> {
    let result = client
        .put_item()
        .table_name(table)
        .item("pk", AttributeValue::S(PK_DEK.to_string()))
        .item("value", AttributeValue::B(Blob::new(encrypted_dek)))
        .condition_expression("attribute_not_exists(pk)")
        .send()
        .await;

    match result {
        Ok(_) => Ok(true),
        Err(e) if is_condition_failed(&e) => Ok(false),
        Err(e) => Err(e).context("DynamoDB store_dek failed"),
    }
}

/// Fetch the KMS-encrypted DEK, or `None` if not yet stored.
pub async fn retrieve_dek(client: &Client, table: &str) -> Result<Option<Vec<u8>>> {
    get_binary_value(client, table, PK_DEK).await
}

// ── cert ─────────────────────────────────────────────────────────────────────

/// Store a PEM-encoded TLS certificate.
pub async fn store_cert(client: &Client, table: &str, cert_pem: &[u8]) -> Result<()> {
    client
        .put_item()
        .table_name(table)
        .item("pk", AttributeValue::S(PK_CERT.to_string()))
        .item("value", AttributeValue::B(Blob::new(cert_pem)))
        .send()
        .await
        .context("DynamoDB store_cert failed")?;
    Ok(())
}

/// Fetch the cached TLS certificate, or `None` if not yet stored.
pub async fn retrieve_cert(client: &Client, table: &str) -> Result<Option<Vec<u8>>> {
    get_binary_value(client, table, PK_CERT).await
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn is_condition_failed(err: &SdkError<PutItemError>) -> bool {
    matches!(
        err,
        SdkError::ServiceError(e)
            if e.err().is_conditional_check_failed_exception()
    )
}

async fn get_binary_value(client: &Client, table: &str, pk: &str) -> Result<Option<Vec<u8>>> {
    let resp = client
        .get_item()
        .table_name(table)
        .key("pk", AttributeValue::S(pk.to_string()))
        .send()
        .await
        .with_context(|| format!("DynamoDB get_item({pk}) failed"))?;

    match resp.item {
        None => Ok(None),
        Some(mut item) => {
            let blob = item
                .remove("value")
                .with_context(|| format!("DynamoDB item({pk}) missing 'value' attribute"))?;
            match blob {
                AttributeValue::B(b) => Ok(Some(b.into_inner())),
                other => anyhow::bail!("unexpected attribute type for '{pk}': {other:?}"),
            }
        }
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

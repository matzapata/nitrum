//! Distributed leader lock for critical sections (e.g. DEK or cert creation).
//!
//! [`Leader`] takes storage as a dependency and uses its DynamoDB client/table for the lock.
//! Dropping [`LeaderGuard`] releases the lock (best-effort via a spawned task).

use std::sync::Arc;

use anyhow::{Context, Result};
use aws_sdk_dynamodb::{
    Client,
    types::AttributeValue,
    error::SdkError,
    operation::put_item::PutItemError,
};

use crate::constants::{LOCK_OBJECT_KEY, LOCK_TTL_SECS};
use crate::utils::time;

use super::client::StorageClient;

// ── Leader ───────────────────────────────────────────────────────────────────

/// Leader election using the same storage table. Try to become leader without passing client/table each time.
pub struct Leader {
    storage: Arc<StorageClient>,
    owner: String,
}

impl Leader {
    pub fn new(storage: Arc<StorageClient>, owner: String) -> Self {
        Self { storage, owner }
    }

    /// Try to become leader for a critical section. Returns a guard if this instance got the lock.
    pub async fn try_acquire_leader(&self) -> Result<Option<LeaderGuard>> {
        let client = self.storage.dynamo_client();
        let table = self.storage.table_name();
        let acquired = try_acquire_lock(client, table, &self.owner).await?;
        Ok(if acquired {
            Some(LeaderGuard {
                storage: self.storage.clone(),
                owner: self.owner.clone(),
            })
        } else {
            None
        })
    }
}

// ── lock primitives (used by Leader and LeaderGuard) ──────────────────────────

/// Try to acquire the distributed setup lock.
///
/// Returns `true` if the lock was obtained, `false` if another instance holds it.
/// The lock auto-expires after [`LOCK_TTL_SECS`] seconds via DynamoDB TTL.
pub async fn try_acquire_lock(client: &Client, table: &str, owner: &str) -> Result<bool> {
    let now = time::unix_now();
    let expiry = (now + LOCK_TTL_SECS).to_string();
    let now_str = now.to_string();

    let result = client
        .put_item()
        .table_name(table)
        .item("pk", AttributeValue::S(LOCK_OBJECT_KEY.to_string()))
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

/// Release a previously acquired lock (owner-checked delete).
pub async fn release_lock(client: &Client, table: &str, owner: &str) -> Result<()> {
    client
        .delete_item()
        .table_name(table)
        .key("pk", AttributeValue::S(LOCK_OBJECT_KEY.to_string()))
        .condition_expression("#o = :owner")
        .expression_attribute_names("#o", "owner")
        .expression_attribute_values(":owner", AttributeValue::S(owner.to_string()))
        .send()
        .await
        .context("DynamoDB release_lock failed")?;
    Ok(())
}

/// Guard that holds the leader lock. Dropping it releases the lock (best-effort, via a spawned task).
pub struct LeaderGuard {
    storage: Arc<StorageClient>,
    owner: String,
}

impl LeaderGuard {
    /// Release the lock immediately so another instance can become leader.
    pub async fn release(self) -> Result<()> {
        release_lock(
            self.storage.dynamo_client(),
            self.storage.table_name(),
            &self.owner,
        )
        .await
    }
}

impl Drop for LeaderGuard {
    fn drop(&mut self) {
        let storage = self.storage.clone();
        let owner = self.owner.clone();
        tokio::spawn(async move {
            let _ = release_lock(
                storage.dynamo_client(),
                storage.table_name(),
                &owner,
            )
            .await;
        });
    }
}

// ── utils ───────────────────────────

/// Returns `true` if the error is a DynamoDB conditional check failure.
fn is_condition_failed(err: &SdkError<PutItemError>) -> bool {
    matches!(
        err,
        SdkError::ServiceError(e) if e.err().is_conditional_check_failed_exception()
    )
}

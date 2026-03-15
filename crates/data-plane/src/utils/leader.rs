//! Distributed leader lock for critical sections (e.g. DEK or cert creation).
//!
//! [`Leader`] wraps a [`StorageClient`] and a lock key, so each concern
//! (crypto, ACME, …) gets its own isolated lock.
//! Dropping [`LeaderGuard`] releases the lock (best-effort via a spawned task).

use crate::storage::StorageClient;
use anyhow::Result;
use std::sync::Arc;

// ── Leader ───────────────────────────────────────────────────────────────────

/// Leader election backed by a DynamoDB lock key. Create one per critical section.
pub struct Leader {
    storage: Arc<StorageClient>,
    owner: String,
    key: String,
}

impl Leader {
    pub fn new(storage: Arc<StorageClient>, owner: String, key: String) -> Self {
        Self {
            storage,
            owner,
            key,
        }
    }

    /// Try to become leader for this lock. Returns a guard if this instance got the lock.
    pub async fn try_acquire_leader(&self) -> Result<Option<LeaderGuard>> {
        let acquired = self
            .storage
            .try_acquire_lock(&self.key, &self.owner)
            .await?;
        Ok(if acquired {
            Some(LeaderGuard {
                storage: self.storage.clone(),
                owner: self.owner.clone(),
                key: self.key.clone(),
            })
        } else {
            None
        })
    }
}

// ── LeaderGuard ──────────────────────────────────────────────────────────────

/// Guard that holds the leader lock. Dropping it releases the lock (best-effort, via a spawned task).
pub struct LeaderGuard {
    storage: Arc<StorageClient>,
    owner: String,
    key: String,
}

impl LeaderGuard {
    /// Release the lock immediately so another instance can become leader.
    pub async fn release(self) -> Result<()> {
        self.storage.release_lock(&self.key, &self.owner).await
    }
}

impl Drop for LeaderGuard {
    fn drop(&mut self) {
        let storage = self.storage.clone();
        let owner = self.owner.clone();
        let key = self.key.clone();
        tokio::spawn(async move {
            let _ = storage.release_lock(&key, &owner).await;
        });
    }
}

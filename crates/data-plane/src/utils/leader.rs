//! Distributed leader lock for critical sections (e.g. DEK or cert creation).
//!
//! [`Leader`] wraps a [`StorageClient`] and a lock key, so each concern
//! (crypto, ACME, …) gets its own isolated lock.
//! Dropping [`LeaderGuard`] releases the lock (best-effort via a spawned task).

use crate::storage::StorageClient;
use crate::storage::keys::{ACME_LEADER_KEY, CRYPTO_LEADER_KEY};
use anyhow::Result;
use observability::MetricsHandle;
use std::sync::Arc;

// ── Leader ───────────────────────────────────────────────────────────────────

/// Leader election backed by a `DynamoDB` lock key. Create one per critical section.
pub struct Leader {
    storage: Arc<StorageClient>,
    owner: String,
    key: String,
    metrics: Option<MetricsHandle>,
    lock_dim: &'static str,
}

impl Leader {
    pub fn new(
        storage: Arc<StorageClient>,
        owner: String,
        key: String,
        metrics: Option<MetricsHandle>,
    ) -> Self {
        let lock_dim = if key == CRYPTO_LEADER_KEY {
            "crypto"
        } else if key == ACME_LEADER_KEY {
            "acme"
        } else {
            "other"
        };
        Self {
            storage,
            owner,
            key,
            metrics,
            lock_dim,
        }
    }

    /// Try to become leader for this lock. Returns a guard if this instance got the lock.
    #[tracing::instrument(name = "leader_acquire", skip(self), fields(lock_key = %self.key, owner = %self.owner))]
    pub async fn try_acquire_leader(&self) -> Result<Option<LeaderGuard>> {
        let acquired = self
            .storage
            .try_acquire_lock(&self.key, &self.owner)
            .await?;
        Ok(if acquired {
            if let Some(m) = &self.metrics {
                m.gauge_dims("LeaderLockHeld", 1.0, &[("LockKey", self.lock_dim)]);
            }
            Some(LeaderGuard {
                storage: self.storage.clone(),
                owner: self.owner.clone(),
                key: self.key.clone(),
                metrics: self.metrics.clone(),
                lock_dim: self.lock_dim,
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
    metrics: Option<MetricsHandle>,
    lock_dim: &'static str,
}

impl LeaderGuard {
    /// Release the lock immediately so another instance can become leader.
    #[tracing::instrument(name = "leader_release", skip(self), fields(lock_key = %self.key, owner = %self.owner))]
    #[allow(dead_code)]
    pub async fn release(self) -> Result<()> {
        self.storage.release_lock(&self.key, &self.owner).await
    }
}

impl Drop for LeaderGuard {
    fn drop(&mut self) {
        if let Some(m) = &self.metrics {
            m.gauge_dims("LeaderLockHeld", 0.0, &[("LockKey", self.lock_dim)]);
        }
        let storage = self.storage.clone();
        let owner = self.owner.clone();
        let key = self.key.clone();
        tokio::spawn(async move {
            let _ = storage.release_lock(&key, &owner).await;
        });
    }
}

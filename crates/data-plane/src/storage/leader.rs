//! Distributed leader lock for critical sections (e.g. DEK or cert creation).
//!
//! [`Leader`] wraps an [`ObjectStore`] and a lock key, so each concern
//! (crypto, ACME, …) gets its own isolated lock.
//! Dropping [`LeaderGuard`] releases the lock (best-effort via a spawned task).

use super::ObjectStore;
use anyhow::Result;
use std::sync::Arc;

// ── Leader ───────────────────────────────────────────────────────────────────

/// Leader election backed by a storage lock key. Create one per critical section.
pub struct Leader<S: ObjectStore + 'static> {
    storage: Arc<S>,
    owner: String,
    key: String,
}

impl<S: ObjectStore + 'static> Leader<S> {
    pub(crate) const fn new(storage: Arc<S>, owner: String, key: String) -> Self {
        Self {
            storage,
            owner,
            key,
        }
    }

    /// Try to become leader for this lock. Returns a guard if this instance got the lock.
    pub async fn try_acquire_leader(&self) -> Result<Option<LeaderGuard<S>>> {
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
pub struct LeaderGuard<S: ObjectStore + 'static> {
    storage: Arc<S>,
    owner: String,
    key: String,
}

impl<S: ObjectStore + 'static> LeaderGuard<S> {
    /// Release the lock immediately so another instance can become leader.
    pub async fn release(self) -> Result<()> {
        self.storage.release_lock(&self.key, &self.owner).await
    }
}

impl<S: ObjectStore + 'static> Drop for LeaderGuard<S> {
    fn drop(&mut self) {
        let storage = self.storage.clone();
        let owner = self.owner.clone();
        let key = self.key.clone();
        tokio::spawn(async move {
            let _ = storage.release_lock(&key, &owner).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::memory::InMemoryObjectStore;

    #[tokio::test]
    async fn acquire_fails_while_held_then_succeeds_after_release() {
        let store = Arc::new(InMemoryObjectStore::new());
        let a = Leader::new(store.clone(), "a".into(), "lock:test".into());
        let b = Leader::new(store.clone(), "b".into(), "lock:test".into());

        let guard = a.try_acquire_leader().await.unwrap().expect("a acquires");
        assert!(b.try_acquire_leader().await.unwrap().is_none());
        guard.release().await.unwrap();
        assert!(b.try_acquire_leader().await.unwrap().is_some());
    }

    #[tokio::test]
    async fn expired_lock_can_be_reacquired() {
        let mem = Arc::new(InMemoryObjectStore::new());
        let a = Leader::new(mem.clone(), "a".into(), "lock:ttl".into());
        let b = Leader::new(mem.clone(), "b".into(), "lock:ttl".into());

        let _guard = a.try_acquire_leader().await.unwrap().expect("a acquires");
        // Keep guard alive but expire the underlying TTL so B can steal.
        mem.expire_lock_for_test("lock:ttl");
        assert!(b.try_acquire_leader().await.unwrap().is_some());
    }
}

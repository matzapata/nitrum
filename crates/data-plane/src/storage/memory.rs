//! In-memory [`ObjectStore`] for unit tests and offline benches.

use super::ObjectStore;
use super::dynamo::LOCK_TTL_SECS;
use crate::utils::time;
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;

struct LockEntry {
    owner: String,
    expiry: u64,
}

/// HashMap-backed store matching DynamoDB conditional put / lock TTL semantics.
pub struct InMemoryObjectStore {
    objects: Mutex<HashMap<String, Vec<u8>>>,
    locks: Mutex<HashMap<String, LockEntry>>,
}

impl InMemoryObjectStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            objects: Mutex::new(HashMap::new()),
            locks: Mutex::new(HashMap::new()),
        }
    }

    /// Force a lock entry to be expired (tests only).
    pub fn expire_lock_for_test(&self, key: &str) {
        let mut locks = self.locks.lock().unwrap();
        if let Some(entry) = locks.get_mut(key) {
            entry.expiry = 0;
        }
    }
}

impl Default for InMemoryObjectStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ObjectStore for InMemoryObjectStore {
    async fn get_object(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.objects.lock().unwrap().get(key).cloned())
    }

    async fn put_object(&self, key: &str, value: &[u8]) -> Result<bool> {
        let mut objects = self.objects.lock().unwrap();
        if objects.contains_key(key) {
            return Ok(false);
        }
        objects.insert(key.to_string(), value.to_vec());
        drop(objects);
        Ok(true)
    }

    async fn set_object(&self, key: &str, value: &[u8]) -> Result<()> {
        self.objects
            .lock()
            .unwrap()
            .insert(key.to_string(), value.to_vec());
        Ok(())
    }

    async fn try_acquire_lock(&self, key: &str, owner: &str) -> Result<bool> {
        let now = time::unix_now();
        let mut locks = self.locks.lock().unwrap();
        match locks.get(key) {
            Some(entry) if entry.expiry >= now => Ok(false),
            _ => {
                locks.insert(
                    key.to_string(),
                    LockEntry {
                        owner: owner.to_string(),
                        expiry: now + LOCK_TTL_SECS,
                    },
                );
                drop(locks);
                Ok(true)
            }
        }
    }

    async fn release_lock(&self, key: &str, owner: &str) -> Result<()> {
        let mut locks = self.locks.lock().unwrap();
        if locks.get(key).is_some_and(|e| e.owner == owner) {
            locks.remove(key);
        }
        drop(locks);
        Ok(())
    }
}

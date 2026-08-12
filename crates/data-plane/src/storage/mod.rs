mod dynamo;
pub mod keys;
mod leader;

pub mod memory;

pub use dynamo::DynamoObjectStore;
pub use leader::Leader;
pub use memory::InMemoryObjectStore;

use anyhow::Result;
use async_trait::async_trait;

/// Distributed object + lock storage (DynamoDB in production; in-memory in tests).
#[async_trait]
pub trait ObjectStore: Send + Sync {
    /// Fetch an object by key. Returns `None` if the key does not exist.
    async fn get_object(&self, key: &str) -> Result<Option<Vec<u8>>>;

    /// Put an object only if it does not exist.
    ///
    /// Returns `true` if this call wrote the value, `false` if the key already existed.
    async fn put_object(&self, key: &str, value: &[u8]) -> Result<bool>;

    /// Overwrite an object (unconditional put).
    async fn set_object(&self, key: &str, value: &[u8]) -> Result<()>;

    /// Try to acquire a distributed lock. Returns `true` if obtained.
    async fn try_acquire_lock(&self, key: &str, owner: &str) -> Result<bool>;

    /// Release a lock (owner-checked).
    async fn release_lock(&self, key: &str, owner: &str) -> Result<()>;
}

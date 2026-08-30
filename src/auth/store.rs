use async_trait::async_trait;

use crate::core::AppResult;

/// Persistence boundary backed by the plaintext application configuration.
#[async_trait]
pub trait SecretStore: Send + Sync {
    async fn get(&self, key: &str, cache_ttl: Option<u64>) -> AppResult<Option<String>>;
    async fn put(&self, key: &str, value: &str) -> AppResult<()>;
    async fn delete(&self, key: &str) -> AppResult<()>;
}

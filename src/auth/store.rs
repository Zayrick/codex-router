use async_trait::async_trait;

use crate::core::AppResult;

/// Persistence boundary for application state.
#[async_trait]
pub trait StateStore: Send + Sync {
    async fn get(&self, key: &str) -> AppResult<Option<String>>;
    async fn put(&self, key: &str, value: &str) -> AppResult<()>;
    async fn delete(&self, key: &str) -> AppResult<()>;
}

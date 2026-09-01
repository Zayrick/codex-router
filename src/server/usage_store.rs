use crate::{
    application::{CodexUsageMonitorState, validate_codex_usage_monitor_state},
    auth::StateStore,
    core::{ApiError, AppResult},
};

pub(super) const CODEX_ACCOUNT_USAGE_KEY_PREFIX: &str = "CODEX_USAGE:";

pub struct CodexUsageStateRepository<'a> {
    store: &'a dyn StateStore,
    storage_key: String,
}

impl<'a> CodexUsageStateRepository<'a> {
    pub fn new(store: &'a dyn StateStore, account_id: &str) -> Self {
        Self {
            store,
            storage_key: format!("{CODEX_ACCOUNT_USAGE_KEY_PREFIX}{account_id}"),
        }
    }

    pub async fn read(&self) -> AppResult<Option<CodexUsageMonitorState>> {
        let Some(serialized) = self.store.get(&self.storage_key).await? else {
            return Ok(None);
        };
        let value = serde_json::from_str(&serialized).map_err(|_| invalid_stored_usage_state())?;
        validate_codex_usage_monitor_state(value).map(Some)
    }

    pub async fn store(&self, state: &CodexUsageMonitorState) -> AppResult<()> {
        let serialized = serde_json::to_string(state).map_err(|_| invalid_stored_usage_state())?;
        self.store.put(&self.storage_key, &serialized).await
    }
}

fn invalid_stored_usage_state() -> ApiError {
    ApiError::new(500, "Stored Codex usage state is unavailable.")
        .with_kind("configuration_error")
        .with_code("invalid_codex_usage_state")
}

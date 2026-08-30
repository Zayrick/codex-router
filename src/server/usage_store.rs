use crate::{
    application::{CodexUsageMonitorState, validate_codex_usage_monitor_state},
    auth::StateStore,
    core::{ApiError, AppResult},
};

const CODEX_USAGE_KEY: &str = "CODEX_USAGE";

pub struct CodexUsageStateRepository<'a> {
    store: &'a dyn StateStore,
}

impl<'a> CodexUsageStateRepository<'a> {
    pub const fn new(store: &'a dyn StateStore) -> Self {
        Self { store }
    }

    pub async fn read(&self) -> AppResult<Option<CodexUsageMonitorState>> {
        let Some(serialized) = self.store.get(CODEX_USAGE_KEY).await? else {
            return Ok(None);
        };
        let value = serde_json::from_str(&serialized).map_err(|_| invalid_stored_usage_state())?;
        validate_codex_usage_monitor_state(value).map(Some)
    }

    pub async fn store(&self, state: &CodexUsageMonitorState) -> AppResult<()> {
        let serialized = serde_json::to_string(state).map_err(|_| invalid_stored_usage_state())?;
        self.store.put(CODEX_USAGE_KEY, &serialized).await
    }
}

fn invalid_stored_usage_state() -> ApiError {
    ApiError::new(500, "Stored Codex usage state is unavailable.")
        .with_kind("configuration_error")
        .with_code("invalid_codex_usage_state")
}

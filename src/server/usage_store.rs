use crate::{
    application::{CodexUsageMonitorState, validate_codex_usage_monitor_state},
    auth::SecretStore,
    core::{ApiError, AppResult},
};

const CODEX_USAGE_KEY: &str = "CODEX_USAGE";
const MAX_CODEX_USAGE_CONFIG_CHARS: usize = 256 * 1024;

pub struct CodexUsageStateRepository<'a> {
    store: &'a dyn SecretStore,
}

impl<'a> CodexUsageStateRepository<'a> {
    pub const fn new(store: &'a dyn SecretStore) -> Self {
        Self { store }
    }

    pub async fn read(&self) -> AppResult<Option<CodexUsageMonitorState>> {
        let Some(serialized) = self.store.get(CODEX_USAGE_KEY, None).await? else {
            return Ok(None);
        };
        if serialized.len() > MAX_CODEX_USAGE_CONFIG_CHARS {
            return Err(invalid_stored_usage_state());
        }
        let value = serde_json::from_str(&serialized).map_err(|_| invalid_stored_usage_state())?;
        validate_codex_usage_monitor_state(value).map(Some)
    }

    pub async fn store(&self, state: &CodexUsageMonitorState) -> AppResult<()> {
        let serialized = serde_json::to_string(state).map_err(|_| invalid_stored_usage_state())?;
        if serialized.len() > MAX_CODEX_USAGE_CONFIG_CHARS {
            return Err(invalid_stored_usage_state());
        }
        self.store.put(CODEX_USAGE_KEY, &serialized).await
    }
}

fn invalid_stored_usage_state() -> ApiError {
    ApiError::new(500, "Stored Codex usage state is unavailable.")
        .with_kind("configuration_error")
        .with_code("invalid_codex_usage_state")
}

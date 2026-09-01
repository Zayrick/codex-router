use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{ApiError, AppResult};

use super::{AuthProxyAccount, ClientApiKey, StateStore, new_record_id, valid_record_id};

pub const ACCOUNT_ROUTING_KEY: &str = "ACCOUNT_ROUTING";
const ROUND_ROBIN_STRATEGY: &str = "round-robin";
pub const WEIGHTED_ROUND_ROBIN_STRATEGY: &str = "weighted-round-robin";
pub const FALLBACK_STRATEGY: &str = "fallback";
pub(crate) const CURRENT_ROUTING_VERSION: u8 = 1;
const MAX_CODEX_ACCOUNTS: usize = 101;
const MAX_ACCOUNT_GROUPS: usize = 100;
const MAX_NAME_LENGTH: usize = 100;
const MAX_SESSION_AFFINITY_TTL_LENGTH: usize = 64;
const UNLIMITED_TTL: &str = "unlimited";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAccount {
    pub id: String,
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountGroup {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub account_ids: Vec<String>,
    #[serde(default = "default_strategy")]
    pub strategy: String,
    #[serde(default)]
    pub session_affinity: bool,
    #[serde(default = "default_session_affinity_ttl")]
    pub session_affinity_ttl: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteConsumerKind {
    ApiKey,
    AuthProxy,
}

impl RouteConsumerKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ApiKey => "api_key",
            Self::AuthProxy => "auth_proxy",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteTargetKind {
    Account,
    Group,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteAssignment {
    pub consumer_type: RouteConsumerKind,
    pub consumer_id: String,
    pub target_type: RouteTargetKind,
    pub target_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountRoutingState {
    #[serde(default)]
    pub accounts: Vec<CodexAccount>,
    #[serde(default)]
    pub groups: Vec<AccountGroup>,
    #[serde(default)]
    pub routes: Vec<RouteAssignment>,
}

impl AccountRoutingState {
    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty() && self.groups.is_empty() && self.routes.is_empty()
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredAccountRouting {
    pub version: u8,
    #[serde(flatten)]
    pub state: AccountRoutingState,
}

pub struct RoutingRepository<'a> {
    store: &'a dyn StateStore,
}

impl<'a> RoutingRepository<'a> {
    pub fn new(store: &'a dyn StateStore) -> Self {
        Self { store }
    }

    pub async fn read(&self) -> AppResult<AccountRoutingState> {
        let Some(serialized) = self.store.get(ACCOUNT_ROUTING_KEY).await? else {
            return Ok(AccountRoutingState::default());
        };
        let stored: StoredAccountRouting =
            serde_json::from_str(&serialized).map_err(|_| invalid_stored_routing())?;
        if stored.version != CURRENT_ROUTING_VERSION {
            return Err(invalid_stored_routing());
        }
        validate_state(stored.state, None, None).map_err(|_| invalid_stored_routing())
    }

    async fn store(&self, state: AccountRoutingState) -> AppResult<AccountRoutingState> {
        let state = validate_state(state, None, None)?;
        let serialized = serde_json::to_string(&StoredAccountRouting {
            version: CURRENT_ROUTING_VERSION,
            state: state.clone(),
        })
        .map_err(|_| invalid_stored_routing())?;
        self.store.put(ACCOUNT_ROUTING_KEY, &serialized).await?;
        Ok(state)
    }

    pub async fn create_account(
        &self,
        id: String,
        preferred_name: Option<&str>,
    ) -> AppResult<AccountRoutingState> {
        if !valid_record_id(&id) {
            return Err(invalid_codex_account());
        }
        let mut state = self.read().await?;
        if state.accounts.iter().any(|account| account.id == id) {
            return Ok(state);
        }
        if state.accounts.len() >= MAX_CODEX_ACCOUNTS {
            return Err(routing_conflict(
                "The Codex account limit has been reached.",
            ));
        }
        let base_name =
            normalized_account_name(preferred_name).unwrap_or_else(|| "Codex 账户".into());
        let name = unique_account_name(
            &base_name,
            state.accounts.iter().map(|entry| entry.name.as_str()),
        );
        state.accounts.push(CodexAccount {
            id,
            name,
            enabled: true,
        });
        self.store(state).await
    }

    pub async fn update_account(
        &self,
        id: &Value,
        value: &Value,
    ) -> AppResult<AccountRoutingState> {
        let id = record_id(id, invalid_codex_account)?;
        let object = value.as_object().ok_or_else(invalid_codex_account)?;
        let name = normalized_account_name(object.get("name").and_then(Value::as_str))
            .ok_or_else(invalid_codex_account)?;
        let enabled = object
            .get("enabled")
            .and_then(Value::as_bool)
            .ok_or_else(invalid_codex_account)?;
        let mut state = self.read().await?;
        if state
            .accounts
            .iter()
            .any(|entry| entry.id != id && entry.name == name)
        {
            return Err(routing_conflict(
                "A Codex account with that name already exists.",
            ));
        }
        let account = state
            .accounts
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(codex_account_not_found)?;
        account.name = name;
        account.enabled = enabled;
        if !enabled {
            state.routes.retain(|route| {
                !(route.target_type == RouteTargetKind::Account && route.target_id == id)
            });
        }
        self.store(state).await
    }

    pub async fn delete_account(&self, id: &Value) -> AppResult<AccountRoutingState> {
        let id = record_id(id, invalid_codex_account)?;
        let mut state = self.read().await?;
        let before = state.accounts.len();
        state.accounts.retain(|entry| entry.id != id);
        if state.accounts.len() == before {
            return Err(codex_account_not_found());
        }
        state.routes.retain(|route| {
            !(route.target_type == RouteTargetKind::Account && route.target_id == id)
        });
        for group in &mut state.groups {
            group.account_ids.retain(|account_id| account_id != &id);
        }
        self.store(state).await
    }

    pub async fn replace_configuration(
        &self,
        value: &Value,
        api_keys: &[ClientApiKey],
        auth_proxy_accounts: &[AuthProxyAccount],
    ) -> AppResult<AccountRoutingState> {
        let object = value
            .as_object()
            .ok_or_else(invalid_routing_configuration)?;
        let groups: Vec<AccountGroup> = serde_json::from_value(
            object
                .get("groups")
                .cloned()
                .ok_or_else(invalid_routing_configuration)?,
        )
        .map_err(|_| invalid_routing_configuration())?;
        let routes: Vec<RouteAssignment> = serde_json::from_value(
            object
                .get("routes")
                .cloned()
                .ok_or_else(invalid_routing_configuration)?,
        )
        .map_err(|_| invalid_routing_configuration())?;
        let mut state = self.read().await?;
        state.groups = groups;
        state.routes = routes;
        let state = validate_state(state, Some(api_keys), Some(auth_proxy_accounts))?;
        self.store(state).await
    }

    pub async fn release_consumer(
        &self,
        kind: RouteConsumerKind,
        id: &str,
    ) -> AppResult<AccountRoutingState> {
        let mut state = self.read().await?;
        state
            .routes
            .retain(|route| !(route.consumer_type == kind && route.consumer_id == id));
        self.store(state).await
    }

    pub async fn account(&self, id: &Value) -> AppResult<CodexAccount> {
        let id = record_id(id, invalid_codex_account)?;
        self.read()
            .await?
            .accounts
            .into_iter()
            .find(|entry| entry.id == id)
            .ok_or_else(codex_account_not_found)
    }

    pub async fn next_account_id(&self) -> AppResult<String> {
        let state = self.read().await?;
        if state.accounts.len() >= MAX_CODEX_ACCOUNTS {
            return Err(routing_conflict(
                "The Codex account limit has been reached.",
            ));
        }
        Ok(new_record_id())
    }
}

fn normalize_session_affinity_ttl(value: &str) -> Option<String> {
    let value = value.trim();
    if value.eq_ignore_ascii_case(UNLIMITED_TTL) {
        return Some(UNLIMITED_TTL.into());
    }
    if value.is_empty() || value.len() > MAX_SESSION_AFFINITY_TTL_LENGTH {
        return None;
    }
    humantime::parse_duration(value)
        .ok()
        .filter(|duration| !duration.is_zero())
        .map(|_| value.to_owned())
}

pub fn session_affinity_ttl(value: &str) -> Option<std::time::Duration> {
    if value.eq_ignore_ascii_case(UNLIMITED_TTL) {
        None
    } else {
        humantime::parse_duration(value).ok()
    }
}

fn validate_state(
    mut state: AccountRoutingState,
    api_keys: Option<&[ClientApiKey]>,
    auth_proxy_accounts: Option<&[AuthProxyAccount]>,
) -> AppResult<AccountRoutingState> {
    if state.accounts.len() > MAX_CODEX_ACCOUNTS || state.groups.len() > MAX_ACCOUNT_GROUPS {
        return Err(invalid_routing_configuration());
    }
    let mut account_ids = HashSet::new();
    let mut account_names = HashSet::new();
    for account in &mut state.accounts {
        if !valid_record_id(&account.id) {
            return Err(invalid_routing_configuration());
        }
        account.name = normalized_account_name(Some(&account.name))
            .ok_or_else(invalid_routing_configuration)?;
        if !account_ids.insert(account.id.clone()) || !account_names.insert(account.name.clone()) {
            return Err(invalid_routing_configuration());
        }
    }
    state
        .accounts
        .sort_by(|left, right| left.name.encode_utf16().cmp(right.name.encode_utf16()));

    let mut group_ids = HashSet::new();
    let mut group_names = HashSet::new();
    for group in &mut state.groups {
        if !valid_record_id(&group.id) {
            return Err(invalid_routing_configuration());
        }
        group.name =
            normalized_account_name(Some(&group.name)).ok_or_else(invalid_routing_configuration)?;
        if !group_ids.insert(group.id.clone()) || !group_names.insert(group.name.clone()) {
            return Err(invalid_routing_configuration());
        }
        group.strategy = normalize_strategy(&group.strategy)
            .ok_or_else(invalid_routing_configuration)?
            .into();
        group.session_affinity_ttl = normalize_session_affinity_ttl(&group.session_affinity_ttl)
            .ok_or_else(invalid_routing_configuration)?;
        let mut members = HashSet::new();
        if group
            .account_ids
            .iter()
            .any(|id| !members.insert(id.clone()) || !account_ids.contains(id))
        {
            return Err(invalid_routing_configuration());
        }
    }
    state
        .groups
        .sort_by(|left, right| left.name.encode_utf16().cmp(right.name.encode_utf16()));

    let configured_api_keys: Option<HashSet<_>> =
        api_keys.map(|entries| entries.iter().map(|entry| entry.id.as_str()).collect());
    let configured_auth_proxy: Option<HashSet<_>> =
        auth_proxy_accounts.map(|entries| entries.iter().map(|entry| entry.id.as_str()).collect());
    let mut consumers = HashSet::new();
    for route in &state.routes {
        if !valid_record_id(&route.consumer_id) || !valid_record_id(&route.target_id) {
            return Err(invalid_routing_configuration());
        }
        if !consumers.insert((route.consumer_type, route.consumer_id.clone())) {
            return Err(invalid_routing_configuration());
        }
        let consumer_exists = match route.consumer_type {
            RouteConsumerKind::ApiKey => configured_api_keys
                .as_ref()
                .is_none_or(|ids| ids.contains(route.consumer_id.as_str())),
            RouteConsumerKind::AuthProxy => configured_auth_proxy
                .as_ref()
                .is_none_or(|ids| ids.contains(route.consumer_id.as_str())),
        };
        if !consumer_exists {
            return Err(invalid_routing_configuration());
        }
        match route.target_type {
            RouteTargetKind::Account => {
                let Some(account) = state
                    .accounts
                    .iter()
                    .find(|entry| entry.id == route.target_id)
                else {
                    return Err(invalid_routing_configuration());
                };
                if !account.enabled {
                    return Err(invalid_routing_configuration());
                }
            }
            RouteTargetKind::Group if !group_ids.contains(&route.target_id) => {
                return Err(invalid_routing_configuration());
            }
            RouteTargetKind::Group => {}
        }
    }
    state.routes.sort_by(|left, right| {
        (left.consumer_type.as_str(), left.consumer_id.as_str())
            .cmp(&(right.consumer_type.as_str(), right.consumer_id.as_str()))
    });
    Ok(state)
}

fn record_id(value: &Value, error: fn() -> ApiError) -> AppResult<String> {
    value
        .as_str()
        .filter(|value| valid_record_id(value))
        .map(str::to_owned)
        .ok_or_else(error)
}

pub(crate) fn normalized_account_name(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty()
        || value.encode_utf16().count() > MAX_NAME_LENGTH
        || value
            .chars()
            .any(|character| matches!(character as u32, 0x00..=0x1f | 0x7f))
    {
        return None;
    }
    Some(value.to_owned())
}

pub(crate) fn unique_account_name<'a>(
    base: &str,
    existing: impl Iterator<Item = &'a str>,
) -> String {
    let names: HashSet<_> = existing.collect();
    if !names.contains(base) {
        return base.into();
    }
    for suffix in 2..=MAX_CODEX_ACCOUNTS + 1 {
        let suffix = format!(" {suffix}");
        let max = MAX_NAME_LENGTH.saturating_sub(suffix.encode_utf16().count());
        let candidate = format!("{}{}", truncate_utf16(base, max), suffix);
        if !names.contains(candidate.as_str()) {
            return candidate;
        }
    }
    unreachable!("the account limit guarantees an available display name")
}

fn truncate_utf16(value: &str, max_units: usize) -> String {
    let mut units = 0;
    value
        .chars()
        .take_while(|character| {
            let next = units + character.len_utf16();
            if next > max_units {
                return false;
            }
            units = next;
            true
        })
        .collect()
}

fn default_strategy() -> String {
    ROUND_ROBIN_STRATEGY.into()
}

fn normalize_strategy(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "round-robin" => Some(ROUND_ROBIN_STRATEGY),
        "weighted-round-robin" => Some(WEIGHTED_ROUND_ROBIN_STRATEGY),
        "fallback" => Some(FALLBACK_STRATEGY),
        _ => None,
    }
}

fn default_session_affinity_ttl() -> String {
    "1h".into()
}

fn invalid_codex_account() -> ApiError {
    ApiError::new(400, "The Codex account is invalid.")
        .with_kind("invalid_request_error")
        .with_code("invalid_codex_account")
}

fn codex_account_not_found() -> ApiError {
    ApiError::new(404, "The requested Codex account does not exist.")
        .with_kind("invalid_request_error")
        .with_code("codex_account_not_found")
}

fn invalid_routing_configuration() -> ApiError {
    ApiError::new(400, "The account routing configuration is invalid.")
        .with_kind("invalid_request_error")
        .with_code("invalid_account_routing")
}

fn invalid_stored_routing() -> ApiError {
    ApiError::new(500, "Stored account routing is unavailable.")
        .with_kind("configuration_error")
        .with_code("invalid_stored_account_routing")
}

fn routing_conflict(message: &str) -> ApiError {
    ApiError::new(409, message)
        .with_kind("invalid_request_error")
        .with_code("account_routing_conflict")
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use async_trait::async_trait;

    use super::*;

    #[derive(Default)]
    struct MemoryStore {
        values: Mutex<HashMap<String, String>>,
    }

    #[async_trait]
    impl StateStore for MemoryStore {
        async fn get(&self, key: &str) -> AppResult<Option<String>> {
            Ok(self.values.lock().unwrap().get(key).cloned())
        }

        async fn put(&self, key: &str, value: &str) -> AppResult<()> {
            self.values
                .lock()
                .unwrap()
                .insert(key.to_owned(), value.to_owned());
            Ok(())
        }

        async fn delete(&self, key: &str) -> AppResult<()> {
            self.values.lock().unwrap().remove(key);
            Ok(())
        }
    }

    #[test]
    fn validates_duration_and_unlimited_affinity_ttls() {
        assert_eq!(
            normalize_session_affinity_ttl(" 1h ").as_deref(),
            Some("1h")
        );
        assert_eq!(
            normalize_session_affinity_ttl("UNLIMITED").as_deref(),
            Some("unlimited")
        );
        assert!(normalize_session_affinity_ttl("0s").is_none());
        assert!(normalize_session_affinity_ttl("tomorrow").is_none());
        assert_eq!(
            session_affinity_ttl("90m"),
            Some(std::time::Duration::from_secs(5_400))
        );
        assert_eq!(session_affinity_ttl("unlimited"), None);
    }

    #[tokio::test]
    async fn account_creation_is_idempotent_for_device_poll_retries() {
        let store = MemoryStore::default();
        let repository = RoutingRepository::new(&store);
        let id = "00000000-0000-4000-8000-000000000001";
        repository
            .create_account(id.into(), Some("first"))
            .await
            .unwrap();
        let state = repository
            .create_account(id.into(), Some("changed"))
            .await
            .unwrap();
        assert_eq!(state.accounts.len(), 1);
        assert_eq!(state.accounts[0].name, "first");
    }

    #[tokio::test]
    async fn disabling_releases_direct_routes_but_keeps_group_membership() {
        let store = MemoryStore::default();
        let repository = RoutingRepository::new(&store);
        let account_id = "00000000-0000-4000-8000-000000000001";
        let group_id = "00000000-0000-4000-8000-000000000002";
        let direct_consumer = "00000000-0000-4000-8000-000000000003";
        let group_consumer = "00000000-0000-4000-8000-000000000004";
        repository
            .store(AccountRoutingState {
                accounts: vec![CodexAccount {
                    id: account_id.into(),
                    name: "primary".into(),
                    enabled: true,
                }],
                groups: vec![AccountGroup {
                    id: group_id.into(),
                    name: "pool".into(),
                    account_ids: vec![account_id.into()],
                    strategy: "round-robin".into(),
                    session_affinity: true,
                    session_affinity_ttl: "1h".into(),
                }],
                routes: vec![
                    RouteAssignment {
                        consumer_type: RouteConsumerKind::ApiKey,
                        consumer_id: direct_consumer.into(),
                        target_type: RouteTargetKind::Account,
                        target_id: account_id.into(),
                    },
                    RouteAssignment {
                        consumer_type: RouteConsumerKind::AuthProxy,
                        consumer_id: group_consumer.into(),
                        target_type: RouteTargetKind::Group,
                        target_id: group_id.into(),
                    },
                ],
            })
            .await
            .unwrap();

        let state = repository
            .update_account(
                &Value::String(account_id.into()),
                &serde_json::json!({"name": "primary", "enabled": false}),
            )
            .await
            .unwrap();
        assert_eq!(state.groups[0].account_ids, vec![account_id]);
        assert_eq!(state.routes.len(), 1);
        assert_eq!(state.routes[0].target_type, RouteTargetKind::Group);
    }
}

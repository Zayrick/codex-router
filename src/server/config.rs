use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use axum::http::HeaderValue;
use serde::{Deserialize, Serialize};
use tokio::{fs, io::AsyncWriteExt, sync::RwLock};
use url::Url;

use crate::{
    application::CodexUsageMonitorState,
    auth::{
        ACCOUNT_ROUTING_KEY, AccountRoutingState, AuthProxyAccount, CODEX_ACCOUNT_OAUTH_KEY_PREFIX,
        CURRENT_ROUTING_VERSION, ClientApiKey, CodexAccount, RouteAssignment, RouteConsumerKind,
        RouteTargetKind, StateStore, StoredAccountRouting, StoredOAuthCredentials,
        derived_record_id, normalized_account_name, unique_account_name,
    },
    core::{ApiError, AppResult},
    upstream::{bark::parse_bark_push_url, dingtalk::signed_dingtalk_webhook},
};

use super::chatgpt_proxy::ChatgptProxy;
use super::pricing::{ModelPrice, normalized_model_prices, validate_model_prices};
use super::usage_store::CODEX_ACCOUNT_USAGE_KEY_PREFIX;

const API_KEYS_KEY: &str = "API_KEYS";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,
    pub admin: AdminConfig,
    pub upstream: UpstreamConfig,
    #[serde(default)]
    pub usage_tracking: UsageTrackingConfig,
    #[serde(default)]
    pub notifications: NotificationConfig,
    #[serde(default)]
    pub state: PersistentState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageTrackingConfig {
    #[serde(default = "default_usage_database_path")]
    pub database_path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_prices: Vec<ModelPrice>,
}

impl Default for UsageTrackingConfig {
    fn default() -> Self {
        Self {
            database_path: default_usage_database_path(),
            model_prices: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_public_origin")]
    pub public_origin: String,
    #[serde(default = "default_cors_origin")]
    pub cors_origin: String,
    #[serde(default = "default_maintenance_interval")]
    pub maintenance_interval_seconds: u64,
    #[serde(default = "default_log_filter")]
    pub log_filter: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            public_origin: default_public_origin(),
            cors_origin: default_cors_origin(),
            maintenance_interval_seconds: default_maintenance_interval(),
            log_filter: default_log_filter(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminConfig {
    pub path: String,
    pub secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chatgpt_proxy: Option<String>,
    #[serde(default = "default_codex_resets_url")]
    pub codex_resets_url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotificationConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bark_push_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dingtalk_webhook_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dingtalk_secret: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersistentState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<StoredOAuthCredentials>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub api_keys: Vec<ClientApiKey>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub auth_proxy_accounts: Vec<AuthProxyAccount>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub auth_proxy_oauth: BTreeMap<String, StoredOAuthCredentials>,
    #[serde(default, skip_serializing_if = "AccountRoutingState::is_empty")]
    pub account_routing: AccountRoutingState,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub codex_account_oauth: BTreeMap<String, StoredOAuthCredentials>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<CodexUsageMonitorState>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub account_usage: BTreeMap<String, CodexUsageMonitorState>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredApiKeys {
    version: u8,
    keys: Vec<ClientApiKey>,
    #[serde(default, rename = "authProxyAccounts")]
    auth_proxy_accounts: Vec<AuthProxyAccount>,
}

pub struct ConfigStore {
    path: PathBuf,
    config: RwLock<AppConfig>,
}

impl ConfigStore {
    pub async fn load(path: impl Into<PathBuf>) -> Result<Arc<Self>> {
        let path = path.into();
        let source = fs::read_to_string(&path)
            .await
            .with_context(|| format!("failed to read configuration {}", path.display()))?;
        let mut config: AppConfig = toml::from_str(&source)
            .with_context(|| format!("failed to parse configuration {}", path.display()))?;
        validate_config(&config)?;
        if migrate_legacy_accounts(&mut config) {
            persist(&path, &config)
                .await
                .with_context(|| format!("failed to migrate configuration {}", path.display()))?;
        }
        Ok(Arc::new(Self {
            path,
            config: RwLock::new(config),
        }))
    }

    pub async fn snapshot(&self) -> AppConfig {
        self.config.read().await.clone()
    }

    pub async fn bind_address(&self) -> Result<SocketAddr> {
        self.config
            .read()
            .await
            .server
            .bind
            .parse()
            .context("server.bind must be an IP socket address")
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn resolve_path(&self, configured: &str) -> PathBuf {
        let configured = PathBuf::from(configured);
        if configured.is_absolute() {
            configured
        } else {
            self.path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(configured)
        }
    }

    pub async fn replace_model_prices(
        &self,
        prices: Vec<ModelPrice>,
    ) -> AppResult<Vec<ModelPrice>> {
        let prices = normalized_model_prices(prices).map_err(|_| invalid_model_prices())?;
        let stored = prices.clone();
        self.update(move |config| {
            config.usage_tracking.model_prices = stored;
            Ok(())
        })
        .await?;
        Ok(prices)
    }

    async fn update(
        &self,
        operation: impl FnOnce(&mut AppConfig) -> AppResult<()>,
    ) -> AppResult<()> {
        let mut current = self.config.write().await;
        let mut next = current.clone();
        operation(&mut next)?;
        persist(&self.path, &next)
            .await
            .map_err(|_| config_write_error())?;
        *current = next;
        Ok(())
    }
}

#[async_trait]
impl StateStore for ConfigStore {
    async fn get(&self, key: &str) -> AppResult<Option<String>> {
        let config = self.config.read().await;
        let value = match key {
            API_KEYS_KEY => serde_json::to_string(&StoredApiKeys {
                version: 2,
                keys: config.state.api_keys.clone(),
                auth_proxy_accounts: config.state.auth_proxy_accounts.clone(),
            })
            .map(Some),
            ACCOUNT_ROUTING_KEY => serde_json::to_string(&StoredAccountRouting {
                version: CURRENT_ROUTING_VERSION,
                state: config.state.account_routing.clone(),
            })
            .map(Some),
            _ => {
                if let Some(value) = key
                    .strip_prefix(CODEX_ACCOUNT_USAGE_KEY_PREFIX)
                    .and_then(|id| config.state.account_usage.get(id))
                {
                    serde_json::to_string(value).map(Some)
                } else {
                    key.strip_prefix(CODEX_ACCOUNT_OAUTH_KEY_PREFIX)
                        .and_then(|id| config.state.codex_account_oauth.get(id))
                        .map(serde_json::to_string)
                        .transpose()
                }
            }
        };
        value.map_err(|_| config_read_error())
    }

    async fn put(&self, key: &str, value: &str) -> AppResult<()> {
        self.update(|config| {
            match key {
                API_KEYS_KEY => {
                    let stored: StoredApiKeys =
                        serde_json::from_str(value).map_err(|_| config_read_error())?;
                    config.state.api_keys = stored.keys;
                    config.state.auth_proxy_accounts = stored.auth_proxy_accounts;
                }
                ACCOUNT_ROUTING_KEY => {
                    let stored: StoredAccountRouting =
                        serde_json::from_str(value).map_err(|_| config_read_error())?;
                    if stored.version != CURRENT_ROUTING_VERSION {
                        return Err(config_read_error());
                    }
                    config.state.account_routing = stored.state;
                }
                _ => {
                    if let Some(id) = key
                        .strip_prefix(CODEX_ACCOUNT_USAGE_KEY_PREFIX)
                        .filter(|id| !id.is_empty())
                    {
                        let usage = serde_json::from_str(value).map_err(|_| config_read_error())?;
                        config.state.account_usage.insert(id.to_owned(), usage);
                        return Ok(());
                    }
                    let credentials =
                        serde_json::from_str(value).map_err(|_| config_read_error())?;
                    if let Some(id) = key
                        .strip_prefix(CODEX_ACCOUNT_OAUTH_KEY_PREFIX)
                        .filter(|id| !id.is_empty())
                    {
                        config
                            .state
                            .codex_account_oauth
                            .insert(id.to_owned(), credentials);
                    } else {
                        return Err(config_read_error());
                    }
                }
            }
            Ok(())
        })
        .await
    }

    async fn delete(&self, key: &str) -> AppResult<()> {
        self.update(|config| {
            match key {
                API_KEYS_KEY => {
                    config.state.api_keys.clear();
                    config.state.auth_proxy_accounts.clear();
                }
                ACCOUNT_ROUTING_KEY => {
                    config.state.account_routing = AccountRoutingState::default()
                }
                _ => {
                    if let Some(id) = key
                        .strip_prefix(CODEX_ACCOUNT_USAGE_KEY_PREFIX)
                        .filter(|id| !id.is_empty())
                    {
                        config.state.account_usage.remove(id);
                    } else if let Some(id) = key
                        .strip_prefix(CODEX_ACCOUNT_OAUTH_KEY_PREFIX)
                        .filter(|id| !id.is_empty())
                    {
                        config.state.codex_account_oauth.remove(id);
                    } else {
                        return Err(config_read_error());
                    }
                }
            }
            Ok(())
        })
        .await
    }
}

fn migrate_legacy_accounts(config: &mut AppConfig) -> bool {
    let Some(primary_credentials) = config.state.oauth.take() else {
        if config.state.auth_proxy_oauth.is_empty() {
            return false;
        }
        return migrate_legacy_proxy_accounts(config, None);
    };

    let primary_id = legacy_codex_account_id("primary", &primary_credentials);
    let primary_name = normalized_account_name(primary_credentials.email.as_deref())
        .unwrap_or_else(|| "Codex 账户".into());
    let primary_name = unique_account_name(
        &primary_name,
        config
            .state
            .account_routing
            .accounts
            .iter()
            .map(|account| account.name.as_str()),
    );
    if !config
        .state
        .account_routing
        .accounts
        .iter()
        .any(|entry| entry.id == primary_id)
    {
        config.state.account_routing.accounts.push(CodexAccount {
            id: primary_id.clone(),
            name: primary_name,
            enabled: true,
        });
    }
    config
        .state
        .codex_account_oauth
        .insert(primary_id.clone(), primary_credentials);
    if let Some(usage) = config.state.usage.take() {
        config.state.account_usage.insert(primary_id.clone(), usage);
    }
    for key in &config.state.api_keys {
        let key_id = if key.id.is_empty() {
            derived_record_id("api-key", &key.name)
        } else {
            key.id.clone()
        };
        add_legacy_route(
            &mut config.state.account_routing,
            RouteConsumerKind::ApiKey,
            &key_id,
            &primary_id,
        );
    }
    migrate_legacy_proxy_accounts(config, Some(&primary_id));
    true
}

fn migrate_legacy_proxy_accounts(config: &mut AppConfig, primary_id: Option<&str>) -> bool {
    let legacy = std::mem::take(&mut config.state.auth_proxy_oauth);
    if legacy.is_empty() && primary_id.is_none() {
        return false;
    }
    for account in &config.state.auth_proxy_accounts {
        let downstream_id = if account.id.is_empty() {
            derived_record_id("auth-proxy-account", &account.name)
        } else {
            account.id.clone()
        };
        let target_id = if let Some(credentials) = legacy.get(&account.id) {
            let existing = credentials.account_id.as_deref().and_then(|upstream_id| {
                config
                    .state
                    .codex_account_oauth
                    .iter()
                    .find(|(_, stored)| stored.account_id.as_deref() == Some(upstream_id))
                    .map(|(id, _)| id.clone())
            });
            if let Some(existing) = existing {
                existing
            } else {
                let id = legacy_codex_account_id(&account.id, credentials);
                let name = normalized_account_name(credentials.email.as_deref())
                    .or_else(|| normalized_account_name(Some(&account.name)))
                    .unwrap_or_else(|| "Codex 账户".into());
                let name = unique_account_name(
                    &name,
                    config
                        .state
                        .account_routing
                        .accounts
                        .iter()
                        .map(|account| account.name.as_str()),
                );
                config.state.account_routing.accounts.push(CodexAccount {
                    id: id.clone(),
                    name,
                    enabled: true,
                });
                config
                    .state
                    .codex_account_oauth
                    .insert(id.clone(), credentials.clone());
                id
            }
        } else if let Some(primary_id) = primary_id {
            primary_id.to_owned()
        } else {
            continue;
        };
        add_legacy_route(
            &mut config.state.account_routing,
            RouteConsumerKind::AuthProxy,
            &downstream_id,
            &target_id,
        );
    }
    true
}

fn legacy_codex_account_id(scope: &str, credentials: &StoredOAuthCredentials) -> String {
    derived_record_id(
        "legacy-codex-account",
        credentials.account_id.as_deref().unwrap_or(scope),
    )
}

fn add_legacy_route(
    state: &mut AccountRoutingState,
    consumer_type: RouteConsumerKind,
    consumer_id: &str,
    account_id: &str,
) {
    if consumer_id.is_empty()
        || state
            .routes
            .iter()
            .any(|entry| entry.consumer_type == consumer_type && entry.consumer_id == consumer_id)
    {
        return;
    }
    state.routes.push(RouteAssignment {
        consumer_type,
        consumer_id: consumer_id.into(),
        target_type: RouteTargetKind::Account,
        target_id: account_id.into(),
    });
}

fn validate_config(config: &AppConfig) -> Result<()> {
    config
        .server
        .bind
        .parse::<SocketAddr>()
        .context("server.bind must be an IP socket address")?;
    validate_origin(&config.server.public_origin, "server.public_origin")?;
    if config.server.cors_origin.trim().is_empty() {
        bail!("server.cors_origin must not be empty");
    }
    HeaderValue::from_str(&config.server.cors_origin)
        .context("server.cors_origin must be a valid HTTP header value")?;
    if config.server.maintenance_interval_seconds == 0 {
        bail!("server.maintenance_interval_seconds must be greater than zero");
    }
    if config.usage_tracking.database_path.trim().is_empty() {
        bail!("usage_tracking.database_path must not be empty");
    }
    validate_model_prices(&config.usage_tracking.model_prices)
        .context("usage_tracking.model_prices is invalid")?;
    if !valid_admin_path(&config.admin.path) {
        bail!("admin.path must be 1-128 URL-safe characters");
    }
    if config.admin.secret.trim().is_empty() {
        bail!("admin.secret must not be empty");
    }
    if let Some(proxy) = config.upstream.chatgpt_proxy.as_deref() {
        ChatgptProxy::parse(proxy).context("upstream.chatgpt_proxy is invalid")?;
    }
    let resets = Url::parse(&config.upstream.codex_resets_url)
        .context("upstream.codex_resets_url must be a valid URL")?;
    if resets.scheme() != "https" || resets.host_str().is_none() {
        bail!("upstream.codex_resets_url must be HTTPS");
    }
    if let Some(endpoint) = config.notifications.bark_push_url.as_deref() {
        parse_bark_push_url(endpoint)
            .map_err(|_| anyhow::anyhow!("notifications.bark_push_url is invalid"))?;
    }
    match (
        config.notifications.dingtalk_webhook_url.as_deref(),
        config.notifications.dingtalk_secret.as_deref(),
    ) {
        (Some(webhook), Some(secret)) => {
            signed_dingtalk_webhook(webhook, secret, 0)
                .map_err(|_| anyhow::anyhow!("DingTalk notification configuration is invalid"))?;
        }
        (None, None) => {}
        _ => bail!(
            "notifications.dingtalk_webhook_url and notifications.dingtalk_secret must be set together"
        ),
    }
    Ok(())
}

fn validate_origin(value: &str, name: &str) -> Result<()> {
    let url = Url::parse(value).with_context(|| format!("{name} must be a valid origin"))?;
    if url.origin().ascii_serialization() != value || url.path() != "/" || url.query().is_some() {
        bail!("{name} must be an exact HTTP or HTTPS origin");
    }
    if !matches!(url.scheme(), "http" | "https") {
        bail!("{name} must use HTTP or HTTPS");
    }
    Ok(())
}

fn valid_admin_path(value: &str) -> bool {
    let value = value.trim();
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

async fn persist(path: &Path, config: &AppConfig) -> Result<()> {
    let serialized = toml::to_string_pretty(config).context("failed to serialize configuration")?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.toml");
    let temporary = path.with_file_name(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .await
        .context("failed to create temporary configuration")?;
    file.write_all(serialized.as_bytes())
        .await
        .context("failed to write configuration")?;
    file.flush()
        .await
        .context("failed to flush configuration")?;
    file.sync_all()
        .await
        .context("failed to sync configuration")?;
    drop(file);
    if let Err(error) = fs::rename(&temporary, path).await {
        let _ = fs::remove_file(&temporary).await;
        return Err(error).context("failed to replace configuration");
    }
    Ok(())
}

fn config_read_error() -> ApiError {
    ApiError::new(500, "The configuration state is unavailable.")
        .with_kind("configuration_error")
        .with_code("invalid_configuration_state")
}

fn config_write_error() -> ApiError {
    ApiError::new(500, "The configuration file could not be updated.")
        .with_kind("configuration_error")
        .with_code("configuration_write_failed")
}

fn invalid_model_prices() -> ApiError {
    ApiError::new(400, "The model pricing configuration is invalid.")
        .with_kind("invalid_request_error")
        .with_code("invalid_model_prices")
}

fn default_bind() -> String {
    "127.0.0.1:8787".into()
}

fn default_public_origin() -> String {
    "http://127.0.0.1:8787".into()
}

fn default_cors_origin() -> String {
    "*".into()
}

const fn default_maintenance_interval() -> u64 {
    300
}

fn default_log_filter() -> String {
    "codex_router=info,tower_http=info".into()
}

fn default_usage_database_path() -> String {
    "usage.sqlite3".into()
}

fn default_codex_resets_url() -> String {
    "https://codex-resets.com/api/v1/status".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> AppConfig {
        AppConfig {
            server: ServerConfig::default(),
            admin: AdminConfig {
                path: "secret".into(),
                secret: "admin-secret".into(),
            },
            upstream: UpstreamConfig {
                chatgpt_proxy: None,
                codex_resets_url: default_codex_resets_url(),
            },
            usage_tracking: UsageTrackingConfig::default(),
            notifications: NotificationConfig::default(),
            state: PersistentState::default(),
        }
    }

    #[test]
    fn rejects_invalid_origins_proxies_and_cors_headers() {
        let mut value = config();
        value.server.public_origin = "https://router.example/path".into();
        assert!(validate_config(&value).is_err());
        value.server.public_origin = "https://router.example".into();
        value.upstream.chatgpt_proxy = Some("http://proxy.example:1080".into());
        assert!(validate_config(&value).is_err());
        value.upstream.chatgpt_proxy = Some("socks5://proxy.example:1080".into());
        value.server.cors_origin = "https://client.example\ninvalid".into();
        assert!(validate_config(&value).is_err());
    }

    #[test]
    fn migrates_primary_and_proxy_oauth_into_the_unified_account_pool() {
        let mut value = config();
        value.state.api_keys.push(ClientApiKey {
            id: "00000000-0000-4000-8000-000000000001".into(),
            name: "client".into(),
            key: "sk-client-value-123!".into(),
            enabled: true,
        });
        value.state.auth_proxy_accounts.push(AuthProxyAccount {
            id: "00000000-0000-4000-8000-000000000002".into(),
            name: "downstream".into(),
            account_id: "downstream-account".into(),
            enabled: true,
        });
        let credentials = StoredOAuthCredentials {
            version: 1,
            access_token: "access".into(),
            refresh_token: "refresh".into(),
            id_token: None,
            account_id: Some("upstream-account".into()),
            email: Some("owner@example.com".into()),
            expires_at: 2_000_000_000_000,
            updated_at: "2026-01-01T00:00:00.000Z".into(),
        };
        value.state.oauth = Some(credentials);

        assert!(migrate_legacy_accounts(&mut value));
        assert!(value.state.oauth.is_none());
        assert!(value.state.auth_proxy_oauth.is_empty());
        assert_eq!(value.state.account_routing.accounts.len(), 1);
        assert_eq!(value.state.codex_account_oauth.len(), 1);
        assert_eq!(value.state.account_routing.routes.len(), 2);
        assert!(
            value
                .state
                .account_routing
                .routes
                .iter()
                .any(|route| route.consumer_type == RouteConsumerKind::ApiKey)
        );
        assert!(
            value
                .state
                .account_routing
                .routes
                .iter()
                .any(|route| route.consumer_type == RouteConsumerKind::AuthProxy)
        );
    }
}

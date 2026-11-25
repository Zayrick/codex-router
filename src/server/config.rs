use std::{
    collections::{BTreeMap, HashSet},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use axum::http::HeaderValue;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use tokio::{fs, io::AsyncWriteExt, sync::RwLock};
use url::Url;

use crate::{
    application::CodexUsageMonitorState,
    auth::{
        ACCOUNT_ROUTING_KEY, AccountRoutingState, CODEX_ACCOUNT_OAUTH_KEY_PREFIX,
        CURRENT_ROUTING_VERSION, CURRENT_USERS_VERSION, CodexAccount, StateStore,
        StoredAccountRouting, StoredOAuthCredentials, StoredUsers, USERS_KEY, User,
        derived_record_id, new_record_id, normalized_account_name, unique_account_name,
        valid_record_id,
    },
    core::{ApiError, AppResult},
    upstream::{
        bark::parse_bark_push_url, codex::DEFAULT_CODEX_CLIENT_VERSION,
        dingtalk::signed_dingtalk_webhook,
    },
};

use super::chatgpt_proxy::ChatgptProxy;

/// State-store key of the last model price synchronization.
pub const PRICING_SYNC_KEY: &str = "PRICING_SYNC";
use super::pricing::{ModelPrice, normalized_model_prices, validate_model_prices};
use super::usage::DEFAULT_FAST_COST_MULTIPLIER;
use super::usage_store::CODEX_ACCOUNT_USAGE_KEY_PREFIX;

const MAX_FAST_COST_MULTIPLIER: f64 = 100.0;

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
    pub public_account: PublicAccountConfig,
    #[serde(default)]
    pub frontend: FrontendConfig,
    #[serde(default)]
    pub session: SessionConfig,
    #[serde(default)]
    pub state: PersistentState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontendConfig {
    /// Directory containing the built web console (`index.html` and `assets/`).
    #[serde(default = "default_frontend_dir")]
    pub dir: String,
}

impl Default for FrontendConfig {
    fn default() -> Self {
        Self {
            dir: default_frontend_dir(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    /// SQLite database holding web console sessions.
    #[serde(default = "default_session_database_path")]
    pub database_path: String,
    /// Allows administrator sign-in when `server.public_origin` is plain HTTP
    /// on a non-loopback host. Session cookies can then be sniffed in transit.
    #[serde(default)]
    pub allow_insecure_http: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            database_path: default_session_database_path(),
            allow_insecure_http: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageTrackingConfig {
    #[serde(default = "default_usage_database_path")]
    pub database_path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_prices: Vec<ModelPrice>,
    /// Cost multiplier for Fast (priority tier) requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fast_cost_multiplier: Option<f64>,
}

impl UsageTrackingConfig {
    pub fn fast_cost_multiplier(&self) -> f64 {
        self.fast_cost_multiplier
            .unwrap_or(DEFAULT_FAST_COST_MULTIPLIER)
    }
}

impl Default for UsageTrackingConfig {
    fn default() -> Self {
        Self {
            database_path: default_usage_database_path(),
            model_prices: Vec::new(),
            fast_cost_multiplier: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_public_origin")]
    pub public_origin: String,
    /// Resolve each request's public origin at a trusted reverse proxy.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub public_origin_from_proxy: bool,
    #[serde(default = "default_cors_origin")]
    pub cors_origin: String,
    #[serde(default = "default_maintenance_interval")]
    pub maintenance_interval_seconds: u64,
    #[serde(default = "default_log_filter")]
    pub log_filter: String,
    /// Reverse proxies whose `X-Forwarded-For` header is trusted, as IPs or CIDRs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trusted_proxies: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            public_origin: default_public_origin(),
            public_origin_from_proxy: false,
            cors_origin: default_cors_origin(),
            maintenance_interval_seconds: default_maintenance_interval(),
            log_filter: default_log_filter(),
            trusted_proxies: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminConfig {
    /// Deprecated hidden path segment. When present, the old `/<path>/admin`
    /// page redirects to `/admin` and its API family is answered locally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chatgpt_proxy: Option<String>,
    #[serde(default = "default_codex_resets_url")]
    pub codex_resets_url: String,
    #[serde(default = "default_codex_client_version")]
    pub codex_client_version: String,
    /// Closes proxied WebSockets after this many seconds without messages
    /// while no response is being generated; 0 keeps them open.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub websocket_idle_timeout_seconds: u64,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

/// Accepted idle timeouts: 0 (never close) or this range, in seconds.
pub const WEBSOCKET_IDLE_TIMEOUT_RANGE: std::ops::RangeInclusive<u64> = 30..=86_400;

fn valid_websocket_idle_timeout(value: u64) -> bool {
    value == 0 || WEBSOCKET_IDLE_TIMEOUT_RANGE.contains(&value)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotificationConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_watch_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_reset_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_warning_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all_accounts: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub account_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bark_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bark_push_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dingtalk_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dingtalk_webhook_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dingtalk_secret: Option<String>,
}

impl NotificationConfig {
    pub fn reset_watch_is_enabled(&self) -> bool {
        self.reset_watch_enabled.unwrap_or(true)
    }

    pub fn quota_reset_is_enabled(&self) -> bool {
        self.quota_reset_enabled.unwrap_or(true)
    }

    pub fn usage_warning_is_enabled(&self) -> bool {
        self.usage_warning_enabled.unwrap_or(true)
    }

    pub fn includes_account(&self, account_id: &str) -> bool {
        self.all_accounts.unwrap_or(self.account_ids.is_empty())
            || self.account_ids.iter().any(|id| id == account_id)
    }

    pub fn bark_is_enabled(&self) -> bool {
        self.bark_enabled
            .unwrap_or_else(|| self.bark_push_url.is_some())
            && self.bark_push_url.is_some()
    }

    pub fn dingtalk_is_enabled(&self) -> bool {
        self.dingtalk_enabled.unwrap_or_else(|| {
            self.dingtalk_webhook_url.is_some() && self.dingtalk_secret.is_some()
        }) && self.dingtalk_webhook_url.is_some()
            && self.dingtalk_secret.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicAccountConfig {
    #[serde(default = "default_true")]
    pub show_quota: bool,
}

impl Default for PublicAccountConfig {
    fn default() -> Self {
        Self { show_quota: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminSettings {
    pub public_account: PublicAccountSettings,
    pub upstream: UpstreamSettings,
    pub usage: UsageSettings,
    pub notifications: AdminNotificationSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpstreamSettings {
    pub codex_client_version: String,
    /// See [`UpstreamConfig::websocket_idle_timeout_seconds`].
    #[serde(default)]
    pub websocket_idle_timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSettings {
    pub fast_cost_multiplier: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicAccountSettings {
    pub show_quota: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminNotificationSettings {
    pub reset_watch_enabled: bool,
    pub quota_reset_enabled: bool,
    pub usage_warning_enabled: bool,
    pub all_accounts: bool,
    pub account_ids: Vec<String>,
    pub reset_watch_api_url: String,
    pub bark: BarkNotificationSettings,
    pub dingtalk: DingTalkNotificationSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BarkNotificationSettings {
    pub enabled: bool,
    pub push_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DingTalkNotificationSettings {
    pub enabled: bool,
    pub webhook_url: String,
    pub secret: String,
}

impl From<&AppConfig> for AdminSettings {
    fn from(config: &AppConfig) -> Self {
        let known_accounts = config
            .state
            .account_routing
            .accounts
            .iter()
            .map(|account| account.id.as_str())
            .collect::<HashSet<_>>();
        let account_ids = config
            .notifications
            .account_ids
            .iter()
            .filter(|id| known_accounts.contains(id.as_str()))
            .cloned()
            .collect();
        Self {
            public_account: PublicAccountSettings {
                show_quota: config.public_account.show_quota,
            },
            upstream: UpstreamSettings {
                codex_client_version: config.upstream.codex_client_version.clone(),
                websocket_idle_timeout_seconds: config.upstream.websocket_idle_timeout_seconds,
            },
            usage: UsageSettings {
                fast_cost_multiplier: config.usage_tracking.fast_cost_multiplier(),
            },
            notifications: AdminNotificationSettings {
                reset_watch_enabled: config.notifications.reset_watch_is_enabled(),
                quota_reset_enabled: config.notifications.quota_reset_is_enabled(),
                usage_warning_enabled: config.notifications.usage_warning_is_enabled(),
                all_accounts: config
                    .notifications
                    .all_accounts
                    .unwrap_or(config.notifications.account_ids.is_empty()),
                account_ids,
                reset_watch_api_url: config.upstream.codex_resets_url.clone(),
                bark: BarkNotificationSettings {
                    enabled: config.notifications.bark_is_enabled(),
                    push_url: config
                        .notifications
                        .bark_push_url
                        .clone()
                        .unwrap_or_default(),
                },
                dingtalk: DingTalkNotificationSettings {
                    enabled: config.notifications.dingtalk_is_enabled(),
                    webhook_url: config
                        .notifications
                        .dingtalk_webhook_url
                        .clone()
                        .unwrap_or_default(),
                    secret: config
                        .notifications
                        .dingtalk_secret
                        .clone()
                        .unwrap_or_default(),
                },
            },
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersistentState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<StoredOAuthCredentials>,
    /// Downstream users and their credentials.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub users: Vec<User>,
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
    /// Outcome of the last model price synchronization (see the pricing admin API).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_sync: Option<serde_json::Value>,
    /// Random key binding web sessions to the credential used to sign in.
    /// Generated on first start; replacing it signs every console user out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_binding_key: Option<String>,
}

pub struct ConfigStore {
    path: PathBuf,
    config: RwLock<AppConfig>,
    /// Increments after every persisted change.
    revision: tokio::sync::watch::Sender<u64>,
    /// Mirror of `upstream.websocket_idle_timeout_seconds` readable without the lock.
    websocket_idle_timeout: Arc<AtomicU64>,
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
        let generated_binding_key = ensure_session_binding_key(&mut config);
        let dropped_routes = drop_legacy_routes(&mut config);
        if migrate_legacy_accounts(&mut config) | generated_binding_key | dropped_routes {
            persist(&path, &config)
                .await
                .with_context(|| format!("failed to migrate configuration {}", path.display()))?;
        }
        Ok(Arc::new(Self {
            path,
            websocket_idle_timeout: Arc::new(AtomicU64::new(
                config.upstream.websocket_idle_timeout_seconds,
            )),
            config: RwLock::new(config),
            revision: tokio::sync::watch::Sender::new(0),
        }))
    }

    /// The current WebSocket idle timeout, following settings changes.
    pub fn websocket_idle_timeout(&self) -> WebsocketIdleTimeout {
        WebsocketIdleTimeout(self.websocket_idle_timeout.clone())
    }

    /// Notifies after every persisted change to the configuration or stored state.
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.revision.subscribe()
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

    pub async fn replace_admin_settings(
        &self,
        settings: AdminSettings,
    ) -> AppResult<AdminSettings> {
        let settings = normalized_admin_settings(settings);
        let stored = settings.clone();
        self.update(move |config| {
            validate_admin_settings(&stored, config)?;
            config.public_account.show_quota = stored.public_account.show_quota;
            config.upstream.codex_client_version = stored.upstream.codex_client_version.clone();
            config.upstream.websocket_idle_timeout_seconds =
                stored.upstream.websocket_idle_timeout_seconds;
            config.usage_tracking.fast_cost_multiplier = Some(stored.usage.fast_cost_multiplier);
            config.upstream.codex_resets_url = stored.notifications.reset_watch_api_url.clone();
            config.notifications.reset_watch_enabled =
                Some(stored.notifications.reset_watch_enabled);
            config.notifications.quota_reset_enabled =
                Some(stored.notifications.quota_reset_enabled);
            config.notifications.usage_warning_enabled =
                Some(stored.notifications.usage_warning_enabled);
            config.notifications.all_accounts = Some(stored.notifications.all_accounts);
            config.notifications.account_ids = stored.notifications.account_ids.clone();
            config.notifications.bark_enabled = Some(stored.notifications.bark.enabled);
            config.notifications.bark_push_url =
                non_empty_value(&stored.notifications.bark.push_url);
            config.notifications.dingtalk_enabled = Some(stored.notifications.dingtalk.enabled);
            config.notifications.dingtalk_webhook_url =
                non_empty_value(&stored.notifications.dingtalk.webhook_url);
            config.notifications.dingtalk_secret =
                non_empty_value(&stored.notifications.dingtalk.secret);
            Ok(())
        })
        .await?;
        Ok(settings)
    }

    pub async fn import_codex_account(
        &self,
        credentials: StoredOAuthCredentials,
        name: Option<&str>,
    ) -> AppResult<CodexAccount> {
        self.update(move |config| {
            if config.state.codex_account_oauth.values().any(|stored| {
                stored.refresh_token == credentials.refresh_token
                    || stored.account_id.is_some() && stored.account_id == credentials.account_id
            }) {
                return Err(ApiError::new(409, "该 Codex 账户已存在，请勿重复导入。")
                    .with_kind("invalid_request_error")
                    .with_code("codex_account_conflict"));
            }
            let account = config
                .state
                .account_routing
                .add_account(new_record_id(), name.or(credentials.email.as_deref()))?;
            config
                .state
                .codex_account_oauth
                .insert(account.id.clone(), credentials);
            Ok(account)
        })
        .await
    }

    async fn update<T>(
        &self,
        operation: impl FnOnce(&mut AppConfig) -> AppResult<T>,
    ) -> AppResult<T> {
        let mut current = self.config.write().await;
        let mut next = current.clone();
        let result = operation(&mut next)?;
        persist(&self.path, &next)
            .await
            .map_err(|_| config_write_error())?;
        self.websocket_idle_timeout.store(
            next.upstream.websocket_idle_timeout_seconds,
            Ordering::Relaxed,
        );
        *current = next;
        drop(current);
        self.revision
            .send_modify(|revision| *revision = revision.wrapping_add(1));
        Ok(result)
    }
}

/// Live handle on the WebSocket idle timeout setting.
#[derive(Clone)]
pub struct WebsocketIdleTimeout(Arc<AtomicU64>);

impl WebsocketIdleTimeout {
    /// `None` when idle WebSockets stay open.
    pub fn get(&self) -> Option<std::time::Duration> {
        match self.0.load(Ordering::Relaxed) {
            0 => None,
            seconds => Some(std::time::Duration::from_secs(seconds)),
        }
    }

    #[cfg(test)]
    pub fn fixed(seconds: u64) -> Self {
        Self(Arc::new(AtomicU64::new(seconds)))
    }
}

fn normalized_admin_settings(mut settings: AdminSettings) -> AdminSettings {
    settings.upstream.codex_client_version =
        settings.upstream.codex_client_version.trim().to_owned();
    settings.notifications.reset_watch_api_url =
        settings.notifications.reset_watch_api_url.trim().to_owned();
    settings.notifications.bark.push_url = settings.notifications.bark.push_url.trim().to_owned();
    settings.notifications.dingtalk.webhook_url = settings
        .notifications
        .dingtalk
        .webhook_url
        .trim()
        .to_owned();
    settings.notifications.dingtalk.secret =
        settings.notifications.dingtalk.secret.trim().to_owned();
    let mut seen = HashSet::new();
    settings
        .notifications
        .account_ids
        .retain(|id| seen.insert(id.clone()));
    if settings.notifications.all_accounts {
        settings.notifications.account_ids.clear();
    }
    settings
}

fn validate_admin_settings(settings: &AdminSettings, config: &AppConfig) -> AppResult<()> {
    if !valid_fast_cost_multiplier(settings.usage.fast_cost_multiplier)
        || !valid_codex_client_version(&settings.upstream.codex_client_version)
        || !valid_websocket_idle_timeout(settings.upstream.websocket_idle_timeout_seconds)
    {
        return Err(invalid_admin_settings());
    }

    let reset_url = Url::parse(&settings.notifications.reset_watch_api_url)
        .map_err(|_| invalid_admin_settings())?;
    if reset_url.scheme() != "https" || reset_url.host_str().is_none() {
        return Err(invalid_admin_settings());
    }

    let known_accounts = config
        .state
        .account_routing
        .accounts
        .iter()
        .map(|account| account.id.as_str())
        .collect::<HashSet<_>>();
    if settings.notifications.account_ids.len() > known_accounts.len()
        || settings
            .notifications
            .account_ids
            .iter()
            .any(|id| !valid_record_id(id) || !known_accounts.contains(id.as_str()))
    {
        return Err(invalid_admin_settings());
    }

    if !settings.notifications.bark.push_url.is_empty() {
        parse_bark_push_url(&settings.notifications.bark.push_url)
            .map_err(|_| invalid_admin_settings())?;
    } else if settings.notifications.bark.enabled {
        return Err(invalid_admin_settings());
    }

    let dingtalk = &settings.notifications.dingtalk;
    match (dingtalk.webhook_url.is_empty(), dingtalk.secret.is_empty()) {
        (false, false) => {
            signed_dingtalk_webhook(&dingtalk.webhook_url, &dingtalk.secret, 0)
                .map_err(|_| invalid_admin_settings())?;
        }
        (true, true) if !dingtalk.enabled => {}
        _ => return Err(invalid_admin_settings()),
    }
    Ok(())
}

fn non_empty_value(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

#[async_trait]
impl StateStore for ConfigStore {
    async fn get(&self, key: &str) -> AppResult<Option<String>> {
        let config = self.config.read().await;
        let value = match key {
            USERS_KEY => serde_json::to_string(&StoredUsers {
                version: CURRENT_USERS_VERSION,
                users: config.state.users.clone(),
            })
            .map(Some),
            ACCOUNT_ROUTING_KEY => serde_json::to_string(&StoredAccountRouting {
                version: CURRENT_ROUTING_VERSION,
                state: config.state.account_routing.clone(),
            })
            .map(Some),
            PRICING_SYNC_KEY => config
                .state
                .pricing_sync
                .as_ref()
                .map(serde_json::to_string)
                .transpose(),
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
                USERS_KEY => {
                    let stored: StoredUsers =
                        serde_json::from_str(value).map_err(|_| config_read_error())?;
                    if stored.version != CURRENT_USERS_VERSION {
                        return Err(config_read_error());
                    }
                    config.state.users = stored.users;
                }
                ACCOUNT_ROUTING_KEY => {
                    let stored: StoredAccountRouting =
                        serde_json::from_str(value).map_err(|_| config_read_error())?;
                    if stored.version != CURRENT_ROUTING_VERSION {
                        return Err(config_read_error());
                    }
                    config.state.account_routing = stored.state;
                }
                PRICING_SYNC_KEY => {
                    config.state.pricing_sync =
                        Some(serde_json::from_str(value).map_err(|_| config_read_error())?);
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
                USERS_KEY => config.state.users.clear(),
                ACCOUNT_ROUTING_KEY => {
                    config.state.account_routing = AccountRoutingState::default()
                }
                PRICING_SYNC_KEY => config.state.pricing_sync = None,
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

/// Generates the persistent session binding key on first start.
fn ensure_session_binding_key(config: &mut AppConfig) -> bool {
    if config.state.session_binding_key.is_some() {
        return false;
    }
    let mut key = [0_u8; 32];
    key[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    key[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    config.state.session_binding_key = Some(URL_SAFE_NO_PAD.encode(key));
    true
}

impl AppConfig {
    /// Decoded session binding key; present after [`ConfigStore::load`].
    pub fn session_binding_key(&self) -> Option<[u8; 32]> {
        let encoded = self.state.session_binding_key.as_deref()?;
        URL_SAFE_NO_PAD.decode(encoded).ok()?.try_into().ok()
    }
}

/// Routes of the former API key and downstream account identities have no
/// user id; they are discarded together with those identities.
fn drop_legacy_routes(config: &mut AppConfig) -> bool {
    let routes = &mut config.state.account_routing.routes;
    let before = routes.len();
    routes.retain(|route| !route.user_id.is_empty());
    routes.len() != before
}

/// Moves OAuth credentials of the single-account and per-proxy layouts into
/// the Codex account pool.
fn migrate_legacy_accounts(config: &mut AppConfig) -> bool {
    let primary = config.state.oauth.take();
    let proxies = std::mem::take(&mut config.state.auth_proxy_oauth);
    if primary.is_none() && proxies.is_empty() {
        return false;
    }
    if let Some(credentials) = primary {
        let id = import_legacy_account(config, "primary", credentials);
        if let Some(usage) = config.state.usage.take() {
            config.state.account_usage.insert(id, usage);
        }
    }
    for (scope, credentials) in proxies {
        import_legacy_account(config, &scope, credentials);
    }
    true
}

fn import_legacy_account(
    config: &mut AppConfig,
    scope: &str,
    credentials: StoredOAuthCredentials,
) -> String {
    let existing = credentials.account_id.as_deref().and_then(|upstream_id| {
        config
            .state
            .codex_account_oauth
            .iter()
            .find(|(_, stored)| stored.account_id.as_deref() == Some(upstream_id))
            .map(|(id, _)| id.clone())
    });
    if let Some(existing) = existing {
        return existing;
    }
    let id = derived_record_id(
        "legacy-codex-account",
        credentials.account_id.as_deref().unwrap_or(scope),
    );
    let name = normalized_account_name(credentials.email.as_deref())
        .unwrap_or_else(|| "Codex 账户".into());
    let accounts = &mut config.state.account_routing.accounts;
    if !accounts.iter().any(|entry| entry.id == id) {
        let name = unique_account_name(&name, accounts.iter().map(|account| account.name.as_str()));
        accounts.push(CodexAccount {
            id: id.clone(),
            name,
            enabled: true,
            proxy: None,
        });
    }
    config
        .state
        .codex_account_oauth
        .insert(id.clone(), credentials);
    id
}

fn valid_fast_cost_multiplier(value: f64) -> bool {
    value.is_finite() && value > 0.0 && value <= MAX_FAST_COST_MULTIPLIER
}

fn valid_codex_client_version(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    value.len() <= 64
        && parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn validate_config(config: &AppConfig) -> Result<()> {
    config
        .server
        .bind
        .parse::<SocketAddr>()
        .context("server.bind must be an IP socket address")?;
    validate_origin(&config.server.public_origin, "server.public_origin")?;
    if config.server.public_origin_from_proxy && config.server.trusted_proxies.is_empty() {
        bail!("server.public_origin_from_proxy requires server.trusted_proxies");
    }
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
    if !config
        .usage_tracking
        .fast_cost_multiplier
        .is_none_or(valid_fast_cost_multiplier)
    {
        bail!("usage_tracking.fast_cost_multiplier must be greater than 0 and at most 100");
    }
    if config
        .admin
        .path
        .as_deref()
        .is_some_and(|path| !valid_admin_path(path))
    {
        bail!("admin.path must be 1-128 URL-safe characters");
    }
    if config.frontend.dir.trim().is_empty() {
        bail!("frontend.dir must not be empty");
    }
    if config.session.database_path.trim().is_empty() {
        bail!("session.database_path must not be empty");
    }
    if let Some(key) = config.state.session_binding_key.as_deref()
        && URL_SAFE_NO_PAD
            .decode(key)
            .ok()
            .is_none_or(|bytes| bytes.len() != 32)
    {
        bail!("state.session_binding_key must be 32 bytes of unpadded base64url");
    }
    for proxy in &config.server.trusted_proxies {
        parse_trusted_proxy(proxy)
            .with_context(|| format!("server.trusted_proxies entry {proxy:?} is invalid"))?;
    }
    if config.admin.secret.trim().is_empty() {
        bail!("admin.secret must not be empty");
    }
    if let Some(proxy) = config.upstream.chatgpt_proxy.as_deref() {
        ChatgptProxy::parse(proxy).context("upstream.chatgpt_proxy is invalid")?;
    }
    if !valid_codex_client_version(&config.upstream.codex_client_version) {
        bail!("upstream.codex_client_version must use the numeric major.minor.patch format");
    }
    if !valid_websocket_idle_timeout(config.upstream.websocket_idle_timeout_seconds) {
        bail!("upstream.websocket_idle_timeout_seconds must be 0 or between 30 and 86400");
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
    let mut notification_account_ids = HashSet::new();
    if config
        .notifications
        .account_ids
        .iter()
        .any(|id| !valid_record_id(id) || !notification_account_ids.insert(id.as_str()))
    {
        bail!("notifications.account_ids contains an invalid or duplicate account id");
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

/// Parses an IP address or CIDR network from `server.trusted_proxies`.
pub fn parse_trusted_proxy(value: &str) -> Result<ipnet::IpNet> {
    let value = value.trim();
    if let Ok(network) = value.parse::<ipnet::IpNet>() {
        return Ok(network.trunc());
    }
    let address = value
        .parse::<std::net::IpAddr>()
        .context("expected an IP address or CIDR")?;
    Ok(ipnet::IpNet::from(address))
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

fn invalid_admin_settings() -> ApiError {
    ApiError::new(400, "设置配置无效，请检查地址、凭据与账户范围。")
        .with_kind("invalid_request_error")
        .with_code("invalid_admin_settings")
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

fn default_frontend_dir() -> String {
    "frontend/dist".into()
}

fn default_session_database_path() -> String {
    "sessions.sqlite3".into()
}

const fn default_true() -> bool {
    true
}

fn default_codex_resets_url() -> String {
    "https://codex-resets.com/api/v1/status".into()
}

fn default_codex_client_version() -> String {
    DEFAULT_CODEX_CLIENT_VERSION.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> AppConfig {
        AppConfig {
            server: ServerConfig::default(),
            admin: AdminConfig {
                path: Some("secret".into()),
                secret: "admin-secret".into(),
            },
            upstream: UpstreamConfig {
                chatgpt_proxy: None,
                codex_resets_url: default_codex_resets_url(),
                codex_client_version: default_codex_client_version(),
                websocket_idle_timeout_seconds: 0,
            },
            usage_tracking: UsageTrackingConfig::default(),
            notifications: NotificationConfig::default(),
            public_account: PublicAccountConfig::default(),
            frontend: FrontendConfig::default(),
            session: SessionConfig::default(),
            state: PersistentState::default(),
        }
    }

    #[test]
    fn rejects_invalid_origins_proxies_and_cors_headers() {
        let mut value = config();
        value.server.public_origin = "https://router.example/path".into();
        assert!(validate_config(&value).is_err());
        value.server.public_origin = "https://router.example".into();
        value.upstream.chatgpt_proxy = Some("ftp://proxy.example:1080".into());
        assert!(validate_config(&value).is_err());
        value.upstream.chatgpt_proxy = Some("socks5://proxy.example:1080".into());
        value.server.cors_origin = "https://client.example\ninvalid".into();
        assert!(validate_config(&value).is_err());
    }

    #[test]
    fn model_client_version_defaults_and_validates_settings() {
        let legacy: UpstreamConfig = toml::from_str("").unwrap();
        assert_eq!(legacy.codex_client_version, DEFAULT_CODEX_CLIENT_VERSION);
        let value = config();
        for invalid in ["", "0.168", "0.168.0.1", "0.168.x", "0.168.\n0"] {
            let mut settings = AdminSettings::from(&value);
            settings.upstream.codex_client_version = invalid.into();
            assert!(validate_admin_settings(&settings, &value).is_err());
        }
    }

    #[test]
    fn websocket_idle_timeout_defaults_off_and_validates_settings() {
        let legacy: UpstreamConfig = toml::from_str("").unwrap();
        assert_eq!(legacy.websocket_idle_timeout_seconds, 0);
        let value = config();
        for (seconds, valid) in [
            (0, true),
            (29, false),
            (30, true),
            (600, true),
            (86_401, false),
        ] {
            let mut settings = AdminSettings::from(&value);
            settings.upstream.websocket_idle_timeout_seconds = seconds;
            assert_eq!(
                validate_admin_settings(&settings, &value).is_ok(),
                valid,
                "{seconds}"
            );
        }
        // Settings saved by an older console omit the field and keep it off.
        let settings: UpstreamSettings =
            serde_json::from_str(r#"{"codexClientVersion":"0.168.0"}"#).unwrap();
        assert_eq!(settings.websocket_idle_timeout_seconds, 0);
    }

    #[test]
    fn admin_settings_keep_legacy_notification_defaults_and_validate_account_scope() {
        let mut value = config();
        value.notifications.bark_push_url = Some("https://api.day.app/device-key".into());
        value.state.account_routing.accounts.push(CodexAccount {
            id: "00000000-0000-4000-8000-000000000001".into(),
            name: "主账户".into(),
            enabled: true,
            proxy: None,
        });

        let mut settings = AdminSettings::from(&value);
        assert!(settings.public_account.show_quota);
        assert!(settings.notifications.reset_watch_enabled);
        assert!(settings.notifications.quota_reset_enabled);
        assert!(settings.notifications.usage_warning_enabled);
        assert!(settings.notifications.all_accounts);
        assert!(settings.notifications.bark.enabled);

        settings.notifications.all_accounts = false;
        settings.notifications.account_ids = vec!["00000000-0000-4000-8000-000000000001".into()];
        assert!(validate_admin_settings(&settings, &value).is_ok());
        settings.notifications.account_ids = vec!["00000000-0000-4000-8000-000000000002".into()];
        assert!(validate_admin_settings(&settings, &value).is_err());
    }

    #[tokio::test]
    async fn admin_settings_are_normalized_and_persisted() {
        let path = std::env::temp_dir().join(format!(
            "codex-router-settings-test-{}.toml",
            uuid::Uuid::new_v4()
        ));
        let mut value = config();
        let account_id = "00000000-0000-4000-8000-000000000001";
        value.state.account_routing.accounts.push(CodexAccount {
            id: account_id.into(),
            name: "主账户".into(),
            enabled: true,
            proxy: None,
        });
        fs::write(&path, toml::to_string_pretty(&value).unwrap())
            .await
            .unwrap();
        let store = ConfigStore::load(path.clone()).await.unwrap();
        let mut settings = AdminSettings::from(&store.snapshot().await);
        settings.public_account.show_quota = false;
        settings.upstream.codex_client_version = " 0.169.0 ".into();
        settings.notifications.reset_watch_enabled = false;
        settings.notifications.all_accounts = false;
        settings.notifications.account_ids = vec![account_id.into(), account_id.into()];
        settings.notifications.reset_watch_api_url = "  https://status.example.com/resets  ".into();
        settings.notifications.bark.enabled = true;
        settings.notifications.bark.push_url = " https://api.day.app/device-key ".into();

        let saved = store.replace_admin_settings(settings).await.unwrap();
        let snapshot = store.snapshot().await;

        assert!(!saved.public_account.show_quota);
        assert_eq!(saved.upstream.codex_client_version, "0.169.0");
        assert_eq!(snapshot.upstream.codex_client_version, "0.169.0");
        assert_eq!(saved.notifications.account_ids, vec![account_id]);
        assert_eq!(
            saved.notifications.reset_watch_api_url,
            "https://status.example.com/resets"
        );
        assert!(!snapshot.public_account.show_quota);
        assert_eq!(snapshot.notifications.reset_watch_enabled, Some(false));
        assert_eq!(
            snapshot.notifications.bark_push_url.as_deref(),
            Some("https://api.day.app/device-key")
        );

        let reloaded = ConfigStore::load(path.clone()).await.unwrap();
        assert_eq!(
            reloaded.snapshot().await.upstream.codex_client_version,
            "0.169.0"
        );
        let _ = fs::remove_file(path).await;
    }

    #[tokio::test]
    async fn pricing_sync_status_round_trips_through_the_state_store() {
        let path = std::env::temp_dir().join(format!(
            "codex-router-pricing-sync-test-{}.toml",
            uuid::Uuid::new_v4()
        ));
        fs::write(&path, toml::to_string_pretty(&config()).unwrap())
            .await
            .unwrap();
        let store = ConfigStore::load(path.clone()).await.unwrap();
        assert_eq!(store.get(PRICING_SYNC_KEY).await.unwrap(), None);
        store
            .put(
                PRICING_SYNC_KEY,
                r#"{"syncedAt":1,"matchedModels":["gpt-5"]}"#,
            )
            .await
            .unwrap();
        let reloaded = ConfigStore::load(path.clone()).await.unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&reloaded.get(PRICING_SYNC_KEY).await.unwrap().unwrap()).unwrap();
        assert_eq!(value["matchedModels"][0], "gpt-5");
        let _ = fs::remove_file(path).await;
    }

    #[test]
    fn migrates_primary_and_proxy_oauth_into_the_unified_account_pool() {
        let mut value = config();
        let credentials = |account: &str| StoredOAuthCredentials {
            version: 1,
            access_token: "access".into(),
            refresh_token: "refresh".into(),
            id_token: None,
            account_id: Some(account.into()),
            email: Some("owner@example.com".into()),
            expires_at: 2_000_000_000_000,
            updated_at: "2026-01-01T00:00:00.000Z".into(),
        };
        value.state.oauth = Some(credentials("upstream-account"));
        value
            .state
            .auth_proxy_oauth
            .insert("proxy-same".into(), credentials("upstream-account"));
        value
            .state
            .auth_proxy_oauth
            .insert("proxy-other".into(), credentials("other-account"));

        assert!(migrate_legacy_accounts(&mut value));
        assert!(value.state.oauth.is_none());
        assert!(value.state.auth_proxy_oauth.is_empty());
        assert_eq!(value.state.account_routing.accounts.len(), 2);
        assert_eq!(value.state.codex_account_oauth.len(), 2);
        assert!(value.state.account_routing.routes.is_empty());
    }

    #[tokio::test]
    async fn drops_routes_of_former_identities_on_load() {
        let path = std::env::temp_dir().join(format!(
            "codex-router-legacy-routes-test-{}.toml",
            uuid::Uuid::new_v4()
        ));
        let mut source = toml::to_string_pretty(&config()).unwrap();
        source.push_str(
            "\n[[state.account_routing.routes]]\nconsumerType = \"api_key\"\nconsumerId = \"00000000-0000-4000-8000-000000000001\"\ntargetType = \"account\"\ntargetId = \"00000000-0000-4000-8000-000000000002\"\n",
        );
        fs::write(&path, source).await.unwrap();
        let store = ConfigStore::load(path.clone()).await.unwrap();
        assert!(
            store
                .snapshot()
                .await
                .state
                .account_routing
                .routes
                .is_empty()
        );
        let _ = fs::remove_file(path).await;
    }
}

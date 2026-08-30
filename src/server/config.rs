use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::{fs, io::AsyncWriteExt, sync::RwLock};
use url::Url;

use crate::{
    application::CodexUsageMonitorState,
    auth::{AuthProxyAccount, ClientApiKey, StateStore, StoredOAuthCredentials},
    core::{ApiError, AppResult},
    upstream::codex::resolve_chatgpt_relay_url,
    upstream::{bark::parse_bark_push_url, dingtalk::signed_dingtalk_webhook},
};

use super::pricing::{ModelPrice, normalized_model_prices, validate_model_prices};

const OAUTH_KEY: &str = "oauth";
const API_KEYS_KEY: &str = "API_KEYS";
const CODEX_USAGE_KEY: &str = "CODEX_USAGE";
const AUTH_PROXY_OAUTH_PREFIX: &str = "oauth:auth-proxy:";

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
    pub chatgpt_relay_url: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<CodexUsageMonitorState>,
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
        let config: AppConfig = toml::from_str(&source)
            .with_context(|| format!("failed to parse configuration {}", path.display()))?;
        validate_config(&config)?;
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
            OAUTH_KEY => config
                .state
                .oauth
                .as_ref()
                .map(serde_json::to_string)
                .transpose(),
            API_KEYS_KEY => serde_json::to_string(&StoredApiKeys {
                version: 2,
                keys: config.state.api_keys.clone(),
                auth_proxy_accounts: config.state.auth_proxy_accounts.clone(),
            })
            .map(Some),
            CODEX_USAGE_KEY => config
                .state
                .usage
                .as_ref()
                .map(serde_json::to_string)
                .transpose(),
            _ => key
                .strip_prefix(AUTH_PROXY_OAUTH_PREFIX)
                .and_then(|id| config.state.auth_proxy_oauth.get(id))
                .map(serde_json::to_string)
                .transpose(),
        };
        value.map_err(|_| config_read_error())
    }

    async fn put(&self, key: &str, value: &str) -> AppResult<()> {
        self.update(|config| {
            match key {
                OAUTH_KEY => {
                    config.state.oauth =
                        Some(serde_json::from_str(value).map_err(|_| config_read_error())?);
                }
                API_KEYS_KEY => {
                    let stored: StoredApiKeys =
                        serde_json::from_str(value).map_err(|_| config_read_error())?;
                    config.state.api_keys = stored.keys;
                    config.state.auth_proxy_accounts = stored.auth_proxy_accounts;
                }
                CODEX_USAGE_KEY => {
                    config.state.usage =
                        Some(serde_json::from_str(value).map_err(|_| config_read_error())?);
                }
                _ => {
                    let id = key
                        .strip_prefix(AUTH_PROXY_OAUTH_PREFIX)
                        .filter(|id| !id.is_empty())
                        .ok_or_else(config_read_error)?;
                    let credentials =
                        serde_json::from_str(value).map_err(|_| config_read_error())?;
                    config
                        .state
                        .auth_proxy_oauth
                        .insert(id.to_owned(), credentials);
                }
            }
            Ok(())
        })
        .await
    }

    async fn delete(&self, key: &str) -> AppResult<()> {
        self.update(|config| {
            match key {
                OAUTH_KEY => config.state.oauth = None,
                API_KEYS_KEY => {
                    config.state.api_keys.clear();
                    config.state.auth_proxy_accounts.clear();
                }
                CODEX_USAGE_KEY => config.state.usage = None,
                _ => {
                    let id = key
                        .strip_prefix(AUTH_PROXY_OAUTH_PREFIX)
                        .filter(|id| !id.is_empty())
                        .ok_or_else(config_read_error)?;
                    config.state.auth_proxy_oauth.remove(id);
                }
            }
            Ok(())
        })
        .await
    }
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
    resolve_chatgpt_relay_url(&config.upstream.chatgpt_relay_url, "/", "")
        .map_err(|_| anyhow::anyhow!("upstream.chatgpt_relay_url must be an exact HTTPS origin"))?;
    if config.server.public_origin == config.upstream.chatgpt_relay_url {
        bail!("server.public_origin and upstream.chatgpt_relay_url must differ");
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
                chatgpt_relay_url: "https://relay.example".into(),
                codex_resets_url: default_codex_resets_url(),
            },
            usage_tracking: UsageTrackingConfig::default(),
            notifications: NotificationConfig::default(),
            state: PersistentState::default(),
        }
    }

    #[test]
    fn round_trips_runtime_state_in_toml() {
        let mut config = config();
        config.state.oauth = Some(StoredOAuthCredentials {
            version: 1,
            access_token: "plain-access-token".into(),
            refresh_token: "plain-refresh-token".into(),
            id_token: None,
            account_id: Some("account".into()),
            email: None,
            expires_at: 2_000_000_000_000,
            updated_at: "2033-05-18T03:33:20.000Z".into(),
        });
        let serialized = toml::to_string_pretty(&config).unwrap();
        let decoded: AppConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(
            decoded.state.oauth.unwrap().access_token,
            "plain-access-token"
        );
    }

    #[test]
    fn rejects_non_origin_relay_and_public_urls() {
        let mut value = config();
        value.server.public_origin = "https://router.example/path".into();
        assert!(validate_config(&value).is_err());
        value.server.public_origin = "https://router.example".into();
        value.upstream.chatgpt_relay_url = "https://relay.example/path".into();
        assert!(validate_config(&value).is_err());
    }
}

use std::sync::Arc;

use anyhow::{Context, Result};

use crate::{
    auth::{CodexAccount, RoutingRepository},
    core::{ApiError, AppResult},
};

use super::{
    account_router::AccountRouter,
    chatgpt_proxy::{ChatgptProxy, ChatgptTransport, ChatgptTransports, upstream_client_builder},
    config::ConfigStore,
    console_security::LoginThrottle,
    live::LiveHub,
    relay_identity::AccountOwnership,
    session_store::SessionStore,
    usage::UsageStore,
};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<ConfigStore>,
    pub client: reqwest::Client,
    /// Upstream transports: the default route plus one per account proxy.
    pub chatgpt: ChatgptTransports,
    pub usage: UsageStore,
    pub account_router: AccountRouter,
    /// Proves relayed callers own the ChatGPT account they claim.
    pub relay_ownership: AccountOwnership,
    pub sessions: SessionStore,
    /// Connections currently being proxied, for the console's live view.
    pub live: LiveHub,
    pub login_throttle: Arc<LoginThrottle>,
    /// Serializes console sign-in with credential changes, so a sign-in can
    /// never observe a credential that is being disabled, deleted, or rotated.
    pub credential_lock: Arc<tokio::sync::Mutex<()>>,
}

impl AppState {
    pub async fn new(config: Arc<ConfigStore>) -> Result<Self> {
        let client_builder = upstream_client_builder;
        let client = client_builder()
            .build()
            .context("failed to create HTTP client")?;
        let snapshot = config.snapshot().await;
        let proxy = snapshot
            .upstream
            .chatgpt_proxy
            .as_deref()
            .map(ChatgptProxy::parse)
            .transpose()
            .context("upstream.chatgpt_proxy is invalid")?;
        let chatgpt_client = if let Some(proxy) = proxy.as_ref() {
            client_builder()
                .proxy(proxy.http_proxy())
                .build()
                .context("failed to create proxied ChatGPT HTTP client")?
        } else {
            client.clone()
        };
        let chatgpt = ChatgptTransports::new(ChatgptTransport::new(
            chatgpt_client,
            proxy,
            config.websocket_idle_timeout(),
        ));
        let usage_path = config.resolve_path(&snapshot.usage_tracking.database_path);
        let usage = UsageStore::open(usage_path)?;
        let sessions = SessionStore::open(config.resolve_path(&snapshot.session.database_path))?;
        let account_router = AccountRouter::default();
        account_router
            .restore_quota_snapshots(config.as_ref())
            .await?;
        Ok(Self {
            config,
            client,
            chatgpt,
            usage,
            account_router,
            relay_ownership: AccountOwnership::default(),
            sessions,
            live: LiveHub::default(),
            login_throttle: Arc::default(),
            credential_lock: Arc::default(),
        })
    }
}

impl AppState {
    /// Transport for all upstream traffic of `account` (its proxy, if any).
    pub fn account_transport(&self, account: &CodexAccount) -> AppResult<ChatgptTransport> {
        self.chatgpt
            .for_proxy(account.proxy.as_deref())
            .map_err(|_| invalid_account_proxy())
    }

    /// Like [`Self::account_transport`] for an account known only by ID.
    pub async fn account_transport_by_id(&self, account_id: &str) -> AppResult<ChatgptTransport> {
        let routing = RoutingRepository::new(self.config.as_ref()).read().await?;
        match routing
            .accounts
            .iter()
            .find(|account| account.id == account_id)
        {
            Some(account) => self.account_transport(account),
            None => Ok(self.chatgpt.default_transport().clone()),
        }
    }
}

fn invalid_account_proxy() -> ApiError {
    ApiError::new(502, "The Codex account's upstream proxy is invalid.")
        .with_kind("upstream_error")
        .with_code("account_proxy_invalid")
}

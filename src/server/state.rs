use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};

use super::{
    chatgpt_proxy::{ChatgptProxy, ChatgptTransport},
    config::ConfigStore,
    usage::UsageStore,
};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<ConfigStore>,
    pub client: reqwest::Client,
    pub chatgpt: ChatgptTransport,
    pub usage: UsageStore,
}

impl AppState {
    pub async fn new(config: Arc<ConfigStore>) -> Result<Self> {
        let client_builder = || {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
        };
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
        let chatgpt = ChatgptTransport::new(chatgpt_client, proxy);
        let usage_path = config.resolve_path(&snapshot.usage_tracking.database_path);
        let usage = UsageStore::open(usage_path)?;
        Ok(Self {
            config,
            client,
            chatgpt,
            usage,
        })
    }
}

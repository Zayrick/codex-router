use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};

use super::{config::ConfigStore, usage::UsageStore};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<ConfigStore>,
    pub client: reqwest::Client,
    pub usage: UsageStore,
}

impl AppState {
    pub async fn new(config: Arc<ConfigStore>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .build()
            .context("failed to create HTTP client")?;
        let snapshot = config.snapshot().await;
        let usage_path = config.resolve_path(&snapshot.usage_tracking.database_path);
        let usage = UsageStore::open(usage_path)?;
        Ok(Self {
            config,
            client,
            usage,
        })
    }
}

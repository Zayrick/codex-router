use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};

use super::config::ConfigStore;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<ConfigStore>,
    pub client: reqwest::Client,
}

impl AppState {
    pub fn new(config: Arc<ConfigStore>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .build()
            .context("failed to create HTTP client")?;
        Ok(Self { config, client })
    }
}

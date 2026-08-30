//! Native server adapters.

mod account;
mod admin;
mod api;
mod body;
mod codex;
pub mod config;
mod frontend;
mod oauth;
mod pricing;
mod provider_error;
mod relay;
mod response;
mod router;
mod scheduled;
mod state;
mod stream;
mod usage;
mod usage_store;
mod websocket;

use std::path::PathBuf;

use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

use self::{config::ConfigStore, state::AppState};

pub async fn run(config_path: PathBuf) -> Result<()> {
    let config = ConfigStore::load(config_path).await?;
    let snapshot = config.snapshot().await;
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_new(&snapshot.server.log_filter)
                .context("server.log_filter is invalid")?,
        )
        .try_init()
        .map_err(|error| anyhow::anyhow!("failed to initialize tracing: {error}"))?;

    let bind = config.bind_address().await?;
    let state = AppState::new(config.clone()).await?;
    let usage_database = state.usage.path().display().to_string();
    let maintenance = scheduled::spawn(state.clone(), snapshot.server.maintenance_interval_seconds);
    let app = router::build(state);
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("failed to bind {bind}"))?;
    tracing::info!(
        event = "server_started",
        bind = %bind,
        config = %config.path().display(),
        usage_database = %usage_database
    );
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("HTTP server failed");
    maintenance.abort();
    result
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        } else {
            std::future::pending::<()>().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

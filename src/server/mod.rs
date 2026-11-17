//! Native server adapters.

mod account;
mod account_router;
mod admin;
mod api;
mod body;
mod chatgpt_proxy;
mod codex;
pub mod config;
mod console;
mod console_security;
mod frontend;
mod oauth;
mod pricing;
mod provider_error;
mod relay;
mod response;
mod router;
mod scheduled;
mod session_store;
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
    warn_about_console_configuration(&config, &snapshot);
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
    let result = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("HTTP server failed");
    maintenance.abort();
    result
}

fn warn_about_console_configuration(config: &ConfigStore, snapshot: &config::AppConfig) {
    if let Some(path) = snapshot.admin.path.as_deref() {
        tracing::warn!(
            event = "deprecated_configuration",
            setting = "admin.path",
            "the web console now lives at /admin; /{path}/admin only redirects there and admin.path can be removed"
        );
    }
    if console::insecure_public_origin(snapshot) {
        tracing::warn!(
            event = "insecure_console_origin",
            public_origin = %snapshot.server.public_origin,
            "administrator sign-in is disabled over plain HTTP; publish the console over HTTPS or set session.allow_insecure_http"
        );
    }
    let index = config
        .resolve_path(&snapshot.frontend.dir)
        .join("index.html");
    if !index.is_file() {
        tracing::warn!(
            event = "frontend_missing",
            index = %index.display(),
            "the web console is not installed; /admin will return 503 until frontend.dir contains a build"
        );
    }
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

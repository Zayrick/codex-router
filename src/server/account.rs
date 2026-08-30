use axum::response::Response;
use serde::Serialize;
use url::Url;

use crate::{
    application::{MatchedPublicAccountRoute, MonitoredQuotaWindow, PublicAccountRoute},
    auth::{
        ApiKeyRepository, AuthProxyAccount, OAuthRepository, constant_time_equal,
        matching_auth_proxy_account,
    },
    core::{ApiError, AppResult},
    upstream::codex::{
        CodexQuotaCategory, CodexQuotaWindow, CodexQuotaWindowKind, codex_subscription_from_usage,
    },
};

use super::{
    codex::CodexClient,
    config::AppConfig,
    frontend,
    oauth::current_time_ms,
    response,
    state::AppState,
    usage::{UsageBounds, UsageIdentityFilter, UsageRange},
    usage_store::CodexUsageStateRepository,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicAccountKind {
    ApiKey,
    AuthProxy,
}

impl PublicAccountKind {
    const fn identity_type(self) -> &'static str {
        match self {
            Self::ApiKey => "api_key",
            Self::AuthProxy => "auth_proxy",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublicAccount {
    id: String,
    kind: PublicAccountKind,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicAccountSummary {
    identity_type: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicQuotaSnapshot {
    sampled_at: i64,
    plan_type: Option<String>,
    windows: Vec<PublicQuotaWindow>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicQuotaWindow {
    id: String,
    category: crate::upstream::codex::CodexQuotaCategory,
    name: String,
    kind: crate::upstream::codex::CodexQuotaWindowKind,
    used_percent: Option<f64>,
    remaining_percent: Option<f64>,
    limit_window_seconds: Option<f64>,
    reset_at: Option<i64>,
}

pub async fn handle_public_account(
    matched: MatchedPublicAccountRoute,
    client_url: &Url,
    config: &AppConfig,
    state: &AppState,
) -> Option<Response> {
    let account = match resolve_account(&matched.credential, state).await {
        Ok(Some(account)) => account,
        Ok(None) => return None,
        Err(error) => {
            tracing::warn!(
                event = "public_account_resolve",
                status = "failed",
                error = %error
            );
            return None;
        }
    };

    if matched.route == PublicAccountRoute::Page {
        return Some(frontend::application_page());
    }

    Some(
        match account_dashboard(&account, client_url, config, state).await {
            Ok(output) => output,
            Err(error) => response::api_error(&error),
        },
    )
}

async fn resolve_account(credential: &str, state: &AppState) -> AppResult<Option<PublicAccount>> {
    let repository = ApiKeyRepository::new(state.config.as_ref());
    let keys = repository.read().await?;
    let mut matched_key = None;
    for key in keys {
        let matches = key.enabled && constant_time_equal(credential, &key.key);
        if matches && matched_key.is_none() {
            matched_key = Some(PublicAccount {
                id: key.id,
                kind: PublicAccountKind::ApiKey,
            });
        }
    }
    if matched_key.is_some() {
        return Ok(matched_key);
    }

    let accounts = repository.read_auth_proxy_accounts().await?;
    Ok(
        matching_auth_proxy_account(Some(credential), &accounts).map(
            |account: &AuthProxyAccount| PublicAccount {
                id: account.id.clone(),
                kind: PublicAccountKind::AuthProxy,
            },
        ),
    )
}

async fn account_dashboard(
    account: &PublicAccount,
    client_url: &Url,
    config: &AppConfig,
    state: &AppState,
) -> AppResult<Response> {
    let range = client_url
        .query_pairs()
        .find(|(name, _)| name == "range")
        .map(|(_, value)| value.into_owned());
    let range = UsageRange::parse(range.as_deref()).ok_or_else(invalid_usage_range)?;
    let now_ms = current_time_ms();
    let primary_oauth = OAuthRepository::new(state.config.as_ref());
    let account_oauth = (account.kind == PublicAccountKind::AuthProxy)
        .then(|| OAuthRepository::for_auth_proxy_account(state.config.as_ref(), &account.id));
    let account_credentials = match account_oauth.as_ref() {
        Some(repository) => repository.read().await?,
        None => None,
    };
    let use_account_oauth = account_credentials.as_ref().is_some_and(|credentials| {
        credentials.expires_at > now_ms
            && credentials
                .account_id
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty())
    });
    let identity = UsageIdentityFilter::parse(account.kind.identity_type(), &account.id)
        .expect("stored public account identity is valid");
    let selected_oauth = if use_account_oauth {
        account_oauth
            .as_ref()
            .expect("account OAuth exists when selected")
    } else {
        &primary_oauth
    };

    let quota = quota_snapshot(selected_oauth, use_account_oauth, config, state, now_ms).await;
    let bounds = (range == UsageRange::Cycle)
        .then(|| {
            quota
                .as_ref()
                .and_then(|quota| quota_cycle_bounds(quota, now_ms))
        })
        .flatten();
    let dashboard = state
        .usage
        .dashboard_with_options(
            range,
            Some(identity),
            bounds,
            &config.usage_tracking.model_prices,
        )
        .await
        .map_err(|error| {
            tracing::warn!(event = "public_account_usage", status = "failed", error = %error);
            usage_query_error()
        })?;

    response::json(
        &serde_json::json!({
            "account": PublicAccountSummary {
                identity_type: account.kind.identity_type(),
            },
            "usage": dashboard,
            "quota": quota,
        }),
        200,
    )
}

async fn quota_snapshot(
    oauth: &OAuthRepository<'_>,
    live: bool,
    config: &AppConfig,
    state: &AppState,
    now_ms: i64,
) -> Option<PublicQuotaSnapshot> {
    let result = if live {
        live_quota_snapshot(oauth, config, state, now_ms).await
    } else {
        cached_quota_snapshot(state).await
    };
    match result {
        Ok(snapshot) => snapshot,
        Err(error) => {
            tracing::warn!(
                event = "public_account_quota",
                status = "failed",
                error = %error
            );
            None
        }
    }
}

async fn live_quota_snapshot(
    oauth: &OAuthRepository<'_>,
    config: &AppConfig,
    state: &AppState,
    now_ms: i64,
) -> AppResult<Option<PublicQuotaSnapshot>> {
    let client = CodexClient::new(
        oauth,
        &state.client,
        config.upstream.chatgpt_relay_url.clone(),
    );
    let usage = client.fetch_usage().await?;
    let subscription =
        codex_subscription_from_usage(&usage.payload, usage.metadata, now_ms as f64)?;
    Ok(Some(PublicQuotaSnapshot {
        sampled_at: now_ms,
        plan_type: subscription.plan_type,
        windows: subscription
            .windows
            .iter()
            .map(PublicQuotaWindow::from)
            .collect(),
    }))
}

async fn cached_quota_snapshot(state: &AppState) -> AppResult<Option<PublicQuotaSnapshot>> {
    Ok(CodexUsageStateRepository::new(state.config.as_ref())
        .read()
        .await?
        .map(|snapshot| PublicQuotaSnapshot {
            sampled_at: snapshot.sampled_at,
            plan_type: snapshot.plan_type,
            windows: snapshot
                .windows
                .iter()
                .map(PublicQuotaWindow::from)
                .collect(),
        }))
}

impl From<&CodexQuotaWindow> for PublicQuotaWindow {
    fn from(window: &CodexQuotaWindow) -> Self {
        Self {
            id: window.id.clone(),
            category: window.category,
            name: window.name.clone(),
            kind: window.kind,
            used_percent: window.used_percent,
            remaining_percent: window.remaining_percent,
            limit_window_seconds: window.limit_window_seconds,
            reset_at: window.reset_at.and_then(f64_to_i64),
        }
    }
}

impl From<&MonitoredQuotaWindow> for PublicQuotaWindow {
    fn from(window: &MonitoredQuotaWindow) -> Self {
        Self {
            id: window.id.clone(),
            category: window.category,
            name: window.name.clone(),
            kind: window.kind,
            used_percent: window.used_percent,
            remaining_percent: window.remaining_percent,
            limit_window_seconds: window.limit_window_seconds,
            reset_at: window.reset_at,
        }
    }
}

fn quota_cycle_bounds(snapshot: &PublicQuotaSnapshot, now_ms: i64) -> Option<UsageBounds> {
    const WEEK_MS: i64 = 7 * 24 * 60 * 60 * 1_000;
    let mut end_at = snapshot
        .windows
        .iter()
        .find(|window| {
            window.category == CodexQuotaCategory::Codex
                && window.kind == CodexQuotaWindowKind::Weekly
                && window.reset_at.is_some()
        })?
        .reset_at?;
    let mut start_at = end_at.checked_sub(WEEK_MS)?;
    while end_at <= now_ms {
        start_at = start_at.checked_add(WEEK_MS)?;
        end_at = end_at.checked_add(WEEK_MS)?;
    }
    while start_at > now_ms {
        start_at = start_at.checked_sub(WEEK_MS)?;
        end_at = end_at.checked_sub(WEEK_MS)?;
    }
    UsageBounds::cycle(start_at, end_at, now_ms)
}

fn f64_to_i64(value: f64) -> Option<i64> {
    (value.is_finite() && value >= i64::MIN as f64 && value <= i64::MAX as f64)
        .then(|| value.round() as i64)
}

fn invalid_usage_range() -> ApiError {
    ApiError::new(400, "The public usage range is invalid.")
        .with_kind("invalid_request_error")
        .with_code("invalid_usage_range")
        .with_param("range")
}

fn usage_query_error() -> ApiError {
    ApiError::new(503, "Account usage is temporarily unavailable.")
        .with_kind("server_error")
        .with_code("usage_query_failed")
}

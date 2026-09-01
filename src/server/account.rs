use axum::{body::Body, http::Request, response::Response};
use serde::Serialize;

use crate::{
    application::MonitoredQuotaWindow,
    auth::{
        ApiKeyRepository, AuthProxyAccount, OAuthRepository, RouteConsumerKind,
        constant_time_equal, matching_auth_proxy_account,
    },
    core::{ApiError, AppResult},
    upstream::codex::{
        CodexQuotaCategory, CodexQuotaWindow, CodexQuotaWindowKind, codex_subscription_from_usage,
    },
};

use super::{
    body,
    codex::CodexClient,
    config::AppConfig,
    oauth::current_time_ms,
    response,
    state::AppState,
    usage::{UsageBounds, UsageFilters, UsageIdentityFilter, UsageRange},
    usage_store::CodexUsageStateRepository,
};

const MAX_PUBLIC_ACCOUNT_BODY_BYTES: usize = 8 * 1024;

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

    const fn route_consumer(self) -> RouteConsumerKind {
        match self {
            Self::ApiKey => RouteConsumerKind::ApiKey,
            Self::AuthProxy => RouteConsumerKind::AuthProxy,
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
    request: Request<Body>,
    config: &AppConfig,
    state: &AppState,
) -> Response {
    match public_account_dashboard(request, config, state).await {
        Ok(output) => output,
        Err(error) => response::api_error(&error),
    }
}

async fn public_account_dashboard(
    request: Request<Body>,
    config: &AppConfig,
    state: &AppState,
) -> AppResult<Response> {
    let (credential, range) = public_account_input(request).await?;
    let account = resolve_account(&credential, state)
        .await?
        .ok_or_else(invalid_account_credential)?;
    account_dashboard(&account, range, config, state).await
}

async fn public_account_input(request: Request<Body>) -> AppResult<(String, UsageRange)> {
    let (parts, body) = request.into_parts();
    let bytes =
        body::read_limited_body(&parts.headers, body, MAX_PUBLIC_ACCOUNT_BODY_BYTES).await?;
    let mut credential = None;
    let mut range = None;
    for (name, value) in url::form_urlencoded::parse(&bytes) {
        match name.as_ref() {
            "credential" if credential.is_none() => credential = Some(value.into_owned()),
            "range" if range.is_none() => range = Some(value.into_owned()),
            _ => {}
        }
    }
    let credential = credential
        .filter(|value| (1..=512).contains(&value.len()))
        .ok_or_else(invalid_account_credential_input)?;
    let range = range
        .as_deref()
        .and_then(|value| UsageRange::parse(Some(value)))
        .ok_or_else(invalid_usage_range)?;
    Ok((credential, range))
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
    range: UsageRange,
    config: &AppConfig,
    state: &AppState,
) -> AppResult<Response> {
    let now_ms = current_time_ms();
    let identity = UsageIdentityFilter::parse_downstream(account.kind.identity_type(), &account.id)
        .expect("stored public account identity is valid");
    let selected = state
        .account_router
        .inspect(
            state.config.as_ref(),
            account.kind.route_consumer(),
            &account.id,
            now_ms,
        )
        .await
        .ok()
        .flatten();
    let quota = match selected.as_ref() {
        Some(selected) => {
            let oauth = OAuthRepository::new(state.config.as_ref(), &selected.account.id);
            quota_snapshot(&oauth, &selected.account.id, state, now_ms).await
        }
        None => None,
    };
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
            UsageFilters::new(Some(identity), None),
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
    account_id: &str,
    state: &AppState,
    now_ms: i64,
) -> Option<PublicQuotaSnapshot> {
    match live_quota_snapshot(oauth, state, now_ms).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            tracing::warn!(
                event = "public_account_quota",
                status = "failed",
                error = %error
            );
            cached_quota_snapshot(state, account_id)
                .await
                .ok()
                .flatten()
        }
    }
}

async fn live_quota_snapshot(
    oauth: &OAuthRepository<'_>,
    state: &AppState,
    now_ms: i64,
) -> AppResult<Option<PublicQuotaSnapshot>> {
    let client = CodexClient::new(oauth, &state.chatgpt);
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

async fn cached_quota_snapshot(
    state: &AppState,
    account_id: &str,
) -> AppResult<Option<PublicQuotaSnapshot>> {
    Ok(
        CodexUsageStateRepository::new(state.config.as_ref(), account_id)
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
            }),
    )
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

fn invalid_account_credential_input() -> ApiError {
    ApiError::new(400, "An API key or account ID is required.")
        .with_kind("invalid_request_error")
        .with_code("invalid_account_credential_input")
        .with_param("credential")
}

fn invalid_account_credential() -> ApiError {
    ApiError::new(404, "The account does not exist or is disabled.")
        .with_kind("authentication_error")
        .with_code("invalid_account_credential")
}

fn usage_query_error() -> ApiError {
    ApiError::new(503, "Account usage is temporarily unavailable.")
        .with_kind("server_error")
        .with_code("usage_query_failed")
}

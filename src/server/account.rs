use axum::{body::Body, http::Request, response::Response};
use futures_util::{StreamExt, stream};
use serde::Serialize;

use crate::{
    application::MonitoredQuotaWindow,
    auth::{
        AccountRoutingState, ApiKeyRepository, AuthProxyAccount, CodexAccount, OAuthRepository,
        RouteConsumerKind, RouteTargetKind, RoutingRepository, constant_time_equal,
        matching_auth_proxy_account,
    },
    core::{ApiError, AppResult},
    upstream::codex::{CodexQuotaWindow, codex_subscription_from_usage},
};

use super::{
    body,
    codex::CodexClient,
    config::AppConfig,
    oauth::current_time_ms,
    response,
    state::AppState,
    usage::{UsageFilters, UsageIdentityFilter, UsageRange},
    usage_store::CodexUsageStateRepository,
};

const MAX_PUBLIC_ACCOUNT_BODY_BYTES: usize = 8 * 1024;
const PUBLIC_QUOTA_READ_CONCURRENCY: usize = 4;

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
    account_id: String,
    account_name: String,
    sampled_at: Option<i64>,
    plan_type: Option<String>,
    windows: Vec<PublicQuotaWindow>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicQuotaCollection {
    group: Option<PublicQuotaGroup>,
    accounts: Vec<PublicQuotaSnapshot>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicQuotaGroup {
    id: String,
    name: String,
}

#[derive(Debug)]
struct QuotaSnapshot {
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
        .filter(|range| *range != UsageRange::Cycle)
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
    let quota = if config.public_account.show_quota {
        match quota_collection(account, state, now_ms).await {
            Ok(quota) => Some(quota),
            Err(error) => {
                tracing::warn!(event = "public_account_quota_targets", status = "failed", error = %error);
                None
            }
        }
    } else {
        None
    };
    let dashboard = state
        .usage
        .dashboard_with_options(
            range,
            UsageFilters::new(Some(identity), None),
            None,
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

async fn quota_collection(
    public_account: &PublicAccount,
    state: &AppState,
    now_ms: i64,
) -> AppResult<PublicQuotaCollection> {
    let routing = RoutingRepository::new(state.config.as_ref()).read().await?;
    let (group, accounts) = quota_targets(routing, public_account);

    let accounts = stream::iter(accounts)
        .map(|account: CodexAccount| async move {
            let oauth = OAuthRepository::new(state.config.as_ref(), &account.id);
            let snapshot = quota_snapshot(&oauth, &account.id, state, now_ms).await;
            PublicQuotaSnapshot {
                account_id: account.id,
                account_name: account.name,
                sampled_at: snapshot.as_ref().map(|snapshot| snapshot.sampled_at),
                plan_type: snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.plan_type.clone()),
                windows: snapshot
                    .map(|snapshot| snapshot.windows)
                    .unwrap_or_default(),
            }
        })
        .buffered(PUBLIC_QUOTA_READ_CONCURRENCY)
        .collect()
        .await;

    Ok(PublicQuotaCollection { group, accounts })
}

fn quota_targets(
    routing: AccountRoutingState,
    public_account: &PublicAccount,
) -> (Option<PublicQuotaGroup>, Vec<CodexAccount>) {
    let Some(assignment) = routing.routes.iter().find(|route| {
        route.consumer_type == public_account.kind.route_consumer()
            && route.consumer_id == public_account.id
    }) else {
        return (None, Vec::new());
    };

    match assignment.target_type {
        RouteTargetKind::Account => {
            let accounts: Vec<CodexAccount> = routing
                .accounts
                .into_iter()
                .filter(|account| account.id == assignment.target_id)
                .collect();
            (None, accounts)
        }
        RouteTargetKind::Group => {
            let Some(group) = routing
                .groups
                .iter()
                .find(|group| group.id == assignment.target_id)
                .cloned()
            else {
                return (None, Vec::new());
            };
            let accounts: Vec<CodexAccount> = group
                .account_ids
                .iter()
                .filter_map(|id| {
                    routing
                        .accounts
                        .iter()
                        .find(|account| account.id == *id)
                        .cloned()
                })
                .collect();
            (
                Some(PublicQuotaGroup {
                    id: group.id,
                    name: group.name,
                }),
                accounts,
            )
        }
    }
}

async fn quota_snapshot(
    oauth: &OAuthRepository<'_>,
    account_id: &str,
    state: &AppState,
    now_ms: i64,
) -> Option<QuotaSnapshot> {
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
) -> AppResult<Option<QuotaSnapshot>> {
    let client = CodexClient::new(oauth, &state.chatgpt);
    let usage = client.fetch_usage().await?;
    let subscription =
        codex_subscription_from_usage(&usage.payload, usage.metadata, now_ms as f64)?;
    Ok(Some(QuotaSnapshot {
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
) -> AppResult<Option<QuotaSnapshot>> {
    Ok(
        CodexUsageStateRepository::new(state.config.as_ref(), account_id)
            .read()
            .await?
            .map(|snapshot| QuotaSnapshot {
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

#[cfg(test)]
mod tests {
    use crate::auth::{AccountGroup, RouteAssignment};

    use super::*;

    fn codex_account(id: &str, name: &str, enabled: bool) -> CodexAccount {
        CodexAccount {
            id: id.into(),
            name: name.into(),
            enabled,
        }
    }

    #[test]
    fn group_quota_targets_include_every_member_in_group_order() {
        let routing = AccountRoutingState {
            accounts: vec![
                codex_account("account-a", "主账户", true),
                codex_account("account-b", "备用账户", false),
                codex_account("account-c", "组外账户", true),
            ],
            groups: vec![AccountGroup {
                id: "group-1".into(),
                name: "生产组".into(),
                account_ids: vec!["account-b".into(), "account-a".into()],
                strategy: "round-robin".into(),
                session_affinity: false,
                session_affinity_ttl: "24h".into(),
            }],
            routes: vec![RouteAssignment {
                consumer_type: RouteConsumerKind::ApiKey,
                consumer_id: "client-1".into(),
                target_type: RouteTargetKind::Group,
                target_id: "group-1".into(),
            }],
        };
        let public_account = PublicAccount {
            id: "client-1".into(),
            kind: PublicAccountKind::ApiKey,
        };

        let (group, accounts) = quota_targets(routing, &public_account);

        assert_eq!(
            group,
            Some(PublicQuotaGroup {
                id: "group-1".into(),
                name: "生产组".into(),
            })
        );
        assert_eq!(
            accounts
                .iter()
                .map(|account| account.name.as_str())
                .collect::<Vec<_>>(),
            vec!["备用账户", "主账户"]
        );
    }

    #[tokio::test]
    async fn public_account_ranges_do_not_accept_the_legacy_cycle_filter() {
        let request = Request::builder()
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("credential=sk-test-value&range=cycle"))
            .unwrap();

        assert!(public_account_input(request).await.is_err());
    }
}

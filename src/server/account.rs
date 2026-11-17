use axum::response::Response;
use futures_util::{StreamExt, stream};
use serde::Serialize;

use crate::{
    application::MonitoredQuotaWindow,
    auth::{
        AccountRoutingState, CodexAccount, OAuthRepository, RouteTargetKind, RoutingRepository,
        UserRepository,
    },
    core::{ApiError, AppResult},
    upstream::codex::{CodexQuotaWindow, codex_subscription_from_usage},
};

use super::{
    codex::CodexClient,
    config::AppConfig,
    oauth::current_time_ms,
    quota::quota_overview,
    response,
    session_store::Principal,
    state::AppState,
    usage::{UsageFilters, UsageIdentityFilter, UsageRange},
    usage_analytics::AnalyticsScope,
    usage_store::CodexUsageStateRepository,
};

const PUBLIC_QUOTA_READ_CONCURRENCY: usize = 4;

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

/// The user behind a console session.
fn session_user(principal: &Principal) -> AppResult<&str> {
    match principal {
        Principal::User(id) => Ok(id),
        Principal::Admin => Err(user_session_required()),
    }
}

/// Usage dashboard of the user behind a session, across all their credentials.
/// The queried user always comes from the session, never from the request.
pub async fn signed_in_account_dashboard(
    principal: &Principal,
    range: Option<&str>,
    config: &AppConfig,
    state: &AppState,
) -> AppResult<Response> {
    let user_id = session_user(principal)?;
    let range = UsageRange::parse(range)
        .filter(|range| *range != UsageRange::Cycle)
        .ok_or_else(invalid_usage_range)?;
    account_dashboard(user_id, range, config, state).await
}

fn user_filter(user_id: &str) -> UsageIdentityFilter {
    UsageIdentityFilter::parse_downstream("user", user_id).expect("stored user id is valid")
}

/// Analytics scope limited to the signed-in user.
pub fn user_scope(principal: &Principal) -> AppResult<AnalyticsScope> {
    Ok(AnalyticsScope::Identity(user_filter(session_user(
        principal,
    )?)))
}

/// Quota forecasts of the accounts the signed-in identity is routed to.
pub async fn signed_in_account_quota(
    principal: &Principal,
    config: &AppConfig,
    state: &AppState,
) -> AppResult<Response> {
    let user_id = session_user(principal)?;
    let name = UserRepository::new(state.config.as_ref())
        .user(user_id)
        .await?
        .map(|user| user.name)
        .unwrap_or_default();
    let identity = serde_json::json!({ "name": name });
    let routing = RoutingRepository::new(state.config.as_ref()).read().await?;
    let route = routing
        .routes
        .iter()
        .find(|route| route.user_id == user_id)
        .map(|route| serde_json::json!({ "targetType": route.target_type, "targetId": route.target_id }));
    let group_settings = routing
        .routes
        .iter()
        .find(|route| route.user_id == user_id)
        .filter(|route| route.target_type == RouteTargetKind::Group)
        .and_then(|route| {
            routing
                .groups
                .iter()
                .find(|group| group.id == route.target_id)
                .cloned()
        })
        .map(|group| {
            serde_json::json!({
                "id": group.id,
                "name": group.name,
                "strategy": group.strategy,
                "sessionAffinity": group.session_affinity,
                "sessionAffinityTtl": group.session_affinity_ttl,
            })
        });
    if !config.public_account.show_quota {
        return response::json(
            &serde_json::json!({
                "visible": false,
                "identity": identity,
                "route": route,
                "group": group_settings,
            }),
            200,
        );
    }
    let (group, accounts) = quota_targets(routing, user_id);
    let ids = accounts
        .iter()
        .map(|account| account.id.clone())
        .collect::<Vec<_>>();
    let mut overview = quota_overview(state, config, Some(&ids), current_time_ms()).await?;
    overview.groups.retain(|shares| {
        group
            .as_ref()
            .is_some_and(|group| group.id == shares.group_id)
    });
    response::json(
        &serde_json::json!({
            "visible": true,
            "identity": identity,
            "route": route,
            "group": group_settings,
            "quota": overview,
        }),
        200,
    )
}

async fn account_dashboard(
    user_id: &str,
    range: UsageRange,
    config: &AppConfig,
    state: &AppState,
) -> AppResult<Response> {
    let now_ms = current_time_ms();
    let quota = if config.public_account.show_quota {
        match quota_collection(user_id, state, now_ms).await {
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
            UsageFilters::new(Some(user_filter(user_id)), None),
            None,
            &config.usage_tracking.model_prices,
            config.usage_tracking.fast_cost_multiplier(),
        )
        .await
        .map_err(|error| {
            tracing::warn!(event = "public_account_usage", status = "failed", error = %error);
            usage_query_error()
        })?;

    response::json(
        &serde_json::json!({
            "usage": dashboard,
            "quota": quota,
        }),
        200,
    )
}

async fn quota_collection(
    user_id: &str,
    state: &AppState,
    now_ms: i64,
) -> AppResult<PublicQuotaCollection> {
    let routing = RoutingRepository::new(state.config.as_ref()).read().await?;
    let (group, accounts) = quota_targets(routing, user_id);

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
    user_id: &str,
) -> (Option<PublicQuotaGroup>, Vec<CodexAccount>) {
    let Some(assignment) = routing.routes.iter().find(|route| route.user_id == user_id) else {
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

fn user_session_required() -> ApiError {
    ApiError::new(403, "This view is only available to user sessions.")
        .with_kind("permission_error")
        .with_code("user_session_required")
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
                user_id: "client-1".into(),
                target_type: RouteTargetKind::Group,
                target_id: "group-1".into(),
            }],
        };
        let (group, accounts) = quota_targets(routing, "client-1");

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
}

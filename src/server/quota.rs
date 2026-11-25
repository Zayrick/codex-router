//! Quota snapshots joined with recorded usage: cost of 1 %, burn rate and
//! projected exhaustion for every Codex account window.

use serde::Serialize;

use crate::{
    application::MonitoredQuotaWindow,
    auth::{AccountGroup, CodexAccount, RoutingRepository},
    core::AppResult,
    upstream::codex::{CodexQuotaCategory, CodexQuotaWindowKind},
};

use super::{
    account_health::HealthView, config::AppConfig, state::AppState, usage_analytics::WindowUsage,
    usage_store::CodexUsageStateRepository,
};

const HOUR_MS: i64 = 3_600_000;
/// Burn rate is measured over this many recent hours.
const BURN_RATE_HOURS: i64 = 3;
/// Below this share used, the cost of 1 % is too noisy to report.
const MIN_PERCENT_FOR_RATE: f64 = 1.0;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WindowForecast {
    pub id: String,
    pub category: CodexQuotaCategory,
    pub name: String,
    pub kind: CodexQuotaWindowKind,
    pub used_percent: Option<f64>,
    pub remaining_percent: Option<f64>,
    pub limit_window_seconds: Option<f64>,
    pub reset_at: Option<i64>,
    pub limit_reached: bool,
    pub window_start_at: Option<i64>,
    /// Usage recorded on this account since the window started.
    pub requests: i64,
    pub total_tokens: i64,
    pub cost_usd: f64,
    pub tokens_per_percent: Option<f64>,
    pub cost_per_percent: Option<f64>,
    pub burn_percent_per_hour: Option<f64>,
    pub exhaust_at: Option<i64>,
    pub exhausts_before_reset: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountQuota {
    pub account_id: String,
    pub account_name: String,
    pub enabled: bool,
    /// Taken out of routing after a 429 or an exhausted window.
    pub quarantined: bool,
    /// Capacity health learned from upstream overloads.
    pub health: HealthView,
    pub sampled_at: Option<i64>,
    pub plan_type: Option<String>,
    pub windows: Vec<WindowForecast>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MemberShare {
    pub account_id: String,
    pub share: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GroupShares {
    pub group_id: String,
    pub members: Vec<MemberShare>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuotaOverview {
    pub sampled_at: Option<i64>,
    pub accounts: Vec<AccountQuota>,
    pub groups: Vec<GroupShares>,
}

/// Quota of the given accounts (all accounts when `None`) with forecasts.
pub async fn quota_overview(
    state: &AppState,
    config: &AppConfig,
    only: Option<&[String]>,
    now_ms: i64,
) -> AppResult<QuotaOverview> {
    let routing = RoutingRepository::new(state.config.as_ref()).read().await?;
    let mut accounts = Vec::new();
    for account in routing
        .accounts
        .iter()
        .filter(|account| only.is_none_or(|ids| ids.contains(&account.id)))
    {
        accounts.push(account_quota(state, config, account, now_ms).await?);
    }
    let mut groups = Vec::new();
    for group in routing
        .groups
        .iter()
        .filter(|group| only.is_none_or(|ids| group.account_ids.iter().any(|id| ids.contains(id))))
    {
        groups.push(group_shares(state, group, &routing.accounts, now_ms).await);
    }
    Ok(QuotaOverview {
        sampled_at: accounts
            .iter()
            .filter_map(|account| account.sampled_at)
            .max(),
        accounts,
        groups,
    })
}

async fn group_shares(
    state: &AppState,
    group: &AccountGroup,
    accounts: &[CodexAccount],
    now_ms: i64,
) -> GroupShares {
    let members = group
        .account_ids
        .iter()
        .filter_map(|id| accounts.iter().find(|account| account.id == *id).cloned())
        .collect::<Vec<_>>();
    let shares = state
        .account_router
        .member_shares(group, &members, now_ms)
        .await;
    GroupShares {
        group_id: group.id.clone(),
        members: members
            .iter()
            .zip(shares)
            .map(|(account, share)| MemberShare {
                account_id: account.id.clone(),
                share,
            })
            .collect(),
    }
}

async fn account_quota(
    state: &AppState,
    config: &AppConfig,
    account: &CodexAccount,
    now_ms: i64,
) -> AppResult<AccountQuota> {
    let snapshot = CodexUsageStateRepository::new(state.config.as_ref(), &account.id)
        .read()
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(event = "quota_overview_snapshot", status = "failed", error = %error);
            None
        });
    let mut windows = Vec::new();
    if let Some(snapshot) = &snapshot {
        for window in &snapshot.windows {
            let start = window_start(window);
            let usage = match start {
                Some(start) => state
                    .usage
                    .account_window_usage(
                        account.id.clone(),
                        start,
                        now_ms - BURN_RATE_HOURS * HOUR_MS,
                        &config.usage_tracking.model_prices,
                        config.usage_tracking.fast_cost_multiplier(),
                    )
                    .await
                    .unwrap_or_else(|error| {
                        tracing::warn!(event = "quota_overview_usage", status = "failed", error = %error);
                        WindowUsage::default()
                    }),
                None => WindowUsage::default(),
            };
            windows.push(forecast(window, start, usage, snapshot.sampled_at, now_ms));
        }
    }
    Ok(AccountQuota {
        account_id: account.id.clone(),
        account_name: account.name.clone(),
        enabled: account.enabled,
        quarantined: state.account_router.is_quarantined(&account.id).await,
        health: state.account_router.health(&account.id, now_ms),
        sampled_at: snapshot.as_ref().map(|snapshot| snapshot.sampled_at),
        plan_type: snapshot.and_then(|snapshot| snapshot.plan_type),
        windows,
    })
}

fn window_start(window: &MonitoredQuotaWindow) -> Option<i64> {
    let length = window
        .limit_window_seconds
        .filter(|value| value.is_finite() && *value > 0.0)?;
    Some(window.reset_at? - (length * 1000.0) as i64)
}

fn forecast(
    window: &MonitoredQuotaWindow,
    window_start_at: Option<i64>,
    usage: WindowUsage,
    sampled_at: i64,
    now_ms: i64,
) -> WindowForecast {
    let used = window
        .used_percent
        .or(window.remaining_percent.map(|remaining| 100.0 - remaining))
        .map(|value| value.clamp(0.0, 100.0));
    let measurable = used.filter(|used| *used >= MIN_PERCENT_FOR_RATE);
    let tokens_per_percent = measurable
        .filter(|_| usage.total_tokens > 0)
        .map(|used| usage.total_tokens as f64 / used);
    let cost_per_percent = measurable
        .filter(|_| usage.cost_usd > 0.0)
        .map(|used| usage.cost_usd / used);
    let burn = match (tokens_per_percent, used, window_start_at) {
        (Some(per_percent), _, _) => {
            Some(usage.recent_tokens as f64 / BURN_RATE_HOURS as f64 / per_percent)
        }
        (None, Some(used), Some(start)) if sampled_at > start => {
            Some(used / ((sampled_at - start) as f64 / HOUR_MS as f64))
        }
        _ => None,
    };
    let exhaust_at = match (used, burn) {
        (Some(used), _) if used >= 100.0 || window.limit_reached => Some(sampled_at.min(now_ms)),
        (Some(used), Some(burn)) if burn > 1e-6 => {
            Some(sampled_at + ((100.0 - used) / burn * HOUR_MS as f64) as i64)
        }
        _ => None,
    };
    WindowForecast {
        id: window.id.clone(),
        category: window.category,
        name: window.name.clone(),
        kind: window.kind,
        used_percent: used,
        remaining_percent: window.remaining_percent,
        limit_window_seconds: window.limit_window_seconds,
        reset_at: window.reset_at,
        limit_reached: window.limit_reached,
        window_start_at,
        requests: usage.requests,
        total_tokens: usage.total_tokens,
        cost_usd: usage.cost_usd,
        tokens_per_percent,
        cost_per_percent,
        burn_percent_per_hour: burn,
        exhaust_at,
        exhausts_before_reset: match (exhaust_at, window.reset_at) {
            (Some(exhaust), Some(reset)) => exhaust < reset,
            _ => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(used: f64, reset_at: i64) -> MonitoredQuotaWindow {
        MonitoredQuotaWindow {
            id: "codex-primary".into(),
            category: CodexQuotaCategory::Codex,
            name: "Codex".into(),
            kind: CodexQuotaWindowKind::FiveHour,
            used_percent: Some(used),
            remaining_percent: Some(100.0 - used),
            limit_window_seconds: Some(5.0 * 3600.0),
            reset_at: Some(reset_at),
            allowed: Some(true),
            limit_reached: false,
            entry_alert_sent: false,
        }
    }

    #[test]
    fn projects_exhaustion_from_recent_burn() {
        let now = 10 * HOUR_MS;
        let window = window(50.0, now + 2 * HOUR_MS);
        let start = window_start(&window);
        assert_eq!(start, Some(now - 3 * HOUR_MS));
        // 1 000 000 tokens for 50 % → 20 000 per 1 %; 3 h used 600 000 → 10 %/h.
        let usage = WindowUsage {
            requests: 10,
            total_tokens: 1_000_000,
            cost_usd: 5.0,
            recent_tokens: 600_000,
        };
        let result = forecast(&window, start, usage, now, now);
        assert_eq!(result.tokens_per_percent, Some(20_000.0));
        assert_eq!(result.cost_per_percent, Some(0.1));
        assert_eq!(result.burn_percent_per_hour, Some(10.0));
        assert_eq!(result.exhaust_at, Some(now + 5 * HOUR_MS));
        assert!(!result.exhausts_before_reset);
    }

    #[test]
    fn falls_back_to_the_average_rate_without_usage() {
        let now = 10 * HOUR_MS;
        let window = window(60.0, now + 2 * HOUR_MS);
        let result = forecast(
            &window,
            window_start(&window),
            WindowUsage::default(),
            now,
            now,
        );
        // 60 % over 3 h → 20 %/h → 2 h to go, exactly at reset.
        assert_eq!(result.burn_percent_per_hour, Some(20.0));
        assert_eq!(result.exhaust_at, Some(now + 2 * HOUR_MS));
    }
}

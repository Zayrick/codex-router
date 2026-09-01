use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use axum::http::HeaderMap;
use tokio::sync::Mutex;

use crate::{
    application::CodexUsageMonitorState,
    auth::{
        AccountGroup, CodexAccount, FALLBACK_STRATEGY, OAuthRepository, RouteConsumerKind,
        RouteTargetKind, RoutingRepository, StateStore, WEIGHTED_ROUND_ROBIN_STRATEGY,
        session_affinity_ttl,
    },
    core::{ApiError, AppResult},
    upstream::codex::{CodexQuotaCategory, CodexSubscriptionInfo},
};

use super::usage_store::CodexUsageStateRepository;

const MAX_AFFINITY_BINDINGS: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAccountRoute {
    pub account: CodexAccount,
    pub group: Option<AccountGroup>,
}

#[derive(Clone, Default)]
pub struct AccountRouter {
    runtime: Arc<Mutex<RouterRuntime>>,
}

#[derive(Default)]
struct RouterRuntime {
    last_picked: HashMap<String, String>,
    affinity: HashMap<String, AffinityBinding>,
    fallback: HashMap<String, String>,
    weighted: HashMap<String, SmoothWeightedState>,
    quota: HashMap<String, AccountQuotaState>,
    quota_limited: HashSet<String>,
}

struct AffinityBinding {
    account_id: String,
    touched_at_ms: i64,
}

#[derive(Default)]
struct SmoothWeightedState {
    current: HashMap<String, f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuotaHealth {
    Unknown,
    Available,
    Exhausted,
}

#[derive(Debug, Clone, Copy)]
struct AccountQuotaState {
    health: QuotaHealth,
    average_ratio: Option<f64>,
}

impl AccountRouter {
    pub async fn resolve(
        &self,
        store: &dyn StateStore,
        consumer_type: RouteConsumerKind,
        consumer_id: &str,
        headers: &HeaderMap,
        now_ms: i64,
    ) -> AppResult<Option<ResolvedAccountRoute>> {
        self.resolve_internal(store, consumer_type, consumer_id, headers, now_ms, true)
            .await
    }

    pub async fn inspect(
        &self,
        store: &dyn StateStore,
        consumer_type: RouteConsumerKind,
        consumer_id: &str,
        now_ms: i64,
    ) -> AppResult<Option<ResolvedAccountRoute>> {
        self.resolve_internal(
            store,
            consumer_type,
            consumer_id,
            &HeaderMap::new(),
            now_ms,
            false,
        )
        .await
    }

    async fn resolve_internal(
        &self,
        store: &dyn StateStore,
        consumer_type: RouteConsumerKind,
        consumer_id: &str,
        headers: &HeaderMap,
        now_ms: i64,
        advance_schedule: bool,
    ) -> AppResult<Option<ResolvedAccountRoute>> {
        let routing = RoutingRepository::new(store).read().await?;
        let Some(assignment) = routing
            .routes
            .iter()
            .find(|route| route.consumer_type == consumer_type && route.consumer_id == consumer_id)
        else {
            return Ok(None);
        };
        match assignment.target_type {
            RouteTargetKind::Account => {
                let account = routing
                    .accounts
                    .into_iter()
                    .find(|entry| entry.id == assignment.target_id && entry.enabled)
                    .ok_or_else(route_unavailable)?;
                if self.is_quota_limited(&account.id).await {
                    return Err(route_unavailable());
                }
                Ok(Some(ResolvedAccountRoute {
                    account,
                    group: None,
                }))
            }
            RouteTargetKind::Group => {
                let group = routing
                    .groups
                    .into_iter()
                    .find(|entry| entry.id == assignment.target_id)
                    .ok_or_else(route_unavailable)?;
                let mut available = Vec::new();
                for account in routing.accounts {
                    if !account.enabled || !group.account_ids.contains(&account.id) {
                        continue;
                    }
                    let oauth = OAuthRepository::new(store, &account.id);
                    if oauth.require_valid(now_ms).await.is_ok() {
                        available.push(account);
                    }
                }
                if available.is_empty() {
                    return Err(route_unavailable());
                }
                available.sort_by(|left, right| left.id.cmp(&right.id));
                let limited = self.quota_limited_ids().await;
                available.retain(|account| !limited.contains(&account.id));
                if available.is_empty() {
                    return Err(route_unavailable());
                }
                let session_id = group
                    .session_affinity
                    .then(|| session_affinity_id(headers))
                    .flatten();
                let account = if advance_schedule {
                    self.select_group_account(
                        &group,
                        &available,
                        consumer_type,
                        consumer_id,
                        session_id.as_deref(),
                        now_ms,
                    )
                    .await
                    .ok_or_else(route_unavailable)?
                } else {
                    available[0].clone()
                };
                Ok(Some(ResolvedAccountRoute {
                    account,
                    group: Some(group),
                }))
            }
        }
    }

    pub async fn has_available_account(
        &self,
        store: &dyn StateStore,
        now_ms: i64,
    ) -> AppResult<bool> {
        let routing = RoutingRepository::new(store).read().await?;
        let limited = self.quota_limited_ids().await;
        for account in routing.accounts.iter().filter(|entry| entry.enabled) {
            if limited.contains(&account.id) {
                continue;
            }
            if OAuthRepository::new(store, &account.id)
                .require_valid(now_ms)
                .await
                .is_ok()
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub async fn restore_quota_snapshots(&self, store: &dyn StateStore) -> AppResult<()> {
        let routing = RoutingRepository::new(store).read().await?;
        for account in routing.accounts {
            let repository = CodexUsageStateRepository::new(store, &account.id);
            if let Some(snapshot) = repository.read().await? {
                self.observe_cached_quota(&account.id, &snapshot).await;
            }
        }
        Ok(())
    }

    pub async fn observe_upstream_status(&self, account_id: &str, status: u16) {
        if status == 429 {
            self.mark_quota_limited(account_id).await;
        }
    }

    pub async fn observe_quota(&self, account_id: &str, subscription: &CodexSubscriptionInfo) {
        let state = account_quota_state_from_live(subscription);
        self.update_quota_state(account_id, state).await;
    }

    async fn observe_cached_quota(&self, account_id: &str, snapshot: &CodexUsageMonitorState) {
        let state = account_quota_state_from_cached(snapshot);
        self.update_quota_state(account_id, state).await;
    }

    async fn update_quota_state(&self, account_id: &str, state: AccountQuotaState) {
        let mut runtime = self.runtime.lock().await;
        runtime.quota.insert(account_id.to_owned(), state);
        match state.health {
            QuotaHealth::Exhausted => {
                if quarantine_account(&mut runtime, account_id) {
                    tracing::warn!(
                        event = "account_quota_routing",
                        action = "quarantined",
                        account_id
                    );
                }
            }
            QuotaHealth::Available => {
                if runtime.quota_limited.remove(account_id) {
                    tracing::info!(
                        event = "account_quota_routing",
                        action = "restored",
                        account_id
                    );
                }
            }
            QuotaHealth::Unknown => {}
        }
    }

    async fn mark_quota_limited(&self, account_id: &str) {
        let mut runtime = self.runtime.lock().await;
        if quarantine_account(&mut runtime, account_id) {
            tracing::warn!(
                event = "account_quota_routing",
                action = "quarantined_after_429",
                account_id
            );
        }
    }

    async fn is_quota_limited(&self, account_id: &str) -> bool {
        self.runtime.lock().await.quota_limited.contains(account_id)
    }

    async fn quota_limited_ids(&self) -> HashSet<String> {
        self.runtime.lock().await.quota_limited.clone()
    }

    async fn select_group_account(
        &self,
        group: &AccountGroup,
        available: &[CodexAccount],
        consumer_type: RouteConsumerKind,
        consumer_id: &str,
        session_id: Option<&str>,
        now_ms: i64,
    ) -> Option<CodexAccount> {
        let ttl = session_affinity_ttl(&group.session_affinity_ttl);
        let mut runtime = self.runtime.lock().await;
        if group.strategy == FALLBACK_STRATEGY {
            return Some(select_fallback(
                &mut runtime,
                group,
                consumer_type,
                consumer_id,
                available,
            ));
        }
        if let Some(session_id) = session_id {
            let cache_key = format!("{}::{session_id}", group.id);
            let cached = runtime.affinity.get(&cache_key).and_then(|binding| {
                binding_is_current(binding, ttl, now_ms).then(|| binding.account_id.clone())
            });
            if let Some(account_id) = cached
                && let Some(account) = available.iter().find(|entry| entry.id == account_id)
            {
                if let Some(binding) = runtime.affinity.get_mut(&cache_key) {
                    binding.touched_at_ms = now_ms;
                }
                return Some(account.clone());
            }
            runtime.affinity.remove(&cache_key);
            let account = select_unbound(&mut runtime, group, available)?;
            make_affinity_room(&mut runtime.affinity);
            runtime.affinity.insert(
                cache_key,
                AffinityBinding {
                    account_id: account.id.clone(),
                    touched_at_ms: now_ms,
                },
            );
            return Some(account);
        }
        select_unbound(&mut runtime, group, available)
    }
}

fn quarantine_account(runtime: &mut RouterRuntime, account_id: &str) -> bool {
    let inserted = runtime.quota_limited.insert(account_id.to_owned());
    runtime
        .affinity
        .retain(|_, binding| binding.account_id != account_id);
    runtime
        .fallback
        .retain(|_, bound_account_id| bound_account_id != account_id);
    inserted
}

fn select_unbound(
    runtime: &mut RouterRuntime,
    group: &AccountGroup,
    available: &[CodexAccount],
) -> Option<CodexAccount> {
    match group.strategy.as_str() {
        WEIGHTED_ROUND_ROBIN_STRATEGY => {
            let weights = quota_weights(&runtime.quota, available);
            select_smooth_weighted(&mut runtime.weighted, &group.id, available, &weights)
        }
        _ => Some(select_round_robin(
            &mut runtime.last_picked,
            &group.id,
            available,
        )),
    }
}

fn select_fallback(
    runtime: &mut RouterRuntime,
    group: &AccountGroup,
    consumer_type: RouteConsumerKind,
    consumer_id: &str,
    available: &[CodexAccount],
) -> CodexAccount {
    let binding_key = format!("{}::{}::{consumer_id}", group.id, consumer_type.as_str());
    if let Some(account_id) = runtime.fallback.get(&binding_key)
        && let Some(account) = available.iter().find(|account| account.id == *account_id)
    {
        return account.clone();
    }
    runtime.fallback.remove(&binding_key);
    let cursor_key = format!("fallback::{}", group.id);
    let selected = select_round_robin(&mut runtime.last_picked, &cursor_key, available);
    runtime.fallback.insert(binding_key, selected.id.clone());
    selected
}

fn quota_weights(
    quota: &HashMap<String, AccountQuotaState>,
    available: &[CodexAccount],
) -> Vec<f64> {
    let known = available
        .iter()
        .filter_map(|account| quota.get(&account.id))
        .filter(|state| state.health != QuotaHealth::Exhausted)
        .filter_map(|state| state.average_ratio)
        .filter(|ratio| ratio.is_finite() && *ratio > 0.0)
        .collect::<Vec<_>>();
    let neutral = if known.is_empty() {
        1.0
    } else {
        known.iter().sum::<f64>() / known.len() as f64
    };
    available
        .iter()
        .map(|account| match quota.get(&account.id) {
            Some(state) if state.health == QuotaHealth::Exhausted => 0.0,
            Some(state) => state.average_ratio.unwrap_or(neutral),
            None => neutral,
        })
        .collect()
}

fn select_smooth_weighted(
    states: &mut HashMap<String, SmoothWeightedState>,
    group_id: &str,
    available: &[CodexAccount],
    weights: &[f64],
) -> Option<CodexAccount> {
    let state = states.entry(group_id.to_owned()).or_default();
    state.current.retain(|account_id, _| {
        available
            .iter()
            .any(|account| account.id == account_id.as_str())
    });
    let mut picked: Option<&CodexAccount> = None;
    let mut picked_current = f64::NEG_INFINITY;
    let total_weight = weights.iter().sum::<f64>();
    if total_weight <= 0.0 {
        return None;
    }
    for current in state.current.values_mut() {
        *current = current.clamp(-total_weight, total_weight);
    }
    for (account, weight) in available.iter().zip(weights) {
        if *weight <= 0.0 {
            continue;
        }
        let current = state.current.entry(account.id.clone()).or_default();
        *current += *weight;
        if picked.is_none() || *current > picked_current {
            picked = Some(account);
            picked_current = *current;
        }
    }
    let picked = picked?;
    if let Some(current) = state.current.get_mut(&picked.id) {
        *current -= total_weight;
    }
    Some(picked.clone())
}

#[derive(Clone, Copy)]
struct QuotaWindowSample {
    category: CodexQuotaCategory,
    remaining_percent: Option<f64>,
    reset_at: Option<f64>,
    allowed: Option<bool>,
    limit_reached: bool,
}

fn account_quota_state_from_live(subscription: &CodexSubscriptionInfo) -> AccountQuotaState {
    account_quota_state(
        subscription.windows.iter().map(|window| QuotaWindowSample {
            category: window.category,
            remaining_percent: window.remaining_percent,
            reset_at: window.reset_at,
            allowed: window.allowed,
            limit_reached: window.limit_reached,
        }),
        subscription.fetched_at,
    )
}

fn account_quota_state_from_cached(snapshot: &CodexUsageMonitorState) -> AccountQuotaState {
    account_quota_state(
        snapshot.windows.iter().map(|window| QuotaWindowSample {
            category: window.category,
            remaining_percent: window.remaining_percent,
            reset_at: window.reset_at.map(|value| value as f64),
            allowed: window.allowed,
            limit_reached: window.limit_reached,
        }),
        snapshot.sampled_at as f64,
    )
}

fn account_quota_state(
    samples: impl Iterator<Item = QuotaWindowSample>,
    sampled_at: f64,
) -> AccountQuotaState {
    let mut saw_codex_window = false;
    let mut saw_available_signal = false;
    let mut exhausted = false;
    let mut ratios = Vec::new();
    for sample in samples.filter(|sample| sample.category == CodexQuotaCategory::Codex) {
        saw_codex_window = true;
        let remaining = sample
            .remaining_percent
            .filter(|value| value.is_finite())
            .map(|value| value.clamp(0.0, 100.0));
        if sample.limit_reached
            || sample.allowed == Some(false)
            || remaining.is_some_and(|value| value <= f64::EPSILON)
        {
            exhausted = true;
        }
        if sample.allowed == Some(true) || remaining.is_some_and(|value| value > 0.0) {
            saw_available_signal = true;
        }
        if let (Some(remaining), Some(reset_at)) = (remaining, sample.reset_at)
            && reset_at.is_finite()
            && reset_at > sampled_at
            && remaining > 0.0
        {
            let remaining_minutes = ((reset_at - sampled_at) / 60_000.0).max(1.0);
            ratios.push(remaining / remaining_minutes);
        }
    }
    let health = if exhausted {
        QuotaHealth::Exhausted
    } else if saw_codex_window && saw_available_signal {
        QuotaHealth::Available
    } else {
        QuotaHealth::Unknown
    };
    let average_ratio = if ratios.is_empty() {
        None
    } else {
        Some(ratios.iter().sum::<f64>() / ratios.len() as f64)
    };
    AccountQuotaState {
        health,
        average_ratio,
    }
}

fn make_affinity_room(affinity: &mut HashMap<String, AffinityBinding>) {
    if affinity.len() < MAX_AFFINITY_BINDINGS {
        return;
    }
    let oldest = affinity
        .iter()
        .min_by_key(|(_, binding)| binding.touched_at_ms)
        .map(|(key, _)| key.clone());
    if let Some(oldest) = oldest {
        affinity.remove(&oldest);
    }
}

fn select_round_robin(
    last_picked: &mut HashMap<String, String>,
    group_id: &str,
    available: &[CodexAccount],
) -> CodexAccount {
    let previous = last_picked.get(group_id).map(String::as_str).unwrap_or("");
    let index = if previous.is_empty() {
        0
    } else {
        let next = available.partition_point(|entry| entry.id.as_str() <= previous);
        if next >= available.len() { 0 } else { next }
    };
    let selected = available[index].clone();
    last_picked.insert(group_id.into(), selected.id.clone());
    selected
}

fn binding_is_current(binding: &AffinityBinding, ttl: Option<Duration>, now_ms: i64) -> bool {
    let Some(ttl) = ttl else {
        return true;
    };
    let ttl_ms = i64::try_from(ttl.as_millis()).unwrap_or(i64::MAX);
    now_ms.saturating_sub(binding.touched_at_ms) <= ttl_ms
}

fn session_affinity_id(headers: &HeaderMap) -> Option<String> {
    for (name, prefix) in [
        ("x-claude-code-session-id", "claude"),
        ("session-id", "codex"),
        ("session_id", "codex"),
        ("x-session-id", "header"),
        ("x-session-affinity", "affinity"),
        ("x-client-request-id", "clientreq"),
    ] {
        for value in headers.get_all(name) {
            let Ok(value) = value.to_str() else {
                continue;
            };
            if let Some(value) = normalized_session_id(value) {
                return Some(format!("{prefix}:{value}"));
            }
        }
    }
    None
}

fn normalized_session_id(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 512
        && !value
            .chars()
            .any(|character| character.is_control() || character == '\u{7f}'))
    .then_some(value)
}

fn route_unavailable() -> ApiError {
    ApiError::new(404, "No enabled Codex account is available for this route.")
        .with_kind("authentication_error")
        .with_code("account_route_unavailable")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str) -> CodexAccount {
        CodexAccount {
            id: id.into(),
            name: id.into(),
            enabled: true,
        }
    }

    #[test]
    fn round_robin_resumes_after_the_previous_id_when_candidates_change() {
        let first = account("00000000-0000-4000-8000-000000000001");
        let second = account("00000000-0000-4000-8000-000000000002");
        let third = account("00000000-0000-4000-8000-000000000003");
        let mut state = HashMap::new();
        assert_eq!(
            select_round_robin(
                &mut state,
                "group",
                &[first.clone(), second.clone(), third.clone()]
            ),
            first
        );
        assert_eq!(
            select_round_robin(&mut state, "group", &[second.clone(), third.clone()]),
            second
        );
        assert_eq!(
            select_round_robin(&mut state, "group", &[first.clone(), third.clone()]),
            third
        );
        assert_eq!(
            select_round_robin(&mut state, "group", &[first.clone(), third]),
            first
        );
    }

    #[test]
    fn extracts_explicit_session_headers_in_reference_priority_order() {
        let mut headers = HeaderMap::new();
        headers.insert("x-session-affinity", "open-code".parse().unwrap());
        headers.insert("session-id", "codex-session".parse().unwrap());
        assert_eq!(
            session_affinity_id(&headers).as_deref(),
            Some("codex:codex-session")
        );
    }

    #[test]
    fn smooth_weighted_round_robin_matches_the_weight_ratio() {
        let first = account("00000000-0000-4000-8000-000000000001");
        let second = account("00000000-0000-4000-8000-000000000002");
        let available = vec![first.clone(), second.clone()];
        let weights = vec![3.0, 1.0];
        let mut states = HashMap::new();
        let mut counts = HashMap::new();
        for _ in 0..40 {
            let selected =
                select_smooth_weighted(&mut states, "group", &available, &weights).unwrap();
            *counts.entry(selected.id).or_insert(0) += 1;
        }
        assert_eq!(counts.get(&first.id), Some(&30));
        assert_eq!(counts.get(&second.id), Some(&10));
    }

    #[test]
    fn quota_weight_averages_remaining_percent_over_remaining_minutes() {
        let sampled_at = 1_000_000.0;
        let first = account_quota_state(
            [QuotaWindowSample {
                category: CodexQuotaCategory::Codex,
                remaining_percent: Some(10.0),
                reset_at: Some(sampled_at + 2.0 * 60.0 * 60_000.0),
                allowed: Some(true),
                limit_reached: false,
            }]
            .into_iter(),
            sampled_at,
        );
        let second = account_quota_state(
            [QuotaWindowSample {
                category: CodexQuotaCategory::Codex,
                remaining_percent: Some(5.0),
                reset_at: Some(sampled_at + 3.0 * 60.0 * 60_000.0),
                allowed: Some(true),
                limit_reached: false,
            }]
            .into_iter(),
            sampled_at,
        );
        assert!((first.average_ratio.unwrap() - 10.0 / 120.0).abs() < 0.000_001);
        assert!((second.average_ratio.unwrap() - 5.0 / 180.0).abs() < 0.000_001);
    }

    #[test]
    fn fallback_sticks_per_consumer_and_only_moves_when_bound_account_is_unavailable() {
        let first = account("00000000-0000-4000-8000-000000000001");
        let second = account("00000000-0000-4000-8000-000000000002");
        let group = AccountGroup {
            id: "00000000-0000-4000-8000-000000000003".into(),
            name: "fallback".into(),
            account_ids: vec![first.id.clone(), second.id.clone()],
            strategy: FALLBACK_STRATEGY.into(),
            session_affinity: false,
            session_affinity_ttl: "1h".into(),
        };
        let mut runtime = RouterRuntime::default();
        let available = vec![first.clone(), second.clone()];
        assert_eq!(
            select_fallback(
                &mut runtime,
                &group,
                RouteConsumerKind::ApiKey,
                "consumer-a",
                &available,
            ),
            first
        );
        assert_eq!(
            select_fallback(
                &mut runtime,
                &group,
                RouteConsumerKind::ApiKey,
                "consumer-a",
                &available,
            ),
            first
        );
        assert_eq!(
            select_fallback(
                &mut runtime,
                &group,
                RouteConsumerKind::ApiKey,
                "consumer-b",
                &available,
            ),
            second
        );
        assert_eq!(
            select_fallback(
                &mut runtime,
                &group,
                RouteConsumerKind::ApiKey,
                "consumer-a",
                std::slice::from_ref(&second),
            ),
            second
        );
        assert_eq!(
            select_fallback(
                &mut runtime,
                &group,
                RouteConsumerKind::ApiKey,
                "consumer-a",
                &available,
            ),
            second
        );
    }

    #[tokio::test]
    async fn a_429_quarantines_until_a_patrol_observes_available_quota() {
        let router = AccountRouter::default();
        router.observe_upstream_status("account-a", 429).await;
        assert!(router.is_quota_limited("account-a").await);

        router
            .update_quota_state(
                "account-a",
                AccountQuotaState {
                    health: QuotaHealth::Exhausted,
                    average_ratio: None,
                },
            )
            .await;
        assert!(router.is_quota_limited("account-a").await);
        router
            .update_quota_state(
                "account-a",
                AccountQuotaState {
                    health: QuotaHealth::Available,
                    average_ratio: Some(1.0),
                },
            )
            .await;
        assert!(!router.is_quota_limited("account-a").await);
    }
}

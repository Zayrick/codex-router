use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use axum::http::HeaderMap;
use tokio::sync::Mutex;

use crate::{
    application::CodexUsageMonitorState,
    auth::{
        AccountGroup, CodexAccount, FALLBACK_STRATEGY, OAuthRepository, RouteTargetKind,
        RoutingRepository, StateStore, WEIGHTED_ROUND_ROBIN_STRATEGY, session_affinity_ttl,
    },
    core::{ApiError, AppResult},
    upstream::codex::{CodexQuotaCategory, CodexSubscriptionInfo},
};

use super::{
    account_health::{AccountHealth, CapacitySignal, HealthView, Outcome},
    usage::OutcomeSink,
    usage_store::CodexUsageStateRepository,
};

const MAX_AFFINITY_BINDINGS: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAccountRoute {
    pub account: CodexAccount,
    pub group: Option<AccountGroup>,
    /// Session key (`group::session`) when the client named its session; used
    /// for affinity (if the group pins sessions) and overload avoidance.
    pub session_key: Option<String>,
    /// Probe ticket when this request tests a recovering account.
    pub probe: Option<u64>,
}

/// What the caller knows about the request when picking an account.
#[derive(Debug, Clone, Copy, Default)]
pub struct RouteHints<'a> {
    /// Model of the request, when the body has been read.
    pub model: Option<&'a str>,
    /// Whether the request generates a response, so it can probe a recovering account.
    pub generative: bool,
}

#[derive(Clone, Default)]
pub struct AccountRouter {
    runtime: Arc<Mutex<RouterRuntime>>,
    health: AccountHealth,
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
    /// Remaining percent per remaining minute, averaged over Codex windows.
    average_ratio: Option<f64>,
    /// Relative absolute capacity of the account's plan (Plus = 1).
    plan_multiplier: f64,
}

impl AccountRouter {
    pub async fn resolve_with(
        &self,
        store: &dyn StateStore,
        user_id: &str,
        headers: &HeaderMap,
        hints: RouteHints<'_>,
        now_ms: i64,
    ) -> AppResult<Option<ResolvedAccountRoute>> {
        self.resolve_internal(store, user_id, headers, hints, &[], now_ms)
            .await
    }

    /// Picks another group member for a request that `failed` could not serve.
    /// Returns `None` when the user is pinned to one account or no other member is
    /// usable; the new account becomes the session's affinity binding.
    pub async fn resolve_failover(
        &self,
        store: &dyn StateStore,
        user_id: &str,
        headers: &HeaderMap,
        model: Option<&str>,
        failed: &[String],
        now_ms: i64,
    ) -> AppResult<Option<ResolvedAccountRoute>> {
        let hints = RouteHints {
            model,
            generative: false,
        };
        match self
            .resolve_internal(store, user_id, headers, hints, failed, now_ms)
            .await
        {
            Ok(Some(route)) if route.group.is_some() && !failed.contains(&route.account.id) => {
                Ok(Some(route))
            }
            Ok(_) => Ok(None),
            Err(error) if error.status == 404 => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn resolve_internal(
        &self,
        store: &dyn StateStore,
        user_id: &str,
        headers: &HeaderMap,
        hints: RouteHints<'_>,
        exclude: &[String],
        now_ms: i64,
    ) -> AppResult<Option<ResolvedAccountRoute>> {
        let routing = RoutingRepository::new(store).read().await?;
        let Some(assignment) = routing.routes.iter().find(|route| route.user_id == user_id) else {
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
                    session_key: None,
                    probe: None,
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
                available.retain(|account| {
                    !limited.contains(&account.id) && !exclude.contains(&account.id)
                });
                if available.is_empty() {
                    return Err(route_unavailable());
                }
                let session_key = session_affinity_id(headers)
                    .map(|session_id| format!("{}::{session_id}", group.id));
                let (account, probe) = self
                    .select_group_account(
                        &group,
                        &available,
                        user_id,
                        session_key.as_deref(),
                        hints,
                        now_ms,
                    )
                    .await
                    .ok_or_else(route_unavailable)?;
                Ok(Some(ResolvedAccountRoute {
                    account,
                    group: Some(group),
                    session_key,
                    probe,
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

    /// Feeds one finished upstream attempt into the account's capacity health.
    pub fn observe_outcome(&self, outcome: &Outcome<'_>) {
        if !outcome.account_id.is_empty() {
            self.health.observe(outcome);
        }
    }

    /// A usage-tracker sink that feeds this router's health tracking. The probe
    /// ticket, if any, travels with the first finished attempt only.
    pub fn outcome_sink(&self, route: &ResolvedAccountRoute) -> OutcomeSink {
        let router = self.clone();
        let session_key = route.session_key.clone();
        let probe = ProbeTicket {
            health: self.health.clone(),
            account_id: route.account.id.clone(),
            ticket: AtomicU64::new(route.probe.unwrap_or(0)),
        };
        Arc::new(move |attempt| {
            // Warm-ups (`generate: false`) say nothing about generation capacity.
            if attempt.warmup {
                return;
            }
            let ticket = probe.ticket.swap(0, Ordering::Relaxed);
            router.observe_outcome(&Outcome {
                account_id: attempt.codex_account_id,
                model: attempt.model,
                session_key: session_key.as_deref(),
                signal: CapacitySignal::classify(attempt.status, attempt.error_code),
                probe: (ticket != 0).then_some(ticket),
                at_ms: attempt.recorded_at_ms,
            });
        })
    }

    pub fn health(&self, account_id: &str, now_ms: i64) -> HealthView {
        self.health.view(account_id, now_ms)
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

    /// Share of new (unbound) requests each enabled member would receive now,
    /// following the group's strategy; quarantined members get 0.
    pub async fn member_shares(
        &self,
        group: &AccountGroup,
        members: &[CodexAccount],
        now_ms: i64,
    ) -> Vec<f64> {
        let runtime = self.runtime.lock().await;
        let available = members
            .iter()
            .filter(|account| account.enabled && !runtime.quota_limited.contains(&account.id))
            .cloned()
            .collect::<Vec<_>>();
        let (usable, health) = usable_candidates(&self.health, &available, &[], None, now_ms);
        let available = usable;
        let weights = match group.strategy.as_str() {
            WEIGHTED_ROUND_ROBIN_STRATEGY => quota_weights(&runtime.quota, &available)
                .into_iter()
                .zip(&health)
                .map(|(quota, health)| quota * health)
                .collect(),
            FALLBACK_STRATEGY => {
                let first = health.iter().position(|factor| *factor > 0.0);
                (0..available.len())
                    .map(|index| if Some(index) == first { 1.0 } else { 0.0 })
                    .collect()
            }
            _ => health,
        };
        let total = weights.iter().sum::<f64>();
        members
            .iter()
            .map(|account| {
                available
                    .iter()
                    .position(|entry| entry.id == account.id)
                    .filter(|_| total > 0.0)
                    .map_or(0.0, |index| weights[index] / total)
            })
            .collect()
    }

    pub async fn is_quarantined(&self, account_id: &str) -> bool {
        self.is_quota_limited(account_id).await
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
        user_id: &str,
        session_key: Option<&str>,
        hints: RouteHints<'_>,
        now_ms: i64,
    ) -> Option<(CodexAccount, Option<u64>)> {
        let ttl = session_affinity_ttl(&group.session_affinity_ttl);
        let avoided = session_key
            .map(|key| self.health.avoided_accounts(key, now_ms))
            .unwrap_or_default();
        let (candidates, weights) =
            usable_candidates(&self.health, available, &avoided, hints.model, now_ms);
        let mut runtime = self.runtime.lock().await;
        let affinity_key = session_key.filter(|_| group.session_affinity);
        if group.strategy != FALLBACK_STRATEGY
            && let Some(cache_key) = affinity_key
        {
            let cached = runtime.affinity.get(cache_key).and_then(|binding| {
                binding_is_current(binding, ttl, now_ms).then(|| binding.account_id.clone())
            });
            // Degraded accounts keep their sessions (and prompt caches); open
            // or avoided ones do not.
            if let Some(account_id) = cached
                && let Some(account) = candidates.iter().find(|entry| entry.id == account_id)
            {
                if let Some(binding) = runtime.affinity.get_mut(cache_key) {
                    binding.touched_at_ms = now_ms;
                }
                return Some((account.clone(), None));
            }
            runtime.affinity.remove(cache_key);
        }
        // A fallback user keeps a usable bound account; probes take new users.
        if group.strategy == FALLBACK_STRATEGY
            && let Some(bound) = runtime.fallback.get(&format!("{}::{user_id}", group.id))
            && let Some(account) = candidates.iter().find(|account| &account.id == bound)
        {
            return Some((account.clone(), None));
        }
        let (account, probe) = match self.claim_probe(available, &avoided, hints, now_ms) {
            Some((account, ticket)) => (account, Some(ticket)),
            None if group.strategy == FALLBACK_STRATEGY => (
                select_fallback(&mut runtime, group, user_id, &candidates),
                None,
            ),
            None => (
                select_unbound(&mut runtime, group, &candidates, &weights)?,
                None,
            ),
        };
        if group.strategy != FALLBACK_STRATEGY
            && let Some(cache_key) = affinity_key
        {
            make_affinity_room(&mut runtime.affinity);
            runtime.affinity.insert(
                cache_key.to_owned(),
                AffinityBinding {
                    account_id: account.id.clone(),
                    touched_at_ms: now_ms,
                },
            );
        }
        Some((account, probe))
    }

    /// Hands one new generative request to a recovering (half-open) account.
    fn claim_probe(
        &self,
        available: &[CodexAccount],
        avoided: &[String],
        hints: RouteHints<'_>,
        now_ms: i64,
    ) -> Option<(CodexAccount, u64)> {
        if !hints.generative {
            return None;
        }
        let (account_id, ticket) = self.health.claim_probe(
            available
                .iter()
                .map(|account| account.id.as_str())
                .filter(|id| !avoided.iter().any(|avoided| avoided == id)),
            now_ms,
        )?;
        tracing::info!(event = "account_health", action = "probe", account_id);
        let account = available.iter().find(|account| account.id == account_id)?;
        Some((account.clone(), ticket))
    }
}

/// A probe ticket held by a tracker; returned if the tracker ends unused
/// (e.g. a WebSocket that closes before sending any request).
struct ProbeTicket {
    health: AccountHealth,
    account_id: String,
    ticket: AtomicU64,
}

impl Drop for ProbeTicket {
    fn drop(&mut self) {
        let ticket = *self.ticket.get_mut();
        if ticket != 0 {
            self.health.release_probe(&self.account_id, ticket);
        }
    }
}

/// Accounts eligible for new traffic and their health weights.
///
/// Usable (non-open) accounts the session has not just failed on come first;
/// then any usable account. When nothing is usable, every account stays
/// eligible, ranked by overload rate: a bad account beats no answer.
fn usable_candidates(
    health: &AccountHealth,
    available: &[CodexAccount],
    avoided: &[String],
    model: Option<&str>,
    now_ms: i64,
) -> (Vec<CodexAccount>, Vec<f64>) {
    let scored = available
        .iter()
        .map(|account| (account, health.factor(&account.id, model, now_ms)))
        .collect::<Vec<_>>();
    let pick = |keep: &dyn Fn(&CodexAccount, f64) -> bool| {
        scored
            .iter()
            .filter(|(account, factor)| keep(account, *factor))
            .map(|(account, factor)| ((*account).clone(), *factor))
            .unzip::<_, _, Vec<_>, Vec<_>>()
    };
    let preferred = pick(&|account, factor| factor > 0.0 && !avoided.contains(&account.id));
    if !preferred.0.is_empty() {
        return preferred;
    }
    let usable = pick(&|_, factor| factor > 0.0);
    if !usable.0.is_empty() {
        return usable;
    }
    // Keep a half-open account's single probe the only request it carries,
    // unless nothing else is left at all.
    let idle = available
        .iter()
        .filter(|account| !health.probing(&account.id, now_ms))
        .collect::<Vec<_>>();
    let pool = if idle.is_empty() {
        available.iter().collect()
    } else {
        idle
    };
    pool.into_iter()
        .map(|account| {
            let rate = health.overload_rate(&account.id, now_ms);
            (account.clone(), (-8.0 * rate).exp().max(f64::MIN_POSITIVE))
        })
        .unzip()
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
    health: &[f64],
) -> Option<CodexAccount> {
    match group.strategy.as_str() {
        WEIGHTED_ROUND_ROBIN_STRATEGY => {
            let weights = quota_weights(&runtime.quota, available)
                .into_iter()
                .zip(health)
                .map(|(quota, health)| quota * health)
                .collect::<Vec<_>>();
            select_smooth_weighted(&mut runtime.weighted, &group.id, available, &weights)
        }
        // Equal base weights: smooth weighted round-robin is plain round-robin
        // until health tilts it.
        _ => select_smooth_weighted(&mut runtime.weighted, &group.id, available, health),
    }
}

fn select_fallback(
    runtime: &mut RouterRuntime,
    group: &AccountGroup,
    user_id: &str,
    available: &[CodexAccount],
) -> CodexAccount {
    let binding_key = format!("{}::{user_id}", group.id);
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
            Some(state) => state.average_ratio.unwrap_or(neutral) * state.plan_multiplier,
            None => neutral,
        })
        .collect()
}

/// Percent-based quota windows hide absolute capacity, so scale by plan size:
/// Pro ($200) is 20x Plus, Pro Lite ($100) is 5x Plus.
fn plan_multiplier(plan_type: Option<&str>) -> f64 {
    match plan_type.map(str::trim) {
        Some("pro") => 20.0,
        Some(plan) if plan == "prolite" || plan.ends_with("_prolite") => 5.0,
        _ => 1.0,
    }
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
        subscription.plan_type.as_deref(),
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
        snapshot.plan_type.as_deref(),
    )
}

fn account_quota_state(
    samples: impl Iterator<Item = QuotaWindowSample>,
    sampled_at: f64,
    plan_type: Option<&str>,
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
        plan_multiplier: plan_multiplier(plan_type),
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
    use crate::server::{account_health::HealthState, usage::FinishedAttempt};

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
            None,
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
            None,
        );
        assert!((first.average_ratio.unwrap() - 10.0 / 120.0).abs() < 0.000_001);
        assert!((second.average_ratio.unwrap() - 5.0 / 180.0).abs() < 0.000_001);
    }

    #[test]
    fn quota_weight_scales_by_plan_capacity() {
        let pro = account("00000000-0000-4000-8000-000000000001");
        let pro_lite = account("00000000-0000-4000-8000-000000000002");
        let plus = account("00000000-0000-4000-8000-000000000003");
        let unknown = account("00000000-0000-4000-8000-000000000004");
        let sampled_at = 1_000_000.0;
        let state = |plan: &str| {
            account_quota_state(
                [QuotaWindowSample {
                    category: CodexQuotaCategory::Codex,
                    remaining_percent: Some(50.0),
                    reset_at: Some(sampled_at + 60.0 * 60_000.0),
                    allowed: Some(true),
                    limit_reached: false,
                }]
                .into_iter(),
                sampled_at,
                Some(plan),
            )
        };
        let quota = HashMap::from([
            (pro.id.clone(), state("pro")),
            (pro_lite.id.clone(), state("prolite")),
            (plus.id.clone(), state("plus")),
        ]);
        let weights = quota_weights(&quota, &[pro, pro_lite, plus, unknown]);
        let unit = 50.0 / 60.0;
        assert!((weights[0] - 20.0 * unit).abs() < 0.000_001);
        assert!((weights[1] - 5.0 * unit).abs() < 0.000_001);
        assert!((weights[2] - unit).abs() < 0.000_001);
        assert!((weights[3] - unit).abs() < 0.000_001);
        assert_eq!(plan_multiplier(Some("self_serve_business_prolite")), 5.0);
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
            select_fallback(&mut runtime, &group, "consumer-a", &available,),
            first
        );
        assert_eq!(
            select_fallback(&mut runtime, &group, "consumer-a", &available,),
            first
        );
        assert_eq!(
            select_fallback(&mut runtime, &group, "consumer-b", &available,),
            second
        );
        assert_eq!(
            select_fallback(
                &mut runtime,
                &group,
                "consumer-a",
                std::slice::from_ref(&second),
            ),
            second
        );
        assert_eq!(
            select_fallback(&mut runtime, &group, "consumer-a", &available,),
            second
        );
    }

    fn round_robin_group(accounts: &[&CodexAccount]) -> AccountGroup {
        AccountGroup {
            id: "group".into(),
            name: "pool".into(),
            account_ids: accounts.iter().map(|account| account.id.clone()).collect(),
            strategy: "round-robin".into(),
            session_affinity: true,
            session_affinity_ttl: "1h".into(),
        }
    }

    const GENERATIVE: RouteHints<'static> = RouteHints {
        model: None,
        generative: true,
    };

    fn overload(router: &AccountRouter, account_id: &str, session: &str, now_ms: i64) {
        router.observe_outcome(&Outcome {
            account_id,
            model: "gpt-test",
            session_key: Some(session),
            signal: CapacitySignal::Overload,
            probe: None,
            at_ms: now_ms,
        });
    }

    /// Trips `account_id` while the rest of the pool serves normally (an
    /// overload everywhere at once is global and only degrades accounts).
    fn open(router: &AccountRouter, account_id: &str) {
        for i in 0..50 {
            router.observe_outcome(&Outcome {
                account_id: "elsewhere",
                model: "gpt-test",
                session_key: None,
                signal: CapacitySignal::Success,
                probe: None,
                at_ms: i,
            });
        }
        for (i, session) in ["x", "y", "z"].into_iter().enumerate() {
            overload(router, account_id, session, i as i64);
        }
        assert_eq!(router.health(account_id, 10).state, HealthState::Open);
    }

    async fn pick(
        router: &AccountRouter,
        group: &AccountGroup,
        available: &[CodexAccount],
        session: Option<&str>,
        now_ms: i64,
    ) -> (CodexAccount, Option<u64>) {
        router
            .select_group_account(group, available, "u", session, GENERATIVE, now_ms)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn an_overloaded_session_moves_to_another_account_and_stays_there() {
        let (a, b, c) = (account("a"), account("b"), account("c"));
        let group = round_robin_group(&[&a, &b, &c]);
        let available = vec![a.clone(), b.clone(), c.clone()];
        let router = AccountRouter::default();
        let session = Some("group::s1");
        assert_eq!(pick(&router, &group, &available, session, 0).await.0, a);
        assert_eq!(pick(&router, &group, &available, session, 1).await.0, a);
        overload(&router, "a", "group::s1", 2);
        let moved = pick(&router, &group, &available, session, 3).await.0;
        assert_ne!(moved, a);
        assert_eq!(pick(&router, &group, &available, session, 4).await.0, moved);
        // A second overload elsewhere must not send the session back to `a`.
        overload(&router, &moved.id, "group::s1", 5);
        let remaining = if moved == b { c } else { b };
        assert_eq!(
            pick(&router, &group, &available, session, 6).await.0,
            remaining
        );
    }

    #[tokio::test]
    async fn avoidance_works_without_session_affinity() {
        let (a, b) = (account("a"), account("b"));
        let mut group = round_robin_group(&[&a, &b]);
        group.session_affinity = false;
        let available = vec![a.clone(), b.clone()];
        let router = AccountRouter::default();
        overload(&router, "a", "group::s1", 0);
        for now in 1..5 {
            assert_eq!(
                pick(&router, &group, &available, Some("group::s1"), now)
                    .await
                    .0,
                b
            );
        }
    }

    #[tokio::test]
    async fn open_accounts_get_no_new_sessions_until_their_probe() {
        let (a, b) = (account("a"), account("b"));
        let group = round_robin_group(&[&a, &b]);
        let available = vec![a.clone(), b.clone()];
        let router = AccountRouter::default();
        open(&router, "a");
        for i in 0..6 {
            let session = format!("group::new-{i}");
            let (picked, probe) = pick(&router, &group, &available, Some(&session), 10 + i).await;
            assert_eq!((picked, probe), (b.clone(), None));
        }
        let reopen = router.health("a", 10).retry_at.unwrap() + 1;
        let (probe_account, ticket) =
            pick(&router, &group, &available, Some("group::p"), reopen).await;
        assert_eq!(probe_account, a);
        assert!(ticket.is_some());
        // Only one probe at a time.
        let (next, ticket) = pick(&router, &group, &available, Some("group::q"), reopen).await;
        assert_eq!((next, ticket), (b, None));
    }

    #[tokio::test]
    async fn bound_sessions_are_not_diverted_to_probes() {
        let (a, b) = (account("a"), account("b"));
        let group = round_robin_group(&[&a, &b]);
        let available = vec![a.clone(), b.clone()];
        let router = AccountRouter::default();
        open(&router, "a");
        assert_eq!(
            pick(&router, &group, &available, Some("group::s"), 10)
                .await
                .0,
            b
        );
        let reopen = router.health("a", 10).retry_at.unwrap() + 1;
        assert_eq!(
            pick(&router, &group, &available, Some("group::s"), reopen).await,
            (b, None)
        );
    }

    #[tokio::test]
    async fn fallback_groups_probe_recovering_accounts_with_new_users() {
        let (a, b) = (account("a"), account("b"));
        let mut group = round_robin_group(&[&a, &b]);
        group.strategy = FALLBACK_STRATEGY.into();
        let available = vec![a.clone(), b.clone()];
        let router = AccountRouter::default();
        open(&router, "a");
        assert_eq!(pick(&router, &group, &available, None, 10).await.0, b);
        let reopen = router.health("a", 10).retry_at.unwrap() + 1;
        // The bound user keeps its healthy account.
        assert_eq!(
            pick(&router, &group, &available, None, reopen).await,
            (b.clone(), None)
        );
        // A new user carries the probe.
        let (probe_account, ticket) = router
            .select_group_account(&group, &available, "new-user", None, GENERATIVE, reopen)
            .await
            .unwrap();
        assert_eq!(probe_account, a);
        assert!(ticket.is_some());
    }

    #[tokio::test]
    async fn every_account_unhealthy_still_serves_from_the_least_bad() {
        let (a, b) = (account("a"), account("b"));
        let group = round_robin_group(&[&a, &b]);
        let available = vec![a.clone(), b.clone()];
        let router = AccountRouter::default();
        open(&router, "a");
        open(&router, "b");
        let picked = router
            .select_group_account(&group, &available, "u", None, RouteHints::default(), 10)
            .await;
        assert!(picked.is_some());
    }

    #[tokio::test]
    async fn avoiding_does_not_prefer_open_accounts_over_a_degraded_one() {
        let (a, b, c) = (account("a"), account("b"), account("c"));
        let group = round_robin_group(&[&a, &b, &c]);
        let available = vec![a.clone(), b.clone(), c.clone()];
        let router = AccountRouter::default();
        open(&router, "b");
        open(&router, "c");
        overload(&router, "a", "group::s", 20);
        assert_eq!(router.health("a", 21).state, HealthState::Healthy);
        // `a` is avoided by the session but the others are open: stay on `a`.
        assert_eq!(
            pick(&router, &group, &available, Some("group::s"), 21)
                .await
                .0,
            a
        );
    }

    fn route_for(account: &CodexAccount, probe: Option<u64>) -> ResolvedAccountRoute {
        ResolvedAccountRoute {
            account: account.clone(),
            group: None,
            session_key: None,
            probe,
        }
    }

    #[tokio::test]
    async fn an_unused_probe_ticket_is_returned_when_its_tracker_ends() {
        let a = account("a");
        let router = AccountRouter::default();
        open(&router, "a");
        let reopen = router.health("a", 10).retry_at.unwrap() + 1;
        let (_, ticket) = router.health.claim_probe(["a"], reopen).unwrap();
        assert!(router.health.probing("a", reopen));
        drop(router.outcome_sink(&route_for(&a, Some(ticket))));
        assert!(!router.health.probing("a", reopen));
    }

    #[tokio::test]
    async fn warmups_do_not_complete_a_probe() {
        let a = account("a");
        let router = AccountRouter::default();
        open(&router, "a");
        let reopen = router.health("a", 10).retry_at.unwrap() + 1;
        let (_, ticket) = router.health.claim_probe(["a"], reopen).unwrap();
        let sink = router.outcome_sink(&route_for(&a, Some(ticket)));
        let attempt = |warmup| FinishedAttempt {
            codex_account_id: "a",
            model: "gpt-test",
            status: "completed",
            error_code: "",
            recorded_at_ms: reopen + 1,
            warmup,
        };
        sink(&attempt(true));
        assert!(router.health.probing("a", reopen + 1));
        sink(&attempt(false));
        assert!(!router.health.probing("a", reopen + 1));
    }

    #[tokio::test]
    async fn when_everything_is_down_a_probing_account_is_left_to_its_probe() {
        let (a, b) = (account("a"), account("b"));
        let available = vec![a.clone(), b.clone()];
        let router = AccountRouter::default();
        open(&router, "a");
        open(&router, "b");
        let reopen = router.health("a", 10).retry_at.unwrap() + 1;
        assert!(router.health.claim_probe(["a"], reopen).is_some());
        let (candidates, _) = usable_candidates(&router.health, &available, &[], None, reopen);
        assert!(!candidates.is_empty());
        assert!(!candidates.contains(&a), "{candidates:?}");
        assert_eq!(candidates, [b]);
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
                    plan_multiplier: 1.0,
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
                    plan_multiplier: 1.0,
                },
            )
            .await;
        assert!(!router.is_quota_limited("account-a").await);
    }
}

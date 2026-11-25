//! Per-account capacity health, learned from upstream outcomes.
//!
//! The Codex backend sheds load per account: one account can answer
//! `server_is_overloaded` for hours while others in the same pool stay healthy.
//! Quota weights describe how much of an account's budget is left; this module
//! describes whether the account can serve right now. Routing multiplies both.
//!
//! Each account runs a small circuit breaker:
//!
//! ```text
//! Healthy ──2 overloads/10m──▶ Degraded ──3 overloads/15m, ≥2 sessions──▶ Open
//!    ▲                            │  ▲                                    │
//!    └── quiet + 5 successes ─────┘  └── 2 probe successes ── HalfOpen ◀──┘ cooldown
//!                                                                │
//!                                          overload ─▶ Open (longer cooldown)
//! ```
//!
//! Overload rates decay with a 30 minute half-life and are shrunk toward the
//! pool rate (accounts) and the account rate (account × model), so a handful of
//! samples cannot swing the weights.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use serde::Serialize;

const MINUTE_MS: i64 = 60_000;
/// Half-life of the smoothed success/overload counts.
const HALF_LIFE_MS: f64 = 30.0 * MINUTE_MS as f64;
/// Pseudo-samples that shrink a rate toward its parent rate.
const PRIOR_SAMPLES: f64 = 20.0;
/// Weight penalty steepness: `exp(-8 p)` halves the weight at ~9% overloads.
const OVERLOAD_PENALTY: f64 = 8.0;
/// Overload rate an account must exceed (and twice the pool rate) to count as bad.
const MIN_BAD_RATE: f64 = 0.06;
/// Smoothed samples needed before the rate alone can degrade an account.
const MIN_RATE_SAMPLES: f64 = 20.0;
/// Assumed pool overload rate before other accounts have reported anything.
const DEFAULT_POOL_RATE: f64 = 0.03;

const DEGRADE_WINDOW_MS: i64 = 10 * MINUTE_MS;
const DEGRADE_OVERLOADS: usize = 2;
const OPEN_WINDOW_MS: i64 = 15 * MINUTE_MS;
const OPEN_OVERLOADS: usize = 3;
const OPEN_SESSIONS: usize = 2;
/// Cooldowns for successive trips; the last one repeats.
const COOLDOWNS_MS: [i64; 4] = [
    5 * MINUTE_MS,
    10 * MINUTE_MS,
    20 * MINUTE_MS,
    30 * MINUTE_MS,
];
const PROBE_SUCCESSES: u32 = 2;
/// A probe that never reports back frees its slot after this long.
const PROBE_TIMEOUT_MS: i64 = 3 * MINUTE_MS;
const RECOVERY_SUCCESSES: u32 = 5;
const RECOVERY_QUIET_MS: i64 = 10 * MINUTE_MS;
/// A degraded account with no traffic recovers once its overloads are this old.
const IDLE_RECOVERY_MS: i64 = 30 * MINUTE_MS;
const DEGRADED_FACTOR: f64 = 0.25;
/// A session that hit an overload avoids that account for this long.
const SESSION_AVOID_MS: i64 = 10 * MINUTE_MS;
const MAX_SESSION_AVOIDANCES: usize = 10_000;

/// Upstream error codes that mean "this account has no capacity right now".
const OVERLOAD_CODES: [&str; 2] = ["server_is_overloaded", "slow_down"];

pub fn is_overload_code(code: &str) -> bool {
    OVERLOAD_CODES.contains(&code)
}

/// What one finished upstream attempt says about its account's capacity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacitySignal {
    Success,
    Overload,
    /// Cancellations, transport failures, client errors: not the account's fault.
    Neutral,
}

impl CapacitySignal {
    pub fn classify(status: &str, error_code: &str) -> Self {
        if is_overload_code(error_code) {
            Self::Overload
        } else if matches!(status, "completed" | "incomplete") {
            Self::Success
        } else {
            Self::Neutral
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Healthy,
    Degraded,
    Open,
    HalfOpen,
}

/// Health of one account as shown in the console.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthView {
    pub state: HealthState,
    /// Smoothed overload rate (0..1).
    pub overload_rate: f64,
    /// When an open breaker lets the next probe through.
    pub retry_at: Option<i64>,
}

/// One finished attempt, as reported by the usage tracker.
pub struct Outcome<'a> {
    pub account_id: &'a str,
    pub model: &'a str,
    /// Session of the request (`group::session`), when it had one.
    pub session_key: Option<&'a str>,
    pub signal: CapacitySignal,
    /// Probe ticket, when this attempt was a half-open account's probe.
    pub probe: Option<u64>,
    pub at_ms: i64,
}

#[derive(Clone, Default)]
pub struct AccountHealth {
    book: Arc<Mutex<HealthBook>>,
}

#[derive(Default)]
struct HealthBook {
    pool: Decayed,
    accounts: HashMap<String, AccountStats>,
    /// Session key → accounts the session should avoid, each until a deadline.
    avoid: HashMap<String, Vec<(String, i64)>>,
    /// Every avoidance in insertion (and therefore expiry) order, for eviction.
    avoid_order: VecDeque<(i64, String, String)>,
    /// Source of probe tickets.
    next_probe: u64,
}

struct AccountStats {
    state: HealthState,
    rate: Decayed,
    models: HashMap<String, Decayed>,
    /// Recent overloads with the session that saw them.
    overloads: VecDeque<(i64, Option<String>)>,
    /// Number of trips since the account was last fully healthy.
    trips: usize,
    open_until: i64,
    /// Outstanding probe ticket and when it was handed out.
    probe: Option<(u64, i64)>,
    successes: u32,
}

impl Default for AccountStats {
    fn default() -> Self {
        Self {
            state: HealthState::Healthy,
            rate: Decayed::default(),
            models: HashMap::new(),
            overloads: VecDeque::new(),
            trips: 0,
            open_until: 0,
            probe: None,
            successes: 0,
        }
    }
}

/// Exponentially decayed sample and overload counts.
#[derive(Debug, Clone, Copy, Default)]
struct Decayed {
    samples: f64,
    overloads: f64,
    at_ms: i64,
}

impl Decayed {
    fn decayed(&self, now_ms: i64) -> Self {
        let elapsed = (now_ms - self.at_ms).max(0) as f64;
        let keep = 0.5_f64.powf(elapsed / HALF_LIFE_MS);
        Self {
            samples: self.samples * keep,
            overloads: self.overloads * keep,
            at_ms: now_ms.max(self.at_ms),
        }
    }

    fn add(&mut self, overload: bool, now_ms: i64) {
        *self = self.decayed(now_ms);
        self.samples += 1.0;
        if overload {
            self.overloads += 1.0;
        }
    }

    /// Overload rate shrunk toward `prior`.
    fn rate(&self, prior: f64, now_ms: i64) -> f64 {
        let current = self.decayed(now_ms);
        (current.overloads + PRIOR_SAMPLES * prior) / (current.samples + PRIOR_SAMPLES)
    }
}

impl AccountHealth {
    fn book(&self) -> MutexGuard<'_, HealthBook> {
        self.book.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn observe(&self, outcome: &Outcome<'_>) {
        let mut book = self.book();
        if outcome.signal == CapacitySignal::Neutral {
            if let Some(stats) = book.accounts.get_mut(outcome.account_id) {
                stats.release_probe(outcome.probe);
            }
            return;
        }
        let overload = outcome.signal == CapacitySignal::Overload;
        let now = outcome.at_ms;
        book.pool.add(overload, now);
        if overload && let Some(session) = outcome.session_key {
            book.avoid_account(session, outcome.account_id, now);
        }
        let pool = book.pool;
        let stats = book
            .accounts
            .entry(outcome.account_id.to_owned())
            .or_default();
        stats.rate.add(overload, now);
        let pool_rate = others_rate(pool, stats.rate, now);
        if !outcome.model.is_empty() {
            stats
                .models
                .entry(outcome.model.to_owned())
                .or_default()
                .add(overload, now);
        }
        let before = stats.state;
        if overload {
            stats.on_overload(outcome, pool_rate, now);
        } else {
            stats.on_success(outcome.probe, pool_rate, now);
        }
        if stats.state != before {
            log_transition(outcome.account_id, before, stats, pool_rate, now);
        }
    }

    /// Routing weight multiplier for an account; 0 removes it from new traffic.
    /// `model` sharpens the overload penalty when the request's model is known.
    pub fn factor(&self, account_id: &str, model: Option<&str>, now_ms: i64) -> f64 {
        let mut book = self.book();
        let pool = book.pool;
        let Some(stats) = book.accounts.get_mut(account_id) else {
            return 1.0;
        };
        let pool_rate = others_rate(pool, stats.rate, now_ms);
        stats.refresh(now_ms);
        let state_factor = match stats.state {
            HealthState::Healthy => 1.0,
            HealthState::Degraded => DEGRADED_FACTOR,
            HealthState::Open | HealthState::HalfOpen => return 0.0,
        };
        let account_rate = stats.rate.rate(pool_rate, now_ms);
        let rate = model
            .and_then(|model| stats.models.get(model))
            .map_or(account_rate, |counts| counts.rate(account_rate, now_ms));
        state_factor * (-OVERLOAD_PENALTY * rate).exp()
    }

    /// Hands out the probe slot of a half-open account among `candidates`,
    /// returning the account and the ticket its outcome must carry.
    pub fn claim_probe<'a>(
        &self,
        candidates: impl IntoIterator<Item = &'a str>,
        now_ms: i64,
    ) -> Option<(&'a str, u64)> {
        let mut guard = self.book();
        let book = &mut *guard;
        for account_id in candidates {
            let Some(stats) = book.accounts.get_mut(account_id) else {
                continue;
            };
            stats.refresh(now_ms);
            if stats.state == HealthState::HalfOpen && !stats.probe_in_flight(now_ms) {
                book.next_probe += 1;
                stats.probe = Some((book.next_probe, now_ms));
                return Some((account_id, book.next_probe));
            }
        }
        None
    }

    /// Returns a probe ticket whose connection ended without any attempt.
    pub fn release_probe(&self, account_id: &str, ticket: u64) {
        if let Some(stats) = self.book().accounts.get_mut(account_id) {
            stats.release_probe(Some(ticket));
        }
    }

    /// Whether a half-open account is busy with its probe.
    pub fn probing(&self, account_id: &str, now_ms: i64) -> bool {
        let mut book = self.book();
        book.accounts.get_mut(account_id).is_some_and(|stats| {
            stats.refresh(now_ms);
            stats.state == HealthState::HalfOpen && stats.probe_in_flight(now_ms)
        })
    }

    /// Accounts a session should move away from after it saw overloads there.
    pub fn avoided_accounts(&self, session_key: &str, now_ms: i64) -> Vec<String> {
        let mut book = self.book();
        let Some(entries) = book.avoid.get_mut(session_key) else {
            return Vec::new();
        };
        entries.retain(|(_, until)| *until > now_ms);
        let accounts = entries
            .iter()
            .map(|(account_id, _)| account_id.clone())
            .collect::<Vec<_>>();
        if accounts.is_empty() {
            book.avoid.remove(session_key);
        }
        accounts
    }

    /// Smoothed overload rate ignoring breaker state; ranks accounts when all are unusable.
    pub fn overload_rate(&self, account_id: &str, now_ms: i64) -> f64 {
        let book = self.book();
        let pool = book.pool;
        book.accounts.get(account_id).map_or_else(
            || pool.rate(DEFAULT_POOL_RATE, now_ms),
            |stats| {
                stats
                    .rate
                    .rate(others_rate(pool, stats.rate, now_ms), now_ms)
            },
        )
    }

    pub fn view(&self, account_id: &str, now_ms: i64) -> HealthView {
        let mut book = self.book();
        let pool = book.pool;
        let Some(stats) = book.accounts.get_mut(account_id) else {
            return HealthView {
                state: HealthState::Healthy,
                overload_rate: pool.rate(DEFAULT_POOL_RATE, now_ms),
                retry_at: None,
            };
        };
        let pool_rate = others_rate(pool, stats.rate, now_ms);
        stats.refresh(now_ms);
        HealthView {
            state: stats.state,
            overload_rate: stats.rate.rate(pool_rate, now_ms),
            retry_at: (stats.state == HealthState::Open).then_some(stats.open_until),
        }
    }
}

impl HealthBook {
    fn avoid_account(&mut self, session: &str, account_id: &str, now_ms: i64) {
        // Expire from the front; deadlines are inserted in time order.
        while let Some((until, _, _)) = self.avoid_order.front()
            && (*until <= now_ms || self.avoid_order.len() >= MAX_SESSION_AVOIDANCES)
        {
            let (until, session, account) = self.avoid_order.pop_front().expect("front exists");
            if let Some(entries) = self.avoid.get_mut(&session) {
                entries.retain(|(id, deadline)| !(id == &account && *deadline == until));
                if entries.is_empty() {
                    self.avoid.remove(&session);
                }
            }
        }
        let until = now_ms + SESSION_AVOID_MS;
        let entries = self.avoid.entry(session.to_owned()).or_default();
        match entries.iter_mut().find(|(id, _)| id == account_id) {
            Some(entry) => entry.1 = until,
            None => entries.push((account_id.to_owned(), until)),
        }
        self.avoid_order
            .push_back((until, session.to_owned(), account_id.to_owned()));
    }
}

impl AccountStats {
    /// Applies time-driven transitions.
    fn refresh(&mut self, now_ms: i64) {
        prune(&mut self.overloads, now_ms - IDLE_RECOVERY_MS);
        match self.state {
            HealthState::Open if now_ms >= self.open_until => {
                self.state = HealthState::HalfOpen;
                self.successes = 0;
                self.probe = None;
            }
            // A slow probe keeps its ticket until a new probe replaces it, so a
            // long but successful generation still counts.
            HealthState::Degraded if self.overloads.is_empty() && self.trips == 0 => {
                self.state = HealthState::Healthy;
                self.trips = 0;
            }
            _ => {}
        }
    }

    fn bad_rate(&self, pool_rate: f64, now_ms: i64) -> bool {
        self.rate.rate(pool_rate, now_ms) > MIN_BAD_RATE.max(2.0 * pool_rate)
    }

    fn probe_in_flight(&self, now_ms: i64) -> bool {
        self.probe
            .is_some_and(|(_, started)| now_ms - started < PROBE_TIMEOUT_MS)
    }

    /// Whether `ticket` is the outstanding probe.
    fn is_probe(&self, ticket: Option<u64>) -> bool {
        ticket.is_some() && self.probe.map(|(current, _)| current) == ticket
    }

    fn release_probe(&mut self, ticket: Option<u64>) {
        if self.is_probe(ticket) {
            self.probe = None;
        }
    }

    fn on_overload(&mut self, outcome: &Outcome<'_>, pool_rate: f64, now_ms: i64) {
        self.refresh(now_ms);
        self.overloads
            .push_back((now_ms, outcome.session_key.map(str::to_owned)));
        match self.state {
            // Only the probe decides a half-open breaker; older requests finishing
            // late describe the account before its cooldown.
            HealthState::HalfOpen if self.is_probe(outcome.probe) => {
                self.trip(outcome.account_id, now_ms);
                return;
            }
            HealthState::HalfOpen | HealthState::Open => return,
            HealthState::Healthy | HealthState::Degraded => {}
        }
        self.successes = 0;
        let account_id = outcome.account_id;
        let recent = |window: i64| {
            self.overloads
                .iter()
                .filter(move |(at, _)| *at > now_ms - window)
        };
        let sessions = recent(OPEN_WINDOW_MS)
            .filter_map(|(_, session)| session.as_deref())
            .collect::<HashSet<_>>()
            .len();
        if recent(OPEN_WINDOW_MS).count() >= OPEN_OVERLOADS
            && sessions >= OPEN_SESSIONS
            && self.bad_rate(pool_rate, now_ms)
        {
            self.trip(account_id, now_ms);
        } else if recent(DEGRADE_WINDOW_MS).count() >= DEGRADE_OVERLOADS
            || (self.rate.decayed(now_ms).samples >= MIN_RATE_SAMPLES
                && self.bad_rate(pool_rate, now_ms))
        {
            self.state = HealthState::Degraded;
        }
    }

    fn on_success(&mut self, ticket: Option<u64>, pool_rate: f64, now_ms: i64) {
        self.refresh(now_ms);
        match self.state {
            HealthState::HalfOpen if self.is_probe(ticket) => {
                self.probe = None;
                self.successes += 1;
                if self.successes >= PROBE_SUCCESSES {
                    self.state = HealthState::Degraded;
                    self.successes = 0;
                }
            }
            HealthState::Degraded => {
                self.successes += 1;
                let quiet = self
                    .overloads
                    .back()
                    .is_none_or(|(at, _)| now_ms - at >= RECOVERY_QUIET_MS);
                if self.successes >= RECOVERY_SUCCESSES
                    && quiet
                    && !self.bad_rate(pool_rate, now_ms)
                {
                    self.state = HealthState::Healthy;
                    self.trips = 0;
                }
            }
            HealthState::Healthy | HealthState::HalfOpen | HealthState::Open => {}
        }
    }

    fn trip(&mut self, account_id: &str, now_ms: i64) {
        let base = COOLDOWNS_MS[self.trips.min(COOLDOWNS_MS.len() - 1)];
        self.trips += 1;
        self.state = HealthState::Open;
        self.open_until = now_ms + jittered(base, account_id, now_ms);
        self.probe = None;
        self.successes = 0;
    }
}

/// Overload rate of every other account, so a bad account cannot raise its own bar.
fn others_rate(pool: Decayed, account: Decayed, now_ms: i64) -> f64 {
    let pool = pool.decayed(now_ms);
    let account = account.decayed(now_ms);
    let others = Decayed {
        samples: (pool.samples - account.samples).max(0.0),
        overloads: (pool.overloads - account.overloads).max(0.0),
        at_ms: now_ms,
    };
    others.rate(DEFAULT_POOL_RATE, now_ms)
}

fn prune(overloads: &mut VecDeque<(i64, Option<String>)>, before_ms: i64) {
    while overloads.front().is_some_and(|(at, _)| *at <= before_ms) {
        overloads.pop_front();
    }
}

/// Spreads a cooldown by ±20% so tripped accounts do not all reopen together.
fn jittered(base_ms: i64, account_id: &str, now_ms: i64) -> i64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in account_id.bytes().chain(now_ms.to_le_bytes()) {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    let unit = (hash % 10_000) as f64 / 10_000.0;
    (base_ms as f64 * (0.8 + 0.4 * unit)) as i64
}

fn log_transition(
    account_id: &str,
    from: HealthState,
    stats: &AccountStats,
    pool_rate: f64,
    now_ms: i64,
) {
    let rate = stats.rate.rate(pool_rate, now_ms);
    let retry_in_s = (stats.state == HealthState::Open).then(|| (stats.open_until - now_ms) / 1000);
    if matches!(stats.state, HealthState::Open | HealthState::Degraded)
        && !matches!(from, HealthState::HalfOpen)
    {
        tracing::warn!(
            event = "account_health",
            account_id,
            from = ?from,
            to = ?stats.state,
            overload_rate = rate,
            pool_rate,
            retry_in_s
        );
    } else {
        tracing::info!(
            event = "account_health",
            account_id,
            from = ?from,
            to = ?stats.state,
            overload_rate = rate,
            pool_rate,
            retry_in_s
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome<'a>(
        account_id: &'a str,
        session: &'a str,
        signal: CapacitySignal,
        at_ms: i64,
    ) -> Outcome<'a> {
        Outcome {
            account_id,
            model: "gpt-test",
            session_key: Some(session),
            signal,
            probe: None,
            at_ms,
        }
    }

    fn probe_outcome(
        account_id: &str,
        ticket: u64,
        signal: CapacitySignal,
        at_ms: i64,
    ) -> Outcome<'_> {
        Outcome {
            probe: Some(ticket),
            ..outcome(account_id, "probe", signal, at_ms)
        }
    }

    fn state(health: &AccountHealth, account_id: &str, now: i64) -> HealthState {
        health.view(account_id, now).state
    }

    fn open(health: &AccountHealth, account_id: &str, t: i64) {
        for (i, session) in ["s1", "s2", "s3"].into_iter().enumerate() {
            health.observe(&outcome(
                account_id,
                session,
                CapacitySignal::Overload,
                t + i as i64,
            ));
        }
        assert_eq!(state(health, account_id, t + 3), HealthState::Open);
    }

    fn reopen_at(health: &AccountHealth, account_id: &str, now: i64) -> i64 {
        health.view(account_id, now).retry_at.unwrap() + 1
    }

    #[test]
    fn classifies_capacity_signals() {
        assert_eq!(
            CapacitySignal::classify("failed", "server_is_overloaded"),
            CapacitySignal::Overload
        );
        assert_eq!(
            CapacitySignal::classify("failed", "slow_down"),
            CapacitySignal::Overload
        );
        assert_eq!(
            CapacitySignal::classify("completed", ""),
            CapacitySignal::Success
        );
        for (status, code) in [
            ("failed", "codex_unavailable"),
            ("cancelled", "client_cancelled"),
            ("interrupted", "stream_truncated"),
            ("failed", "context_length_exceeded"),
        ] {
            assert_eq!(
                CapacitySignal::classify(status, code),
                CapacitySignal::Neutral
            );
        }
    }

    #[test]
    fn two_overloads_degrade_and_three_across_sessions_open() {
        let health = AccountHealth::default();
        let t = 1_000_000;
        for i in 0..20 {
            health.observe(&outcome("b", "s", CapacitySignal::Success, t + i));
        }
        health.observe(&outcome("a", "s1", CapacitySignal::Overload, t));
        assert_eq!(state(&health, "a", t), HealthState::Healthy);
        health.observe(&outcome("a", "s1", CapacitySignal::Overload, t + MINUTE_MS));
        assert_eq!(state(&health, "a", t + MINUTE_MS), HealthState::Degraded);
        let factor = health.factor("a", None, t + MINUTE_MS);
        assert!(factor > 0.0 && factor < DEGRADED_FACTOR, "{factor}");
        // A third overload from the same session alone does not trip the breaker.
        health.observe(&outcome(
            "a",
            "s1",
            CapacitySignal::Overload,
            t + 2 * MINUTE_MS,
        ));
        assert_eq!(
            state(&health, "a", t + 2 * MINUTE_MS),
            HealthState::Degraded
        );
        health.observe(&outcome(
            "a",
            "s2",
            CapacitySignal::Overload,
            t + 3 * MINUTE_MS,
        ));
        assert_eq!(state(&health, "a", t + 3 * MINUTE_MS), HealthState::Open);
        assert_eq!(health.factor("a", None, t + 3 * MINUTE_MS), 0.0);
        assert_eq!(state(&health, "b", t + 3 * MINUTE_MS), HealthState::Healthy);
    }

    #[test]
    fn open_breaker_probes_one_request_then_recovers_gradually() {
        let health = AccountHealth::default();
        let t = 1_000_000;
        open(&health, "a", t);
        let retry_at = health.view("a", t + 3).retry_at.unwrap();
        assert!(retry_at >= t + 4 * MINUTE_MS && retry_at <= t + 6 * MINUTE_MS + 3);
        assert_eq!(health.claim_probe(["a"], retry_at - 1), None);

        let after = retry_at + 1;
        assert_eq!(state(&health, "a", after), HealthState::HalfOpen);
        assert_eq!(health.factor("a", None, after), 0.0);
        let (_, first) = health.claim_probe(["a"], after).unwrap();
        // One probe at a time.
        assert_eq!(health.claim_probe(["a"], after), None);
        health.observe(&probe_outcome(
            "a",
            first,
            CapacitySignal::Success,
            after + 1,
        ));
        let (_, second) = health.claim_probe(["a"], after + 2).unwrap();
        assert_ne!(first, second);
        health.observe(&probe_outcome(
            "a",
            second,
            CapacitySignal::Success,
            after + 3,
        ));
        assert_eq!(state(&health, "a", after + 3), HealthState::Degraded);

        // A few successes do not outweigh a recent trip while the rate is still high.
        let later = after + RECOVERY_QUIET_MS;
        for i in 0..RECOVERY_SUCCESSES {
            health.observe(&outcome(
                "a",
                "x",
                CapacitySignal::Success,
                later + i64::from(i),
            ));
        }
        assert_eq!(state(&health, "a", later + 10), HealthState::Degraded);
        // Sustained successes bring the rate down and restore full health.
        for i in 0..30 {
            health.observe(&outcome(
                "a",
                "x",
                CapacitySignal::Success,
                later + MINUTE_MS * i,
            ));
        }
        assert_eq!(
            state(&health, "a", later + 30 * MINUTE_MS),
            HealthState::Healthy
        );
    }

    #[test]
    fn failed_probe_reopens_with_a_longer_cooldown() {
        let health = AccountHealth::default();
        let t = 1_000_000;
        open(&health, "a", t);
        let first = health.view("a", t + 3).retry_at.unwrap() - t;
        let probe_at = t + first + 1;
        let (_, ticket) = health.claim_probe(["a"], probe_at).unwrap();
        health.observe(&probe_outcome(
            "a",
            ticket,
            CapacitySignal::Overload,
            probe_at + 1,
        ));
        let view = health.view("a", probe_at + 1);
        assert_eq!(view.state, HealthState::Open);
        let second = view.retry_at.unwrap() - (probe_at + 1);
        assert!(second >= 8 * MINUTE_MS, "{second}");
    }

    #[test]
    fn only_the_probe_decides_a_half_open_breaker() {
        let health = AccountHealth::default();
        let t = 1_000_000;
        open(&health, "a", t);
        let after = reopen_at(&health, "a", t + 3);
        let (_, ticket) = health.claim_probe(["a"], after).unwrap();
        // Older requests finishing late neither free, advance nor trip the probe.
        health.observe(&outcome("a", "old", CapacitySignal::Neutral, after + 1));
        health.observe(&outcome("a", "old", CapacitySignal::Success, after + 2));
        health.observe(&outcome("a", "old", CapacitySignal::Success, after + 3));
        health.observe(&outcome("a", "old", CapacitySignal::Overload, after + 4));
        assert_eq!(state(&health, "a", after + 5), HealthState::HalfOpen);
        assert_eq!(health.claim_probe(["a"], after + 5), None);
        // A stale ticket from an earlier generation does nothing either.
        health.observe(&probe_outcome(
            "a",
            ticket + 100,
            CapacitySignal::Neutral,
            after + 6,
        ));
        assert_eq!(health.claim_probe(["a"], after + 7), None);
        // The probe's own cancellation frees the slot.
        health.observe(&probe_outcome(
            "a",
            ticket,
            CapacitySignal::Neutral,
            after + 8,
        ));
        assert!(health.claim_probe(["a"], after + 9).is_some());
    }

    #[test]
    fn stuck_probe_frees_its_slot() {
        let health = AccountHealth::default();
        let t = 1_000_000;
        open(&health, "a", t);
        let after = reopen_at(&health, "a", t + 3);
        assert!(health.claim_probe(["a"], after).is_some());
        assert!(
            health
                .claim_probe(["a"], after + PROBE_TIMEOUT_MS)
                .is_some()
        );
    }

    #[test]
    fn degraded_account_recovers_after_going_quiet() {
        let health = AccountHealth::default();
        let t = 1_000_000;
        health.observe(&outcome("a", "s", CapacitySignal::Overload, t));
        health.observe(&outcome("a", "s", CapacitySignal::Overload, t + 1));
        assert_eq!(state(&health, "a", t + 1), HealthState::Degraded);
        assert_eq!(
            state(&health, "a", t + IDLE_RECOVERY_MS + 2),
            HealthState::Healthy
        );
    }

    #[test]
    fn model_rate_sharpens_the_penalty() {
        let health = AccountHealth::default();
        let t = 1_000_000;
        for i in 0..40 {
            let model = if i % 4 == 0 { "heavy" } else { "light" };
            let signal = if model == "heavy" && i % 8 == 0 {
                CapacitySignal::Overload
            } else {
                CapacitySignal::Success
            };
            health.observe(&Outcome {
                account_id: "a",
                model,
                session_key: None,
                signal,
                probe: None,
                at_ms: t + i * 20 * MINUTE_MS,
            });
        }
        let now = t + 40 * 20 * MINUTE_MS;
        let heavy = health.factor("a", Some("heavy"), now);
        let light = health.factor("a", Some("light"), now);
        assert!(heavy < light, "heavy={heavy} light={light}");
    }

    #[test]
    fn a_session_avoids_every_account_it_recently_overloaded_on() {
        let health = AccountHealth::default();
        health.observe(&outcome("a", "g::s1", CapacitySignal::Overload, 0));
        health.observe(&outcome("b", "g::s1", CapacitySignal::Overload, 1));
        assert_eq!(health.avoided_accounts("g::s1", 2), ["a", "b"]);
        assert!(health.avoided_accounts("g::s2", 2).is_empty());
        assert_eq!(health.avoided_accounts("g::s1", SESSION_AVOID_MS), ["b"]);
        assert!(
            health
                .avoided_accounts("g::s1", SESSION_AVOID_MS + 1)
                .is_empty()
        );
    }

    #[test]
    fn session_avoidance_stays_bounded() {
        let health = AccountHealth::default();
        for i in 0..MAX_SESSION_AVOIDANCES + 50 {
            let session = format!("g::{i}");
            health.observe(&outcome("a", &session, CapacitySignal::Overload, i as i64));
        }
        let book = health.book();
        assert!(book.avoid_order.len() <= MAX_SESSION_AVOIDANCES);
        assert!(book.avoid.len() <= MAX_SESSION_AVOIDANCES);
        drop(book);
        // The newest session is still remembered.
        let newest = format!("g::{}", MAX_SESSION_AVOIDANCES + 49);
        assert_eq!(health.avoided_accounts(&newest, 0), ["a"]);
    }

    #[test]
    fn a_slow_probe_still_counts_until_another_replaces_it() {
        let health = AccountHealth::default();
        let t = 1_000_000;
        open(&health, "a", t);
        let after = reopen_at(&health, "a", t + 3);
        let (_, slow) = health.claim_probe(["a"], after).unwrap();
        let late = after + PROBE_TIMEOUT_MS + 1;
        health.observe(&probe_outcome("a", slow, CapacitySignal::Success, late));
        let (_, next) = health.claim_probe(["a"], late).unwrap();
        health.observe(&probe_outcome("a", next, CapacitySignal::Success, late + 1));
        assert_eq!(state(&health, "a", late + 1), HealthState::Degraded);
    }

    #[test]
    fn a_replaced_probe_no_longer_counts() {
        let health = AccountHealth::default();
        let t = 1_000_000;
        open(&health, "a", t);
        let after = reopen_at(&health, "a", t + 3);
        let (_, old) = health.claim_probe(["a"], after).unwrap();
        let (_, new) = health.claim_probe(["a"], after + PROBE_TIMEOUT_MS).unwrap();
        assert_ne!(old, new);
        health.observe(&probe_outcome(
            "a",
            old,
            CapacitySignal::Overload,
            after + PROBE_TIMEOUT_MS + 1,
        ));
        assert_eq!(
            state(&health, "a", after + PROBE_TIMEOUT_MS + 1),
            HealthState::HalfOpen
        );
        assert!(health.probing("a", after + PROBE_TIMEOUT_MS + 1));
    }
}

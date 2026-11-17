//! Analytics and request-log queries over `usage_events`.
//!
//! Every query loads the events of one time range (narrowed in SQL by time and
//! identity) and computes all filters and aggregates in a single pass, so each
//! view shares the same definitions of success, cost, percentiles and tiers.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::Result;
use rusqlite::{Connection, params};
use serde::Serialize;
use url::Url;

use super::{
    pricing::ModelPrice,
    usage::{
        CostIndex, EVENT_COLUMNS, PRIORITY_SERVICE_TIER, STATUS_CANCELLED, STATUS_COMPLETED,
        STATUS_FAILED, STATUS_INCOMPLETE, STATUS_INTERRUPTED, UsageEventRow, UsageFilters,
        UsageIdentityFilter, UsageStore, event_cost, event_from_row, filter_parameters,
        usage_filters_sql,
    },
};

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;
/// Longest custom range accepted, and the most buckets one series may hold.
const MAX_RANGE_MS: i64 = 400 * DAY;
const MAX_BUCKETS: i64 = 2_000;
/// Number of equal slices in per-row trend sparklines.
const TREND_SLICES: usize = 7;
const TOP_LIMIT: usize = 12;
const RECENT_FAILURE_LIMIT: usize = 20;
const DEFAULT_PAGE_LIMIT: usize = 50;
const MAX_PAGE_LIMIT: usize = 500;
const MAX_EXPORT_ROWS: usize = 50_000;
const SESSION_EVENT_LIMIT: usize = 20;
const HTTP_TOO_MANY_REQUESTS: u16 = 429;
/// Log-scale edges (ms) for the TTFT × total-latency density grid.
const TTFT_EDGES_MS: [i64; 15] = [
    0, 250, 500, 750, 1_000, 1_500, 2_000, 3_000, 5_000, 7_500, 10_000, 15_000, 20_000, 30_000,
    50_000,
];
const LATENCY_EDGES_MS: [i64; 15] = [
    0, 1_000, 2_000, 3_000, 5_000, 7_500, 10_000, 15_000, 20_000, 30_000, 45_000, 60_000, 90_000,
    120_000, 180_000,
];

// ---------------------------------------------------------------------------
// Query parameters
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalyticsRange {
    Day,
    Week,
    Month,
    All,
    Custom,
}

impl AnalyticsRange {
    fn parse(value: Option<&str>) -> Option<Self> {
        match value {
            Some("24h") => Some(Self::Day),
            None | Some("7d") => Some(Self::Week),
            Some("30d") => Some(Self::Month),
            Some("all") => Some(Self::All),
            Some("custom") => Some(Self::Custom),
            Some(_) => None,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Day => "24h",
            Self::Week => "7d",
            Self::Month => "30d",
            Self::All => "all",
            Self::Custom => "custom",
        }
    }

    const fn duration_ms(self) -> Option<i64> {
        match self {
            Self::Day => Some(DAY),
            Self::Week => Some(7 * DAY),
            Self::Month => Some(30 * DAY),
            Self::All | Self::Custom => None,
        }
    }
}

/// Event filters shared by analytics and the request log.
#[derive(Debug, Clone, Default)]
pub struct EventFilters {
    pub downstream: Option<UsageIdentityFilter>,
    pub upstream: Option<UsageIdentityFilter>,
    pub models: Vec<String>,
    pub statuses: Vec<String>,
    pub efforts: Vec<String>,
    /// Requested service tiers (`priority`, `default`, `auto`, `flex`).
    pub tiers: Vec<String>,
    pub transports: Vec<String>,
    pub session_kinds: Vec<String>,
    pub session_id: Option<String>,
    pub search: Option<String>,
}

impl EventFilters {
    fn identity_filters(&self) -> UsageFilters {
        UsageFilters::new(self.downstream.clone(), self.upstream.clone())
    }

    fn matches(&self, event: &UsageEventRow) -> bool {
        fn allowed(values: &[String], value: &str) -> bool {
            values.is_empty() || values.iter().any(|candidate| candidate == value)
        }
        allowed(&self.models, &event.model)
            && allowed(&self.statuses, &event.status)
            && allowed(&self.efforts, &event.reasoning_effort)
            && allowed(&self.tiers, &event.service_tier)
            && allowed(&self.transports, &event.transport)
            && allowed(&self.session_kinds, &event.session_kind)
            && self.session_id.as_deref().is_none_or(|session| {
                event.session_id == session || event.parent_session_id == session
            })
            && self.search.as_deref().is_none_or(|search| {
                let search = search.to_ascii_lowercase();
                [&event.request_id, &event.response_id, &event.session_id]
                    .iter()
                    .any(|value| value.to_ascii_lowercase().contains(&search))
            })
    }
}

/// Saved shortcuts of the request log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventView {
    All,
    Failed,
    Slow,
    Downgraded,
    Subagent,
    RateLimited,
}

impl EventView {
    fn parse(value: Option<&str>) -> Option<Self> {
        match value {
            None | Some("all") => Some(Self::All),
            Some("failed") => Some(Self::Failed),
            Some("slow") => Some(Self::Slow),
            Some("downgraded") => Some(Self::Downgraded),
            Some("subagent") => Some(Self::Subagent),
            Some("rate_limited") => Some(Self::RateLimited),
            Some(_) => None,
        }
    }

    fn matches(self, event: &UsageEventRow, slow_threshold_ms: Option<i64>) -> bool {
        match self {
            Self::All => true,
            Self::Failed => is_failure(event),
            Self::Slow => slow_threshold_ms.is_some_and(|threshold| event.latency_ms > threshold),
            Self::Downgraded => is_downgraded(event),
            Self::Subagent => event.session_kind == "subagent",
            Self::RateLimited => event.status_code == Some(HTTP_TOO_MANY_REQUESTS),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AnalyticsRequest {
    pub range: AnalyticsRange,
    pub start_at: i64,
    pub end_at: i64,
    pub bucket_ms: i64,
    pub filters: EventFilters,
}

#[derive(Debug, Clone)]
pub struct EventPageRequest {
    pub analytics: AnalyticsRequest,
    pub view: EventView,
    pub offset: usize,
    pub limit: usize,
    pub group_by_session: bool,
}

/// Who is asking: administrators may filter freely, users only see themselves.
pub enum AnalyticsScope {
    Admin,
    Identity(UsageIdentityFilter),
}

#[derive(Debug)]
pub struct AnalyticsQueryError {
    pub param: &'static str,
}

impl AnalyticsRequest {
    pub fn from_url(
        url: &Url,
        scope: &AnalyticsScope,
        earliest: Option<i64>,
        now: i64,
    ) -> Result<Self, AnalyticsQueryError> {
        let range = AnalyticsRange::parse(single(url, "range").as_deref())
            .ok_or(AnalyticsQueryError { param: "range" })?;
        let (start_at, end_at) = match range {
            AnalyticsRange::Custom => {
                let start_at =
                    integer(url, "startAt")?.ok_or(AnalyticsQueryError { param: "startAt" })?;
                let end_at =
                    integer(url, "endAt")?.ok_or(AnalyticsQueryError { param: "endAt" })?;
                if start_at < 0
                    || end_at <= start_at
                    || end_at - start_at > MAX_RANGE_MS
                    || end_at > now.saturating_add(2 * DAY)
                {
                    return Err(AnalyticsQueryError { param: "startAt" });
                }
                (start_at, end_at)
            }
            AnalyticsRange::All => (earliest.unwrap_or(now).min(now.saturating_sub(HOUR)), now),
            _ => (now - range.duration_ms().unwrap_or(DAY), now),
        };
        let bucket_ms = match integer(url, "bucket")? {
            Some(minutes) if minutes > 0 => minutes.saturating_mul(MINUTE),
            Some(_) => return Err(AnalyticsQueryError { param: "bucket" }),
            None => default_bucket(end_at - start_at),
        };
        if (end_at - start_at) / bucket_ms > MAX_BUCKETS {
            return Err(AnalyticsQueryError { param: "bucket" });
        }
        let downstream = identity_param(
            url,
            "downstreamType",
            "downstreamId",
            UsageIdentityFilter::parse_downstream,
        )?;
        let upstream = identity_param(
            url,
            "upstreamType",
            "upstreamId",
            UsageIdentityFilter::parse_upstream,
        )?;
        let downstream = match scope {
            AnalyticsScope::Admin => downstream,
            AnalyticsScope::Identity(identity) => {
                if downstream.is_some() {
                    return Err(AnalyticsQueryError {
                        param: "downstreamType",
                    });
                }
                Some(identity.clone())
            }
        };
        Ok(Self {
            range,
            start_at,
            end_at,
            bucket_ms,
            filters: EventFilters {
                downstream,
                upstream,
                models: multiple(url, "model"),
                statuses: multiple(url, "status"),
                efforts: multiple(url, "effort"),
                tiers: multiple(url, "tier"),
                transports: multiple(url, "transport"),
                session_kinds: multiple(url, "sessionKind"),
                session_id: single(url, "sessionId").filter(|value| !value.is_empty()),
                search: single(url, "q")
                    .map(|value| value.trim().to_owned())
                    .filter(|value| !value.is_empty()),
            },
        })
    }
}

impl EventPageRequest {
    pub fn from_url(
        url: &Url,
        scope: &AnalyticsScope,
        earliest: Option<i64>,
        now: i64,
    ) -> Result<Self, AnalyticsQueryError> {
        let analytics = AnalyticsRequest::from_url(url, scope, earliest, now)?;
        let view = EventView::parse(single(url, "view").as_deref())
            .ok_or(AnalyticsQueryError { param: "view" })?;
        let offset = integer(url, "offset")?.unwrap_or(0);
        let limit = integer(url, "limit")?.unwrap_or(DEFAULT_PAGE_LIMIT as i64);
        if offset < 0 || !(1..=MAX_PAGE_LIMIT as i64).contains(&limit) {
            return Err(AnalyticsQueryError { param: "limit" });
        }
        let group_by_session = match single(url, "group").as_deref() {
            None | Some("request") => false,
            Some("session") => true,
            Some(_) => return Err(AnalyticsQueryError { param: "group" }),
        };
        Ok(Self {
            analytics,
            view,
            offset: offset as usize,
            limit: limit as usize,
            group_by_session,
        })
    }
}

fn default_bucket(duration: i64) -> i64 {
    if duration <= 2 * DAY {
        HOUR
    } else if duration <= 8 * DAY {
        6 * HOUR
    } else {
        DAY
    }
}

fn single(url: &Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

fn multiple(url: &Url, name: &str) -> Vec<String> {
    url.query_pairs()
        .filter(|(key, _)| key == name)
        .flat_map(|(_, value)| {
            value
                .split(',')
                .map(|item| item.trim().to_owned())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn integer(url: &Url, name: &'static str) -> Result<Option<i64>, AnalyticsQueryError> {
    single(url, name)
        .map(|value| {
            value
                .parse::<i64>()
                .map_err(|_| AnalyticsQueryError { param: name })
        })
        .transpose()
}

fn identity_param(
    url: &Url,
    kind: &'static str,
    id: &'static str,
    parse: fn(&str, &str) -> Option<UsageIdentityFilter>,
) -> Result<Option<UsageIdentityFilter>, AnalyticsQueryError> {
    match (single(url, kind), single(url, id)) {
        (None, None) => Ok(None),
        (Some(kind_value), Some(id_value)) => parse(&kind_value, &id_value)
            .map(Some)
            .ok_or(AnalyticsQueryError { param: kind }),
        _ => Err(AnalyticsQueryError { param: kind }),
    }
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Percentiles {
    pub p50: Option<i64>,
    pub p95: Option<i64>,
    pub max: Option<i64>,
    pub samples: i64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OutcomeCounts {
    pub requests: i64,
    pub completed: i64,
    pub incomplete: i64,
    pub failed: i64,
    pub cancelled: i64,
    pub interrupted: i64,
    /// Requests the upstream answered with HTTP 429.
    pub rate_limited: i64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TokenCounts {
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_output_tokens: i64,
    pub total_tokens: i64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsTotals {
    #[serde(flatten)]
    pub outcomes: OutcomeCounts,
    #[serde(flatten)]
    pub tokens: TokenCounts,
    /// Tokens and requests of models without a price (not in `costUsd`).
    pub unpriced_tokens: i64,
    pub unpriced_requests: i64,
    /// Fast (priority tier) requests and their cost before the Fast multiplier.
    pub fast_requests: i64,
    pub fast_base_cost_usd: f64,
    /// Fast requests the upstream served as Fast / with another tier.
    pub fast_fulfilled: i64,
    pub fast_downgraded: i64,
    pub sessions: i64,
    pub ttft: Percentiles,
    pub latency: Percentiles,
    /// Median of `output_tokens × 1000 ÷ latency_ms` over completed requests.
    pub output_speed: Option<f64>,
    pub first_at: Option<i64>,
    pub last_at: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsBucket {
    pub start_at: i64,
    #[serde(flatten)]
    pub outcomes: OutcomeCounts,
    #[serde(flatten)]
    pub tokens: TokenCounts,
    pub ttft_p50: Option<i64>,
    pub ttft_p95: Option<i64>,
    pub latency_p50: Option<i64>,
    pub latency_p95: Option<i64>,
    pub fast_requests: i64,
    pub fast_downgraded: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NamedCount {
    pub name: String,
    pub requests: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelStats {
    pub model: String,
    pub priced: bool,
    #[serde(flatten)]
    pub outcomes: OutcomeCounts,
    #[serde(flatten)]
    pub tokens: TokenCounts,
    pub ttft: Percentiles,
    pub latency: Percentiles,
    pub output_speed: Option<f64>,
    /// Median output speed in the first and second half of the range.
    pub output_speed_first_half: Option<f64>,
    pub output_speed_second_half: Option<f64>,
    /// Requested models the upstream answered with this model instead.
    pub redirects: Vec<NamedCount>,
    pub first_at: i64,
    pub last_at: i64,
    pub trend: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IdentityStats {
    pub user_id: String,
    pub user_name: String,
    #[serde(flatten)]
    pub outcomes: OutcomeCounts,
    #[serde(flatten)]
    pub tokens: TokenCounts,
    pub sessions: i64,
    pub last_at: i64,
    pub last_user_agent: String,
    /// Total tokens per equal slice of the range.
    pub trend: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpstreamAccountStats {
    pub codex_account_id: String,
    pub codex_account_name: String,
    pub account_group_id: String,
    pub account_group_name: String,
    #[serde(flatten)]
    pub outcomes: OutcomeCounts,
    #[serde(flatten)]
    pub tokens: TokenCounts,
    pub ttft: Percentiles,
    pub latency: Percentiles,
    pub fast_requests: i64,
    pub fast_fulfilled: i64,
    pub fast_downgraded: i64,
    pub sessions: i64,
    pub last_at: i64,
    /// HTTP 429 responses per series bucket.
    pub rate_limit_series: Vec<i64>,
    pub trend: Vec<i64>,
    pub consumers: Vec<ConsumerShare>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpstreamGroupStats {
    /// Empty for requests routed directly to an account.
    pub account_group_id: String,
    pub account_group_name: String,
    #[serde(flatten)]
    pub outcomes: OutcomeCounts,
    #[serde(flatten)]
    pub tokens: TokenCounts,
    pub sessions: i64,
    /// Share of session requests served by the session's main account.
    pub session_affinity_rate: Option<f64>,
    pub trend: Vec<i64>,
    pub consumers: Vec<ConsumerShare>,
    /// Usage of each member account through this group.
    pub accounts: Vec<GroupAccountShare>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GroupAccountShare {
    pub codex_account_id: String,
    pub codex_account_name: String,
    pub requests: i64,
    pub total_tokens: i64,
    pub rate_limited: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConsumerShare {
    pub user_id: String,
    pub user_name: String,
    pub requests: i64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IdentityModelCell {
    pub user_id: String,
    pub user_name: String,
    pub model: String,
    pub requests: i64,
    pub total_tokens: i64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EffortTierCell {
    pub effort: String,
    /// Tier the upstream answered with; empty when it did not report one.
    pub tier: String,
    pub requests: i64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FastBreakdown {
    pub name: String,
    pub requested: i64,
    pub fulfilled: i64,
    pub downgraded: i64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FastStats {
    pub requested: i64,
    pub fulfilled: i64,
    pub downgraded: i64,
    pub accounts: Vec<FastBreakdown>,
    pub models: Vec<FastBreakdown>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ErrorStats {
    pub status: String,
    pub status_code: Option<u16>,
    pub error_code: String,
    pub requests: i64,
    pub last_at: i64,
    pub accounts: Vec<NamedCount>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionKindStats {
    pub kind: String,
    pub requests: i64,
    pub sessions: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub session_id: String,
    pub session_kind: String,
    pub parent_session_id: String,
    pub user_name: String,
    pub requests: i64,
    pub child_sessions: i64,
    pub total_tokens: i64,
    pub cost_usd: f64,
    pub first_at: i64,
    pub last_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionStats {
    pub sessions: i64,
    pub requests_with_session: i64,
    pub max_requests: i64,
    pub median_duration_ms: Option<i64>,
    pub kinds: Vec<SessionKindStats>,
    pub recent: Vec<SessionSummary>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClientStats {
    pub client: String,
    pub version: String,
    pub os: String,
    pub requests: i64,
    pub failed: i64,
    pub websocket_requests: i64,
    pub ttft: Percentiles,
    pub last_at: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IpStats {
    pub ip: String,
    pub forwarded_for: String,
    pub requests: i64,
    pub last_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClientOverview {
    pub clients: Vec<ClientStats>,
    pub distinct_clients: i64,
    pub ips: Vec<IpStats>,
    pub distinct_ips: i64,
    pub forwarded_requests: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TransportStats {
    pub name: String,
    #[serde(flatten)]
    pub outcomes: OutcomeCounts,
    pub ttft: Percentiles,
    pub latency: Percentiles,
    pub average_input_tokens: f64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TransportOverview {
    pub transports: Vec<TransportStats>,
    pub streaming: Vec<TransportStats>,
    pub endpoints: Vec<TransportStats>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LatencyDensity {
    pub ttft_edges_ms: Vec<i64>,
    pub latency_edges_ms: Vec<i64>,
    /// `cells[latency_bin][ttft_bin]` request counts.
    pub cells: Vec<Vec<i64>>,
    pub missing_ttft: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UnpricedModel {
    pub model: String,
    pub requests: i64,
    pub total_tokens: i64,
    pub first_at: i64,
    pub last_at: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageAnalytics {
    pub range: &'static str,
    pub start_at: i64,
    pub end_at: i64,
    pub bucket_ms: i64,
    pub totals: AnalyticsTotals,
    /// The equally long period right before this one, up to the same elapsed time.
    pub previous: Option<AnalyticsTotals>,
    pub series: Vec<AnalyticsBucket>,
    pub models: Vec<ModelStats>,
    pub identities: Vec<IdentityStats>,
    pub upstream_accounts: Vec<UpstreamAccountStats>,
    pub upstream_groups: Vec<UpstreamGroupStats>,
    pub identity_models: Vec<IdentityModelCell>,
    pub effort_tiers: Vec<EffortTierCell>,
    pub fast: FastStats,
    pub errors: Vec<ErrorStats>,
    pub sessions: SessionStats,
    pub clients: ClientOverview,
    pub transports: TransportOverview,
    pub latency_density: LatencyDensity,
    pub unpriced_models: Vec<UnpricedModel>,
    pub recent_failures: Vec<UsageEventRow>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EventViewCounts {
    pub all: i64,
    pub failed: i64,
    pub slow: i64,
    pub downgraded: i64,
    pub subagent: i64,
    pub rate_limited: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionChild {
    pub session_id: String,
    pub session_kind: String,
    #[serde(flatten)]
    pub outcomes: OutcomeCounts,
    pub total_tokens: i64,
    pub cost_usd: f64,
    pub first_at: i64,
    pub last_at: i64,
    pub accounts: Vec<String>,
    pub fast_downgraded: i64,
    /// `[recordedAt, latencyMs, status]` of each request, oldest first.
    pub timeline: Vec<(i64, i64, String)>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionGroup {
    /// Root session; empty for requests without a session (grouped by user).
    pub session_id: String,
    pub user_id: String,
    pub user_name: String,
    pub account_group_name: String,
    pub accounts: Vec<String>,
    pub user_agent: String,
    #[serde(flatten)]
    pub outcomes: OutcomeCounts,
    pub total_tokens: i64,
    pub cost_usd: f64,
    pub first_at: i64,
    pub last_at: i64,
    pub kinds: BTreeMap<String, i64>,
    pub timeline: Vec<(i64, i64, String)>,
    pub children: Vec<SessionChild>,
    /// Latest requests of the root session itself.
    pub events: Vec<UsageEventRow>,
    pub event_count: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageEventPage {
    pub start_at: i64,
    pub end_at: i64,
    pub bucket_ms: i64,
    /// Requests in the range before filters and views.
    pub range_total: i64,
    /// Requests matching the filters and view.
    pub total: i64,
    pub offset: usize,
    pub summary: AnalyticsTotals,
    pub series: Vec<AnalyticsBucket>,
    pub views: EventViewCounts,
    pub slow_threshold_ms: Option<i64>,
    pub events: Vec<UsageEventRow>,
    /// Present when grouped by session; `total` then counts groups.
    pub sessions: Option<Vec<SessionGroup>>,
    /// Models seen in the range for the identity filters (filter options).
    pub models: Vec<String>,
    /// Requests per session kind of each root session on this page, across the range.
    pub session_counts: BTreeMap<String, BTreeMap<String, i64>>,
}

// ---------------------------------------------------------------------------
// Store entry points
// ---------------------------------------------------------------------------

impl UsageStore {
    pub async fn earliest_event_at(&self, filters: UsageFilters) -> Result<Option<i64>> {
        self.read(move |connection| {
            let sql = format!(
                "SELECT MIN(recorded_at_ms) FROM usage_events WHERE {}",
                usage_filters_sql(1, 2, 3, 4)
            );
            let (a, b, c, d) = filter_parameters(&filters);
            Ok(connection.query_row(&sql, params![a, b, c, d], |row| row.get(0))?)
        })
        .await
    }

    pub async fn analytics(
        &self,
        request: AnalyticsRequest,
        prices: &[ModelPrice],
        fast_cost_multiplier: f64,
        now: i64,
    ) -> Result<UsageAnalytics> {
        let costs = CostIndex::new(prices, fast_cost_multiplier);
        self.read(move |connection| analytics(connection, &request, &costs, now))
            .await
    }

    pub async fn event_page(
        &self,
        request: EventPageRequest,
        prices: &[ModelPrice],
        fast_cost_multiplier: f64,
        now: i64,
    ) -> Result<UsageEventPage> {
        let costs = CostIndex::new(prices, fast_cost_multiplier);
        self.read(move |connection| event_page(connection, &request, &costs, now))
            .await
    }

    /// Events matching the filters, newest first, for CSV export.
    pub async fn export_events(
        &self,
        request: EventPageRequest,
        prices: &[ModelPrice],
        fast_cost_multiplier: f64,
    ) -> Result<Vec<UsageEventRow>> {
        let costs = CostIndex::new(prices, fast_cost_multiplier);
        self.read(move |connection| {
            let request_range = &request.analytics;
            let events = load_events(
                connection,
                request_range,
                request_range.start_at,
                request_range.end_at,
                &costs,
            )?;
            let slow = slow_threshold(&events);
            let mut events = events
                .into_iter()
                .filter(|event| request.view.matches(event, slow))
                .collect::<Vec<_>>();
            events.reverse();
            events.truncate(MAX_EXPORT_ROWS);
            Ok(events)
        })
        .await
    }
}

/// When each model and identity was last used, across all recorded events.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LastSeen {
    pub models: Vec<ModelLastSeen>,
    pub identities: Vec<IdentityLastSeen>,
    pub credentials: Vec<CredentialLastSeen>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelLastSeen {
    pub model: String,
    pub first_at: i64,
    pub last_at: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IdentityLastSeen {
    pub user_id: String,
    pub last_at: i64,
    pub user_agent: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CredentialLastSeen {
    pub credential_id: String,
    pub last_at: i64,
    pub user_agent: String,
}

impl UsageStore {
    pub async fn last_seen(&self) -> Result<LastSeen> {
        self.read(|connection| {
            let mut statement = connection.prepare(
                "SELECT model, MIN(recorded_at_ms), MAX(recorded_at_ms) FROM usage_events \
                 WHERE model <> '' GROUP BY model ORDER BY model",
            )?;
            let models = statement
                .query_map([], |row| {
                    Ok(ModelLastSeen {
                        model: row.get(0)?,
                        first_at: row.get(1)?,
                        last_at: row.get(2)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let latest = |column: &str| {
                format!(
                    "SELECT {column}, recorded_at_ms, user_agent FROM ( \
                         SELECT {column}, recorded_at_ms, user_agent, \
                                ROW_NUMBER() OVER ( \
                                    PARTITION BY {column} \
                                    ORDER BY recorded_at_ms DESC, id DESC) AS position \
                         FROM usage_events) \
                     WHERE position = 1"
                )
            };
            let identities = connection
                .prepare(&latest("user_id"))?
                .query_map([], |row| {
                    Ok(IdentityLastSeen {
                        user_id: row.get(0)?,
                        last_at: row.get(1)?,
                        user_agent: row.get(2)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let credentials = connection
                .prepare(&latest("credential_id"))?
                .query_map([], |row| {
                    Ok(CredentialLastSeen {
                        credential_id: row.get(0)?,
                        last_at: row.get(1)?,
                        user_agent: row.get(2)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(LastSeen {
                models,
                identities,
                credentials,
            })
        })
        .await
    }
}

/// Usage of one Codex account since a quota window started.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WindowUsage {
    pub requests: i64,
    pub total_tokens: i64,
    pub cost_usd: f64,
    /// Tokens recorded at or after `recent_since`.
    pub recent_tokens: i64,
}

impl UsageStore {
    pub async fn account_window_usage(
        &self,
        account_id: String,
        since: i64,
        recent_since: i64,
        prices: &[ModelPrice],
        fast_cost_multiplier: f64,
    ) -> Result<WindowUsage> {
        let costs = CostIndex::new(prices, fast_cost_multiplier);
        self.read(move |connection| {
            let mut statement = connection.prepare(
                "SELECT model, service_tier = 'priority', COUNT(*), \
                        COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0), \
                        COALESCE(SUM(cached_input_tokens), 0), COALESCE(SUM(cache_creation_input_tokens), 0), \
                        COALESCE(SUM(total_tokens), 0), \
                        COALESCE(SUM(CASE WHEN recorded_at_ms >= ?3 THEN total_tokens ELSE 0 END), 0) \
                 FROM usage_events WHERE codex_account_id = ?1 AND recorded_at_ms >= ?2 \
                 GROUP BY model, 2",
            )?;
            let rows = statement.query_map(params![account_id, since, recent_since], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, bool>(1)?,
                    [
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                    ],
                ))
            })?;
            let mut usage = WindowUsage::default();
            for row in rows {
                let (model, fast, [requests, input, output, cached, creation, total, recent]) = row?;
                usage.requests += requests;
                usage.total_tokens += total;
                usage.recent_tokens += recent;
                usage.cost_usd += costs.token_cost(&model, fast, input, output, cached, creation);
            }
            Ok(usage)
        })
        .await
    }
}

/// Events in `[start_at, end_at)` matching the filters, oldest first, with costs.
fn load_events(
    connection: &Connection,
    request: &AnalyticsRequest,
    start_at: i64,
    end_at: i64,
    costs: &CostIndex,
) -> Result<Vec<UsageEventRow>> {
    let sql = format!(
        "SELECT {EVENT_COLUMNS} FROM usage_events \
         WHERE recorded_at_ms >= ?1 AND recorded_at_ms < ?2 AND {} \
         ORDER BY recorded_at_ms, id",
        usage_filters_sql(3, 4, 5, 6)
    );
    let identity_filters = request.filters.identity_filters();
    let (a, b, c, d) = filter_parameters(&identity_filters);
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params![start_at, end_at, a, b, c, d], event_from_row)?;
    let mut events = Vec::new();
    for row in rows {
        let mut event = row?;
        if request.filters.matches(&event) {
            event.cost_usd = event_cost(&event, costs);
            events.push(event);
        }
    }
    Ok(events)
}

fn analytics(
    connection: &Connection,
    request: &AnalyticsRequest,
    costs: &CostIndex,
    now: i64,
) -> Result<UsageAnalytics> {
    let events = load_events(connection, request, request.start_at, request.end_at, costs)?;
    let previous = if request.range == AnalyticsRange::All {
        None
    } else {
        let length = request.end_at - request.start_at;
        let elapsed_end = request.end_at.min(now);
        let previous = load_events(
            connection,
            request,
            request.start_at - length,
            elapsed_end - length,
            costs,
        )?;
        Some(totals(&previous, costs))
    };
    let grid = Grid::new(request.start_at, request.end_at, request.bucket_ms);
    Ok(UsageAnalytics {
        range: request.range.label(),
        start_at: request.start_at,
        end_at: request.end_at,
        bucket_ms: request.bucket_ms,
        totals: totals(&events, costs),
        previous,
        series: series(&events, &grid),
        models: models(&events, &grid, costs),
        identities: identities(&events, &grid),
        upstream_accounts: upstream_accounts(&events, &grid),
        upstream_groups: upstream_groups(&events, &grid),
        identity_models: identity_models(&events),
        effort_tiers: effort_tiers(&events),
        fast: fast_stats(&events),
        errors: errors(&events),
        sessions: session_stats(&events),
        clients: client_overview(&events),
        transports: transport_overview(&events),
        latency_density: latency_density(&events),
        unpriced_models: unpriced_models(&events, costs),
        recent_failures: events
            .iter()
            .rev()
            .filter(|event| !is_success(event))
            .take(RECENT_FAILURE_LIMIT)
            .cloned()
            .collect(),
    })
}

fn event_page(
    connection: &Connection,
    request: &EventPageRequest,
    costs: &CostIndex,
    _now: i64,
) -> Result<UsageEventPage> {
    let range = &request.analytics;
    let identity_filters = range.filters.identity_filters();
    let (a, b, c, d) = filter_parameters(&identity_filters);
    let range_total: i64 = connection.query_row(
        &format!(
            "SELECT COUNT(*) FROM usage_events \
             WHERE recorded_at_ms >= ?1 AND recorded_at_ms < ?2 AND {}",
            usage_filters_sql(3, 4, 5, 6)
        ),
        params![range.start_at, range.end_at, a, b, c, d],
        |row| row.get(0),
    )?;
    let events = load_events(connection, range, range.start_at, range.end_at, costs)?;
    let slow = slow_threshold(&events);
    let mut views = EventViewCounts::default();
    for event in &events {
        views.all += 1;
        views.failed += i64::from(EventView::Failed.matches(event, slow));
        views.slow += i64::from(EventView::Slow.matches(event, slow));
        views.downgraded += i64::from(is_downgraded(event));
        views.subagent += i64::from(event.session_kind == "subagent");
        views.rate_limited += i64::from(event.status_code == Some(HTTP_TOO_MANY_REQUESTS));
    }
    let matching = events
        .into_iter()
        .filter(|event| request.view.matches(event, slow))
        .collect::<Vec<_>>();
    let grid = Grid::new(range.start_at, range.end_at, range.bucket_ms);
    let summary = totals(&matching, costs);
    let series = series(&matching, &grid);
    let (total, events, sessions) = if request.group_by_session {
        let groups = session_groups(&matching);
        let total = groups.len() as i64;
        let page = groups
            .into_iter()
            .skip(request.offset)
            .take(request.limit)
            .collect::<Vec<_>>();
        (total, Vec::new(), Some(page))
    } else {
        let total = matching.len() as i64;
        let page = matching
            .iter()
            .rev()
            .skip(request.offset)
            .take(request.limit)
            .cloned()
            .collect();
        (total, page, None)
    };
    let models = range_models(connection, range)?;
    let roots = events
        .iter()
        .map(root_session)
        .chain(
            sessions
                .iter()
                .flatten()
                .map(|group| group.session_id.as_str()),
        )
        .filter(|root| !root.is_empty())
        .map(str::to_owned)
        .collect::<HashSet<_>>();
    let session_counts = root_session_counts(connection, range, &roots)?;
    Ok(UsageEventPage {
        start_at: range.start_at,
        end_at: range.end_at,
        bucket_ms: range.bucket_ms,
        range_total,
        total,
        offset: request.offset,
        summary,
        series,
        views,
        slow_threshold_ms: slow,
        events,
        sessions,
        models,
        session_counts,
    })
}

/// Root of a request's session tree: its parent session, else its own session.
fn root_session(event: &UsageEventRow) -> &str {
    if event.parent_session_id.is_empty() {
        &event.session_id
    } else {
        &event.parent_session_id
    }
}

fn range_models(connection: &Connection, range: &AnalyticsRequest) -> Result<Vec<String>> {
    let identity_filters = range.filters.identity_filters();
    let (a, b, c, d) = filter_parameters(&identity_filters);
    let mut statement = connection.prepare(&format!(
        "SELECT DISTINCT model FROM usage_events \
         WHERE recorded_at_ms >= ?1 AND recorded_at_ms < ?2 AND model <> '' AND {} ORDER BY model",
        usage_filters_sql(3, 4, 5, 6)
    ))?;
    let rows = statement.query_map(params![range.start_at, range.end_at, a, b, c, d], |row| {
        row.get(0)
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
}

fn root_session_counts(
    connection: &Connection,
    range: &AnalyticsRequest,
    roots: &HashSet<String>,
) -> Result<BTreeMap<String, BTreeMap<String, i64>>> {
    let mut counts = BTreeMap::<String, BTreeMap<String, i64>>::new();
    if roots.is_empty() {
        return Ok(counts);
    }
    let identity_filters = range.filters.identity_filters();
    let (a, b, c, d) = filter_parameters(&identity_filters);
    let mut statement = connection.prepare(&format!(
        "SELECT CASE WHEN parent_session_id <> '' THEN parent_session_id ELSE session_id END, \
                session_kind, COUNT(*) FROM usage_events \
         WHERE recorded_at_ms >= ?1 AND recorded_at_ms < ?2 AND session_id <> '' AND {} \
         GROUP BY 1, 2",
        usage_filters_sql(3, 4, 5, 6)
    ))?;
    let rows = statement.query_map(params![range.start_at, range.end_at, a, b, c, d], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (root, kind, requests) = row?;
        if roots.contains(&root) {
            *counts.entry(root).or_default().entry(kind).or_default() += requests;
        }
    }
    Ok(counts)
}

// ---------------------------------------------------------------------------
// Classification helpers
// ---------------------------------------------------------------------------

fn is_success(event: &UsageEventRow) -> bool {
    event.status == STATUS_COMPLETED || event.status == STATUS_INCOMPLETE
}

fn is_failure(event: &UsageEventRow) -> bool {
    event.status == STATUS_FAILED || event.status == STATUS_INTERRUPTED
}

fn is_fast(event: &UsageEventRow) -> bool {
    event.service_tier == PRIORITY_SERVICE_TIER
}

fn is_fulfilled(event: &UsageEventRow) -> bool {
    is_fast(event) && event.response_service_tier == PRIORITY_SERVICE_TIER
}

fn is_downgraded(event: &UsageEventRow) -> bool {
    is_fast(event)
        && !event.response_service_tier.is_empty()
        && event.response_service_tier != PRIORITY_SERVICE_TIER
}

fn output_speed(event: &UsageEventRow) -> Option<f64> {
    (event.status == STATUS_COMPLETED && event.output_tokens > 0 && event.latency_ms > 0)
        .then(|| event.output_tokens as f64 * 1000.0 / event.latency_ms as f64)
}

fn slow_threshold(events: &[UsageEventRow]) -> Option<i64> {
    let mut latencies = events
        .iter()
        .filter(|event| is_success(event))
        .map(|event| event.latency_ms)
        .collect::<Vec<_>>();
    percentiles(&mut latencies).p95
}

fn add_outcome(target: &mut OutcomeCounts, event: &UsageEventRow) {
    target.requests += 1;
    match event.status.as_str() {
        STATUS_COMPLETED => target.completed += 1,
        STATUS_INCOMPLETE => target.incomplete += 1,
        STATUS_FAILED => target.failed += 1,
        STATUS_CANCELLED => target.cancelled += 1,
        STATUS_INTERRUPTED => target.interrupted += 1,
        _ => {}
    }
    if event.status_code == Some(HTTP_TOO_MANY_REQUESTS) {
        target.rate_limited += 1;
    }
}

fn add_tokens(target: &mut TokenCounts, event: &UsageEventRow) {
    target.input_tokens += event.input_tokens;
    target.cached_input_tokens += event.cached_input_tokens;
    target.cache_creation_input_tokens += event.cache_creation_input_tokens;
    target.output_tokens += event.output_tokens;
    target.reasoning_output_tokens += event.reasoning_output_tokens;
    target.total_tokens += event.total_tokens;
    target.cost_usd += event.cost_usd;
}

fn percentiles(values: &mut [i64]) -> Percentiles {
    if values.is_empty() {
        return Percentiles::default();
    }
    values.sort_unstable();
    Percentiles {
        p50: Some(nearest_rank(values, 0.5)),
        p95: Some(nearest_rank(values, 0.95)),
        max: values.last().copied(),
        samples: values.len() as i64,
    }
}

fn nearest_rank(sorted: &[i64], quantile: f64) -> i64 {
    let rank = (quantile * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable_by(f64::total_cmp);
    let middle = values.len() / 2;
    Some(if values.len().is_multiple_of(2) {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    })
}

/// Latency samples of one group: TTFT and total latency of successful requests.
#[derive(Default)]
struct LatencySamples {
    ttft: Vec<i64>,
    latency: Vec<i64>,
    speed: Vec<f64>,
}

impl LatencySamples {
    fn add(&mut self, event: &UsageEventRow) {
        if !is_success(event) {
            return;
        }
        if let Some(ttft) = event.ttft_ms {
            self.ttft.push(ttft);
        }
        self.latency.push(event.latency_ms);
        if let Some(speed) = output_speed(event) {
            self.speed.push(speed);
        }
    }

    fn finish(mut self) -> (Percentiles, Percentiles, Option<f64>) {
        (
            percentiles(&mut self.ttft),
            percentiles(&mut self.latency),
            median(&mut self.speed),
        )
    }
}

/// Fixed time buckets aligned to the range start.
struct Grid {
    start_at: i64,
    end_at: i64,
    bucket_ms: i64,
    buckets: usize,
}

impl Grid {
    fn new(start_at: i64, end_at: i64, bucket_ms: i64) -> Self {
        let buckets = ((end_at - start_at + bucket_ms - 1) / bucket_ms).max(1) as usize;
        Self {
            start_at,
            end_at,
            bucket_ms,
            buckets,
        }
    }

    fn bucket(&self, at: i64) -> usize {
        (((at - self.start_at) / self.bucket_ms).max(0) as usize).min(self.buckets - 1)
    }

    fn slice(&self, at: i64) -> usize {
        let length = (self.end_at - self.start_at).max(1);
        (((at - self.start_at).max(0) as i128 * TREND_SLICES as i128 / length as i128) as usize)
            .min(TREND_SLICES - 1)
    }

    fn midpoint(&self) -> i64 {
        self.start_at + (self.end_at - self.start_at) / 2
    }
}

// ---------------------------------------------------------------------------
// Aggregates
// ---------------------------------------------------------------------------

fn totals(events: &[UsageEventRow], costs: &CostIndex) -> AnalyticsTotals {
    let mut totals = AnalyticsTotals::default();
    let mut samples = LatencySamples::default();
    let mut sessions = HashSet::new();
    for event in events {
        add_outcome(&mut totals.outcomes, event);
        add_tokens(&mut totals.tokens, event);
        samples.add(event);
        if !event.model.is_empty() && !costs.is_priced(&event.model) {
            totals.unpriced_tokens += event.total_tokens;
            totals.unpriced_requests += 1;
        }
        totals.fast_fulfilled += i64::from(is_fulfilled(event));
        totals.fast_downgraded += i64::from(is_downgraded(event));
        if is_fast(event) {
            totals.fast_requests += 1;
            totals.fast_base_cost_usd += costs.token_cost(
                &event.model,
                false,
                event.input_tokens,
                event.output_tokens,
                event.cached_input_tokens,
                event.cache_creation_input_tokens,
            );
        }
        if !event.session_id.is_empty() {
            sessions.insert(event.session_id.as_str());
        }
    }
    totals.sessions = sessions.len() as i64;
    totals.first_at = events.first().map(|event| event.recorded_at);
    totals.last_at = events.last().map(|event| event.recorded_at);
    (totals.ttft, totals.latency, totals.output_speed) = samples.finish();
    totals
}

fn series(events: &[UsageEventRow], grid: &Grid) -> Vec<AnalyticsBucket> {
    let mut buckets = (0..grid.buckets)
        .map(|index| AnalyticsBucket {
            start_at: grid.start_at + index as i64 * grid.bucket_ms,
            ..AnalyticsBucket::default()
        })
        .collect::<Vec<_>>();
    let mut samples = (0..grid.buckets)
        .map(|_| LatencySamples::default())
        .collect::<Vec<_>>();
    for event in events {
        let index = grid.bucket(event.recorded_at);
        let bucket = &mut buckets[index];
        add_outcome(&mut bucket.outcomes, event);
        add_tokens(&mut bucket.tokens, event);
        bucket.fast_requests += i64::from(is_fast(event));
        bucket.fast_downgraded += i64::from(is_downgraded(event));
        samples[index].add(event);
    }
    for (bucket, samples) in buckets.iter_mut().zip(samples) {
        let (ttft, latency, _) = samples.finish();
        bucket.ttft_p50 = ttft.p50;
        bucket.ttft_p95 = ttft.p95;
        bucket.latency_p50 = latency.p50;
        bucket.latency_p95 = latency.p95;
    }
    buckets
}

fn models(events: &[UsageEventRow], grid: &Grid, costs: &CostIndex) -> Vec<ModelStats> {
    #[derive(Default)]
    struct Accumulator {
        outcomes: OutcomeCounts,
        tokens: TokenCounts,
        samples: LatencySamples,
        first_half: Vec<f64>,
        second_half: Vec<f64>,
        redirects: BTreeMap<String, i64>,
        first_at: i64,
        last_at: i64,
        trend: Vec<i64>,
    }
    let midpoint = grid.midpoint();
    let mut groups = BTreeMap::<&str, Accumulator>::new();
    for event in events.iter().filter(|event| !event.model.is_empty()) {
        let entry = groups.entry(&event.model).or_insert_with(|| Accumulator {
            first_at: event.recorded_at,
            trend: vec![0; TREND_SLICES],
            ..Accumulator::default()
        });
        add_outcome(&mut entry.outcomes, event);
        add_tokens(&mut entry.tokens, event);
        entry.samples.add(event);
        if let Some(speed) = output_speed(event) {
            if event.recorded_at < midpoint {
                entry.first_half.push(speed);
            } else {
                entry.second_half.push(speed);
            }
        }
        if !event.requested_model.is_empty() && event.requested_model != event.model {
            *entry
                .redirects
                .entry(event.requested_model.clone())
                .or_default() += 1;
        }
        entry.last_at = event.recorded_at;
        entry.trend[grid.slice(event.recorded_at)] += event.total_tokens;
    }
    let mut rows = groups
        .into_iter()
        .map(|(model, mut entry)| {
            let (ttft, latency, output_speed) = entry.samples.finish();
            ModelStats {
                model: model.to_owned(),
                priced: costs.is_priced(model),
                outcomes: entry.outcomes,
                tokens: entry.tokens,
                ttft,
                latency,
                output_speed,
                output_speed_first_half: median(&mut entry.first_half),
                output_speed_second_half: median(&mut entry.second_half),
                redirects: named_counts(entry.redirects),
                first_at: entry.first_at,
                last_at: entry.last_at,
                trend: entry.trend,
            }
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        right
            .tokens
            .total_tokens
            .cmp(&left.tokens.total_tokens)
            .then(right.outcomes.requests.cmp(&left.outcomes.requests))
    });
    rows
}

fn named_counts(values: BTreeMap<String, i64>) -> Vec<NamedCount> {
    let mut rows = values
        .into_iter()
        .map(|(name, requests)| NamedCount { name, requests })
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| std::cmp::Reverse(row.requests));
    rows
}

fn identities(events: &[UsageEventRow], grid: &Grid) -> Vec<IdentityStats> {
    let mut groups = BTreeMap::<&str, (IdentityStats, HashSet<&str>)>::new();
    for event in events {
        let (row, sessions) = groups.entry(&event.user_id).or_insert_with(|| {
            (
                IdentityStats {
                    user_id: event.user_id.clone(),
                    user_name: event.user_name.clone(),
                    outcomes: OutcomeCounts::default(),
                    tokens: TokenCounts::default(),
                    sessions: 0,
                    last_at: 0,
                    last_user_agent: String::new(),
                    trend: vec![0; TREND_SLICES],
                },
                HashSet::new(),
            )
        });
        add_outcome(&mut row.outcomes, event);
        add_tokens(&mut row.tokens, event);
        row.user_name.clone_from(&event.user_name);
        row.last_at = event.recorded_at;
        if !event.user_agent.is_empty() {
            row.last_user_agent.clone_from(&event.user_agent);
        }
        row.trend[grid.slice(event.recorded_at)] += event.total_tokens;
        if !event.session_id.is_empty() {
            sessions.insert(&event.session_id);
        }
    }
    let mut rows = groups
        .into_values()
        .map(|(mut row, sessions)| {
            row.sessions = sessions.len() as i64;
            row
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| std::cmp::Reverse(row.tokens.total_tokens));
    rows
}

fn consumer_shares(values: BTreeMap<String, ConsumerShare>) -> Vec<ConsumerShare> {
    let mut rows = values.into_values().collect::<Vec<_>>();
    rows.sort_by_key(|row| std::cmp::Reverse(row.requests));
    rows
}

fn add_consumer(target: &mut BTreeMap<String, ConsumerShare>, event: &UsageEventRow) {
    let entry = target
        .entry(event.user_id.clone())
        .or_insert_with(|| ConsumerShare {
            user_id: event.user_id.clone(),
            user_name: event.user_name.clone(),
            requests: 0,
            total_tokens: 0,
        });
    entry.requests += 1;
    entry.total_tokens += event.total_tokens;
}

fn upstream_accounts(events: &[UsageEventRow], grid: &Grid) -> Vec<UpstreamAccountStats> {
    #[derive(Default)]
    struct Accumulator {
        row: Option<UpstreamAccountStats>,
        samples: LatencySamples,
        sessions: HashSet<String>,
        consumers: BTreeMap<String, ConsumerShare>,
    }
    let mut groups = BTreeMap::<&str, Accumulator>::new();
    for event in events
        .iter()
        .filter(|event| !event.codex_account_id.is_empty())
    {
        let entry = groups.entry(&event.codex_account_id).or_default();
        let row = entry.row.get_or_insert_with(|| UpstreamAccountStats {
            codex_account_id: event.codex_account_id.clone(),
            codex_account_name: String::new(),
            account_group_id: String::new(),
            account_group_name: String::new(),
            outcomes: OutcomeCounts::default(),
            tokens: TokenCounts::default(),
            ttft: Percentiles::default(),
            latency: Percentiles::default(),
            fast_requests: 0,
            fast_fulfilled: 0,
            fast_downgraded: 0,
            sessions: 0,
            last_at: 0,
            rate_limit_series: vec![0; grid.buckets],
            trend: vec![0; TREND_SLICES],
            consumers: Vec::new(),
        });
        row.codex_account_name.clone_from(&event.codex_account_name);
        row.account_group_id.clone_from(&event.account_group_id);
        row.account_group_name.clone_from(&event.account_group_name);
        add_outcome(&mut row.outcomes, event);
        add_tokens(&mut row.tokens, event);
        row.fast_requests += i64::from(is_fast(event));
        row.fast_fulfilled += i64::from(is_fulfilled(event));
        row.fast_downgraded += i64::from(is_downgraded(event));
        row.last_at = event.recorded_at;
        if event.status_code == Some(HTTP_TOO_MANY_REQUESTS) {
            row.rate_limit_series[grid.bucket(event.recorded_at)] += 1;
        }
        row.trend[grid.slice(event.recorded_at)] += event.total_tokens;
        entry.samples.add(event);
        if !event.session_id.is_empty() {
            entry.sessions.insert(event.session_id.clone());
        }
        add_consumer(&mut entry.consumers, event);
    }
    let mut rows = groups
        .into_values()
        .filter_map(|entry| {
            let mut row = entry.row?;
            let (ttft, latency, _) = entry.samples.finish();
            row.ttft = ttft;
            row.latency = latency;
            row.sessions = entry.sessions.len() as i64;
            row.consumers = consumer_shares(entry.consumers);
            Some(row)
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| std::cmp::Reverse(row.tokens.total_tokens));
    rows
}

fn upstream_groups(events: &[UsageEventRow], grid: &Grid) -> Vec<UpstreamGroupStats> {
    #[derive(Default)]
    struct Accumulator {
        row: Option<UpstreamGroupStats>,
        sessions: HashMap<String, HashMap<String, i64>>,
        consumers: BTreeMap<String, ConsumerShare>,
        accounts: BTreeMap<String, GroupAccountShare>,
    }
    let mut groups = BTreeMap::<&str, Accumulator>::new();
    for event in events
        .iter()
        .filter(|event| !event.codex_account_id.is_empty())
    {
        let entry = groups.entry(&event.account_group_id).or_default();
        let row = entry.row.get_or_insert_with(|| UpstreamGroupStats {
            account_group_id: event.account_group_id.clone(),
            account_group_name: String::new(),
            outcomes: OutcomeCounts::default(),
            tokens: TokenCounts::default(),
            sessions: 0,
            session_affinity_rate: None,
            trend: vec![0; TREND_SLICES],
            consumers: Vec::new(),
            accounts: Vec::new(),
        });
        row.account_group_name.clone_from(&event.account_group_name);
        let account = entry
            .accounts
            .entry(event.codex_account_id.clone())
            .or_insert_with(|| GroupAccountShare {
                codex_account_id: event.codex_account_id.clone(),
                codex_account_name: event.codex_account_name.clone(),
                requests: 0,
                total_tokens: 0,
                rate_limited: 0,
            });
        account.requests += 1;
        account.total_tokens += event.total_tokens;
        account.rate_limited += i64::from(event.status_code == Some(HTTP_TOO_MANY_REQUESTS));
        add_outcome(&mut row.outcomes, event);
        add_tokens(&mut row.tokens, event);
        row.trend[grid.slice(event.recorded_at)] += event.total_tokens;
        if !event.session_id.is_empty() {
            *entry
                .sessions
                .entry(event.session_id.clone())
                .or_default()
                .entry(event.codex_account_id.clone())
                .or_default() += 1;
        }
        add_consumer(&mut entry.consumers, event);
    }
    let mut rows = groups
        .into_values()
        .filter_map(|entry| {
            let mut row = entry.row?;
            row.sessions = entry.sessions.len() as i64;
            let (pinned, total) =
                entry
                    .sessions
                    .values()
                    .fold((0, 0), |(pinned, total), accounts| {
                        let sum = accounts.values().sum::<i64>();
                        let main = accounts.values().copied().max().unwrap_or_default();
                        (pinned + main, total + sum)
                    });
            row.session_affinity_rate = (total > 0).then(|| pinned as f64 / total as f64);
            row.consumers = consumer_shares(entry.consumers);
            row.accounts = entry.accounts.into_values().collect();
            row.accounts
                .sort_by_key(|account| std::cmp::Reverse(account.total_tokens));
            Some(row)
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| std::cmp::Reverse(row.tokens.total_tokens));
    rows
}

fn identity_models(events: &[UsageEventRow]) -> Vec<IdentityModelCell> {
    let mut cells = BTreeMap::<(&str, &str), IdentityModelCell>::new();
    for event in events.iter().filter(|event| !event.model.is_empty()) {
        let cell = cells
            .entry((&event.user_id, &event.model))
            .or_insert_with(|| IdentityModelCell {
                user_id: event.user_id.clone(),
                user_name: event.user_name.clone(),
                model: event.model.clone(),
                requests: 0,
                total_tokens: 0,
                cost_usd: 0.0,
            });
        cell.requests += 1;
        cell.total_tokens += event.total_tokens;
        cell.cost_usd += event.cost_usd;
    }
    cells.into_values().collect()
}

fn effort_tiers(events: &[UsageEventRow]) -> Vec<EffortTierCell> {
    let mut cells = BTreeMap::<(&str, &str), EffortTierCell>::new();
    for event in events
        .iter()
        .filter(|event| !event.reasoning_effort.is_empty())
    {
        let cell = cells
            .entry((&event.reasoning_effort, &event.response_service_tier))
            .or_insert_with(|| EffortTierCell {
                effort: event.reasoning_effort.clone(),
                tier: event.response_service_tier.clone(),
                requests: 0,
                total_tokens: 0,
            });
        cell.requests += 1;
        cell.total_tokens += event.total_tokens;
    }
    cells.into_values().collect()
}

fn fast_stats(events: &[UsageEventRow]) -> FastStats {
    let mut stats = FastStats::default();
    let mut accounts = BTreeMap::<&str, FastBreakdown>::new();
    let mut models = BTreeMap::<&str, FastBreakdown>::new();
    for event in events.iter().filter(|event| is_fast(event)) {
        let fulfilled = i64::from(is_fulfilled(event));
        let downgraded = i64::from(is_downgraded(event));
        stats.requested += 1;
        stats.fulfilled += fulfilled;
        stats.downgraded += downgraded;
        for (map, key) in [
            (&mut accounts, event.codex_account_name.as_str()),
            (&mut models, event.model.as_str()),
        ] {
            if key.is_empty() {
                continue;
            }
            let entry = map.entry(key).or_insert_with(|| FastBreakdown {
                name: key.to_owned(),
                requested: 0,
                fulfilled: 0,
                downgraded: 0,
            });
            entry.requested += 1;
            entry.fulfilled += fulfilled;
            entry.downgraded += downgraded;
        }
    }
    let sorted = |map: BTreeMap<&str, FastBreakdown>| {
        let mut rows = map.into_values().collect::<Vec<_>>();
        rows.sort_by_key(|row| std::cmp::Reverse(row.requested));
        rows
    };
    stats.accounts = sorted(accounts);
    stats.models = sorted(models);
    stats
}

fn errors(events: &[UsageEventRow]) -> Vec<ErrorStats> {
    let mut groups =
        BTreeMap::<(&str, Option<u16>, &str), (ErrorStats, BTreeMap<String, i64>)>::new();
    for event in events
        .iter()
        .filter(|event| event.status != STATUS_COMPLETED)
    {
        let (row, accounts) = groups
            .entry((&event.status, event.status_code, &event.error_code))
            .or_insert_with(|| {
                (
                    ErrorStats {
                        status: event.status.clone(),
                        status_code: event.status_code,
                        error_code: event.error_code.clone(),
                        requests: 0,
                        last_at: 0,
                        accounts: Vec::new(),
                    },
                    BTreeMap::new(),
                )
            });
        row.requests += 1;
        row.last_at = event.recorded_at;
        if !event.codex_account_name.is_empty() {
            *accounts
                .entry(event.codex_account_name.clone())
                .or_default() += 1;
        }
    }
    let mut rows = groups
        .into_values()
        .map(|(mut row, accounts)| {
            row.accounts = named_counts(accounts);
            row
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        right
            .requests
            .cmp(&left.requests)
            .then(right.last_at.cmp(&left.last_at))
    });
    rows
}

fn session_stats(events: &[UsageEventRow]) -> SessionStats {
    let mut kinds = BTreeMap::<&str, (i64, HashSet<&str>)>::new();
    let mut sessions = HashMap::<&str, SessionSummary>::new();
    let mut children = HashMap::<&str, HashSet<&str>>::new();
    let mut with_session = 0;
    for event in events {
        let label = if event.session_id.is_empty() {
            ""
        } else {
            event.session_kind.as_str()
        };
        let entry = kinds.entry(label).or_default();
        entry.0 += 1;
        if event.session_id.is_empty() {
            continue;
        }
        with_session += 1;
        entry.1.insert(&event.session_id);
        if !event.parent_session_id.is_empty() {
            children
                .entry(&event.parent_session_id)
                .or_default()
                .insert(&event.session_id);
        }
        let summary = sessions
            .entry(&event.session_id)
            .or_insert_with(|| SessionSummary {
                session_id: event.session_id.clone(),
                session_kind: event.session_kind.clone(),
                parent_session_id: event.parent_session_id.clone(),
                user_name: event.user_name.clone(),
                requests: 0,
                child_sessions: 0,
                total_tokens: 0,
                cost_usd: 0.0,
                first_at: event.recorded_at,
                last_at: event.recorded_at,
            });
        summary.requests += 1;
        summary.total_tokens += event.total_tokens;
        summary.cost_usd += event.cost_usd;
        summary.last_at = event.recorded_at;
    }
    let mut durations = sessions
        .values()
        .map(|session| session.last_at - session.first_at)
        .collect::<Vec<_>>();
    let mut recent = sessions
        .values()
        .filter(|session| session.parent_session_id.is_empty())
        .cloned()
        .map(|mut session| {
            session.child_sessions = children
                .get(session.session_id.as_str())
                .map_or(0, |children| children.len() as i64);
            session
        })
        .collect::<Vec<_>>();
    recent.sort_by_key(|row| std::cmp::Reverse(row.last_at));
    recent.truncate(TOP_LIMIT);
    let mut kinds = kinds
        .into_iter()
        .map(|(kind, (requests, sessions))| SessionKindStats {
            kind: kind.to_owned(),
            requests,
            sessions: sessions.len() as i64,
        })
        .collect::<Vec<_>>();
    kinds.sort_by_key(|row| std::cmp::Reverse(row.requests));
    SessionStats {
        sessions: sessions.len() as i64,
        requests_with_session: with_session,
        max_requests: sessions
            .values()
            .map(|session| session.requests)
            .max()
            .unwrap_or_default(),
        median_duration_ms: percentiles(&mut durations).p50,
        kinds,
        recent,
    }
}

/// Splits a User-Agent such as `codex_cli_rs/0.63.0 (Mac OS 26.1.0; arm64) iTerm.app/3.6.5`.
pub fn parse_user_agent(value: &str) -> (String, String, String) {
    let value = value.trim();
    let product = value.split_whitespace().next().unwrap_or_default();
    let (client, version) = product.split_once('/').unwrap_or((product, ""));
    let os = value
        .split_once('(')
        .and_then(|(_, rest)| rest.split_once(')'))
        .map(|(inside, _)| {
            inside
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned()
        })
        .unwrap_or_default();
    (client.to_owned(), version.to_owned(), os)
}

fn client_overview(events: &[UsageEventRow]) -> ClientOverview {
    #[derive(Default)]
    struct Accumulator {
        row: Option<ClientStats>,
        samples: LatencySamples,
    }
    let mut clients = BTreeMap::<(String, String, String), Accumulator>::new();
    let mut ips = BTreeMap::<&str, IpStats>::new();
    let mut forwarded = 0;
    for event in events {
        let key = parse_user_agent(&event.user_agent);
        let entry = clients.entry(key.clone()).or_default();
        let row = entry.row.get_or_insert_with(|| ClientStats {
            client: key.0,
            version: key.1,
            os: key.2,
            requests: 0,
            failed: 0,
            websocket_requests: 0,
            ttft: Percentiles::default(),
            last_at: 0,
        });
        row.requests += 1;
        row.failed += i64::from(is_failure(event));
        row.websocket_requests += i64::from(event.transport == "websocket");
        row.last_at = event.recorded_at;
        entry.samples.add(event);
        if !event.x_forwarded_for.is_empty() {
            forwarded += 1;
        }
        if !event.client_ip.is_empty() {
            let ip = ips.entry(&event.client_ip).or_insert_with(|| IpStats {
                ip: event.client_ip.clone(),
                forwarded_for: String::new(),
                requests: 0,
                last_at: 0,
            });
            ip.requests += 1;
            ip.last_at = event.recorded_at;
            if !event.x_forwarded_for.is_empty() {
                ip.forwarded_for.clone_from(&event.x_forwarded_for);
            }
        }
    }
    let distinct_clients = clients.len() as i64;
    let distinct_ips = ips.len() as i64;
    let mut clients = clients
        .into_values()
        .filter_map(|entry| {
            let mut row = entry.row?;
            row.ttft = entry.samples.finish().0;
            Some(row)
        })
        .collect::<Vec<_>>();
    clients.sort_by_key(|row| std::cmp::Reverse(row.requests));
    clients.truncate(TOP_LIMIT);
    let mut ips = ips.into_values().collect::<Vec<_>>();
    ips.sort_by_key(|row| std::cmp::Reverse(row.requests));
    ips.truncate(TOP_LIMIT);
    ClientOverview {
        clients,
        distinct_clients,
        ips,
        distinct_ips,
        forwarded_requests: forwarded,
    }
}

fn transport_overview(events: &[UsageEventRow]) -> TransportOverview {
    fn grouped<'a>(
        events: &'a [UsageEventRow],
        key: impl Fn(&'a UsageEventRow) -> String,
    ) -> Vec<TransportStats> {
        let mut groups = BTreeMap::<String, (OutcomeCounts, LatencySamples, i64)>::new();
        for event in events {
            let entry = groups.entry(key(event)).or_default();
            add_outcome(&mut entry.0, event);
            entry.1.add(event);
            entry.2 += event.input_tokens;
        }
        let mut rows = groups
            .into_iter()
            .map(|(name, (outcomes, samples, input))| {
                let (ttft, latency, _) = samples.finish();
                TransportStats {
                    average_input_tokens: input as f64 / outcomes.requests.max(1) as f64,
                    name,
                    outcomes,
                    ttft,
                    latency,
                }
            })
            .collect::<Vec<_>>();
        rows.sort_by_key(|row| std::cmp::Reverse(row.outcomes.requests));
        rows
    }
    TransportOverview {
        transports: grouped(events, |event| event.transport.clone()),
        streaming: grouped(events, |event| {
            if event.stream { "stream" } else { "non_stream" }.to_owned()
        }),
        endpoints: grouped(events, |event| event.endpoint.clone()),
    }
}

fn latency_density(events: &[UsageEventRow]) -> LatencyDensity {
    fn bin(edges: &[i64], value: i64) -> usize {
        edges
            .iter()
            .rposition(|edge| value >= *edge)
            .unwrap_or_default()
    }
    let mut density = LatencyDensity {
        ttft_edges_ms: TTFT_EDGES_MS.to_vec(),
        latency_edges_ms: LATENCY_EDGES_MS.to_vec(),
        cells: vec![vec![0; TTFT_EDGES_MS.len()]; LATENCY_EDGES_MS.len()],
        missing_ttft: 0,
    };
    for event in events.iter().filter(|event| is_success(event)) {
        match event.ttft_ms {
            Some(ttft) => {
                density.cells[bin(&LATENCY_EDGES_MS, event.latency_ms)]
                    [bin(&TTFT_EDGES_MS, ttft)] += 1;
            }
            None => density.missing_ttft += 1,
        }
    }
    density
}

fn unpriced_models(events: &[UsageEventRow], costs: &CostIndex) -> Vec<UnpricedModel> {
    let mut models = BTreeMap::<&str, UnpricedModel>::new();
    for event in events.iter().filter(|event| {
        !event.model.is_empty() && event.total_tokens > 0 && !costs.is_priced(&event.model)
    }) {
        let entry = models.entry(&event.model).or_insert_with(|| UnpricedModel {
            model: event.model.clone(),
            requests: 0,
            total_tokens: 0,
            first_at: event.recorded_at,
            last_at: event.recorded_at,
        });
        entry.requests += 1;
        entry.total_tokens += event.total_tokens;
        entry.last_at = event.recorded_at;
    }
    let mut rows = models.into_values().collect::<Vec<_>>();
    rows.sort_by_key(|row| std::cmp::Reverse(row.total_tokens));
    rows
}

/// Groups requests into session trees (root session → child sessions); requests
/// without a session are grouped per identity. Most recently active first.
fn session_groups(events: &[UsageEventRow]) -> Vec<SessionGroup> {
    let parents = events
        .iter()
        .filter(|event| !event.session_id.is_empty() && !event.parent_session_id.is_empty())
        .map(|event| (event.session_id.as_str(), event.parent_session_id.as_str()))
        .collect::<HashMap<_, _>>();
    let root_of = |session: &str| {
        let mut current = session;
        for _ in 0..16 {
            match parents.get(current) {
                Some(parent) if *parent != current => current = parent,
                _ => break,
            }
        }
        current.to_owned()
    };
    let mut groups = HashMap::<(String, String), SessionGroup>::new();
    let mut children = HashMap::<(String, String), BTreeMap<String, SessionChild>>::new();
    for event in events {
        let key = if event.session_id.is_empty() {
            (String::new(), event.user_id.clone())
        } else {
            (root_of(&event.session_id), String::new())
        };
        let group = groups.entry(key.clone()).or_insert_with(|| SessionGroup {
            session_id: key.0.clone(),
            user_id: event.user_id.clone(),
            user_name: event.user_name.clone(),
            account_group_name: event.account_group_name.clone(),
            accounts: Vec::new(),
            user_agent: event.user_agent.clone(),
            outcomes: OutcomeCounts::default(),
            total_tokens: 0,
            cost_usd: 0.0,
            first_at: event.recorded_at,
            last_at: event.recorded_at,
            kinds: BTreeMap::new(),
            timeline: Vec::new(),
            children: Vec::new(),
            events: Vec::new(),
            event_count: 0,
        });
        add_outcome(&mut group.outcomes, event);
        group.total_tokens += event.total_tokens;
        group.cost_usd += event.cost_usd;
        group.last_at = event.recorded_at;
        if !event.codex_account_name.is_empty()
            && !group.accounts.contains(&event.codex_account_name)
        {
            group.accounts.push(event.codex_account_name.clone());
        }
        *group.kinds.entry(event.session_kind.clone()).or_default() += 1;
        group
            .timeline
            .push((event.recorded_at, event.latency_ms, event.status.clone()));
        let is_child = !event.session_id.is_empty() && event.session_id != key.0;
        if is_child {
            let child = children
                .entry(key.clone())
                .or_default()
                .entry(event.session_id.clone())
                .or_insert_with(|| SessionChild {
                    session_id: event.session_id.clone(),
                    session_kind: event.session_kind.clone(),
                    outcomes: OutcomeCounts::default(),
                    total_tokens: 0,
                    cost_usd: 0.0,
                    first_at: event.recorded_at,
                    last_at: event.recorded_at,
                    accounts: Vec::new(),
                    fast_downgraded: 0,
                    timeline: Vec::new(),
                });
            add_outcome(&mut child.outcomes, event);
            child.total_tokens += event.total_tokens;
            child.cost_usd += event.cost_usd;
            child.last_at = event.recorded_at;
            child.fast_downgraded += i64::from(is_downgraded(event));
            if !event.codex_account_name.is_empty()
                && !child.accounts.contains(&event.codex_account_name)
            {
                child.accounts.push(event.codex_account_name.clone());
            }
            child
                .timeline
                .push((event.recorded_at, event.latency_ms, event.status.clone()));
        } else {
            group.event_count += 1;
            group.events.push(event.clone());
        }
    }
    let mut rows = groups
        .into_iter()
        .map(|(key, mut group)| {
            group.events.reverse();
            group.events.truncate(SESSION_EVENT_LIMIT);
            let mut list = children
                .remove(&key)
                .map(|map| map.into_values().collect::<Vec<_>>())
                .unwrap_or_default();
            list.sort_by_key(|row| std::cmp::Reverse(row.last_at));
            group.children = list;
            group
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| std::cmp::Reverse(row.last_at));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(at: i64, status: &str) -> UsageEventRow {
        UsageEventRow {
            id: at,
            recorded_at: at,
            request_id: format!("req_{at}"),
            response_id: String::new(),
            credential_type: "api_key".into(),
            user_id: "key".into(),
            user_name: "Key".into(),
            credential_id: "credential".into(),
            credential_name: "laptop".into(),
            codex_account_id: "acct".into(),
            codex_account_name: "a@example.com".into(),
            account_group_id: "group".into(),
            account_group_name: "Pool".into(),
            requested_model: "gpt-5".into(),
            model: "gpt-5".into(),
            transport: "http".into(),
            endpoint: "/v1/responses".into(),
            stream: true,
            status: status.into(),
            status_code: Some(200),
            error_code: String::new(),
            reasoning_effort: "high".into(),
            service_tier: "priority".into(),
            response_service_tier: "default".into(),
            session_id: String::new(),
            parent_session_id: String::new(),
            session_kind: String::new(),
            client_ip: "10.0.0.1".into(),
            x_forwarded_for: String::new(),
            user_agent: "codex_cli_rs/0.63.0 (Mac OS 26.1.0; arm64) iTerm.app/3.6.5".into(),
            latency_ms: 1_000,
            ttft_ms: Some(200),
            input_tokens: 100,
            cached_input_tokens: 50,
            cache_creation_input_tokens: 0,
            output_tokens: 10,
            reasoning_output_tokens: 2,
            total_tokens: 110,
            cost_usd: 0.0,
        }
    }

    fn insert(connection: &Connection, event: &UsageEventRow) {
        connection
            .execute(
                &format!(
                    "INSERT INTO usage_events ({}) VALUES ({})",
                    EVENT_COLUMNS.replace("id, recorded_at_ms", "recorded_at_ms"),
                    (1..=37)
                        .map(|index| format!("?{index}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                rusqlite::params![
                    event.recorded_at,
                    event.request_id,
                    event.response_id,
                    event.credential_type,
                    event.user_id,
                    event.user_name,
                    event.codex_account_id,
                    event.codex_account_name,
                    event.account_group_id,
                    event.account_group_name,
                    event.requested_model,
                    event.model,
                    event.transport,
                    event.endpoint,
                    event.stream,
                    event.status,
                    event.status_code,
                    event.error_code,
                    event.reasoning_effort,
                    event.service_tier,
                    event.response_service_tier,
                    event.session_id,
                    event.parent_session_id,
                    event.session_kind,
                    event.client_ip,
                    event.x_forwarded_for,
                    event.user_agent,
                    event.latency_ms,
                    event.ttft_ms,
                    event.input_tokens,
                    event.cached_input_tokens,
                    event.cache_creation_input_tokens,
                    event.output_tokens,
                    event.reasoning_output_tokens,
                    event.total_tokens,
                    event.credential_id,
                    event.credential_name
                ],
            )
            .unwrap();
    }

    #[tokio::test]
    async fn queries_analytics_and_paged_events_from_the_store() {
        let path = std::env::temp_dir().join(format!(
            "codex-router-analytics-{}.sqlite3",
            uuid::Uuid::new_v4()
        ));
        let store = UsageStore::open(path.clone()).unwrap();
        let now = 10 * DAY;
        let mut events = Vec::new();
        for index in 0..6 {
            let mut current = event(now - HOUR + index * MINUTE, "completed");
            current.request_id = format!("req_now_{index}");
            current.latency_ms = 1_000 * (index + 1);
            current.session_id = "root".into();
            current.session_kind = "main".into();
            events.push(current);
        }
        let mut failed = event(now - 30 * MINUTE, "failed");
        failed.request_id = "req_failed".into();
        failed.status_code = Some(429);
        failed.error_code = "usage_limit_reached".into();
        failed.model = "gpt-5-mini".into();
        events.push(failed);
        let mut previous = event(now - DAY - HOUR, "completed");
        previous.request_id = "req_previous".into();
        events.push(previous);
        let prepared = events.clone();
        store
            .read(move |connection| {
                for event in &prepared {
                    insert(connection, event);
                }
                Ok(())
            })
            .await
            .unwrap();
        let prices = vec![ModelPrice {
            model: "gpt-5".into(),
            input: 1.0,
            output: 10.0,
            cache_read: 0.1,
            cache_write: 0.0,
            multiplier: 1.0,
        }];
        let request = AnalyticsRequest {
            range: AnalyticsRange::Day,
            start_at: now - DAY,
            end_at: now,
            bucket_ms: HOUR,
            filters: EventFilters::default(),
        };
        let analytics = store
            .analytics(request.clone(), &prices, 2.0, now)
            .await
            .unwrap();
        assert_eq!(analytics.totals.outcomes.requests, 7);
        assert_eq!(analytics.totals.outcomes.rate_limited, 1);
        assert_eq!(analytics.previous.as_ref().unwrap().outcomes.requests, 1);
        assert_eq!(analytics.series.len(), 24);
        assert_eq!(analytics.unpriced_models[0].model, "gpt-5-mini");
        assert_eq!(analytics.errors[0].error_code, "usage_limit_reached");
        assert!(analytics.totals.tokens.cost_usd > 0.0);
        assert_eq!(analytics.sessions.sessions, 1);

        let page = store
            .event_page(
                EventPageRequest {
                    analytics: request.clone(),
                    view: EventView::Failed,
                    offset: 0,
                    limit: 50,
                    group_by_session: false,
                },
                &prices,
                2.0,
                now,
            )
            .await
            .unwrap();
        assert_eq!(page.range_total, 7);
        assert_eq!(page.views.failed, 1);
        assert_eq!(page.views.rate_limited, 1);
        assert_eq!(page.total, 1);
        assert_eq!(page.events[0].request_id, "req_failed");
        assert_eq!(page.models, vec!["gpt-5", "gpt-5-mini"]);
        let last_seen = store.last_seen().await.unwrap();
        assert_eq!(last_seen.models.len(), 2);
        assert_eq!(last_seen.identities[0].last_at, now - 30 * MINUTE);

        let mut filtered = request.clone();
        filtered.filters.search = Some("NOW_5".into());
        let page = store
            .event_page(
                EventPageRequest {
                    analytics: filtered,
                    view: EventView::All,
                    offset: 0,
                    limit: 1,
                    group_by_session: false,
                },
                &prices,
                2.0,
                now,
            )
            .await
            .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.events[0].request_id, "req_now_5");
        assert_eq!(page.session_counts["root"]["main"], 6);

        let grouped = store
            .event_page(
                EventPageRequest {
                    analytics: request,
                    view: EventView::All,
                    offset: 0,
                    limit: 10,
                    group_by_session: true,
                },
                &prices,
                2.0,
                now,
            )
            .await
            .unwrap();
        let sessions = grouped.sessions.unwrap();
        assert_eq!(grouped.total, 2);
        assert_eq!(
            sessions
                .iter()
                .find(|group| group.session_id == "root")
                .unwrap()
                .outcomes
                .requests,
            6
        );

        drop(store);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
        let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
    }

    #[test]
    fn nearest_rank_percentiles() {
        let mut values = (1..=100).collect::<Vec<i64>>();
        let result = percentiles(&mut values);
        assert_eq!(result.p50, Some(50));
        assert_eq!(result.p95, Some(95));
        assert_eq!(result.max, Some(100));
        assert_eq!(percentiles(&mut []).p50, None);
    }

    #[test]
    fn parses_codex_and_generic_user_agents() {
        assert_eq!(
            parse_user_agent("codex_cli_rs/0.63.0 (Mac OS 26.1.0; arm64) iTerm.app/3.6.5"),
            (
                "codex_cli_rs".into(),
                "0.63.0".into(),
                "Mac OS 26.1.0".into()
            )
        );
        assert_eq!(
            parse_user_agent("python-httpx/0.28.1"),
            ("python-httpx".into(), "0.28.1".into(), String::new())
        );
    }

    #[test]
    fn counts_outcomes_fast_downgrades_and_buckets() {
        let mut failed = event(3_600_000 + 5, "failed");
        failed.status_code = Some(429);
        let events = vec![event(10, "completed"), failed];
        let grid = Grid::new(0, 7_200_000, HOUR);
        let points = series(&events, &grid);
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].outcomes.completed, 1);
        assert_eq!(points[1].outcomes.failed, 1);
        assert_eq!(points[1].outcomes.rate_limited, 1);
        assert_eq!(points[0].fast_downgraded, 1);
        let fast = fast_stats(&events);
        assert_eq!((fast.requested, fast.fulfilled, fast.downgraded), (2, 0, 2));
        let accounts = upstream_accounts(&events, &grid);
        assert_eq!(accounts[0].rate_limit_series, vec![0, 1]);
    }

    #[test]
    fn groups_sessions_into_trees() {
        let mut main = event(10, "completed");
        main.session_id = "root".into();
        main.session_kind = "main".into();
        let mut child = event(20, "completed");
        child.session_id = "child".into();
        child.parent_session_id = "root".into();
        child.session_kind = "subagent".into();
        let loose = event(30, "failed");
        let groups = session_groups(&[main, child, loose]);
        assert_eq!(groups.len(), 2);
        let root = groups
            .iter()
            .find(|group| group.session_id == "root")
            .unwrap();
        assert_eq!(root.outcomes.requests, 2);
        assert_eq!(root.children.len(), 1);
        assert_eq!(root.event_count, 1);
        let loose = groups
            .iter()
            .find(|group| group.session_id.is_empty())
            .unwrap();
        assert_eq!(loose.user_id, "key");
    }
}

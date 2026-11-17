use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use bytes::Bytes;
use futures_util::{Stream, StreamExt, stream};
use rusqlite::{Connection, params};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::{
    auth::{AccountRoutingState, AuthenticatedUser, User},
    core::ApiError,
    http::parse_json_body,
    protocol::openai,
};

use super::{
    oauth::current_time_ms,
    pricing::{ModelPrice, calculate_cost, price_index},
    usage_client::{SessionKind, UsageClient},
};

const MAX_TRACKED_JSON_BYTES: usize = 16 * 1024 * 1024;
const RECENT_EVENT_LIMIT: i64 = 50;
const BREAKDOWN_LIMIT: i64 = 12;
const ACTIVITY_BUCKET_COUNT: i64 = 7 * 24;
const MAX_ERROR_CODE_CHARS: usize = 128;

/// Bumped whenever the table layout changes; tables from other versions are dropped and rebuilt.
const USAGE_SCHEMA_VERSION: i64 = 3;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS usage_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    recorded_at_ms INTEGER NOT NULL,
    request_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    user_name TEXT NOT NULL,
    credential_type TEXT NOT NULL,
    credential_id TEXT NOT NULL,
    credential_name TEXT NOT NULL,
    codex_account_id TEXT NOT NULL,
    codex_account_name TEXT NOT NULL,
    account_group_id TEXT NOT NULL,
    account_group_name TEXT NOT NULL,
    requested_model TEXT NOT NULL,
    model TEXT NOT NULL,
    transport TEXT NOT NULL,
    endpoint TEXT NOT NULL,
    stream INTEGER NOT NULL,
    status TEXT NOT NULL,
    status_code INTEGER,
    error_code TEXT NOT NULL,
    response_id TEXT NOT NULL,
    reasoning_effort TEXT NOT NULL,
    service_tier TEXT NOT NULL,
    response_service_tier TEXT NOT NULL,
    session_id TEXT NOT NULL,
    parent_session_id TEXT NOT NULL,
    session_kind TEXT NOT NULL,
    client_ip TEXT NOT NULL,
    x_forwarded_for TEXT NOT NULL,
    user_agent TEXT NOT NULL,
    latency_ms INTEGER NOT NULL,
    ttft_ms INTEGER,
    input_tokens INTEGER NOT NULL,
    cached_input_tokens INTEGER NOT NULL,
    cache_creation_input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    reasoning_output_tokens INTEGER NOT NULL,
    total_tokens INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_usage_events_recorded_at
    ON usage_events(recorded_at_ms DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_usage_events_user
    ON usage_events(user_id, recorded_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_usage_events_credential
    ON usage_events(credential_id, recorded_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_usage_events_model
    ON usage_events(model, recorded_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_usage_events_codex_account
    ON usage_events(codex_account_id, recorded_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_usage_events_account_group
    ON usage_events(account_group_id, recorded_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_usage_events_session
    ON usage_events(session_id, recorded_at_ms DESC) WHERE session_id <> '';
CREATE UNIQUE INDEX IF NOT EXISTS idx_usage_events_response_id
    ON usage_events(response_id) WHERE response_id <> '';
"#;

/// Recorded when the request was observed but did not set the field explicitly.
const DEFAULT_REQUEST_SETTING: &str = "default";
pub(super) const PRIORITY_SERVICE_TIER: &str = "priority";
/// Requests without an explicit tier let the upstream choose.
const AUTO_SERVICE_TIER: &str = "auto";
/// Groups cost queries by whether the request used Fast mode.
const FAST_SQL: &str = "(service_tier = 'priority')";
pub const DEFAULT_FAST_COST_MULTIPLIER: f64 = 2.0;

/// The upstream finished the response.
pub(super) const STATUS_COMPLETED: &str = "completed";
/// The upstream stopped early, for example at `max_output_tokens`.
pub(super) const STATUS_INCOMPLETE: &str = "incomplete";
/// The upstream or the router rejected the request.
pub(super) const STATUS_FAILED: &str = "failed";
/// The client went away (or started another response) before the upstream finished.
pub(super) const STATUS_CANCELLED: &str = "cancelled";
/// The upstream body ended or broke without a terminal event.
pub(super) const STATUS_INTERRUPTED: &str = "interrupted";
/// Request health: successful and failed buckets; cancellations count as neither.
const SUCCESS_SQL: &str = "(status IN ('completed', 'incomplete'))";
const FAILURE_SQL: &str = "(status IN ('failed', 'interrupted'))";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageIdentity {
    pub user_id: String,
    pub user_name: String,
    pub credential_type: &'static str,
    pub credential_id: String,
    pub credential_name: String,
    pub codex_account_id: String,
    pub codex_account_name: String,
    pub account_group_id: String,
    pub account_group_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageIdentityKind {
    User,
    Credential,
    CodexAccount,
    AccountGroup,
}

impl UsageIdentityKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Credential => "credential",
            Self::CodexAccount => "codex_account",
            Self::AccountGroup => "account_group",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageIdentityFilter {
    kind: UsageIdentityKind,
    id: String,
}

impl UsageIdentityFilter {
    pub fn parse_any(kind: &str, id: &str) -> Option<Self> {
        let kind = match kind {
            "user" => UsageIdentityKind::User,
            "credential" => UsageIdentityKind::Credential,
            "codex_account" => UsageIdentityKind::CodexAccount,
            "account_group" => UsageIdentityKind::AccountGroup,
            _ => return None,
        };
        if id.is_empty() || id.len() > 256 {
            return None;
        }
        Some(Self {
            kind,
            id: id.to_owned(),
        })
    }

    pub fn parse_downstream(kind: &str, id: &str) -> Option<Self> {
        Self::parse_any(kind, id).filter(|filter| {
            matches!(
                filter.kind,
                UsageIdentityKind::User | UsageIdentityKind::Credential
            )
        })
    }

    pub fn parse_upstream(kind: &str, id: &str) -> Option<Self> {
        Self::parse_any(kind, id).filter(|filter| {
            matches!(
                filter.kind,
                UsageIdentityKind::CodexAccount | UsageIdentityKind::AccountGroup
            )
        })
    }

    pub(super) fn kind(&self) -> &'static str {
        self.kind.as_str()
    }

    pub(super) fn id(&self) -> &str {
        &self.id
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageFilters {
    downstream: Option<UsageIdentityFilter>,
    upstream: Option<UsageIdentityFilter>,
}

impl UsageFilters {
    pub fn new(
        downstream: Option<UsageIdentityFilter>,
        upstream: Option<UsageIdentityFilter>,
    ) -> Self {
        Self {
            downstream,
            upstream,
        }
    }

    pub fn from_identity(filter: UsageIdentityFilter) -> Self {
        match filter.kind {
            UsageIdentityKind::User | UsageIdentityKind::Credential => {
                Self::new(Some(filter), None)
            }
            UsageIdentityKind::CodexAccount | UsageIdentityKind::AccountGroup => {
                Self::new(None, Some(filter))
            }
        }
    }

    pub fn upstream_is_account(&self) -> bool {
        self.upstream
            .as_ref()
            .is_some_and(|filter| filter.kind == UsageIdentityKind::CodexAccount)
    }
}

impl From<&AuthenticatedUser> for UsageIdentity {
    fn from(value: &AuthenticatedUser) -> Self {
        Self {
            user_id: value.user.id.clone(),
            user_name: value.user.name.clone(),
            credential_type: value.credential.kind.as_str(),
            credential_id: value.credential.id.clone(),
            credential_name: value.credential.display_name(),
            codex_account_id: String::new(),
            codex_account_name: String::new(),
            account_group_id: String::new(),
            account_group_name: String::new(),
        }
    }
}

impl UsageIdentity {
    pub fn with_account_route(
        mut self,
        account_id: &str,
        account_name: &str,
        group: Option<(&str, &str)>,
    ) -> Self {
        self.codex_account_id = account_id.into();
        self.codex_account_name = account_name.into();
        if let Some((id, name)) = group {
            self.account_group_id = id.into();
            self.account_group_name = name.into();
        }
        self
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TokenUsage {
    input_tokens: i64,
    cached_input_tokens: i64,
    cache_creation_input_tokens: i64,
    output_tokens: i64,
    reasoning_output_tokens: i64,
    total_tokens: i64,
}

/// One upstream request attempt; exactly one row is written per attempt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct UsageEvent {
    recorded_at_ms: i64,
    request_id: String,
    user_id: String,
    user_name: String,
    credential_type: String,
    credential_id: String,
    credential_name: String,
    codex_account_id: String,
    codex_account_name: String,
    account_group_id: String,
    account_group_name: String,
    requested_model: String,
    /// Model reported by the upstream, falling back to the requested model; used for pricing.
    model: String,
    transport: String,
    endpoint: String,
    stream: bool,
    status: String,
    status_code: Option<u16>,
    error_code: String,
    response_id: String,
    reasoning_effort: String,
    service_tier: String,
    response_service_tier: String,
    session_id: String,
    parent_session_id: String,
    session_kind: String,
    client_ip: String,
    x_forwarded_for: String,
    user_agent: String,
    latency_ms: i64,
    ttft_ms: Option<i64>,
    tokens: TokenUsage,
}

/// How an attempt ended, as reported by the upstream or inferred by the router.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Outcome {
    status: &'static str,
    status_code: Option<u16>,
    error_code: String,
    response_id: String,
    response_model: String,
    response_service_tier: String,
    tokens: TokenUsage,
}

impl Outcome {
    fn new(status: &'static str, error_code: &str) -> Self {
        Self {
            status,
            error_code: error_code.into(),
            ..Self::default()
        }
    }
}

#[derive(Clone)]
pub struct UsageStore {
    connection: Arc<Mutex<Connection>>,
    path: Arc<PathBuf>,
}

impl UsageStore {
    pub fn open(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create usage database directory {}",
                    parent.display()
                )
            })?;
        }
        let connection = Connection::open(&path)
            .with_context(|| format!("failed to open usage database {}", path.display()))?;
        restrict_database_permissions(&path)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .context("failed to configure usage database busy timeout")?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .context("failed to enable usage database WAL mode")?;
        connection
            .pragma_update(None, "synchronous", "NORMAL")
            .context("failed to configure usage database synchronization")?;
        prepare_schema(&connection)?;
        restrict_database_permissions(&path)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            path: Arc::new(path),
        })
    }

    pub fn path(&self) -> &Path {
        self.path.as_ref()
    }

    /// Persists without blocking the caller; falls back to a direct write outside a runtime.
    fn record(&self, event: UsageEvent) {
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let store = self.clone();
            runtime.spawn(async move {
                if let Err(error) = store.insert(event).await {
                    tracing::warn!(event = "usage_record", status = "failed", error = %error);
                }
            });
        } else if let Err(error) = insert_event(&self.lock(), &event) {
            tracing::warn!(event = "usage_record", status = "failed", error = %error);
        }
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.connection
            .lock()
            .expect("usage database lock poisoned")
    }

    async fn insert(&self, event: UsageEvent) -> Result<()> {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let connection = connection.lock().expect("usage database lock poisoned");
            insert_event(&connection, &event)
        })
        .await
        .context("usage database writer task failed")??;
        Ok(())
    }

    #[cfg(test)]
    pub async fn dashboard(
        &self,
        range: UsageRange,
        filters: UsageFilters,
    ) -> Result<UsageDashboard> {
        self.dashboard_with_options(
            range,
            filters,
            None,
            &[],
            DEFAULT_FAST_COST_MULTIPLIER,
            UsageNames::default(),
        )
        .await
    }

    pub async fn dashboard_with_options(
        &self,
        range: UsageRange,
        filters: UsageFilters,
        bounds: Option<UsageBounds>,
        prices: &[ModelPrice],
        fast_cost_multiplier: f64,
        names: UsageNames,
    ) -> Result<UsageDashboard> {
        let connection = self.connection.clone();
        let costs = CostIndex::new(prices, fast_cost_multiplier);
        tokio::task::spawn_blocking(move || {
            let connection = connection.lock().expect("usage database lock poisoned");
            let mut dashboard = query_dashboard(&connection, range, &filters, bounds, &costs)?;
            for identity in &mut dashboard.identities {
                names.apply_user(&identity.user_id, &mut identity.user_name);
            }
            for event in &mut dashboard.recent_events {
                names.apply_event(event);
            }
            Ok(dashboard)
        })
        .await
        .context("usage database query task failed")?
    }

    /// Runs a read-only query on the usage database off the async runtime.
    pub(super) async fn read<T, F>(&self, query: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
    {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let connection = connection.lock().expect("usage database lock poisoned");
            query(&connection)
        })
        .await
        .context("usage database query task failed")?
    }

    pub async fn used_models(&self) -> Result<Vec<String>> {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let connection = connection.lock().expect("usage database lock poisoned");
            let mut statement = connection.prepare(
                "SELECT DISTINCT model FROM usage_events WHERE model <> '' ORDER BY model",
            )?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await
        .context("usage model query task failed")?
    }
}

/// Creates the current table layout, discarding a table written by any other schema version.
fn prepare_schema(connection: &Connection) -> Result<()> {
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let reset = if version == USAGE_SCHEMA_VERSION {
        ""
    } else {
        let existing: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'usage_events')",
            [],
            |row| row.get(0),
        )?;
        if existing {
            tracing::warn!(
                event = "usage_schema_reset",
                previous_version = version,
                current_version = USAGE_SCHEMA_VERSION,
                "dropping usage events recorded with an incompatible schema"
            );
        }
        "DROP TABLE IF EXISTS usage_events;"
    };
    connection
        .execute_batch(&format!(
            "BEGIN; {reset} {SCHEMA} PRAGMA user_version = {USAGE_SCHEMA_VERSION}; COMMIT;"
        ))
        .context("failed to initialize usage database schema")
}

fn insert_event(connection: &Connection, event: &UsageEvent) -> Result<()> {
    connection.execute(
        r#"
        INSERT OR IGNORE INTO usage_events (
            recorded_at_ms, request_id, user_id, user_name,
            credential_type, credential_id, credential_name, codex_account_id, codex_account_name, account_group_id, account_group_name,
            requested_model, model, transport, endpoint, stream,
            status, status_code, error_code, response_id,
            reasoning_effort, service_tier, response_service_tier,
            session_id, parent_session_id, session_kind,
            client_ip, x_forwarded_for, user_agent, latency_ms, ttft_ms,
            input_tokens, cached_input_tokens, cache_creation_input_tokens,
            output_tokens, reasoning_output_tokens, total_tokens
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18,
            ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35,
            ?36, ?37
        )
        "#,
        params![
            event.recorded_at_ms,
            event.request_id,
            event.user_id,
            event.user_name,
            event.credential_type,
            event.credential_id,
            event.credential_name,
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
            event.response_id,
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
            event.tokens.input_tokens,
            event.tokens.cached_input_tokens,
            event.tokens.cache_creation_input_tokens,
            event.tokens.output_tokens,
            event.tokens.reasoning_output_tokens,
            event.tokens.total_tokens,
        ],
    )?;
    Ok(())
}

#[cfg(unix)]
fn restrict_database_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    for candidate in [
        path.to_path_buf(),
        sqlite_sidecar_path(path, "-wal"),
        sqlite_sidecar_path(path, "-shm"),
    ] {
        let metadata = match std::fs::metadata(&candidate) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o600);
        std::fs::set_permissions(candidate, permissions)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn restrict_database_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn sqlite_sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transport {
    Http,
    WebSocket,
}

impl Transport {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::WebSocket => "websocket",
        }
    }
}

/// Records one usage row per upstream attempt.
///
/// An HTTP tracker covers a single attempt that starts when the tracker is
/// created. A WebSocket tracker starts an attempt for every `response.create`
/// or `response.append`. An attempt ends at its terminal event, an explicit
/// failure, or — when the last clone is dropped — as cancelled or interrupted.
#[derive(Clone)]
pub struct UsageTracker {
    inner: Arc<UsageTrackerInner>,
}

struct UsageTrackerInner {
    store: UsageStore,
    identity: UsageIdentity,
    client: UsageClient,
    endpoint: String,
    transport: Transport,
    state: Mutex<TrackerState>,
}

#[derive(Default)]
struct TrackerState {
    request: RequestSettings,
    /// Set when a converted protocol decides streaming instead of the upstream body.
    client_stream: Option<bool>,
    attempt: Option<Attempt>,
}

/// Request-side attributes copied onto every usage event recorded by a tracker.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RequestSettings {
    model: String,
    reasoning_effort: String,
    service_tier: String,
    stream: bool,
    prompt_cache_key: String,
}

struct Attempt {
    request_id: String,
    started_at: Instant,
    first_output_at: Option<Instant>,
    status_code: Option<u16>,
    /// The upstream body ended, or broke, while the attempt was still open.
    upstream_ended: bool,
}

impl Attempt {
    fn start() -> Self {
        Self {
            request_id: uuid::Uuid::new_v4().simple().to_string(),
            started_at: Instant::now(),
            first_output_at: None,
            status_code: None,
            upstream_ended: false,
        }
    }

    /// Classifies an attempt that never received a terminal event.
    fn abandoned(&self) -> Outcome {
        match self.status_code {
            Some(status) if !(200..300).contains(&status) => Outcome::new(STATUS_FAILED, ""),
            _ if self.upstream_ended => Outcome::new(STATUS_INTERRUPTED, "stream_truncated"),
            _ => Outcome::new(STATUS_CANCELLED, "client_cancelled"),
        }
    }
}

/// Request-side usage attributes extracted from an upstream Responses body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageRequest {
    model: Option<String>,
    reasoning_effort: String,
    service_tier: String,
    stream: bool,
    prompt_cache_key: String,
}

impl UsageRequest {
    /// Reads the final body sent upstream, so egress policies are already applied.
    pub fn from_body(object: &Map<String, Value>) -> Self {
        let model = request_field(object, "model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .map(str::to_owned);
        let reasoning_effort = request_field(object, "reasoning")
            .and_then(Value::as_object)
            .and_then(|reasoning| string(reasoning, "effort"))
            .or_else(|| request_field(object, "reasoning_effort").and_then(Value::as_str))
            .map(normalize_setting)
            .filter(|effort| !effort.is_empty())
            .unwrap_or_else(|| DEFAULT_REQUEST_SETTING.into());
        let service_tier = request_field(object, "service_tier")
            .and_then(Value::as_str)
            .map(normalize_service_tier)
            .unwrap_or_else(|| AUTO_SERVICE_TIER.into());
        let stream = request_field(object, "stream")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let prompt_cache_key = request_field(object, "prompt_cache_key")
            .and_then(Value::as_str)
            .map(|value| value.trim().chars().take(MAX_ERROR_CODE_CHARS).collect())
            .unwrap_or_default();
        Self {
            model,
            reasoning_effort,
            service_tier,
            stream,
            prompt_cache_key,
        }
    }
}

impl RequestSettings {
    fn apply(&mut self, request: UsageRequest) {
        if let Some(model) = request.model {
            self.model = model;
        }
        self.reasoning_effort = request.reasoning_effort;
        self.service_tier = request.service_tier;
        self.stream = request.stream;
        self.prompt_cache_key = request.prompt_cache_key;
    }
}

impl UsageTracker {
    pub fn http(
        store: UsageStore,
        identity: UsageIdentity,
        client: UsageClient,
        endpoint: impl Into<String>,
    ) -> Self {
        Self::new(store, identity, client, endpoint, Transport::Http)
    }

    pub fn websocket(
        store: UsageStore,
        identity: UsageIdentity,
        client: UsageClient,
        endpoint: impl Into<String>,
    ) -> Self {
        Self::new(store, identity, client, endpoint, Transport::WebSocket)
    }

    fn new(
        store: UsageStore,
        identity: UsageIdentity,
        client: UsageClient,
        endpoint: impl Into<String>,
        transport: Transport,
    ) -> Self {
        Self {
            inner: Arc::new(UsageTrackerInner {
                store,
                identity,
                client,
                endpoint: endpoint.into(),
                transport,
                state: Mutex::new(TrackerState {
                    attempt: (transport == Transport::Http).then(Attempt::start),
                    ..TrackerState::default()
                }),
            }),
        }
    }

    fn state(&self) -> MutexGuard<'_, TrackerState> {
        self.inner
            .state
            .lock()
            .expect("usage tracker lock poisoned")
    }

    pub fn observe_request_value(&self, value: &Value) {
        let Some(object) = value.as_object() else {
            return;
        };
        self.observe_request_object(object);
    }

    pub fn observe_request_object(&self, object: &Map<String, Value>) {
        let kind = string(object, "type");
        if kind.is_some_and(|kind| !matches!(kind, "response.create" | "response.append")) {
            return;
        }
        let request = UsageRequest::from_body(object);
        let mut state = self.state();
        if self.inner.transport == Transport::WebSocket {
            if let Some(previous) = state.attempt.take() {
                let outcome = Outcome::new(STATUS_CANCELLED, "superseded");
                let event = self.inner.event(&state, previous, outcome);
                self.inner.store.record(event);
            }
            state.attempt = Some(Attempt::start());
        }
        if kind == Some("response.append") {
            // Appends extend the previous response.create and inherit its settings.
            if let Some(model) = request.model {
                state.request.model = model;
            }
            return;
        }
        state.request.apply(request);
    }

    pub fn record_request(&self, request: UsageRequest) {
        self.state().request.apply(request);
    }

    pub fn observe_request_text(&self, text: &str) {
        if let Ok(value) = serde_json::from_str::<Value>(text) {
            self.observe_request_value(&value);
        }
    }

    /// Records whether the client asked for a stream when the upstream body always streams.
    pub fn set_client_stream(&self, stream: bool) {
        self.state().client_stream = Some(stream);
    }

    pub fn observe_upstream_status(&self, status: u16) {
        self.state()
            .attempt
            .get_or_insert_with(Attempt::start)
            .status_code = Some(status);
    }

    /// Ends the attempt with an error raised before the upstream produced a response.
    pub fn fail(&self, error: &ApiError) {
        let mut state = self.state();
        let attempt = state.attempt.take().unwrap_or_else(Attempt::start);
        let outcome = Outcome {
            status: STATUS_FAILED,
            status_code: Some(error.status),
            error_code: clip_error_code(error.code.as_deref().unwrap_or(&error.kind)),
            ..Outcome::default()
        };
        let event = self.inner.event(&state, attempt, outcome);
        drop(state);
        self.inner.store.record(event);
    }

    /// Marks that the upstream body is finished; an attempt still open is then interrupted.
    pub fn upstream_finished(&self) {
        if let Some(attempt) = self.state().attempt.as_mut() {
            attempt.upstream_ended = true;
        }
    }

    /// Observes one streamed event (SSE or WebSocket message).
    pub fn observe_response_object(&self, object: &Map<String, Value>) {
        self.observe_response(object, false);
    }

    pub fn observe_response_text(&self, text: &str) {
        if let Ok(Value::Object(object)) = serde_json::from_str::<Value>(text) {
            self.observe_response_object(&object);
        }
    }

    /// Observes a complete non-streamed JSON body.
    fn observe_response_body(&self, value: &Value) {
        if let Some(object) = value.as_object() {
            self.observe_response(object, true);
        }
    }

    fn observe_response(&self, object: &Map<String, Value>, whole_body: bool) {
        let mut state = self.state();
        let Some(attempt) = state.attempt.as_mut() else {
            return;
        };
        if attempt.first_output_at.is_none()
            && string(object, "type").is_some_and(|kind| kind.ends_with(".delta"))
        {
            attempt.first_output_at = Some(Instant::now());
        }
        let Some(outcome) = parse_outcome(object, whole_body) else {
            return;
        };
        let Some(attempt) = state.attempt.take() else {
            return;
        };
        let event = self.inner.event(&state, attempt, outcome);
        drop(state);
        self.inner.store.record(event);
    }

    /// Passes a streamed request body through unchanged while reading its usage settings.
    pub fn observe_request_stream<S, E>(
        &self,
        body: S,
        content_encoding: Option<String>,
    ) -> impl Stream<Item = Result<Bytes, E>> + use<S, E>
    where
        S: Stream<Item = Result<Bytes, E>> + Unpin,
    {
        let observer = RequestBodyObserver {
            tracker: self.clone(),
            content_encoding,
            buffer: Vec::new(),
        };
        stream::unfold(
            (body, Some(observer)),
            |(mut body, mut observer)| async move {
                match body.next().await {
                    Some(Ok(chunk)) => {
                        if let Some(active) = observer.as_mut()
                            && !active.push(&chunk)
                        {
                            observer = None;
                        }
                        Some((Ok(chunk), (body, observer)))
                    }
                    Some(Err(error)) => Some((Err(error), (body, None))),
                    None => {
                        if let Some(active) = observer.take() {
                            active.finish();
                        }
                        None
                    }
                }
            },
        )
    }

    pub fn wire_observer(&self, content_type: Option<&str>) -> UsageWireObserver {
        UsageWireObserver::new(self.clone(), content_type)
    }
}

impl UsageTrackerInner {
    fn event(&self, state: &TrackerState, attempt: Attempt, outcome: Outcome) -> UsageEvent {
        let request = &state.request;
        let session = &self.client.session;
        // Codex reuses the conversation ID as its prompt cache key.
        let (session_id, session_kind) = if session.id.is_empty() {
            let kind = if request.prompt_cache_key.is_empty() {
                SessionKind::Unknown
            } else {
                SessionKind::Main
            };
            (request.prompt_cache_key.clone(), kind)
        } else {
            (session.id.clone(), session.kind)
        };
        let session_kind = if self.endpoint.trim_end_matches('/').ends_with("/compact") {
            SessionKind::Compaction
        } else {
            session_kind
        };
        let model = if outcome.response_model.is_empty() {
            request.model.clone()
        } else {
            outcome.response_model
        };
        UsageEvent {
            recorded_at_ms: current_time_ms(),
            request_id: attempt.request_id,
            user_id: self.identity.user_id.clone(),
            user_name: self.identity.user_name.clone(),
            credential_type: self.identity.credential_type.into(),
            credential_id: self.identity.credential_id.clone(),
            credential_name: self.identity.credential_name.clone(),
            codex_account_id: self.identity.codex_account_id.clone(),
            codex_account_name: self.identity.codex_account_name.clone(),
            account_group_id: self.identity.account_group_id.clone(),
            account_group_name: self.identity.account_group_name.clone(),
            requested_model: request.model.clone(),
            model,
            transport: self.transport.as_str().into(),
            endpoint: self.endpoint.clone(),
            stream: self.transport == Transport::WebSocket
                || state.client_stream.unwrap_or(request.stream),
            status: outcome.status.into(),
            status_code: outcome.status_code.or(attempt.status_code),
            error_code: outcome.error_code,
            response_id: outcome.response_id,
            reasoning_effort: request.reasoning_effort.clone(),
            service_tier: request.service_tier.clone(),
            response_service_tier: outcome.response_service_tier,
            session_id,
            parent_session_id: session.parent_id.clone(),
            session_kind: session_kind.as_str().into(),
            client_ip: self.client.ip.clone(),
            x_forwarded_for: self.client.forwarded_for.clone(),
            user_agent: self.client.user_agent.clone(),
            latency_ms: elapsed_ms(attempt.started_at, Instant::now()),
            ttft_ms: attempt
                .first_output_at
                .map(|at| elapsed_ms(attempt.started_at, at)),
            tokens: outcome.tokens,
        }
    }
}

impl Drop for UsageTrackerInner {
    fn drop(&mut self) {
        let mut state =
            std::mem::take(self.state.get_mut().unwrap_or_else(PoisonError::into_inner));
        if let Some(attempt) = state.attempt.take() {
            let outcome = attempt.abandoned();
            let event = self.event(&state, attempt, outcome);
            self.store.record(event);
        }
    }
}

fn elapsed_ms(from: Instant, to: Instant) -> i64 {
    i64::try_from(to.saturating_duration_since(from).as_millis()).unwrap_or(i64::MAX)
}

struct RequestBodyObserver {
    tracker: UsageTracker,
    content_encoding: Option<String>,
    buffer: Vec<u8>,
}

impl RequestBodyObserver {
    /// Returns false once the body is too large to track.
    fn push(&mut self, bytes: &[u8]) -> bool {
        if bytes.len() > MAX_TRACKED_JSON_BYTES.saturating_sub(self.buffer.len()) {
            return false;
        }
        self.buffer.extend_from_slice(bytes);
        true
    }

    fn finish(self) {
        if let Ok(body) = parse_json_body(&self.buffer, self.content_encoding.as_deref()) {
            self.tracker.observe_request_object(&body);
        }
    }
}

/// Watches a proxied upstream body. Dropping it before [`Self::finish`] means the
/// client disconnected mid-response.
pub struct UsageWireObserver {
    tracker: UsageTracker,
    format: WireFormat,
}

enum WireFormat {
    Sse(Option<openai::sse::SseDecoder>),
    Json(Option<Vec<u8>>),
}

impl UsageWireObserver {
    fn new(tracker: UsageTracker, content_type: Option<&str>) -> Self {
        let is_sse = content_type.is_some_and(|value| {
            value.split(';').next().is_some_and(|media_type| {
                media_type.trim().eq_ignore_ascii_case("text/event-stream")
            })
        });
        Self {
            tracker,
            format: if is_sse {
                WireFormat::Sse(Some(openai::sse::SseDecoder::new()))
            } else {
                WireFormat::Json(Some(Vec::new()))
            },
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        match &mut self.format {
            WireFormat::Sse(decoder) => {
                let Some(active) = decoder.as_mut() else {
                    return;
                };
                match active.push_bytes(bytes) {
                    Ok(events) => {
                        for event in events {
                            self.tracker.observe_response_object(&event);
                        }
                    }
                    Err(_) => *decoder = None,
                }
            }
            WireFormat::Json(buffer) => {
                let Some(active) = buffer.as_mut() else {
                    return;
                };
                if bytes.len() > MAX_TRACKED_JSON_BYTES.saturating_sub(active.len()) {
                    *buffer = None;
                } else {
                    active.extend_from_slice(bytes);
                }
            }
        }
    }

    /// Called once the upstream body has ended, cleanly or not.
    pub fn finish(mut self) {
        match &mut self.format {
            WireFormat::Sse(decoder) => {
                if let Some(active) = decoder.as_mut()
                    && let Ok(events) = active.finish()
                {
                    for event in events {
                        self.tracker.observe_response_object(&event);
                    }
                }
            }
            WireFormat::Json(buffer) => {
                if let Some(active) = buffer.take()
                    && let Ok(value) = serde_json::from_slice::<Value>(&active)
                {
                    self.tracker.observe_response_body(&value);
                }
            }
        }
        self.tracker.upstream_finished();
    }
}

/// Looks up a request field at the top level or inside a websocket `response` envelope.
fn request_field<'a>(object: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    object.get(key).or_else(|| {
        object
            .get("response")
            .and_then(Value::as_object)
            .and_then(|response| response.get(key))
    })
}

fn normalize_setting(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

/// Codex sends Fast mode as `priority`; `fast` is accepted as its alias.
fn normalize_service_tier(value: &str) -> String {
    match normalize_setting(value).as_str() {
        "" => AUTO_SERVICE_TIER.into(),
        "fast" => PRIORITY_SERVICE_TIER.into(),
        tier => tier.into(),
    }
}

/// Reads a terminal event, an error event, or (for `whole_body`) a complete JSON
/// response. Returns `None` for events that do not end the attempt.
fn parse_outcome(root: &Map<String, Value>, whole_body: bool) -> Option<Outcome> {
    let has_error = |object: &Map<String, Value>| {
        object.get("error").is_some_and(|error| !error.is_null())
            || object.get("detail").is_some_and(|detail| !detail.is_null())
    };
    let status = match string(root, "type") {
        Some("response.completed") => STATUS_COMPLETED,
        Some("response.incomplete") => STATUS_INCOMPLETE,
        Some("response.failed" | "error") => STATUS_FAILED,
        Some(_) => return None,
        None if has_error(root) => STATUS_FAILED,
        None if whole_body || root.get("usage").is_some_and(Value::is_object) => STATUS_COMPLETED,
        None => return None,
    };
    let response = root
        .get("response")
        .and_then(Value::as_object)
        .unwrap_or(root);
    let usage = response
        .get("usage")
        .and_then(Value::as_object)
        .or_else(|| root.get("usage").and_then(Value::as_object));
    Some(Outcome {
        status,
        status_code: root
            .get("status")
            .and_then(Value::as_u64)
            .and_then(|status| u16::try_from(status).ok())
            .filter(|status| (100..600).contains(status)),
        error_code: error_code(root, response),
        response_id: string(response, "id").unwrap_or_default().trim().into(),
        response_model: string(response, "model")
            .or_else(|| string(root, "model"))
            .unwrap_or_default()
            .trim()
            .into(),
        response_service_tier: string(response, "service_tier")
            .or_else(|| string(root, "service_tier"))
            .filter(|tier| !tier.trim().is_empty())
            .map(normalize_service_tier)
            .unwrap_or_default(),
        tokens: usage.map(parse_tokens).unwrap_or_default(),
    })
}

fn parse_tokens(usage: &Map<String, Value>) -> TokenUsage {
    let input_tokens = token(usage, "input_tokens").unwrap_or(0);
    let output_tokens = token(usage, "output_tokens").unwrap_or(0);
    let input_details = usage.get("input_tokens_details").and_then(Value::as_object);
    let output_details = usage
        .get("output_tokens_details")
        .and_then(Value::as_object);
    TokenUsage {
        input_tokens,
        cached_input_tokens: input_details
            .and_then(|details| token(details, "cached_tokens"))
            .unwrap_or(0),
        cache_creation_input_tokens: input_details
            .and_then(|details| token(details, "cache_creation_tokens"))
            .unwrap_or(0),
        output_tokens,
        reasoning_output_tokens: output_details
            .and_then(|details| token(details, "reasoning_tokens"))
            .unwrap_or(0),
        total_tokens: token(usage, "total_tokens").unwrap_or(input_tokens + output_tokens),
    }
}

/// First machine-readable reason found in Codex error and incomplete payloads.
fn error_code(root: &Map<String, Value>, response: &Map<String, Value>) -> String {
    [
        nested(response, "error", "code"),
        nested(response, "error", "type"),
        nested(response, "incomplete_details", "reason"),
        string(root, "code"),
        nested(root, "error", "code"),
        nested(root, "error", "type"),
        nested(root, "detail", "code"),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .find(|code| !code.is_empty())
    .map(clip_error_code)
    .unwrap_or_default()
}

fn nested<'a>(object: &'a Map<String, Value>, parent: &str, key: &str) -> Option<&'a str> {
    object
        .get(parent)
        .and_then(Value::as_object)
        .and_then(|child| string(child, key))
}

fn clip_error_code(value: &str) -> String {
    value.chars().take(MAX_ERROR_CODE_CHARS).collect()
}

fn string<'a>(object: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    object.get(key).and_then(Value::as_str)
}

fn token(object: &Map<String, Value>, key: &str) -> Option<i64> {
    object.get(key)?.as_i64().filter(|value| *value >= 0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageRange {
    Cycle,
    Day,
    Week,
    Month,
    All,
}

impl UsageRange {
    pub fn parse(value: Option<&str>) -> Option<Self> {
        match value {
            Some("cycle") => Some(Self::Cycle),
            Some("24h") => Some(Self::Day),
            None | Some("7d") => Some(Self::Week),
            Some("30d") => Some(Self::Month),
            Some("all") => Some(Self::All),
            Some(_) => None,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Cycle => "cycle",
            Self::Day => "24h",
            Self::Week => "7d",
            Self::Month => "30d",
            Self::All => "all",
        }
    }

    const fn duration_ms(self) -> Option<i64> {
        const DAY: i64 = 86_400_000;
        match self {
            Self::Cycle => None,
            Self::Day => Some(DAY),
            Self::Week => Some(7 * DAY),
            Self::Month => Some(30 * DAY),
            Self::All => None,
        }
    }

    const fn bucket_ms(self) -> i64 {
        if matches!(self, Self::Day | Self::Cycle) {
            3_600_000
        } else {
            86_400_000
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsageBounds {
    pub start_at: i64,
    pub end_at: i64,
}

impl UsageBounds {
    pub fn cycle(start_at: i64, end_at: i64, now: i64) -> Option<Self> {
        const DAY: i64 = 86_400_000;
        let duration = end_at.checked_sub(start_at)?;
        if duration != 7 * DAY || start_at < 0 || end_at > now.saturating_add(7 * DAY) {
            return None;
        }
        Some(Self { start_at, end_at })
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageDashboard {
    pub range: String,
    pub start_at: i64,
    pub end_at: i64,
    pub totals: UsageTotals,
    pub series: Vec<UsageSeriesPoint>,
    pub models: Vec<UsageBreakdownRow>,
    pub identities: Vec<UsageIdentityRow>,
    pub reasoning_efforts: Vec<UsageDimensionRow>,
    pub service_tiers: Vec<UsageDimensionRow>,
    pub recent_events: Vec<UsageEventRow>,
    pub unpriced_models: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageTotals {
    pub requests: i64,
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_output_tokens: i64,
    pub total_tokens: i64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageSeriesPoint {
    pub start_at: i64,
    pub successful_requests: i64,
    pub failed_requests: i64,
    pub requests: i64,
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_output_tokens: i64,
    pub total_tokens: i64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageBreakdownRow {
    pub model: String,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

/// Usage grouped by a request setting; empty when the attempt failed before its request was read.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageDimensionRow {
    pub value: String,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageIdentityRow {
    pub user_id: String,
    pub user_name: String,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageEventRow {
    pub id: i64,
    pub recorded_at: i64,
    pub request_id: String,
    pub response_id: String,
    pub user_id: String,
    pub user_name: String,
    pub credential_type: String,
    pub credential_id: String,
    pub credential_name: String,
    pub codex_account_id: String,
    pub codex_account_name: String,
    pub account_group_id: String,
    pub account_group_name: String,
    pub requested_model: String,
    pub model: String,
    pub transport: String,
    pub endpoint: String,
    pub stream: bool,
    pub status: String,
    pub status_code: Option<u16>,
    pub error_code: String,
    pub reasoning_effort: String,
    pub service_tier: String,
    pub response_service_tier: String,
    pub session_id: String,
    pub parent_session_id: String,
    pub session_kind: String,
    pub client_ip: String,
    pub x_forwarded_for: String,
    pub user_agent: String,
    pub latency_ms: i64,
    pub ttft_ms: Option<i64>,
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_output_tokens: i64,
    pub total_tokens: i64,
    pub cost_usd: f64,
}

/// Current display names keyed by stable IDs. Deleted identities keep their event names.
#[derive(Debug, Clone, Default)]
pub struct UsageNames {
    users: HashMap<String, String>,
    credentials: HashMap<String, String>,
    accounts: HashMap<String, String>,
    groups: HashMap<String, String>,
}

impl UsageNames {
    pub fn new(users: &[User], routing: &AccountRoutingState) -> Self {
        Self {
            users: users
                .iter()
                .map(|user| (user.id.clone(), user.name.clone()))
                .collect(),
            credentials: users
                .iter()
                .flat_map(|user| &user.credentials)
                .map(|credential| (credential.id.clone(), credential.display_name()))
                .collect(),
            accounts: routing
                .accounts
                .iter()
                .map(|account| (account.id.clone(), account.name.clone()))
                .collect(),
            groups: routing
                .groups
                .iter()
                .map(|group| (group.id.clone(), group.name.clone()))
                .collect(),
        }
    }

    fn apply_user(&self, id: &str, name: &mut String) {
        if let Some(current) = self.users.get(id) {
            name.clone_from(current);
        }
    }

    pub(super) fn apply_event(&self, event: &mut UsageEventRow) {
        self.apply_user(&event.user_id, &mut event.user_name);
        if let Some(name) = self.credentials.get(&event.credential_id) {
            event.credential_name.clone_from(name);
        }
        if let Some(name) = self.accounts.get(&event.codex_account_id) {
            event.codex_account_name.clone_from(name);
        }
        if let Some(name) = self.groups.get(&event.account_group_id) {
            event.account_group_name.clone_from(name);
        }
    }
}

fn query_dashboard(
    connection: &Connection,
    range: UsageRange,
    filters: &UsageFilters,
    bounds: Option<UsageBounds>,
    costs: &CostIndex,
) -> Result<UsageDashboard> {
    let now = current_time_ms();
    let (downstream_type, downstream_id, upstream_type, upstream_id) = filter_parameters(filters);
    let earliest_sql = format!(
        "SELECT MIN(recorded_at_ms) FROM usage_events WHERE {}",
        usage_filters_sql(1, 2, 3, 4)
    );
    let earliest = connection.query_row(
        &earliest_sql,
        params![downstream_type, downstream_id, upstream_type, upstream_id],
        |row| row.get::<_, Option<i64>>(0),
    )?;
    let (start_at, end_at) = if range == UsageRange::Cycle {
        bounds
            .map(|bounds| (bounds.start_at, bounds.end_at))
            .unwrap_or((now.saturating_sub(7 * 86_400_000), now))
    } else {
        let start_at = range
            .duration_ms()
            .map(|duration| now.saturating_sub(duration))
            .or(earliest)
            .unwrap_or(now);
        (start_at, now)
    };
    let mut totals = query_totals(connection, start_at, end_at, filters)?;
    let model_costs = query_model_costs(connection, start_at, end_at, filters, costs)?;
    totals.cost_usd = model_costs.values().sum();
    let series = query_series(connection, range, start_at, end_at, filters, costs)?;
    let mut models = query_models(connection, start_at, end_at, filters)?;
    for row in &mut models {
        row.totals.cost_usd = model_costs.get(&row.model).copied().unwrap_or_default();
    }
    let mut identities = query_identities(connection, start_at, end_at, filters)?;
    apply_identity_costs(
        connection,
        start_at,
        end_at,
        filters,
        costs,
        &mut identities,
    )?;
    let reasoning_efforts = query_dimension(
        connection,
        UsageDimension::ReasoningEffort,
        start_at,
        end_at,
        filters,
        costs,
    )?;
    let service_tiers = query_dimension(
        connection,
        UsageDimension::ServiceTier,
        start_at,
        end_at,
        filters,
        costs,
    )?;
    let mut recent_events = query_recent_events(connection, start_at, end_at, filters)?;
    for event in &mut recent_events {
        event.cost_usd = event_cost(event, costs);
    }
    let unpriced_models = query_unpriced_models(connection, start_at, end_at, filters, costs)?;
    Ok(UsageDashboard {
        range: range.label().into(),
        start_at,
        end_at,
        totals,
        series,
        models,
        identities,
        reasoning_efforts,
        service_tiers,
        recent_events,
        unpriced_models,
    })
}

pub(super) fn filter_parameters(
    filters: &UsageFilters,
) -> (Option<&str>, Option<&str>, Option<&str>, Option<&str>) {
    let (downstream_type, downstream_id) = filters
        .downstream
        .as_ref()
        .map(|value| (Some(value.kind()), Some(value.id())))
        .unwrap_or((None, None));
    let (upstream_type, upstream_id) = filters
        .upstream
        .as_ref()
        .map(|value| (Some(value.kind()), Some(value.id())))
        .unwrap_or((None, None));
    (downstream_type, downstream_id, upstream_type, upstream_id)
}

fn identity_filter_sql(kind_parameter: usize, id_parameter: usize) -> String {
    format!(
        "(?{kind_parameter} IS NULL \
         OR (?{kind_parameter} = 'user' AND user_id = ?{id_parameter}) \
         OR (?{kind_parameter} = 'credential' AND credential_id = ?{id_parameter}) \
         OR (?{kind_parameter} = 'codex_account' AND codex_account_id = ?{id_parameter}) \
         OR (?{kind_parameter} = 'account_group' AND account_group_id = ?{id_parameter}))"
    )
}

pub(super) fn usage_filters_sql(
    downstream_kind_parameter: usize,
    downstream_id_parameter: usize,
    upstream_kind_parameter: usize,
    upstream_id_parameter: usize,
) -> String {
    format!(
        "{} AND {}",
        identity_filter_sql(downstream_kind_parameter, downstream_id_parameter),
        identity_filter_sql(upstream_kind_parameter, upstream_id_parameter),
    )
}

const TOTAL_COLUMNS: &str = r#"
    COUNT(*),
    COALESCE(SUM(input_tokens), 0),
    COALESCE(SUM(cached_input_tokens), 0),
    COALESCE(SUM(cache_creation_input_tokens), 0),
    COALESCE(SUM(output_tokens), 0),
    COALESCE(SUM(reasoning_output_tokens), 0),
    COALESCE(SUM(total_tokens), 0)
"#;

fn query_totals(
    connection: &Connection,
    start_at: i64,
    end_at: i64,
    filters: &UsageFilters,
) -> Result<UsageTotals> {
    let usage_filters = usage_filters_sql(3, 4, 5, 6);
    let sql = format!(
        "SELECT {TOTAL_COLUMNS} FROM usage_events \
         WHERE recorded_at_ms >= ?1 AND recorded_at_ms < ?2 \
         AND {usage_filters}"
    );
    let (downstream_type, downstream_id, upstream_type, upstream_id) = filter_parameters(filters);
    Ok(connection.query_row(
        &sql,
        params![
            start_at,
            end_at,
            downstream_type,
            downstream_id,
            upstream_type,
            upstream_id
        ],
        totals_from_row,
    )?)
}

fn query_models(
    connection: &Connection,
    start_at: i64,
    end_at: i64,
    filters: &UsageFilters,
) -> Result<Vec<UsageBreakdownRow>> {
    let usage_filters = usage_filters_sql(3, 4, 5, 6);
    let sql = format!(
        "SELECT model, {TOTAL_COLUMNS} FROM usage_events \
         WHERE recorded_at_ms >= ?1 AND recorded_at_ms < ?2 \
         AND model <> '' AND {usage_filters} \
         GROUP BY model ORDER BY SUM(total_tokens) DESC, COUNT(*) DESC, model LIMIT ?7"
    );
    let mut statement = connection.prepare(&sql)?;
    let (downstream_type, downstream_id, upstream_type, upstream_id) = filter_parameters(filters);
    let rows = statement.query_map(
        params![
            start_at,
            end_at,
            downstream_type,
            downstream_id,
            upstream_type,
            upstream_id,
            BREAKDOWN_LIMIT
        ],
        |row| {
            Ok(UsageBreakdownRow {
                model: row.get(0)?,
                totals: totals_from_row_at(row, 1)?,
            })
        },
    )?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn query_identities(
    connection: &Connection,
    start_at: i64,
    end_at: i64,
    filters: &UsageFilters,
) -> Result<Vec<UsageIdentityRow>> {
    let usage_filters = usage_filters_sql(3, 4, 5, 6);
    let sql = format!(
        "SELECT user_id, MAX(user_name), {TOTAL_COLUMNS} FROM usage_events \
         WHERE recorded_at_ms >= ?1 AND recorded_at_ms < ?2 \
         AND {usage_filters} \
         GROUP BY user_id \
         ORDER BY SUM(total_tokens) DESC, COUNT(*) DESC, 2 LIMIT ?7"
    );
    let mut statement = connection.prepare(&sql)?;
    let (downstream_type, downstream_id, upstream_type, upstream_id) = filter_parameters(filters);
    let rows = statement.query_map(
        params![
            start_at,
            end_at,
            downstream_type,
            downstream_id,
            upstream_type,
            upstream_id,
            BREAKDOWN_LIMIT
        ],
        |row| {
            Ok(UsageIdentityRow {
                user_id: row.get(0)?,
                user_name: row.get(1)?,
                totals: totals_from_row_at(row, 2)?,
            })
        },
    )?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[derive(Debug, Clone, Copy)]
enum UsageDimension {
    ReasoningEffort,
    ServiceTier,
}

impl UsageDimension {
    const fn column(self) -> &'static str {
        match self {
            Self::ReasoningEffort => "reasoning_effort",
            Self::ServiceTier => "service_tier",
        }
    }
}

fn query_dimension(
    connection: &Connection,
    dimension: UsageDimension,
    start_at: i64,
    end_at: i64,
    filters: &UsageFilters,
    costs: &CostIndex,
) -> Result<Vec<UsageDimensionRow>> {
    let column = dimension.column();
    let usage_filters = usage_filters_sql(3, 4, 5, 6);
    let sql = format!(
        "SELECT {column}, model, {FAST_SQL}, {TOTAL_COLUMNS} FROM usage_events \
         WHERE recorded_at_ms >= ?1 AND recorded_at_ms < ?2 \
         AND {usage_filters} \
         GROUP BY {column}, model, 3"
    );
    let mut statement = connection.prepare(&sql)?;
    let (downstream_type, downstream_id, upstream_type, upstream_id) = filter_parameters(filters);
    let rows = statement.query_map(
        params![
            start_at,
            end_at,
            downstream_type,
            downstream_id,
            upstream_type,
            upstream_id
        ],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, bool>(2)?,
                totals_from_row_at(row, 3)?,
            ))
        },
    )?;
    let mut grouped = BTreeMap::<String, UsageTotals>::new();
    for row in rows {
        let (value, model, fast, totals) = row?;
        let target = grouped.entry(value).or_default();
        add_totals(target, &totals);
        target.cost_usd += costs.cost(&model, fast, &totals);
    }
    let mut rows = grouped
        .into_iter()
        .map(|(value, totals)| UsageDimensionRow { value, totals })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        right
            .totals
            .requests
            .cmp(&left.totals.requests)
            .then_with(|| left.value.cmp(&right.value))
    });
    Ok(rows)
}

fn query_series(
    connection: &Connection,
    range: UsageRange,
    start_at: i64,
    end_at: i64,
    filters: &UsageFilters,
    costs: &CostIndex,
) -> Result<Vec<UsageSeriesPoint>> {
    let bucket_ms = range.bucket_ms();
    let fixed_activity_grid = range != UsageRange::All;
    let aligned_start = if fixed_activity_grid {
        start_at
    } else {
        start_at - start_at.rem_euclid(bucket_ms)
    };
    let range_duration = end_at.saturating_sub(start_at).max(1);
    let bucket_scale = if fixed_activity_grid {
        ACTIVITY_BUCKET_COUNT
    } else {
        1
    };
    let bucket_divisor = if fixed_activity_grid {
        range_duration
    } else {
        bucket_ms
    };
    let usage_filters = usage_filters_sql(6, 7, 8, 9);
    let sql = format!(
        r#"
        SELECT (((recorded_at_ms - ?1) * ?2) / ?3), model, {FAST_SQL}, {TOTAL_COLUMNS},
               COALESCE(SUM({SUCCESS_SQL}), 0),
               COALESCE(SUM({FAILURE_SQL}), 0)
        FROM usage_events
        WHERE recorded_at_ms >= ?4 AND recorded_at_ms < ?5
          AND {usage_filters}
        GROUP BY 1, model, 3 ORDER BY 1
        "#,
    );
    let mut statement = connection.prepare(&sql)?;
    let (downstream_type, downstream_id, upstream_type, upstream_id) = filter_parameters(filters);
    let rows = statement.query_map(
        params![
            aligned_start,
            bucket_scale,
            bucket_divisor,
            start_at,
            end_at,
            downstream_type,
            downstream_id,
            upstream_type,
            upstream_id
        ],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, bool>(2)?,
                totals_from_row_at(row, 3)?,
                row.get::<_, i64>(10)?,
                row.get::<_, i64>(11)?,
            ))
        },
    )?;
    let mut populated = BTreeMap::<i64, SeriesAccumulator>::new();
    for row in rows {
        let (index, model, fast, totals, successful_requests, failed_requests) = row?;
        let cost = costs.cost(&model, fast, &totals);
        let point = populated.entry(index).or_default();
        add_totals(&mut point.totals, &totals);
        point.totals.cost_usd += cost;
        point.successful_requests += successful_requests;
        point.failed_requests += failed_requests;
    }
    let (first_index, last_index) = if fixed_activity_grid {
        (0, ACTIVITY_BUCKET_COUNT - 1)
    } else {
        let last = (end_at.saturating_sub(1) - aligned_start).div_euclid(bucket_ms);
        let first = (start_at - aligned_start).div_euclid(bucket_ms);
        (populated.keys().next().copied().unwrap_or(first), last)
    };
    Ok((first_index..=last_index)
        .map(|index| {
            let point = populated.remove(&index).unwrap_or_default();
            let point_start = if fixed_activity_grid {
                start_at.saturating_add(
                    index
                        .saturating_mul(range_duration)
                        .div_euclid(ACTIVITY_BUCKET_COUNT),
                )
            } else {
                aligned_start.saturating_add(index.saturating_mul(bucket_ms))
            };
            UsageSeriesPoint {
                start_at: point_start,
                successful_requests: point.successful_requests,
                failed_requests: point.failed_requests,
                requests: point.totals.requests,
                input_tokens: point.totals.input_tokens,
                cached_input_tokens: point.totals.cached_input_tokens,
                cache_creation_input_tokens: point.totals.cache_creation_input_tokens,
                output_tokens: point.totals.output_tokens,
                reasoning_output_tokens: point.totals.reasoning_output_tokens,
                total_tokens: point.totals.total_tokens,
                cost_usd: point.totals.cost_usd,
            }
        })
        .collect())
}

#[derive(Default)]
struct SeriesAccumulator {
    successful_requests: i64,
    failed_requests: i64,
    totals: UsageTotals,
}

fn add_totals(target: &mut UsageTotals, source: &UsageTotals) {
    target.requests += source.requests;
    target.input_tokens += source.input_tokens;
    target.cached_input_tokens += source.cached_input_tokens;
    target.cache_creation_input_tokens += source.cache_creation_input_tokens;
    target.output_tokens += source.output_tokens;
    target.reasoning_output_tokens += source.reasoning_output_tokens;
    target.total_tokens += source.total_tokens;
}

/// Model prices plus the surcharge applied to Fast (priority tier) requests.
pub(super) struct CostIndex {
    prices: BTreeMap<String, ModelPrice>,
    fast_multiplier: f64,
}

impl CostIndex {
    pub(super) fn new(prices: &[ModelPrice], fast_multiplier: f64) -> Self {
        Self {
            prices: price_index(prices),
            fast_multiplier,
        }
    }

    pub(super) fn is_priced(&self, model: &str) -> bool {
        self.prices.contains_key(&model.trim().to_ascii_lowercase())
    }

    fn cost(&self, model: &str, fast: bool, totals: &UsageTotals) -> f64 {
        self.token_cost(
            model,
            fast,
            totals.input_tokens,
            totals.output_tokens,
            totals.cached_input_tokens,
            totals.cache_creation_input_tokens,
        )
    }

    pub(super) fn token_cost(
        &self,
        model: &str,
        fast: bool,
        input_tokens: i64,
        output_tokens: i64,
        cached_input_tokens: i64,
        cache_creation_input_tokens: i64,
    ) -> f64 {
        let Some(price) = self.prices.get(&model.trim().to_ascii_lowercase()) else {
            return 0.0;
        };
        let cost = calculate_cost(
            input_tokens,
            output_tokens,
            cached_input_tokens,
            cache_creation_input_tokens,
            price,
        );
        if fast {
            cost * self.fast_multiplier
        } else {
            cost
        }
    }
}

fn query_model_costs(
    connection: &Connection,
    start_at: i64,
    end_at: i64,
    filters: &UsageFilters,
    costs: &CostIndex,
) -> Result<BTreeMap<String, f64>> {
    let usage_filters = usage_filters_sql(3, 4, 5, 6);
    let sql = format!(
        "SELECT model, {FAST_SQL}, {TOTAL_COLUMNS} FROM usage_events \
         WHERE recorded_at_ms >= ?1 AND recorded_at_ms < ?2 \
         AND {usage_filters} \
         GROUP BY model, 2"
    );
    let mut statement = connection.prepare(&sql)?;
    let (downstream_type, downstream_id, upstream_type, upstream_id) = filter_parameters(filters);
    let rows = statement.query_map(
        params![
            start_at,
            end_at,
            downstream_type,
            downstream_id,
            upstream_type,
            upstream_id
        ],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, bool>(1)?,
                totals_from_row_at(row, 2)?,
            ))
        },
    )?;
    let mut model_costs = BTreeMap::new();
    for row in rows {
        let (model, fast, totals) = row?;
        let cost = costs.cost(&model, fast, &totals);
        *model_costs.entry(model).or_default() += cost;
    }
    Ok(model_costs)
}

fn apply_identity_costs(
    connection: &Connection,
    start_at: i64,
    end_at: i64,
    filters: &UsageFilters,
    costs: &CostIndex,
    identities: &mut [UsageIdentityRow],
) -> Result<()> {
    let usage_filters = usage_filters_sql(3, 4, 5, 6);
    let sql = format!(
        "SELECT user_id, model, {FAST_SQL}, {TOTAL_COLUMNS} FROM usage_events \
         WHERE recorded_at_ms >= ?1 AND recorded_at_ms < ?2 \
         AND {usage_filters} \
         GROUP BY user_id, model, 3"
    );
    let mut statement = connection.prepare(&sql)?;
    let (downstream_type, downstream_id, upstream_type, upstream_id) = filter_parameters(filters);
    let rows = statement.query_map(
        params![
            start_at,
            end_at,
            downstream_type,
            downstream_id,
            upstream_type,
            upstream_id
        ],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, bool>(2)?,
                totals_from_row_at(row, 3)?,
            ))
        },
    )?;
    let mut identity_costs = BTreeMap::<String, f64>::new();
    for row in rows {
        let (id, model, fast, totals) = row?;
        *identity_costs.entry(id).or_default() += costs.cost(&model, fast, &totals);
    }
    for row in identities {
        row.totals.cost_usd = identity_costs
            .get(&row.user_id)
            .copied()
            .unwrap_or_default();
    }
    Ok(())
}

fn query_unpriced_models(
    connection: &Connection,
    start_at: i64,
    end_at: i64,
    filters: &UsageFilters,
    costs: &CostIndex,
) -> Result<Vec<String>> {
    let sql = format!(
        "SELECT DISTINCT model FROM usage_events \
         WHERE recorded_at_ms >= ?1 AND recorded_at_ms < ?2 AND total_tokens > 0 \
         AND {} ORDER BY model",
        usage_filters_sql(3, 4, 5, 6)
    );
    let mut statement = connection.prepare(&sql)?;
    let (downstream_type, downstream_id, upstream_type, upstream_id) = filter_parameters(filters);
    let rows = statement.query_map(
        params![
            start_at,
            end_at,
            downstream_type,
            downstream_id,
            upstream_type,
            upstream_id
        ],
        |row| row.get::<_, String>(0),
    )?;
    Ok(rows
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter(|model| !costs.is_priced(model))
        .collect())
}

pub(super) fn event_cost(event: &UsageEventRow, costs: &CostIndex) -> f64 {
    costs.token_cost(
        &event.model,
        event.service_tier == PRIORITY_SERVICE_TIER,
        event.input_tokens,
        event.output_tokens,
        event.cached_input_tokens,
        event.cache_creation_input_tokens,
    )
}

/// Columns read by [`event_from_row`], in order.
pub(super) const EVENT_COLUMNS: &str = r#"
    id, recorded_at_ms, request_id, response_id, credential_type, user_id, user_name,
    codex_account_id, codex_account_name, account_group_id, account_group_name,
    requested_model, model, transport, endpoint, stream, status, status_code,
    error_code, reasoning_effort, service_tier, response_service_tier,
    session_id, parent_session_id, session_kind, client_ip, x_forwarded_for,
    user_agent, latency_ms, ttft_ms, input_tokens, cached_input_tokens,
    cache_creation_input_tokens, output_tokens, reasoning_output_tokens, total_tokens,
    credential_id, credential_name
"#;

pub(super) fn event_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<UsageEventRow> {
    Ok(UsageEventRow {
        id: row.get(0)?,
        recorded_at: row.get(1)?,
        request_id: row.get(2)?,
        response_id: row.get(3)?,
        credential_type: row.get(4)?,
        user_id: row.get(5)?,
        user_name: row.get(6)?,
        codex_account_id: row.get(7)?,
        codex_account_name: row.get(8)?,
        account_group_id: row.get(9)?,
        account_group_name: row.get(10)?,
        requested_model: row.get(11)?,
        model: row.get(12)?,
        transport: row.get(13)?,
        endpoint: row.get(14)?,
        stream: row.get(15)?,
        status: row.get(16)?,
        status_code: row.get(17)?,
        error_code: row.get(18)?,
        reasoning_effort: row.get(19)?,
        service_tier: row.get(20)?,
        response_service_tier: row.get(21)?,
        session_id: row.get(22)?,
        parent_session_id: row.get(23)?,
        session_kind: row.get(24)?,
        client_ip: row.get(25)?,
        x_forwarded_for: row.get(26)?,
        user_agent: row.get(27)?,
        latency_ms: row.get(28)?,
        ttft_ms: row.get(29)?,
        input_tokens: row.get(30)?,
        cached_input_tokens: row.get(31)?,
        cache_creation_input_tokens: row.get(32)?,
        output_tokens: row.get(33)?,
        reasoning_output_tokens: row.get(34)?,
        total_tokens: row.get(35)?,
        credential_id: row.get(36)?,
        credential_name: row.get(37)?,
        cost_usd: 0.0,
    })
}

fn query_recent_events(
    connection: &Connection,
    start_at: i64,
    end_at: i64,
    filters: &UsageFilters,
) -> Result<Vec<UsageEventRow>> {
    let sql = format!(
        "SELECT {EVENT_COLUMNS} FROM usage_events \
         WHERE recorded_at_ms >= ?1 AND recorded_at_ms < ?2 AND {} \
         ORDER BY recorded_at_ms DESC, id DESC LIMIT ?7",
        usage_filters_sql(3, 4, 5, 6)
    );
    let mut statement = connection.prepare(&sql)?;
    let (downstream_type, downstream_id, upstream_type, upstream_id) = filter_parameters(filters);
    let rows = statement.query_map(
        params![
            start_at,
            end_at,
            downstream_type,
            downstream_id,
            upstream_type,
            upstream_id,
            RECENT_EVENT_LIMIT
        ],
        event_from_row,
    )?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn totals_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<UsageTotals> {
    totals_from_row_at(row, 0)
}

fn totals_from_row_at(row: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<UsageTotals> {
    Ok(UsageTotals {
        requests: row.get(offset)?,
        input_tokens: row.get(offset + 1)?,
        cached_input_tokens: row.get(offset + 2)?,
        cache_creation_input_tokens: row.get(offset + 3)?,
        output_tokens: row.get(offset + 4)?,
        reasoning_output_tokens: row.get(offset + 5)?,
        total_tokens: row.get(offset + 6)?,
        cost_usd: 0.0,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn identity() -> UsageIdentity {
        UsageIdentity {
            user_id: "user-1".into(),
            user_name: "alice".into(),
            credential_type: "api_key",
            credential_id: "key-1".into(),
            credential_name: "laptop".into(),
            codex_account_id: "codex-1".into(),
            codex_account_name: "primary".into(),
            account_group_id: "group-1".into(),
            account_group_name: "pool".into(),
        }
    }

    fn client() -> UsageClient {
        UsageClient::default()
    }

    /// A completed event with the given tokens; tests override the fields they check.
    fn completed(credential_type: &str, user_id: &str, total_tokens: i64) -> UsageEvent {
        UsageEvent {
            recorded_at_ms: current_time_ms().saturating_sub(1),
            credential_type: credential_type.into(),
            user_id: user_id.into(),
            transport: "http".into(),
            endpoint: "/v1/responses".into(),
            status: STATUS_COMPLETED.into(),
            tokens: TokenUsage {
                total_tokens,
                ..TokenUsage::default()
            },
            ..UsageEvent::default()
        }
    }

    async fn only_event(store: &UsageStore) -> UsageEventRow {
        let dashboard = wait_for_requests(store, 1).await;
        dashboard.recent_events.into_iter().next().unwrap()
    }

    fn temporary_store() -> (UsageStore, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "codex-router-usage-{}.sqlite3",
            uuid::Uuid::new_v4()
        ));
        (UsageStore::open(path.clone()).unwrap(), path)
    }

    fn remove_temporary_store(path: &Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
        let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
    }

    async fn wait_for_requests(store: &UsageStore, expected: i64) -> UsageDashboard {
        for _ in 0..100 {
            let dashboard = store
                .dashboard(UsageRange::Week, UsageFilters::default())
                .await
                .unwrap();
            if dashboard.totals.requests == expected {
                return dashboard;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("usage events were not persisted before the test timeout");
    }

    #[test]
    fn parses_codex_terminal_usage_and_subset_details() {
        let value = json!({
            "type":"response.completed",
            "response":{
                "id":"resp_1",
                "model":"gpt-5.6-sol",
                "usage":{
                    "input_tokens":120,
                    "input_tokens_details":{"cached_tokens":80,"cache_creation_tokens":4},
                    "output_tokens":30,
                    "output_tokens_details":{"reasoning_tokens":12},
                    "total_tokens":150
                }
            }
        });
        let parsed = parse_outcome(value.as_object().unwrap(), false).unwrap();
        assert_eq!(parsed.status, STATUS_COMPLETED);
        assert_eq!(parsed.response_model, "gpt-5.6-sol");
        assert_eq!(
            parsed.tokens,
            TokenUsage {
                input_tokens: 120,
                cached_input_tokens: 80,
                cache_creation_input_tokens: 4,
                output_tokens: 30,
                reasoning_output_tokens: 12,
                total_tokens: 150,
            }
        );
    }

    #[test]
    fn parses_compact_json_and_error_payloads() {
        let value = json!({"id":"resp_compact","usage":{"input_tokens":2,"output_tokens":3}});
        let parsed = parse_outcome(value.as_object().unwrap(), true).unwrap();
        assert_eq!(parsed.tokens.total_tokens, 5);
        assert_eq!(parsed.status, STATUS_COMPLETED);

        let failed = json!({"type":"response.failed","response":{"id":"resp_x","error":{"code":"server_error"}}});
        let parsed = parse_outcome(failed.as_object().unwrap(), false).unwrap();
        assert_eq!(
            (parsed.status, parsed.error_code.as_str()),
            (STATUS_FAILED, "server_error")
        );

        let incomplete = json!({"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"}}});
        let parsed = parse_outcome(incomplete.as_object().unwrap(), false).unwrap();
        assert_eq!(parsed.error_code, "max_output_tokens");

        let websocket = json!({"type":"error","status":429,"error":{"type":"usage_limit_reached"}});
        let parsed = parse_outcome(websocket.as_object().unwrap(), false).unwrap();
        assert_eq!(parsed.status_code, Some(429));
        assert_eq!(parsed.error_code, "usage_limit_reached");

        let body = json!({"detail":{"code":"token_expired"}});
        let parsed = parse_outcome(body.as_object().unwrap(), true).unwrap();
        assert_eq!(
            (parsed.status, parsed.error_code.as_str()),
            (STATUS_FAILED, "token_expired")
        );

        let delta = json!({"type":"response.output_text.delta","delta":"hi"});
        assert!(parse_outcome(delta.as_object().unwrap(), false).is_none());
    }

    #[tokio::test]
    async fn persists_and_aggregates_usage_with_response_deduplication() {
        let (store, path) = temporary_store();
        let event = UsageEvent {
            recorded_at_ms: current_time_ms().saturating_sub(1),
            credential_type: "api_key".into(),
            user_id: "key-1".into(),
            user_name: "laptop".into(),
            model: "gpt-5.6-sol".into(),
            transport: "http".into(),
            endpoint: "/v1/responses".into(),
            status: "completed".into(),
            response_id: "resp_unique".into(),
            tokens: TokenUsage {
                input_tokens: 10,
                cached_input_tokens: 4,
                output_tokens: 5,
                reasoning_output_tokens: 2,
                total_tokens: 15,
                ..TokenUsage::default()
            },
            ..UsageEvent::default()
        };
        store.insert(event.clone()).await.unwrap();
        store.insert(event).await.unwrap();
        let dashboard = store
            .dashboard(UsageRange::Week, UsageFilters::default())
            .await
            .unwrap();
        assert_eq!(dashboard.totals.requests, 1);
        assert_eq!(dashboard.totals.total_tokens, 15);
        assert_eq!(dashboard.models[0].model, "gpt-5.6-sol");
        assert_eq!(dashboard.identities[0].user_name, "laptop");
        assert_eq!(dashboard.recent_events[0].transport, "http");
        drop(store);
        remove_temporary_store(&path);
    }

    #[tokio::test]
    async fn cycle_is_seven_days_half_open_and_calculates_cost() {
        const WEEK_MS: i64 = 7 * 86_400_000;
        let (store, path) = temporary_store();
        let end_at = current_time_ms();
        let start_at = end_at - WEEK_MS;
        for (recorded_at_ms, response_id, service_tier) in [
            (start_at, "resp_cycle_start", "default"),
            (start_at + 1, "resp_cycle_fast", "priority"),
            (end_at, "resp_next_cycle", "default"),
        ] {
            store
                .insert(UsageEvent {
                    recorded_at_ms,
                    credential_type: "api_key".into(),
                    user_id: "key-1".into(),
                    user_name: "laptop".into(),
                    model: "gpt-test".into(),
                    transport: "http".into(),
                    endpoint: "/v1/responses".into(),
                    status: "completed".into(),
                    response_id: response_id.into(),
                    service_tier: service_tier.into(),
                    tokens: TokenUsage {
                        input_tokens: 1_000_000,
                        cached_input_tokens: 400_000,
                        cache_creation_input_tokens: 100_000,
                        output_tokens: 100_000,
                        total_tokens: 1_100_000,
                        ..TokenUsage::default()
                    },
                    ..UsageEvent::default()
                })
                .await
                .unwrap();
        }
        let price = ModelPrice {
            model: "gpt-test".into(),
            input: 2.0,
            output: 10.0,
            cache_read: 0.5,
            cache_write: 1.5,
            multiplier: 2.0,
        };
        let bounds = UsageBounds::cycle(start_at, end_at, end_at).unwrap();
        let dashboard = store
            .dashboard_with_options(
                UsageRange::Cycle,
                UsageFilters::default(),
                Some(bounds),
                &[price],
                3.0,
                UsageNames::default(),
            )
            .await
            .unwrap();

        assert_eq!(dashboard.start_at, start_at);
        assert_eq!(dashboard.end_at, end_at);
        assert_eq!(dashboard.series.len(), 7 * 24);
        assert_eq!(dashboard.totals.requests, 2);
        assert_eq!(dashboard.recent_events[1].recorded_at, start_at);
        // 4.7 for the standard request plus 3x for the Fast one.
        assert!((dashboard.totals.cost_usd - 18.8).abs() < 0.000_001);
        assert!((dashboard.models[0].totals.cost_usd - 18.8).abs() < 0.000_001);
        assert!((dashboard.recent_events[0].cost_usd - 14.1).abs() < 0.000_001);
        assert!((dashboard.recent_events[1].cost_usd - 4.7).abs() < 0.000_001);
        let fast_tier = dashboard
            .service_tiers
            .iter()
            .find(|row| row.value == "priority")
            .unwrap();
        assert!((fast_tier.totals.cost_usd - 14.1).abs() < 0.000_001);
        let series_cost = dashboard
            .series
            .iter()
            .map(|point| point.cost_usd)
            .sum::<f64>();
        assert!((series_cost - 18.8).abs() < 0.000_001);
        assert!((dashboard.identities[0].totals.cost_usd - 18.8).abs() < 0.000_001);
        assert!(dashboard.unpriced_models.is_empty());

        assert!(UsageBounds::cycle(start_at, end_at - 1, end_at).is_none());
        drop(store);
        remove_temporary_store(&path);
    }

    #[tokio::test]
    async fn finite_activity_ranges_are_always_seven_by_twenty_four_buckets() {
        let (store, path) = temporary_store();
        for range in [UsageRange::Day, UsageRange::Week, UsageRange::Month] {
            let dashboard = store
                .dashboard(range, UsageFilters::default())
                .await
                .unwrap();
            assert_eq!(dashboard.series.len(), ACTIVITY_BUCKET_COUNT as usize);
            assert_eq!(dashboard.series[0].start_at, dashboard.start_at);
            assert!(
                dashboard
                    .series
                    .windows(2)
                    .all(|points| points[0].start_at < points[1].start_at)
            );
            assert!(dashboard.series.last().unwrap().start_at < dashboard.end_at);
        }
        drop(store);
        remove_temporary_store(&path);
    }

    #[tokio::test]
    async fn filters_every_dashboard_view_by_identity() {
        let (store, path) = temporary_store();
        let now = current_time_ms().saturating_sub(1);
        for event in [
            UsageEvent {
                recorded_at_ms: now,
                user_id: "user-1".into(),
                user_name: "alice".into(),
                credential_type: "api_key".into(),
                credential_id: "key-1".into(),
                credential_name: "laptop".into(),
                codex_account_id: "codex-1".into(),
                codex_account_name: "personal".into(),
                account_group_id: "group-1".into(),
                account_group_name: "pool".into(),
                model: "gpt-5.6-sol".into(),
                transport: "http".into(),
                endpoint: "/v1/responses".into(),
                status: "completed".into(),
                response_id: "resp_filter_key".into(),
                tokens: TokenUsage {
                    input_tokens: 10,
                    cached_input_tokens: 2,
                    output_tokens: 5,
                    reasoning_output_tokens: 1,
                    total_tokens: 15,
                    ..TokenUsage::default()
                },
                ..UsageEvent::default()
            },
            UsageEvent {
                recorded_at_ms: now,
                user_id: "user-2".into(),
                user_name: "bob".into(),
                credential_type: "account_id".into(),
                credential_id: "account-1".into(),
                credential_name: "···nt-1".into(),
                codex_account_id: "codex-2".into(),
                codex_account_name: "team".into(),
                account_group_id: "group-1".into(),
                account_group_name: "pool".into(),
                model: "gpt-5.6-terra".into(),
                transport: "websocket".into(),
                endpoint: "/v1/responses".into(),
                status: "completed".into(),
                response_id: "resp_filter_account".into(),
                tokens: TokenUsage {
                    input_tokens: 60,
                    cached_input_tokens: 20,
                    cache_creation_input_tokens: 4,
                    output_tokens: 30,
                    reasoning_output_tokens: 12,
                    total_tokens: 90,
                },
                ..UsageEvent::default()
            },
            // A second credential of the first user, routed outside the group.
            UsageEvent {
                recorded_at_ms: now,
                user_id: "user-1".into(),
                user_name: "alice".into(),
                credential_type: "account_id".into(),
                credential_id: "account-2".into(),
                codex_account_id: "codex-3".into(),
                model: "gpt-5.6-sol".into(),
                transport: "http".into(),
                endpoint: "/v1/responses".into(),
                status: "completed".into(),
                response_id: "resp_filter_second_credential".into(),
                tokens: TokenUsage {
                    total_tokens: 5,
                    ..TokenUsage::default()
                },
                ..UsageEvent::default()
            },
        ] {
            store.insert(event).await.unwrap();
        }

        let user_filter = UsageIdentityFilter::parse_downstream("user", "user-1").unwrap();
        let user_dashboard = store
            .dashboard(UsageRange::Week, UsageFilters::new(Some(user_filter), None))
            .await
            .unwrap();
        assert_eq!(user_dashboard.totals.requests, 2);
        assert_eq!(user_dashboard.identities.len(), 1);
        assert_eq!(user_dashboard.identities[0].user_name, "alice");
        assert_eq!(user_dashboard.identities[0].totals.requests, 2);

        let filter = UsageIdentityFilter::parse_downstream("user", "user-2").unwrap();
        let dashboard = store
            .dashboard(UsageRange::Week, UsageFilters::new(Some(filter), None))
            .await
            .unwrap();

        assert_eq!(dashboard.totals.requests, 1);
        assert_eq!(dashboard.totals.total_tokens, 90);
        assert_eq!(dashboard.models.len(), 1);
        assert_eq!(dashboard.models[0].model, "gpt-5.6-terra");
        assert_eq!(dashboard.identities.len(), 1);
        assert_eq!(dashboard.identities[0].user_id, "user-2");
        assert_eq!(dashboard.recent_events.len(), 1);
        assert_eq!(dashboard.recent_events[0].credential_type, "account_id");
        assert_eq!(dashboard.recent_events[0].credential_id, "account-1");
        assert_eq!(
            dashboard
                .series
                .iter()
                .map(|point| point.requests)
                .sum::<i64>(),
            1
        );

        let account_filter =
            UsageIdentityFilter::parse_upstream("codex_account", "codex-1").unwrap();
        let account_dashboard = store
            .dashboard(
                UsageRange::Week,
                UsageFilters::new(None, Some(account_filter.clone())),
            )
            .await
            .unwrap();
        assert_eq!(account_dashboard.totals.requests, 1);
        assert_eq!(
            account_dashboard.recent_events[0].codex_account_name,
            "personal"
        );

        let group_filter = UsageIdentityFilter::parse_upstream("account_group", "group-1").unwrap();
        let group_dashboard = store
            .dashboard(
                UsageRange::Week,
                UsageFilters::new(None, Some(group_filter)),
            )
            .await
            .unwrap();
        assert_eq!(group_dashboard.totals.requests, 2);

        let downstream_filter =
            UsageIdentityFilter::parse_downstream("credential", "key-1").unwrap();
        let combined_dashboard = store
            .dashboard(
                UsageRange::Week,
                UsageFilters::new(Some(downstream_filter), Some(account_filter)),
            )
            .await
            .unwrap();
        assert_eq!(combined_dashboard.totals.requests, 1);
        assert_eq!(combined_dashboard.recent_events[0].user_id, "user-1");
        assert_eq!(
            combined_dashboard.recent_events[0].codex_account_id,
            "codex-1"
        );

        assert!(UsageIdentityFilter::parse_downstream("codex_account", "codex-1").is_none());
        assert!(UsageIdentityFilter::parse_upstream("user", "user-1").is_none());
        assert!(UsageIdentityFilter::parse_downstream("api_key", "key-1").is_none());
        assert!(UsageIdentityFilter::parse_downstream("user", "").is_none());
        drop(store);
        remove_temporary_store(&path);
    }

    #[tokio::test]
    async fn websocket_tracker_records_multiple_responses_and_request_models() {
        let (store, path) = temporary_store();
        let tracker = UsageTracker::websocket(store.clone(), identity(), client(), "/v1/responses");

        tracker.observe_request_text(r#"{"type":"response.create","model":"gpt-5.6-sol"}"#);
        tracker.observe_response_text(
            r#"{"type":"response.completed","response":{"id":"resp_ws_1","model":"gpt-5.6-sol","usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}}}"#,
        );
        tracker.observe_response_text(
            r#"{"type":"response.completed","response":{"id":"resp_ws_1","model":"gpt-5.6-sol","usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}}}"#,
        );
        tracker.observe_request_text(r#"{"type":"response.create","model":"gpt-5.6-terra"}"#);
        tracker.observe_response_text(
            r#"{"type":"response.completed","response":{"id":"resp_ws_2","usage":{"input_tokens":20,"output_tokens":7,"total_tokens":27}}}"#,
        );

        let dashboard = wait_for_requests(&store, 2).await;
        assert_eq!(dashboard.totals.total_tokens, 42);
        assert_eq!(dashboard.models.len(), 2);
        assert_eq!(dashboard.models[0].model, "gpt-5.6-terra");
        assert_eq!(dashboard.models[1].model, "gpt-5.6-sol");
        assert!(
            dashboard
                .recent_events
                .iter()
                .all(|event| event.transport == "websocket")
        );

        drop(tracker);
        drop(store);
        remove_temporary_store(&path);
    }

    #[test]
    fn extracts_reasoning_effort_and_service_tier_from_request_bodies() {
        let request = UsageRequest::from_body(
            json!({"model":"gpt-5.5","reasoning":{"effort":" XHigh "},"service_tier":"priority"})
                .as_object()
                .unwrap(),
        );
        assert_eq!(request.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(request.reasoning_effort, "xhigh");
        assert_eq!(request.service_tier, "priority");

        let nested = UsageRequest::from_body(
            json!({"type":"response.create","response":{"reasoning":{"effort":"low"},"service_tier":"fast"}})
                .as_object()
                .unwrap(),
        );
        assert_eq!(nested.reasoning_effort, "low");
        assert_eq!(nested.service_tier, "priority");

        let defaults = UsageRequest::from_body(json!({"model":"gpt-5.5"}).as_object().unwrap());
        assert_eq!(defaults.reasoning_effort, "default");
        assert_eq!(defaults.service_tier, "auto");
    }

    #[tokio::test]
    async fn records_request_settings_and_groups_them_in_the_dashboard() {
        let (store, path) = temporary_store();
        let tracker = UsageTracker::websocket(store.clone(), identity(), client(), "/v1/responses");

        tracker.observe_request_text(
            r#"{"type":"response.create","model":"gpt-5.5","reasoning":{"effort":"high"},"service_tier":"priority"}"#,
        );
        tracker.observe_response_text(
            r#"{"type":"response.completed","response":{"id":"resp_fast","service_tier":"priority","usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}}}"#,
        );
        tracker.observe_request_text(r#"{"type":"response.append","input":[]}"#);
        tracker.observe_response_text(
            r#"{"type":"response.completed","response":{"id":"resp_append","usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}}"#,
        );
        tracker.observe_request_text(r#"{"type":"response.create","model":"gpt-5.5"}"#);
        tracker.observe_response_text(
            r#"{"type":"response.completed","response":{"id":"resp_standard","service_tier":"auto","usage":{"input_tokens":20,"output_tokens":7,"total_tokens":27}}}"#,
        );

        let dashboard = wait_for_requests(&store, 3).await;
        let efforts = dashboard
            .reasoning_efforts
            .iter()
            .map(|row| (row.value.as_str(), row.totals.requests))
            .collect::<Vec<_>>();
        assert_eq!(efforts, vec![("high", 2), ("default", 1)]);
        let tiers = dashboard
            .service_tiers
            .iter()
            .map(|row| (row.value.as_str(), row.totals.total_tokens))
            .collect::<Vec<_>>();
        assert_eq!(tiers, vec![("priority", 17), ("auto", 27)]);
        let fast = dashboard
            .recent_events
            .iter()
            .find(|event| event.total_tokens == 15)
            .unwrap();
        assert_eq!(fast.reasoning_effort, "high");
        assert_eq!(fast.service_tier, "priority");
        assert_eq!(fast.response_service_tier, "priority");
        let standard = dashboard
            .recent_events
            .iter()
            .find(|event| event.total_tokens == 27)
            .unwrap();
        assert_eq!(standard.service_tier, "auto");
        assert_eq!(standard.response_service_tier, "auto");

        drop(tracker);
        drop(store);
        remove_temporary_store(&path);
    }

    #[test]
    fn rebuilds_tables_written_by_other_schema_versions() {
        let path = std::env::temp_dir().join(format!(
            "codex-router-usage-{}.sqlite3",
            uuid::Uuid::new_v4()
        ));
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE usage_events (id INTEGER PRIMARY KEY, model TEXT NOT NULL); \
                     INSERT INTO usage_events (model) VALUES ('legacy');",
                )
                .unwrap();
        }
        let store = UsageStore::open(path.clone()).unwrap();
        {
            let connection = store.lock();
            let rows: i64 = connection
                .query_row("SELECT COUNT(*) FROM usage_events", [], |row| row.get(0))
                .unwrap();
            assert_eq!(rows, 0);
            let version: i64 = connection
                .query_row("PRAGMA user_version", [], |row| row.get(0))
                .unwrap();
            assert_eq!(version, USAGE_SCHEMA_VERSION);
            insert_event(&connection, &completed("api_key", "k", 1)).unwrap();
        }
        drop(store);
        // Reopening at the current version keeps the data.
        let store = UsageStore::open(path.clone()).unwrap();
        let rows: i64 = store
            .lock()
            .query_row("SELECT COUNT(*) FROM usage_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1);
        drop(store);
        remove_temporary_store(&path);
    }

    #[tokio::test]
    async fn request_stream_observer_reads_compressed_bodies_without_changing_them() {
        use ruzstd::encoding::{CompressionLevel, compress_to_vec};

        let (store, path) = temporary_store();
        let tracker = UsageTracker::http(
            store.clone(),
            identity(),
            client(),
            "/backend-api/codex/responses",
        );
        let encoded = compress_to_vec(
            &br#"{"model":"gpt-5.5","reasoning":{"effort":"medium"},"service_tier":"priority"}"#[..],
            CompressionLevel::Fastest,
        );
        let chunks = encoded
            .chunks(5)
            .map(|chunk| Ok::<_, std::io::Error>(Bytes::copy_from_slice(chunk)))
            .collect::<Vec<_>>();
        let forwarded = tracker
            .observe_request_stream(stream::iter(chunks), Some("zstd".into()))
            .map(Result::unwrap)
            .collect::<Vec<_>>()
            .await
            .concat();
        assert_eq!(forwarded, encoded);

        tracker.observe_response_text(
            r#"{"type":"response.completed","response":{"id":"resp_relay","usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}}}"#,
        );
        let dashboard = wait_for_requests(&store, 1).await;
        let event = &dashboard.recent_events[0];
        assert_eq!(event.model, "gpt-5.5");
        assert_eq!(event.reasoning_effort, "medium");
        assert_eq!(event.service_tier, "priority");

        drop(tracker);
        drop(store);
        remove_temporary_store(&path);
    }

    #[tokio::test]
    async fn sse_wire_observer_handles_split_terminal_events() {
        let (store, path) = temporary_store();
        let tracker = UsageTracker::http(store.clone(), identity(), client(), "/v1/responses");
        tracker.record_request(UsageRequest::from_body(
            json!({"model":"gpt-5.4","stream":true})
                .as_object()
                .unwrap(),
        ));
        let mut observer = tracker.wire_observer(Some("text/event-stream; charset=utf-8"));
        let payload = br#"data: {"type":"response.completed","response":{"id":"resp_sse_1","usage":{"input_tokens":8,"input_tokens_details":{"cached_tokens":6},"output_tokens":3,"total_tokens":11}}}

data: [DONE]

"#;
        for chunk in payload.chunks(7) {
            observer.push(chunk);
        }
        observer.finish();

        let dashboard = wait_for_requests(&store, 1).await;
        assert_eq!(dashboard.totals.total_tokens, 11);
        assert_eq!(dashboard.totals.cached_input_tokens, 6);
        assert_eq!(dashboard.models[0].model, "gpt-5.4");
        assert_eq!(dashboard.recent_events[0].transport, "http");

        drop(tracker);
        drop(store);
        remove_temporary_store(&path);
    }

    #[tokio::test]
    async fn records_client_metadata_session_and_timing() {
        use super::super::usage_client::UsageSession;

        let (store, path) = temporary_store();
        let client = UsageClient {
            ip: "203.0.113.7".into(),
            forwarded_for: "203.0.113.7, 10.0.0.2".into(),
            user_agent: "codex_cli_rs/0.80.0".into(),
            session: UsageSession {
                id: "thread-2".into(),
                parent_id: "thread-1".into(),
                kind: SessionKind::Subagent,
            },
        };
        let tracker = UsageTracker::http(store.clone(), identity(), client, "/v1/responses");
        tracker.record_request(UsageRequest::from_body(
            json!({"model":"gpt-5.5","stream":true})
                .as_object()
                .unwrap(),
        ));
        tracker.observe_upstream_status(200);
        tracker.observe_response_text(r#"{"type":"response.output_text.delta","delta":"hi"}"#);
        tracker.observe_response_text(
            r#"{"type":"response.completed","response":{"id":"resp_meta","model":"gpt-5.5-2026","usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}}"#,
        );
        let event = only_event(&store).await;
        assert_eq!(event.requested_model, "gpt-5.5");
        assert_eq!(event.model, "gpt-5.5-2026");
        assert!(event.stream);
        assert_eq!(event.status, "completed");
        assert_eq!(event.status_code, Some(200));
        assert_eq!(event.client_ip, "203.0.113.7");
        assert_eq!(event.x_forwarded_for, "203.0.113.7, 10.0.0.2");
        assert_eq!(event.user_agent, "codex_cli_rs/0.80.0");
        assert_eq!(event.session_id, "thread-2");
        assert_eq!(event.parent_session_id, "thread-1");
        assert_eq!(event.session_kind, "subagent");
        assert_eq!(event.request_id.len(), 32);
        assert!(event.ttft_ms.is_some_and(|ttft| ttft <= event.latency_ms));

        drop(tracker);
        drop(store);
        remove_temporary_store(&path);
    }

    #[tokio::test]
    async fn falls_back_to_the_prompt_cache_key_and_marks_compactions() {
        let (store, path) = temporary_store();
        let tracker =
            UsageTracker::http(store.clone(), identity(), client(), "/v1/responses/compact");
        tracker.record_request(UsageRequest::from_body(
            json!({"model":"gpt-5.5","prompt_cache_key":"conv-1"})
                .as_object()
                .unwrap(),
        ));
        tracker.observe_response_body(
            &json!({"id":"resp_c","usage":{"input_tokens":4,"output_tokens":1}}),
        );
        let event = only_event(&store).await;
        assert_eq!(event.session_id, "conv-1");
        assert_eq!(event.session_kind, "compaction");
        assert!(!event.stream);
        assert_eq!(event.ttft_ms, None);

        drop(tracker);
        drop(store);
        remove_temporary_store(&path);
    }

    #[tokio::test]
    async fn records_upstream_http_errors_with_their_code() {
        let (store, path) = temporary_store();
        let tracker = UsageTracker::http(store.clone(), identity(), client(), "/v1/responses");
        tracker.record_request(UsageRequest::from_body(
            json!({"model":"gpt-5.5"}).as_object().unwrap(),
        ));
        tracker.observe_upstream_status(429);
        let mut observer = tracker.wire_observer(Some("application/json"));
        observer.push(br#"{"error":{"type":"usage_limit_reached","message":"limit"}}"#);
        observer.finish();
        drop(tracker);

        let event = only_event(&store).await;
        assert_eq!(event.status, "failed");
        assert_eq!(event.status_code, Some(429));
        assert_eq!(event.error_code, "usage_limit_reached");
        assert_eq!(event.model, "gpt-5.5");
        assert_eq!(event.total_tokens, 0);

        drop(store);
        remove_temporary_store(&path);
    }

    #[tokio::test]
    async fn records_router_errors_raised_before_the_upstream_answers() {
        let (store, path) = temporary_store();
        let tracker = UsageTracker::http(store.clone(), identity(), client(), "/v1/responses");
        tracker.fail(
            &ApiError::new(502, "unreachable")
                .with_kind("upstream_error")
                .with_code("codex_unavailable"),
        );
        // Dropping after an explicit failure must not add a second row.
        drop(tracker);
        let event = only_event(&store).await;
        assert_eq!(event.status, "failed");
        assert_eq!(event.status_code, Some(502));
        assert_eq!(event.error_code, "codex_unavailable");
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(wait_for_requests(&store, 1).await.totals.requests, 1);

        drop(store);
        remove_temporary_store(&path);
    }

    #[tokio::test]
    async fn distinguishes_cancelled_from_interrupted_streams() {
        let (store, path) = temporary_store();
        // The client disconnected: the observer is dropped without finishing.
        let tracker = UsageTracker::http(store.clone(), identity(), client(), "/v1/responses");
        tracker.observe_upstream_status(200);
        let observer = tracker.wire_observer(Some("text/event-stream"));
        drop(observer);
        drop(tracker);
        // The upstream ended without a terminal event.
        let tracker = UsageTracker::http(store.clone(), identity(), client(), "/v1/responses");
        tracker.observe_upstream_status(200);
        let mut observer = tracker.wire_observer(Some("text/event-stream"));
        observer.push(b"data: {\"type\":\"response.created\"}\n\n");
        observer.finish();
        drop(tracker);

        let dashboard = wait_for_requests(&store, 2).await;
        let mut statuses = dashboard
            .recent_events
            .iter()
            .map(|event| (event.status.as_str(), event.error_code.as_str()))
            .collect::<Vec<_>>();
        statuses.sort_unstable();
        assert_eq!(
            statuses,
            vec![
                ("cancelled", "client_cancelled"),
                ("interrupted", "stream_truncated")
            ]
        );
        let failed = dashboard
            .series
            .iter()
            .map(|point| point.failed_requests)
            .sum::<i64>();
        let successful = dashboard
            .series
            .iter()
            .map(|point| point.successful_requests)
            .sum::<i64>();
        assert_eq!((successful, failed), (0, 1));

        drop(store);
        remove_temporary_store(&path);
    }

    #[tokio::test]
    async fn websocket_attempts_cover_errors_and_superseded_responses() {
        let (store, path) = temporary_store();
        let tracker = UsageTracker::websocket(store.clone(), identity(), client(), "/v1/responses");
        tracker.observe_request_text(r#"{"type":"response.create","model":"gpt-5.5"}"#);
        tracker.observe_request_text(r#"{"type":"response.create","model":"gpt-5.5"}"#);
        tracker.observe_response_text(
            r#"{"type":"error","status":429,"error":{"type":"usage_limit_reached"}}"#,
        );
        // A terminal event without an open attempt is ignored.
        tracker
            .observe_response_text(r#"{"type":"response.failed","response":{"id":"resp_late"}}"#);
        drop(tracker);

        let dashboard = wait_for_requests(&store, 2).await;
        let mut outcomes = dashboard
            .recent_events
            .iter()
            .map(|event| {
                (
                    event.status.as_str(),
                    event.error_code.as_str(),
                    event.status_code,
                )
            })
            .collect::<Vec<_>>();
        outcomes.sort_unstable();
        assert_eq!(
            outcomes,
            vec![
                ("cancelled", "superseded", None),
                ("failed", "usage_limit_reached", Some(429)),
            ]
        );
        assert!(dashboard.recent_events.iter().all(|event| event.stream));

        drop(store);
        remove_temporary_store(&path);
    }

    #[tokio::test]
    async fn websocket_handshake_rejections_are_recorded_once() {
        let (store, path) = temporary_store();
        let tracker = UsageTracker::websocket(store.clone(), identity(), client(), "/v1/responses");
        tracker.observe_upstream_status(401);
        drop(tracker);
        let event = only_event(&store).await;
        assert_eq!(event.status, "failed");
        assert_eq!(event.status_code, Some(401));
        assert_eq!(event.model, "");

        drop(store);
        remove_temporary_store(&path);
    }

    #[cfg(unix)]
    #[test]
    fn restricts_database_and_sidecar_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let (store, path) = temporary_store();
        for candidate in [
            path.clone(),
            sqlite_sidecar_path(&path, "-wal"),
            sqlite_sidecar_path(&path, "-shm"),
        ] {
            let mode = std::fs::metadata(candidate).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        drop(store);
        remove_temporary_store(&path);
    }
}

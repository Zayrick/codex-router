use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::{
    auth::{AuthProxyAccount, ClientApiKey},
    protocol::openai,
};

use super::{
    oauth::current_time_ms,
    pricing::{ModelPrice, calculate_cost, price_index},
};

const MAX_TRACKED_JSON_BYTES: usize = 16 * 1024 * 1024;
const RECENT_EVENT_LIMIT: i64 = 50;
const BREAKDOWN_LIMIT: i64 = 12;
const ACTIVITY_BUCKET_COUNT: i64 = 7 * 24;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS usage_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    recorded_at_ms INTEGER NOT NULL,
    identity_type TEXT NOT NULL,
    identity_id TEXT NOT NULL,
    identity_name TEXT NOT NULL,
    codex_account_id TEXT NOT NULL DEFAULT '',
    codex_account_name TEXT NOT NULL DEFAULT '',
    account_group_id TEXT NOT NULL DEFAULT '',
    account_group_name TEXT NOT NULL DEFAULT '',
    model TEXT NOT NULL,
    transport TEXT NOT NULL,
    endpoint TEXT NOT NULL,
    status TEXT NOT NULL,
    response_id TEXT NOT NULL DEFAULT '',
    input_tokens INTEGER NOT NULL DEFAULT 0,
    cached_input_tokens INTEGER NOT NULL DEFAULT 0,
    cache_creation_input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    reasoning_output_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_usage_events_recorded_at
    ON usage_events(recorded_at_ms DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_usage_events_identity
    ON usage_events(identity_type, identity_id, recorded_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_usage_events_model
    ON usage_events(model, recorded_at_ms DESC);
CREATE UNIQUE INDEX IF NOT EXISTS idx_usage_events_response_id
    ON usage_events(response_id) WHERE response_id <> '';
"#;

const ROUTING_INDEXES: &str = r#"
CREATE INDEX IF NOT EXISTS idx_usage_events_codex_account
    ON usage_events(codex_account_id, recorded_at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_usage_events_account_group
    ON usage_events(account_group_id, recorded_at_ms DESC);
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageIdentity {
    pub kind: UsageIdentityKind,
    pub id: String,
    pub name: String,
    pub codex_account_id: String,
    pub codex_account_name: String,
    pub account_group_id: String,
    pub account_group_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageIdentityKind {
    ApiKey,
    AuthProxy,
    CodexAccount,
    AccountGroup,
}

impl UsageIdentityKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ApiKey => "api_key",
            Self::AuthProxy => "auth_proxy",
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
            "api_key" => UsageIdentityKind::ApiKey,
            "auth_proxy" => UsageIdentityKind::AuthProxy,
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
                UsageIdentityKind::ApiKey | UsageIdentityKind::AuthProxy
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

    fn kind(&self) -> &'static str {
        self.kind.as_str()
    }

    fn id(&self) -> &str {
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
            UsageIdentityKind::ApiKey | UsageIdentityKind::AuthProxy => {
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

impl From<&ClientApiKey> for UsageIdentity {
    fn from(value: &ClientApiKey) -> Self {
        Self {
            kind: UsageIdentityKind::ApiKey,
            id: value.id.clone(),
            name: value.name.clone(),
            codex_account_id: String::new(),
            codex_account_name: String::new(),
            account_group_id: String::new(),
            account_group_name: String::new(),
        }
    }
}

impl From<&AuthProxyAccount> for UsageIdentity {
    fn from(value: &AuthProxyAccount) -> Self {
        Self {
            kind: UsageIdentityKind::AuthProxy,
            id: value.id.clone(),
            name: value.name.clone(),
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct UsageEvent {
    recorded_at_ms: i64,
    identity_type: String,
    identity_id: String,
    identity_name: String,
    codex_account_id: String,
    codex_account_name: String,
    account_group_id: String,
    account_group_name: String,
    model: String,
    transport: String,
    endpoint: String,
    status: String,
    response_id: String,
    input_tokens: i64,
    cached_input_tokens: i64,
    cache_creation_input_tokens: i64,
    output_tokens: i64,
    reasoning_output_tokens: i64,
    total_tokens: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedUsage {
    model: String,
    status: String,
    response_id: String,
    input_tokens: i64,
    cached_input_tokens: i64,
    cache_creation_input_tokens: i64,
    output_tokens: i64,
    reasoning_output_tokens: i64,
    total_tokens: i64,
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
        connection
            .execute_batch(SCHEMA)
            .context("failed to initialize usage database schema")?;
        ensure_usage_routing_columns(&connection)?;
        connection
            .execute_batch(ROUTING_INDEXES)
            .context("failed to initialize usage routing indexes")?;
        restrict_database_permissions(&path)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            path: Arc::new(path),
        })
    }

    pub fn path(&self) -> &Path {
        self.path.as_ref()
    }

    fn record_background(&self, event: UsageEvent) {
        let store = self.clone();
        tokio::spawn(async move {
            if let Err(error) = store.insert(event).await {
                tracing::warn!(event = "usage_record", status = "failed", error = %error);
            }
        });
    }

    async fn insert(&self, event: UsageEvent) -> Result<()> {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let connection = connection.lock().expect("usage database lock poisoned");
            connection.execute(
                r#"
                INSERT OR IGNORE INTO usage_events (
                    recorded_at_ms, identity_type, identity_id, identity_name,
                    codex_account_id, codex_account_name, account_group_id, account_group_name,
                    model, transport, endpoint, status, response_id,
                    input_tokens, cached_input_tokens, cache_creation_input_tokens,
                    output_tokens, reasoning_output_tokens, total_tokens
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
                "#,
                params![
                    event.recorded_at_ms,
                    event.identity_type,
                    event.identity_id,
                    event.identity_name,
                    event.codex_account_id,
                    event.codex_account_name,
                    event.account_group_id,
                    event.account_group_name,
                    event.model,
                    event.transport,
                    event.endpoint,
                    event.status,
                    event.response_id,
                    event.input_tokens,
                    event.cached_input_tokens,
                    event.cache_creation_input_tokens,
                    event.output_tokens,
                    event.reasoning_output_tokens,
                    event.total_tokens,
                ],
            )?;
            Ok::<_, anyhow::Error>(())
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
        self.dashboard_with_options(range, filters, None, &[]).await
    }

    pub async fn dashboard_with_options(
        &self,
        range: UsageRange,
        filters: UsageFilters,
        bounds: Option<UsageBounds>,
        prices: &[ModelPrice],
    ) -> Result<UsageDashboard> {
        let connection = self.connection.clone();
        let prices = prices.to_vec();
        tokio::task::spawn_blocking(move || {
            let connection = connection.lock().expect("usage database lock poisoned");
            query_dashboard(&connection, range, &filters, bounds, &prices)
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

fn ensure_usage_routing_columns(connection: &Connection) -> Result<()> {
    let mut statement = connection.prepare("PRAGMA table_info(usage_events)")?;
    let columns: std::collections::HashSet<String> = statement
        .query_map([], |row| row.get(1))?
        .collect::<rusqlite::Result<_>>()?;
    drop(statement);
    for name in [
        "codex_account_id",
        "codex_account_name",
        "account_group_id",
        "account_group_name",
    ] {
        if !columns.contains(name) {
            connection.execute(
                &format!("ALTER TABLE usage_events ADD COLUMN {name} TEXT NOT NULL DEFAULT ''"),
                [],
            )?;
        }
    }
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

#[derive(Clone)]
pub struct UsageTracker {
    inner: Arc<UsageTrackerInner>,
}

struct UsageTrackerInner {
    store: UsageStore,
    identity: UsageIdentity,
    endpoint: String,
    transport: &'static str,
    requested_model: Mutex<String>,
}

impl UsageTracker {
    pub fn http(store: UsageStore, identity: UsageIdentity, endpoint: impl Into<String>) -> Self {
        Self::new(store, identity, endpoint, "http")
    }

    pub fn websocket(
        store: UsageStore,
        identity: UsageIdentity,
        endpoint: impl Into<String>,
    ) -> Self {
        Self::new(store, identity, endpoint, "websocket")
    }

    fn new(
        store: UsageStore,
        identity: UsageIdentity,
        endpoint: impl Into<String>,
        transport: &'static str,
    ) -> Self {
        Self {
            inner: Arc::new(UsageTrackerInner {
                store,
                identity,
                endpoint: endpoint.into(),
                transport,
                requested_model: Mutex::new(String::new()),
            }),
        }
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
        let Some(model) = request_model(object) else {
            return;
        };
        self.set_requested_model(model);
    }

    pub fn observe_request_text(&self, text: &str) {
        if let Ok(value) = serde_json::from_str::<Value>(text) {
            self.observe_request_value(&value);
        }
    }

    pub fn set_requested_model(&self, model: &str) {
        let model = model.trim();
        if model.is_empty() {
            return;
        }
        *self
            .inner
            .requested_model
            .lock()
            .expect("usage request lock poisoned") = model.to_owned();
    }

    pub fn observe_response_value(&self, value: &Value) {
        let Some(object) = value.as_object() else {
            return;
        };
        self.observe_response_object(object);
    }

    pub fn observe_response_object(&self, object: &Map<String, Value>) {
        let fallback_model = self
            .inner
            .requested_model
            .lock()
            .expect("usage request lock poisoned")
            .clone();
        let Some(parsed) = parse_usage(object, &fallback_model) else {
            return;
        };
        self.inner.store.record_background(UsageEvent {
            recorded_at_ms: current_time_ms(),
            identity_type: self.inner.identity.kind.as_str().into(),
            identity_id: self.inner.identity.id.clone(),
            identity_name: self.inner.identity.name.clone(),
            codex_account_id: self.inner.identity.codex_account_id.clone(),
            codex_account_name: self.inner.identity.codex_account_name.clone(),
            account_group_id: self.inner.identity.account_group_id.clone(),
            account_group_name: self.inner.identity.account_group_name.clone(),
            model: parsed.model,
            transport: self.inner.transport.into(),
            endpoint: self.inner.endpoint.clone(),
            status: parsed.status,
            response_id: parsed.response_id,
            input_tokens: parsed.input_tokens,
            cached_input_tokens: parsed.cached_input_tokens,
            cache_creation_input_tokens: parsed.cache_creation_input_tokens,
            output_tokens: parsed.output_tokens,
            reasoning_output_tokens: parsed.reasoning_output_tokens,
            total_tokens: parsed.total_tokens,
        });
    }

    pub fn observe_response_text(&self, text: &str) {
        if let Ok(value) = serde_json::from_str::<Value>(text) {
            self.observe_response_value(&value);
        }
    }

    pub fn wire_observer(&self, content_type: Option<&str>) -> UsageWireObserver {
        UsageWireObserver::new(self.clone(), content_type)
    }
}

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
                    self.tracker.observe_response_value(&value);
                }
            }
        }
    }
}

fn request_model(object: &Map<String, Value>) -> Option<&str> {
    string(object, "model").or_else(|| {
        object
            .get("response")
            .and_then(Value::as_object)
            .and_then(|response| string(response, "model"))
    })
}

fn parse_usage(root: &Map<String, Value>, fallback_model: &str) -> Option<ParsedUsage> {
    let kind = string(root, "type");
    let terminal = match kind {
        Some("response.completed") => "completed",
        Some("response.incomplete") => "incomplete",
        Some("response.failed") => "failed",
        Some(_) => return None,
        None if root.get("usage").and_then(Value::as_object).is_some() => "completed",
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
    let usage = usage?;
    let input_tokens = token(usage, "input_tokens").unwrap_or(0);
    let output_tokens = token(usage, "output_tokens").unwrap_or(0);
    let cached_input_tokens = usage
        .get("input_tokens_details")
        .and_then(Value::as_object)
        .and_then(|details| token(details, "cached_tokens"))
        .unwrap_or(0);
    let cache_creation_input_tokens = usage
        .get("input_tokens_details")
        .and_then(Value::as_object)
        .and_then(|details| token(details, "cache_creation_tokens"))
        .unwrap_or(0);
    let reasoning_output_tokens = usage
        .get("output_tokens_details")
        .and_then(Value::as_object)
        .and_then(|details| token(details, "reasoning_tokens"))
        .unwrap_or(0);
    let total_tokens = token(usage, "total_tokens").unwrap_or(input_tokens + output_tokens);
    let model = string(response, "model")
        .or_else(|| string(root, "model"))
        .unwrap_or(fallback_model)
        .trim();
    Some(ParsedUsage {
        model: if model.is_empty() { "unknown" } else { model }.into(),
        status: terminal.into(),
        response_id: string(response, "id").unwrap_or_default().trim().into(),
        input_tokens,
        cached_input_tokens,
        cache_creation_input_tokens,
        output_tokens,
        reasoning_output_tokens,
        total_tokens,
    })
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

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageIdentityRow {
    pub identity_type: String,
    pub identity_id: String,
    pub identity_name: String,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageEventRow {
    pub id: i64,
    pub recorded_at: i64,
    pub identity_type: String,
    pub identity_id: String,
    pub identity_name: String,
    pub codex_account_id: String,
    pub codex_account_name: String,
    pub account_group_id: String,
    pub account_group_name: String,
    pub model: String,
    pub transport: String,
    pub endpoint: String,
    pub status: String,
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_output_tokens: i64,
    pub total_tokens: i64,
    pub cost_usd: f64,
}

fn query_dashboard(
    connection: &Connection,
    range: UsageRange,
    filters: &UsageFilters,
    bounds: Option<UsageBounds>,
    prices: &[ModelPrice],
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
    let prices = price_index(prices);
    let mut totals = query_totals(connection, start_at, end_at, filters)?;
    totals.cost_usd = query_total_cost(connection, start_at, end_at, filters, &prices)?;
    let series = query_series(connection, range, start_at, end_at, filters, &prices)?;
    let mut models = query_models(connection, start_at, end_at, filters)?;
    for row in &mut models {
        row.totals.cost_usd = cost_for_model(&row.model, &row.totals, &prices);
    }
    let mut identities = query_identities(connection, start_at, end_at, filters)?;
    apply_identity_costs(
        connection,
        start_at,
        end_at,
        filters,
        &prices,
        &mut identities,
    )?;
    let mut recent_events = query_recent_events(connection, start_at, end_at, filters)?;
    for event in &mut recent_events {
        event.cost_usd = event_cost(event, &prices);
    }
    let unpriced_models = query_unpriced_models(connection, start_at, end_at, filters, &prices)?;
    Ok(UsageDashboard {
        range: range.label().into(),
        start_at,
        end_at,
        totals,
        series,
        models,
        identities,
        recent_events,
        unpriced_models,
    })
}

fn filter_parameters(
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
         OR (?{kind_parameter} IN ('api_key', 'auth_proxy') \
             AND identity_type = ?{kind_parameter} AND identity_id = ?{id_parameter}) \
         OR (?{kind_parameter} = 'codex_account' AND codex_account_id = ?{id_parameter}) \
         OR (?{kind_parameter} = 'account_group' AND account_group_id = ?{id_parameter}))"
    )
}

fn usage_filters_sql(
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
         AND {usage_filters} \
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
        "SELECT identity_type, identity_id, identity_name, {TOTAL_COLUMNS} FROM usage_events \
         WHERE recorded_at_ms >= ?1 AND recorded_at_ms < ?2 \
         AND {usage_filters} \
         GROUP BY identity_type, identity_id, identity_name \
         ORDER BY SUM(total_tokens) DESC, COUNT(*) DESC, identity_name LIMIT ?7"
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
                identity_type: row.get(0)?,
                identity_id: row.get(1)?,
                identity_name: row.get(2)?,
                totals: totals_from_row_at(row, 3)?,
            })
        },
    )?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn query_series(
    connection: &Connection,
    range: UsageRange,
    start_at: i64,
    end_at: i64,
    filters: &UsageFilters,
    prices: &BTreeMap<String, ModelPrice>,
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
        SELECT (((recorded_at_ms - ?1) * ?2) / ?3), model, {TOTAL_COLUMNS},
               COALESCE(SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END), 0),
               COALESCE(SUM(CASE WHEN status = 'completed' THEN 0 ELSE 1 END), 0)
        FROM usage_events
        WHERE recorded_at_ms >= ?4 AND recorded_at_ms < ?5
          AND {usage_filters}
        GROUP BY 1, model ORDER BY 1
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
                totals_from_row_at(row, 2)?,
                row.get::<_, i64>(9)?,
                row.get::<_, i64>(10)?,
            ))
        },
    )?;
    let mut populated = BTreeMap::<i64, SeriesAccumulator>::new();
    for row in rows {
        let (index, model, totals, successful_requests, failed_requests) = row?;
        let cost = cost_for_model(&model, &totals, prices);
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

fn cost_for_model(model: &str, totals: &UsageTotals, prices: &BTreeMap<String, ModelPrice>) -> f64 {
    let Some(price) = prices.get(&model.trim().to_ascii_lowercase()) else {
        return 0.0;
    };
    calculate_cost(
        totals.input_tokens,
        totals.output_tokens,
        totals.cached_input_tokens,
        totals.cache_creation_input_tokens,
        price,
    )
}

fn query_total_cost(
    connection: &Connection,
    start_at: i64,
    end_at: i64,
    filters: &UsageFilters,
    prices: &BTreeMap<String, ModelPrice>,
) -> Result<f64> {
    let usage_filters = usage_filters_sql(3, 4, 5, 6);
    let sql = format!(
        "SELECT model, {TOTAL_COLUMNS} FROM usage_events \
         WHERE recorded_at_ms >= ?1 AND recorded_at_ms < ?2 \
         AND {usage_filters} \
         GROUP BY model"
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
        |row| Ok((row.get::<_, String>(0)?, totals_from_row_at(row, 1)?)),
    )?;
    let mut cost = 0.0;
    for row in rows {
        let (model, totals) = row?;
        cost += cost_for_model(&model, &totals, prices);
    }
    Ok(cost)
}

fn apply_identity_costs(
    connection: &Connection,
    start_at: i64,
    end_at: i64,
    filters: &UsageFilters,
    prices: &BTreeMap<String, ModelPrice>,
    identities: &mut [UsageIdentityRow],
) -> Result<()> {
    let usage_filters = usage_filters_sql(3, 4, 5, 6);
    let sql = format!(
        "SELECT identity_type, identity_id, model, {TOTAL_COLUMNS} FROM usage_events \
         WHERE recorded_at_ms >= ?1 AND recorded_at_ms < ?2 \
         AND {usage_filters} \
         GROUP BY identity_type, identity_id, model"
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
                row.get::<_, String>(2)?,
                totals_from_row_at(row, 3)?,
            ))
        },
    )?;
    let mut costs = BTreeMap::<(String, String), f64>::new();
    for row in rows {
        let (kind, id, model, totals) = row?;
        *costs.entry((kind, id)).or_default() += cost_for_model(&model, &totals, prices);
    }
    for row in identities {
        row.totals.cost_usd = costs
            .get(&(row.identity_type.clone(), row.identity_id.clone()))
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
    prices: &BTreeMap<String, ModelPrice>,
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
        .filter(|model| !prices.contains_key(&model.trim().to_ascii_lowercase()))
        .collect())
}

fn event_cost(event: &UsageEventRow, prices: &BTreeMap<String, ModelPrice>) -> f64 {
    let Some(price) = prices.get(&event.model.trim().to_ascii_lowercase()) else {
        return 0.0;
    };
    calculate_cost(
        event.input_tokens,
        event.output_tokens,
        event.cached_input_tokens,
        event.cache_creation_input_tokens,
        price,
    )
}

fn query_recent_events(
    connection: &Connection,
    start_at: i64,
    end_at: i64,
    filters: &UsageFilters,
) -> Result<Vec<UsageEventRow>> {
    let sql = format!(
        r#"
        SELECT id, recorded_at_ms, identity_type, identity_id, identity_name,
               codex_account_id, codex_account_name, account_group_id, account_group_name,
               model, transport, endpoint, status, input_tokens, cached_input_tokens,
               cache_creation_input_tokens, output_tokens, reasoning_output_tokens, total_tokens
        FROM usage_events
        WHERE recorded_at_ms >= ?1 AND recorded_at_ms < ?2
          AND {}
        ORDER BY recorded_at_ms DESC, id DESC LIMIT ?7
        "#,
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
        |row| {
            Ok(UsageEventRow {
                id: row.get(0)?,
                recorded_at: row.get(1)?,
                identity_type: row.get(2)?,
                identity_id: row.get(3)?,
                identity_name: row.get(4)?,
                codex_account_id: row.get(5)?,
                codex_account_name: row.get(6)?,
                account_group_id: row.get(7)?,
                account_group_name: row.get(8)?,
                model: row.get(9)?,
                transport: row.get(10)?,
                endpoint: row.get(11)?,
                status: row.get(12)?,
                input_tokens: row.get(13)?,
                cached_input_tokens: row.get(14)?,
                cache_creation_input_tokens: row.get(15)?,
                output_tokens: row.get(16)?,
                reasoning_output_tokens: row.get(17)?,
                total_tokens: row.get(18)?,
                cost_usd: 0.0,
            })
        },
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
            kind: UsageIdentityKind::ApiKey,
            id: "key-1".into(),
            name: "laptop".into(),
            codex_account_id: "codex-1".into(),
            codex_account_name: "primary".into(),
            account_group_id: "group-1".into(),
            account_group_name: "pool".into(),
        }
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
        let parsed = parse_usage(value.as_object().unwrap(), "fallback").unwrap();
        assert_eq!(parsed.model, "gpt-5.6-sol");
        assert_eq!(parsed.input_tokens, 120);
        assert_eq!(parsed.cached_input_tokens, 80);
        assert_eq!(parsed.cache_creation_input_tokens, 4);
        assert_eq!(parsed.output_tokens, 30);
        assert_eq!(parsed.reasoning_output_tokens, 12);
        assert_eq!(parsed.total_tokens, 150);
    }

    #[test]
    fn accepts_compact_json_and_uses_the_request_model() {
        let value = json!({"id":"resp_compact","usage":{"input_tokens":2,"output_tokens":3}});
        let parsed = parse_usage(value.as_object().unwrap(), "gpt-5.4").unwrap();
        assert_eq!(parsed.model, "gpt-5.4");
        assert_eq!(parsed.total_tokens, 5);
        assert_eq!(parsed.status, "completed");
    }

    #[tokio::test]
    async fn persists_and_aggregates_usage_with_response_deduplication() {
        let (store, path) = temporary_store();
        let event = UsageEvent {
            recorded_at_ms: current_time_ms().saturating_sub(1),
            identity_type: "api_key".into(),
            identity_id: "key-1".into(),
            identity_name: "laptop".into(),
            codex_account_id: String::new(),
            codex_account_name: String::new(),
            account_group_id: String::new(),
            account_group_name: String::new(),
            model: "gpt-5.6-sol".into(),
            transport: "http".into(),
            endpoint: "/v1/responses".into(),
            status: "completed".into(),
            response_id: "resp_unique".into(),
            input_tokens: 10,
            cached_input_tokens: 4,
            cache_creation_input_tokens: 0,
            output_tokens: 5,
            reasoning_output_tokens: 2,
            total_tokens: 15,
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
        assert_eq!(dashboard.identities[0].identity_name, "laptop");
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
        for (recorded_at_ms, response_id) in
            [(start_at, "resp_cycle_start"), (end_at, "resp_next_cycle")]
        {
            store
                .insert(UsageEvent {
                    recorded_at_ms,
                    identity_type: "api_key".into(),
                    identity_id: "key-1".into(),
                    identity_name: "laptop".into(),
                    codex_account_id: String::new(),
                    codex_account_name: String::new(),
                    account_group_id: String::new(),
                    account_group_name: String::new(),
                    model: "gpt-test".into(),
                    transport: "http".into(),
                    endpoint: "/v1/responses".into(),
                    status: "completed".into(),
                    response_id: response_id.into(),
                    input_tokens: 1_000_000,
                    cached_input_tokens: 400_000,
                    cache_creation_input_tokens: 100_000,
                    output_tokens: 100_000,
                    reasoning_output_tokens: 0,
                    total_tokens: 1_100_000,
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
            )
            .await
            .unwrap();

        assert_eq!(dashboard.start_at, start_at);
        assert_eq!(dashboard.end_at, end_at);
        assert_eq!(dashboard.series.len(), 7 * 24);
        assert_eq!(dashboard.totals.requests, 1);
        assert_eq!(dashboard.recent_events[0].recorded_at, start_at);
        assert!((dashboard.totals.cost_usd - 4.7).abs() < 0.000_001);
        assert!((dashboard.models[0].totals.cost_usd - 4.7).abs() < 0.000_001);
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
                identity_type: "api_key".into(),
                identity_id: "key-1".into(),
                identity_name: "laptop".into(),
                codex_account_id: "codex-1".into(),
                codex_account_name: "personal".into(),
                account_group_id: "group-1".into(),
                account_group_name: "pool".into(),
                model: "gpt-5.6-sol".into(),
                transport: "http".into(),
                endpoint: "/v1/responses".into(),
                status: "completed".into(),
                response_id: "resp_filter_key".into(),
                input_tokens: 10,
                cached_input_tokens: 2,
                cache_creation_input_tokens: 0,
                output_tokens: 5,
                reasoning_output_tokens: 1,
                total_tokens: 15,
            },
            UsageEvent {
                recorded_at_ms: now,
                identity_type: "auth_proxy".into(),
                identity_id: "account-1".into(),
                identity_name: "production".into(),
                codex_account_id: "codex-2".into(),
                codex_account_name: "team".into(),
                account_group_id: "group-1".into(),
                account_group_name: "pool".into(),
                model: "gpt-5.6-terra".into(),
                transport: "websocket".into(),
                endpoint: "/v1/responses".into(),
                status: "completed".into(),
                response_id: "resp_filter_account".into(),
                input_tokens: 60,
                cached_input_tokens: 20,
                cache_creation_input_tokens: 4,
                output_tokens: 30,
                reasoning_output_tokens: 12,
                total_tokens: 90,
            },
        ] {
            store.insert(event).await.unwrap();
        }

        let filter = UsageIdentityFilter::parse_downstream("auth_proxy", "account-1").unwrap();
        let dashboard = store
            .dashboard(UsageRange::Week, UsageFilters::new(Some(filter), None))
            .await
            .unwrap();

        assert_eq!(dashboard.totals.requests, 1);
        assert_eq!(dashboard.totals.total_tokens, 90);
        assert_eq!(dashboard.models.len(), 1);
        assert_eq!(dashboard.models[0].model, "gpt-5.6-terra");
        assert_eq!(dashboard.identities.len(), 1);
        assert_eq!(dashboard.identities[0].identity_id, "account-1");
        assert_eq!(dashboard.recent_events.len(), 1);
        assert_eq!(dashboard.recent_events[0].identity_type, "auth_proxy");
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

        let downstream_filter = UsageIdentityFilter::parse_downstream("api_key", "key-1").unwrap();
        let combined_dashboard = store
            .dashboard(
                UsageRange::Week,
                UsageFilters::new(Some(downstream_filter), Some(account_filter)),
            )
            .await
            .unwrap();
        assert_eq!(combined_dashboard.totals.requests, 1);
        assert_eq!(combined_dashboard.recent_events[0].identity_id, "key-1");
        assert_eq!(
            combined_dashboard.recent_events[0].codex_account_id,
            "codex-1"
        );

        assert!(UsageIdentityFilter::parse_downstream("codex_account", "codex-1").is_none());
        assert!(UsageIdentityFilter::parse_upstream("api_key", "key-1").is_none());
        assert!(UsageIdentityFilter::parse_downstream("unknown", "account-1").is_none());
        assert!(UsageIdentityFilter::parse_downstream("api_key", "").is_none());
        drop(store);
        remove_temporary_store(&path);
    }

    #[tokio::test]
    async fn websocket_tracker_records_multiple_responses_and_request_models() {
        let (store, path) = temporary_store();
        let tracker = UsageTracker::websocket(store.clone(), identity(), "/v1/responses");

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

    #[tokio::test]
    async fn sse_wire_observer_handles_split_terminal_events() {
        let (store, path) = temporary_store();
        let tracker = UsageTracker::http(store.clone(), identity(), "/v1/responses");
        tracker.set_requested_model("gpt-5.4");
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

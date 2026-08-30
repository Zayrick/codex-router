use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::{
    auth::{AuthProxyAccount, ClientApiKey},
    protocol::openai,
};

use super::oauth::current_time_ms;

const SCHEMA_VERSION: i64 = 1;
const MAX_TRACKED_JSON_BYTES: usize = 16 * 1024 * 1024;
const MAX_TRACKED_RESPONSES_PER_SOCKET: usize = 4_096;
const RECENT_EVENT_LIMIT: i64 = 50;
const BREAKDOWN_LIMIT: i64 = 12;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS usage_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    recorded_at_ms INTEGER NOT NULL,
    identity_type TEXT NOT NULL,
    identity_id TEXT NOT NULL,
    identity_name TEXT NOT NULL,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageIdentity {
    pub kind: UsageIdentityKind,
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageIdentityKind {
    ApiKey,
    AuthProxy,
}

impl UsageIdentityKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ApiKey => "api_key",
            Self::AuthProxy => "auth_proxy",
        }
    }
}

impl From<&ClientApiKey> for UsageIdentity {
    fn from(value: &ClientApiKey) -> Self {
        Self {
            kind: UsageIdentityKind::ApiKey,
            id: value.id.clone(),
            name: value.name.clone(),
        }
    }
}

impl From<&AuthProxyAccount> for UsageIdentity {
    fn from(value: &AuthProxyAccount) -> Self {
        Self {
            kind: UsageIdentityKind::AuthProxy,
            id: value.id.clone(),
            name: value.name.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UsageEvent {
    recorded_at_ms: i64,
    identity_type: String,
    identity_id: String,
    identity_name: String,
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
        connection
            .pragma_update(None, "user_version", SCHEMA_VERSION)
            .context("failed to set usage database schema version")?;
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
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::warn!(
                event = "usage_record",
                status = "skipped",
                reason = "no_runtime"
            );
            return;
        };
        runtime.spawn(async move {
            if let Err(error) = store.insert(event).await {
                tracing::warn!(event = "usage_record", status = "failed", error = %error);
            }
        });
    }

    async fn insert(&self, event: UsageEvent) -> Result<()> {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let connection = connection
                .lock()
                .map_err(|_| anyhow!("usage database lock is poisoned"))?;
            connection.execute(
                r#"
                INSERT OR IGNORE INTO usage_events (
                    recorded_at_ms, identity_type, identity_id, identity_name,
                    model, transport, endpoint, status, response_id,
                    input_tokens, cached_input_tokens, cache_creation_input_tokens,
                    output_tokens, reasoning_output_tokens, total_tokens
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
                "#,
                params![
                    event.recorded_at_ms,
                    event.identity_type,
                    event.identity_id,
                    event.identity_name,
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

    pub async fn dashboard(&self, range: UsageRange) -> Result<UsageDashboard> {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let connection = connection
                .lock()
                .map_err(|_| anyhow!("usage database lock is poisoned"))?;
            query_dashboard(&connection, range)
        })
        .await
        .context("usage database query task failed")?
    }
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
    allow_multiple: bool,
    terminal_seen: AtomicBool,
    seen_responses: Mutex<HashSet<String>>,
}

impl UsageTracker {
    pub fn http(store: UsageStore, identity: UsageIdentity, endpoint: impl Into<String>) -> Self {
        Self::new(store, identity, endpoint, "http", false)
    }

    pub fn websocket(
        store: UsageStore,
        identity: UsageIdentity,
        endpoint: impl Into<String>,
    ) -> Self {
        Self::new(store, identity, endpoint, "websocket", true)
    }

    fn new(
        store: UsageStore,
        identity: UsageIdentity,
        endpoint: impl Into<String>,
        transport: &'static str,
        allow_multiple: bool,
    ) -> Self {
        Self {
            inner: Arc::new(UsageTrackerInner {
                store,
                identity,
                endpoint: endpoint.into(),
                transport,
                requested_model: Mutex::new(String::new()),
                allow_multiple,
                terminal_seen: AtomicBool::new(false),
                seen_responses: Mutex::new(HashSet::new()),
            }),
        }
    }

    pub fn observe_request_value(&self, value: &Value) {
        let Some(model) = request_model(value) else {
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
        if let Ok(mut current) = self.inner.requested_model.lock() {
            *current = model.to_owned();
        }
    }

    pub fn observe_response_value(&self, value: &Value) {
        let fallback_model = self
            .inner
            .requested_model
            .lock()
            .ok()
            .map(|model| model.clone())
            .unwrap_or_default();
        let Some(parsed) = parse_usage(value, &fallback_model) else {
            return;
        };
        if !self.claim_terminal(&parsed, value) {
            return;
        }
        self.inner.store.record_background(UsageEvent {
            recorded_at_ms: current_time_ms(),
            identity_type: self.inner.identity.kind.as_str().into(),
            identity_id: self.inner.identity.id.clone(),
            identity_name: self.inner.identity.name.clone(),
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

    fn claim_terminal(&self, parsed: &ParsedUsage, value: &Value) -> bool {
        if !self.inner.allow_multiple {
            return !self.inner.terminal_seen.swap(true, Ordering::AcqRel);
        }
        let key = if parsed.response_id.is_empty() {
            format!("payload:{value}")
        } else {
            format!("response:{}", parsed.response_id)
        };
        let Ok(mut seen) = self.inner.seen_responses.lock() else {
            return false;
        };
        if seen.len() >= MAX_TRACKED_RESPONSES_PER_SOCKET {
            seen.clear();
        }
        seen.insert(key)
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
                            self.tracker.observe_response_value(&Value::Object(event));
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
                        self.tracker.observe_response_value(&Value::Object(event));
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

fn request_model(value: &Value) -> Option<&str> {
    let object = value.as_object()?;
    let kind = object.get("type").and_then(Value::as_str);
    if kind.is_some_and(|kind| !matches!(kind, "response.create" | "response.append")) {
        return None;
    }
    string(object, "model").or_else(|| {
        object
            .get("response")
            .and_then(Value::as_object)
            .and_then(|response| string(response, "model"))
    })
}

fn parse_usage(value: &Value, fallback_model: &str) -> Option<ParsedUsage> {
    let root = value.as_object()?;
    let kind = string(root, "type");
    let terminal = match kind {
        Some("response.completed" | "response.done") => "completed",
        Some("response.incomplete") => "incomplete",
        Some("response.failed" | "error") => "failed",
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
    if usage.is_none() && terminal != "failed" {
        return None;
    }
    let empty = Map::new();
    let usage = usage.unwrap_or(&empty);
    let input_tokens = token(usage, "input_tokens")
        .or_else(|| token(usage, "prompt_tokens"))
        .unwrap_or(0);
    let output_tokens = token(usage, "output_tokens")
        .or_else(|| token(usage, "completion_tokens"))
        .unwrap_or(0);
    let cached_input_tokens = usage
        .get("input_tokens_details")
        .or_else(|| usage.get("prompt_tokens_details"))
        .and_then(Value::as_object)
        .and_then(|details| {
            token(details, "cached_tokens").or_else(|| token(details, "cache_read_tokens"))
        })
        .unwrap_or(0);
    let cache_creation_input_tokens = usage
        .get("input_tokens_details")
        .or_else(|| usage.get("prompt_tokens_details"))
        .and_then(Value::as_object)
        .and_then(|details| {
            token(details, "cache_creation_tokens").or_else(|| token(details, "cache_write_tokens"))
        })
        .unwrap_or(0);
    let reasoning_output_tokens = usage
        .get("output_tokens_details")
        .or_else(|| usage.get("completion_tokens_details"))
        .and_then(Value::as_object)
        .and_then(|details| token(details, "reasoning_tokens"))
        .unwrap_or(0);
    let total_tokens = token(usage, "total_tokens")
        .filter(|total| *total > 0)
        .unwrap_or_else(|| input_tokens.saturating_add(output_tokens));
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
    let value = object.get(key)?;
    value.as_i64().filter(|value| *value >= 0).or_else(|| {
        value
            .as_u64()
            .map(|value| i64::try_from(value).unwrap_or(i64::MAX))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageRange {
    Day,
    Week,
    Month,
    All,
}

impl UsageRange {
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("24h") => Self::Day,
            Some("30d") => Self::Month,
            Some("all") => Self::All,
            _ => Self::Week,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Day => "24h",
            Self::Week => "7d",
            Self::Month => "30d",
            Self::All => "all",
        }
    }

    const fn duration_ms(self) -> Option<i64> {
        const DAY: i64 = 86_400_000;
        match self {
            Self::Day => Some(DAY),
            Self::Week => Some(7 * DAY),
            Self::Month => Some(30 * DAY),
            Self::All => None,
        }
    }

    const fn bucket_ms(self) -> i64 {
        if matches!(self, Self::Day) {
            3_600_000
        } else {
            86_400_000
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
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
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsageTotals {
    pub requests: i64,
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_output_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsageSeriesPoint {
    pub start_at: i64,
    pub requests: i64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsageBreakdownRow {
    pub model: String,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsageIdentityRow {
    pub identity_type: String,
    pub identity_id: String,
    pub identity_name: String,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsageEventRow {
    pub id: i64,
    pub recorded_at: i64,
    pub identity_type: String,
    pub identity_id: String,
    pub identity_name: String,
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
}

fn query_dashboard(connection: &Connection, range: UsageRange) -> Result<UsageDashboard> {
    let end_at = current_time_ms();
    let earliest = connection
        .query_row("SELECT MIN(recorded_at_ms) FROM usage_events", [], |row| {
            row.get::<_, Option<i64>>(0)
        })
        .optional()?
        .flatten();
    let start_at = range
        .duration_ms()
        .map(|duration| end_at.saturating_sub(duration))
        .or(earliest)
        .unwrap_or(end_at);
    let totals = query_totals(connection, start_at, end_at)?;
    let series = query_series(connection, range, start_at, end_at)?;
    let models = query_models(connection, start_at, end_at)?;
    let identities = query_identities(connection, start_at, end_at)?;
    let recent_events = query_recent_events(connection, start_at, end_at)?;
    Ok(UsageDashboard {
        range: range.label().into(),
        start_at,
        end_at,
        totals,
        series,
        models,
        identities,
        recent_events,
    })
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

fn query_totals(connection: &Connection, start_at: i64, end_at: i64) -> Result<UsageTotals> {
    let sql = format!(
        "SELECT {TOTAL_COLUMNS} FROM usage_events WHERE recorded_at_ms >= ?1 AND recorded_at_ms <= ?2"
    );
    Ok(connection.query_row(&sql, params![start_at, end_at], totals_from_row)?)
}

fn query_models(
    connection: &Connection,
    start_at: i64,
    end_at: i64,
) -> Result<Vec<UsageBreakdownRow>> {
    let sql = format!(
        "SELECT model, {TOTAL_COLUMNS} FROM usage_events \
         WHERE recorded_at_ms >= ?1 AND recorded_at_ms <= ?2 \
         GROUP BY model ORDER BY SUM(total_tokens) DESC, COUNT(*) DESC, model LIMIT ?3"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params![start_at, end_at, BREAKDOWN_LIMIT], |row| {
        Ok(UsageBreakdownRow {
            model: row.get(0)?,
            totals: totals_from_row_at(row, 1)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn query_identities(
    connection: &Connection,
    start_at: i64,
    end_at: i64,
) -> Result<Vec<UsageIdentityRow>> {
    let sql = format!(
        "SELECT identity_type, identity_id, identity_name, {TOTAL_COLUMNS} FROM usage_events \
         WHERE recorded_at_ms >= ?1 AND recorded_at_ms <= ?2 \
         GROUP BY identity_type, identity_id, identity_name \
         ORDER BY SUM(total_tokens) DESC, COUNT(*) DESC, identity_name LIMIT ?3"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params![start_at, end_at, BREAKDOWN_LIMIT], |row| {
        Ok(UsageIdentityRow {
            identity_type: row.get(0)?,
            identity_id: row.get(1)?,
            identity_name: row.get(2)?,
            totals: totals_from_row_at(row, 3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn query_series(
    connection: &Connection,
    range: UsageRange,
    start_at: i64,
    end_at: i64,
) -> Result<Vec<UsageSeriesPoint>> {
    let bucket_ms = range.bucket_ms();
    let aligned_start = start_at - start_at.rem_euclid(bucket_ms);
    let mut statement = connection.prepare(
        r#"
        SELECT ((recorded_at_ms - ?1) / ?2), COUNT(*), COALESCE(SUM(total_tokens), 0)
        FROM usage_events
        WHERE recorded_at_ms >= ?3 AND recorded_at_ms <= ?4
        GROUP BY 1 ORDER BY 1
        "#,
    )?;
    let rows = statement.query_map(params![aligned_start, bucket_ms, start_at, end_at], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    let populated = rows
        .map(|row| row.map(|(index, requests, tokens)| (index, (requests, tokens))))
        .collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
    let last_index = (end_at - aligned_start).div_euclid(bucket_ms);
    let first_index = (start_at - aligned_start).div_euclid(bucket_ms);
    let first_index = if range == UsageRange::All {
        populated.keys().next().copied().unwrap_or(first_index)
    } else {
        first_index
    };
    Ok((first_index..=last_index)
        .map(|index| {
            let (requests, total_tokens) = populated.get(&index).copied().unwrap_or_default();
            UsageSeriesPoint {
                start_at: aligned_start.saturating_add(index.saturating_mul(bucket_ms)),
                requests,
                total_tokens,
            }
        })
        .collect())
}

fn query_recent_events(
    connection: &Connection,
    start_at: i64,
    end_at: i64,
) -> Result<Vec<UsageEventRow>> {
    let mut statement = connection.prepare(
        r#"
        SELECT id, recorded_at_ms, identity_type, identity_id, identity_name,
               model, transport, endpoint, status, input_tokens, cached_input_tokens,
               cache_creation_input_tokens, output_tokens, reasoning_output_tokens, total_tokens
        FROM usage_events
        WHERE recorded_at_ms >= ?1 AND recorded_at_ms <= ?2
        ORDER BY recorded_at_ms DESC, id DESC LIMIT ?3
        "#,
    )?;
    let rows = statement.query_map(params![start_at, end_at, RECENT_EVENT_LIMIT], |row| {
        Ok(UsageEventRow {
            id: row.get(0)?,
            recorded_at: row.get(1)?,
            identity_type: row.get(2)?,
            identity_id: row.get(3)?,
            identity_name: row.get(4)?,
            model: row.get(5)?,
            transport: row.get(6)?,
            endpoint: row.get(7)?,
            status: row.get(8)?,
            input_tokens: row.get(9)?,
            cached_input_tokens: row.get(10)?,
            cache_creation_input_tokens: row.get(11)?,
            output_tokens: row.get(12)?,
            reasoning_output_tokens: row.get(13)?,
            total_tokens: row.get(14)?,
        })
    })?;
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
            let dashboard = store.dashboard(UsageRange::Week).await.unwrap();
            if dashboard.totals.requests == expected {
                return dashboard;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("usage events were not persisted before the test timeout");
    }

    #[test]
    fn parses_codex_terminal_usage_and_subset_details() {
        let parsed = parse_usage(
            &json!({
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
            }),
            "fallback",
        )
        .unwrap();
        assert_eq!(parsed.model, "gpt-5.6-sol");
        assert_eq!(parsed.input_tokens, 120);
        assert_eq!(parsed.cached_input_tokens, 80);
        assert_eq!(parsed.cache_creation_input_tokens, 4);
        assert_eq!(parsed.output_tokens, 30);
        assert_eq!(parsed.reasoning_output_tokens, 12);
        assert_eq!(parsed.total_tokens, 150);
    }

    #[test]
    fn accepts_compact_json_and_uses_the_request_model_fallback() {
        let parsed = parse_usage(
            &json!({"id":"resp_compact","usage":{"input_tokens":2,"output_tokens":3,"total_tokens":0}}),
            "gpt-5.4",
        )
        .unwrap();
        assert_eq!(parsed.model, "gpt-5.4");
        assert_eq!(parsed.total_tokens, 5);
        assert_eq!(parsed.status, "completed");
    }

    #[tokio::test]
    async fn persists_and_aggregates_usage_with_response_deduplication() {
        let (store, path) = temporary_store();
        let event = UsageEvent {
            recorded_at_ms: current_time_ms(),
            identity_type: "api_key".into(),
            identity_id: "key-1".into(),
            identity_name: "laptop".into(),
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
        let dashboard = store.dashboard(UsageRange::Week).await.unwrap();
        assert_eq!(dashboard.totals.requests, 1);
        assert_eq!(dashboard.totals.total_tokens, 15);
        assert_eq!(dashboard.models[0].model, "gpt-5.6-sol");
        assert_eq!(dashboard.identities[0].identity_name, "laptop");
        assert_eq!(dashboard.recent_events[0].transport, "http");
        drop(store);
        remove_temporary_store(&path);
    }

    #[tokio::test]
    async fn websocket_tracker_records_multiple_responses_and_request_models() {
        let (store, path) = temporary_store();
        let tracker = UsageTracker::websocket(store.clone(), identity(), "/v1/responses");

        tracker.observe_request_text(r#"{"type":"response.create","model":"gpt-5.6-sol"}"#);
        tracker.observe_response_text(
            r#"{"type":"response.completed","response":{"id":"resp_ws_1","usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}}}"#,
        );
        tracker.observe_response_text(
            r#"{"type":"response.completed","response":{"id":"resp_ws_1","usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}}}"#,
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

    #[test]
    fn identities_never_include_the_api_key_secret() {
        let key = ClientApiKey {
            id: "key-id".into(),
            name: "desktop".into(),
            key: "sk-secret-value-123!".into(),
            enabled: true,
        };
        let usage = UsageIdentity::from(&key);
        assert_eq!(
            usage,
            UsageIdentity {
                kind: UsageIdentityKind::ApiKey,
                id: "key-id".into(),
                name: "desktop".into(),
            }
        );
        assert!(!format!("{usage:?}").contains("secret"));
        let _ = identity();
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

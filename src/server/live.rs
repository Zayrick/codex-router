//! Live view for the console: the connections currently being proxied, and a
//! Server-Sent Events stream that pushes them together with change signals so
//! the console refetches only when its data actually changed.

use std::{
    collections::HashMap,
    convert::Infallible,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use axum::{
    body::Body,
    http::{HeaderValue, header},
    response::Response,
};
use bytes::Bytes;
use serde::Serialize;
use tokio::sync::watch;

use super::{oauth::current_time_ms, state::AppState};

/// Changes arriving within this window are sent as one snapshot.
const COALESCE: Duration = Duration::from_millis(500);
/// Keeps idle streams alive through proxies.
const HEARTBEAT: Duration = Duration::from_secs(15);
/// Streams end after this long so the browser reconnects and the session is
/// re-checked; a revoked administrator stops receiving data within this bound.
const MAX_STREAM_LIFETIME: Duration = Duration::from_secs(60);
const RECONNECT_MS: u64 = 3_000;

/// Registry of in-flight proxied connections.
#[derive(Clone)]
pub struct LiveHub {
    inner: Arc<HubInner>,
}

struct HubInner {
    next_id: AtomicU64,
    connections: Mutex<HashMap<u64, Arc<Mutex<LiveConnection>>>>,
    version: watch::Sender<u64>,
}

impl Default for LiveHub {
    fn default() -> Self {
        Self {
            inner: Arc::new(HubInner {
                next_id: AtomicU64::new(1),
                connections: Mutex::default(),
                version: watch::Sender::new(0),
            }),
        }
    }
}

/// One proxied connection: an HTTP request until its response body ends, or a
/// WebSocket until either side closes.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveConnection {
    pub id: u64,
    pub transport: &'static str,
    pub endpoint: String,
    pub user_id: String,
    pub user_name: String,
    pub credential_type: &'static str,
    pub credential_id: String,
    pub credential_name: String,
    pub codex_account_id: String,
    pub codex_account_name: String,
    pub account_group_id: String,
    pub account_group_name: String,
    pub client_ip: String,
    pub user_agent: String,
    pub model: String,
    pub started_at: i64,
    pub last_activity_at: i64,
    /// Whether a response is being generated right now (WebSockets idle between turns).
    pub active: bool,
    /// Responses started on this connection.
    pub requests: u64,
}

impl LiveHub {
    /// Registers a connection; it stays listed until the guard is dropped.
    pub fn register(&self, mut connection: LiveConnection) -> LiveConnectionGuard {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        connection.id = id;
        let entry = Arc::new(Mutex::new(connection));
        self.lock().insert(id, entry.clone());
        self.bump();
        LiveConnectionGuard {
            hub: self.clone(),
            id,
            entry,
        }
    }

    /// Connections ordered by start time, oldest first.
    pub fn snapshot(&self) -> Vec<LiveConnection> {
        let entries = self.lock().values().cloned().collect::<Vec<_>>();
        let mut connections = entries
            .iter()
            .map(|entry| entry.lock().unwrap_or_else(PoisonError::into_inner).clone())
            .collect::<Vec<_>>();
        connections.sort_by_key(|connection| (connection.started_at, connection.id));
        connections
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.inner.version.subscribe()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Arc<Mutex<LiveConnection>>>> {
        self.inner
            .connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn bump(&self) {
        self.inner
            .version
            .send_modify(|version| *version = version.wrapping_add(1));
    }
}

/// Removes its connection from the registry when dropped.
pub struct LiveConnectionGuard {
    hub: LiveHub,
    id: u64,
    entry: Arc<Mutex<LiveConnection>>,
}

impl LiveConnectionGuard {
    fn update(&self, change: impl FnOnce(&mut LiveConnection)) {
        {
            let mut connection = self.entry.lock().unwrap_or_else(PoisonError::into_inner);
            change(&mut connection);
            connection.last_activity_at = current_time_ms();
        }
        self.hub.bump();
    }

    pub fn response_started(&self, model: &str) {
        self.update(|connection| {
            connection.active = true;
            connection.requests += 1;
            if !model.is_empty() {
                connection.model = model.to_owned();
            }
        });
    }

    pub fn model_known(&self, model: &str) {
        if model.is_empty() {
            return;
        }
        let changed = self
            .entry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .model
            != model;
        if changed {
            self.update(|connection| connection.model = model.to_owned());
        }
    }

    pub fn response_finished(&self) {
        self.update(|connection| connection.active = false);
    }
}

impl Drop for LiveConnectionGuard {
    fn drop(&mut self) {
        self.hub.lock().remove(&self.id);
        self.hub.bump();
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LiveSnapshot {
    at: i64,
    /// Changes whenever a usage row is written; analytics refetch on it.
    usage_version: u64,
    /// Changes whenever configuration or stored state (accounts, quota, users) is written.
    state_version: u64,
    connections: Vec<LiveConnection>,
}

fn snapshot_frame(
    hub: &LiveHub,
    usage: &mut watch::Receiver<u64>,
    state: &mut watch::Receiver<u64>,
    connections: &mut watch::Receiver<u64>,
) -> Bytes {
    connections.mark_unchanged();
    let snapshot = LiveSnapshot {
        at: current_time_ms(),
        usage_version: *usage.borrow_and_update(),
        state_version: *state.borrow_and_update(),
        connections: hub.snapshot(),
    };
    let data = serde_json::to_string(&snapshot).unwrap_or_else(|_| "{}".into());
    Bytes::from(format!("event: snapshot\ndata: {data}\n\n"))
}

enum Wake {
    Changed,
    Heartbeat,
    End,
}

impl Wake {
    fn changed(result: Result<(), watch::error::RecvError>) -> Self {
        if result.is_ok() {
            Self::Changed
        } else {
            Self::End
        }
    }
}

/// `GET /admin/api/live`: an SSE stream of `snapshot` events, sent on connect
/// and after every change (coalesced), with heartbeats in between.
pub fn stream(state: &AppState) -> Response {
    let hub = state.live.clone();
    let mut usage = state.usage.subscribe();
    let mut stored = state.config.subscribe();
    let mut connections = hub.subscribe();
    let body = async_stream::stream! {
        yield Ok::<_, Infallible>(Bytes::from(format!("retry: {RECONNECT_MS}\n\n")));
        yield Ok(snapshot_frame(&hub, &mut usage, &mut stored, &mut connections));
        let deadline = tokio::time::Instant::now() + MAX_STREAM_LIFETIME;
        loop {
            let wake = tokio::select! {
                result = usage.changed() => Wake::changed(result),
                result = stored.changed() => Wake::changed(result),
                result = connections.changed() => Wake::changed(result),
                () = tokio::time::sleep(HEARTBEAT) => Wake::Heartbeat,
                () = tokio::time::sleep_until(deadline) => Wake::End,
            };
            match wake {
                Wake::Heartbeat => yield Ok(Bytes::from_static(b": ping\n\n")),
                Wake::Changed => {
                    tokio::time::sleep(COALESCE).await;
                    yield Ok(snapshot_frame(&hub, &mut usage, &mut stored, &mut connections));
                }
                Wake::End => break,
            }
        }
    };
    let mut output = Response::new(Body::from_stream(body));
    let headers = output.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    // Tells nginx not to buffer the stream.
    headers.insert("x-accel-buffering", HeaderValue::from_static("no"));
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection() -> LiveConnection {
        LiveConnection {
            id: 0,
            transport: "websocket",
            endpoint: "/v1/responses".into(),
            user_id: "user".into(),
            user_name: "User".into(),
            credential_type: "api_key",
            credential_id: "key".into(),
            credential_name: "Key".into(),
            codex_account_id: "account".into(),
            codex_account_name: "Account".into(),
            account_group_id: String::new(),
            account_group_name: String::new(),
            client_ip: "127.0.0.1".into(),
            user_agent: String::new(),
            model: String::new(),
            started_at: 1,
            last_activity_at: 1,
            active: false,
            requests: 0,
        }
    }

    #[test]
    fn lists_connections_until_their_guard_drops() {
        let hub = LiveHub::default();
        let mut changes = hub.subscribe();
        let first = hub.register(connection());
        let second = hub.register(connection());
        assert_eq!(hub.snapshot().len(), 2);
        assert!(changes.has_changed().unwrap());
        changes.mark_unchanged();

        first.response_started("gpt-5");
        let listed = hub.snapshot();
        assert!(listed[0].active);
        assert_eq!(listed[0].requests, 1);
        assert_eq!(listed[0].model, "gpt-5");
        assert!(changes.has_changed().unwrap());

        first.response_finished();
        assert!(!hub.snapshot()[0].active);

        drop(first);
        let listed = hub.snapshot();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, second.id);
        drop(second);
        assert!(hub.snapshot().is_empty());
    }
}

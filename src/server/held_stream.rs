//! Streams a Responses SSE body to the client with two capacity fixes.
//!
//! * **Hold until committed.** Response headers and the `response.created`
//!   preamble are buffered until the first real output. An overload that arrives
//!   before then has shown the client nothing, so the caller can replay the
//!   request on another account instead.
//! * **End at the terminal event.** Once a response has failed, completed or
//!   been overloaded, the body ends. Codex 0.167–0.171 only surface a stashed
//!   `response.failed` at EOF, and otherwise sit out their 300 s idle timeout.

use std::{pin::Pin, time::Duration};

use axum::{
    body::Body,
    http::{
        HeaderMap, StatusCode,
        header::{CONTENT_LENGTH, CONTENT_TYPE},
    },
    response::Response,
};
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use serde_json::{Map, Value};
use tokio::time::Instant;

use crate::protocol::openai::sse::{SseFrame, SseFrameDecoder};

use super::{
    account_health::is_overload_code,
    response::{proxy_headers, response_with_status, tracked_body_bytes},
    usage::{UsageTracker, UsageWireObserver, event_error_code},
};

/// Longest wait for the first output before the response is released anyway.
pub const DEFAULT_HOLD: Duration = Duration::from_secs(45);
/// Most bytes buffered while holding.
const MAX_HELD_BYTES: usize = 1024 * 1024;
/// Most bytes of an error body read to find its error code.
const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;
/// Longest wait for an error body.
const ERROR_BODY_TIMEOUT: Duration = Duration::from_secs(10);
/// After a bare `error` overload event, wait this long for its `response.failed`.
const TERMINAL_GRACE: Duration = Duration::from_secs(2);

type Chunk = reqwest::Result<Bytes>;
type Source = Pin<Box<dyn Stream<Item = Chunk> + Send>>;

/// How one streamed Responses event affects delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventClass {
    /// Bookkeeping before any output (`response.created`, Codex metadata).
    Preamble,
    /// The account had no capacity. `terminal` is false for a bare `error`
    /// event that may still be followed by `response.failed`.
    Overload { terminal: bool },
    /// The response is over.
    Terminal,
    /// Anything the client must see.
    Output,
}

pub fn classify_event(event: &Map<String, Value>) -> EventClass {
    let kind = event.get("type").and_then(Value::as_str);
    let failure = matches!(
        kind,
        Some("response.failed" | "response.error" | "error") | None
    ) || kind == Some("response.done")
        && event
            .get("response")
            .and_then(|response| response.get("status"))
            .and_then(Value::as_str)
            == Some("failed");
    if failure && is_overload_code(&event_error_code(event)) {
        return EventClass::Overload {
            terminal: kind != Some("error") && kind.is_some(),
        };
    }
    match kind {
        Some(
            "response.created" | "response.in_progress" | "response.queued" | "response.metadata",
        ) => EventClass::Preamble,
        Some(kind) if kind.starts_with("codex.") => EventClass::Preamble,
        Some(
            "response.completed"
            | "response.incomplete"
            | "response.failed"
            | "response.error"
            | "response.done",
        ) => EventClass::Terminal,
        _ => EventClass::Output,
    }
}

/// How the body is split into events.
enum Framing {
    /// No `Content-Type`: decided by the first non-blank byte, like usage tracking.
    Sniff,
    Sse(SseFrameDecoder),
    /// Not an event stream (or undecodable): passed through untouched.
    Opaque,
}

/// An upstream response whose head has been read but not yet sent.
pub struct HeldResponse {
    status: StatusCode,
    headers: HeaderMap,
    prefix: Vec<Chunk>,
    source: Option<Source>,
    /// Taken once the body has been fully observed.
    observer: Option<UsageWireObserver>,
    framing: Framing,
    /// The client-visible part of the response has started; never resets.
    seen_output: bool,
    /// The upstream reported an overload before any output.
    pub overloaded: bool,
    /// The held prefix already contains the terminal event.
    finished: bool,
    /// After a bare overload `error`, the body ends at this instant at the latest.
    overload_deadline: Option<Instant>,
}

impl HeldResponse {
    fn new(response: reqwest::Response, tracker: &UsageTracker) -> Self {
        let status = response.status();
        let mut headers = proxy_headers(response.headers());
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        // Same detection as usage tracking: event-stream by MIME, else sniffed.
        let framing = if content_type.as_deref().is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|media| media.trim().eq_ignore_ascii_case("text/event-stream"))
        }) {
            Framing::Sse(SseFrameDecoder::new())
        } else {
            Framing::Sniff
        };
        // An event stream may end early at its terminal event.
        headers.remove(CONTENT_LENGTH);
        tracker.observe_upstream_status(status.as_u16());
        Self {
            status,
            headers,
            prefix: Vec::new(),
            source: Some(Box::pin(tracked_body_bytes(response, tracker))),
            observer: Some(tracker.wire_observer(content_type.as_deref())),
            framing,
            seen_output: false,
            overloaded: false,
            finished: false,
            overload_deadline: None,
        }
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    fn observe(&mut self, bytes: &[u8]) {
        if let Some(observer) = self.observer.as_mut() {
            observer.push(bytes);
        }
    }

    /// Records the outcome of a body that has been read to the end.
    fn finish_observing(&mut self) {
        if let Some(observer) = self.observer.take() {
            observer.finish();
        }
    }

    /// Settles a sniffed body's framing from its first non-blank byte.
    fn sniff(&mut self, bytes: &[u8]) {
        if matches!(self.framing, Framing::Sniff)
            && let Some(first) = bytes.iter().find(|byte| !byte.is_ascii_whitespace())
        {
            self.framing = if matches!(first, b'{' | b'[') {
                Framing::Opaque
            } else {
                Framing::Sse(SseFrameDecoder::new())
            };
        }
    }

    fn decode(&mut self, bytes: &[u8]) -> Vec<Map<String, Value>> {
        self.sniff(bytes);
        let Framing::Sse(decoder) = &mut self.framing else {
            return Vec::new();
        };
        let Ok(frames) = decoder.push_bytes(bytes) else {
            self.framing = Framing::Opaque;
            return Vec::new();
        };
        frames
            .into_iter()
            .filter_map(|frame| match frame {
                SseFrame::Data(data) => Some(data),
                SseFrame::Done => None,
            })
            // A frame may hold several JSON objects; anything that is not JSON
            // is skipped rather than fatal.
            .flat_map(|data| {
                serde_json::Deserializer::from_str(&data)
                    .into_iter::<Value>()
                    .map_while(Result::ok)
                    .filter_map(|value| match value {
                        Value::Object(event) => Some(event),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Classifies the events in `bytes`; returns true when holding should stop.
    fn inspect(&mut self, bytes: &[u8]) -> bool {
        self.observe(bytes);
        let events = self.decode(bytes);
        if matches!(self.framing, Framing::Opaque) {
            return true;
        }
        let mut commit = false;
        for event in &events {
            // Nothing after the terminal event belongs to this response.
            if self.finished {
                break;
            }
            match classify_event(event) {
                EventClass::Preamble => {}
                EventClass::Overload { terminal } => {
                    // An overload after output cannot be replayed elsewhere.
                    self.overloaded |= !self.seen_output;
                    if terminal {
                        self.finished = true;
                    } else {
                        self.overload_deadline
                            .get_or_insert_with(|| Instant::now() + TERMINAL_GRACE);
                    }
                    commit = true;
                }
                EventClass::Terminal => {
                    self.finished = true;
                    commit = true;
                }
                EventClass::Output => {
                    self.seen_output = true;
                    commit = true;
                }
            }
        }
        commit
    }

    /// Sends the response to the client, ending it at the terminal event.
    pub fn release(mut self) -> Response {
        let status = self.status;
        let headers = std::mem::take(&mut self.headers);
        let prefix = std::mem::take(&mut self.prefix);
        let output = async_stream::stream! {
            for chunk in prefix {
                yield chunk;
            }
            if !self.finished {
                while let Some(mut source) = self.source.take() {
                    let next = match self.overload_deadline {
                        Some(deadline) if Instant::now() >= deadline => break,
                        Some(deadline) => match tokio::time::timeout_at(deadline, source.next()).await {
                            Ok(next) => next,
                            Err(_) => break,
                        },
                        None => source.next().await,
                    };
                    let Some(chunk) = next else {
                        break;
                    };
                    if let Ok(bytes) = &chunk {
                        self.inspect(bytes);
                    }
                    yield chunk;
                    if self.finished {
                        break;
                    }
                    self.source = Some(source);
                }
            }
            self.finish_observing();
        };
        response_with_status(status, headers, Body::from_stream(output))
    }
}

/// Reads the response until its first output, an overload, EOF, `hold` or the
/// buffer limit, whichever comes first. Error bodies are read (up to a limit)
/// so an overload reported as HTTP 503/429 is recognized too.
pub async fn hold(
    response: reqwest::Response,
    tracker: &UsageTracker,
    hold: Duration,
) -> HeldResponse {
    let mut held = HeldResponse::new(response, tracker);
    let success = held.status.is_success();
    if success && hold.is_zero() {
        return held;
    }
    let deadline = Instant::now() + if success { hold } else { ERROR_BODY_TIMEOUT };
    let mut buffered = 0;
    let Some(mut source) = held.source.take() else {
        return held;
    };
    let mut error_body = Vec::new();
    loop {
        let Ok(next) = tokio::time::timeout_at(deadline, source.next()).await else {
            break;
        };
        let Some(chunk) = next else {
            if !success {
                held.overloaded = error_body_is_overload(&error_body);
            }
            held.finish_observing();
            return held;
        };
        if let Ok(bytes) = &chunk {
            held.sniff(bytes);
        }
        let commit = match &chunk {
            // A JSON error body is read whole to find its error code.
            Ok(bytes) if !success && matches!(held.framing, Framing::Opaque | Framing::Sniff) => {
                held.observe(bytes);
                error_body.extend_from_slice(bytes);
                error_body.len() > MAX_ERROR_BODY_BYTES
            }
            Ok(bytes) => {
                buffered += bytes.len();
                held.inspect(bytes) || buffered >= MAX_HELD_BYTES
            }
            Err(_) => true,
        };
        held.prefix.push(chunk);
        if commit {
            break;
        }
    }
    held.source = Some(source);
    held
}

fn error_body_is_overload(body: &[u8]) -> bool {
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(object)) => is_overload_code(&event_error_code(&object)),
        _ => false,
    }
}

/// Streams `response` without holding, still ending at the terminal event.
pub fn release_tracked(response: reqwest::Response, tracker: &UsageTracker) -> Response {
    HeldResponse::new(response, tracker).release()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::body::to_bytes;
    use futures_util::stream;
    use serde_json::json;

    use super::*;
    use crate::server::{
        usage::{UsageIdentity, UsageStore},
        usage_client::UsageClient,
    };

    /// A tracker plus the (status, error code) of every attempt it finished.
    type Seen = Arc<Mutex<Vec<(String, String)>>>;

    fn tracker() -> (UsageTracker, Seen, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "codex-router-held-{}.sqlite3",
            uuid::Uuid::new_v4()
        ));
        let store = UsageStore::open(path.clone()).unwrap();
        let identity = UsageIdentity {
            user_id: "user".into(),
            user_name: "user".into(),
            credential_type: "api_key",
            credential_id: "key".into(),
            credential_name: "key".into(),
            codex_account_id: "account".into(),
            codex_account_name: "account".into(),
            account_group_id: String::new(),
            account_group_name: String::new(),
        };
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink_seen = seen.clone();
        let tracker = UsageTracker::http(store, identity, UsageClient::default(), "/responses")
            .with_outcome_sink(Arc::new(move |attempt| {
                sink_seen
                    .lock()
                    .unwrap()
                    .push((attempt.status.to_owned(), attempt.error_code.to_owned()));
            }));
        (tracker, seen, path)
    }

    /// An SSE response sending `events`, then staying open forever.
    fn open_ended_sse(events: &[Value]) -> reqwest::Response {
        let chunks = events
            .iter()
            .map(|event| Ok::<_, std::io::Error>(Bytes::from(format!("data: {event}\n\n"))))
            .collect::<Vec<_>>();
        let body = stream::iter(chunks).chain(stream::pending());
        reqwest::Response::from(
            axum::http::Response::builder()
                .status(200)
                .header("content-type", "text/event-stream")
                .header("x-codex-turn-state", "turn")
                .body(reqwest::Body::wrap_stream(body))
                .unwrap(),
        )
    }

    async fn body_text(response: Response) -> String {
        let bytes = tokio::time::timeout(
            Duration::from_secs(5),
            to_bytes(response.into_body(), usize::MAX),
        )
        .await
        .expect("the body must end without waiting for upstream EOF")
        .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn overload_before_output_is_held_back_and_reported() {
        let (tracker, seen, path) = tracker();
        let upstream = open_ended_sse(&[
            json!({"type":"response.created","response":{"id":"r1"}}),
            json!({"type":"response.failed","response":{"id":"r1","error":{"code":"server_is_overloaded"}}}),
        ]);
        let held = hold(upstream, &tracker, DEFAULT_HOLD).await;
        assert!(held.overloaded);
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            [("failed".to_owned(), "server_is_overloaded".to_owned())]
        );
        // Without another account the failure is delivered and the body ends.
        let text = body_text(held.release()).await;
        assert!(text.contains("response.created") && text.contains("server_is_overloaded"));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn bare_overload_error_ends_the_body_after_a_grace_period() {
        let (tracker, _, path) = tracker();
        let upstream = open_ended_sse(&[json!({"type":"error","code":"server_is_overloaded"})]);
        let held = hold(upstream, &tracker, DEFAULT_HOLD).await;
        assert!(held.overloaded);
        let text = body_text(held.release()).await;
        assert!(text.contains("server_is_overloaded"));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn output_commits_and_the_body_ends_at_the_terminal_event() {
        let (tracker, seen, path) = tracker();
        let upstream = open_ended_sse(&[
            json!({"type":"response.created","response":{"id":"r2"}}),
            json!({"type":"response.output_text.delta","delta":"hi"}),
            json!({"type":"response.completed","response":{"id":"r2","usage":{"input_tokens":1,"output_tokens":1}}}),
        ]);
        let held = hold(upstream, &tracker, DEFAULT_HOLD).await;
        assert!(!held.overloaded);
        let response = held.release();
        assert_eq!(
            response.headers().get("x-codex-turn-state").unwrap(),
            "turn"
        );
        let text = body_text(response).await;
        assert!(text.contains("\"delta\":\"hi\"") && text.contains("response.completed"));
        assert_eq!(seen.lock().unwrap()[0].0, "completed");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn slow_first_output_is_released_after_the_hold_limit() {
        let (tracker, _, path) = tracker();
        let upstream = open_ended_sse(&[json!({"type":"response.created","response":{"id":"r3"}})]);
        let held = hold(upstream, &tracker, Duration::from_millis(50)).await;
        assert!(!held.overloaded);
        assert_eq!(held.prefix.len(), 1);
        let _ = std::fs::remove_file(path);
    }

    fn class(value: Value) -> EventClass {
        classify_event(value.as_object().unwrap())
    }

    #[test]
    fn classifies_responses_events() {
        assert_eq!(
            class(json!({"type":"response.created","response":{"id":"r"}})),
            EventClass::Preamble
        );
        assert_eq!(
            class(json!({"type":"response.output_item.added"})),
            EventClass::Output
        );
        assert_eq!(
            class(json!({"type":"response.completed","response":{}})),
            EventClass::Terminal
        );
        assert_eq!(
            class(json!({"type":"response.failed","response":{"error":{"code":"server_error"}}})),
            EventClass::Terminal
        );
        assert_eq!(
            class(
                json!({"type":"response.failed","response":{"error":{"code":"server_is_overloaded"}}})
            ),
            EventClass::Overload { terminal: true }
        );
        assert_eq!(
            class(json!({"type":"error","code":"server_is_overloaded"})),
            EventClass::Overload { terminal: false }
        );
        assert_eq!(
            class(json!({"type":"error","status":503,"error":{"code":"slow_down"}})),
            EventClass::Overload { terminal: false }
        );
        assert_eq!(
            class(json!({"type":"error","error":{"code":"invalid_request"}})),
            EventClass::Output
        );
    }

    /// A response with the given raw chunks, optionally staying open afterwards.
    fn raw_response(
        status: u16,
        content_type: Option<&str>,
        chunks: Vec<&'static str>,
        open_ended: bool,
    ) -> reqwest::Response {
        let chunks = chunks
            .into_iter()
            .map(|chunk| Ok::<_, std::io::Error>(Bytes::from_static(chunk.as_bytes())))
            .collect::<Vec<_>>();
        let body = stream::iter(chunks);
        let body = if open_ended {
            reqwest::Body::wrap_stream(body.chain(stream::pending()))
        } else {
            reqwest::Body::wrap_stream(body)
        };
        let mut builder = axum::http::Response::builder()
            .status(status)
            .header("content-length", "100000");
        if let Some(content_type) = content_type {
            builder = builder.header("content-type", content_type);
        }
        reqwest::Response::from(builder.body(body).unwrap())
    }

    const SSE: Option<&str> = Some("text/event-stream");

    #[tokio::test]
    async fn output_before_an_overload_in_the_same_chunk_is_not_replayable() {
        let (tracker, _, path) = tracker();
        let upstream = raw_response(
            200,
            SSE,
            vec![concat!(
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"x\"}\n\n",
                "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"server_is_overloaded\"}}}\n\n",
            )],
            true,
        );
        let held = hold(upstream, &tracker, DEFAULT_HOLD).await;
        assert!(!held.overloaded);
        assert!(body_text(held.release()).await.contains("\"delta\":\"x\""));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn codex_metadata_stays_in_the_held_preamble() {
        let (tracker, _, path) = tracker();
        let upstream = raw_response(
            200,
            SSE,
            vec![
                "data: {\"type\":\"response.created\",\"response\":{\"id\":\"r\"}}\n\n",
                "data: {\"type\":\"codex.response.metadata\",\"headers\":{}}\n\n",
                "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"server_is_overloaded\"}}}\n\n",
            ],
            true,
        );
        assert!(hold(upstream, &tracker, DEFAULT_HOLD).await.overloaded);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn heartbeats_do_not_extend_the_overload_grace_period() {
        let (tracker, _, path) = tracker();
        let heartbeats = futures_util::stream::unfold((), |()| async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            Some((
                Ok::<_, std::io::Error>(Bytes::from_static(b": heartbeat\n\n")),
                (),
            ))
        });
        let body = stream::iter([Ok::<_, std::io::Error>(Bytes::from_static(
            b"data: {\"type\":\"error\",\"code\":\"server_is_overloaded\"}\n\n",
        ))])
        .chain(heartbeats);
        let upstream = reqwest::Response::from(
            axum::http::Response::builder()
                .status(200)
                .header("content-type", "text/event-stream")
                .body(reqwest::Body::wrap_stream(body))
                .unwrap(),
        );
        let held = hold(upstream, &tracker, DEFAULT_HOLD).await;
        let started = Instant::now();
        body_text(held.release()).await;
        assert!(started.elapsed() < TERMINAL_GRACE + Duration::from_millis(500));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn a_non_json_event_does_not_disable_terminal_detection() {
        let (tracker, _, path) = tracker();
        let upstream = raw_response(
            200,
            None,
            vec![
                "event: notice\ndata: maintenance\n\n",
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"x\"}\n\n",
                "data: {\"type\":\"response.completed\",\"response\":{}}\n\n",
            ],
            true,
        );
        let response = hold(upstream, &tracker, DEFAULT_HOLD).await.release();
        assert!(response.headers().get("content-length").is_none());
        assert!(body_text(response).await.contains("response.completed"));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn an_http_503_overload_is_replayable() {
        let (tracker, seen, path) = tracker();
        let upstream = raw_response(
            503,
            Some("application/json"),
            vec!["{\"error\":{\"code\":\"server_is_overloaded\",\"message\":\"busy\"}}"],
            false,
        );
        let held = hold(upstream, &tracker, DEFAULT_HOLD).await;
        assert!(held.overloaded);
        assert_eq!(seen.lock().unwrap()[0].1, "server_is_overloaded");
        // Without another account the error is delivered unchanged.
        let response = held.release();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(body_text(response).await.contains("busy"));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn other_http_errors_are_not_replayed() {
        let (tracker, _, path) = tracker();
        let upstream = raw_response(
            429,
            Some("application/json"),
            vec!["{\"error\":{\"type\":\"usage_limit_reached\"}}"],
            false,
        );
        assert!(!hold(upstream, &tracker, DEFAULT_HOLD).await.overloaded);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn events_after_the_terminal_event_are_ignored() {
        let (tracker, _, path) = tracker();
        let upstream = raw_response(
            200,
            SSE,
            vec![concat!(
                "data: {\"type\":\"response.created\",\"response\":{}}\n\n",
                "data: {\"type\":\"response.completed\",\"response\":{}}\n\n",
                "data: {\"type\":\"error\",\"code\":\"server_is_overloaded\"}\n\n",
            )],
            true,
        );
        let held = hold(upstream, &tracker, DEFAULT_HOLD).await;
        assert!(!held.overloaded);
        body_text(held.release()).await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn an_error_status_event_stream_still_ends_at_its_terminal_event() {
        let (tracker, _, path) = tracker();
        let upstream = raw_response(
            503,
            SSE,
            vec![
                "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"server_error\"}}}\n\n",
            ],
            true,
        );
        let held = hold(upstream, &tracker, DEFAULT_HOLD).await;
        assert!(!held.overloaded);
        assert!(body_text(held.release()).await.contains("server_error"));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn plain_text_streams_with_several_objects_per_frame_are_understood() {
        let (tracker, _, path) = tracker();
        let upstream = raw_response(
            200,
            Some("text/plain"),
            vec![concat!(
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"x\"}",
                "{\"type\":\"response.done\",\"response\":{\"status\":\"completed\"}}\n\n",
            )],
            true,
        );
        let response = hold(upstream, &tracker, DEFAULT_HOLD).await.release();
        assert!(body_text(response).await.contains("response.done"));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn an_unheld_error_body_is_still_classified() {
        let (tracker, _, path) = tracker();
        let upstream = raw_response(
            429,
            Some("application/json"),
            vec!["{\"error\":{\"code\":\"slow_down\"}}"],
            false,
        );
        assert!(hold(upstream, &tracker, Duration::ZERO).await.overloaded);
        let _ = std::fs::remove_file(path);
    }
}

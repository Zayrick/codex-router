use axum::{
    body::Body,
    extract::ws::{
        CloseFrame as AxumCloseFrame, Message as AxumMessage, WebSocket, WebSocketUpgrade,
    },
    http::{HeaderName, HeaderValue, StatusCode},
    response::Response,
};
use std::{
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Duration,
};

use tokio::time::Instant;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, client_async_tls, connect_async,
    tungstenite::{
        Error as TungsteniteError, Message as UpstreamMessage, client::IntoClientRequest,
        protocol::CloseFrame as UpstreamCloseFrame,
    },
};
use url::Url;

use crate::{
    core::{ApiError, AppResult},
    protocol::openai::responses::adapt_responses_websocket_message,
    upstream::codex::HeaderBag,
};

use super::{
    chatgpt_proxy::ChatgptProxy,
    config::WebsocketIdleTimeout,
    held_stream::{EventClass, classify_event},
    response,
    usage::UsageTracker,
};

/// Rechecks the idle timeout at least this often, so setting changes apply.
const IDLE_RECHECK: Duration = Duration::from_secs(30);
/// Close code sent to both sides when the proxy ends a WebSocket; reasons below.
const NORMAL_CLOSE_CODE: u16 = 1000;
const IDLE_CLOSE_REASON: &str = "idle timeout";
/// Sent after an upstream overload. Clients that missed the error frame retry on
/// a new connection, which routing then places on a healthier account.
const OVERLOAD_CLOSE_REASON: &str = "upstream overloaded";
/// After a bare overload `error` frame, wait this long for its terminal frame.
const OVERLOAD_GRACE: Duration = Duration::from_secs(2);

/// How a proxied WebSocket treats Responses traffic.
#[derive(Debug, Clone, Copy, Default)]
pub struct ResponsesOptions {
    /// Rewrite `response.create` messages for the Codex backend.
    pub adapt: bool,
    /// Close after an upstream overload so the client reconnects; only useful when
    /// a reconnect can be routed to another account.
    pub close_on_overload: bool,
}

pub async fn proxy(
    upgrade: WebSocketUpgrade,
    target: Url,
    headers: HeaderBag,
    options: ResponsesOptions,
    tracker: Option<UsageTracker>,
    proxy: Option<&ChatgptProxy>,
    idle_timeout: WebsocketIdleTimeout,
) -> AppResult<Response> {
    let result = connect(
        upgrade,
        target,
        headers,
        options,
        tracker.clone(),
        proxy,
        idle_timeout,
    )
    .await;
    if let Some(tracker) = &tracker {
        match &result {
            Ok(response) if response.status() != StatusCode::SWITCHING_PROTOCOLS => {
                tracker.observe_upstream_status(response.status().as_u16());
            }
            Ok(_) => {}
            Err(error) => tracker.fail(error),
        }
    }
    result
}

async fn connect(
    upgrade: WebSocketUpgrade,
    mut target: Url,
    headers: HeaderBag,
    options: ResponsesOptions,
    tracker: Option<UsageTracker>,
    proxy: Option<&ChatgptProxy>,
    idle_timeout: WebsocketIdleTimeout,
) -> AppResult<Response> {
    match target.scheme() {
        "https" => target
            .set_scheme("wss")
            .map_err(|_| websocket_unavailable())?,
        "http" => target
            .set_scheme("ws")
            .map_err(|_| websocket_unavailable())?,
        "wss" | "ws" => {}
        _ => return Err(websocket_unavailable()),
    }
    let mut request = target
        .as_str()
        .into_client_request()
        .map_err(|_| websocket_unavailable())?;
    for (name, value) in headers.iter() {
        if name == "upgrade" {
            continue;
        }
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| websocket_unavailable())?;
        let value = HeaderValue::from_str(value).map_err(|_| websocket_unavailable())?;
        request.headers_mut().append(name, value);
    }

    let connection = if let Some(proxy) = proxy {
        let stream = proxy
            .connect(&target)
            .await
            .map_err(|_| websocket_unavailable())?;
        client_async_tls(request, stream).await
    } else {
        connect_async(request).await
    };
    let (upstream, handshake) = match connection {
        Ok(result) => result,
        Err(TungsteniteError::Http(rejection)) => {
            return Ok(rejection_response(*rejection));
        }
        Err(_) => return Err(websocket_unavailable()),
    };
    let selected_protocol = handshake
        .headers()
        .get("sec-websocket-protocol")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let upgrade = if let Some(protocol) = selected_protocol {
        upgrade.protocols([protocol])
    } else {
        upgrade
    };
    Ok(upgrade.on_upgrade(move |client| bridge(client, upstream, options, tracker, idle_timeout)))
}

/// Time of the last data message in either direction.
struct Activity {
    origin: Instant,
    last_ms: AtomicU64,
}

impl Activity {
    fn new() -> Self {
        Self {
            origin: Instant::now(),
            last_ms: AtomicU64::new(0),
        }
    }

    fn touch(&self) {
        let elapsed = u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.last_ms.store(elapsed, Ordering::Relaxed);
    }

    fn idle_for(&self) -> Duration {
        self.origin
            .elapsed()
            .saturating_sub(Duration::from_millis(self.last_ms.load(Ordering::Relaxed)))
    }
}

/// Resolves once the connection has been idle for the configured timeout with
/// no response in progress; never resolves while the timeout is disabled.
async fn idle_expired(
    activity: &Activity,
    timeout: &WebsocketIdleTimeout,
    tracker: Option<&UsageTracker>,
) {
    loop {
        let wait = match timeout.get() {
            None => IDLE_RECHECK,
            Some(limit) => {
                let idle = activity.idle_for();
                let generating = tracker.is_some_and(UsageTracker::response_in_progress);
                if idle >= limit && !generating {
                    return;
                }
                limit
                    .saturating_sub(idle)
                    .clamp(Duration::from_secs(1), IDLE_RECHECK)
            }
        };
        tokio::time::sleep(wait).await;
    }
}

fn is_data(message: &AxumMessage) -> bool {
    matches!(message, AxumMessage::Text(_) | AxumMessage::Binary(_))
}

enum Ended {
    Client,
    Upstream,
    Idle,
    Overloaded,
}

/// Overload class of an upstream frame. Frames are only parsed when they could
/// be an overload, or while waiting for the terminal frame after one.
fn overload_class(message: &UpstreamMessage, after_overload: bool) -> Option<EventClass> {
    let UpstreamMessage::Text(text) = message else {
        return None;
    };
    let text = text.as_str();
    if !after_overload && !text.contains("overloaded") && !text.contains("slow_down") {
        return None;
    }
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(serde_json::Value::Object(object)) => Some(classify_event(&object)),
        _ => None,
    }
}

async fn bridge(
    client: WebSocket,
    upstream: WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    options: ResponsesOptions,
    tracker: Option<UsageTracker>,
    idle_timeout: WebsocketIdleTimeout,
) {
    let (mut client_sender, mut client_receiver) = client.split();
    let (mut upstream_sender, mut upstream_receiver) = upstream.split();
    let activity = Activity::new();

    let request_tracker = tracker.clone();
    // Set after an overload: the connection is about to close, so new requests
    // are not started upstream (the client retries them on a new connection).
    let closing = AtomicBool::new(false);
    let to_upstream = async {
        while let Some(message) = client_receiver.next().await {
            let Ok(message) = message else {
                break;
            };
            if is_data(&message) {
                if closing.load(Ordering::Relaxed) {
                    continue;
                }
                activity.touch();
            }
            let message = client_message(message, options.adapt, request_tracker.as_ref());
            if upstream_sender.send(message).await.is_err() {
                break;
            }
        }
        let _ = upstream_sender.close().await;
    };
    let to_client = async {
        // After a bare overload `error`, wait briefly for the response's own
        // terminal frame (which may carry a retry hint), then close.
        let mut deadline: Option<tokio::time::Instant> = None;
        loop {
            let next = match deadline {
                Some(deadline) => {
                    match tokio::time::timeout_at(deadline, upstream_receiver.next()).await {
                        Ok(next) => next,
                        Err(_) => return true,
                    }
                }
                None => upstream_receiver.next().await,
            };
            let Some(Ok(message)) = next else {
                break;
            };
            let class = options
                .close_on_overload
                .then(|| overload_class(&message, deadline.is_some()))
                .flatten();
            let Some(message) = upstream_message(message, tracker.as_ref()) else {
                continue;
            };
            if is_data(&message) {
                activity.touch();
            }
            if client_sender.send(message).await.is_err() {
                break;
            }
            match class {
                Some(EventClass::Overload { terminal: true }) => return true,
                Some(EventClass::Terminal) if deadline.is_some() => return true,
                Some(EventClass::Overload { terminal: false }) => {
                    closing.store(true, Ordering::Relaxed);
                    deadline.get_or_insert_with(|| tokio::time::Instant::now() + OVERLOAD_GRACE);
                }
                _ => {}
            }
        }
        let _ = client_sender.close().await;
        false
    };
    let idle = idle_expired(&activity, &idle_timeout, tracker.as_ref());
    let ended = tokio::select! {
        _ = to_upstream => Ended::Client,
        overloaded = to_client => if overloaded { Ended::Overloaded } else { Ended::Upstream },
        () = idle => Ended::Idle,
    };
    match ended {
        // A response still open when the upstream hangs up was interrupted; when the
        // client leaves first, the tracker's drop records it as cancelled.
        Ended::Upstream => {
            if let Some(tracker) = &tracker {
                tracker.upstream_finished();
            }
        }
        Ended::Client => {}
        Ended::Idle | Ended::Overloaded => {
            let reason = if matches!(ended, Ended::Idle) {
                tracing::info!(event = "websocket_idle_closed");
                IDLE_CLOSE_REASON
            } else {
                tracing::info!(event = "websocket_overload_closed");
                OVERLOAD_CLOSE_REASON
            };
            let close_client = client_sender.send(AxumMessage::Close(Some(AxumCloseFrame {
                code: NORMAL_CLOSE_CODE,
                reason: reason.into(),
            })));
            let close_upstream =
                upstream_sender.send(UpstreamMessage::Close(Some(UpstreamCloseFrame {
                    code: NORMAL_CLOSE_CODE.into(),
                    reason: reason.into(),
                })));
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                futures_util::future::join(close_client, close_upstream),
            )
            .await;
        }
    }
}

fn client_message(
    message: AxumMessage,
    adapt_responses: bool,
    tracker: Option<&UsageTracker>,
) -> UpstreamMessage {
    match message {
        AxumMessage::Text(text) => {
            let text = if adapt_responses {
                adapt_responses_websocket_message(&text)
            } else {
                text.to_string()
            };
            if let Some(tracker) = tracker {
                tracker.observe_request_text(&text);
            }
            UpstreamMessage::Text(text.into())
        }
        AxumMessage::Binary(bytes) => UpstreamMessage::Binary(bytes),
        AxumMessage::Ping(bytes) => UpstreamMessage::Ping(bytes),
        AxumMessage::Pong(bytes) => UpstreamMessage::Pong(bytes),
        AxumMessage::Close(frame) => {
            UpstreamMessage::Close(frame.map(|frame| UpstreamCloseFrame {
                code: frame.code.into(),
                reason: frame.reason.to_string().into(),
            }))
        }
    }
}

fn upstream_message(
    message: UpstreamMessage,
    tracker: Option<&UsageTracker>,
) -> Option<AxumMessage> {
    match message {
        UpstreamMessage::Text(text) => {
            if let Some(tracker) = tracker {
                tracker.observe_response_text(&text);
            }
            Some(AxumMessage::Text(text.to_string().into()))
        }
        UpstreamMessage::Binary(bytes) => Some(AxumMessage::Binary(bytes)),
        UpstreamMessage::Ping(bytes) => Some(AxumMessage::Ping(bytes)),
        UpstreamMessage::Pong(bytes) => Some(AxumMessage::Pong(bytes)),
        UpstreamMessage::Close(frame) => {
            Some(AxumMessage::Close(frame.map(|frame| AxumCloseFrame {
                code: frame.code.into(),
                reason: frame.reason.to_string().into(),
            })))
        }
        UpstreamMessage::Frame(_) => None,
    }
}

fn rejection_response(
    rejection: tokio_tungstenite::tungstenite::http::Response<Option<Vec<u8>>>,
) -> Response {
    let (parts, body) = rejection.into_parts();
    let mut output = Response::new(Body::from(body.unwrap_or_default()));
    *output.status_mut() = parts.status;
    *output.headers_mut() = response::proxy_headers(&parts.headers);
    output
}

fn websocket_unavailable() -> ApiError {
    ApiError::new(502, "The upstream WebSocket is unavailable.")
        .with_kind("upstream_error")
        .with_code("websocket_proxy_unavailable")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn idle_timer_fires_after_quiet_period_and_resets_on_activity() {
        let activity = Activity::new();
        let timeout = WebsocketIdleTimeout::fixed(60);
        let started = Instant::now();
        tokio::time::sleep(Duration::from_secs(40)).await;
        activity.touch();
        idle_expired(&activity, &timeout, None).await;
        let waited = started.elapsed();
        assert!(waited >= Duration::from_secs(100), "{waited:?}");
        assert!(waited < Duration::from_secs(102), "{waited:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn disabled_timeout_never_fires() {
        let activity = Activity::new();
        let timeout = WebsocketIdleTimeout::fixed(0);
        let fired = tokio::time::timeout(
            Duration::from_secs(3_600),
            idle_expired(&activity, &timeout, None),
        )
        .await;
        assert!(fired.is_err());
    }
}

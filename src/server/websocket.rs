use axum::{
    body::Body,
    extract::ws::{
        CloseFrame as AxumCloseFrame, Message as AxumMessage, WebSocket, WebSocketUpgrade,
    },
    http::{HeaderName, HeaderValue, StatusCode},
    response::Response,
};
use std::{
    sync::atomic::{AtomicU64, Ordering},
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
    chatgpt_proxy::ChatgptProxy, config::WebsocketIdleTimeout, response, usage::UsageTracker,
};

/// Rechecks the idle timeout at least this often, so setting changes apply.
const IDLE_RECHECK: Duration = Duration::from_secs(30);
/// Close code and reason sent to both sides of an idle WebSocket.
const IDLE_CLOSE_CODE: u16 = 1000;
const IDLE_CLOSE_REASON: &str = "idle timeout";

pub async fn proxy(
    upgrade: WebSocketUpgrade,
    target: Url,
    headers: HeaderBag,
    adapt_responses: bool,
    tracker: Option<UsageTracker>,
    proxy: Option<&ChatgptProxy>,
    idle_timeout: WebsocketIdleTimeout,
) -> AppResult<Response> {
    let result = connect(
        upgrade,
        target,
        headers,
        adapt_responses,
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
    adapt_responses: bool,
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
    Ok(upgrade
        .on_upgrade(move |client| bridge(client, upstream, adapt_responses, tracker, idle_timeout)))
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
}

async fn bridge(
    client: WebSocket,
    upstream: WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    adapt_responses: bool,
    tracker: Option<UsageTracker>,
    idle_timeout: WebsocketIdleTimeout,
) {
    let (mut client_sender, mut client_receiver) = client.split();
    let (mut upstream_sender, mut upstream_receiver) = upstream.split();
    let activity = Activity::new();

    let request_tracker = tracker.clone();
    let to_upstream = async {
        while let Some(message) = client_receiver.next().await {
            let Ok(message) = message else {
                break;
            };
            if is_data(&message) {
                activity.touch();
            }
            let message = client_message(message, adapt_responses, request_tracker.as_ref());
            if upstream_sender.send(message).await.is_err() {
                break;
            }
        }
        let _ = upstream_sender.close().await;
    };
    let to_client = async {
        while let Some(message) = upstream_receiver.next().await {
            let Ok(message) = message else {
                break;
            };
            let Some(message) = upstream_message(message, tracker.as_ref()) else {
                continue;
            };
            if is_data(&message) {
                activity.touch();
            }
            if client_sender.send(message).await.is_err() {
                break;
            }
        }
        let _ = client_sender.close().await;
    };
    let idle = idle_expired(&activity, &idle_timeout, tracker.as_ref());
    let ended = tokio::select! {
        _ = to_upstream => Ended::Client,
        _ = to_client => Ended::Upstream,
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
        Ended::Idle => {
            tracing::info!(event = "websocket_idle_closed");
            let close_client = client_sender.send(AxumMessage::Close(Some(AxumCloseFrame {
                code: IDLE_CLOSE_CODE,
                reason: IDLE_CLOSE_REASON.into(),
            })));
            let close_upstream =
                upstream_sender.send(UpstreamMessage::Close(Some(UpstreamCloseFrame {
                    code: IDLE_CLOSE_CODE.into(),
                    reason: IDLE_CLOSE_REASON.into(),
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

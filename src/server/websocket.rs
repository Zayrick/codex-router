use axum::{
    body::Body,
    extract::ws::{
        CloseFrame as AxumCloseFrame, Message as AxumMessage, WebSocket, WebSocketUpgrade,
    },
    http::{HeaderName, HeaderValue},
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        Message as UpstreamMessage, client::IntoClientRequest,
        protocol::CloseFrame as UpstreamCloseFrame,
    },
};
use url::Url;

use crate::{
    core::{ApiError, AppResult},
    http::{HeadersDto, ResponseBodyDto, ResponseDto, upstream_proxy_response},
    protocol::openai::responses::adapt_responses_websocket_message,
    upstream::codex::HeaderBag,
};

use super::{response, usage::UsageTracker};

pub async fn proxy(
    upgrade: WebSocketUpgrade,
    mut target: Url,
    headers: HeaderBag,
    adapt_responses: bool,
    tracker: Option<UsageTracker>,
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

    let (upstream, handshake) = match connect_async(request).await {
        Ok(result) => result,
        Err(tokio_tungstenite::tungstenite::Error::Http(rejection)) => {
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
    Ok(upgrade.on_upgrade(move |client| bridge(client, upstream, adapt_responses, tracker)))
}

async fn bridge(
    client: WebSocket,
    upstream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    adapt_responses: bool,
    tracker: Option<UsageTracker>,
) {
    let (mut client_sender, mut client_receiver) = client.split();
    let (mut upstream_sender, mut upstream_receiver) = upstream.split();

    let request_tracker = tracker.clone();
    let to_upstream = async {
        while let Some(message) = client_receiver.next().await {
            let Ok(message) = message else {
                break;
            };
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
            if client_sender.send(message).await.is_err() {
                break;
            }
        }
        let _ = client_sender.close().await;
    };
    tokio::select! {
        _ = to_upstream => {},
        _ = to_client => {},
    }
}

fn client_message(
    message: AxumMessage,
    adapt_responses: bool,
    tracker: Option<&UsageTracker>,
) -> UpstreamMessage {
    match message {
        AxumMessage::Text(text) => {
            if let Some(tracker) = tracker {
                tracker.observe_request_text(&text);
            }
            let text = if adapt_responses {
                adapt_responses_websocket_message(&text)
            } else {
                text.to_string()
            };
            UpstreamMessage::Text(text.into())
        }
        AxumMessage::Binary(bytes) => {
            if let Some(tracker) = tracker
                && let Ok(text) = std::str::from_utf8(&bytes)
            {
                tracker.observe_request_text(text);
            }
            UpstreamMessage::Binary(bytes)
        }
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
        UpstreamMessage::Binary(bytes) => {
            if let Some(tracker) = tracker
                && let Ok(text) = std::str::from_utf8(&bytes)
            {
                tracker.observe_response_text(text);
            }
            Some(AxumMessage::Binary(bytes))
        }
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
    let headers = HeadersDto::from_pairs(parts.headers.iter().filter_map(|(name, value)| {
        value
            .to_str()
            .ok()
            .map(|value| (name.as_str().to_owned(), value.to_owned()))
    }));
    let policy = upstream_proxy_response(ResponseDto {
        status: parts.status.as_u16(),
        headers,
        body: ResponseBodyDto::Passthrough,
    });
    let mut output = Response::new(Body::from(body.unwrap_or_default()));
    *output.status_mut() = parts.status;
    if let Ok(headers) = response::header_map(&policy.headers) {
        *output.headers_mut() = headers;
    }
    output
}

fn websocket_unavailable() -> ApiError {
    ApiError::new(502, "The upstream WebSocket is unavailable.")
        .with_kind("upstream_error")
        .with_code("websocket_proxy_unavailable")
}

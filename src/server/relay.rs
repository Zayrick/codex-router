use axum::{
    body::Body,
    extract::ws::WebSocketUpgrade,
    http::{Method, Request},
    response::Response,
};
use url::Url;

use crate::{
    auth::{OAuthRepository, UserRepository},
    core::{ApiError, AppResult},
    upstream::{
        codex::{CodexCredentials, resolve_chatgpt_url},
        relay::{ACCOUNT_ID_HEADER, relay_request_headers},
    },
};

use super::{
    account_router::RouteHints,
    codex::header_bag,
    config::AppConfig,
    oauth::current_time_ms,
    response,
    state::AppState,
    usage::{OutcomeSink, UsageIdentity, UsageTracker},
    usage_client::UsageClient,
    websocket,
};

struct RelayReplacement {
    account_id: String,
    /// Feeds the routed account's health; carries the session and probe ticket.
    outcome_sink: OutcomeSink,
    credentials: CodexCredentials,
    identity: UsageIdentity,
}

pub async fn handle_relay(
    request: Request<Body>,
    client_url: Url,
    upgrade: Option<WebSocketUpgrade>,
    config: &AppConfig,
    state: &AppState,
) -> Response {
    match dispatch_relay(request, &client_url, upgrade, config, state).await {
        Ok(output) => output,
        Err(error) => response::api_error(&error),
    }
}

async fn dispatch_relay(
    request: Request<Body>,
    client_url: &Url,
    upgrade: Option<WebSocketUpgrade>,
    config: &AppConfig,
    state: &AppState,
) -> AppResult<Response> {
    let replacement = if client_url.path().starts_with("/backend-api/codex/") {
        let incoming_account_id = request
            .headers()
            .get(ACCOUNT_ID_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        replacement_credentials(
            incoming_account_id.as_deref(),
            request.headers(),
            is_codex_usage_path(client_url.path()),
            state,
        )
        .await?
    } else {
        None
    };
    forward_relay(
        request,
        client_url,
        upgrade,
        replacement.as_ref(),
        config,
        state,
    )
    .await
}

async fn replacement_credentials(
    incoming_account_id: Option<&str>,
    headers: &axum::http::HeaderMap,
    generative: bool,
    state: &AppState,
) -> AppResult<Option<RelayReplacement>> {
    let Some(authenticated) = UserRepository::new(state.config.as_ref())
        .match_account_id(incoming_account_id)
        .await?
    else {
        return Ok(None);
    };
    let Some(account_route) = state
        .account_router
        .resolve_with(
            state.config.as_ref(),
            &authenticated.user.id,
            headers,
            RouteHints {
                model: None,
                generative,
            },
            current_time_ms(),
        )
        .await?
    else {
        return Ok(None);
    };
    let oauth = OAuthRepository::new(state.config.as_ref(), &account_route.account.id);
    let stored = oauth.require_valid(current_time_ms()).await?;
    if stored
        .account_id
        .as_deref()
        .is_none_or(|value| value.trim().is_empty())
    {
        return Err(missing_oauth_account_id());
    }
    let identity = UsageIdentity::from(&authenticated).with_account_route(
        &account_route.account.id,
        &account_route.account.name,
        account_route
            .group
            .as_ref()
            .map(|group| (group.id.as_str(), group.name.as_str())),
    );
    Ok(Some(RelayReplacement {
        outcome_sink: state.account_router.outcome_sink(&account_route),
        account_id: account_route.account.id,
        credentials: CodexCredentials {
            token: stored.access_token,
            account_id: stored.account_id,
        },
        identity,
    }))
}

async fn forward_relay(
    request: Request<Body>,
    client_url: &Url,
    upgrade: Option<WebSocketUpgrade>,
    replacement: Option<&RelayReplacement>,
    config: &AppConfig,
    state: &AppState,
) -> AppResult<Response> {
    let (parts, body) = request.into_parts();
    let source = header_bag(&parts.headers);
    let headers = relay_request_headers(
        &source,
        replacement.map(|replacement| &replacement.credentials),
    );
    let target = resolve_chatgpt_url(client_url.path(), client_url.query());
    let tracker = replacement
        .filter(|_| is_codex_usage_path(client_url.path()))
        .map(|replacement| {
            let client = UsageClient::from_request(&parts.extensions, &parts.headers, config);
            let tracker = if upgrade.is_some() {
                UsageTracker::websocket(
                    state.usage.clone(),
                    replacement.identity.clone(),
                    client,
                    client_url.path(),
                )
            } else {
                UsageTracker::http(
                    state.usage.clone(),
                    replacement.identity.clone(),
                    client,
                    client_url.path(),
                )
            };
            tracker
                .with_outcome_sink(replacement.outcome_sink.clone())
                .track_live(&state.live)
        });
    let routed_account_id = replacement.map(|replacement| replacement.account_id.clone());
    if let Some(upgrade) = upgrade {
        let output = websocket::proxy(
            upgrade,
            target,
            headers,
            websocket::ResponsesOptions {
                adapt: false,
                // Only routed Responses sockets: a reconnect there can pick another account.
                close_on_overload: tracker.is_some(),
            },
            tracker,
            state.chatgpt.proxy(),
            state.chatgpt.websocket_idle_timeout(),
        )
        .await?;
        if let Some(account_id) = routed_account_id.as_deref() {
            state
                .account_router
                .observe_upstream_status(account_id, output.status().as_u16())
                .await;
        }
        return Ok(output);
    }

    let mut outgoing = state
        .chatgpt
        .client()
        .request(parts.method.clone(), target.as_str());
    for (name, value) in headers.iter() {
        outgoing = outgoing.header(name, value);
    }
    if parts.method != Method::GET && parts.method != Method::HEAD {
        let body = body.into_data_stream();
        outgoing = outgoing.body(match tracker.as_ref() {
            Some(tracker) => {
                let content_encoding = headers.get("content-encoding").map(str::to_owned);
                reqwest::Body::wrap_stream(tracker.observe_request_stream(body, content_encoding))
            }
            None => reqwest::Body::wrap_stream(body),
        });
    }
    let upstream = outgoing
        .send()
        .await
        .map_err(relay_fetch_error)
        .inspect_err(|error| {
            if let Some(tracker) = &tracker {
                tracker.fail(error);
            }
        })?;
    if let Some(account_id) = routed_account_id.as_deref() {
        state
            .account_router
            .observe_upstream_status(account_id, upstream.status().as_u16())
            .await;
    }
    Ok(match tracker {
        Some(tracker) => response::upstream_proxy_tracked(upstream, tracker),
        None => response::upstream_proxy(upstream),
    })
}

fn is_codex_usage_path(pathname: &str) -> bool {
    matches!(
        pathname,
        "/backend-api/codex/responses" | "/backend-api/codex/responses/"
    )
}

fn relay_fetch_error(error: reqwest::Error) -> ApiError {
    if error.is_timeout() {
        return ApiError::new(408, "The request was cancelled or timed out.")
            .with_kind("request_timeout")
            .with_code("request_aborted");
    }
    ApiError::new(502, "Unable to reach the ChatGPT upstream.")
        .with_kind("upstream_error")
        .with_code("chatgpt_upstream_unavailable")
}

fn missing_oauth_account_id() -> ApiError {
    ApiError::new(
        503,
        "Stored OAuth credentials do not contain a ChatGPT account ID.",
    )
    .with_kind("configuration_error")
    .with_code("missing_oauth_account_id")
}

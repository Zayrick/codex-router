use axum::{
    body::Body,
    extract::ws::WebSocketUpgrade,
    http::{Method, Request},
    response::Response,
};
use url::Url;

use crate::{
    auth::{ApiKeyRepository, OAuthRepository, RouteConsumerKind, matching_auth_proxy_account},
    core::{ApiError, AppResult},
    upstream::{
        codex::{CodexCredentials, resolve_chatgpt_url},
        relay::{ACCOUNT_ID_HEADER, is_backend_api_path, relay_request_headers},
    },
};

use super::{
    codex::header_bag,
    oauth::current_time_ms,
    response,
    state::AppState,
    usage::{UsageIdentity, UsageTracker},
    websocket,
};

struct RelayReplacement {
    account_id: String,
    credentials: CodexCredentials,
    identity: UsageIdentity,
}

pub async fn handle_relay(
    request: Request<Body>,
    client_url: Url,
    upgrade: Option<WebSocketUpgrade>,
    state: &AppState,
) -> Response {
    match dispatch_relay(request, &client_url, upgrade, state).await {
        Ok(output) => output,
        Err(error) => response::api_error(&error),
    }
}

async fn dispatch_relay(
    request: Request<Body>,
    client_url: &Url,
    upgrade: Option<WebSocketUpgrade>,
    state: &AppState,
) -> AppResult<Response> {
    let replacement = if is_backend_api_path(client_url.path()) {
        let incoming_account_id = request
            .headers()
            .get(ACCOUNT_ID_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        replacement_credentials(incoming_account_id.as_deref(), request.headers(), state).await?
    } else {
        None
    };
    forward_relay(request, client_url, upgrade, replacement.as_ref(), state).await
}

async fn replacement_credentials(
    incoming_account_id: Option<&str>,
    headers: &axum::http::HeaderMap,
    state: &AppState,
) -> AppResult<Option<RelayReplacement>> {
    let accounts = ApiKeyRepository::new(state.config.as_ref())
        .read_auth_proxy_accounts()
        .await?;
    let Some(account) = matching_auth_proxy_account(incoming_account_id, &accounts) else {
        return Ok(None);
    };
    let Some(account_route) = state
        .account_router
        .resolve(
            state.config.as_ref(),
            RouteConsumerKind::AuthProxy,
            &account.id,
            headers,
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
    let identity = UsageIdentity::from(account).with_account_route(
        &account_route.account.id,
        &account_route.account.name,
        account_route
            .group
            .as_ref()
            .map(|group| (group.id.as_str(), group.name.as_str())),
    );
    Ok(Some(RelayReplacement {
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
            if upgrade.is_some() {
                UsageTracker::websocket(
                    state.usage.clone(),
                    replacement.identity.clone(),
                    client_url.path(),
                )
            } else {
                UsageTracker::http(
                    state.usage.clone(),
                    replacement.identity.clone(),
                    client_url.path(),
                )
            }
        });
    let routed_account_id = replacement.map(|replacement| replacement.account_id.clone());
    if let Some(upgrade) = upgrade {
        let output = websocket::proxy(
            upgrade,
            target,
            headers,
            false,
            tracker,
            state.chatgpt.proxy(),
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
        outgoing = outgoing.body(reqwest::Body::wrap_stream(body.into_data_stream()));
    }
    let upstream = outgoing.send().await.map_err(relay_fetch_error)?;
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

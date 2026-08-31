use axum::{
    body::Body,
    extract::ws::WebSocketUpgrade,
    http::{Method, Request},
    response::Response,
};
use url::Url;

use crate::{
    auth::{
        ApiKeyRepository, OAuthRepository, auth_proxy_credentials_or_primary,
        matching_auth_proxy_account,
    },
    core::{ApiError, AppResult},
    upstream::{
        codex::CodexCredentials,
        relay::{
            ACCOUNT_ID_HEADER, is_backend_api_path, relay_request_headers,
            resolve_chatgpt_upstream_url,
        },
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
        replacement_credentials(incoming_account_id.as_deref(), state).await?
    } else {
        None
    };
    forward_relay(request, client_url, upgrade, replacement.as_ref(), state).await
}

async fn replacement_credentials(
    incoming_account_id: Option<&str>,
    state: &AppState,
) -> AppResult<Option<RelayReplacement>> {
    let accounts = ApiKeyRepository::new(state.config.as_ref())
        .read_auth_proxy_accounts()
        .await?;
    let Some(account) = matching_auth_proxy_account(incoming_account_id, &accounts) else {
        return Ok(None);
    };
    let primary = OAuthRepository::new(state.config.as_ref());
    let auth_proxy = OAuthRepository::for_auth_proxy_account(state.config.as_ref(), &account.id);
    let stored =
        auth_proxy_credentials_or_primary(&auth_proxy, &primary, current_time_ms()).await?;
    if stored
        .account_id
        .as_deref()
        .is_none_or(|value| value.trim().is_empty())
    {
        return Err(missing_oauth_account_id());
    }
    Ok(Some(RelayReplacement {
        credentials: CodexCredentials {
            token: stored.access_token,
            account_id: stored.account_id,
        },
        identity: account.into(),
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
    let target = resolve_chatgpt_upstream_url(client_url)?;
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
    if let Some(upgrade) = upgrade {
        return websocket::proxy(
            upgrade,
            target,
            headers,
            false,
            tracker,
            state.chatgpt.proxy(),
        )
        .await;
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

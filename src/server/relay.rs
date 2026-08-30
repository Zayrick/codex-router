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
        relay::{ACCOUNT_ID_HEADER, is_backend_api_path, relay_request_headers, resolve_relay_url},
    },
};

use super::{
    codex::header_bag, config::AppConfig, oauth::current_time_ms, response, state::AppState,
    websocket,
};

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
    forward_relay(
        request,
        client_url,
        upgrade,
        &config.upstream.chatgpt_relay_url,
        replacement.as_ref(),
        state,
    )
    .await
}

async fn replacement_credentials(
    incoming_account_id: Option<&str>,
    state: &AppState,
) -> AppResult<Option<CodexCredentials>> {
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
    Ok(Some(CodexCredentials {
        token: stored.access_token,
        account_id: stored.account_id,
    }))
}

async fn forward_relay(
    request: Request<Body>,
    client_url: &Url,
    upgrade: Option<WebSocketUpgrade>,
    relay_origin: &str,
    credentials: Option<&CodexCredentials>,
    state: &AppState,
) -> AppResult<Response> {
    let (parts, body) = request.into_parts();
    let source = header_bag(&parts.headers);
    let headers = relay_request_headers(&source, credentials);
    let target = resolve_relay_url(relay_origin, client_url)?;
    if let Some(upgrade) = upgrade {
        return websocket::proxy(upgrade, target, headers, false).await;
    }

    let mut outgoing = state.client.request(parts.method.clone(), target.as_str());
    for (name, value) in headers.iter() {
        outgoing = outgoing.header(name, value);
    }
    if parts.method != Method::GET && parts.method != Method::HEAD {
        outgoing = outgoing.body(reqwest::Body::wrap_stream(body.into_data_stream()));
    }
    let upstream = outgoing.send().await.map_err(relay_fetch_error)?;
    Ok(response::upstream_proxy(upstream))
}

fn relay_fetch_error(error: reqwest::Error) -> ApiError {
    if error.is_timeout() {
        return ApiError::new(408, "The request was cancelled or timed out.")
            .with_kind("request_timeout")
            .with_code("request_aborted");
    }
    ApiError::new(502, "Unable to reach the configured ChatGPT relay.")
        .with_kind("upstream_error")
        .with_code("relay_unavailable")
}

fn missing_oauth_account_id() -> ApiError {
    ApiError::new(
        503,
        "Stored OAuth credentials do not contain a ChatGPT account ID.",
    )
    .with_kind("configuration_error")
    .with_code("missing_oauth_account_id")
}

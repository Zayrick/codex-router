use axum::{body::Body, extract::ws::WebSocketUpgrade, http::Request, response::Response};
use serde_json::{Value, json};
use url::Url;

use crate::{
    application::{ApiRoute, Cl100kTokenCounter, ProtocolFamily},
    auth::OAuthRepository,
    core::{ApiError, AppResult},
    protocol::{anthropic, gemini},
    upstream::codex::{MAX_MODEL_CATALOG_BYTES, to_openai_model_list},
};

use super::{
    account_router::ResolvedAccountRoute,
    body,
    codex::{CodexClient, CodexProxyRoute},
    config::AppConfig,
    oauth::current_time_ms,
    provider_error::{anthropic_upstream_error_response, gemini_upstream_error_response},
    response,
    state::AppState,
    stream::{present_non_stream, present_stream},
    usage::{UsageIdentity, UsageTracker},
    usage_client::UsageClient,
};

/// Who sent a request, captured before the request is consumed.
struct UsageSource {
    identity: UsageIdentity,
    client: UsageClient,
}

impl UsageSource {
    fn tracker(self, state: &AppState, endpoint: &str, websocket: bool) -> UsageTracker {
        let store = state.usage.clone();
        if websocket {
            UsageTracker::websocket(store, self.identity, self.client, endpoint)
        } else {
            UsageTracker::http(store, self.identity, self.client, endpoint)
        }
    }
}

pub struct RoutedApiIdentity {
    pub usage: UsageIdentity,
    pub account: ResolvedAccountRoute,
}

pub async fn handle_api(
    route: ApiRoute,
    request: Request<Body>,
    client_url: Url,
    websocket: Option<WebSocketUpgrade>,
    config: &AppConfig,
    state: &AppState,
    identity: RoutedApiIdentity,
) -> Response {
    let family = route.family();
    let usage_client = UsageClient::from_request(request.extensions(), request.headers(), config);
    match dispatch(
        route,
        request,
        &client_url,
        websocket,
        state,
        UsageSource {
            identity: identity.usage,
            client: usage_client,
        },
        &identity.account,
    )
    .await
    {
        Ok(output) => response::with_cors(output, &config.server.cors_origin),
        Err(error)
            if error.status == 404
                && !matches!(family, ProtocolFamily::Anthropic | ProtocolFamily::Gemini) =>
        {
            response::empty(404)
        }
        Err(error) => {
            let output = match family {
                ProtocolFamily::Anthropic => response::json(
                    &anthropic::anthropic_error_payload(&error, None),
                    error.status,
                ),
                ProtocolFamily::Gemini => {
                    response::json(&gemini::gemini_error_payload(&error), error.status)
                }
                ProtocolFamily::OpenAi | ProtocolFamily::Codex => {
                    response::json(&error.openai_payload(), error.status)
                }
            }
            .unwrap_or_else(|_| response::empty(500));
            response::with_cors(output, &config.server.cors_origin)
        }
    }
}

async fn dispatch(
    route: ApiRoute,
    request: Request<Body>,
    client_url: &Url,
    websocket: Option<WebSocketUpgrade>,
    state: &AppState,
    usage: UsageSource,
    account_route: &ResolvedAccountRoute,
) -> AppResult<Response> {
    match &route {
        ApiRoute::MessageTokens => {
            let (parts, body) = request.into_parts();
            let input = body::request_json(&parts.headers, body).await?;
            let adapted = anthropic::messages_request_to_responses(
                &input,
                anthropic::MessageRequestOptions {
                    require_max_tokens: false,
                },
            )?;
            let counter = Cl100kTokenCounter;
            let count = anthropic::count_codex_input_tokens(&adapted.body, &counter);
            return response::json(&json!({ "input_tokens": count }), 200);
        }
        ApiRoute::GeminiTokens { model } => {
            let (parts, body) = request.into_parts();
            let input = body::request_json(&parts.headers, body).await?;
            let counter = Cl100kTokenCounter;
            return response::json(&gemini::gemini_count_tokens(&input, model, &counter)?, 200);
        }
        _ => {}
    }

    let oauth = OAuthRepository::new(state.config.as_ref(), &account_route.account.id);
    let client = CodexClient::new(&oauth, &state.chatgpt);
    match route {
        ApiRoute::Models => {
            let upstream = client.fetch_models(client_url, request.headers()).await?;
            observe_upstream_status(state, account_route, upstream.status().as_u16()).await;
            if !upstream.status().is_success() {
                return Ok(response::upstream_error(upstream));
            }
            if client_url
                .query_pairs()
                .any(|(name, _)| name == "client_version")
            {
                return Ok(response::upstream_json(upstream));
            }
            let payload = upstream_json(upstream).await?;
            response::json(&to_openai_model_list(&payload)?, 200)
        }
        ApiRoute::GeminiModels => {
            let upstream = client.fetch_models(client_url, request.headers()).await?;
            observe_upstream_status(state, account_route, upstream.status().as_u16()).await;
            if !upstream.status().is_success() {
                return Ok(gemini_upstream_error_response(upstream).await);
            }
            let payload = upstream_json(upstream).await?;
            response::json(&gemini::gemini_model_list(&payload)?, 200)
        }
        ApiRoute::GeminiModel { model } => {
            let upstream = client.fetch_models(client_url, request.headers()).await?;
            observe_upstream_status(state, account_route, upstream.status().as_u16()).await;
            if !upstream.status().is_success() {
                return Ok(gemini_upstream_error_response(upstream).await);
            }
            let payload = upstream_json(upstream).await?;
            response::json(&gemini::gemini_model_detail(&payload, &model)?, 200)
        }
        route @ (ApiRoute::Responses | ApiRoute::Compact | ApiRoute::Proxy) => {
            let track_usage = matches!(route, ApiRoute::Responses | ApiRoute::Compact);
            let proxy_route = match route {
                ApiRoute::Responses => CodexProxyRoute::Responses,
                ApiRoute::Compact => CodexProxyRoute::Compact,
                ApiRoute::Proxy => CodexProxyRoute::Proxy,
                _ => return Err(runtime_failure()),
            };
            let tracker =
                track_usage.then(|| usage.tracker(state, client_url.path(), websocket.is_some()));
            if let Some(upgrade) = websocket {
                let output = client
                    .forward_websocket(
                        upgrade,
                        client_url,
                        request.method(),
                        request.headers(),
                        proxy_route,
                        tracker,
                    )
                    .await?;
                observe_upstream_status(state, account_route, output.status().as_u16()).await;
                return Ok(output);
            }
            let upstream = client
                .forward_proxy(request, client_url, proxy_route, tracker.as_ref())
                .await
                .inspect_err(|error| fail_tracker(tracker.as_ref(), error))?;
            observe_upstream_status(state, account_route, upstream.status().as_u16()).await;
            Ok(match tracker {
                Some(tracker) => response::upstream_proxy_tracked(upstream, tracker),
                None => response::upstream_proxy(upstream),
            })
        }
        ApiRoute::MessageTokens | ApiRoute::GeminiTokens { .. } => Err(runtime_failure()),
        route @ (ApiRoute::ChatCompletions
        | ApiRoute::Completions
        | ApiRoute::Messages
        | ApiRoute::GeminiGenerate { .. }) => {
            let (parts, body) = request.into_parts();
            let input = body::request_json(&parts.headers, body).await?;
            let adapted = route.adapter().ok_or_else(runtime_failure)?.adapt(&input)?;
            let tracker = usage.tracker(state, client_url.path(), false);
            tracker.set_client_stream(adapted.stream);
            let upstream = client
                .send_converted_responses(&adapted.body, &parts.headers, &tracker)
                .await
                .inspect_err(|error| tracker.fail(error))?;
            tracker.observe_upstream_status(upstream.status().as_u16());
            observe_upstream_status(state, account_route, upstream.status().as_u16()).await;
            if !upstream.status().is_success() {
                return Ok(match route {
                    ApiRoute::Messages => anthropic_upstream_error_response(upstream).await,
                    ApiRoute::GeminiGenerate { .. } => {
                        gemini_upstream_error_response(upstream).await
                    }
                    _ => response::upstream_error(upstream),
                });
            }
            let created = Value::from(current_time_ms() / 1_000);
            if adapted.stream {
                return Ok(present_stream(upstream, adapted.response, created, tracker));
            }
            let output = present_non_stream(upstream, &adapted.response, created, tracker).await?;
            response::json(&output, 200)
        }
    }
}

fn fail_tracker(tracker: Option<&UsageTracker>, error: &ApiError) {
    if let Some(tracker) = tracker {
        tracker.fail(error);
    }
}

async fn observe_upstream_status(
    state: &AppState,
    account_route: &ResolvedAccountRoute,
    status: u16,
) {
    state
        .account_router
        .observe_upstream_status(&account_route.account.id, status)
        .await;
}

async fn upstream_json(upstream: reqwest::Response) -> AppResult<Value> {
    let bytes = body::read_limited_response(upstream, MAX_MODEL_CATALOG_BYTES).await?;
    serde_json::from_slice(&bytes).map_err(|_| {
        ApiError::new(502, "The Codex backend returned invalid JSON.")
            .with_kind("upstream_error")
            .with_code("invalid_codex_response")
    })
}

fn runtime_failure() -> ApiError {
    ApiError::new(500, "The request could not be completed.")
        .with_kind("internal_error")
        .with_code("native_runtime_error")
}

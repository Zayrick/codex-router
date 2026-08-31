use axum::{
    body::Body,
    http::{HeaderValue, Method, Request, StatusCode, header},
    response::Response,
};
use futures_util::{StreamExt, TryStreamExt, stream};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

use crate::{
    application::{AdminRoute, MatchedAdminRoute},
    auth::{
        ApiKeyRepository, AuthProxyAccount, DeviceAuthorizationService, DevicePollResult,
        OAuthProvider, OAuthRepository, OAuthStatus, admin_secret_matches,
        admin_session_cookie_header, clear_admin_session_cookie_header, create_admin_session,
        has_valid_admin_session, oauth_status,
    },
    core::{ApiError, AppResult, JsonObject},
    upstream::codex::{codex_subscription_from_usage, codex_subscription_metadata},
};

use super::{
    body,
    codex::CodexClient,
    config::AppConfig,
    oauth::{ReqwestOAuthHttpClient, SystemClock, current_time_ms},
    pricing::{ModelPrice, sync_model_prices},
    response,
    state::AppState,
    usage::{UsageBounds, UsageIdentityFilter, UsageRange},
};

const MAX_ADMIN_BODY_BYTES: usize = 16 * 1024;
const AUTH_PROXY_OAUTH_READ_CONCURRENCY: usize = 4;

pub async fn handle_admin(
    matched: MatchedAdminRoute,
    request: Request<Body>,
    client_url: Url,
    config: &AppConfig,
    state: &AppState,
) -> Response {
    match dispatch(matched, request, &client_url, config, state).await {
        Ok(output) => output,
        Err(error) => response::api_error(&error),
    }
}

async fn dispatch(
    matched: MatchedAdminRoute,
    request: Request<Body>,
    client_url: &Url,
    config: &AppConfig,
    state: &AppState,
) -> AppResult<Response> {
    let now_ms = current_time_ms();
    match matched.route {
        AdminRoute::Page => return Ok(response::empty(404)),
        AdminRoute::Login => {
            require_same_origin(&request, config)?;
            let (parts, body) = request.into_parts();
            let bytes = body::read_limited_body(&parts.headers, body, MAX_ADMIN_BODY_BYTES).await?;
            let secret = url::form_urlencoded::parse(&bytes)
                .find(|(name, _)| name == "secret")
                .map(|(_, value)| value.into_owned());
            if !admin_secret_matches(
                &secret.map(Value::String).unwrap_or(Value::Null),
                &config.admin.secret,
            ) {
                return Err(invalid_admin_secret());
            }
            let session = create_admin_session(&config.admin.secret, now_ms)?;
            return redirect_response(
                &matched.base_path,
                client_url,
                Some(&admin_session_cookie_header(&session)),
            );
        }
        AdminRoute::Logout => {
            require_same_origin(&request, config)?;
            return redirect_response(
                &matched.base_path,
                client_url,
                Some(&clear_admin_session_cookie_header()),
            );
        }
        _ => {}
    }

    let cookie = request
        .headers()
        .get("cookie")
        .and_then(|value| value.to_str().ok());
    if !has_valid_admin_session(cookie, &config.admin.secret, now_ms) {
        return Err(invalid_admin_session());
    }
    if request.method() != Method::GET {
        require_same_origin(&request, config)?;
    }

    let oauth = OAuthRepository::new(state.config.as_ref());
    let keys = ApiKeyRepository::new(state.config.as_ref());
    match matched.route {
        AdminRoute::State => {
            let credentials = oauth.read().await?;
            let api_keys = keys.read().await?;
            let accounts = keys.read_auth_proxy_accounts().await?;
            let accounts = auth_proxy_account_states(state, accounts).await?;
            response::json(
                &json!({
                    "oauth": credentials.as_ref().map(oauth_status),
                    "subscription": credentials.as_ref().map(|credentials| {
                        codex_subscription_metadata(credentials.id_token.as_deref())
                    }),
                    "apiKeys": api_keys,
                    "authProxyAccounts": accounts,
                }),
                200,
            )
        }
        AdminRoute::Subscription => {
            let client = CodexClient::new(&oauth, &state.chatgpt);
            let usage = client.fetch_usage().await?;
            let subscription = codex_subscription_from_usage(
                &usage.payload,
                usage.metadata,
                current_time_ms() as f64,
            )?;
            response::json(&json!({ "subscription": subscription }), 200)
        }
        AdminRoute::Usage => {
            let range = client_url
                .query_pairs()
                .find(|(name, _)| name == "range")
                .map(|(_, value)| value.into_owned());
            let range = UsageRange::parse(range.as_deref()).ok_or_else(invalid_usage_range)?;
            let bounds = if range == UsageRange::Cycle {
                let start_at = query_i64(client_url, "startAt")?;
                let end_at = query_i64(client_url, "endAt")?;
                match (start_at, end_at) {
                    (None, None) => None,
                    (Some(start_at), Some(end_at)) => Some(
                        UsageBounds::cycle(start_at, end_at, now_ms)
                            .ok_or_else(invalid_usage_range)?,
                    ),
                    _ => return Err(invalid_usage_range()),
                }
            } else {
                None
            };
            let identity_type = client_url
                .query_pairs()
                .find(|(name, _)| name == "identityType")
                .map(|(_, value)| value.into_owned());
            let identity_id = client_url
                .query_pairs()
                .find(|(name, _)| name == "identityId")
                .map(|(_, value)| value.into_owned());
            let identity = match (identity_type.as_deref(), identity_id.as_deref()) {
                (None, None) => None,
                (Some(kind), Some(id)) => Some(
                    UsageIdentityFilter::parse(kind, id)
                        .ok_or_else(invalid_usage_identity_filter)?,
                ),
                _ => return Err(invalid_usage_identity_filter()),
            };
            let dashboard = state
                .usage
                .dashboard_with_options(
                    range,
                    identity,
                    bounds,
                    &config.usage_tracking.model_prices,
                )
                .await
                .map_err(|error| {
                    tracing::warn!(event = "usage_dashboard", status = "failed", error = %error);
                    usage_query_error()
                })?;
            response::json(&dashboard, 200)
        }
        AdminRoute::PricingGet => {
            let used_models = state.usage.used_models().await.map_err(|error| {
                tracing::warn!(event = "pricing_models", status = "failed", error = %error);
                usage_query_error()
            })?;
            response::json(
                &json!({
                    "prices": config.usage_tracking.model_prices,
                    "usedModels": used_models,
                }),
                200,
            )
        }
        AdminRoute::PricingUpdate => {
            let input = serde_json::from_value::<PricingUpdateInput>(Value::Object(
                admin_json(request).await?,
            ))
            .map_err(|_| invalid_model_prices())?;
            let prices = state.config.replace_model_prices(input.prices).await?;
            response::json(&json!({ "prices": prices }), 200)
        }
        AdminRoute::PricingSync => {
            let used_models = state.usage.used_models().await.map_err(|error| {
                tracing::warn!(event = "pricing_models", status = "failed", error = %error);
                usage_query_error()
            })?;
            let result = sync_model_prices(
                &state.client,
                &used_models,
                &config.usage_tracking.model_prices,
            )
            .await
            .map_err(|error| {
                tracing::warn!(event = "pricing_sync", status = "failed", error = %error);
                pricing_sync_error()
            })?;
            state
                .config
                .replace_model_prices(result.prices.clone())
                .await?;
            response::json(&result, 200)
        }
        AdminRoute::OAuthStart => {
            let clock = SystemClock;
            let http = ReqwestOAuthHttpClient::new(&state.client);
            let provider = OAuthProvider::new(&http, &clock);
            let service =
                DeviceAuthorizationService::new(&oauth, &provider, &clock, &config.admin.secret);
            response::json(&service.start().await?, 201)
        }
        AdminRoute::OAuthPoll => {
            let input = admin_json(request).await?;
            let signed_state = required_device_state(&input)?;
            let clock = SystemClock;
            let http = ReqwestOAuthHttpClient::new(&state.client);
            let provider = OAuthProvider::new(&http, &clock);
            let service =
                DeviceAuthorizationService::new(&oauth, &provider, &clock, &config.admin.secret);
            match service.poll(signed_state).await? {
                DevicePollResult::Pending { retry_after } => response::json(
                    &json!({ "status": "pending", "retryAfter": retry_after }),
                    202,
                ),
                DevicePollResult::Stored { credentials } => response::json(
                    &json!({
                        "status": "stored",
                        "oauth": oauth_status(&credentials),
                        "subscription": codex_subscription_metadata(credentials.id_token.as_deref()),
                    }),
                    200,
                ),
            }
        }
        AdminRoute::OAuthDelete => {
            oauth.delete().await?;
            response::json(&json!({ "oauth": Value::Null }), 200)
        }
        AdminRoute::ApiKeysGet => response::json(&json!({ "apiKeys": keys.read().await? }), 200),
        AdminRoute::ApiKeysCreate => {
            let input = Value::Object(admin_json(request).await?);
            response::json(&json!({ "apiKeys": keys.create(&input).await? }), 201)
        }
        AdminRoute::ApiKeysUpdate => {
            let input = admin_json(request).await?;
            let id = input.get("id").cloned().unwrap_or(Value::Null);
            let value = Value::Object(input);
            response::json(&json!({ "apiKeys": keys.update(&id, &value).await? }), 200)
        }
        AdminRoute::ApiKeysDelete => {
            let input = admin_json(request).await?;
            let id = input.get("id").cloned().unwrap_or(Value::Null);
            response::json(&json!({ "apiKeys": keys.delete(&id).await? }), 200)
        }
        AdminRoute::AuthProxyCreate => {
            let input = Value::Object(admin_json(request).await?);
            let accounts = keys.create_auth_proxy_account(&input).await?;
            auth_proxy_accounts_response(state, accounts, 201).await
        }
        AdminRoute::AuthProxyUpdate => {
            let input = admin_json(request).await?;
            let id = input.get("id").cloned().unwrap_or(Value::Null);
            let value = Value::Object(input);
            let accounts = keys.update_auth_proxy_account(&id, &value).await?;
            auth_proxy_accounts_response(state, accounts, 200).await
        }
        AdminRoute::AuthProxyDelete => {
            let input = admin_json(request).await?;
            let id = input.get("id").cloned().unwrap_or(Value::Null);
            let account = keys.auth_proxy_account(&id).await?;
            OAuthRepository::for_auth_proxy_account(state.config.as_ref(), &account.id)
                .delete()
                .await?;
            let accounts = keys.delete_auth_proxy_account(&id).await?;
            auth_proxy_accounts_response(state, accounts, 200).await
        }
        AdminRoute::AuthProxyOAuthStart => {
            let input = admin_json(request).await?;
            let id = input.get("id").cloned().unwrap_or(Value::Null);
            let account = keys.auth_proxy_account(&id).await?;
            let account_oauth =
                OAuthRepository::for_auth_proxy_account(state.config.as_ref(), &account.id);
            let clock = SystemClock;
            let http = ReqwestOAuthHttpClient::new(&state.client);
            let provider = OAuthProvider::new(&http, &clock);
            let service = DeviceAuthorizationService::scoped(
                &account_oauth,
                &provider,
                &clock,
                &config.admin.secret,
                &account.id,
            );
            response::json(&service.start().await?, 201)
        }
        AdminRoute::AuthProxyOAuthPoll => {
            let input = admin_json(request).await?;
            let id = input.get("id").cloned().unwrap_or(Value::Null);
            let signed_state = required_device_state(&input)?;
            let account = keys.auth_proxy_account(&id).await?;
            let account_oauth =
                OAuthRepository::for_auth_proxy_account(state.config.as_ref(), &account.id);
            let clock = SystemClock;
            let http = ReqwestOAuthHttpClient::new(&state.client);
            let provider = OAuthProvider::new(&http, &clock);
            let service = DeviceAuthorizationService::scoped(
                &account_oauth,
                &provider,
                &clock,
                &config.admin.secret,
                &account.id,
            );
            match service.poll(signed_state).await? {
                DevicePollResult::Pending { retry_after } => response::json(
                    &json!({ "status": "pending", "retryAfter": retry_after }),
                    202,
                ),
                DevicePollResult::Stored { credentials } => response::json(
                    &json!({ "status": "stored", "oauth": oauth_status(&credentials) }),
                    200,
                ),
            }
        }
        AdminRoute::AuthProxyOAuthDelete => {
            let input = admin_json(request).await?;
            let id = input.get("id").cloned().unwrap_or(Value::Null);
            let account = keys.auth_proxy_account(&id).await?;
            OAuthRepository::for_auth_proxy_account(state.config.as_ref(), &account.id)
                .delete()
                .await?;
            response::json(&json!({ "oauth": Value::Null }), 200)
        }
        AdminRoute::Page | AdminRoute::Login | AdminRoute::Logout => Err(invalid_admin_request()),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PricingUpdateInput {
    prices: Vec<ModelPrice>,
}

fn query_i64(url: &Url, name: &str) -> AppResult<Option<i64>> {
    let Some((_, value)) = url.query_pairs().find(|(key, _)| key == name) else {
        return Ok(None);
    };
    value
        .parse::<i64>()
        .map(Some)
        .map_err(|_| invalid_usage_range())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthProxyAccountState {
    #[serde(flatten)]
    account: AuthProxyAccount,
    oauth: Option<OAuthStatus>,
}

async fn auth_proxy_account_states(
    state: &AppState,
    accounts: Vec<AuthProxyAccount>,
) -> AppResult<Vec<AuthProxyAccountState>> {
    stream::iter(accounts)
        .map(|account| {
            let store = state.config.clone();
            async move {
                let oauth = OAuthRepository::for_auth_proxy_account(store.as_ref(), &account.id);
                let credentials = oauth.read().await?;
                Ok(AuthProxyAccountState {
                    account,
                    oauth: credentials.as_ref().map(oauth_status),
                })
            }
        })
        .buffered(AUTH_PROXY_OAUTH_READ_CONCURRENCY)
        .try_collect()
        .await
}

async fn auth_proxy_accounts_response(
    state: &AppState,
    accounts: Vec<AuthProxyAccount>,
    status: u16,
) -> AppResult<Response> {
    let accounts = auth_proxy_account_states(state, accounts).await?;
    response::json(&json!({ "authProxyAccounts": accounts }), status)
}

fn required_device_state(body: &JsonObject) -> AppResult<&str> {
    body.get("state")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApiError::new(400, "Missing device authorization state.")
                .with_kind("invalid_request_error")
                .with_code("missing_required_parameter")
                .with_param("state")
        })
}

async fn admin_json(request: Request<Body>) -> AppResult<JsonObject> {
    let (parts, body) = request.into_parts();
    let bytes = body::read_limited_body(&parts.headers, body, MAX_ADMIN_BODY_BYTES).await?;
    let value = serde_json::from_slice::<Value>(&bytes).map_err(|_| invalid_admin_json())?;
    value.as_object().cloned().ok_or_else(invalid_admin_json)
}

fn require_same_origin(request: &Request<Body>, config: &AppConfig) -> AppResult<()> {
    let origin = request
        .headers()
        .get("origin")
        .and_then(|value| value.to_str().ok());
    if origin == Some(config.server.public_origin.as_str()) {
        Ok(())
    } else {
        Err(invalid_admin_origin())
    }
}

fn redirect_response(
    base_path: &str,
    request_url: &Url,
    cookie: Option<&str>,
) -> AppResult<Response> {
    let location = request_url
        .join(base_path)
        .map_err(|_| invalid_admin_request())?;
    let mut output = Response::new(Body::empty());
    *output.status_mut() = StatusCode::SEE_OTHER;
    output.headers_mut().insert(
        header::LOCATION,
        HeaderValue::from_str(location.as_str()).map_err(|_| invalid_admin_request())?,
    );
    output
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Some(cookie) = cookie {
        output.headers_mut().append(
            header::SET_COOKIE,
            HeaderValue::from_str(cookie).map_err(|_| invalid_admin_request())?,
        );
    }
    Ok(output)
}

fn invalid_admin_session() -> ApiError {
    ApiError::new(401, "The management session is missing or expired.")
        .with_kind("authentication_error")
        .with_code("invalid_admin_session")
}

fn invalid_admin_secret() -> ApiError {
    ApiError::new(401, "管理密钥无效。")
        .with_kind("authentication_error")
        .with_code("invalid_admin_secret")
}

fn invalid_admin_origin() -> ApiError {
    ApiError::new(403, "The management request must be same-origin.")
        .with_kind("authentication_error")
        .with_code("invalid_admin_origin")
}

fn invalid_admin_json() -> ApiError {
    ApiError::new(400, "The management request body is not valid JSON.")
        .with_kind("invalid_request_error")
        .with_code("invalid_json")
}

fn invalid_admin_request() -> ApiError {
    ApiError::new(500, "The management request could not be completed.")
        .with_kind("internal_error")
        .with_code("admin_request_failed")
}

fn usage_query_error() -> ApiError {
    ApiError::new(500, "The usage database could not be queried.")
        .with_kind("internal_error")
        .with_code("usage_query_failed")
}

fn invalid_usage_range() -> ApiError {
    ApiError::new(400, "The usage range is invalid.")
        .with_kind("invalid_request_error")
        .with_code("invalid_usage_range")
}

fn invalid_model_prices() -> ApiError {
    ApiError::new(400, "The model pricing configuration is invalid.")
        .with_kind("invalid_request_error")
        .with_code("invalid_model_prices")
}

fn pricing_sync_error() -> ApiError {
    ApiError::new(502, "Model prices could not be fetched from Models.dev.")
        .with_kind("upstream_error")
        .with_code("pricing_sync_failed")
}

fn invalid_usage_identity_filter() -> ApiError {
    ApiError::new(400, "The usage identity filter is invalid.")
        .with_kind("invalid_request_error")
        .with_code("invalid_usage_identity_filter")
}

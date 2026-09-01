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
        ApiKeyRepository, CodexAccount, DeviceAuthorizationService, DevicePollResult,
        OAuthProvider, OAuthRepository, OAuthStatus, RouteConsumerKind, RoutingRepository,
        admin_secret_matches, admin_session_cookie_header, clear_admin_session_cookie_header,
        create_admin_session, has_valid_admin_session, oauth_status, valid_record_id,
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
    usage::{UsageBounds, UsageFilters, UsageIdentityFilter, UsageRange},
};

const MAX_ADMIN_BODY_BYTES: usize = 16 * 1024;
const CODEX_ACCOUNT_OAUTH_READ_CONCURRENCY: usize = 4;

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

    let keys = ApiKeyRepository::new(state.config.as_ref());
    let routing = RoutingRepository::new(state.config.as_ref());
    match matched.route {
        AdminRoute::State => {
            let api_keys = keys.read().await?;
            let accounts = keys.read_auth_proxy_accounts().await?;
            let routing_state = routing.read().await?;
            let codex_accounts = codex_account_states(state, routing_state.accounts).await?;
            response::json(
                &json!({
                    "codexAccounts": codex_accounts,
                    "apiKeys": api_keys,
                    "authProxyAccounts": accounts,
                    "accountGroups": routing_state.groups,
                    "routes": routing_state.routes,
                }),
                200,
            )
        }
        AdminRoute::CodexAccountSubscription => {
            let id = client_url
                .query_pairs()
                .find(|(name, _)| name == "id")
                .map(|(_, value)| Value::String(value.into_owned()))
                .unwrap_or(Value::Null);
            let account = routing.account(&id).await?;
            let oauth = OAuthRepository::new(state.config.as_ref(), &account.id);
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

            let downstream = query_usage_filter(
                client_url,
                "downstreamType",
                "downstreamId",
                UsageIdentityFilter::parse_downstream,
            )?;
            let upstream = query_usage_filter(
                client_url,
                "upstreamType",
                "upstreamId",
                UsageIdentityFilter::parse_upstream,
            )?;
            let legacy_identity = query_usage_filter(
                client_url,
                "identityType",
                "identityId",
                UsageIdentityFilter::parse_any,
            )?;
            let filters = match legacy_identity {
                Some(identity) if downstream.is_none() && upstream.is_none() => {
                    UsageFilters::from_identity(identity)
                }
                Some(_) => return Err(invalid_usage_identity_filter()),
                None => UsageFilters::new(downstream, upstream),
            };
            let bounds = if range == UsageRange::Cycle {
                if !filters.upstream_is_account() {
                    return Err(invalid_usage_range());
                }
                let start_at = query_i64(client_url, "startAt")?;
                let end_at = query_i64(client_url, "endAt")?;
                match (start_at, end_at) {
                    (Some(start_at), Some(end_at)) => Some(
                        UsageBounds::cycle(start_at, end_at, now_ms)
                            .ok_or_else(invalid_usage_range)?,
                    ),
                    _ => return Err(invalid_usage_range()),
                }
            } else {
                None
            };
            let dashboard = state
                .usage
                .dashboard_with_options(range, filters, bounds, &config.usage_tracking.model_prices)
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
        AdminRoute::CodexAccountOAuthStart => {
            let id = routing.next_account_id().await?;
            let oauth = OAuthRepository::new(state.config.as_ref(), &id);
            let clock = SystemClock;
            let http = ReqwestOAuthHttpClient::new(&state.client);
            let provider = OAuthProvider::new(&http, &clock);
            let service = DeviceAuthorizationService::new(
                &oauth,
                &provider,
                &clock,
                &config.admin.secret,
                &id,
            );
            response::json(
                &json!({
                    "accountId": id,
                    "authorization": service.start().await?,
                }),
                201,
            )
        }
        AdminRoute::CodexAccountOAuthPoll => {
            let input = admin_json(request).await?;
            let id = input
                .get("accountId")
                .and_then(Value::as_str)
                .filter(|value| valid_record_id(value))
                .ok_or_else(invalid_codex_account_id)?;
            let signed_state = required_device_state(&input)?;
            let oauth = OAuthRepository::new(state.config.as_ref(), id);
            let clock = SystemClock;
            let http = ReqwestOAuthHttpClient::new(&state.client);
            let provider = OAuthProvider::new(&http, &clock);
            let service = DeviceAuthorizationService::new(
                &oauth,
                &provider,
                &clock,
                &config.admin.secret,
                id,
            );
            match service.poll(signed_state).await? {
                DevicePollResult::Pending { retry_after } => response::json(
                    &json!({ "status": "pending", "retryAfter": retry_after }),
                    202,
                ),
                DevicePollResult::Stored { credentials } => {
                    if duplicate_codex_login(state, id, &credentials).await? {
                        oauth.delete().await?;
                        return Err(duplicate_codex_account());
                    }
                    let routing_state = match routing
                        .create_account(id.to_owned(), credentials.email.as_deref())
                        .await
                    {
                        Ok(routing_state) => routing_state,
                        Err(error) => {
                            if let Err(cleanup_error) = oauth.delete().await {
                                tracing::warn!(
                                    event = "codex_account_login_cleanup",
                                    status = "failed",
                                    error = %cleanup_error
                                );
                            }
                            return Err(error);
                        }
                    };
                    let account = routing_state
                        .accounts
                        .into_iter()
                        .find(|entry| entry.id == id)
                        .ok_or_else(invalid_admin_request)?;
                    response::json(
                        &json!({
                            "status": "stored",
                            "account": CodexAccountState {
                                account,
                                oauth: Some(oauth_status(&credentials)),
                                subscription: Some(codex_subscription_metadata(credentials.id_token.as_deref())),
                            },
                        }),
                        200,
                    )
                }
            }
        }
        AdminRoute::CodexAccountUpdate => {
            let input = admin_json(request).await?;
            let id = input.get("id").cloned().unwrap_or(Value::Null);
            let value = Value::Object(input);
            let updated = routing.update_account(&id, &value).await?;
            codex_accounts_response(state, updated.accounts, 200).await
        }
        AdminRoute::CodexAccountDelete => {
            let input = admin_json(request).await?;
            let id = input.get("id").cloned().unwrap_or(Value::Null);
            let account = routing.account(&id).await?;
            let updated = routing.delete_account(&id).await?;
            OAuthRepository::new(state.config.as_ref(), &account.id)
                .delete()
                .await?;
            codex_accounts_response(state, updated.accounts, 200).await
        }
        AdminRoute::AccountRoutingGet => {
            let routing_state = routing.read().await?;
            response::json(
                &json!({
                    "accountGroups": routing_state.groups,
                    "routes": routing_state.routes,
                }),
                200,
            )
        }
        AdminRoute::AccountRoutingUpdate => {
            let value = Value::Object(admin_json(request).await?);
            let api_keys = keys.read().await?;
            let accounts = keys.read_auth_proxy_accounts().await?;
            let updated = routing
                .replace_configuration(&value, &api_keys, &accounts)
                .await?;
            response::json(
                &json!({
                    "accountGroups": updated.groups,
                    "routes": updated.routes,
                }),
                200,
            )
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
            let updated = keys.delete(&id).await?;
            if let Some(id) = id.as_str() {
                routing
                    .release_consumer(RouteConsumerKind::ApiKey, id)
                    .await?;
            }
            response::json(&json!({ "apiKeys": updated }), 200)
        }
        AdminRoute::AuthProxyCreate => {
            let input = Value::Object(admin_json(request).await?);
            let accounts = keys.create_auth_proxy_account(&input).await?;
            response::json(&json!({ "authProxyAccounts": accounts }), 201)
        }
        AdminRoute::AuthProxyUpdate => {
            let input = admin_json(request).await?;
            let id = input.get("id").cloned().unwrap_or(Value::Null);
            let value = Value::Object(input);
            let accounts = keys.update_auth_proxy_account(&id, &value).await?;
            response::json(&json!({ "authProxyAccounts": accounts }), 200)
        }
        AdminRoute::AuthProxyDelete => {
            let input = admin_json(request).await?;
            let id = input.get("id").cloned().unwrap_or(Value::Null);
            let accounts = keys.delete_auth_proxy_account(&id).await?;
            if let Some(id) = id.as_str() {
                routing
                    .release_consumer(RouteConsumerKind::AuthProxy, id)
                    .await?;
            }
            response::json(&json!({ "authProxyAccounts": accounts }), 200)
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

fn query_usage_filter(
    url: &Url,
    type_name: &str,
    id_name: &str,
    parse: fn(&str, &str) -> Option<UsageIdentityFilter>,
) -> AppResult<Option<UsageIdentityFilter>> {
    let identity_type = url
        .query_pairs()
        .find(|(name, _)| name == type_name)
        .map(|(_, value)| value.into_owned());
    let identity_id = url
        .query_pairs()
        .find(|(name, _)| name == id_name)
        .map(|(_, value)| value.into_owned());
    match (identity_type.as_deref(), identity_id.as_deref()) {
        (None, None) => Ok(None),
        (Some(kind), Some(id)) => parse(kind, id)
            .map(Some)
            .ok_or_else(invalid_usage_identity_filter),
        _ => Err(invalid_usage_identity_filter()),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexAccountState {
    #[serde(flatten)]
    account: CodexAccount,
    oauth: Option<OAuthStatus>,
    subscription: Option<crate::upstream::codex::CodexSubscriptionMetadata>,
}

async fn codex_account_states(
    state: &AppState,
    accounts: Vec<CodexAccount>,
) -> AppResult<Vec<CodexAccountState>> {
    stream::iter(accounts)
        .map(|account| {
            let store = state.config.clone();
            async move {
                let oauth = OAuthRepository::new(store.as_ref(), &account.id);
                let credentials = oauth.read().await?;
                Ok(CodexAccountState {
                    account,
                    oauth: credentials.as_ref().map(oauth_status),
                    subscription: credentials.as_ref().map(|credentials| {
                        codex_subscription_metadata(credentials.id_token.as_deref())
                    }),
                })
            }
        })
        .buffered(CODEX_ACCOUNT_OAUTH_READ_CONCURRENCY)
        .try_collect()
        .await
}

async fn codex_accounts_response(
    state: &AppState,
    accounts: Vec<CodexAccount>,
    status: u16,
) -> AppResult<Response> {
    let accounts = codex_account_states(state, accounts).await?;
    response::json(&json!({ "codexAccounts": accounts }), status)
}

async fn duplicate_codex_login(
    state: &AppState,
    pending_id: &str,
    credentials: &crate::auth::StoredOAuthCredentials,
) -> AppResult<bool> {
    let Some(account_id) = credentials.account_id.as_deref() else {
        return Ok(false);
    };
    let routing = RoutingRepository::new(state.config.as_ref()).read().await?;
    for account in routing.accounts {
        if account.id == pending_id {
            continue;
        }
        let stored = OAuthRepository::new(state.config.as_ref(), &account.id)
            .read()
            .await?;
        if stored
            .as_ref()
            .and_then(|value| value.account_id.as_deref())
            == Some(account_id)
        {
            return Ok(true);
        }
    }
    Ok(false)
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

fn invalid_codex_account_id() -> ApiError {
    ApiError::new(400, "The Codex account ID is invalid.")
        .with_kind("invalid_request_error")
        .with_code("invalid_codex_account")
}

fn duplicate_codex_account() -> ApiError {
    ApiError::new(409, "This Codex account is already logged in.")
        .with_kind("invalid_request_error")
        .with_code("codex_account_conflict")
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

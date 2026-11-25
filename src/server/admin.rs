use axum::{body::Body, http::Request, response::Response};
use futures_util::{StreamExt, TryStreamExt, stream};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

use crate::{
    application::AdminRoute,
    auth::{
        CodexAccount, CredentialImport, CredentialKind, DeviceAuthorizationService,
        DevicePollResult, MAX_CREDENTIAL_IMPORT_BYTES, OAuthProvider, OAuthRepository, OAuthStatus,
        RoutingRepository, StateStore, User, UserRepository, constant_time_equal, oauth_status,
        valid_record_id,
    },
    core::{ApiError, AppResult, JsonObject},
    upstream::codex::{codex_subscription_from_usage, codex_subscription_metadata},
};

use super::{
    body,
    chatgpt_proxy::{ChatgptProxy, ChatgptTransport, REDACTED_PASSWORD, redact_proxy_url},
    codex::CodexClient,
    config::{AdminSettings, AppConfig, PRICING_SYNC_KEY},
    live,
    oauth::{ReqwestOAuthHttpClient, SystemClock, current_time_ms},
    pricing::{ModelPrice, sync_model_prices},
    quota::quota_overview,
    response,
    scheduled::{NotificationChannel, refresh_account_quotas, send_test_notification},
    session_store::Principal,
    state::AppState,
    usage::{UsageBounds, UsageFilters, UsageIdentityFilter, UsageNames, UsageRange},
    usage_analytics::AnalyticsScope,
    usage_api,
};

/// Outcome of the last price synchronization, kept to label synced prices.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PricingSyncStatus {
    synced_at: i64,
    source: String,
    source_url: String,
    matched_models: Vec<String>,
    unmatched_models: Vec<String>,
    updated_models: Vec<String>,
    /// Models with hand-edited prices that the sync left unchanged.
    #[serde(default)]
    skipped_models: Vec<String>,
    /// Prices as written by the sync; a different current price was edited by hand.
    synced_prices: Vec<ModelPrice>,
}

const MAX_ADMIN_BODY_BYTES: usize = 16 * 1024;
const CODEX_ACCOUNT_OAUTH_READ_CONCURRENCY: usize = 4;

#[cfg(test)]
#[path = "admin_import_tests.rs"]
mod import_tests;

/// Management API dispatch. Callers must already have verified an
/// administrator session and, for writes, a same-origin request.
pub async fn handle_admin(
    route: AdminRoute,
    request: Request<Body>,
    client_url: Url,
    config: &AppConfig,
    state: &AppState,
) -> Response {
    match dispatch(route, request, &client_url, config, state).await {
        Ok(output) => output,
        Err(error) => response::api_error(&error),
    }
}

async fn dispatch(
    route: AdminRoute,
    request: Request<Body>,
    client_url: &Url,
    config: &AppConfig,
    state: &AppState,
) -> AppResult<Response> {
    let now_ms = current_time_ms();
    let users = UserRepository::new(state.config.as_ref());
    let routing = RoutingRepository::new(state.config.as_ref());
    match route {
        AdminRoute::State => {
            let user_list = users.read().await?;
            let routing_state = routing.read().await?;
            let codex_accounts = codex_account_states(state, routing_state.accounts).await?;
            response::json(
                &json!({
                    "codexAccounts": codex_accounts,
                    "users": user_list,
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
            let transport = state.account_transport(&account)?;
            let client = CodexClient::new(&oauth, &transport);
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
                .dashboard_with_options(
                    range,
                    filters,
                    bounds,
                    &config.usage_tracking.model_prices,
                    config.usage_tracking.fast_cost_multiplier(),
                    UsageNames::new(&config.state.users, &config.state.account_routing),
                )
                .await
                .map_err(|error| {
                    tracing::warn!(event = "usage_dashboard", status = "failed", error = %error);
                    usage_query_error()
                })?;
            response::json(&dashboard, 200)
        }
        AdminRoute::UsageAnalytics => {
            usage_api::analytics(client_url, AnalyticsScope::Admin, config, state).await
        }
        AdminRoute::UsageLastSeen => {
            let last_seen = state.usage.last_seen().await.map_err(|error| {
                tracing::warn!(event = "usage_last_seen", status = "failed", error = %error);
                usage_query_error()
            })?;
            response::json(&last_seen, 200)
        }
        AdminRoute::Live => Ok(live::stream(state)),
        AdminRoute::UsageEvents => {
            usage_api::events(client_url, AnalyticsScope::Admin, config, state).await
        }
        AdminRoute::Quota => {
            // `refresh=1` samples every enabled account, `refresh=<id>` only that one.
            if let Some(target) = client_url
                .query_pairs()
                .find(|(name, _)| name == "refresh")
                .map(|(_, value)| value.into_owned())
            {
                let only = (target != "1").then_some(target);
                refresh_account_quotas(state, only.as_deref()).await;
            }
            let config = state.config.snapshot().await;
            let overview = quota_overview(state, &config, None, current_time_ms()).await?;
            response::json(&overview, 200)
        }
        AdminRoute::NotificationTest => {
            let input = admin_json(request).await?;
            let channel = match input.get("channel").and_then(Value::as_str) {
                Some("bark") => NotificationChannel::Bark,
                Some("dingtalk") => NotificationChannel::DingTalk,
                _ => return Err(invalid_admin_request()),
            };
            if !send_test_notification(state, channel).await? {
                return Err(notification_channel_disabled());
            }
            response::json(&json!({ "sent": true }), 200)
        }
        AdminRoute::SettingsGet => response::json(&AdminSettings::from(config), 200),
        AdminRoute::SettingsUpdate => {
            let input =
                serde_json::from_value::<AdminSettings>(Value::Object(admin_json(request).await?))
                    .map_err(|_| invalid_admin_settings())?;
            let settings = state.config.replace_admin_settings(input).await?;
            response::json(&settings, 200)
        }
        AdminRoute::PricingGet => {
            let used_models = state.usage.used_models().await.map_err(|error| {
                tracing::warn!(event = "pricing_models", status = "failed", error = %error);
                usage_query_error()
            })?;
            let sync = read_pricing_sync(state).await;
            response::json(
                &json!({
                    "prices": config.usage_tracking.model_prices,
                    "usedModels": used_models,
                    "sync": sync,
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
            // Prices edited by hand since the last sync are kept as they are.
            let previous_sync = read_pricing_sync(state).await;
            let skipped_models =
                manual_price_models(&config.usage_tracking.model_prices, previous_sync.as_ref());
            let candidates = used_models
                .into_iter()
                .filter(|model| {
                    !skipped_models
                        .iter()
                        .any(|skipped| skipped.eq_ignore_ascii_case(model))
                })
                .collect::<Vec<_>>();
            let result = sync_model_prices(
                &state.client,
                &candidates,
                &config.usage_tracking.model_prices,
            )
            .await
            .map_err(|error| {
                tracing::warn!(event = "pricing_sync", status = "failed", error = %error);
                pricing_sync_error()
            })?;
            let previous = &config.usage_tracking.model_prices;
            let updated_models = result
                .matched_models
                .iter()
                .filter(|model| {
                    let before = previous
                        .iter()
                        .find(|price| price.model.eq_ignore_ascii_case(model));
                    let after = result
                        .prices
                        .iter()
                        .find(|price| price.model.eq_ignore_ascii_case(model));
                    before != after
                })
                .cloned()
                .collect();
            state
                .config
                .replace_model_prices(result.prices.clone())
                .await?;
            let status = PricingSyncStatus {
                synced_at: now_ms,
                source: result.source.into(),
                source_url: result.source_url.into(),
                matched_models: result.matched_models.clone(),
                unmatched_models: result.unmatched_models.clone(),
                updated_models,
                skipped_models,
                // Newly matched prices, plus earlier synced prices left untouched.
                synced_prices: result
                    .prices
                    .iter()
                    .filter(|price| {
                        result
                            .matched_models
                            .iter()
                            .any(|model| model.eq_ignore_ascii_case(&price.model))
                            || previous_sync
                                .as_ref()
                                .is_some_and(|sync| sync.synced_prices.contains(price))
                    })
                    .cloned()
                    .collect(),
            };
            if let Ok(serialized) = serde_json::to_string(&status) {
                state.config.put(PRICING_SYNC_KEY, &serialized).await?;
            }
            response::json(
                &json!({
                    "source": result.source,
                    "sourceUrl": result.source_url,
                    "prices": result.prices,
                    "matchedModels": result.matched_models,
                    "unmatchedModels": result.unmatched_models,
                    "sync": status,
                }),
                200,
            )
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
        AdminRoute::CodexAccountImport => {
            let (parts, body) = request.into_parts();
            let bytes =
                body::read_limited_body(&parts.headers, body, MAX_CREDENTIAL_IMPORT_BYTES).await?;
            let input =
                serde_json::from_slice::<Value>(&bytes).map_err(|_| invalid_admin_json())?;
            let import = CredentialImport::parse(&input)?;
            // Detect existing credentials before a refresh can rotate their token.
            if state
                .config
                .snapshot()
                .await
                .state
                .codex_account_oauth
                .values()
                .any(|stored| import.matches_credentials(stored))
            {
                return Err(duplicate_codex_account());
            }
            let clock = SystemClock;
            let http = ReqwestOAuthHttpClient::new(&state.client);
            let provider = OAuthProvider::new(&http, &clock);
            let credentials = import.resolve(&provider, &clock).await?;
            let oauth = Some(oauth_status(&credentials));
            let subscription = Some(codex_subscription_metadata(credentials.id_token.as_deref()));
            let account = state
                .config
                .import_codex_account(credentials, import.name.as_deref())
                .await?;
            response::json(
                &json!({
                    "account": CodexAccountState { account, oauth, subscription },
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
            let mut input = admin_json(request).await?;
            let id = input.get("id").cloned().unwrap_or(Value::Null);
            if let Some(proxy) = input.get("proxy").cloned() {
                let current = routing.account(&id).await?.proxy;
                input.insert(
                    "proxy".into(),
                    match submitted_proxy(&proxy, current.as_deref())? {
                        Some(proxy) => Value::String(proxy),
                        None => Value::Null,
                    },
                );
            }
            let value = Value::Object(input);
            let updated = routing.update_account(&id, &value).await?;
            codex_accounts_response(state, updated.accounts, 200).await
        }
        AdminRoute::CodexAccountProxyTest => {
            let input = admin_json(request).await?;
            let current = match input.get("id") {
                Some(id) if !id.is_null() => routing.account(id).await?.proxy,
                _ => None,
            };
            let proxy = submitted_proxy(
                input.get("proxy").unwrap_or(&Value::Null),
                current.as_deref(),
            )?;
            let transport = state
                .chatgpt
                .for_proxy(proxy.as_deref())
                .map_err(|_| invalid_proxy("The proxy URL is invalid."))?;
            response::json(&probe_proxy(&transport).await, 200)
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
            let user_list = users.read().await?;
            let updated = routing.replace_configuration(&value, &user_list).await?;
            response::json(
                &json!({
                    "accountGroups": updated.groups,
                    "routes": updated.routes,
                }),
                200,
            )
        }
        AdminRoute::UsersGet => response::json(&json!({ "users": users.read().await? }), 200),
        AdminRoute::UsersCreate => {
            let input = admin_json(request).await?;
            reject_admin_secret_reuse(&input, config)?;
            let input = Value::Object(input);
            response::json(&json!({ "users": users.create(&input).await? }), 201)
        }
        AdminRoute::UsersUpdate => {
            let input = admin_json(request).await?;
            reject_admin_secret_reuse(&input, config)?;
            let id = input.get("id").cloned().unwrap_or(Value::Null);
            let _credentials = state.credential_lock.lock().await;
            if let Some(id) = id.as_str() {
                let current = users.user(id).await?;
                if current.is_some_and(|user| revokes_sessions(&user, &input)) {
                    revoke_sessions(state, Some(Principal::User(id.into()))).await?;
                }
            }
            let value = Value::Object(input);
            response::json(&json!({ "users": users.update(&id, &value).await? }), 200)
        }
        AdminRoute::UsersDelete => {
            let input = admin_json(request).await?;
            let id = input.get("id").cloned().unwrap_or(Value::Null);
            let _credentials = state.credential_lock.lock().await;
            revoke_sessions(state, id.as_str().map(|id| Principal::User(id.into()))).await?;
            let updated = users.delete(&id).await?;
            if let Some(id) = id.as_str() {
                routing.release_user(id).await?;
            }
            response::json(&json!({ "users": updated }), 200)
        }
    }
}

/// Whether an update takes away a credential that may back a console session:
/// the user is disabled, or an active credential is removed, disabled, or
/// changed. Pure additions and renames keep existing sessions.
fn revokes_sessions(current: &User, input: &JsonObject) -> bool {
    if input.get("enabled") != Some(&Value::Bool(true)) {
        return true;
    }
    let submitted = input
        .get("credentials")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    current.active_credentials().any(|credential| {
        !submitted.iter().any(|entry| {
            let value = entry.get("value").and_then(Value::as_str).map(|value| {
                if credential.kind == CredentialKind::AccountId {
                    value.trim()
                } else {
                    value
                }
            });
            entry.get("id").and_then(Value::as_str) == Some(credential.id.as_str())
                && entry.get("enabled") == Some(&Value::Bool(true))
                && entry.get("type").and_then(Value::as_str) == Some(credential.kind.as_str())
                && value == Some(credential.value.as_str())
        })
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PricingUpdateInput {
    prices: Vec<ModelPrice>,
}

/// Models whose current price differs from the last synchronized one (or was
/// never synchronized). Without an earlier sync every price may be replaced.
fn manual_price_models(prices: &[ModelPrice], previous: Option<&PricingSyncStatus>) -> Vec<String> {
    let Some(previous) = previous else {
        return Vec::new();
    };
    prices
        .iter()
        .filter(|price| !previous.synced_prices.contains(price))
        .map(|price| price.model.clone())
        .collect()
}

async fn read_pricing_sync(state: &AppState) -> Option<PricingSyncStatus> {
    let serialized = state.config.get(PRICING_SYNC_KEY).await.ok()??;
    serde_json::from_str(&serialized).ok()
}

fn notification_channel_disabled() -> ApiError {
    ApiError::new(
        409,
        "The notification channel is not enabled or not configured.",
    )
    .with_kind("invalid_request_error")
    .with_code("notification_channel_disabled")
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
                    // The console never sees a stored proxy password.
                    account: CodexAccount {
                        proxy: account.proxy.as_deref().map(redact_proxy_url),
                        ..account
                    },
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

/// Revokes a principal's console sessions before its credential is disabled,
/// deleted, or changed, so re-enabling it or restoring the old value cannot
/// revive a stolen session. Callers hold `credential_lock`, which also blocks
/// sign-ins, so no session can be created between revocation and the change.
async fn revoke_sessions(state: &AppState, principal: Option<Principal>) -> AppResult<()> {
    let Some(principal) = principal else {
        return Ok(());
    };
    state
        .sessions
        .revoke_principal(&principal)
        .await
        .map(|_| ())
        .map_err(|error| {
            tracing::error!(event = "console_session_revoke", status = "failed", error = %error);
            ApiError::new(
                503,
                "Existing sessions could not be revoked; nothing was changed.",
            )
            .with_kind("server_error")
            .with_code("session_unavailable")
        })
}

/// A user credential equal to the admin secret would make sign-in ambiguous.
fn reject_admin_secret_reuse(input: &JsonObject, config: &AppConfig) -> AppResult<()> {
    let field = "credentials";
    let reused = input
        .get(field)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("value").and_then(Value::as_str))
        .any(|value| {
            constant_time_equal(value, &config.admin.secret)
                || constant_time_equal(value.trim(), config.admin.secret.trim())
        });
    if reused {
        return Err(
            ApiError::new(400, "This credential is already used by the administrator.")
                .with_kind("invalid_request_error")
                .with_code("credential_conflict")
                .with_param(field),
        );
    }
    Ok(())
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
    ApiError::new(409, "该 Codex 账户已存在，请勿重复添加。")
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

fn invalid_admin_settings() -> ApiError {
    ApiError::new(400, "设置配置无效，请检查地址、凭据与账户范围。")
        .with_kind("invalid_request_error")
        .with_code("invalid_admin_settings")
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

/// Resolves a proxy URL from the console: blank clears it, the redacted form
/// of the stored URL keeps it, anything else must parse.
fn submitted_proxy(value: &Value, current: Option<&str>) -> AppResult<Option<String>> {
    let proxy = match value {
        Value::Null => return Ok(None),
        Value::String(proxy) => proxy.trim(),
        _ => return Err(invalid_proxy("The proxy URL must be a string.")),
    };
    if proxy.is_empty() {
        return Ok(None);
    }
    if let Some(current) = current
        && proxy == redact_proxy_url(current)
    {
        return Ok(Some(current.to_owned()));
    }
    if url::Url::parse(proxy).is_ok_and(|url| url.password() == Some(REDACTED_PASSWORD)) {
        return Err(invalid_proxy("Re-enter the proxy password."));
    }
    ChatgptProxy::parse(proxy).map_err(|error| invalid_proxy(&format!("{error}.")))?;
    Ok(Some(proxy.to_owned()))
}

fn invalid_proxy(message: &str) -> ApiError {
    ApiError::new(400, message.to_owned())
        .with_kind("invalid_request_error")
        .with_code("invalid_proxy")
}

/// Reaches ChatGPT through `transport` and reports the exit address it saw.
async fn probe_proxy(transport: &ChatgptTransport) -> Value {
    let started = std::time::Instant::now();
    let result = transport
        .client()
        .get("https://chatgpt.com/cdn-cgi/trace")
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await;
    let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let response = match result {
        Ok(response) => response,
        Err(error) => {
            return json!({ "ok": false, "latencyMs": latency_ms, "error": proxy_error_text(&error) });
        }
    };
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let field = |name: &str| {
        body.lines()
            .find_map(|line| line.strip_prefix(&format!("{name}=")))
            .map(str::to_owned)
    };
    if !status.is_success() {
        return json!({ "ok": false, "latencyMs": latency_ms, "error": format!("HTTP {}", status.as_u16()) });
    }
    json!({ "ok": true, "latencyMs": latency_ms, "ip": field("ip"), "location": field("loc") })
}

fn proxy_error_text(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "timed out".into()
    } else if error.is_connect() {
        "could not connect through the proxy".into()
    } else {
        "request failed".into()
    }
}

#[cfg(test)]
mod pricing_sync_tests {
    use super::*;

    fn price(model: &str, input: f64) -> ModelPrice {
        ModelPrice {
            model: model.into(),
            input,
            output: 10.0,
            cache_read: 0.0,
            cache_write: 0.0,
            multiplier: 1.0,
        }
    }

    fn status(synced: Vec<ModelPrice>) -> PricingSyncStatus {
        PricingSyncStatus {
            synced_at: 1,
            source: "Models.dev".into(),
            source_url: String::new(),
            matched_models: Vec::new(),
            unmatched_models: Vec::new(),
            updated_models: Vec::new(),
            skipped_models: Vec::new(),
            synced_prices: synced,
        }
    }

    #[test]
    fn hand_edited_and_hand_added_prices_are_skipped_by_the_next_sync() {
        let current = vec![
            price("synced", 1.0),
            price("edited", 2.0),
            price("added", 3.0),
        ];
        let previous = status(vec![price("synced", 1.0), price("edited", 1.5)]);
        assert_eq!(
            manual_price_models(&current, Some(&previous)),
            vec!["edited", "added"]
        );
        assert!(manual_price_models(&current, None).is_empty());
    }
}

#[cfg(test)]
mod proxy_tests {
    use serde_json::json;

    use super::submitted_proxy;

    #[test]
    fn submitted_proxies_are_cleared_kept_or_validated() {
        let stored = "socks5h://user:secret@proxy.example:1080";
        // Blank or null clears.
        assert_eq!(submitted_proxy(&json!(""), Some(stored)).unwrap(), None);
        assert_eq!(submitted_proxy(&json!(null), Some(stored)).unwrap(), None);
        // The redacted form the console shows keeps the stored password.
        assert_eq!(
            submitted_proxy(
                &json!("socks5h://user:********@proxy.example:1080"),
                Some(stored)
            )
            .unwrap()
            .as_deref(),
            Some(stored)
        );
        // A new URL replaces it after validation.
        assert_eq!(
            submitted_proxy(&json!(" http://proxy.example:3128 "), Some(stored))
                .unwrap()
                .as_deref(),
            Some("http://proxy.example:3128")
        );
        for invalid in [json!("ftp://proxy.example"), json!("not a url"), json!(42)] {
            let error = submitted_proxy(&invalid, None).unwrap_err();
            assert_eq!(error.status, 400);
            assert_eq!(error.code.as_deref(), Some("invalid_proxy"));
        }
        // The placeholder password is never stored as a real one.
        assert!(
            submitted_proxy(
                &json!("socks5h://user:********@other.example:1080"),
                Some(stored)
            )
            .is_err()
        );
        assert!(
            submitted_proxy(&json!("socks5h://user:********@proxy.example:1080"), None).is_err()
        );
    }
}
